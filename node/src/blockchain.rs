use crate::block::Block;
use crate::consensus::{
    ConsensusParams, validate_block_timestamps_vs_parent, validate_block_vs_local_time,
};
use crate::errors::ProtocolError;
use crate::genesis::Genesis;
use crate::mempool::Mempool;
use crate::state::State;

/// Recompute account state by replaying non-genesis blocks on top of genesis allocations.
pub fn recompute_state_from_blocks(
    blocks: &[Block],
    genesis: &Genesis,
) -> Result<State, ProtocolError> {
    let mut state = State::from_genesis(genesis)?;
    for block in blocks.iter().skip(1) {
        for tx in &block.transactions {
            state.apply_transaction(tx)?;
        }
    }
    Ok(state)
}

pub struct Blockchain {
    blocks: Vec<Block>,
    state: State,
    consensus: ConsensusParams,
}

impl Default for Blockchain {
    fn default() -> Self {
        Self::new()
    }
}

impl Blockchain {
    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut State {
        &mut self.state
    }

    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// Empty genesis allocations (no accounts). Prefer [`Self::from_genesis`] for real networks.
    pub fn new() -> Self {
        Self::from_genesis(&Genesis::empty()).expect("empty genesis is valid")
    }

    /// Build chain with genesis block + initial [`State`] from `genesis` allocations.
    pub fn from_genesis(genesis: &Genesis) -> Result<Self, ProtocolError> {
        let state = State::from_genesis(genesis)?;
        Ok(Self {
            blocks: vec![Block::genesis()],
            state,
            consensus: ConsensusParams::default(),
        })
    }

    /// Same as [`Self::from_genesis`] with explicit consensus timestamp policy.
    pub fn from_genesis_with_consensus(
        genesis: &Genesis,
        consensus: ConsensusParams,
    ) -> Result<Self, ProtocolError> {
        let mut c = Self::from_genesis(genesis)?;
        c.consensus = consensus;
        Ok(c)
    }

    /// Empty genesis + explicit consensus (unit tests).
    pub fn with_consensus_params(consensus: ConsensusParams) -> Self {
        Self::from_genesis_with_consensus(&Genesis::empty(), consensus).expect("valid")
    }

    pub fn consensus_params(&self) -> &ConsensusParams {
        &self.consensus
    }

    pub fn consensus_params_mut(&mut self) -> &mut ConsensusParams {
        &mut self.consensus
    }

    /// Blocks with `height >= start_height` (genesis is `0`), at most `limit` clones.
    ///
    /// Used for bounded catch-up replies: callers should pass a small `limit` (e.g.
    /// [`crate::network::MAX_BLOCKS_PER_BATCH`]) so a low `start_height` on a long chain does not
    /// materialize the full suffix.
    pub fn blocks_from_height_limited(&self, start_height: u64, limit: usize) -> Vec<Block> {
        if limit == 0 {
            return Vec::new();
        }
        self.blocks
            .iter()
            .filter(|b| b.height >= start_height)
            .take(limit)
            .cloned()
            .collect()
    }

    /// Blocks with `height >= start_height` (genesis is `0`). Clones the full matching suffix.
    pub fn blocks_from_height(&self, start_height: u64) -> Vec<Block> {
        self.blocks_from_height_limited(start_height, usize::MAX)
    }

    /// Network ingress path: reject blocks too far in the future vs `now_unix`, then full append.
    pub fn try_append_network_block(
        &mut self,
        block: Block,
        now_unix: u64,
    ) -> Result<(), ProtocolError> {
        validate_block_vs_local_time(
            block.timestamp_unix,
            now_unix,
            self.consensus.max_future_drift_secs,
        )?;
        self.append_block(block)
    }

    pub fn height(&self) -> u64 {
        self.blocks.last().map_or(0, |b| b.height)
    }

