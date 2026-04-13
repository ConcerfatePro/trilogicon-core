//! V2 persistence / binding integration checks (library-level; mirrors `docs/design_notes/v2_persistence_restart.md`).
//! Broader cold-start / restart matrix: `restart_matrix_v2.rs`.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use ed25519_dalek::{Signer, SigningKey};
use node::block::Block;
use node::blockchain::Blockchain;
use node::crypto::Crypto;
use node::data_dir_bind::verify_or_create_binding;
use node::encoding::{encode_block, encode_transaction};
use node::genesis::{Genesis, GenesisAllocation};
use node::mempool::Mempool;
use node::pending_tx_file::{
    append_pending_transaction, drain_pending_file, parse_pending_file_bytes,
};
use node::storage::{BlockStore, load_blockchain_from_disk};
use node::transaction::Transaction;
use node::types::Address;

fn tmp_dir(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let p = std::env::temp_dir().join(format!(
        "trilogicon_persist_v2_{label}_{}_{}",
        std::process::id(),
        nanos
    ));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

fn one_account_genesis(name: &str) -> Genesis {
    Genesis {
        allocations: vec![GenesisAllocation {
            address: Address::new(name).0,
            balance: 1_000_000,
        }],
    }
}

#[test]
fn missing_chain_file_loads_genesis_only() {
    let dir = tmp_dir("no_chain");
    let g = one_account_genesis("alice_miss_chain");
    let chain_path = dir.join("chain.blocks");
    let c = load_blockchain_from_disk(&chain_path, &g).unwrap();
    assert_eq!(c.height(), 0);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn truncated_chain_file_fails_closed() {
    let dir = tmp_dir("trunc_chain");
    let g = Genesis::empty();
    let chain_path = dir.join("chain.blocks");
    fs::write(&chain_path, [0u8, 0u8, 0u8, 0x20]).unwrap();
    assert!(load_blockchain_from_disk(&chain_path, &g).is_err());
    let _ = fs::remove_dir_all(&dir);
}

/// Legacy `chain.blocks` (no V2 magic, no per-frame CRC) must still load after V2 storage changes.
#[test]
fn legacy_chain_blocks_file_without_magic_loads_and_replays() {
    let signing_key = SigningKey::from_bytes(&[98u8; 32]);
    let verifying_key = signing_key.verifying_key();
    let sender = Address::new(Crypto::address_from_public_key(&verifying_key.to_bytes()));

    let dir = tmp_dir("legacy_chain_load");
    let g = Genesis {
        allocations: vec![GenesisAllocation {
            address: sender.0.clone(),
            balance: 1_000_000,
        }],
    };

    let mut tx = Transaction {
        sender: sender.clone(),
        receiver: Address::new("recv_legacy_chain"),
        amount: 10,
        fee: 1,
        nonce: 0,
        timestamp_unix: 1_720_000_000,
        public_key: verifying_key.to_bytes().to_vec(),
        signature: Vec::new(),
        tx_hash: String::new(),
    };
    let pl = tx.unsigned_payload_bytes();
    tx.signature = signing_key.sign(&pl).to_bytes().to_vec();
    tx.tx_hash = Crypto::hash_bytes(&pl);

    let mut chain = Blockchain::from_genesis(&g).unwrap();
    let tip = chain.blocks().last().unwrap();
    let mut block = Block {
        height: 1,
        previous_hash: tip.block_hash.clone(),
        timestamp_unix: 1_720_000_010,
        transactions: vec![tx],
        block_hash: String::new(),
    };
    block.block_hash = block.compute_block_hash();
    chain.append_block(block.clone()).unwrap();

    let payload = encode_block(&block);
    let len = u32::try_from(payload.len()).unwrap();
    let mut raw = len.to_be_bytes().to_vec();
    raw.extend_from_slice(&payload);
    assert!(
        !raw.starts_with(b"TRILBC01"),
        "fixture must be legacy layout (no V2 magic)"
    );

    let chain_path = dir.join("chain.blocks");
    fs::write(&chain_path, &raw).unwrap();

    let loaded = load_blockchain_from_disk(&chain_path, &g).unwrap();
    assert_eq!(loaded.height(), 1);
    assert_eq!(
        loaded.state().get_account(&sender).unwrap().nonce,
        1,
        "replayed legacy file should match in-memory seal"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn genesis_bind_mismatch_refuses() {
    let dir = tmp_dir("bind_mismatch");
    let g1 = one_account_genesis("alice_bind_mis");
    let gen_path = dir.join("genesis.toml");
    g1.write_to_path(&gen_path).unwrap();
    verify_or_create_binding(&dir, &g1).unwrap();

    let g2 = one_account_genesis("bob_bind_mis");
    g2.write_to_path(&gen_path).unwrap();
    assert!(verify_or_create_binding(&dir, &g2).is_err());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn restart_reload_preserves_height_after_block_persisted() {
    let signing_key = SigningKey::from_bytes(&[44u8; 32]);
    let verifying_key = signing_key.verifying_key();
    let sender = Address::new(Crypto::address_from_public_key(&verifying_key.to_bytes()));

    let dir = tmp_dir("restart_h");
    let g = Genesis {
        allocations: vec![GenesisAllocation {
            address: sender.0.clone(),
            balance: 1_000_000,
        }],
    };
    let gen_path = dir.join("genesis.toml");
    g.write_to_path(&gen_path).unwrap();
    verify_or_create_binding(&dir, &g).unwrap();
    let chain_path = dir.join("chain.blocks");

    let mut chain = Blockchain::from_genesis(&g).unwrap();
    let mut tx = Transaction {
        sender: sender.clone(),
        receiver: Address::new("recv_restart"),
        amount: 10,
        fee: 1,
        nonce: 0,
        timestamp_unix: 1_702_000_000,
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
        timestamp_unix: 1_702_000_001,
        transactions: vec![tx],
        block_hash: String::new(),
    };
    b.block_hash = b.compute_block_hash();
    chain.append_block(b.clone()).unwrap();

    let mut store = BlockStore::open_append(&chain_path).unwrap();
    store.append_block(&b).unwrap();
    drop(store);

    let loaded = load_blockchain_from_disk(&chain_path, &g).unwrap();
    assert_eq!(loaded.height(), 1);
    assert_eq!(
        loaded.state().accounts_sorted(),
        chain.state().accounts_sorted()
    );
    verify_or_create_binding(&dir, &g).unwrap();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn pending_tx_survives_second_drain_after_mempool_has_room() {
    let dir = tmp_dir("pend_survive");
    let path = dir.join("pending_tx.tril");

    let signing_key = SigningKey::from_bytes(&[55u8; 32]);
    let verifying_key = signing_key.verifying_key();
    let mut t1 = Transaction {
        sender: Address::new(Crypto::address_from_public_key(&verifying_key.to_bytes())),
        receiver: Address::new("r1"),
        amount: 1,
        fee: 1,
        nonce: 0,
        timestamp_unix: 1_703_000_000,
        public_key: verifying_key.to_bytes().to_vec(),
        signature: Vec::new(),
        tx_hash: String::new(),
    };
    let pl = t1.unsigned_payload_bytes();
    t1.signature = signing_key.sign(&pl).to_bytes().to_vec();
    t1.tx_hash = Crypto::hash_bytes(&pl);

    let payload = encode_transaction(&t1);
    let len = u32::try_from(payload.len()).unwrap();
    let mut buf = len.to_be_bytes().to_vec();
    buf.extend_from_slice(&payload);
    fs::write(&path, &buf).unwrap();

    let mut pool = Mempool::new(1);
    let filler_sk = SigningKey::from_bytes(&[56u8; 32]);
    let filler_vk = filler_sk.verifying_key();
    let mut filler = Transaction {
        sender: Address::new(Crypto::address_from_public_key(&filler_vk.to_bytes())),
        receiver: Address::new("fill"),
        amount: 1,
        fee: 1,
        nonce: 0,
        timestamp_unix: 1_703_000_001,
        public_key: filler_vk.to_bytes().to_vec(),
        signature: Vec::new(),
        tx_hash: String::new(),
    };
    let fp = filler.unsigned_payload_bytes();
    filler.signature = filler_sk.sign(&fp).to_bytes().to_vec();
    filler.tx_hash = Crypto::hash_bytes(&fp);
    pool.try_submit(filler).unwrap();

    drain_pending_file(&path, &mut pool).unwrap();
    let rest = fs::read(&path).unwrap();
    assert!(!rest.is_empty());
    let parsed = parse_pending_file_bytes(&rest).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].tx_hash, t1.tx_hash);

    let mut pool2 = Mempool::new(100);
    drain_pending_file(&path, &mut pool2).unwrap();
    assert!(fs::read(&path).unwrap().is_empty());
    assert_eq!(pool2.len(), 1);
    assert_eq!(pool2.ordered_candidates(1)[0].tx_hash, t1.tx_hash);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn pending_concurrent_public_appends_integration_parse_clean() {
    let dir = tmp_dir("pend_pub_conc");
    let path = Arc::new(dir.join("pending_tx.tril"));

    let mut handles = Vec::new();
    for tid in 0..4usize {
        let p = path.clone();
        handles.push(thread::spawn(move || {
            let sk = SigningKey::from_bytes(&[(200 + tid) as u8; 32]);
            let vk = sk.verifying_key();
            for j in 0..15usize {
                let mut tx = Transaction {
                    sender: Address::new(Crypto::address_from_public_key(&vk.to_bytes())),
                    receiver: Address::new(&format!("ix_{tid}_{j}")),
                    amount: 1,
                    fee: 1,
                    nonce: j as u64,
                    timestamp_unix: 1_710_000_000 + (tid * 100 + j) as u64,
                    public_key: vk.to_bytes().to_vec(),
                    signature: Vec::new(),
                    tx_hash: String::new(),
                };
                let pl = tx.unsigned_payload_bytes();
                tx.signature = sk.sign(&pl).to_bytes().to_vec();
                tx.tx_hash = Crypto::hash_bytes(&pl);
                append_pending_transaction(&*p, &tx).unwrap();
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }

    let raw = fs::read(path.as_ref()).unwrap();
    let parsed = parse_pending_file_bytes(&raw).unwrap();
    assert_eq!(parsed.len(), 60);
    let _ = fs::remove_dir_all(&dir);
}

/// Cold mempool + on-disk pending queue (same as a new `run` process) must still admit txs.
#[test]
fn pending_tx_survives_empty_mempool_like_process_restart() {
    let dir = tmp_dir("pend_restart_sim");
    let path = dir.join("pending_tx.tril");

    let signing_key = SigningKey::from_bytes(&[58u8; 32]);
    let verifying_key = signing_key.verifying_key();
    let mut t = Transaction {
        sender: Address::new(Crypto::address_from_public_key(&verifying_key.to_bytes())),
        receiver: Address::new("r_restart_sim"),
        amount: 1,
        fee: 1,
        nonce: 0,
        timestamp_unix: 1_705_000_000,
        public_key: verifying_key.to_bytes().to_vec(),
        signature: Vec::new(),
        tx_hash: String::new(),
    };
    let pl = t.unsigned_payload_bytes();
    t.signature = signing_key.sign(&pl).to_bytes().to_vec();
    t.tx_hash = Crypto::hash_bytes(&pl);

    append_pending_transaction(&path, &t).unwrap();
    let mut fresh_pool = Mempool::new(100);
    drain_pending_file(&path, &mut fresh_pool).unwrap();
    assert!(fs::read(&path).unwrap().is_empty());
    assert_eq!(fresh_pool.len(), 1);
    assert_eq!(fresh_pool.ordered_candidates(1)[0].tx_hash, t.tx_hash);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn genesis_concurrent_different_binding_integration_one_wins() {
    let dir = tmp_dir("bind_int_diff");
    let g1 = one_account_genesis("ix_alice");
    let g2 = one_account_genesis("ix_bob");
    assert_ne!(
        g1.state_commitment_hex().unwrap(),
        g2.state_commitment_hex().unwrap()
    );
    let dir = Arc::new(dir);
    let g1 = Arc::new(g1);
    let g2 = Arc::new(g2);
    let h1 = {
        let d = dir.clone();
        let g = g1.clone();
        thread::spawn(move || verify_or_create_binding(d.as_ref(), g.as_ref()))
    };
    let h2 = {
        let d = dir.clone();
        let g = g2.clone();
        thread::spawn(move || verify_or_create_binding(d.as_ref(), g.as_ref()))
    };
    let r1 = h1.join().unwrap();
    let r2 = h2.join().unwrap();
    assert!(r1.is_ok() ^ r2.is_ok());
    let winner = if r1.is_ok() { g1.as_ref() } else { g2.as_ref() };
    verify_or_create_binding(dir.as_ref(), winner).unwrap();
    assert!(
        verify_or_create_binding(dir.as_ref(), if r1.is_ok() {
            g2.as_ref()
        } else {
            g1.as_ref()
        })
        .is_err()
    );
    let _ = fs::remove_dir_all(&dir.as_ref());
}

#[test]
fn duplicate_pending_frames_dedup_via_mempool_second_drain() {
    let dir = tmp_dir("pend_dup");
    let path = dir.join("pending_tx.tril");

    let signing_key = SigningKey::from_bytes(&[57u8; 32]);
    let verifying_key = signing_key.verifying_key();
    let mut t = Transaction {
        sender: Address::new(Crypto::address_from_public_key(&verifying_key.to_bytes())),
        receiver: Address::new("rd"),
        amount: 1,
        fee: 1,
        nonce: 0,
        timestamp_unix: 1_704_000_000,
        public_key: verifying_key.to_bytes().to_vec(),
        signature: Vec::new(),
        tx_hash: String::new(),
    };
    let pl = t.unsigned_payload_bytes();
    t.signature = signing_key.sign(&pl).to_bytes().to_vec();
    t.tx_hash = Crypto::hash_bytes(&pl);

    let mut buf = Vec::new();
    for _ in 0..2 {
        let payload = encode_transaction(&t);
        let len = u32::try_from(payload.len()).unwrap();
        buf.extend_from_slice(&len.to_be_bytes());
        buf.extend_from_slice(&payload);
    }
    fs::write(&path, &buf).unwrap();

    let mut pool = Mempool::new(100);
    drain_pending_file(&path, &mut pool).unwrap();
    assert_eq!(pool.len(), 1);
    assert!(fs::read(&path).unwrap().is_empty());

    fs::write(&path, &buf).unwrap();
    drain_pending_file(&path, &mut pool).unwrap();
    assert_eq!(pool.len(), 1);

    let _ = fs::remove_dir_all(&dir);
}
