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
}
