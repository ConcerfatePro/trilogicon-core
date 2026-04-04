//! Local signing helper: one Ed25519 keypair per wallet, TRIL address = hash(pubkey) (see [`crate::crypto`]).
//!
//! V1 scope: create keys, derive address, build a signed [`Transaction`]. Persistence and key derivation
//! (mnemonics, HD paths) are intentionally out of scope.

use std::fmt;

use ed25519_dalek::{Signer, SigningKey};
use rand::rngs::OsRng;
use rand::{CryptoRng, RngCore};

use crate::crypto::Crypto;
use crate::errors::ProtocolError;
use crate::transaction::Transaction;
use crate::types::Address;

/// Holds a signing key. Does not implement [`fmt::Debug`] to avoid leaking secret material to logs.
pub struct Wallet {
    signing_key: SigningKey,
}

impl Wallet {
    /// Random key from the OS RNG.
    pub fn generate() -> Self {
        Self::generate_with_rng(&mut OsRng)
    }

    /// Random key from a provided CSPRNG (tests, deterministic fixtures via [`StdRng`](rand::rngs::StdRng)).
    pub fn generate_with_rng<R: CryptoRng + RngCore>(rng: &mut R) -> Self {
        let mut secret = [0u8; 32];
        rng.fill_bytes(&mut secret);
        Self {
            signing_key: SigningKey::from_bytes(&secret),
        }
    }

    /// Restore from a 32-byte secret seed (same encoding as [`Self::seed_bytes`]).
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        Self {
            signing_key: SigningKey::from_bytes(seed),
        }
    }

    /// Secret seed bytes. Treat like a password: never log or persist in plaintext in production.
    pub fn seed_bytes(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }

    pub fn address(&self) -> Address {
        let vk = self.signing_key.verifying_key();
        Address::new(Crypto::address_from_public_key(&vk.to_bytes()))
    }

    pub fn verifying_key_bytes(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }

    /// Build and sign a transfer from this wallet's address. `receiver` must satisfy [`Address::is_valid`].
    pub fn sign_transfer(
        &self,
        receiver: Address,
        amount: u64,
        fee: u64,
        nonce: u64,
        timestamp_unix: u64,
    ) -> Result<Transaction, ProtocolError> {
        if amount == 0 {
            return Err(ProtocolError::InvalidAmount);
        }
        if !receiver.is_valid() {
            return Err(ProtocolError::InvalidAddress);
        }

        let vk = self.signing_key.verifying_key();
        let sender = Address::new(Crypto::address_from_public_key(&vk.to_bytes()));

        let mut tx = Transaction {
            sender,
            receiver,
            amount,
            fee,
            nonce,
            timestamp_unix,
            public_key: vk.to_bytes().to_vec(),
            signature: Vec::new(),
            tx_hash: String::new(),
        };

        let payload = tx.unsigned_payload_bytes();
        tx.signature = self.signing_key.sign(&payload).to_bytes().to_vec();
        tx.tx_hash = Crypto::hash_bytes(&payload);
        Ok(tx)
    }
}

impl Clone for Wallet {
    fn clone(&self) -> Self {
        Self {
            signing_key: SigningKey::from_bytes(&self.signing_key.to_bytes()),
        }
    }
}

impl fmt::Debug for Wallet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Wallet")
            .field("address", &self.address().0)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    #[test]
    fn sign_transfer_passes_basic_validate() {
        let w = Wallet::from_seed(&[9u8; 32]);
        let tx = w
            .sign_transfer(Address::new("recv_wallet"), 3, 1, 0, 1_700_100_000)
            .unwrap();
        assert!(tx.basic_validate().is_ok());
        assert_eq!(tx.sender, w.address());
    }

    #[test]
    fn from_seed_is_deterministic() {
        let seed = [5u8; 32];
        assert_eq!(
            Wallet::from_seed(&seed).address().0,
            Wallet::from_seed(&seed).address().0
        );
    }

    #[test]
    fn generate_with_rng_is_deterministic_for_seed() {
        let mut a = StdRng::seed_from_u64(42);
        let mut b = StdRng::seed_from_u64(42);
        assert_eq!(
            Wallet::generate_with_rng(&mut a).address().0,
            Wallet::generate_with_rng(&mut b).address().0
        );
    }

    #[test]
    fn sign_transfer_rejects_invalid_receiver() {
        let w = Wallet::from_seed(&[8u8; 32]);
        assert!(matches!(
            w.sign_transfer(Address::new(""), 1, 1, 0, 1),
            Err(ProtocolError::InvalidAddress)
        ));
    }

    #[test]
    fn sign_transfer_rejects_zero_amount() {
        let w = Wallet::from_seed(&[8u8; 32]);
        assert!(matches!(
            w.sign_transfer(Address::new("bob"), 0, 1, 0, 1),
            Err(ProtocolError::InvalidAmount)
        ));
    }
}
