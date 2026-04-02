use crate::errors::ProtocolError;
use crate::types::Address;
use crate::crypto::Crypto;

#[derive(Clone, Debug)]
pub struct Transaction {
    pub sender: Address,
    pub receiver: Address,
    pub amount: u64,
    pub fee: u64,
    pub nonce: u64,
    pub timestamp_unix: u64,

    // Auth material (real verification comes in next step)
    pub public_key: Vec<u8>,
    pub signature: Vec<u8>,

    // Deterministic tx id (hash of canonical unsigned payload for now)
    pub tx_hash: String,
}

impl Transaction {

    pub fn validate_sender_binding(&self) -> Result<(), ProtocolError> {
        let derived = Crypto::address_from_public_key(&self.public_key);
        if self.sender.0 != derived {
            return Err(ProtocolError::InvalidAddress);
        }
        Ok(())
    }

    pub fn validate_signature(&self) -> Result<(), ProtocolError> {
        let payload = self.unsigned_payload_bytes();
        let is_valid = Crypto::verify_signature(&payload, &self.signature, &self.public_key);
        if !is_valid {
            return Err(ProtocolError::SignatureInvalid);
        }
        Ok(())
    }
    
    pub fn compute_tx_hash(&self) -> String {
        Crypto::hash_bytes(&self.unsigned_payload_bytes())
    }
    
    pub fn validate_hash(&self) -> Result<(), ProtocolError> {
        let expected = self.compute_tx_hash();
        if self.tx_hash != expected {
            return Err(ProtocolError::StateError("transaction hash mismatch".to_string()));
        }
        Ok(())
    }

    pub fn unsigned_payload_bytes(&self) -> Vec<u8> {
        // Canonical, explicit field order:
        // sender|receiver|amount|fee|nonce|timestamp
        format!(
            "{}|{}|{}|{}|{}|{}",
            self.sender.0,
            self.receiver.0,
            self.amount,
            self.fee,
            self.nonce,
            self.timestamp_unix
        )
        .into_bytes()
    }

    pub fn basic_validate(&self) -> Result<(), ProtocolError> {
        if !self.sender.is_valid() || !self.receiver.is_valid() {
            return Err(ProtocolError::InvalidAddress);
        }
        if self.amount == 0 {
            return Err(ProtocolError::InvalidAmount);
        }
        if self.public_key.is_empty() || self.signature.is_empty() {
            return Err(ProtocolError::SignatureInvalid);
        }
        if self.tx_hash.trim().is_empty() {
            return Err(ProtocolError::StateError("missing tx hash".to_string()));
        }

        self.validate_hash()?;
        self.validate_signature()?;
        self.validate_sender_binding()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::Crypto;
    use crate::types::Address;
    use ed25519_dalek::{Signer, SigningKey};

    fn sample_tx() -> Transaction {
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let verifying_key = signing_key.verifying_key();

        let mut tx = Transaction {
            sender: Address::new(Crypto::address_from_public_key(&verifying_key.to_bytes())),
            receiver: Address::new("bob"),
            amount: 10,
            fee: 1,
            nonce: 0,
            timestamp_unix: 1_717_171_717,
            public_key: verifying_key.to_bytes().to_vec(),
            signature: Vec::new(),
            tx_hash: String::new(),
        };

        let payload = tx.unsigned_payload_bytes();
        let sig = signing_key.sign(&payload);

        tx.signature = sig.to_bytes().to_vec();
        tx.tx_hash = Crypto::hash_bytes(&payload);
        tx
    }

    #[test]
    fn basic_validate_accepts_valid_hash_and_signature() {
        let tx = sample_tx();
        assert!(tx.basic_validate().is_ok());
    }

    #[test]
    fn basic_validate_rejects_hash_mismatch() {
        let mut tx = sample_tx();
        tx.amount = 11; // payload changed after hash/signature generation
        assert!(tx.basic_validate().is_err());
    }

    #[test]
    fn basic_validate_rejects_bad_signature() {
        let mut tx = sample_tx();
        tx.signature[0] ^= 0x01; // tamper signature bytes
        assert!(tx.basic_validate().is_err());
    }

    #[test]
    fn basic_validate_rejects_sender_public_key_mismatch() {
        let mut tx = sample_tx();
        tx.sender = Address::new("not_the_derived_address");
        assert!(tx.basic_validate().is_err());
    }
}