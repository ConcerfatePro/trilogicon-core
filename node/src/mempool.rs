use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use crate::errors::ProtocolError;
use crate::state::State;
use crate::transaction::Transaction;
use crate::types::Address;

/// In-memory queue of transactions waiting for block inclusion.
///
/// ## Submit-time policy (`try_submit`)
///
/// Accepts a transaction only if:
/// - [`Transaction::basic_validate`] passes (signature, binding, amounts, canonical hash),
/// - `tx_hash` is not already in the pool,
/// - **Mempool nonce rule:** for the sender, a different transaction must not already occupy the
///   same `nonce` slot (see [`ProtocolError::MempoolNonceConflict`]).
///
/// **Not** checked at submit time: current account **balance**, chain **nonce** vs what the node
/// will use when sealing, or whether the tx will eventually execute. A tx can sit in the mempool
/// until state or ordering makes it applicable.
///
/// ## Seal-time policy (`ordered_candidates_for_seal` / [`crate::blockchain::Blockchain::append_block_from_mempool`])
///
/// When building a block, the node walks submission order and **simulates**
/// [`State::apply_transaction`] on a **clone** of chain state; only txs that apply successfully in
/// that walk are included. Nonce gaps for one sender do not block other senders (see mempool tests).
///
/// Documented gate checklist: [`docs/v1_checkpoint.md`](../../docs/v1_checkpoint.md).
pub struct Mempool {
    capacity: usize,
    txs: HashMap<String, Transaction>,
    order: VecDeque<String>,
    by_sender: HashMap<Address, BTreeMap<u64, String>>,
}