    /// Drop blocks above `target_height` and rebuild [`State`] from `genesis` + remaining blocks.
    ///
    /// Used when local chain advanced in memory but durable storage failed: restores consistency
    /// with the last persisted tip. `target_height` must be the height of the current durable tip
    /// (typically `tip.height - 1` after one failed seal).
    pub fn rollback_to_height(
        &mut self,
        target_height: u64,
        genesis: &Genesis,
    ) -> Result<(), ProtocolError> {
        let current = self.height();
        if target_height > current {
            return Err(ProtocolError::StateError(format!(
                "rollback target height {target_height} above current tip {current}"
            )));
        }
        while self.height() > target_height {
            self.blocks.pop();
        }
        self.state = recompute_state_from_blocks(&self.blocks, genesis)?;
        Ok(())
    }

    pub fn append_block(&mut self, block: Block) -> Result<(), ProtocolError> {
        block.basic_validate()?;

        let tip = self
            .blocks
            .last()
            .ok_or_else(|| ProtocolError::StateError(String::from("chain tip missing")))?;

        if block.height != tip.height + 1 {
            return Err(ProtocolError::InvalidBlock(String::from(
                "invalid block height",
            )));
        }

        if block.previous_hash != tip.block_hash {
            return Err(ProtocolError::InvalidBlock(String::from(
                "invalid previous hash",
            )));
        }

        validate_block_timestamps_vs_parent(tip, &block, &self.consensus)?;

        // Apply all transactions atomically: clone state, apply on the copy,
        // commit only if every tx succeeds (no partial block effects).
        let mut next_state = self.state.clone();
        for tx in &block.transactions {
            next_state.apply_transaction(tx)?;
        }
        self.state = next_state;

        self.blocks.push(block);
        Ok(())
    }

    /// Build a block from up to `max_transactions` FIFO mempool candidates, append it, but **do
    /// not** remove those transactions from the mempool yet.
    ///
    /// After **durable** persistence of the new tip succeeds, call [`Mempool::remove_by_tx_hashes`]
    /// with the returned `tx_hashes`. If persistence fails, call [`Self::rollback_last_block`];
    /// the mempool order and contents stay equivalent to the pre-seal state.
    ///
    /// Returns `Ok(None)` when there are no candidates. If [`Self::append_block`] fails, the chain
    /// and mempool are unchanged.
    pub fn append_block_from_mempool_pending_removal(
        &mut self,
        mempool: &Mempool,
        max_transactions: usize,
        timestamp_unix: u64,
    ) -> Result<Option<Vec<String>>, ProtocolError> {
        let txs = mempool.ordered_candidates_for_seal(&self.state, max_transactions)?;
        if txs.is_empty() {
            return Ok(None);
        }

        let tip = self
            .blocks
            .last()
            .ok_or_else(|| ProtocolError::StateError(String::from("chain tip missing")))?;

        let mut block = Block {
            height: tip.height + 1,
            previous_hash: tip.block_hash.clone(),
            timestamp_unix,
            transactions: txs,
            block_hash: String::new(),
        };
        block.block_hash = block.compute_block_hash();

        let hashes: Vec<String> = block
            .transactions
            .iter()
            .map(|t| t.tx_hash.clone())
            .collect();

        self.append_block(block)?;
        Ok(Some(hashes))
    }

    /// Take up to `max_transactions` from the mempool (FIFO order), build a sealed block on
    /// the current tip, and append it. On success, removes those transactions from the mempool.
    ///
    /// Returns how many transactions were committed. If the mempool has no candidates, returns
    /// `Ok(0)` and leaves the chain unchanged.
    ///
    /// If `append_block` fails (e.g. insufficient balance), the mempool is unchanged so callers
    /// can add eviction or revalidation policy later.
    ///
    /// **Persistence note:** For local sealing with disk append, prefer
    /// [`Self::append_block_from_mempool_pending_removal`] so mempool removal happens only after
    /// durable `chain.blocks` writes succeed.
    pub fn append_block_from_mempool(
        &mut self,
        mempool: &mut Mempool,
        max_transactions: usize,
        timestamp_unix: u64,
    ) -> Result<usize, ProtocolError> {
        match self.append_block_from_mempool_pending_removal(mempool, max_transactions, timestamp_unix)?
        {
            None => Ok(0),
            Some(hashes) => {
                let n = hashes.len();
                mempool.remove_by_tx_hashes(hashes.iter().map(|s| s.as_str()));
                Ok(n)
            }
        }
    }

