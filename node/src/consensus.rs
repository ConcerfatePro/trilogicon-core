//! V1 consensus policy: deterministic rules every node can apply from chain data alone,
//! plus an optional local clock check for gossiped blocks.
//!
//! Block validity (signatures, state, hashes) remains in [`crate::block`] / [`crate::state`].
//! This module only encodes **time policy** and a **producer placeholder** for future headers.

use crate::block::Block;
use crate::errors::ProtocolError;

/// Tunable timestamp policy. Defaults impose **no** parent interval and **no** future skew cap
/// so unit tests and offline replay stay permissive; testnets can tighten values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsensusParams {
    /// Minimum `candidate.timestamp_unix - parent.timestamp_unix` (saturation-safe).
    pub min_block_interval_secs: u64,
    /// When using [`validate_block_vs_local_time`], reject if `block_ts > now + this`.
    /// Set to `u64::MAX` in [`Default`] so the check is a no-op unless configured smaller.
    pub max_future_drift_secs: u64,
}

impl Default for ConsensusParams {
    fn default() -> Self {
        Self {
            min_block_interval_secs: 0,
            max_future_drift_secs: u64::MAX,
        }
    }
}

/// Reserved for a future block header field (e.g. proposer pubkey). V1 blocks carry no producer
/// identity on the wire; pinning a single producer is a **network operator** concern until then.
#[derive(Clone, Debug, Default)]
pub struct ProducerAuthorityV1Placeholder {
    /// When `Some`, future versions may require matching signed producer metadata on each block.
    pub expected_ed25519_public_key: Option<[u8; 32]>,
}

/// Enforce parent-relative timestamps: `candidate.timestamp_unix` must be at least
/// `parent.timestamp_unix + params.min_block_interval_secs`.
pub fn validate_block_timestamps_vs_parent(
    parent: &Block,
    candidate: &Block,
    params: &ConsensusParams,
) -> Result<(), ProtocolError> {
    let min_ts = parent.timestamp_unix.checked_add(params.min_block_interval_secs).ok_or_else(|| {
        ProtocolError::InvalidBlock("block timestamp arithmetic overflow".to_string())
    })?;

    if candidate.timestamp_unix < min_ts {
        return Err(ProtocolError::InvalidBlock(
            "block timestamp violates minimum interval after parent".to_string(),
        ));
    }

    Ok(())
}

/// Optional anti-spam / sanity check when a node has a local Unix clock (`now_unix`).
pub fn validate_block_vs_local_time(
    block_ts: u64,
    now_unix: u64,
    max_future_drift_secs: u64,
) -> Result<(), ProtocolError> {
    let limit = now_unix
        .checked_add(max_future_drift_secs)
        .unwrap_or(u64::MAX);
    if block_ts > limit {
        return Err(ProtocolError::InvalidBlock(
            "block timestamp too far in the future".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_params_allow_any_non_regressing_timestamps() {
        let parent = Block {
            height: 1,
            previous_hash: "x".into(),
            timestamp_unix: 100,
            transactions: vec![],
            block_hash: "y".into(),
        };
        let candidate_ok = Block {
            height: 2,
            previous_hash: "y".into(),
            timestamp_unix: 100,
            transactions: vec![],
            block_hash: "z".into(),
        };
        assert!(validate_block_timestamps_vs_parent(
            &parent,
            &candidate_ok,
            &ConsensusParams::default()
        )
        .is_ok());

        let candidate_early = Block {
            height: 2,
            previous_hash: "y".into(),
            timestamp_unix: 99,
            transactions: vec![],
            block_hash: "z".into(),
        };
        assert!(validate_block_timestamps_vs_parent(
            &parent,
            &candidate_early,
            &ConsensusParams::default()
        )
        .is_err());
    }

    #[test]
    fn min_interval_requires_gap() {
        let params = ConsensusParams {
            min_block_interval_secs: 5,
            max_future_drift_secs: 60,
        };
        let parent = Block {
            height: 1,
            previous_hash: "x".into(),
            timestamp_unix: 100,
            transactions: vec![],
            block_hash: "y".into(),
        };
        let too_soon = Block {
            height: 2,
            previous_hash: "y".into(),
            timestamp_unix: 104,
            transactions: vec![],
            block_hash: "z".into(),
        };
        assert!(validate_block_timestamps_vs_parent(&parent, &too_soon, &params).is_err());

        let ok = Block {
            height: 2,
            previous_hash: "y".into(),
            timestamp_unix: 105,
            transactions: vec![],
            block_hash: "z".into(),
        };
        assert!(validate_block_timestamps_vs_parent(&parent, &ok, &params).is_ok());
    }

    #[test]
    fn local_time_rejects_distant_future() {
        assert!(validate_block_vs_local_time(200, 100, 50).is_err());
        assert!(validate_block_vs_local_time(150, 100, 50).is_ok());
    }
}
