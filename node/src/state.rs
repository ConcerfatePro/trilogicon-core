use std::collections::HashMap;

use crate::errors::ProtocolError;
use crate::transaction::Transaction;
use crate::types::{Account, Address};

#[derive(Default)]
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
