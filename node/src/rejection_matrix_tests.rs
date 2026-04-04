//! Explicit V1 rejection coverage: one named test per adversarial class.
//!
//! Pairing table: [`docs/protocol_invariants.md`](../docs/protocol_invariants.md#automated-rejection-coverage).

use ed25519_dalek::{Signer, SigningKey};

use crate::block::Block;
use crate::blockchain::Blockchain;
use crate::consensus::{ConsensusParams, validate_block_vs_local_time};
use crate::crypto::Crypto;
use crate::encoding::{decode_block, decode_transaction, encode_block, encode_transaction};
use crate::errors::ProtocolError;
use crate::state::State;
use crate::transaction::Transaction;
use crate::types::Address;

fn signed_tx(seed: u8, receiver: &str, amount: u64, fee: u64, nonce: u64, ts: u64) -> Transaction {
    let signing_key = SigningKey::from_bytes(&[seed; 32]);
    let verifying_key = signing_key.verifying_key();
    let mut tx = Transaction {
        sender: Address::new(Crypto::address_from_public_key(&verifying_key.to_bytes())),
        receiver: Address::new(receiver),
        amount,
        fee,
        nonce,
        timestamp_unix: ts,
        public_key: verifying_key.to_bytes().to_vec(),
        signature: Vec::new(),
        tx_hash: String::new(),
    };
    let payload = tx.unsigned_payload_bytes();
    tx.signature = signing_key.sign(&payload).to_bytes().to_vec();
    tx.tx_hash = Crypto::hash_bytes(&payload);
    tx
}

fn seal(mut block: Block) -> Block {
    block.block_hash = block.compute_block_hash();
    block
}

#[test]
fn v1_rejects_bad_signature() {
    let mut tx = signed_tx(1, "recv", 10, 1, 0, 1_800_001_000);
    tx.signature[0] ^= 0x01;
    assert!(matches!(
        tx.basic_validate(),
        Err(ProtocolError::SignatureInvalid)
    ));
}

#[test]
fn v1_rejects_wrong_nonce_when_expected_lower() {
    let mut state = State::new();
    let tx = signed_tx(2, "recv", 1, 1, 1, 1_800_002_000);
    state.create_account(tx.sender.clone(), 100);
    assert!(matches!(
        state.apply_transaction(&tx),
        Err(ProtocolError::InvalidNonce)
    ));
}

#[test]
fn v1_rejects_nonce_reuse_same_signed_transaction() {
    let mut state = State::new();
    let tx = signed_tx(3, "recv", 5, 1, 0, 1_800_003_000);
    state.create_account(tx.sender.clone(), 100);
    state.apply_transaction(&tx).unwrap();
    assert!(matches!(
        state.apply_transaction(&tx),
        Err(ProtocolError::InvalidNonce)
    ));
}

#[test]
fn v1_rejects_insufficient_balance() {
    let mut state = State::new();
    let tx = signed_tx(4, "recv", 100, 1, 0, 1_800_004_000);
    state.create_account(tx.sender.clone(), 50);
    assert!(matches!(
        state.apply_transaction(&tx),
        Err(ProtocolError::InsufficientBalance)
    ));
}

#[test]
fn v1_rejects_wrong_previous_hash_on_append() {
    let mut chain = Blockchain::new();
    let tx = signed_tx(5, "recv", 5, 1, 0, 1_800_005_000);
    chain.state_mut().create_account(tx.sender.clone(), 100);
    let block = seal(Block {
        height: 1,
        previous_hash: "NOT_GENESIS_TIP".into(),
        timestamp_unix: 1_800_005_001,
        transactions: vec![tx],
        block_hash: String::new(),
    });
    assert!(matches!(
        chain.append_block(block),
        Err(ProtocolError::InvalidBlock(_))
    ));
}

#[test]
fn v1_rejects_wrong_block_height_on_append() {
    let mut chain = Blockchain::new();
    let tx = signed_tx(6, "recv", 5, 1, 0, 1_800_006_000);
    chain.state_mut().create_account(tx.sender.clone(), 100);
    let block = seal(Block {
        height: 99,
        previous_hash: "GENESIS_HASH".into(),
        timestamp_unix: 1_800_006_001,
        transactions: vec![tx],
        block_hash: String::new(),
    });
    assert!(matches!(
        chain.append_block(block),
        Err(ProtocolError::InvalidBlock(_))
    ));
}

