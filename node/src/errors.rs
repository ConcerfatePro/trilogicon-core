use std::error::Error;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    InvalidAddress,
    InvalidAmount,
    InvalidFee,
    InvalidNonce,
    InsufficientBalance,
    SignatureInvalid,
    InvalidBlock(String),
    StateError(String),
    /// Mempool already holds this transaction id.
    DuplicateTransaction,
    /// Mempool has reached its configured capacity.
    MempoolFull,
    /// Another pending transaction already uses this sender and nonce (different tx id).
    MempoolNonceConflict,
    /// Invalid or conflicting genesis configuration.
    GenesisError(String),
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAddress => write!(f, "invalid address"),
            Self::InvalidAmount => write!(f, "invalid amount"),
            Self::InvalidFee => write!(f, "invalid fee"),
            Self::InvalidNonce => write!(f, "invalid nonce"),
            Self::InsufficientBalance => write!(f, "insufficient balance"),
            Self::SignatureInvalid => write!(f, "signature invalid"),
            Self::InvalidBlock(msg) => write!(f, "invalid block: {msg}"),
            Self::StateError(msg) => write!(f, "state error: {msg}"),
            Self::DuplicateTransaction => write!(f, "duplicate transaction"),
            Self::MempoolFull => write!(f, "mempool full"),
            Self::MempoolNonceConflict => write!(f, "mempool nonce conflict"),
            Self::GenesisError(msg) => write!(f, "genesis error: {msg}"),
        }
    }
}

impl Error for ProtocolError {}
