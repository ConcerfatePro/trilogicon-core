use std::collections::VecDeque;

use crate::errors::ProtocolError;
use crate::transaction::Transaction;

/// In-memory queue of transactions waiting for block inclusion.
///
/// V1 policy: submissions must pass `Transaction::basic_validate`. State checks
/// (balance, nonce) are **not** applied here; they run again at block apply time.
/// Inclusion order is FIFO among successfully submitted transactions.
pub struct Mempool {
    capacity: usize,
    txs: VecDeque<Transaction>,
}

impl Mempool {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            txs: VecDeque::new(),
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

    /// Validate and append if there is capacity and the tx id is not already present.
    pub fn try_submit(&mut self, tx: Transaction) -> Result<(), ProtocolError> {
        tx.basic_validate()?;
        if self.txs.iter().any(|t| t.tx_hash == tx.tx_hash) {
            return Err(ProtocolError::DuplicateTransaction);
        }
        if self.txs.len() >= self.capacity {
            return Err(ProtocolError::MempoolFull);
        }
        self.txs.push_back(tx);
        Ok(())
    }

    /// Ordered view (FIFO) without removing entries, for block building.
    pub fn ordered_candidates(&self, max: usize) -> Vec<Transaction> {
        self.txs.iter().take(max).cloned().collect()
    }

    /// Drop transactions that were committed in a block (or otherwise finalized).
    pub fn remove_by_tx_hashes<'a>(&mut self, hashes: impl IntoIterator<Item = &'a str>) {
        let remove: std::collections::HashSet<&str> = hashes.into_iter().collect();
        self.txs
            .retain(|t| !remove.contains(t.tx_hash.as_str()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::Crypto;
    use crate::types::Address;
    use ed25519_dalek::{Signer, SigningKey};

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
    fn try_submit_rejects_when_full() {
        let mut pool = Mempool::new(2);
        pool.try_submit(signed_tx(5, "r1", 1, 1, 0, 1)).unwrap();
        pool.try_submit(signed_tx(6, "r2", 1, 1, 0, 2)).unwrap();
        assert!(matches!(
            pool.try_submit(signed_tx(7, "r3", 1, 1, 0, 3)),
            Err(ProtocolError::MempoolFull)
        ));
        assert_eq!(pool.len(), 2);
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
}
