//! Trilogicon V3 **inert scaffolding**.
//!
//! This module tree is **not** connected to canonical commit, `append_block`,
//! `try_append_network_block`, sync, storage migration, or CLI. It exists so
//! height-first branch selection (repository `docs/fork_choice.md`)
//! and block indexing / [`reorg_plan::ReorgPlan`] shapes
//! (including [`block_index::BlockIndex::fork_slices_between_tips`])
//! and read-only [`reorg_preflight::ReorgPreflight`] reports
//! plus read-only [`replay_sandbox::ReplaySandbox`] ledger simulation reports
//! can be reviewed and unit-tested in isolation.
//!
//! **Do not** import `crate::network` (or any peer/session types) from
//! [`branch_select`](branch_select).

pub mod block_index;
pub mod branch_select;
pub mod reorg_plan;
pub mod reorg_preflight;
pub mod replay_sandbox;
