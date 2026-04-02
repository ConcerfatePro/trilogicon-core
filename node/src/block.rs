use crate::errors::ProtocolError;
use crate::transaction::Transaction;

#[derive(Clone, Debug)]
pub struct Block {
    pub height: u64,
    pub previous_hash: String,
    pub timestamp_unix: u64,
    pub transactions: Vec<Transaction>,
    pub block_hash: String,
}

impl Block {
    pub fn genesis() -> Self {
        Self {
            height: 0,
            previous_hash: String::from("GENESIS"),
            timestamp_unix: 0,
            transactions: Vec::new(),
            block_hash: String::from("GENESIS_HASH"),
        }
    }

    pub fn is_genesis(&self) -> bool {
        self.height == 0
    }

    pub fn basic_validate(&self) -> Result<(), ProtocolError> {
        if self.is_genesis() {
            if self.previous_hash != "GENESIS" || self.block_hash != "GENESIS_HASH" {
                return Err(ProtocolError::InvalidBlock(
                    "invalid genesis constants".to_string(),
                ));
            }
            return Ok(());
        }

        if self.previous_hash.trim().is_empty() {
            return Err(ProtocolError::InvalidBlock(
                "missing previous hash".to_string(),
            ));
        }

        if self.block_hash.trim().is_empty() {
            return Err(ProtocolError::InvalidBlock("missing block hash".to_string()));
        }

        for tx in &self.transactions {
            tx.basic_validate()?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::Crypto;
    use crate::types::Address;
    use ed25519_dalek::{Signer, SigningKey};

    fn sample_valid_tx() -> Transaction {
        let signing_key = SigningKey::from_bytes(&[11u8; 32]);
        let verifying_key = signing_key.verifying_key();

        let mut tx = Transaction {
            sender: Address::new(Crypto::address_from_public_key(&verifying_key.to_bytes())),
            receiver: Address::new("receiver_block_test"),
            amount: 10,
            fee: 1,
            nonce: 0,
            timestamp_unix: 1_700_000_100,
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
    fn genesis_block_is_valid() {
        let block = Block::genesis();
        assert!(block.basic_validate().is_ok());
    }

    #[test]
    fn non_genesis_rejects_missing_previous_hash() {
        let block = Block {
            height: 1,
            previous_hash: String::new(),
            timestamp_unix: 1_700_000_101,
            transactions: vec![sample_valid_tx()],
            block_hash: "abc123".to_string(),
        };
        assert!(block.basic_validate().is_err());
    }

    #[test]
    fn non_genesis_rejects_missing_block_hash() {
        let block = Block {
            height: 1,
            previous_hash: "prev_hash".to_string(),
            timestamp_unix: 1_700_000_102,
            transactions: vec![sample_valid_tx()],
            block_hash: String::new(),
        };
        assert!(block.basic_validate().is_err());
    }

    #[test]
    fn non_genesis_rejects_invalid_transaction() {
        let mut bad_tx = sample_valid_tx();
        bad_tx.amount = 0; // breaks tx basic validation

        let block = Block {
            height: 1,
            previous_hash: "prev_hash".to_string(),
            timestamp_unix: 1_700_000_103,
            transactions: vec![bad_tx],
            block_hash: "block_hash".to_string(),
        };
        assert!(block.basic_validate().is_err());
    }

    #[test]
    fn non_genesis_accepts_valid_structure_and_transactions() {
        let block = Block {
            height: 1,
            previous_hash: "prev_hash".to_string(),
            timestamp_unix: 1_700_000_104,
            transactions: vec![sample_valid_tx()],
            block_hash: "block_hash".to_string(),
        };
        assert!(block.basic_validate().is_ok());
    }
}