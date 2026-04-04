use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Address(pub String);

impl Address {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn is_valid(&self) -> bool {
        // Placeholder format rule for V1 scaffold.
        !self.0.trim().is_empty() && self.0.len() <= 128
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub address: Address,
    pub balance: u64,
    pub nonce: u64,
}

impl Account {
    pub fn new(address: Address, balance: u64) -> Self {
        Self {
            address,
            balance,
            nonce: 0,
        }
    }
}
