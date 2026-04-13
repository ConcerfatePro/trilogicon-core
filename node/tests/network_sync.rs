//! Integration: TCP session handshake + get-blocks / block batch over the V2 wire format.

use std::fs;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

use ed25519_dalek::{Signer, SigningKey};
use node::block::Block;
use node::blockchain::Blockchain;
use node::crypto::Crypto;
use node::genesis::Genesis;
use node::network::{
    decode_session_payload, encode_session_payload, pull_blocks_from_peer, read_framed,
    wire_encode_blocks_response, write_framed, OutboundPeerTimeouts, OP_GET_BLOCKS,
    OP_SESSION_HELLO, OP_SESSION_HELLO_ACK,
};
use node::storage::BlockStore;
use node::transaction::Transaction;
use node::types::Address;

fn sample_block_height_1() -> Block {
    let signing_key = SigningKey::from_bytes(&[88u8; 32]);
    let verifying_key = signing_key.verifying_key();
    let mut tx = Transaction {
        sender: Address::new(Crypto::address_from_public_key(&verifying_key.to_bytes())),
        receiver: Address::new("recv_integration"),
        amount: 1,
        fee: 1,
        nonce: 0,
        timestamp_unix: 1_700_300_000,
        public_key: verifying_key.to_bytes().to_vec(),
        signature: Vec::new(),
        tx_hash: String::new(),
    };
    let p = tx.unsigned_payload_bytes();
    tx.signature = signing_key.sign(&p).to_bytes().to_vec();
    tx.tx_hash = Crypto::hash_bytes(&p);

    let mut b = Block {
        height: 1,
        previous_hash: "GENESIS_HASH".into(),
        timestamp_unix: 1_700_300_001,
        transactions: vec![tx],
        block_hash: String::new(),
    };
    b.block_hash = b.compute_block_hash();
    b
}

#[test]
fn tcp_pull_blocks_matches_mini_server() {
    let block = sample_block_height_1();
    let blocks = vec![block.clone()];
    let g = Genesis::empty();
    let g_srv = g.clone();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let hello = read_framed(&mut stream).unwrap();
        let (op, _, _, _) = decode_session_payload(&hello).unwrap();
        assert_eq!(op, OP_SESSION_HELLO);
        let ack = encode_session_payload(OP_SESSION_HELLO_ACK, &g_srv, u64::MAX).unwrap();
        write_framed(&mut stream, &ack).unwrap();

        let req = read_framed(&mut stream).unwrap();
        assert_eq!(req[0], OP_GET_BLOCKS);
        let resp = wire_encode_blocks_response(&blocks).unwrap();
        write_framed(&mut stream, &resp).unwrap();
    });

    let (pulled, _) = pull_blocks_from_peer(&addr, 1, &g, 0, &OutboundPeerTimeouts::default()).unwrap();
    server.join().unwrap();

    assert_eq!(pulled.len(), 1);
    assert_eq!(pulled[0].block_hash, block.block_hash);
}

#[test]
fn tcp_pull_blocks_with_v2_handshake() {
    let genesis = Genesis::empty();
    let wire = WireRuntimeConfig::from_genesis(&genesis, 1, true).unwrap();
    let chain = Blockchain::from_genesis(&genesis).unwrap();
    let (tip_h, tip_hash) = {
        let tip = chain.blocks().last().unwrap();
        (tip.height, tip.block_hash.clone())
    };

    let block = sample_block_height_1();
    let blocks = vec![block.clone()];

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    let wire_srv = wire.clone();
    let tip_hash_srv = tip_hash.clone();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let hello = read_framed(&mut stream).unwrap();
        assert_eq!(hello[0], node::network::OP_HELLO);
        let ok = wire_payload_hello_ok(&wire_srv, tip_h, &tip_hash_srv).unwrap();
        write_framed(&mut stream, &ok).unwrap();
        let req = read_framed(&mut stream).unwrap();
        assert_eq!(req[0], OP_GET_BLOCKS);
        let resp = wire_encode_blocks_response(&blocks).unwrap();
        write_framed(&mut stream, &resp).unwrap();
    });

    let pulled = pull_blocks_from_peer_inner(&addr, 1, Some(&wire), tip_h, &tip_hash).unwrap();
    server.join().unwrap();

    assert_eq!(pulled.len(), 1);
    assert_eq!(pulled[0].block_hash, block.block_hash);
}

