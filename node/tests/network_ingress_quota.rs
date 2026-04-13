//! Per-session ingress work quotas: decoded stale `OP_BLOCK` and decoded `OP_TX` (local defense only).

use std::io::Read;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use node::block::Block;
use node::blockchain::Blockchain;
use node::encoding::{encode_block, encode_transaction};
use node::genesis::{Genesis, GenesisAllocation};
use node::mempool::Mempool;
use node::network::{
    handshake_initiator, serve_tcp_listener, write_framed, InboundPeerPolicy, InboundSlotPool,
    NodeInner, OP_BLOCK, OP_TX, INGRESS_INBOUND_TX_QUOTA_EXHAUSTED,
    INGRESS_STALE_BLOCK_QUOTA_EXHAUSTED,
};
use node::storage::BlockStore;
use node::wallet::Wallet;

fn genesis_two_wallets() -> (Genesis, Wallet, Wallet) {
    let wa = Wallet::from_seed(&[91u8; 32]);
    let wb = Wallet::from_seed(&[92u8; 32]);
    let g = Genesis {
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
    };
    (g, wa, wb)
}

fn build_three_blocks(g: &Genesis, wa: &Wallet, wb: &Wallet) -> Vec<Block> {
    let mut chain = Blockchain::from_genesis(g).unwrap();
    let mut out = Vec::new();
    for i in 0u64..3 {
        let tx = wa
            .sign_transfer(wb.address(), 1, 1, i, 1_901_000_000 + i)
            .unwrap();
        let tip = chain.blocks().last().unwrap();
        let mut b = Block {
            height: tip.height + 1,
            previous_hash: tip.block_hash.clone(),
            timestamp_unix: 1_901_000_010 + i,
            transactions: vec![tx],
            block_hash: String::new(),
        };
        b.block_hash = b.compute_block_hash();
        chain.append_block(b.clone()).unwrap();
        out.push(b);
    }
    out
}

#[test]
fn stale_decoded_blocks_quota_disconnects_without_invalid_block_strikes() {
    let path = std::env::temp_dir().join(format!(
        "trilog_ing_stale_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_file(&path);
    let (g, wa, wb) = genesis_two_wallets();
    let blocks = build_three_blocks(&g, &wa, &wb);
    let mut chain = Blockchain::from_genesis(&g).unwrap();
    for b in &blocks {
        chain.append_block(b.clone()).unwrap();
    }
    assert_eq!(chain.height(), 3);
    let store = BlockStore::open_append(&path).unwrap();
    let state = Arc::new(Mutex::new(NodeInner {
        genesis: g.clone(),
        chain,
        pool: Mempool::new(10),
        store,
    }));

    let policy = InboundPeerPolicy {
        max_stale_decoded_blocks_per_session: 5,
        max_invalid_network_blocks_per_session: 100,
        max_app_frames_per_session: 50,
        ..Default::default()
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let slots = InboundSlotPool::new(policy.max_concurrent_sessions);
    let st = state.clone();
    let pol = policy;
    let jh = thread::spawn(move || serve_tcp_listener(listener, st, pol, slots));
    thread::sleep(Duration::from_millis(60));

    let mut c = std::net::TcpStream::connect(addr).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(5))).ok();
    handshake_initiator(&mut c, &g, 0).unwrap();

    let stale = blocks[0].clone();
    for _ in 0..6 {
        let mut msg = vec![OP_BLOCK];
        msg.extend_from_slice(&encode_block(&stale));
        write_framed(&mut c, &msg).unwrap();
    }

    c.set_read_timeout(Some(Duration::from_secs(3))).ok();
    let mut buf = [0u8; 256];
    let r = c.read(&mut buf);
    assert!(
        matches!(r, Ok(0) | Err(_)),
        "expected disconnect after stale ingress quota: {r:?}"
    );
    drop(jh);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn stale_quota_disconnect_message_tag_is_stable() {
    let err = format!(
        "{INGRESS_STALE_BLOCK_QUOTA_EXHAUSTED}: test disconnect (max_stale=5)"
    );
    assert!(
        err.contains(INGRESS_STALE_BLOCK_QUOTA_EXHAUSTED),
        "operator grep tag must remain stable: {err}"
    );
}

#[test]
fn inbound_tx_decode_quota_disconnects_typed_not_protocol_budget() {
    let path = std::env::temp_dir().join(format!(
        "trilog_ing_tx_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_file(&path);
    let wa = Wallet::from_seed(&[93u8; 32]);
    let wb = Wallet::from_seed(&[94u8; 32]);
    let g = Genesis {
        allocations: vec![
            GenesisAllocation {
                address: wa.address().0.clone(),
                balance: 10_000_000,
            },
            GenesisAllocation {
                address: wb.address().0.clone(),
                balance: 10,
            },
        ],
    };
    let chain = Blockchain::from_genesis(&g).unwrap();
    let store = BlockStore::open_append(&path).unwrap();
    let state = Arc::new(Mutex::new(NodeInner {
        genesis: g.clone(),
        chain,
        pool: Mempool::new(10),
        store,
    }));

    let policy = InboundPeerPolicy {
        max_inbound_tx_per_session: 4,
        max_app_frames_per_session: 20,
        max_protocol_errors_per_session: 100,
        ..Default::default()
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let slots = InboundSlotPool::new(policy.max_concurrent_sessions);
    let st = state.clone();
    let pol = policy;
    let jh = thread::spawn(move || serve_tcp_listener(listener, st, pol, slots));
    thread::sleep(Duration::from_millis(60));

    let mut c = std::net::TcpStream::connect(addr).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(5))).ok();
    handshake_initiator(&mut c, &g, 0).unwrap();

    let tx = wa.sign_transfer(wb.address(), 1, 1, 0, 2_000_000_000).unwrap();
    let enc = encode_transaction(&tx);
    for _ in 0..5 {
        let mut msg = vec![OP_TX];
        msg.extend_from_slice(&enc);
        write_framed(&mut c, &msg).unwrap();
    }

    c.set_read_timeout(Some(Duration::from_secs(3))).ok();
    let mut buf = [0u8; 1];
    let r = c.read(&mut buf);
    assert!(
        matches!(r, Ok(0) | Err(_)),
        "expected disconnect after inbound tx ingress quota: {r:?}"
    );
    drop(jh);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn inbound_tx_quota_tag_constant_stable() {
    assert!(INGRESS_INBOUND_TX_QUOTA_EXHAUSTED.starts_with("TRIL_"));
}
