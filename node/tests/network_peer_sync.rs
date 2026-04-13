//! V2 peer session + linear sync hardening (handshake, batch caps, gap/stale handling).

use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

use node::block::Block;
use node::blockchain::Blockchain;
use node::genesis::{Genesis, GenesisAllocation};
use node::mempool::Mempool;
use node::network::{
    decode_session_payload, encode_session_payload, pull_blocks_from_peer, read_framed,
    sync_from_peer, validate_linear_sync_batch, wire_encode_blocks_response, write_framed,
    NodeInner, OutboundPeerTimeouts, OP_BLOCKS, OP_GET_BLOCKS, OP_SESSION_HELLO,
    OP_SESSION_HELLO_ACK, SyncWorkBudget, TRIL_WIRE_PROTOCOL_VERSION,
};
use node::storage::BlockStore;
use node::wallet::Wallet;

fn genesis_two_wallets() -> (Genesis, Wallet, Wallet) {
    let wa = Wallet::from_seed(&[101u8; 32]);
    let wb = Wallet::from_seed(&[102u8; 32]);
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

fn build_linear_three_blocks(g: &Genesis, wa: &Wallet, wb: &Wallet) -> Vec<Block> {
    let mut chain = Blockchain::from_genesis(g).unwrap();
    let mut out = Vec::new();
    for i in 0u64..3 {
        let tx = wa
            .sign_transfer(wb.address(), 1, 1, i, 1_801_000_000 + i)
            .unwrap();
        let tip = chain.blocks().last().unwrap();
        let mut b = Block {
            height: tip.height + 1,
            previous_hash: tip.block_hash.clone(),
            timestamp_unix: 1_801_000_010 + i,
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
fn handshake_rejects_wrong_genesis_commitment() {
    let (g_a, _, _) = genesis_two_wallets();
    let solo = Wallet::from_seed(&[55u8; 32]);
    let g_b = Genesis {
        allocations: vec![GenesisAllocation {
            address: solo.address().0.clone(),
            balance: 1_000,
        }],
    };
    assert_ne!(
        g_a.state_commitment_hex().unwrap(),
        g_b.state_commitment_hex().unwrap()
    );

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let hello = read_framed(&mut stream).unwrap();
        let (_, _, _, _) = decode_session_payload(&hello).unwrap();
        let ack = encode_session_payload(OP_SESSION_HELLO_ACK, &g_b, 0).unwrap();
        write_framed(&mut stream, &ack).unwrap();
    });
    thread::sleep(std::time::Duration::from_millis(30));

    let err = pull_blocks_from_peer(&addr, 1, &g_a, 0, &OutboundPeerTimeouts::default()).unwrap_err();
    assert!(
        err.contains("mismatch") || err.contains("commitment"),
        "unexpected err: {err}"
    );
}

#[test]
fn handshake_rejects_peer_ack_with_wrong_wire_version() {
    let (g, _, _) = genesis_two_wallets();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let g2 = g.clone();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let hello = read_framed(&mut stream).unwrap();
        let (_, _, _, _) = decode_session_payload(&hello).unwrap();
        let mut ack = encode_session_payload(OP_SESSION_HELLO_ACK, &g2, 0).unwrap();
        ack[1] = 0;
        ack[2] = 1;
        write_framed(&mut stream, &ack).unwrap();
    });
    thread::sleep(std::time::Duration::from_millis(30));

    let err = pull_blocks_from_peer(&addr, 1, &g, 0, &OutboundPeerTimeouts::default()).unwrap_err();
    assert!(
        err.contains("wire version") || err.contains("version"),
        "unexpected err: {err}"
    );
}

#[test]
fn get_blocks_uses_local_start_height_advisory_height_does_not_shift_request() {
    let (g, wa, wb) = genesis_two_wallets();
    let blocks = Arc::new(build_linear_three_blocks(&g, &wa, &wb));
    let recorded = Arc::new(Mutex::new(None::<u64>));
    let rec_clone = recorded.clone();
    let blocks_s = blocks.clone();
    let g2 = g.clone();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let hello = read_framed(&mut stream).unwrap();
        let (op, ver, _, _) = decode_session_payload(&hello).unwrap();
        assert_eq!(op, OP_SESSION_HELLO);
        assert_eq!(ver, TRIL_WIRE_PROTOCOL_VERSION);
        let ack = encode_session_payload(OP_SESSION_HELLO_ACK, &g2, u64::MAX).unwrap();
        write_framed(&mut stream, &ack).unwrap();

        let req = read_framed(&mut stream).unwrap();
        assert_eq!(req[0], OP_GET_BLOCKS);
        let start = u64::from_be_bytes(req[1..9].try_into().unwrap());
        *rec_clone.lock().unwrap() = Some(start);
        let slice: Vec<Block> = blocks_s
            .iter()
            .filter(|b| b.height >= start)
            .take(2)
            .cloned()
            .collect();
        let resp = wire_encode_blocks_response(&slice).unwrap();
        write_framed(&mut stream, &resp).unwrap();
    });
    thread::sleep(std::time::Duration::from_millis(30));

    let _ = pull_blocks_from_peer(&addr, 1, &g, 0, &OutboundPeerTimeouts::default()).unwrap();
    assert_eq!(
        *recorded.lock().unwrap(),
        Some(1),
        "GET_BLOCKS must use explicit start_height, not peer advisory from handshake"
    );
}