impl Mempool {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            txs: HashMap::new(),
            order: VecDeque::new(),
            by_sender: HashMap::new(),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.txs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.txs.is_empty()
    }

    fn remove_tx_hash(&mut self, hash: &str) {
        if let Some(tx) = self.txs.remove(hash) {
            if let Some(m) = self.by_sender.get_mut(&tx.sender) {
                m.remove(&tx.nonce);
                if m.is_empty() {
                    self.by_sender.remove(&tx.sender);
                }
            }
        }
    }

    fn evict_oldest(&mut self) {
        while let Some(h) = self.order.pop_front() {
            if self.txs.contains_key(&h) {
                self.remove_tx_hash(&h);
                return;
            }
        }
    }

    /// Validate and append if there is capacity and the tx id is not already present.
    pub fn try_submit(&mut self, tx: Transaction) -> Result<(), ProtocolError> {
        tx.basic_validate()?;
        if self.txs.contains_key(&tx.tx_hash) {
            return Err(ProtocolError::DuplicateTransaction);
        }
        if let Some(m) = self.by_sender.get(&tx.sender) {
            if let Some(existing) = m.get(&tx.nonce) {
                if existing != &tx.tx_hash {
                    return Err(ProtocolError::MempoolNonceConflict);
                }
            }
        }
        while self.txs.len() >= self.capacity {
            self.evict_oldest();
            if self.txs.is_empty() {
                break;
            }
        }
        if self.txs.len() >= self.capacity {
            return Err(ProtocolError::MempoolFull);
        }
        let h = tx.tx_hash.clone();
        self.by_sender
            .entry(tx.sender.clone())
            .or_default()
            .insert(tx.nonce, h.clone());
        self.txs.insert(h.clone(), tx);
        self.order.push_back(h);
        Ok(())
    }

    /// FIFO view of currently held txs (does not simulate state; tests / diagnostics).
    pub fn ordered_candidates(&self, max: usize) -> Vec<Transaction> {
        self.order
            .iter()
            .filter_map(|h| self.txs.get(h).cloned())
            .take(max)
            .collect()
    }

    /// Walk submission order; include txs that apply successfully on a clone of `state`
    /// (respects per-sender nonce progression within one block).
    pub fn ordered_candidates_for_seal(&self, state: &State, max: usize) -> Vec<Transaction> {
        let mut s = state.clone();
        let mut out = Vec::new();
        for h in &self.order {
            if out.len() >= max {
                break;
            }
            let Some(tx) = self.txs.get(h) else {
                continue;
            };
            if s.apply_transaction(tx).is_ok() {
                out.push(tx.clone());
            }
        }
        out
    }

    /// Drop transactions that were committed in a block (or otherwise finalized).
    pub fn remove_by_tx_hashes<'a>(&mut self, hashes: impl IntoIterator<Item = &'a str>) {
        let remove: HashSet<&str> = hashes.into_iter().collect();
        for h in &remove {
            self.remove_tx_hash(h);
        }
        self.order.retain(|x| !remove.contains(x.as_str()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::Crypto;
    use crate::types::Address;
    use ed25519_dalek::{Signer, SigningKey};

    fn signed_tx(
        seed: u8,
        receiver: &str,
        amount: u64,
        fee: u64,
        nonce: u64,
        ts: u64,
    ) -> Transaction {
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

    #[test]
    fn try_submit_accepts_valid_tx() {
        let mut pool = Mempool::new(10);
        let tx = signed_tx(5, "recv_a", 1, 1, 0, 1_700_010_000);
        assert!(pool.try_submit(tx).is_ok());
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn try_submit_rejects_invalid_tx() {
        let mut pool = Mempool::new(10);
        let mut tx = signed_tx(5, "recv_a", 1, 1, 0, 1_700_010_001);
        tx.amount = 0;
        assert!(pool.try_submit(tx).is_err());
        assert!(pool.is_empty());
    }

    #[test]
    fn try_submit_rejects_duplicate_tx_hash() {
        let mut pool = Mempool::new(10);
        let tx = signed_tx(5, "recv_a", 1, 1, 0, 1_700_010_002);
        pool.try_submit(tx.clone()).unwrap();
        assert!(matches!(
            pool.try_submit(tx),
            Err(ProtocolError::DuplicateTransaction)
        ));
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn try_submit_rejects_conflicting_nonce() {
        let mut pool = Mempool::new(10);
        let a = signed_tx(5, "r1", 1, 1, 0, 1);
        let b = signed_tx(5, "r2", 1, 1, 0, 2);
        pool.try_submit(a).unwrap();
        assert!(matches!(
            pool.try_submit(b),
            Err(ProtocolError::MempoolNonceConflict)
        ));
    }

    #[test]
    fn eviction_drops_oldest() {
        let mut pool = Mempool::new(2);
        let a = signed_tx(5, "a", 1, 1, 0, 1);
        let b = signed_tx(6, "b", 1, 1, 0, 2);
        let c = signed_tx(7, "c", 1, 1, 0, 3);
        pool.try_submit(a.clone()).unwrap();
        pool.try_submit(b.clone()).unwrap();
        pool.try_submit(c.clone()).unwrap();
        assert_eq!(pool.len(), 2);
        assert!(pool.txs.contains_key(&b.tx_hash));
        assert!(pool.txs.contains_key(&c.tx_hash));
        assert!(!pool.txs.contains_key(&a.tx_hash));
    }

    #[test]
    fn ordered_candidates_respects_fifo() {
        let mut pool = Mempool::new(10);
        let a = signed_tx(5, "r", 1, 1, 0, 1);
        let b = signed_tx(6, "r", 1, 1, 0, 2);
        pool.try_submit(a.clone()).unwrap();
        pool.try_submit(b.clone()).unwrap();
        let c = pool.ordered_candidates(10);
        assert_eq!(c[0].tx_hash, a.tx_hash);
        assert_eq!(c[1].tx_hash, b.tx_hash);
    }

    #[test]
    fn remove_by_tx_hashes_drops_matches() {
        let mut pool = Mempool::new(10);
        let a = signed_tx(5, "r", 1, 1, 0, 1);
        let b = signed_tx(6, "r", 1, 1, 0, 2);
        pool.try_submit(a.clone()).unwrap();
        pool.try_submit(b.clone()).unwrap();
        pool.remove_by_tx_hashes([a.tx_hash.as_str()]);
        assert_eq!(pool.len(), 1);
        assert_eq!(pool.ordered_candidates(1)[0].tx_hash, b.tx_hash);
    }

    #[test]
    fn ordered_candidates_for_seal_skips_nonce_gap_but_takes_other_senders() {
        use crate::blockchain::Blockchain;

        let mut pool = Mempool::new(10);
        let mut chain = Blockchain::new();
        let sk = SigningKey::from_bytes(&[99u8; 32]);
        let sender = Address::new(Crypto::address_from_public_key(
            &sk.verifying_key().to_bytes(),
        ));
        chain.state_mut().create_account(sender.clone(), 100);

        let mut tx_wrong = signed_tx(99, "recv_x", 1, 1, 1, 1_700_020_000);
        tx_wrong.sender = sender.clone();
        let p = tx_wrong.unsigned_payload_bytes();
        tx_wrong.signature = sk.sign(&p).to_bytes().to_vec();
        tx_wrong.tx_hash = Crypto::hash_bytes(&p);

        let mut tx_ok = signed_tx(99, "recv_y", 1, 1, 0, 1_700_020_001);
        tx_ok.sender = sender.clone();
        let p2 = tx_ok.unsigned_payload_bytes();
        tx_ok.signature = sk.sign(&p2).to_bytes().to_vec();
        tx_ok.tx_hash = Crypto::hash_bytes(&p2);

        pool.try_submit(tx_wrong).unwrap();
        pool.try_submit(tx_ok).unwrap();

        let cand = pool.ordered_candidates_for_seal(chain.state(), 8);
        assert_eq!(cand.len(), 1);
        assert_eq!(cand[0].nonce, 0);
    }
}
