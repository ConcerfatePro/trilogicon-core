use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

pub struct Crypto;

impl Crypto {
    pub fn hash_bytes(payload: &[u8]) -> String {
        let digest = Sha256::digest(payload);
        hex::encode(digest)
    }

    pub fn address_from_public_key(public_key: &[u8]) -> String {
        Self::hash_bytes(public_key)
    }

    pub fn verify_signature(message: &[u8], signature: &[u8], public_key: &[u8]) -> bool {
        let Ok(pk_bytes): Result<[u8; 32], _> = public_key.try_into() else {
            return false;
        };
        let Ok(sig_bytes): Result<[u8; 64], _> = signature.try_into() else {
            return false;
        };

        let Ok(verifying_key) = VerifyingKey::from_bytes(&pk_bytes) else {
            return false;
        };

        let sig = Signature::from_bytes(&sig_bytes);
        verifying_key.verify(message, &sig).is_ok()
    }
}
