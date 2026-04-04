//! Two-node integration: real `network::spawn_incoming_loop` + `sync_from_peer` over TCP.
//!
//! Node A holds a chain with one post-genesis block; node B starts from genesis-only and
//! catches up. Asserts height, block hash, and replicated sender account state match.

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use node::block::Block;
use node::blockchain::Blockchain;
use node::mempool::Mempool;
use node::network::{spawn_incoming_loop, sync_from_peer, NodeInner};
use node::storage::BlockStore;
use node::types::Address;
use node::wallet::Wallet;

struct TempDataDir(PathBuf);

impl TempDataDir {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!(
            "trilogicon_two_node_{}_{}_{}",
            label,
            std::process::id(),
            nanos
        ));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }

    fn path(&self) -> &PathBuf {
        &self.0
    }
}

impl Drop for TempDataDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Node A: funded wallet, one sealed block persisted to `chain.blocks`, ready to serve GET_BLOCKS.
fn build_node_a(dir: &PathBuf) -> (Arc<Mutex<NodeInner>>, Wallet) {
    let chain_path = dir.join("chain.blocks");
    let _ = fs::remove_file(&chain_path);

    let wallet_a = Wallet::generate();
    let mut chain = Blockchain::new();
    chain
        .state_mut()
        .create_account(wallet_a.address(), 100);

    let tx = wallet_a
        .sign_transfer(Address::new("recv_two_node"), 5, 1, 0, 1_700_500_000)
        .unwrap();
    let mut block = Block {
        height: 1,
        previous_hash: "GENESIS_HASH".into(),
        timestamp_unix: 1_700_500_001,
        transactions: vec![tx],
        block_hash: String::new(),
    };
    block.block_hash = block.compute_block_hash();
    chain.append_block(block.clone()).unwrap();

    let mut store = BlockStore::open_append(&chain_path).unwrap();
    store.append_block(&block).unwrap();

    let inner = NodeInner {
        chain,
        pool: Mempool::new(100),
        store,
    };
    (Arc::new(Mutex::new(inner)), wallet_a)
}

#[test]
fn follower_syncs_height_and_state_from_listener_peer() {
    let dir_a = TempDataDir::new("a");
    let dir_b = TempDataDir::new("b");

    let (state_a, wallet_a) = build_node_a(dir_a.path());

    let (_listen_handle, listen_addr) =
        spawn_incoming_loop("127.0.0.1:0", state_a.clone()).expect("bind listener");

    let chain_path_b = dir_b.path().join("chain.blocks");
    let _ = fs::remove_file(&chain_path_b);

    let wallet_b = Wallet::generate();
    let mut chain_b = Blockchain::new();
    chain_b
        .state_mut()
        .create_account(wallet_b.address(), 1_000_000);
    // Same pre-state as A before height-1: sender must exist for replay (no state snapshot on wire yet).
    chain_b
        .state_mut()
        .create_account(wallet_a.address(), 100);
    let store_b = BlockStore::open_append(&chain_path_b).unwrap();
    let mut inner_b = NodeInner {
        chain: chain_b,
        pool: Mempool::new(100),
        store: store_b,
    };

    let now = node::network::unix_now_secs();
    let n = sync_from_peer(&mut inner_b, &listen_addr, now).expect("sync");
    assert_eq!(n, 1, "expected one block pulled from peer");

    assert_eq!(inner_b.chain.height(), 1);
    assert_eq!(inner_b.chain.len(), 2);

    let tip_hash_a = state_a
        .lock()
        .expect("lock a")
        .chain
        .blocks()
        .last()
        .unwrap()
        .block_hash
        .clone();
    assert_eq!(
        inner_b.chain.blocks().last().unwrap().block_hash,
        tip_hash_a
    );

    let acc_a = {
        let ga = state_a.lock().expect("lock a");
        ga.chain
            .state()
            .get_account(&wallet_a.address())
            .unwrap()
            .clone()
    };
    let acc_b = inner_b
        .chain
        .state()
        .get_account(&wallet_a.address())
        .unwrap();
    assert_eq!(acc_b.balance, acc_a.balance);
    assert_eq!(acc_b.nonce, acc_a.nonce);

    let recv = Address::new("recv_two_node");
    let recv_a = {
        let ga = state_a.lock().expect("lock a");
        ga.chain.state().get_account(&recv).unwrap().clone()
    };
    let recv_b = inner_b.chain.state().get_account(&recv).unwrap();
    assert_eq!(recv_b.balance, recv_a.balance);
}
