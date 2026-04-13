//! Stable `[subsystem]` prefixes for operator-facing stderr lines (reference `node` binary).
//!
//! Interpretation: see `README.md` (“Interpreting stderr”) and `docs/v2_scope.md`.

/// Startup: genesis load, data-dir binding, `chain.blocks` load on `run` / `send`.
pub const PFX_STARTUP: &str = "[startup]";

/// Local `chain.blocks` I/O, append/sync failure, in-process store poison.
pub const PFX_STORAGE: &str = "[storage]";

/// Outbound catch-up (`sync_from_peer`), per-call work budget (`stopped_due_to_budget`).
pub const PFX_SYNC: &str = "[sync]";

/// TCP sessions: handshake mismatch, wire rejects, inbound caps, gossip to a peer.
pub const PFX_PEER: &str = "[peer]";

/// In-memory mempool admits and committed-ledger purge drops.
pub const PFX_MEMPOOL: &str = "[mempool]";

/// Local seal attempt: `append_block_from_mempool` + persist outcome.
pub const PFX_SEAL: &str = "[seal]";

/// `pending_tx.tril` lock/drain (before mempool).
pub const PFX_PENDING: &str = "[pending]";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_are_stable_nonempty() {
        for (name, p) in [
            ("startup", PFX_STARTUP),
            ("storage", PFX_STORAGE),
            ("sync", PFX_SYNC),
            ("peer", PFX_PEER),
            ("mempool", PFX_MEMPOOL),
            ("seal", PFX_SEAL),
            ("pending", PFX_PENDING),
        ] {
            assert!(
                p.starts_with('[') && p.ends_with(']'),
                "{name}: expected bracketed tag, got {p:?}"
            );
        }
    }
}
