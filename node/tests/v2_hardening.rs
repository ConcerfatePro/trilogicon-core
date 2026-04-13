//! V2 operational hardening: outbound write deadlines, sync budgets (soft byte cap), fresh sync time,
//! invalid-block ingress budget, committed-ledger mempool purge (see design notes under `docs/design_notes/`).

use std::cell::Cell;
use std::io::Read;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use ed25519_dalek::{Signer, SigningKey};
use node::block::Block;
use node::blockchain::Blockchain;
use node::crypto::Crypto;
use node::encoding::encode_block;
use node::genesis::{Genesis, GenesisAllocation};
use node::mempool::Mempool;
use node::network::{
    encode_session_payload, handshake_initiator, read_framed, serve_tcp_listener, sync_from_peer,
    sync_from_peer_with_clock, validate_sync_work_budget, wire_encode_blocks_response,
    write_framed, InboundPeerPolicy, InboundSlotPool, NodeInner, OutboundPeerTimeouts, OP_BLOCK,
    OP_SESSION_HELLO_ACK, SyncWorkBudget,
};
use node::storage::BlockStore;
use node::transaction::Transaction;
use node::types::Address;
use node::wallet::Wallet;

fn tmp_chain_path(label: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "trilog_v2hard_{}_{}_{}",
        label,
        std::process::id(),
        nanos
    ))
}

fn genesis_two_wallets() -> (Genesis, Wallet, Wallet) {
    let wa = Wallet::from_seed(&[61u8; 32]);
    let wb = Wallet::from_seed(&[62u8; 32]);
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

/// A peer that completes the session handshake then never reads will eventually stall a large outbound
/// `write_all`; with [`OutboundPeerTimeouts::write`] set, that surfaces as a timeout (local I/O policy).
#[test]
fn outbound_large_payload_write_times_out_when_peer_never_drains() {
    use std::io::Write;
    use node::network::{apply_outbound_stream_timeouts, handshake_initiator, tcp_connect_peer};

    let (g, _, _) = genesis_two_wallets();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let g_srv = g.clone();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = read_framed(&mut stream).unwrap();
        let ack = encode_session_payload(OP_SESSION_HELLO_ACK, &g_srv, 0).unwrap();
        write_framed(&mut stream, &ack).unwrap();
        thread::sleep(Duration::from_secs(8));
    });
    thread::sleep(Duration::from_millis(40));

    let timeouts = OutboundPeerTimeouts {
        read: Some(Duration::from_secs(5)),
        write: Some(Duration::from_millis(400)),
    };
    let mut stream = tcp_connect_peer(&addr).unwrap();
    apply_outbound_stream_timeouts(&mut stream, &timeouts);
    handshake_initiator(&mut stream, &g, 0).unwrap();

    // Many moderate writes fill the peer's receive buffer; with write_timeout set, the next write stalls.
    let chunk = [0xabu8; 16384];
    let mut last_err = None;
    for _ in 0..16_384 {
        match stream.write_all(&chunk) {
            Ok(()) => {}
            Err(e) => {
                last_err = Some(e);
                break;
            }
        }
    }
    let e = last_err.expect(
        "expected TCP send buffer to back up while peer does not read (increase chunk loop if flaky on your OS)",
    );
    let s = e.to_string();
    assert!(
        e.kind() == std::io::ErrorKind::TimedOut
            || e.kind() == std::io::ErrorKind::WouldBlock
            || s.contains("timed out")
            || s.contains("Resource temporarily unavailable"),
        "unexpected err: {e} (kind {:?})",
        e.kind()
    );
}

