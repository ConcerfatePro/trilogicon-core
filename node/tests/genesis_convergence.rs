//! Multi-node convergence: same protocol genesis, no per-node `create_account` hacks.
//!
//! Two logical nodes share one [`Genesis`] (fixed test keys). Producer serves a block over TCP;
//! follower syncs and ends with identical height, tip hash, and full account table.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use node::block::Block;
use node::blockchain::Blockchain;
use node::genesis::{Genesis, GenesisAllocation};
use node::mempool::Mempool;
use node::network::{
    InboundPeerPolicy, NodeInner, spawn_incoming_loop, sync_from_peer, SyncWorkBudget,
};
use node::storage::BlockStore;
use node::transaction::Transaction;
use node::wallet::Wallet;

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!(
            "trilogicon_genconv_{}_{}_{}",
            label,
            std::process::id(),
            nanos
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }

    fn path(&self) -> &PathBuf {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn test_genesis(wa: &Wallet, wb: &Wallet) -> Genesis {
    Genesis {
        allocations: vec![
            GenesisAllocation {
                address: wa.address().0.clone(),
                balance: 10_000_000,
            },
            GenesisAllocation {
                address: wb.address().0.clone(),
                balance: 10_000_000,
            },
        ],
    }
}

fn seal_height_one(chain: &Blockchain, block_ts: u64, tx: Transaction) -> Block {
    let tip = chain.blocks().last().unwrap();
    let mut block = Block {
        height: tip.height + 1,
        previous_hash: tip.block_hash.clone(),
        timestamp_unix: block_ts,
        transactions: vec![tx],
        block_hash: String::new(),
    };
    block.block_hash = block.compute_block_hash();
    block
}

fn assert_ledgers_equal(a: &Blockchain, b: &Blockchain) {
    assert_eq!(a.height(), b.height(), "height");
    assert_eq!(a.len(), b.len(), "block count");
    assert_eq!(
        a.blocks().last().unwrap().block_hash,
        b.blocks().last().unwrap().block_hash,
        "tip hash"
    );
    assert_eq!(
        a.state().accounts_sorted(),
        b.state().accounts_sorted(),
        "accounts"
    );
}

#[test]
fn two_nodes_converge_without_manual_funding() {
    let wa = Wallet::from_seed(&[201u8; 32]);
    let wb = Wallet::from_seed(&[202u8; 32]);
    let genesis = test_genesis(&wa, &wb);

    let dir_a = TempDir::new("a");
    let dir_b = TempDir::new("b");
    let chain_path_a = dir_a.path().join("chain.blocks");
    let chain_path_b = dir_b.path().join("chain.blocks");
    let _ = std::fs::remove_file(&chain_path_a);
    let _ = std::fs::remove_file(&chain_path_b);

    // Producer: from_genesis only — no create_account.
    let mut chain_a = Blockchain::from_genesis(&genesis).unwrap();
    let tx = wa
        .sign_transfer(wb.address(), 100, 1, 0, 1_700_600_000)
        .unwrap();
    let block = seal_height_one(&chain_a, 1_700_600_001, tx);
    chain_a.append_block(block.clone()).unwrap();
    let mut store_a = BlockStore::open_append(&chain_path_a).unwrap();
    store_a.append_block(&block).unwrap();

    let state_a = Arc::new(Mutex::new(NodeInner {
        genesis: genesis.clone(),
        chain: chain_a,
        pool: Mempool::new(100),
        store: store_a,
    }));

    let (_jh, listen_addr) =
        spawn_incoming_loop("127.0.0.1:0", state_a.clone(), InboundPeerPolicy::default()).unwrap();

    // Follower: same genesis, empty blocks file — no manual funding of producer's key.
    let chain_b = Blockchain::from_genesis(&genesis).unwrap();
    let store_b = BlockStore::open_append(&chain_path_b).unwrap();
    let mut inner_b = NodeInner {
        genesis: genesis.clone(),
        chain: chain_b,
        pool: Mempool::new(100),
        store: store_b,
    };

    let out = sync_from_peer(&mut inner_b, &listen_addr, &SyncWorkBudget::default()).unwrap();
    assert_eq!(out.blocks_appended, 1);

    let ga = state_a.lock().unwrap();
    assert_ledgers_equal(&ga.chain, &inner_b.chain);
}