#[test]
fn sync_multi_round_when_server_caps_batch_size() {
    let (g, wa, wb) = genesis_two_wallets();
    let blocks = Arc::new(build_linear_three_blocks(&g, &wa, &wb));
    let g2 = g.clone();
    let b2 = blocks.clone();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    thread::spawn(move || {
        for _ in 0..3 {
            let (mut stream, _) = listener.accept().unwrap();
            let hello = read_framed(&mut stream).unwrap();
            assert_eq!(decode_session_payload(&hello).unwrap().0, OP_SESSION_HELLO);
            let ack = encode_session_payload(OP_SESSION_HELLO_ACK, &g2, 0).unwrap();
            write_framed(&mut stream, &ack).unwrap();

            let req = read_framed(&mut stream).unwrap();
            assert_eq!(req[0], OP_GET_BLOCKS);
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
    thread::sleep(std::time::Duration::from_millis(30));

    let dir = tempfile_for_chain();
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("c.blocks");
    let _ = std::fs::remove_file(&path);
    let mut inner = NodeInner {
        genesis: g.clone(),
        chain: Blockchain::from_genesis(&g).unwrap(),
        pool: Mempool::new(10),
        store: BlockStore::open_append(&path).unwrap(),
        seen_tx: node::seen::SeenCache::new(50_000),
        seen_block: node::seen::SeenCache::new(50_000),
        peer_book: node::peer_book::PeerBook::default(),
    };
    let out = sync_from_peer(&mut inner, &addr, &SyncWorkBudget::default()).unwrap();
    assert_eq!(out.blocks_appended, 3);
    assert_eq!(inner.chain.height(), 3);
}

#[test]
fn peer_sync_mempool_hygiene_drops_stale_nonce_after_catch_up() {
    let (g, wa, wb) = genesis_two_wallets();
    let blocks = build_linear_three_blocks(&g, &wa, &wb);
    let b_first = blocks[0].clone();
    let g2 = g.clone();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    thread::spawn(move || {
        for round in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let hello = read_framed(&mut stream).unwrap();
            assert_eq!(decode_session_payload(&hello).unwrap().0, OP_SESSION_HELLO);
            let ack = encode_session_payload(OP_SESSION_HELLO_ACK, &g2, 0).unwrap();
            write_framed(&mut stream, &ack).unwrap();
            let req = read_framed(&mut stream).unwrap();
            assert_eq!(req[0], OP_GET_BLOCKS);
            let slice: Vec<Block> = if round == 0 {
                vec![b_first.clone()]
            } else {
                vec![]
            };
            let resp = wire_encode_blocks_response(&slice).unwrap();
            write_framed(&mut stream, &resp).unwrap();
        }
    });
    thread::sleep(std::time::Duration::from_millis(30));

    let dir = tempfile_for_chain();
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("c.blocks");
    let _ = std::fs::remove_file(&path);
    let stale = wa
        .sign_transfer(wb.address(), 99, 1, 0, 9_000_000_000)
        .unwrap();
    let mut inner = NodeInner {
        genesis: g.clone(),
        chain: Blockchain::from_genesis(&g).unwrap(),
        pool: Mempool::new(20),
        store: BlockStore::open_append(&path).unwrap(),
        seen_tx: node::seen::SeenCache::new(50_000),
        seen_block: node::seen::SeenCache::new(50_000),
        peer_book: node::peer_book::PeerBook::default(),
    };
    inner.pool.try_submit(stale).unwrap();
    assert_eq!(inner.pool.len(), 1);

    let out = sync_from_peer(&mut inner, &addr, &SyncWorkBudget::default()).unwrap();
    assert_eq!(out.blocks_appended, 1);
    assert_eq!(inner.chain.height(), 1);
    assert!(
        inner.pool.is_empty(),
        "stale nonce-0 tx must be dropped by post-sync mempool hygiene"
    );
}

#[test]
fn validate_linear_batch_rejects_stale_duplicate_height() {
    let (g, wa, wb) = genesis_two_wallets();
    let blocks = build_linear_three_blocks(&g, &wa, &wb);
    let mut chain = Blockchain::from_genesis(&g).unwrap();
    chain.append_block(blocks[0].clone()).unwrap();
    let err = validate_linear_sync_batch(&chain, &[blocks[0].clone()]).unwrap_err();
    assert!(
        err.contains("not linear") || err.contains("expected height"),
        "{err}"
    );
}

#[test]
fn validate_linear_batch_rejects_internal_gap() {
    let (g, wa, wb) = genesis_two_wallets();
    let blocks = build_linear_three_blocks(&g, &wa, &wb);
    let chain = Blockchain::from_genesis(&g).unwrap();
    let err = validate_linear_sync_batch(&chain, &[blocks[0].clone(), blocks[2].clone()]).unwrap_err();
    assert!(err.contains("not linear"), "{err}");
}

fn tempfile_for_chain() -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "trilog_peer_sync_{}_{}",
        std::process::id(),
        nanos
    ))
}

#[test]
fn oversized_op_blocks_count_rejected_before_decode() {
    use node::network::MAX_BLOCKS_PER_BATCH;

    let mut body = vec![OP_BLOCKS];
    let over = MAX_BLOCKS_PER_BATCH.saturating_add(1);
    body.extend_from_slice(&over.to_be_bytes());
    let err = node::network::wire_decode_blocks_response(&body).unwrap_err();
    assert!(err.contains("exceeds max"), "{err}");
}
