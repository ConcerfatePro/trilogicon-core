use std::collections::HashMap;

use crate::errors::ProtocolError;
use crate::genesis::Genesis;
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

    /// Height-0 state from protocol genesis (sorted allocations, no duplicates).
    pub fn from_genesis(genesis: &Genesis) -> Result<Self, ProtocolError> {
        let pairs = genesis.sorted_pairs()?;
        let mut state = Self::new();
        for (addr, balance) in pairs {
            let key = addr.clone();
            state.accounts.insert(key, Account::new(addr, balance));
        }
        Ok(state)
    }

    /// Deterministic account list for tests and debugging (`address` ascending).
    pub fn accounts_sorted(&self) -> Vec<(Address, Account)> {
        let mut v: Vec<_> = self
            .accounts
            .iter()
            .map(|(a, ac)| (a.clone(), ac.clone()))
            .collect();
        v.sort_by(|x, y| x.0.0.cmp(&y.0.0));
        v
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

    /// Sum of all account balances. Used for supply audits; uses checked arithmetic.
    pub fn total_balance_sum(&self) -> Result<u64, ProtocolError> {
        self.accounts.values().try_fold(0u64, |acc, a| {
            acc.checked_add(a.balance).ok_or_else(|| {
                ProtocolError::StateError(String::from("total balance sum overflow"))
            })
        })
    }

    /// Apply one validated transfer. All arithmetic is **checked** before any mutation; overflow
    /// paths return [`ProtocolError::StateError`] and leave `self` unchanged.
    pub fn apply_transaction(&mut self, tx: &Transaction) -> Result<(), ProtocolError> {
        tx.basic_validate()?;

        if tx.sender == tx.receiver {
            return self.apply_self_transfer(tx);
        }

        let total_cost = tx
            .amount
            .checked_add(tx.fee)
            .ok_or_else(|| ProtocolError::StateError(String::from("amount+fee overflow")))?;

        let (sender_balance, sender_nonce) = {
            let sender = self
                .accounts
                .get(&tx.sender)
                .ok_or_else(|| ProtocolError::StateError(String::from("sender account missing")))?;
            (sender.balance, sender.nonce)
        };

        if sender_nonce != tx.nonce {
            return Err(ProtocolError::InvalidNonce);
        }

        if sender_balance < total_cost {
            return Err(ProtocolError::InsufficientBalance);
        }

        let new_sender_balance = sender_balance.checked_sub(total_cost).ok_or_else(|| {
            ProtocolError::StateError(String::from("sender balance debit underflow"))
        })?;
        let new_sender_nonce = sender_nonce
            .checked_add(1)
            .ok_or_else(|| ProtocolError::StateError(String::from("sender nonce overflow")))?;

        let new_receiver_balance = match self.accounts.get(&tx.receiver) {
            Some(r) => r.balance.checked_add(tx.amount).ok_or_else(|| {
                ProtocolError::StateError(String::from("receiver balance overflow"))
            })?,
            None => tx.amount,
        };

        {
            let sender = self.accounts.get_mut(&tx.sender).expect("sender exists");
            sender.balance = new_sender_balance;
            sender.nonce = new_sender_nonce;
        }

        let receiver = self
            .accounts
            .entry(tx.receiver.clone())
            .or_insert_with(|| Account::new(tx.receiver.clone(), 0));
        receiver.balance = new_receiver_balance;

        // V1: `fee` is burned (deducted from sender, not credited to any account).

        Ok(())
    }

    fn apply_self_transfer(&mut self, tx: &Transaction) -> Result<(), ProtocolError> {
        let total_cost = tx
            .amount
            .checked_add(tx.fee)
            .ok_or_else(|| ProtocolError::StateError(String::from("amount+fee overflow")))?;

        let acc = self
            .accounts
            .get_mut(&tx.sender)
            .ok_or_else(|| ProtocolError::StateError(String::from("sender account missing")))?;

        if acc.nonce != tx.nonce {
            return Err(ProtocolError::InvalidNonce);
        }
        if acc.balance < total_cost {
            return Err(ProtocolError::InsufficientBalance);
        }

        let new_balance = acc
            .balance
            .checked_sub(total_cost)
            .and_then(|b| b.checked_add(tx.amount))
            .ok_or_else(|| ProtocolError::StateError(String::from("balance update overflow")))?;
        let new_nonce = acc
            .nonce
            .checked_add(1)
            .ok_or_else(|| ProtocolError::StateError(String::from("sender nonce overflow")))?;

        acc.balance = new_balance;
        acc.nonce = new_nonce;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn replace_account_for_tests(&mut self, account: Account) {
        self.accounts.insert(account.address.clone(), account);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::Crypto;
    use crate::transaction::Transaction;
    use crate::types::Account;
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
    fn apply_transaction_reduces_total_supply_by_fee() {
        let signing_key = SigningKey::from_bytes(&[11u8; 32]);
        let sender_vk = signing_key.verifying_key();
        let sender_addr = Address::new(Crypto::address_from_public_key(&sender_vk.to_bytes()));
        let receiver_addr = Address::new("receiver_supply");

        let mut state = State::new();
        state.create_account(sender_addr.clone(), 100);

        let before = state.total_balance_sum().unwrap();
        let tx = make_signed_tx(&signing_key, receiver_addr.clone(), 10, 3, 0, 1_700_000_100);
        state.apply_transaction(&tx).unwrap();
        let after = state.total_balance_sum().unwrap();

        assert_eq!(before, 100);
        assert_eq!(after, 97); // 100 - fee 3; receiver gained 10, sender lost 13
        assert_eq!(
            after,
            before
                .checked_sub(tx.fee)
                .expect("fee <= before for this fixture")
        );
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

    #[test]
    fn apply_transaction_rejects_replay_same_signed_tx() {
        let signing_key = SigningKey::from_bytes(&[12u8; 32]);
        let sender_vk = signing_key.verifying_key();
        let sender_addr = Address::new(Crypto::address_from_public_key(&sender_vk.to_bytes()));
        let receiver_addr = Address::new("receiver_replay");

        let mut state = State::new();
        state.create_account(sender_addr.clone(), 100);

        let tx = make_signed_tx(&signing_key, receiver_addr.clone(), 5, 1, 0, 1_700_000_500);
        state.apply_transaction(&tx).unwrap();

        let replay = state.apply_transaction(&tx);
        assert!(matches!(replay, Err(ProtocolError::InvalidNonce)));

        let sender = state.get_account(&sender_addr).unwrap();
        assert_eq!(sender.nonce, 1);
        assert_eq!(sender.balance, 94);
        assert_eq!(state.get_account(&receiver_addr).unwrap().balance, 5);
    }

    #[test]
    fn apply_transaction_rejects_receiver_balance_overflow_without_mutation() {
        let signing_key = SigningKey::from_bytes(&[13u8; 32]);
        let sender_vk = signing_key.verifying_key();
        let sender_addr = Address::new(Crypto::address_from_public_key(&sender_vk.to_bytes()));
        let receiver_addr = Address::new("recv_ovf");

        let mut state = State::new();
        state.create_account(sender_addr.clone(), 500);
        state.replace_account_for_tests(Account::new(receiver_addr.clone(), u64::MAX));

        let tx = make_signed_tx(&signing_key, receiver_addr.clone(), 1, 1, 0, 1_700_000_600);
        let r = state.apply_transaction(&tx);
        assert!(
            matches!(r, Err(ProtocolError::StateError(ref m)) if m.contains("receiver balance overflow")),
            "unexpected: {r:?}"
        );
        assert_eq!(state.get_account(&sender_addr).unwrap().balance, 500);
        assert_eq!(state.get_account(&sender_addr).unwrap().nonce, 0);
        assert_eq!(state.get_account(&receiver_addr).unwrap().balance, u64::MAX);
    }

    #[test]
    fn apply_transaction_rejects_sender_nonce_overflow_without_mutation() {
        let signing_key = SigningKey::from_bytes(&[14u8; 32]);
        let sender_vk = signing_key.verifying_key();
        let sender_addr = Address::new(Crypto::address_from_public_key(&sender_vk.to_bytes()));
        let receiver_addr = Address::new("recv_nonce_max");

        let mut state = State::new();
        state.replace_account_for_tests(Account {
            address: sender_addr.clone(),
            balance: 1_000,
            nonce: u64::MAX,
        });

        let tx = make_signed_tx(
            &signing_key,
            receiver_addr.clone(),
            1,
            1,
            u64::MAX,
            1_700_000_700,
        );
        let r = state.apply_transaction(&tx);
        assert!(
            matches!(r, Err(ProtocolError::StateError(ref m)) if m.contains("nonce overflow")),
            "unexpected: {r:?}"
        );
        assert_eq!(state.get_account(&sender_addr).unwrap().nonce, u64::MAX);
        assert_eq!(state.get_account(&sender_addr).unwrap().balance, 1_000);
        assert!(state.get_account(&receiver_addr).is_none());
    }
}