#[test]
fn v1_rejects_second_transaction_in_block_with_skipped_nonce() {
    let mut chain = Blockchain::new();
    let signing_key = SigningKey::from_bytes(&[7u8; 32]);
    let verifying_key = signing_key.verifying_key();
    let sender = Address::new(Crypto::address_from_public_key(&verifying_key.to_bytes()));

    chain.state_mut().create_account(sender.clone(), 100);

    let mut tx0 = Transaction {
        sender: sender.clone(),
        receiver: Address::new("r0"),
        amount: 1,
        fee: 1,
        nonce: 0,
        timestamp_unix: 1_800_007_000,
        public_key: verifying_key.to_bytes().to_vec(),
        signature: Vec::new(),
        tx_hash: String::new(),
    };
    let p0 = tx0.unsigned_payload_bytes();
    tx0.signature = signing_key.sign(&p0).to_bytes().to_vec();
    tx0.tx_hash = Crypto::hash_bytes(&p0);

    let mut tx_skip = Transaction {
        sender: sender.clone(),
        receiver: Address::new("r1"),
        amount: 1,
        fee: 1,
        nonce: 2,
        timestamp_unix: 1_800_007_001,
        public_key: verifying_key.to_bytes().to_vec(),
        signature: Vec::new(),
        tx_hash: String::new(),
    };
    let p1 = tx_skip.unsigned_payload_bytes();
    tx_skip.signature = signing_key.sign(&p1).to_bytes().to_vec();
    tx_skip.tx_hash = Crypto::hash_bytes(&p1);

    let block = seal(Block {
        height: 1,
        previous_hash: "GENESIS_HASH".into(),
        timestamp_unix: 1_800_007_002,
        transactions: vec![tx0, tx_skip],
        block_hash: String::new(),
    });

    assert!(matches!(
        chain.append_block(block),
        Err(ProtocolError::InvalidNonce)
    ));
    assert_eq!(chain.height(), 0);
}

#[test]
fn v1_rejects_malformed_transaction_encoding() {
    let bytes = encode_transaction(&signed_tx(8, "r", 1, 1, 0, 1));
    assert!(decode_transaction(&bytes[..bytes.len().saturating_sub(2)]).is_err());
}

#[test]
fn v1_rejects_malformed_block_encoding_truncated() {
    let tx = signed_tx(9, "r", 1, 1, 0, 1_800_009_000);
    let block = seal(Block {
        height: 1,
        previous_hash: "GENESIS_HASH".into(),
        timestamp_unix: 1_800_009_001,
        transactions: vec![tx],
        block_hash: String::new(),
    });
    let bytes = encode_block(&block);
    assert!(decode_block(&bytes[..bytes.len().saturating_sub(3)]).is_err());
}

#[test]
fn v1_rejects_malformed_block_encoding_trailing_garbage() {
    let tx = signed_tx(10, "r", 1, 1, 0, 1_800_010_000);
    let block = seal(Block {
        height: 1,
        previous_hash: "GENESIS_HASH".into(),
        timestamp_unix: 1_800_010_001,
        transactions: vec![tx],
        block_hash: String::new(),
    });
    let mut bytes = encode_block(&block);
    bytes.extend_from_slice(&[0xAB, 0xCD]);
    assert!(decode_block(&bytes).is_err());
}

#[test]
fn v1_rejects_block_timestamp_violating_min_interval_after_parent() {
    let mut chain = Blockchain::from_genesis_with_consensus(
        &crate::genesis::Genesis::empty(),
        ConsensusParams {
            min_block_interval_secs: 1_000,
            max_future_drift_secs: u64::MAX,
        },
    )
    .expect("genesis");
    let tx = signed_tx(11, "r", 1, 1, 0, 1_800_011_000);
    chain.state_mut().create_account(tx.sender.clone(), 50);
    let block = seal(Block {
        height: 1,
        previous_hash: "GENESIS_HASH".into(),
        timestamp_unix: 500,
        transactions: vec![tx],
        block_hash: String::new(),
    });
    assert!(matches!(
        chain.append_block(block),
        Err(ProtocolError::InvalidBlock(_))
    ));
}

#[test]
fn v1_rejects_block_timestamp_too_far_in_future_vs_local_time_on_network_path() {
    let mut chain = Blockchain::new();
    chain.consensus_params_mut().max_future_drift_secs = 60;
    let tx = signed_tx(12, "r", 1, 1, 0, 1_800_012_000);
    chain.state_mut().create_account(tx.sender.clone(), 50);
    let mut block = seal(Block {
        height: 1,
        previous_hash: "GENESIS_HASH".into(),
        timestamp_unix: 2_000_000_000,
        transactions: vec![tx],
        block_hash: String::new(),
    });
    block.block_hash = block.compute_block_hash();
    assert!(matches!(
        chain.try_append_network_block(block, 1_000_000_000),
        Err(ProtocolError::InvalidBlock(_))
    ));
}

#[test]
fn v1_consensus_local_time_rule_documented() {
    assert!(validate_block_vs_local_time(200, 100, 50).is_err());
    assert!(validate_block_vs_local_time(149, 100, 50).is_ok());
}
