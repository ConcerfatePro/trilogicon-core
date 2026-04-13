use std::collections::{HashSet, VecDeque};

use crate::errors::ProtocolError;
use crate::operator_msg::PFX_MEMPOOL;
use crate::state::State;
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
        if self.txs.iter().any(|t| t.sender == tx.sender && t.nonce == tx.nonce) {
            return Err(ProtocolError::MempoolSenderNonceConflict);
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
        self.txs.retain(|t| !remove.contains(t.tx_hash.as_str()));
    }

    /// FIFO snapshot for rollback (e.g. pending-file drain or seal paths).
    pub(crate) fn clone_fifo(&self) -> VecDeque<Transaction> {
        self.txs.clone()
    }

    pub(crate) fn restore_fifo(&mut self, txs: VecDeque<Transaction>) {
        self.txs = txs;
    }

    /// Drop from the FIFO **head** transactions that are **not executable as the next tx on the
    /// current committed ledger** (`committed` only). **Local mempool policy**, not consensus.
    ///
    /// **Intentionally does not** keep entries waiting for **future** remote blocks to fund the
    /// sender or create accounts—that would speculate on uncommitted history. Operators who need
    /// that behavior must re-submit after sync.
    ///
    /// While the head matches, drops: `basic_validate` failure; missing sender in `committed`;
    /// stale nonce (`tx.nonce < account.nonce`); at `tx.nonce == account.nonce`, `amount+fee`
    /// overflow or insufficient balance.
    ///
    /// Stops at `tx.nonce > account.nonce` (gap): no future-nonce speculation, no reordering, no
    /// in-seal skipping.
    ///
    /// **Known gap:** if an atomic FIFO-prefix seal fails because a **later** tx in the prefix is
    /// invalid while an **earlier** tx is still valid, this purge does not remove the valid head;
    /// the queue can stay stuck until manual intervention or new submissions — fixing that would
    /// require changing producer/FIFO-prefix rules (out of scope for this helper).
    pub fn purge_nonviable_under_committed_state(&mut self, committed: &State) -> usize {
        let state = committed;
        let mut removed = 0usize;
        while let Some(tx) = self.txs.front() {
            if let Err(e) = tx.basic_validate() {
                eprintln!(
                    "{PFX_MEMPOOL} dropping front tx {} (basic_validate: {e}) — local queue hygiene vs committed ledger",
                    tx.tx_hash
                );
                self.txs.pop_front();
                removed += 1;
                continue;
            }
            let Some(acc) = state.get_account(&tx.sender) else {
                eprintln!(
                    "{PFX_MEMPOOL} dropping front tx {} (no sender on committed ledger — not queued for future funding)",
                    tx.tx_hash
                );
                self.txs.pop_front();
                removed += 1;
                continue;
            };
            if tx.nonce < acc.nonce {
                eprintln!(
                    "{PFX_MEMPOOL} dropping front tx {} (stale nonce {} < {})",
                    tx.tx_hash, tx.nonce, acc.nonce
                );
                self.txs.pop_front();
                removed += 1;
                continue;
            }
            if tx.nonce > acc.nonce {
                break;
            }
            let total_cost = match tx.amount.checked_add(tx.fee) {
                Some(c) => c,
                None => {
                    eprintln!(
                        "{PFX_MEMPOOL} dropping front tx {} (amount+fee overflow)",
                        tx.tx_hash
                    );
                    self.txs.pop_front();
                    removed += 1;
                    continue;
                }
            };
            if acc.balance < total_cost {
                eprintln!(
                    "{PFX_MEMPOOL} dropping front tx {} (insufficient balance on committed ledger — not queued for future income)",
                    tx.tx_hash
                );
                self.txs.pop_front();
                removed += 1;
                continue;
            }
            break;
        }
        removed
    }

    /// Drop transactions that are **permanently invalid** under the committed ledger because the
    /// sender already advanced past `tx.nonce`. Safe at **any queue position** (local policy only).
    ///
    /// Does **not** drop unknown senders (could exist only after a future remote commit), and does
    /// not drop `nonce > account.nonce` gap entries — same rules as
    /// [`Self::purge_nonviable_under_committed_state`] for heads.
    pub fn drop_stale_nonces_vs_committed(&mut self, committed: &State) -> usize {
        let before = self.txs.len();
        self.txs.retain(|tx| {
            match committed.get_account(&tx.sender) {
                Some(acc) if tx.nonce < acc.nonce => {
                    eprintln!(
                        "{PFX_MEMPOOL} dropping tx {} (stale nonce {} < committed {}) — global queue hygiene",
                        tx.tx_hash, tx.nonce, acc.nonce
                    );
                    false
                }
                _ => true,
            }
        });
        before.saturating_sub(self.txs.len())
    }

    /// Drop later queued txs that reuse a `(sender, nonce)` already taken earlier in the FIFO
    /// (local policy; first submission wins).
    pub fn drop_later_sender_nonce_conflicts_keep_fifo_first(&mut self) -> usize {
        let mut seen: HashSet<(crate::types::Address, u64)> = HashSet::new();
        let mut removed = 0usize;
        self.txs.retain(|tx| {
            let key = (tx.sender.clone(), tx.nonce);
            if seen.insert(key) {
                true
            } else {
                eprintln!(
                    "{PFX_MEMPOOL} dropping tx {} (duplicate sender+nonce vs an earlier queued tx; local FIFO policy)",
                    tx.tx_hash
                );
                removed += 1;
                false
            }
        });
        removed
    }

    /// Front-of-line purge then global stale-nonce removal vs `committed` (local policy only).
    pub fn hygiene_vs_committed_ledger(&mut self, committed: &State) -> (usize, usize, usize) {
        let front = self.purge_nonviable_under_committed_state(committed);
        let stale = self.drop_stale_nonces_vs_committed(committed);
        let dup = self.drop_later_sender_nonce_conflicts_keep_fifo_first();
        (front, stale, dup)
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

    #[test]
    fn purge_under_committed_drops_stale_nonce_at_front() {
        use crate::blockchain::Blockchain;
        use crate::genesis::{Genesis, GenesisAllocation};

        let first = signed_tx(5, "recv_z", 1, 1, 0, 10);
        let addr = first.sender.clone();
        let g = Genesis {
            allocations: vec![GenesisAllocation {
                address: addr.0.clone(),
                balance: 100,
            }],
        };
        let mut chain = Blockchain::from_genesis(&g).unwrap();
        let mut pool = Mempool::new(10);
        pool.try_submit(first).unwrap();
        chain
            .append_block_from_mempool(&mut pool, 8, 20)
            .unwrap();
        assert_eq!(chain.state().get_account(&addr).unwrap().nonce, 1);
        let bad = signed_tx(5, "recv_z", 1, 1, 0, 12);
        pool.try_submit(bad).unwrap();
        let n = pool.purge_nonviable_under_committed_state(chain.state());
        assert_eq!(n, 1);
        assert!(pool.is_empty());
    }

    #[test]
    fn purge_under_committed_keeps_nonce_gap_at_front() {
        use crate::genesis::{Genesis, GenesisAllocation};

        let head = signed_tx(5, "recv_gap", 1, 1, 0, 100);
        let addr = head.sender.clone();
        let g = Genesis {
            allocations: vec![GenesisAllocation {
                address: addr.0.clone(),
                balance: 100,
            }],
        };
        let state = State::from_genesis(&g).unwrap();
        let mut pool = Mempool::new(10);
        let gap = signed_tx(5, "recv_gap", 1, 1, 1, 101);
        pool.try_submit(gap).unwrap();
        assert_eq!(pool.purge_nonviable_under_committed_state(&state), 0);
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn purge_under_committed_drops_head_without_sender_on_ledger() {
        let mut pool = Mempool::new(10);
        let tx = signed_tx(8, "recv_ns", 1, 1, 0, 3_000_000);
        pool.try_submit(tx).unwrap();
        let state = State::new();
        assert_eq!(pool.purge_nonviable_under_committed_state(&state), 1);
        assert!(pool.is_empty());
    }

    #[test]
    fn purge_under_committed_drops_head_insufficient_balance_intentional_policy() {
        use crate::genesis::{Genesis, GenesisAllocation};

        let head = signed_tx(9, "recv_ib", 50, 1, 0, 3_000_001);
        let addr = head.sender.clone();
        let g = Genesis {
            allocations: vec![GenesisAllocation {
                address: addr.0.clone(),
                balance: 40,
            }],
        };
        let state = State::from_genesis(&g).unwrap();
        let mut pool = Mempool::new(10);
        pool.try_submit(head).unwrap();
        assert_eq!(pool.purge_nonviable_under_committed_state(&state), 1);
        assert!(pool.is_empty());
    }

    #[test]
    fn drop_stale_nonces_removes_mid_queue_obsolete_txs() {
        use crate::genesis::{Genesis, GenesisAllocation};

        let first_a = signed_tx(10, "recv_mq_a", 1, 1, 0, 4_000_001);
        let addr_a = first_a.sender.clone();
        let addr_b = signed_tx(11, "recv_mq_b", 1, 1, 0, 4_000_002).sender.clone();
        let g = Genesis {
            allocations: vec![
                GenesisAllocation {
                    address: addr_a.0.clone(),
                    balance: 100,
                },
                GenesisAllocation {
                    address: addr_b.0.clone(),
                    balance: 100,
                },
            ],
        };
        let mut state = State::from_genesis(&g).unwrap();
        state.apply_transaction(&first_a).unwrap();
        assert_eq!(state.get_account(&addr_a).unwrap().nonce, 1);

        let mut pool = Mempool::new(20);
        let b_head = signed_tx(11, "recv_mq_b", 1, 1, 0, 4_000_010);
        let stale_a = signed_tx(10, "recv_mq_stale", 1, 1, 0, 4_000_011);
        pool.try_submit(b_head.clone()).unwrap();
        pool.try_submit(stale_a).unwrap();

        assert_eq!(pool.drop_stale_nonces_vs_committed(&state), 1);
        assert_eq!(pool.len(), 1);
        assert_eq!(pool.ordered_candidates(1)[0].tx_hash, b_head.tx_hash);
    }

    #[test]
    fn try_submit_rejects_second_distinct_tx_same_sender_nonce() {
        let mut pool = Mempool::new(10);
        let first = signed_tx(15, "recv_a", 1, 1, 0, 5_000_001);
        let second = signed_tx(15, "recv_b", 2, 1, 0, 5_000_002);
        pool.try_submit(first.clone()).unwrap();
        assert!(matches!(
            pool.try_submit(second),
            Err(ProtocolError::MempoolSenderNonceConflict)
        ));
        assert_eq!(pool.len(), 1);
        assert_eq!(pool.ordered_candidates(1)[0].tx_hash, first.tx_hash);
    }

    #[test]
    fn drop_later_sender_nonce_conflicts_keeps_fifo_first() {
        let mut pool = Mempool::new(10);
        let first = signed_tx(16, "r1", 1, 1, 0, 5_000_010);
        let second = signed_tx(16, "r2", 2, 1, 0, 5_000_011);
        pool.try_submit(first.clone()).unwrap();
        let mut q = pool.clone_fifo();
        q.push_back(second);
        pool.restore_fifo(q);
        assert_eq!(pool.len(), 2);
        assert_eq!(pool.drop_later_sender_nonce_conflicts_keep_fifo_first(), 1);
        assert_eq!(pool.len(), 1);
        assert_eq!(pool.ordered_candidates(1)[0].tx_hash, first.tx_hash);
    }

    #[test]
    fn same_nonce_conflict_does_not_stall_seal_first_wins_after_dup_drop() {
        use crate::blockchain::Blockchain;
        use crate::genesis::{Genesis, GenesisAllocation};

        let t0 = signed_tx(17, "recv_seal", 10, 1, 0, 5_000_020);
        let addr = t0.sender.clone();
        let g = Genesis {
            allocations: vec![GenesisAllocation {
                address: addr.0.clone(),
                balance: 100,
            }],
        };
        let mut chain = Blockchain::from_genesis(&g).unwrap();
        let mut pool = Mempool::new(10);
        pool.try_submit(t0.clone()).unwrap();
        let mut q = pool.clone_fifo();
        q.push_back(signed_tx(17, "other", 20, 1, 0, 5_000_021));
        pool.restore_fifo(q);
        pool.drop_later_sender_nonce_conflicts_keep_fifo_first();
        chain
            .append_block_from_mempool(&mut pool, 8, 5_000_030)
            .unwrap();
        assert_eq!(chain.height(), 1);
        assert_eq!(chain.state().get_account(&addr).unwrap().nonce, 1);
        assert!(pool.is_empty());
    }
}
