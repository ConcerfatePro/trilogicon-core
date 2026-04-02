use std::collections::VecDeque;

use crate::transaction::Transaction;

pub struct Mempool {
    capacity: usize,
    txs: VecDeque<Transaction>,
}

impl Mempool {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            txs: VecDeque::new(),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn push(&mut self, tx: Transaction) {
        if self.txs.len() >= self.capacity {
            self.txs.pop_front();
        }
        self.txs.push_back(tx);
    }

    pub fn len(&self) -> usize {
        self.txs.len()
    }
}
