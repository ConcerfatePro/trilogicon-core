use crate::block::Block;
use crate::errors::ProtocolError;
use crate::state::State;

pub struct Blockchain {
    blocks: Vec<Block>,
    state: State,
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

    pub fn new() -> Self {
        Self {
            blocks: vec![Block::genesis()],
            state: State::new(),
        }
    }

    pub fn height(&self) -> u64 {
        self.blocks.last().map_or(0, |b| b.height)
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
    
        // Apply transactions in order before accepting block.
        for tx in &block.transactions {
            self.state.apply_transaction(tx)?;
        }
    
        self.blocks.push(block);
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::Crypto;
    use crate::transaction::Transaction;
    use crate::types::Address;
    use ed25519_dalek::{Signer, SigningKey};

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
        Block {
            height: 1,
            previous_hash: prev_hash,
            timestamp_unix: 1_700_001_001,
            transactions: vec![sample_valid_tx()],
            block_hash: "block_1_hash".to_string(),
        }
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
        let block = Block {
            height: 2, // should be 1
            previous_hash: "GENESIS_HASH".to_string(),
            timestamp_unix: 1_700_001_002,
            transactions: vec![sample_valid_tx()],
            block_hash: "bad_height_block".to_string(),
        };

        let result = chain.append_block(block);
        assert!(matches!(result, Err(ProtocolError::InvalidBlock(_))));
        assert_eq!(chain.height(), 0);
        assert_eq!(chain.len(), 1);
    }

    #[test]
    fn append_block_rejects_invalid_previous_hash() {
        let mut chain = Blockchain::new();
        let block = Block {
            height: 1,
            previous_hash: "WRONG_HASH".to_string(),
            timestamp_unix: 1_700_001_003,
            transactions: vec![sample_valid_tx()],
            block_hash: "bad_prev_hash_block".to_string(),
        };

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

    let block = Block {
        height: 1,
        previous_hash: "GENESIS_HASH".to_string(),
        timestamp_unix: 1_700_002_001,
        transactions: vec![tx],
        block_hash: "state_apply_block".to_string(),
    };

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

    let block = Block {
        height: 1,
        previous_hash: "GENESIS_HASH".to_string(),
        timestamp_unix: 1_700_002_011,
        transactions: vec![tx],
        block_hash: "state_fail_block".to_string(),
    };

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

    let block = Block {
        height: 1,
        previous_hash: "GENESIS_HASH".to_string(),
        timestamp_unix: 1_700_002_021,
        transactions: vec![tx],
        block_hash: "height_guard_block".to_string(),
    };

    assert!(chain.append_block(block).is_err());
    assert_eq!(chain.height(), 0);
    assert_eq!(chain.len(), 1);
}
}