    /// Recompute [`State`] from `genesis` by re-applying every non-genesis block in order.
    pub fn rebuild_state_from_genesis(&mut self, genesis: &Genesis) -> Result<(), ProtocolError> {
        let mut st = State::from_genesis(genesis)?;
        for b in self.blocks.iter().skip(1) {
            for tx in &b.transactions {
                st.apply_transaction(tx)?;
            }
        }
        self.state = st;
        Ok(())
    }

    /// Drop the current non-genesis tip and rebuild state. Used when chain RAM advanced but disk
    /// persist failed, so memory matches disk again.
    pub fn rollback_last_block(&mut self, genesis: &Genesis) -> Result<(), ProtocolError> {
        if self.blocks.len() <= 1 {
            return Err(ProtocolError::StateError(
                "cannot rollback genesis tip".into(),
            ));
        }
        self.blocks.pop();
        self.rebuild_state_from_genesis(genesis)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::ConsensusParams;
    use crate::crypto::Crypto;
    use crate::genesis::{Genesis, GenesisAllocation};
    use crate::mempool::Mempool;
    use crate::transaction::Transaction;
    use crate::types::Address;
    use ed25519_dalek::{Signer, SigningKey};

    /// Set `block_hash` from canonical preimage (must call after txs are fully valid).
    fn seal_block(mut block: Block) -> Block {
        block.block_hash = block.compute_block_hash();
        block
    }

    fn sample_valid_tx() -> Transaction {
        let signing_key = SigningKey::from_bytes(&[21u8; 32]);
        let verifying_key = signing_key.verifying_key();

        let mut tx = Transaction {
            sender: Address::new(Crypto::address_from_public_key(&verifying_key.to_bytes())),
            receiver: Address::new("receiver_chain_test"),
            amount: 5,
            fee: 1,
            nonce: 0,
            timestamp_unix: 1_700_001_000,
            public_key: verifying_key.to_bytes().to_vec(),
            signature: Vec::new(),
            tx_hash: String::new(),
        };

        let payload = tx.unsigned_payload_bytes();
        tx.signature = signing_key.sign(&payload).to_bytes().to_vec();
        tx.tx_hash = Crypto::hash_bytes(&payload);
        tx
    }

    fn valid_block_1(prev_hash: String) -> Block {
        seal_block(Block {
            height: 1,
            previous_hash: prev_hash,
            timestamp_unix: 1_700_001_001,
            transactions: vec![sample_valid_tx()],
            block_hash: String::new(),
        })
    }

    /// `blocks_from_height_limited` must match `blocks().iter().filter(|b| b.height >= start).take(limit)`
    /// (the GET_BLOCKS path): bounded clones, not the full suffix.
    #[test]
    fn blocks_from_height_limited_matches_prefix_of_full_suffix() {
        let mut chain = Blockchain::new();
        let signing_key = SigningKey::from_bytes(&[44u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let sender = Address::new(Crypto::address_from_public_key(&verifying_key.to_bytes()));
        chain.state_mut().create_account(sender.clone(), 1_000_000_000);

        let mut prev = "GENESIS_HASH".to_string();
        for h in 1u64..=30 {
            let mut tx = Transaction {
                sender: sender.clone(),
                receiver: Address::new("recv_lim_test"),
                amount: 1,
                fee: 1,
                nonce: h - 1,
                timestamp_unix: 1_800_000_000 + h,
                public_key: verifying_key.to_bytes().to_vec(),
                signature: Vec::new(),
                tx_hash: String::new(),
            };
            let payload = tx.unsigned_payload_bytes();
            tx.signature = signing_key.sign(&payload).to_bytes().to_vec();
            tx.tx_hash = Crypto::hash_bytes(&payload);

            let mut b = Block {
                height: h,
                previous_hash: prev,
                timestamp_unix: 1_800_000_100 + h,
                transactions: vec![tx],
                block_hash: String::new(),
            };
            b.block_hash = b.compute_block_hash();
            prev = b.block_hash.clone();
            chain.append_block(b).unwrap();
        }

        const CAP: usize = 5;
        let limited = chain.blocks_from_height_limited(0, CAP);
        let want: Vec<Block> = chain.blocks().iter().take(CAP).cloned().collect();
        assert_eq!(limited.len(), CAP);
        assert_eq!(
            limited.iter().map(|b| b.height).collect::<Vec<_>>(),
            want.iter().map(|b| b.height).collect::<Vec<_>>()
        );
        assert_eq!(
            limited.iter().map(|b| &b.block_hash).collect::<Vec<_>>(),
            want.iter().map(|b| &b.block_hash).collect::<Vec<_>>()
        );
        assert_eq!(limited.last().unwrap().height, (CAP - 1) as u64);

        let from_ten = chain.blocks_from_height_limited(10, 3);
        let want_ten: Vec<Block> = chain
            .blocks()
            .iter()
            .filter(|b| b.height >= 10)
            .take(3)
            .cloned()
            .collect();
        assert_eq!(from_ten.len(), 3);
        assert_eq!(
            from_ten.iter().map(|b| b.height).collect::<Vec<_>>(),
            want_ten.iter().map(|b| b.height).collect::<Vec<_>>()
        );
        assert_eq!(
            from_ten.iter().map(|b| &b.block_hash).collect::<Vec<_>>(),
            want_ten.iter().map(|b| &b.block_hash).collect::<Vec<_>>()
        );
    }

    #[test]
    fn try_append_network_block_rejects_too_far_future() {
        let mut chain = Blockchain::new();
        chain.consensus_params_mut().max_future_drift_secs = 60;
        let mut b = valid_block_1("GENESIS_HASH".into());
        b.timestamp_unix = 2_000_000_000;
        b.block_hash = b.compute_block_hash();
        let acc = b.transactions[0].sender.clone();
        chain.state_mut().create_account(acc, 100);
        let now = 1_000_000_000u64;
        let r = chain.try_append_network_block(b, now);
        assert!(matches!(r, Err(ProtocolError::InvalidBlock(_))));
    }

    #[test]
    fn append_block_rejects_timestamp_under_min_interval() {
        // Genesis time is 0; fixture block uses 1_700_001_001 — require a higher floor than that.
        let mut chain = Blockchain::with_consensus_params(ConsensusParams {
            min_block_interval_secs: 2_000_000_000,
            max_future_drift_secs: u64::MAX,
        });
        let prev_hash = "GENESIS_HASH".to_string();
        let block = valid_block_1(prev_hash);
        let sender = block.transactions[0].sender.clone();
        chain.state_mut().create_account(sender, 100);

        assert!(matches!(
            chain.append_block(block),
            Err(ProtocolError::InvalidBlock(_))
        ));
        assert_eq!(chain.height(), 0);
    }

    #[test]
    fn append_block_accepts_valid_next_block() {
        let mut chain = Blockchain::new();
        let prev_hash = "GENESIS_HASH".to_string();
        let block = valid_block_1(prev_hash);

        // NEW: seed sender account so state application can succeed
        let sender = block.transactions[0].sender.clone();
        chain.state_mut().create_account(sender, 100);

        assert!(chain.append_block(block).is_ok());
        assert_eq!(chain.height(), 1);
        assert_eq!(chain.len(), 2);
    }

    #[test]
    fn append_block_rejects_invalid_height() {
        let mut chain = Blockchain::new();
        let block = seal_block(Block {
            height: 2, // should be 1
            previous_hash: "GENESIS_HASH".to_string(),
            timestamp_unix: 1_700_001_002,
            transactions: vec![sample_valid_tx()],
            block_hash: String::new(),
        });

        let result = chain.append_block(block);
        assert!(matches!(result, Err(ProtocolError::InvalidBlock(_))));
        assert_eq!(chain.height(), 0);
        assert_eq!(chain.len(), 1);
    }

    #[test]
    fn append_block_rejects_invalid_previous_hash() {
        let mut chain = Blockchain::new();
        let block = seal_block(Block {
            height: 1,
            previous_hash: "WRONG_HASH".to_string(),
            timestamp_unix: 1_700_001_003,
            transactions: vec![sample_valid_tx()],
            block_hash: String::new(),
        });

        let result = chain.append_block(block);
        assert!(matches!(result, Err(ProtocolError::InvalidBlock(_))));
        assert_eq!(chain.height(), 0);
        assert_eq!(chain.len(), 1);
    }

    #[test]
    fn append_block_rejects_structurally_invalid_block() {
        let mut chain = Blockchain::new();

        let mut bad_tx = sample_valid_tx();
        bad_tx.amount = 0; // invalid tx => block.basic_validate should fail

        let block = Block {
            height: 1,
            previous_hash: "GENESIS_HASH".to_string(),
            timestamp_unix: 1_700_001_004,
            transactions: vec![bad_tx],
            block_hash: "bad_struct_block".to_string(),
        };

        let result = chain.append_block(block);
        assert!(result.is_err());
        assert_eq!(chain.height(), 0);
        assert_eq!(chain.len(), 1);
    }

    #[test]
    fn append_block_applies_state_updates_for_valid_block() {
        let mut chain = Blockchain::new();

        let signing_key = SigningKey::from_bytes(&[31u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let sender_addr = Address::new(Crypto::address_from_public_key(&verifying_key.to_bytes()));
        let receiver_addr = Address::new("receiver_state_apply");

        chain.state_mut().create_account(sender_addr.clone(), 100);

        let mut tx = Transaction {
            sender: sender_addr.clone(),
            receiver: receiver_addr.clone(),
            amount: 10,
            fee: 1,
            nonce: 0,
            timestamp_unix: 1_700_002_000,
            public_key: verifying_key.to_bytes().to_vec(),
            signature: Vec::new(),
            tx_hash: String::new(),
        };
        let payload = tx.unsigned_payload_bytes();
        tx.signature = signing_key.sign(&payload).to_bytes().to_vec();
        tx.tx_hash = Crypto::hash_bytes(&payload);

        let block = seal_block(Block {
            height: 1,
            previous_hash: "GENESIS_HASH".to_string(),
            timestamp_unix: 1_700_002_001,
            transactions: vec![tx],
            block_hash: String::new(),
        });

        assert!(chain.append_block(block).is_ok());
        assert_eq!(chain.height(), 1);

        let sender = chain.state().get_account(&sender_addr).unwrap();
        let receiver = chain.state().get_account(&receiver_addr).unwrap();
        assert_eq!(sender.balance, 89);
        assert_eq!(sender.nonce, 1);
        assert_eq!(receiver.balance, 10);
    }

    #[test]
    fn append_block_rejects_when_tx_fails_state_rules() {
        let mut chain = Blockchain::new();

        let signing_key = SigningKey::from_bytes(&[32u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let sender_addr = Address::new(Crypto::address_from_public_key(&verifying_key.to_bytes()));
        let receiver_addr = Address::new("receiver_state_fail");

        chain.state_mut().create_account(sender_addr.clone(), 5);

        let mut tx = Transaction {
            sender: sender_addr.clone(),
            receiver: receiver_addr.clone(),
            amount: 10, // exceeds balance
            fee: 1,
            nonce: 0,
            timestamp_unix: 1_700_002_010,
            public_key: verifying_key.to_bytes().to_vec(),
            signature: Vec::new(),
            tx_hash: String::new(),
        };
        let payload = tx.unsigned_payload_bytes();
        tx.signature = signing_key.sign(&payload).to_bytes().to_vec();
        tx.tx_hash = Crypto::hash_bytes(&payload);

        let block = seal_block(Block {
            height: 1,
            previous_hash: "GENESIS_HASH".to_string(),
            timestamp_unix: 1_700_002_011,
            transactions: vec![tx],
            block_hash: String::new(),
        });

        let result = chain.append_block(block);
        assert!(matches!(result, Err(ProtocolError::InsufficientBalance)));
    }

    #[test]
    fn append_block_does_not_advance_height_on_state_failure() {
        let mut chain = Blockchain::new();

        let signing_key = SigningKey::from_bytes(&[33u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let sender_addr = Address::new(Crypto::address_from_public_key(&verifying_key.to_bytes()));
        let receiver_addr = Address::new("receiver_height_guard");

        chain.state_mut().create_account(sender_addr.clone(), 1);

        let mut tx = Transaction {
            sender: sender_addr.clone(),
            receiver: receiver_addr,
            amount: 5, // invalid by balance
            fee: 1,
            nonce: 0,
            timestamp_unix: 1_700_002_020,
            public_key: verifying_key.to_bytes().to_vec(),
            signature: Vec::new(),
            tx_hash: String::new(),
        };
        let payload = tx.unsigned_payload_bytes();
        tx.signature = signing_key.sign(&payload).to_bytes().to_vec();
        tx.tx_hash = Crypto::hash_bytes(&payload);

        let block = seal_block(Block {
            height: 1,
            previous_hash: "GENESIS_HASH".to_string(),
            timestamp_unix: 1_700_002_021,
            transactions: vec![tx],
            block_hash: String::new(),
        });

        assert!(chain.append_block(block).is_err());
        assert_eq!(chain.height(), 0);
        assert_eq!(chain.len(), 1);
    }

    #[test]
    fn append_block_rejects_mid_block_failure_without_partial_state() {
        let mut chain = Blockchain::new();

        let signing_key = SigningKey::from_bytes(&[40u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let sender_addr = Address::new(Crypto::address_from_public_key(&verifying_key.to_bytes()));
        let receiver_a = Address::new("receiver_atomic_a");
        let receiver_b = Address::new("receiver_atomic_b");

        chain.state_mut().create_account(sender_addr.clone(), 20);

        let mut tx1 = Transaction {
            sender: sender_addr.clone(),
            receiver: receiver_a.clone(),
            amount: 5,
            fee: 1,
            nonce: 0,
            timestamp_unix: 1_700_003_000,
            public_key: verifying_key.to_bytes().to_vec(),
            signature: Vec::new(),
            tx_hash: String::new(),
        };
        let p1 = tx1.unsigned_payload_bytes();
        tx1.signature = signing_key.sign(&p1).to_bytes().to_vec();
        tx1.tx_hash = Crypto::hash_bytes(&p1);

        let mut tx2 = Transaction {
            sender: sender_addr.clone(),
            receiver: receiver_b,
            amount: 20,
            fee: 1,
            nonce: 1,
            timestamp_unix: 1_700_003_001,
            public_key: verifying_key.to_bytes().to_vec(),
            signature: Vec::new(),
            tx_hash: String::new(),
        };
        let p2 = tx2.unsigned_payload_bytes();
        tx2.signature = signing_key.sign(&p2).to_bytes().to_vec();
        tx2.tx_hash = Crypto::hash_bytes(&p2);

        let block = seal_block(Block {
            height: 1,
            previous_hash: "GENESIS_HASH".to_string(),
            timestamp_unix: 1_700_003_002,
            transactions: vec![tx1, tx2],
            block_hash: String::new(),
        });

        assert!(matches!(
            chain.append_block(block),
            Err(ProtocolError::InsufficientBalance)
        ));
        assert_eq!(chain.height(), 0);
        assert_eq!(chain.len(), 1);

        let sender = chain.state().get_account(&sender_addr).unwrap();
        assert_eq!(sender.balance, 20);
        assert_eq!(sender.nonce, 0);
        assert!(chain.state().get_account(&receiver_a).is_none());
    }

    #[test]
    fn append_block_from_mempool_empty_returns_zero() {
        let mut chain = Blockchain::new();
        let mut pool = Mempool::new(10);
        assert_eq!(
            chain
                .append_block_from_mempool(&mut pool, 8, 1_700_004_000)
                .unwrap(),
            0
        );
        assert_eq!(chain.height(), 0);
        assert!(pool.is_empty());
    }

    #[test]
    fn append_block_from_mempool_pending_removal_rollback_preserves_fifo_order() {
        let mut pool = Mempool::new(10);

        let signing_key = SigningKey::from_bytes(&[59u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let sender_addr = Address::new(Crypto::address_from_public_key(&verifying_key.to_bytes()));
        let g = Genesis {
            allocations: vec![GenesisAllocation {
                address: sender_addr.0.clone(),
                balance: 100,
            }],
        };
        let mut chain = Blockchain::from_genesis(&g).unwrap();

        for nonce in 0u64..2u64 {
            let mut tx = Transaction {
                sender: sender_addr.clone(),
                receiver: Address::new("recv_fifo"),
                amount: 1,
                fee: 1,
                nonce,
                timestamp_unix: 1_700_020_000 + nonce,
                public_key: verifying_key.to_bytes().to_vec(),
                signature: Vec::new(),
                tx_hash: String::new(),
            };
            let p = tx.unsigned_payload_bytes();
            tx.signature = signing_key.sign(&p).to_bytes().to_vec();
            tx.tx_hash = Crypto::hash_bytes(&p);
            pool.try_submit(tx).unwrap();
        }

        let h0 = pool.ordered_candidates(2)[0].tx_hash.clone();
        let h1 = pool.ordered_candidates(2)[1].tx_hash.clone();

        let hashes = chain
            .append_block_from_mempool_pending_removal(&pool, 8, 1_700_020_010)
            .unwrap()
            .expect("sealed");
        assert_eq!(hashes.len(), 2);
        assert_eq!(chain.height(), 1);
        assert_eq!(pool.len(), 2, "mempool must not drain before persist");

        chain.rollback_last_block(&g).unwrap();
        assert_eq!(chain.height(), 0);

        let c = pool.ordered_candidates(2);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].tx_hash, h0);
        assert_eq!(c[1].tx_hash, h1);
    }

    #[test]
    fn append_block_from_mempool_commits_and_drains() {
        let mut chain = Blockchain::new();
        let mut pool = Mempool::new(10);

        let signing_key = SigningKey::from_bytes(&[50u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let sender_addr = Address::new(Crypto::address_from_public_key(&verifying_key.to_bytes()));
        let receiver_addr = Address::new("mempool_block_recv");

        chain.state_mut().create_account(sender_addr.clone(), 50);

        let mut tx = Transaction {
            sender: sender_addr.clone(),
            receiver: receiver_addr.clone(),
            amount: 5,
            fee: 1,
            nonce: 0,
            timestamp_unix: 1_700_004_001,
            public_key: verifying_key.to_bytes().to_vec(),
            signature: Vec::new(),
            tx_hash: String::new(),
        };
        let payload = tx.unsigned_payload_bytes();
        tx.signature = signing_key.sign(&payload).to_bytes().to_vec();
        tx.tx_hash = Crypto::hash_bytes(&payload);

        pool.try_submit(tx).unwrap();
        let n = chain
            .append_block_from_mempool(&mut pool, 8, 1_700_004_002)
            .unwrap();
        assert_eq!(n, 1);
        assert_eq!(chain.height(), 1);
        assert!(pool.is_empty());
        assert_eq!(chain.state().get_account(&sender_addr).unwrap().balance, 44);
        assert_eq!(
            chain.state().get_account(&receiver_addr).unwrap().balance,
            5
        );
    }

    #[test]
    fn append_block_from_mempool_skips_unexecutable_tx_without_draining_pool() {
        let mut chain = Blockchain::new();
        let mut pool = Mempool::new(10);

        let signing_key = SigningKey::from_bytes(&[51u8; 32]);
        let verifying_key = signing_key.verifying_key();

        let mut tx = Transaction {
            sender: Address::new(Crypto::address_from_public_key(&verifying_key.to_bytes())),
            receiver: Address::new("recv_fail"),
            amount: 10,
            fee: 1,
            nonce: 0,
            timestamp_unix: 1_700_004_010,
            public_key: verifying_key.to_bytes().to_vec(),
            signature: Vec::new(),
            tx_hash: String::new(),
        };
        let payload = tx.unsigned_payload_bytes();
        tx.signature = signing_key.sign(&payload).to_bytes().to_vec();
        tx.tx_hash = Crypto::hash_bytes(&payload);

        pool.try_submit(tx.clone()).unwrap();
        assert_eq!(
            chain
                .append_block_from_mempool(&mut pool, 8, 1_700_004_011)
                .unwrap(),
            0
        );
        assert_eq!(chain.height(), 0);
        assert_eq!(pool.len(), 1);
    }

    /// If nonce-1 is queued before nonce-0, seal selection skips the gap and commits nonce-0 first.
    #[test]
    fn append_block_from_mempool_skips_nonce_gap_and_seals_executable_tx() {
        let mut chain = Blockchain::new();
        let mut pool = Mempool::new(10);

        let signing_key = SigningKey::from_bytes(&[52u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let sender_addr = Address::new(Crypto::address_from_public_key(&verifying_key.to_bytes()));
        let r1 = Address::new("recv_nonce_order_1");
        let r0 = Address::new("recv_nonce_order_0");

        chain.state_mut().create_account(sender_addr.clone(), 100);

        let mut tx_nonce1 = Transaction {
            sender: sender_addr.clone(),
            receiver: r1.clone(),
            amount: 1,
            fee: 1,
            nonce: 1,
            timestamp_unix: 1_700_005_000,
            public_key: verifying_key.to_bytes().to_vec(),
            signature: Vec::new(),
            tx_hash: String::new(),
        };
        let p1 = tx_nonce1.unsigned_payload_bytes();
        tx_nonce1.signature = signing_key.sign(&p1).to_bytes().to_vec();
        tx_nonce1.tx_hash = Crypto::hash_bytes(&p1);

        let mut tx_nonce0 = Transaction {
            sender: sender_addr.clone(),
            receiver: r0,
            amount: 1,
            fee: 1,
            nonce: 0,
            timestamp_unix: 1_700_005_001,
            public_key: verifying_key.to_bytes().to_vec(),
            signature: Vec::new(),
            tx_hash: String::new(),
        };
        let p0 = tx_nonce0.unsigned_payload_bytes();
        tx_nonce0.signature = signing_key.sign(&p0).to_bytes().to_vec();
        tx_nonce0.tx_hash = Crypto::hash_bytes(&p0);

        pool.try_submit(tx_nonce1).unwrap();
        pool.try_submit(tx_nonce0).unwrap();
        assert_eq!(pool.len(), 2);

        let n = chain
            .append_block_from_mempool(&mut pool, 8, 1_700_005_002)
            .unwrap();
        assert_eq!(n, 1);
        assert_eq!(chain.height(), 1);
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn try_append_network_block_accepts_valid_block_when_future_drift_unbounded() {
        let mut chain = Blockchain::new();
        let prev_hash = "GENESIS_HASH".to_string();
        let block = valid_block_1(prev_hash);
        let sender = block.transactions[0].sender.clone();
        chain.state_mut().create_account(sender, 100);
        let now = 1u64;
        assert!(chain.try_append_network_block(block, now).is_ok());
        assert_eq!(chain.height(), 1);
    }

    #[test]
    fn rollback_last_block_restores_prior_state() {
        let signing_key = SigningKey::from_bytes(&[90u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let sender_addr = Address::new(Crypto::address_from_public_key(&verifying_key.to_bytes()));
        let genesis = Genesis {
            allocations: vec![GenesisAllocation {
                address: sender_addr.0.clone(),
                balance: 50,
            }],
        };
        let mut chain = Blockchain::from_genesis(&genesis).unwrap();
        let mut pool = Mempool::new(10);

        let mut tx = Transaction {
            sender: sender_addr.clone(),
            receiver: Address::new("recv_rb"),
            amount: 5,
            fee: 1,
            nonce: 0,
            timestamp_unix: 1_700_010_000,
            public_key: verifying_key.to_bytes().to_vec(),
            signature: Vec::new(),
            tx_hash: String::new(),
        };
        let payload = tx.unsigned_payload_bytes();
        tx.signature = signing_key.sign(&payload).to_bytes().to_vec();
        tx.tx_hash = Crypto::hash_bytes(&payload);
        pool.try_submit(tx).unwrap();

        chain
            .append_block_from_mempool(&mut pool, 8, 1_700_010_001)
            .unwrap();
        assert_eq!(chain.height(), 1);
        assert_eq!(chain.state().get_account(&sender_addr).unwrap().nonce, 1);

        chain.rollback_last_block(&genesis).unwrap();
        assert_eq!(chain.height(), 0);
        assert_eq!(chain.state().get_account(&sender_addr).unwrap().nonce, 0);
        assert_eq!(chain.state().get_account(&sender_addr).unwrap().balance, 50);
    }
}