#[test]
fn tcp_sync_from_peer_pulls_multiple_rounds() {
    let b1 = sample_block_height_1();
    let b2 = sample_block_height_2(&b1);
    let sender_addr = b1.transactions[0].sender.0.clone();
    let genesis = Genesis {
        allocations: vec![GenesisAllocation {
            address: sender_addr,
            balance: 1000,
        }],
    };

    let path = std::env::temp_dir().join(format!(
        "trilogicon_sync_multi_{}.blocks",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);

    let mut inner = NodeInner::for_tests(
        genesis.clone(),
        Blockchain::from_genesis(&genesis).unwrap(),
        Mempool::new(8),
        BlockStore::open_append(&path).unwrap(),
    )
    .unwrap();
    inner.wire.handshake_outbound = true;

    let wire_srv = inner.wire.clone();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let b1_m = b1.clone();
    let b2_m = b2.clone();
    let server = thread::spawn(move || {
        for _ in 0..3 {
            let (mut stream, _) = listener.accept().unwrap();
            let hello = read_framed(&mut stream).unwrap();
            assert_eq!(hello[0], node::network::OP_HELLO);
            let ok = wire_payload_hello_ok(&wire_srv, 0, "GENESIS_HASH").unwrap();
            write_framed(&mut stream, &ok).unwrap();
            let req = read_framed(&mut stream).unwrap();
            assert_eq!(req[0], OP_GET_BLOCKS);
            let start = u64::from_be_bytes(req[1..9].try_into().unwrap());
            let batch = if start == 1 {
                vec![b1_m.clone()]
            } else if start == 2 {
                vec![b2_m.clone()]
            } else {
                vec![]
            };
            let resp = wire_encode_blocks_response(&batch).unwrap();
            write_framed(&mut stream, &resp).unwrap();
        }
    });

    let n = sync_from_peer(&mut inner, &addr, 1_700_400_000).unwrap();
    server.join().unwrap();
    assert_eq!(n, 2);
    assert_eq!(inner.chain.height(), 2);
    assert_eq!(
        inner.chain.blocks().last().unwrap().block_hash,
        b2.block_hash
    );

    let _ = fs::remove_file(&path);
}

#[test]
fn tcp_announce_inv_want_delivers_block() {
    let b1 = sample_block_height_1();
    let sender_addr = b1.transactions[0].sender.0.clone();
    let genesis = Genesis {
        allocations: vec![GenesisAllocation {
            address: sender_addr,
            balance: 1000,
        }],
    };

    let path = std::env::temp_dir().join(format!("trilogicon_ann_{}.blocks", std::process::id()));
    let _ = fs::remove_file(&path);

    let mut inner = NodeInner::for_tests(
        genesis.clone(),
        Blockchain::from_genesis(&genesis).unwrap(),
        Mempool::new(8),
        BlockStore::open_append(&path).unwrap(),
    )
    .unwrap();
    inner.wire = WireRuntimeConfig::from_genesis(&genesis, 1, false)
        .unwrap()
        .with_inbound_policy(false, true)
        .with_gossip_extensions(false, true);

    let state = Arc::new(Mutex::new(inner));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let st = state.clone();
    let h = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let _ = node::network::peer_connection_loop(stream, st);
    });

    let wire = {
        let g = state.lock().unwrap();
        g.wire.clone()
    };
    node::network::push_block_to_peer_inner(&addr, &b1, Some(&wire), 1, &b1.block_hash).unwrap();

    h.join().expect("peer session");
    let g = state.lock().unwrap();
    assert_eq!(g.chain.height(), 1);
    assert_eq!(g.chain.blocks().last().unwrap().block_hash, b1.block_hash);

    let _ = fs::remove_file(&path);
}
