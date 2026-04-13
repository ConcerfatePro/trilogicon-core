//! Integration: TCP session handshake + get-blocks / block batch over the V2 wire format.

use std::net::TcpListener;
use std::thread;

use ed25519_dalek::{Signer, SigningKey};
use node::block::Block;
use node::crypto::Crypto;
use node::genesis::Genesis;
use node::network::{
    decode_session_payload, encode_session_payload, pull_blocks_from_peer, read_framed,
    wire_encode_blocks_response, write_framed, OutboundPeerTimeouts, OP_GET_BLOCKS,
    OP_SESSION_HELLO, OP_SESSION_HELLO_ACK,
};
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
