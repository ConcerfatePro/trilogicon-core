use std::collections::HashMap;

use crate::errors::ProtocolError;
use crate::transaction::Transaction;
use crate::types::{Account, Address};

#[derive(Clone, Default)]
pub struct State {
    accounts: HashMap<Address, Account>,
}

impl State {
    pub fn new() -> Self {
        Self {
            accounts: HashMap::new(),
        }
    }

    pub fn account_count(&self) -> usize {
        self.accounts.len()
    }

    pub fn get_account(&self, address: &Address) -> Option<&Account> {
        self.accounts.get(address)
    }

    pub fn create_account(&mut self, address: Address, initial_balance: u64) {
        self.accounts
            .entry(address.clone())
            .or_insert_with(|| Account::new(address, initial_balance));
    }

    pub fn apply_transaction(&mut self, tx: &Transaction) -> Result<(), ProtocolError> {
        tx.basic_validate()?;

        let sender = self
            .accounts
            .get_mut(&tx.sender)
            .ok_or_else(|| ProtocolError::StateError(String::from("sender account missing")))?;

        if sender.nonce != tx.nonce {
            return Err(ProtocolError::InvalidNonce);
        }

        let total_cost = tx
            .amount
            .checked_add(tx.fee)
            .ok_or_else(|| ProtocolError::StateError(String::from("amount+fee overflow")))?;

        if sender.balance < total_cost {
            return Err(ProtocolError::InsufficientBalance);
        }

        sender.balance -= total_cost;
        sender.nonce += 1;

        let receiver = self
            .accounts
            .entry(tx.receiver.clone())
            .or_insert_with(|| Account::new(tx.receiver.clone(), 0));
        receiver.balance += tx.amount;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::Crypto;
    use crate::transaction::Transaction;
    use ed25519_dalek::{Signer, SigningKey};

    fn make_signed_tx(
        signing_key: &SigningKey,
        receiver: Address,
        amount: u64,
        fee: u64,
        nonce: u64,
        timestamp_unix: u64,
    ) -> Transaction {
        let verifying_key = signing_key.verifying_key();
        let sender = Address::new(Crypto::address_from_public_key(&verifying_key.to_bytes()));

        let mut tx = Transaction {
            sender,
            receiver,
            amount,
            fee,
            nonce,
            timestamp_unix,
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
    fn apply_transaction_updates_balances_and_nonce() {
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let sender_vk = signing_key.verifying_key();
        let sender_addr = Address::new(Crypto::address_from_public_key(&sender_vk.to_bytes()));
        let receiver_addr = Address::new("receiver_1");

        let mut state = State::new();
        state.create_account(sender_addr.clone(), 100);

        let tx = make_signed_tx(&signing_key, receiver_addr.clone(), 10, 1, 0, 1_700_000_000);
        state.apply_transaction(&tx).unwrap();

        let sender = state.get_account(&sender_addr).unwrap();
        let receiver = state.get_account(&receiver_addr).unwrap();

        assert_eq!(sender.balance, 89); // 100 - (10 + 1)
        assert_eq!(sender.nonce, 1);
        assert_eq!(receiver.balance, 10);
    }

    #[test]
    fn apply_transaction_rejects_stale_nonce_without_state_change() {
        let signing_key = SigningKey::from_bytes(&[8u8; 32]);
        let sender_vk = signing_key.verifying_key();
        let sender_addr = Address::new(Crypto::address_from_public_key(&sender_vk.to_bytes()));
        let receiver_addr = Address::new("receiver_2");

        let mut state = State::new();
        state.create_account(sender_addr.clone(), 100);

        let tx = make_signed_tx(&signing_key, receiver_addr.clone(), 10, 1, 1, 1_700_000_001);
        let result = state.apply_transaction(&tx);
        assert!(matches!(result, Err(ProtocolError::InvalidNonce)));

        let sender = state.get_account(&sender_addr).unwrap();
        assert_eq!(sender.balance, 100);
        assert_eq!(sender.nonce, 0);
        assert!(state.get_account(&receiver_addr).is_none());
    }

    #[test]
    fn apply_transaction_rejects_insufficient_balance_without_state_change() {
        let signing_key = SigningKey::from_bytes(&[9u8; 32]);
        let sender_vk = signing_key.verifying_key();
        let sender_addr = Address::new(Crypto::address_from_public_key(&sender_vk.to_bytes()));
        let receiver_addr = Address::new("receiver_3");

        let mut state = State::new();
        state.create_account(sender_addr.clone(), 5);

        let tx = make_signed_tx(&signing_key, receiver_addr.clone(), 10, 1, 0, 1_700_000_002);
        let result = state.apply_transaction(&tx);
        assert!(matches!(result, Err(ProtocolError::InsufficientBalance)));

        let sender = state.get_account(&sender_addr).unwrap();
        assert_eq!(sender.balance, 5);
        assert_eq!(sender.nonce, 0);
        assert!(state.get_account(&receiver_addr).is_none());
    }

    #[test]
    fn apply_transaction_rejects_missing_sender_account() {
        let signing_key = SigningKey::from_bytes(&[10u8; 32]);
        let receiver_addr = Address::new("receiver_4");

        let mut state = State::new();

        let tx = make_signed_tx(&signing_key, receiver_addr, 1, 1, 0, 1_700_000_003);
        let result = state.apply_transaction(&tx);
        assert!(matches!(result, Err(ProtocolError::StateError(_))));
    }
}