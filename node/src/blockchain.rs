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

    /// Blocks with `height >= start_height` (genesis is `0`). Used for catch-up replies.
    pub fn blocks_from_height(&self, start_height: u64) -> Vec<Block> {
        self.blocks
            .iter()
            .filter(|b| b.height >= start_height)
            .cloned()
            .collect()
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

    /// Take up to `max_transactions` from the mempool (FIFO order), build a sealed block on
    /// the current tip, and append it. On success, removes those transactions from the mempool.
    ///
    /// Returns how many transactions were committed. If the mempool has no candidates, returns
    /// `Ok(0)` and leaves the chain unchanged.
    ///
    /// If `append_block` fails (e.g. insufficient balance), the mempool is unchanged so callers
    /// can add eviction or revalidation policy later.
    pub fn append_block_from_mempool(
        &mut self,
        mempool: &mut Mempool,
        max_transactions: usize,
        timestamp_unix: u64,
    ) -> Result<usize, ProtocolError> {
        let txs = mempool.ordered_candidates_for_seal(&self.state, max_transactions);
        if txs.is_empty() {
            return Ok(0);
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
        mempool.remove_by_tx_hashes(hashes.iter().map(|s| s.as_str()));
        Ok(hashes.len())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::ConsensusParams;
    use crate::crypto::Crypto;
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
    fn recompute_state_from_blocks_matches_incremental_apply() {
        use crate::genesis::{Genesis, GenesisAllocation};

        let prev_hash = "GENESIS_HASH".to_string();
        let b1 = valid_block_1(prev_hash.clone());
        let sender = b1.transactions[0].sender.clone();
        let genesis = Genesis {
            allocations: vec![GenesisAllocation {
                address: sender.0.clone(),
                balance: 100,
            }],
        };
        let mut from_doc = Blockchain::from_genesis(&genesis).unwrap();
        from_doc.append_block(b1.clone()).unwrap();

        let tip_hash = from_doc.blocks().last().unwrap().block_hash.clone();
        let b2 = seal_block(Block {
            height: 2,
            previous_hash: tip_hash,
            timestamp_unix: 1_700_001_002,
            transactions: vec![sample_valid_tx()],
            block_hash: String::new(),
        });
        let mut tx2 = b2.transactions[0].clone();
        tx2.nonce = 1;
        tx2.timestamp_unix = 1_700_001_003;
        let p2 = tx2.unsigned_payload_bytes();
        let signing_key = SigningKey::from_bytes(&[21u8; 32]);
        tx2.signature = signing_key.sign(&p2).to_bytes().to_vec();
        tx2.tx_hash = Crypto::hash_bytes(&p2);
        let mut b2 = b2;
        b2.transactions = vec![tx2];
        b2.block_hash = b2.compute_block_hash();
        from_doc.append_block(b2.clone()).unwrap();

        let blocks = from_doc.blocks();
        let replayed = recompute_state_from_blocks(blocks, &genesis).unwrap();
        assert_eq!(
            replayed.accounts_sorted(),
            from_doc.state().accounts_sorted()
        );
    }

    #[test]
    fn rollback_to_height_restores_state_and_tip() {
        use crate::genesis::{Genesis, GenesisAllocation};

        let b1 = valid_block_1("GENESIS_HASH".into());
        let sender = b1.transactions[0].sender.clone();
        let genesis = Genesis {
            allocations: vec![GenesisAllocation {
                address: sender.0.clone(),
                balance: 100,
            }],
        };
        let mut chain = Blockchain::from_genesis(&genesis).unwrap();
        chain.append_block(b1).unwrap();
        assert_eq!(chain.height(), 1);
        assert_eq!(chain.state().get_account(&sender).unwrap().nonce, 1);

        chain.rollback_to_height(0, &genesis).unwrap();
        assert_eq!(chain.height(), 0);
        assert_eq!(chain.len(), 1);
        assert_eq!(chain.state().get_account(&sender).unwrap().nonce, 0);
        assert_eq!(chain.state().get_account(&sender).unwrap().balance, 100);
    }

    #[test]
    fn rollback_to_height_rejects_target_above_tip() {
        let mut chain = Blockchain::new();
        let r = chain.rollback_to_height(1, &crate::genesis::Genesis::empty());
        assert!(matches!(r, Err(ProtocolError::StateError(_))));
    }
}
