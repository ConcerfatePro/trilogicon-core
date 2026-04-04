use std::collections::HashSet;

use crate::crypto::Crypto;
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

    /// Canonical preimage for block identity (V1).
    /// Field order: height | previous_hash | timestamp | tx_hash_0 | tx_hash_1 | ...
    /// Every included transaction must already pass `basic_validate` so `tx_hash` is meaningful.
    pub fn block_header_preimage_bytes(&self) -> Vec<u8> {
        let tx_hashes: Vec<&str> = self.transactions.iter().map(|t| t.tx_hash.as_str()).collect();
        let joined = tx_hashes.join("|");
        format!(
            "{}|{}|{}|{}",
            self.height, self.previous_hash, self.timestamp_unix, joined
        )
        .into_bytes()
    }

    pub fn compute_block_hash(&self) -> String {
        Crypto::hash_bytes(&self.block_header_preimage_bytes())
    }

    pub fn validate_block_hash(&self) -> Result<(), ProtocolError> {
        if self.block_hash.trim().is_empty() {
            return Err(ProtocolError::InvalidBlock("missing block hash".to_string()));
        }
        let expected = self.compute_block_hash();
        if self.block_hash != expected {
            return Err(ProtocolError::InvalidBlock(
                "block hash mismatch".to_string(),
            ));
        }
        Ok(())
    }

    pub fn basic_validate(&self) -> Result<(), ProtocolError> {
        if self.is_genesis() {
            if self.previous_hash != "GENESIS" || self.block_hash != "GENESIS_HASH" {
                return Err(ProtocolError::InvalidBlock(
                    "invalid genesis constants".to_string(),
                ));
            }
            if !self.transactions.is_empty() {
                return Err(ProtocolError::InvalidBlock(
                    "genesis must not contain transactions".to_string(),
                ));
            }
            return Ok(());
        }

        if self.previous_hash.trim().is_empty() {
            return Err(ProtocolError::InvalidBlock(
                "missing previous hash".to_string(),
            ));
        }

        let mut seen_tx_hashes: HashSet<&str> = HashSet::new();
        for tx in &self.transactions {
            if !seen_tx_hashes.insert(tx.tx_hash.as_str()) {
                return Err(ProtocolError::DuplicateTransaction);
            }
        }

        for tx in &self.transactions {
            tx.basic_validate()?;
        }

        self.validate_block_hash()?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::Crypto;
    use crate::errors::ProtocolError;
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
    fn genesis_rejects_non_empty_transactions() {
        let mut block = Block::genesis();
        block.transactions.push(sample_valid_tx());
        assert!(matches!(
            block.basic_validate(),
            Err(ProtocolError::InvalidBlock(_))
        ));
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
            block_hash: "will_not_be_checked".to_string(),
        };
        assert!(block.basic_validate().is_err());
    }

    #[test]
    fn non_genesis_accepts_valid_structure_and_transactions() {
        let mut block = Block {
            height: 1,
            previous_hash: "prev_hash".to_string(),
            timestamp_unix: 1_700_000_104,
            transactions: vec![sample_valid_tx()],
            block_hash: String::new(),
        };
        block.block_hash = block.compute_block_hash();
        assert!(block.basic_validate().is_ok());
    }

    #[test]
    fn non_genesis_rejects_block_hash_mismatch() {
        let mut block = Block {
            height: 1,
            previous_hash: "prev_hash".to_string(),
            timestamp_unix: 1_700_000_105,
            transactions: vec![sample_valid_tx()],
            block_hash: String::new(),
        };
        block.block_hash = block.compute_block_hash();
        block.timestamp_unix += 1; // preimage changed, hash now wrong
        assert!(block.basic_validate().is_err());
    }

    #[test]
    fn non_genesis_rejects_duplicate_transaction_id() {
        let tx = sample_valid_tx();
        let mut block = Block {
            height: 1,
            previous_hash: "prev_hash".to_string(),
            timestamp_unix: 1_700_000_106,
            transactions: vec![tx.clone(), tx],
            block_hash: String::new(),
        };
        block.block_hash = block.compute_block_hash();
        assert!(matches!(
            block.basic_validate(),
            Err(ProtocolError::DuplicateTransaction)
        ));
    }
}