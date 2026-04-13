//! Crash / cold-start / restart integration matrix for V2 persistence (see
//! `docs/design_notes/v2_persistence_restart.md`). Protocol semantics are unchanged; these tests
//! prove reload and fail-closed behavior of the **reference node** on-disk artifacts.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use ed25519_dalek::{Signer, SigningKey};
use node::block::Block;
use node::blockchain::Blockchain;
use node::crypto::Crypto;
use node::data_dir_bind::verify_or_create_binding;
use node::genesis::{Genesis, GenesisAllocation};
use node::mempool::Mempool;
use node::pending_tx_file::{append_pending_transaction, drain_pending_file};
use node::storage::{BlockStore, load_blockchain_from_disk};
use node::transaction::Transaction;
use node::types::Address;

fn tmp_dir(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let p = std::env::temp_dir().join(format!(
        "trilog_restart_v2_{label}_{}_{}",
        std::process::id(),
        nanos
    ));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

fn genesis_one_sender() -> (Genesis, SigningKey, Address) {
    let signing_key = SigningKey::from_bytes(&[120u8; 32]);
    let verifying_key = signing_key.verifying_key();
    let sender = Address::new(Crypto::address_from_public_key(&verifying_key.to_bytes()));
    let g = Genesis {
        allocations: vec![GenesisAllocation {
            address: sender.0.clone(),
            balance: 1_000_000,
        }],
    };
    (g, signing_key, sender)
}

fn sign_transfer(
    sk: &SigningKey,
    sender: &Address,
    receiver: &str,
    amount: u64,
    fee: u64,
    nonce: u64,
    ts: u64,
) -> Transaction {
    let verifying_key = sk.verifying_key();
    let mut tx = Transaction {
        sender: sender.clone(),
        receiver: Address::new(receiver),
        amount,
        fee,
        nonce,
        timestamp_unix: ts,
        public_key: verifying_key.to_bytes().to_vec(),
        signature: Vec::new(),
        tx_hash: String::new(),
    };
    let p = tx.unsigned_payload_bytes();
    tx.signature = sk.sign(&p).to_bytes().to_vec();
    tx.tx_hash = Crypto::hash_bytes(&p);
    tx
}

fn seal_block(sk: &SigningKey, sender: &Address, prev_hash: &str, height: u64, nonce: u64, ts: u64) -> Block {
    let tx = sign_transfer(sk, sender, "recv_rmx", 1, 1, nonce, ts);
    let mut b = Block {
        height,
        previous_hash: prev_hash.into(),
        timestamp_unix: ts.saturating_add(1),
        transactions: vec![tx],
        block_hash: String::new(),
    };
    b.block_hash = b.compute_block_hash();
    b
}

#[test]
fn cold_start_genesis_only_missing_chain_file_height_zero() {
    let dir = tmp_dir("cold_genesis");
    let (g, _, _) = genesis_one_sender();
    let gen_path = dir.join("genesis.toml");
    g.write_to_path(&gen_path).unwrap();
    verify_or_create_binding(&dir, &g).unwrap();

    let chain_path = dir.join("chain.blocks");
    assert!(!chain_path.exists());
    let (c, _) = load_blockchain_from_disk(&chain_path, &g).unwrap();
    assert_eq!(c.height(), 0);
    verify_or_create_binding(&dir, &g).unwrap();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn replay_equivalence_multiblock_reload_matches_built_chain() {
    let dir = tmp_dir("replay_multi");
    let (g, sk, sender) = genesis_one_sender();
    let gen_path = dir.join("genesis.toml");
    g.write_to_path(&gen_path).unwrap();
    verify_or_create_binding(&dir, &g).unwrap();
    let chain_path = dir.join("chain.blocks");

    let mut chain = Blockchain::from_genesis(&g).unwrap();
    let mut store = BlockStore::open_append(&chain_path).unwrap();
    let mut prev = "GENESIS_HASH".to_string();
    for i in 0u64..3 {
        let h = i + 1;
        let b = seal_block(&sk, &sender, &prev, h, i, 2_200_000 + i);
        chain.append_block(b.clone()).unwrap();
        store.append_block(&b).unwrap();
        prev = b.block_hash.clone();
    }
    drop(store);

    let (loaded, _) = load_blockchain_from_disk(&chain_path, &g).unwrap();
    assert_eq!(loaded.height(), 3);
    assert_eq!(loaded.blocks().last().unwrap().block_hash, chain.blocks().last().unwrap().block_hash);
    assert_eq!(
        loaded.state().accounts_sorted(),
        chain.state().accounts_sorted()
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn reload_idempotent_two_reads_same_tip_and_state() {
    let dir = tmp_dir("reload_idem");
    let (g, sk, sender) = genesis_one_sender();
    let gen_path = dir.join("genesis.toml");
    g.write_to_path(&gen_path).unwrap();
    verify_or_create_binding(&dir, &g).unwrap();
    let chain_path = dir.join("chain.blocks");

    let b1 = seal_block(&sk, &sender, "GENESIS_HASH", 1, 0, 2_300_000);
    let mut store = BlockStore::open_append(&chain_path).unwrap();
    store.append_block(&b1).unwrap();
    drop(store);

    let (a, _) = load_blockchain_from_disk(&chain_path, &g).unwrap();
    let (b, _) = load_blockchain_from_disk(&chain_path, &g).unwrap();
    assert_eq!(a.height(), b.height());
    assert_eq!(
        a.blocks().last().unwrap().block_hash,
        b.blocks().last().unwrap().block_hash
    );
    assert_eq!(a.state().accounts_sorted(), b.state().accounts_sorted());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn chain_blocks_valid_frame_then_truncated_tail_refuses_startup() {
    let dir = tmp_dir("chain_trunc_tail");
    let (g, sk, sender) = genesis_one_sender();
    let chain_path = dir.join("chain.blocks");

    let b1 = seal_block(&sk, &sender, "GENESIS_HASH", 1, 0, 2_400_000);
    let mut store = BlockStore::open_append(&chain_path).unwrap();
    store.append_block(&b1).unwrap();
    drop(store);

    let mut f = OpenOptions::new().append(true).open(&chain_path).unwrap();
    let bogus_len = 10_000u32;
    f.write_all(&bogus_len.to_be_bytes()).unwrap();
    f.write_all(&[0u8; 8]).unwrap();
    drop(f);

    match load_blockchain_from_disk(&chain_path, &g) {
        Ok(_) => panic!("expected corrupt chain.blocks tail to fail load"),
        Err(e) => {
            let s = e.to_string();
            assert!(
                s.contains("truncated") || s.contains("storage decode"),
                "unexpected err: {s}"
            );
        }
    }
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn pending_garbled_file_drain_fails_and_does_not_rewrite_file() {
    let dir = tmp_dir("pend_garbled");
    let path = dir.join("pending_tx.tril");
    let garbage = vec![0xffu8, 0xfeu8, 0x00u8, 0x10u8, 1, 2, 3];
    fs::write(&path, &garbage).unwrap();

    let mut pool = Mempool::new(100);
    let r = drain_pending_file(&path, &mut pool);
    assert!(r.is_err(), "expected drain to fail closed on parse");
    assert_eq!(fs::read(&path).unwrap(), garbage);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn pending_append_then_fresh_mempool_drain_survives_restart_semantics() {
    let dir = tmp_dir("pend_restart");
    let path = dir.join("pending_tx.tril");
    let (_, sk, sender) = genesis_one_sender();

    let tx = sign_transfer(&sk, &sender, "recv_pr", 5, 1, 0, 2_500_000);
    append_pending_transaction(&path, &tx).unwrap();

    let mut pool = Mempool::new(100);
    drain_pending_file(&path, &mut pool).unwrap();
    assert!(fs::read(&path).unwrap().is_empty());
    assert_eq!(pool.len(), 1);
    assert_eq!(pool.ordered_candidates(1)[0].tx_hash, tx.tx_hash);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn binding_verify_idempotent_after_chain_persisted() {
    let dir = tmp_dir("bind_idem");
    let (g, sk, sender) = genesis_one_sender();
    let gen_path = dir.join("genesis.toml");
    g.write_to_path(&gen_path).unwrap();
    verify_or_create_binding(&dir, &g).unwrap();

    let chain_path = dir.join("chain.blocks");
    let b1 = seal_block(&sk, &sender, "GENESIS_HASH", 1, 0, 2_600_000);
    let mut store = BlockStore::open_append(&chain_path).unwrap();
    store.append_block(&b1).unwrap();
    drop(store);

    verify_or_create_binding(&dir, &g).unwrap();
    let (c, _) = load_blockchain_from_disk(&chain_path, &g).unwrap();
    assert_eq!(c.height(), 1);
    verify_or_create_binding(&dir, &g).unwrap();
    let _ = fs::remove_dir_all(&dir);
}