#[test]
fn sync_block_budget_stops_cleanly_then_second_call_finishes() {
    let (g, wa, wb) = genesis_two_wallets();
    let blocks = Arc::new(build_three_blocks(&g, &wa, &wb));
    let g2 = g.clone();
    let b2 = blocks.clone();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    thread::spawn(move || {
        loop {
            let r = listener.accept();
            if r.is_err() {
                break;
            }
            let (mut stream, _) = r.unwrap();
            let _ = read_framed(&mut stream).unwrap();
            let ack = encode_session_payload(OP_SESSION_HELLO_ACK, &g2, 0).unwrap();
            write_framed(&mut stream, &ack).unwrap();
            let req = read_framed(&mut stream).unwrap();
            assert_eq!(req[0], node::network::OP_GET_BLOCKS);
            let start = u64::from_be_bytes(req[1..9].try_into().unwrap());
            let slice: Vec<Block> = b2
                .iter()
                .filter(|b| b.height >= start)
                .take(2)
                .cloned()
                .collect();
            let resp = wire_encode_blocks_response(&slice).unwrap();
            write_framed(&mut stream, &resp).unwrap();
        }
    });
    thread::sleep(Duration::from_millis(40));

    let dir = tmp_chain_path("sync_budget");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("c.blocks");
    let _ = std::fs::remove_file(&path);
    let mut inner = NodeInner {
        genesis: g.clone(),
        chain: Blockchain::from_genesis(&g).unwrap(),
        pool: Mempool::new(10),
        store: BlockStore::open_append(&path).unwrap(),
    };

    let tight = SyncWorkBudget {
        max_rounds_per_call: 100,
        max_blocks_per_call: 2,
        max_wire_bytes_per_call: u64::MAX,
    };
    let o1 = sync_from_peer(&mut inner, &addr, &tight).unwrap();
    assert_eq!(o1.blocks_appended, 2);
    assert!(o1.stopped_due_to_budget);
    assert_eq!(inner.chain.height(), 2);

    let o2 = sync_from_peer(&mut inner, &addr, &SyncWorkBudget::default()).unwrap();
    assert_eq!(o2.blocks_appended, 1);
    assert!(!o2.stopped_due_to_budget);
    assert_eq!(inner.chain.height(), 3);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sync_soft_byte_budget_smaller_than_one_response_still_makes_progress() {
    let (g, wa, wb) = genesis_two_wallets();
    let blocks = Arc::new(build_three_blocks(&g, &wa, &wb));
    let g2 = g.clone();
    let b2 = blocks.clone();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    thread::spawn(move || {
        loop {
            let r = listener.accept();
            if r.is_err() {
                break;
            }
            let (mut stream, _) = r.unwrap();
            let _ = read_framed(&mut stream).unwrap();
            let ack = encode_session_payload(OP_SESSION_HELLO_ACK, &g2, 0).unwrap();
            write_framed(&mut stream, &ack).unwrap();
            let req = read_framed(&mut stream).unwrap();
            let start = u64::from_be_bytes(req[1..9].try_into().unwrap());
            let slice: Vec<Block> = b2
                .iter()
                .filter(|b| b.height >= start)
                .take(2)
                .cloned()
                .collect();
            let resp = wire_encode_blocks_response(&slice).unwrap();
            write_framed(&mut stream, &resp).unwrap();
        }
    });
    thread::sleep(Duration::from_millis(40));

    let dir = tmp_chain_path("sync_soft_byte");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("c.blocks");
    let _ = std::fs::remove_file(&path);
    let mut inner = NodeInner {
        genesis: g.clone(),
        chain: Blockchain::from_genesis(&g).unwrap(),
        pool: Mempool::new(10),
        store: BlockStore::open_append(&path).unwrap(),
    };

    let tiny_wire = SyncWorkBudget {
        max_rounds_per_call: 100,
        max_blocks_per_call: 100,
        max_wire_bytes_per_call: 64,
    };
    let o1 = sync_from_peer(&mut inner, &addr, &tiny_wire).unwrap();
    assert!(
        o1.blocks_appended >= 1,
        "expected at least one block appended despite byte budget << one response"
    );
    assert!(o1.stopped_due_to_budget);

    sync_from_peer(&mut inner, &addr, &SyncWorkBudget::default()).unwrap();
    assert_eq!(inner.chain.height(), 3);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sync_wire_byte_budget_exact_response_size_stops_before_second_pull_same_call() {
    let (g, wa, wb) = genesis_two_wallets();
    let blocks = build_three_blocks(&g, &wa, &wb);
    let slice = blocks[0..2].to_vec();
    let exact = wire_encode_blocks_response(&slice).unwrap().len() as u64;

    let blocks = Arc::new(blocks);
    let g2 = g.clone();
    let b2 = blocks.clone();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    thread::spawn(move || {
        loop {
            let r = listener.accept();
            if r.is_err() {
                break;
            }
            let (mut stream, _) = r.unwrap();
            let _ = read_framed(&mut stream).unwrap();
            let ack = encode_session_payload(OP_SESSION_HELLO_ACK, &g2, 0).unwrap();
            write_framed(&mut stream, &ack).unwrap();
            let req = read_framed(&mut stream).unwrap();
            let start = u64::from_be_bytes(req[1..9].try_into().unwrap());
            let batch: Vec<Block> = b2
                .iter()
                .filter(|b| b.height >= start)
                .take(2)
                .cloned()
                .collect();
            let resp = wire_encode_blocks_response(&batch).unwrap();
            write_framed(&mut stream, &resp).unwrap();
        }
    });
    thread::sleep(Duration::from_millis(40));

    let dir = tmp_chain_path("sync_exact_byte");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("c.blocks");
    let _ = std::fs::remove_file(&path);
    let mut inner = NodeInner {
        genesis: g.clone(),
        chain: Blockchain::from_genesis(&g).unwrap(),
        pool: Mempool::new(10),
        store: BlockStore::open_append(&path).unwrap(),
    };

    let budget = SyncWorkBudget {
        max_rounds_per_call: 100,
        max_blocks_per_call: 100,
        max_wire_bytes_per_call: exact,
    };
    let o1 = sync_from_peer(&mut inner, &addr, &budget).unwrap();
    assert_eq!(o1.blocks_appended, 2);
    assert!(o1.stopped_due_to_budget);
    assert_eq!(inner.chain.height(), 2);

    let o2 = sync_from_peer(&mut inner, &addr, &SyncWorkBudget::default()).unwrap();
    assert_eq!(o2.blocks_appended, 1);
    assert!(!o2.stopped_due_to_budget);
    assert_eq!(inner.chain.height(), 3);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sync_rejects_zero_max_wire_bytes_budget() {
    let bad = SyncWorkBudget {
        max_rounds_per_call: 10,
        max_blocks_per_call: 10,
        max_wire_bytes_per_call: 0,
    };
    let err = validate_sync_work_budget(&bad).unwrap_err();
    assert!(err.contains("max_wire_bytes"), "{err}");
}

#[test]
fn sync_from_peer_refreshes_clock_per_appended_block() {
    let (g, wa, wb) = genesis_two_wallets();
    let blocks = Arc::new(build_three_blocks(&g, &wa, &wb));
    let g2 = g.clone();
    let b2 = blocks.clone();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    thread::spawn(move || {
        for _ in 0..3 {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_framed(&mut stream).unwrap();
            let ack = encode_session_payload(OP_SESSION_HELLO_ACK, &g2, 0).unwrap();
            write_framed(&mut stream, &ack).unwrap();
            let req = read_framed(&mut stream).unwrap();
            let start = u64::from_be_bytes(req[1..9].try_into().unwrap());
            let slice: Vec<Block> = b2
                .iter()
                .filter(|b| b.height >= start)
                .take(2)
                .cloned()
                .collect();
            let resp = wire_encode_blocks_response(&slice).unwrap();
            write_framed(&mut stream, &resp).unwrap();
        }
    });
    thread::sleep(Duration::from_millis(40));

    let dir = tmp_chain_path("sync_clock");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("c.blocks");
    let _ = std::fs::remove_file(&path);
    let mut inner = NodeInner {
        genesis: g.clone(),
        chain: Blockchain::from_genesis(&g).unwrap(),
        pool: Mempool::new(10),
        store: BlockStore::open_append(&path).unwrap(),
    };

    let calls = Cell::new(0u32);
    let t = Cell::new(1_000_000u64);
    let out = sync_from_peer_with_clock(&mut inner, &addr, &SyncWorkBudget::default(), || {
        calls.set(calls.get().saturating_add(1));
        let v = t.get();
        t.set(v.saturating_add(1));
        v
    })
    .unwrap();
    assert_eq!(out.blocks_appended, 3);
    assert!(
        calls.get() >= 3,
        "expected fresh now per append (at least 3 clock samples), got {}",
        calls.get()
    );

    let _ = std::fs::remove_dir_all(&dir);
}

fn bad_next_block_wrong_prev() -> Block {
    let signing_key = SigningKey::from_bytes(&[99u8; 32]);
    let verifying_key = signing_key.verifying_key();
    let mut tx = Transaction {
        sender: Address::new(Crypto::address_from_public_key(&verifying_key.to_bytes())),
        receiver: Address::new("recv_badblk"),
        amount: 1,
        fee: 1,
        nonce: 0,
        timestamp_unix: 1_902_000_000,
        public_key: verifying_key.to_bytes().to_vec(),
        signature: Vec::new(),
        tx_hash: String::new(),
    };
    let p = tx.unsigned_payload_bytes();
    tx.signature = signing_key.sign(&p).to_bytes().to_vec();
    tx.tx_hash = Crypto::hash_bytes(&p);

    let mut b = Block {
        height: 1,
        previous_hash: "not_the_tip".into(),
        timestamp_unix: 1_902_000_001,
        transactions: vec![tx],
        block_hash: String::new(),
    };
    b.block_hash = b.compute_block_hash();
    b
}

#[test]
fn invalid_block_budget_disconnects_after_repeated_bad_next_blocks() {
    let path = tmp_chain_path("invblk");
    let _ = std::fs::remove_file(&path);
    let genesis = Genesis::empty();
    let chain = Blockchain::from_genesis(&genesis).unwrap();
    let store = BlockStore::open_append(&path).unwrap();
    let state = Arc::new(Mutex::new(NodeInner {
        genesis: genesis.clone(),
        chain,
        pool: Mempool::new(10),
        store,
    }));

    let policy = InboundPeerPolicy {
        max_concurrent_sessions: 4,
        idle_read_timeout: Duration::from_secs(30),
        write_timeout: None,
        max_protocol_errors_per_session: 100,
        max_app_frames_per_session: 50,
        max_invalid_network_blocks_per_session: 3,
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
    handshake_initiator(&mut c, &genesis, 0).unwrap();

    let bad = bad_next_block_wrong_prev();
    for _ in 0..3 {
        let mut msg = vec![OP_BLOCK];
        msg.extend_from_slice(&encode_block(&bad));
        write_framed(&mut c, &msg).unwrap();
    }

    c.set_read_timeout(Some(Duration::from_secs(3))).ok();
    let mut buf = [0u8; 1];
    let r = c.read(&mut buf);
    assert!(
        matches!(r, Ok(0) | Err(_)),
        "expected disconnect after invalid-block budget: {r:?}"
    );
    drop(jh);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn stale_low_height_blocks_do_not_consume_invalid_block_budget() {
    let path = tmp_chain_path("stale_benign");
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
        max_invalid_network_blocks_per_session: 3,
        max_app_frames_per_session: 80,
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
    for _ in 0..25 {
        let mut msg = vec![OP_BLOCK];
        msg.extend_from_slice(&encode_block(&stale));
        write_framed(&mut c, &msg).unwrap();
    }

    for _ in 0..3 {
        let mut msg = vec![OP_BLOCK];
        msg.extend_from_slice(&encode_block(&bad_next_block_wrong_prev()));
        write_framed(&mut c, &msg).unwrap();
    }

    c.set_read_timeout(Some(Duration::from_secs(2))).ok();
    let mut buf = [0u8; 1];
    let r = c.read(&mut buf);
    assert!(
        matches!(r, Ok(0) | Err(_)),
        "expected disconnect after strikes despite many benign stales: {r:?}"
    );
    drop(jh);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn mempool_purge_unblocks_seal_after_state_advances() {
    let wa = Wallet::from_seed(&[71u8; 32]);
    let wb = Wallet::from_seed(&[72u8; 32]);
    let g = Genesis {
        allocations: vec![
            GenesisAllocation {
                address: wa.address().0.clone(),
                balance: 100,
            },
            GenesisAllocation {
                address: wb.address().0.clone(),
                balance: 100,
            },
        ],
    };
    let mut chain = Blockchain::from_genesis(&g).unwrap();
    let mut pool = Mempool::new(20);
    let tx0 = wa.sign_transfer(wb.address(), 5, 1, 0, 2_000_000).unwrap();
    pool.try_submit(tx0).unwrap();
    chain
        .append_block_from_mempool(&mut pool, 8, 2_000_001)
        .unwrap();

    let stale = wa.sign_transfer(wb.address(), 1, 1, 0, 2_000_002).unwrap();
    pool.try_submit(stale).unwrap();
    let good = wa.sign_transfer(wb.address(), 1, 1, 1, 2_000_003).unwrap();
    pool.try_submit(good).unwrap();

    assert!(chain
        .append_block_from_mempool_pending_removal(&mut pool, 8, 2_000_004)
        .is_err());
    let n = pool.purge_nonviable_under_committed_state(chain.state());
    assert_eq!(n, 1);
    assert!(chain
        .append_block_from_mempool_pending_removal(&mut pool, 8, 2_000_005)
        .unwrap()
        .is_some());
    assert_eq!(chain.height(), 2);
}

/// Atomic FIFO-prefix seal tries the whole prefix in one block; a valid head + invalid second tx
/// leaves the queue stuck — `purge_nonviable_under_committed_state` does not skip mid-queue (deferred).
#[test]
fn fifo_prefix_seal_failure_with_valid_head_not_solved_by_committed_purge() {
    let wa = Wallet::from_seed(&[80u8; 32]);
    let wb = Wallet::from_seed(&[81u8; 32]);
    let g = Genesis {
        allocations: vec![
            GenesisAllocation {
                address: wa.address().0.clone(),
                balance: 100,
            },
            GenesisAllocation {
                address: wb.address().0.clone(),
                balance: 100,
            },
        ],
    };
    let mut chain = Blockchain::from_genesis(&g).unwrap();
    let mut pool = Mempool::new(20);
    let t0 = wa.sign_transfer(wb.address(), 40, 1, 0, 2_100_000).unwrap();
    let t1 = wa.sign_transfer(wb.address(), 70, 1, 1, 2_100_001).unwrap();
    pool.try_submit(t0).unwrap();
    pool.try_submit(t1).unwrap();

    assert!(chain
        .append_block_from_mempool_pending_removal(&mut pool, 8, 2_100_002)
        .is_err());
    assert_eq!(pool.purge_nonviable_under_committed_state(chain.state()), 0);
    assert!(chain
        .append_block_from_mempool_pending_removal(&mut pool, 8, 2_100_003)
        .is_err());
    assert_eq!(pool.len(), 2);
}
