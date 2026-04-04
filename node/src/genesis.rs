//! Protocol-defined initial ledger state (V1).
//!
//! The fixed genesis **block** is [`Block::genesis`]. This module defines the matching **state**
//! at height 0: explicit balance allocations. Every honest node must use the same genesis
//! document for the same network; otherwise replay diverges.
//!
//! See `docs/genesis.md` for operator guidance.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::crypto::Crypto;
use crate::errors::ProtocolError;
use crate::types::Address;

/// One genesis balance line (from TOML `[[allocations]]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenesisAllocation {
    pub address: String,
    pub balance: u64,
}

/// Full genesis document: deterministic initial accounts before any non-genesis block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Genesis {
    /// Declared allocations (may be reordered when building state; duplicates are rejected).
    #[serde(default)]
    pub allocations: Vec<GenesisAllocation>,
}

impl Genesis {
    /// No initial accounts (only the empty state at height 0). Useful for unit tests that fund manually.
    pub fn empty() -> Self {
        Self {
            allocations: Vec::new(),
        }
    }

    /// Parse TOML from disk. Canonical form uses `[[allocations]]` tables with `address` and `balance`.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, String> {
        let raw = fs::read_to_string(path.as_ref()).map_err(|e| e.to_string())?;
        Self::from_toml_str(&raw)
    }

    pub fn from_toml_str(s: &str) -> Result<Self, String> {
        let g: Genesis = toml::from_str(s).map_err(|e| format!("genesis TOML: {e}"))?;
        g.validate_decl().map_err(|e| e.to_string())?;
        Ok(g)
    }

    /// Write TOML for operators to share (same format as [`Self::from_path`]).
    pub fn write_to_path(&self, path: impl AsRef<Path>) -> Result<(), String> {
        self.validate_decl().map_err(|e| e.to_string())?;
        let body = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        let out = format!(
            "# Trilogicon V1 genesis — allocations at height 0 (before first non-genesis block).\n\
             # State commitment (compare across nodes): {}\n\n{body}",
            self.state_commitment_hex().map_err(|e| e.to_string())?
        );
        fs::write(path.as_ref(), out).map_err(|e| e.to_string())
    }

    fn validate_decl(&self) -> Result<(), ProtocolError> {
        for a in &self.allocations {
            if !Address::new(a.address.clone()).is_valid() {
                return Err(ProtocolError::GenesisError(format!(
                    "invalid address in genesis: {:?}",
                    a.address
                )));
            }
            if a.balance == 0 {
                return Err(ProtocolError::GenesisError(format!(
                    "zero balance not allowed for {}",
                    a.address
                )));
            }
        }
        Ok(())
    }

    /// Sorted `(Address, balance)` for state construction and hashing.
    pub fn sorted_pairs(&self) -> Result<Vec<(Address, u64)>, ProtocolError> {
        self.validate_decl()?;
        let mut pairs: Vec<(Address, u64)> = self
            .allocations
            .iter()
            .map(|a| (Address::new(a.address.clone()), a.balance))
            .collect();
        pairs.sort_by(|x, y| x.0.0.cmp(&y.0.0));
        for w in pairs.windows(2) {
            if w[0].0 == w[1].0 {
                return Err(ProtocolError::GenesisError(format!(
                    "duplicate genesis address: {}",
                    w[0].0
                )));
            }
        }
        Ok(pairs)
    }

    /// Commitment over allocations (sorted). Operators compare hex to ensure they share genesis.
    pub fn state_commitment_hex(&self) -> Result<String, ProtocolError> {
        let pairs = self.sorted_pairs()?;
        let lines: Vec<String> = pairs
            .iter()
            .map(|(a, b)| format!("{}|{}", a.0, b))
            .collect();
        let preimage = lines.join("\n");
        Ok(Crypto::hash_bytes(preimage.as_bytes()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_duplicate_address() {
        let g = Genesis {
            allocations: vec![
                GenesisAllocation {
                    address: "alice".into(),
                    balance: 1,
                },
                GenesisAllocation {
                    address: "alice".into(),
                    balance: 2,
                },
            ],
        };
        assert!(g.sorted_pairs().is_err());
    }

    #[test]
    fn from_toml_roundtrip() {
        let raw = r#"
[[allocations]]
address = "bob"
balance = 10

[[allocations]]
address = "alice"
balance = 20
"#;
        let g = Genesis::from_toml_str(raw).unwrap();
        assert_eq!(g.allocations.len(), 2);
        let h = g.state_commitment_hex().unwrap();
        let g2 = Genesis::from_toml_str(raw).unwrap();
        assert_eq!(g2.state_commitment_hex().unwrap(), h);
    }

    #[test]
    fn commitment_stable_under_input_order() {
        let a = Genesis {
            allocations: vec![
                GenesisAllocation {
                    address: "b".into(),
                    balance: 2,
                },
                GenesisAllocation {
                    address: "a".into(),
                    balance: 1,
                },
            ],
        };
        let b = Genesis {
            allocations: vec![
                GenesisAllocation {
                    address: "a".into(),
                    balance: 1,
                },
                GenesisAllocation {
                    address: "b".into(),
                    balance: 2,
                },
            ],
        };
        assert_eq!(
            a.state_commitment_hex().unwrap(),
            b.state_commitment_hex().unwrap()
        );
    }
}
