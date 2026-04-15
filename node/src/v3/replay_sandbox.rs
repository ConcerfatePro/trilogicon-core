//! **Inert** read-only **replay sandbox** report — preparatory V3 scaffolding only (V3-07).
//!
//! Pure, library-local simulation: validates a [`ReorgPlan`](super::reorg_plan::ReorgPlan) against a
//! [`BlockIndex`](super::block_index::BlockIndex), applies **local preflight policy** via
//! [`ReorgPreflight`](super::reorg_preflight::ReorgPreflight), then **optionally** replays the plan on
//! **cloned** [`State`](crate::state::State) using caller-supplied [`Block`](crate::block::Block) bodies.
//!
//! **Guarantees:** no canonical [`Blockchain`](crate::blockchain::Blockchain) mutation, no
//! `append_block` / network / sync / storage / CLI wiring, no persistence, no automatic reorg
//! execution. All ledger work happens on in-memory clones only.
//!
//! ## Separation of concerns (report sections)
//!
//! 1. **Structural** — [`ReorgPlan::validate_against_index`](super::reorg_plan::ReorgPlan::validate_against_index).
//! 2. **Preflight / local policy** — [`ReorgPreflight::evaluate`](super::reorg_preflight::ReorgPreflight::evaluate)
//!    (policy is not evaluated when structural validation fails).
//! 3. **Sandbox replay** — runs only when preflight verdict is [`ReorgPreflightVerdict::NoOp`] or
//!    [`ReorgPreflightVerdict::Acceptable`]; otherwise skipped without touching [`ReplaySandboxMaterial`].

use std::collections::HashMap;

use crate::block::Block;
use crate::crypto::Crypto;
use crate::state::State;

use super::block_index::BlockIndex;
use super::reorg_plan::{ReorgPlan, ReorgPlanValidationError};
use super::reorg_preflight::{
    ReorgPreflight, ReorgPreflightPolicy, ReorgPreflightReport, ReorgPreflightVerdict,
};

/// Caller-supplied replay material: fork state snapshot, full blocks for every planned hash, and an
/// optional expected post–old-tip state for rollback verification.
///
/// **Inert:** this struct is never written by the sandbox; only read and cloned.
#[derive(Clone)]
pub struct ReplaySandboxMaterial {
    /// Ledger state immediately after the fork block (height of `plan.fork_hash`), used as the
    /// replay starting point after simulated rollback.
    pub state_at_fork: State,
    /// Block bodies keyed by the same `block_hash` strings used in `plan` and `index`.
    pub blocks_by_hash: HashMap<String, Block>,
    /// When `plan.rollback_ordered` is non-empty, must be `Some` so the sandbox can verify the old
    /// branch forward-replays from [`Self::state_at_fork`] to the committed old tip.
    pub expected_state_at_old_tip: Option<State>,
}

/// Deterministic digest of account rows (`address|balance|nonce`, sorted by address).
///
/// **Inert scaffolding:** stable enough for tests and operator reports; not a consensus commitment.
pub fn sandbox_state_fingerprint(state: &State) -> String {
    let lines: Vec<String> = state
        .accounts_sorted()
        .into_iter()
        .map(|(a, ac)| format!("{}|{}|{}", a.0, ac.balance, ac.nonce))
        .collect();
    Crypto::hash_bytes(lines.join("\n").as_bytes())
}

/// Replay-time failures after structural + preflight gates passed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplaySandboxReplayError {
    /// `rollback_ordered` is non-empty but [`ReplaySandboxMaterial::expected_state_at_old_tip`] is `None`.
    MissingExpectedOldTipState,
    /// No [`Block`] provided for a hash referenced by the plan.
    MissingBlockBody { block_hash: String },
    /// [`Block`] header fields disagree with [`BlockIndex`] for the same hash.
    BlockHeaderMismatch {
        block_hash: String,
        detail: &'static str,
    },
    /// Forward walk along the old branch reached a different ledger than the caller expected.
    OldTipStateMismatch {
        expected_fingerprint: String,
        replayed_fingerprint: String,
    },
    /// `basic_validate` failed for a replayed block body.
    InvalidBlockBody {
        block_hash: String,
        error: String,
    },
    /// `apply_transaction` failed while simulating a block.
    StateTransition {
        block_hash: String,
        error: String,
    },
    /// [`State::total_balance_sum`] failed on the final cloned state (supply audit).
    SupplyAudit(String),
}

/// Successful sandbox replay summary (all values derived from cloned state only).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplaySandboxReplayOk {
    pub rollback_blocks_verified: usize,
    pub apply_blocks_simulated: usize,
    pub final_state_fingerprint: String,
    pub final_supply_total: u64,
}

/// Whether the sandbox attempted ledger simulation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReplaySandboxReplayPhase {
    /// Structural validation failed; replay is meaningless.
    SkippedDueToStructuralInvalid,
    /// Structural OK but [`ReorgPreflightVerdict::Rejected`].
    SkippedDueToPolicy,
    /// Preflight allowed replay; see inner result.
    Completed(Result<ReplaySandboxReplayOk, ReplaySandboxReplayError>),
}

/// Read-only report combining structural validation, local preflight, and optional replay.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplaySandboxReport {
    /// Pure structural gate from [`ReorgPlan::validate_against_index`].
    pub structural: Result<(), ReorgPlanValidationError>,
    /// Full preflight report (structural is duplicated here for operator clarity).
    pub preflight: ReorgPreflightReport,
    /// Isolated replay outcome (fork-only clones).
    pub replay: ReplaySandboxReplayPhase,
}

/// Namespace for the replay sandbox entrypoint (no instance state).
pub struct ReplaySandbox;

impl ReplaySandbox {
    /// Evaluate `plan` against `index` and `policy`, then simulate replay on clones of `material`.
    pub fn run_report(
        plan: &ReorgPlan,
        index: &BlockIndex,
        policy: &ReorgPreflightPolicy,
        material: &ReplaySandboxMaterial,
    ) -> ReplaySandboxReport {
        let structural = plan.validate_against_index(index);
        let preflight = ReorgPreflight::evaluate(plan, index, policy);

        let replay = match (&structural, preflight.verdict) {
            (Err(_), _) => ReplaySandboxReplayPhase::SkippedDueToStructuralInvalid,
            (Ok(_), ReorgPreflightVerdict::Rejected) => ReplaySandboxReplayPhase::SkippedDueToPolicy,
            (Ok(_), ReorgPreflightVerdict::NoOp | ReorgPreflightVerdict::Acceptable) => {
                ReplaySandboxReplayPhase::Completed(run_replay_on_clones(plan, index, material))
            }
        };

        ReplaySandboxReport {
            structural,
            preflight,
            replay,
        }
    }
}

fn run_replay_on_clones(
    plan: &ReorgPlan,
    index: &BlockIndex,
    material: &ReplaySandboxMaterial,
) -> Result<ReplaySandboxReplayOk, ReplaySandboxReplayError> {
    if !plan.rollback_ordered.is_empty() && material.expected_state_at_old_tip.is_none() {
        return Err(ReplaySandboxReplayError::MissingExpectedOldTipState);
    }

    let required_hashes: Vec<String> = plan
        .rollback_ordered
        .iter()
        .chain(plan.apply_ordered.iter())
        .cloned()
        .collect();

    for h in &required_hashes {
        if !material.blocks_by_hash.contains_key(h) {
            return Err(ReplaySandboxReplayError::MissingBlockBody {
                block_hash: h.clone(),
            });
        }
    }

    if !plan.rollback_ordered.is_empty() {
        let mut walk = material.state_at_fork.clone();
        for h in plan.rollback_ordered.iter().rev() {
            let block = material.blocks_by_hash.get(h).expect("checked contains");
            validate_block_against_index(block, index, h)?;
            apply_block_to_state(&mut walk, block, h)?;
        }
        let replayed_fp = sandbox_state_fingerprint(&walk);
        let expected = material
            .expected_state_at_old_tip
            .as_ref()
            .expect("checked Some when rollback non-empty");
        let expected_fp = sandbox_state_fingerprint(expected);
        if replayed_fp != expected_fp {
            return Err(ReplaySandboxReplayError::OldTipStateMismatch {
                expected_fingerprint: expected_fp,
                replayed_fingerprint: replayed_fp,
            });
        }
    }

    let mut sandbox = material.state_at_fork.clone();
    for h in &plan.apply_ordered {
        let block = material.blocks_by_hash.get(h).expect("checked contains");
        validate_block_against_index(block, index, h)?;
        apply_block_to_state(&mut sandbox, block, h)?;
    }

    let final_state_fingerprint = sandbox_state_fingerprint(&sandbox);
    let final_supply_total = sandbox
        .total_balance_sum()
        .map_err(|e| ReplaySandboxReplayError::SupplyAudit(e.to_string()))?;

    Ok(ReplaySandboxReplayOk {
        rollback_blocks_verified: plan.rollback_ordered.len(),
        apply_blocks_simulated: plan.apply_ordered.len(),
        final_state_fingerprint,
        final_supply_total,
    })
}

fn validate_block_against_index(
    block: &Block,
    index: &BlockIndex,
    expected_hash: &str,
) -> Result<(), ReplaySandboxReplayError> {
    if block.block_hash != expected_hash {
        return Err(ReplaySandboxReplayError::BlockHeaderMismatch {
            block_hash: expected_hash.to_string(),
            detail: "block.block_hash must equal map/plan hash",
        });
    }
    let entry = index.get(expected_hash).ok_or_else(|| {
        ReplaySandboxReplayError::BlockHeaderMismatch {
            block_hash: expected_hash.to_string(),
            detail: "block hash missing from index (structural validation should prevent this)",
        }
    })?;
    if block.height != entry.height {
        return Err(ReplaySandboxReplayError::BlockHeaderMismatch {
            block_hash: expected_hash.to_string(),
            detail: "block.height must equal index height",
        });
    }
    if block.previous_hash != entry.parent_hash {
        return Err(ReplaySandboxReplayError::BlockHeaderMismatch {
            block_hash: expected_hash.to_string(),
            detail: "block.previous_hash must equal index parent_hash",
        });
    }
    Ok(())
}

fn apply_block_to_state(
    state: &mut State,
    block: &Block,
    block_hash: &str,
) -> Result<(), ReplaySandboxReplayError> {
    block.basic_validate().map_err(|e| ReplaySandboxReplayError::InvalidBlockBody {
        block_hash: block_hash.to_string(),
        error: e.to_string(),
    })?;
    for tx in &block.transactions {
        state.apply_transaction(tx).map_err(|e| {
            ReplaySandboxReplayError::StateTransition {
                block_hash: block_hash.to_string(),
                error: e.to_string(),
            }
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use ed25519_dalek::{Signer, SigningKey};

    use super::super::block_index::{BlockIndex, BlockIndexEntry};
    use super::super::reorg_plan::ReorgPlan;
    use super::super::reorg_preflight::{ReorgPreflightPolicy, ReorgPreflightVerdict};
    use super::{
        sandbox_state_fingerprint, ReplaySandbox, ReplaySandboxMaterial, ReplaySandboxReplayError,
        ReplaySandboxReplayPhase,
    };
    use crate::block::Block;
    use crate::crypto::Crypto;
    use crate::genesis::Genesis;
    use crate::state::State;
    use crate::transaction::Transaction;
    use crate::types::Address;

    fn permissive_policy() -> ReorgPreflightPolicy {
        ReorgPreflightPolicy {
            max_reorg_depth: usize::MAX,
        }
    }

    fn genesis_index_entry() -> BlockIndexEntry {
        BlockIndexEntry {
            parent_hash: String::new(),
            height: 0,
        }
    }

    fn child_entry(parent: &str, height: u64) -> BlockIndexEntry {
        BlockIndexEntry {
            parent_hash: parent.into(),
            height,
        }
    }

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

    fn block_with_txs(
        height: u64,
        previous_hash: &str,
        timestamp_unix: u64,
        txs: Vec<Transaction>,
    ) -> Block {
        let mut block = Block {
            height,
            previous_hash: previous_hash.to_string(),
            timestamp_unix,
            transactions: txs,
            block_hash: String::new(),
        };
        block.block_hash = block.compute_block_hash();
        block
    }

    fn fund_state_at_genesis(signing_key: &SigningKey, balance: u64) -> State {
        let verifying_key = signing_key.verifying_key();
        let sender = Address::new(Crypto::address_from_public_key(&verifying_key.to_bytes()));
        let mut state = State::from_genesis(&Genesis::empty()).unwrap();
        state.create_account(sender, balance);
        state
    }

    fn linear_index_with_two_blocks(signing_key: &SigningKey) -> (BlockIndex, Block, Block) {
        let genesis = Block::genesis();
        let ghash = genesis.block_hash.clone();

        let tx0 = make_signed_tx(
            signing_key,
            Address::new("recv_b1"),
            1,
            0,
            0,
            1_700_000_200,
        );
        let b1 = block_with_txs(1, &ghash, 1_700_000_201, vec![tx0]);

        let tx1 = make_signed_tx(
            signing_key,
            Address::new("recv_b2"),
            2,
            0,
            1,
            1_700_000_202,
        );
        let b2 = block_with_txs(2, &b1.block_hash, 1_700_000_203, vec![tx1]);

        let mut index = BlockIndex::new();
        index.insert(ghash.clone(), genesis_index_entry());
        index.insert(
            b1.block_hash.clone(),
            child_entry(&ghash, 1),
        );
        index.insert(
            b2.block_hash.clone(),
            child_entry(&b1.block_hash, 2),
        );

        (index, b1, b2)
    }

    fn apply_block_to_state_for_test(state: &mut State, block: &Block) {
        block.basic_validate().unwrap();
        for tx in &block.transactions {
            state.apply_transaction(tx).unwrap();
        }
    }

    fn empty_block(height: u64, previous_hash: &str, timestamp_unix: u64) -> Block {
        block_with_txs(height, previous_hash, timestamp_unix, vec![])
    }

    #[test]
    fn replay_noop_report_ok_and_deterministic() {
        let signing_key = SigningKey::from_bytes(&[21u8; 32]);
        let (index, b1, b2) = linear_index_with_two_blocks(&signing_key);
        let b2_hash = b2.block_hash.clone();

        let plan = ReorgPlan::try_from_tips(&index, &b2_hash, &b2_hash).unwrap();

        let mut at_tip = fund_state_at_genesis(&signing_key, 100);
        apply_block_to_state_for_test(&mut at_tip, &b1);
        apply_block_to_state_for_test(&mut at_tip, &b2);

        let material = ReplaySandboxMaterial {
            state_at_fork: at_tip.clone(),
            blocks_by_hash: HashMap::new(),
            expected_state_at_old_tip: None,
        };

        let a = ReplaySandbox::run_report(&plan, &index, &permissive_policy(), &material);
        let b = ReplaySandbox::run_report(&plan, &index, &permissive_policy(), &material);
        assert_eq!(a, b);
        assert_eq!(a.structural, Ok(()));
        assert_eq!(a.preflight.verdict, ReorgPreflightVerdict::NoOp);
        match &a.replay {
            ReplaySandboxReplayPhase::Completed(Ok(ok)) => {
                assert_eq!(ok.rollback_blocks_verified, 0);
                assert_eq!(ok.apply_blocks_simulated, 0);
                assert_eq!(ok.final_state_fingerprint, sandbox_state_fingerprint(&at_tip));
            }
            other => panic!("unexpected replay: {other:?}"),
        }
    }

    #[test]
    fn replay_rollback_only_sandbox() {
        let signing_key = SigningKey::from_bytes(&[22u8; 32]);
        let (index, b1, b2) = linear_index_with_two_blocks(&signing_key);
        let b2_hash = b2.block_hash.clone();

        let plan = ReorgPlan::try_from_tips(&index, &b2_hash, &b1.block_hash).unwrap();
        assert_eq!(plan.fork_hash, b1.block_hash);
        assert_eq!(plan.rollback_ordered, vec![b2_hash.clone()]);
        assert!(plan.apply_ordered.is_empty());

        let mut at_fork = fund_state_at_genesis(&signing_key, 100);
        apply_block_to_state_for_test(&mut at_fork, &b1);

        let mut at_tip = at_fork.clone();
        apply_block_to_state_for_test(&mut at_tip, &b2);

        let mut blocks = HashMap::new();
        blocks.insert(b1.block_hash.clone(), b1.clone());
        blocks.insert(b2.block_hash.clone(), b2.clone());

        let material = ReplaySandboxMaterial {
            state_at_fork: at_fork.clone(),
            blocks_by_hash: blocks,
            expected_state_at_old_tip: Some(at_tip.clone()),
        };

        let r = ReplaySandbox::run_report(&plan, &index, &permissive_policy(), &material);
        assert_eq!(r.preflight.verdict, ReorgPreflightVerdict::Acceptable);
        match &r.replay {
            ReplaySandboxReplayPhase::Completed(Ok(ok)) => {
                assert_eq!(ok.rollback_blocks_verified, 1);
                assert_eq!(ok.apply_blocks_simulated, 0);
                assert_eq!(ok.final_state_fingerprint, sandbox_state_fingerprint(&at_fork));
            }
            other => panic!("unexpected replay: {other:?}"),
        }
    }

    #[test]
    fn replay_apply_only_sandbox() {
        let signing_key = SigningKey::from_bytes(&[23u8; 32]);
        let (index, b1, b2) = linear_index_with_two_blocks(&signing_key);
        let b2_hash = b2.block_hash.clone();

        let plan = ReorgPlan::try_from_tips(&index, &b1.block_hash, &b2_hash).unwrap();
        assert!(plan.rollback_ordered.is_empty());
        assert_eq!(plan.apply_ordered, vec![b2.block_hash.clone()]);

        let mut at_fork = fund_state_at_genesis(&signing_key, 100);
        apply_block_to_state_for_test(&mut at_fork, &b1);

        let mut blocks = HashMap::new();
        blocks.insert(b1.block_hash.clone(), b1.clone());
        blocks.insert(b2.block_hash.clone(), b2.clone());

        let material = ReplaySandboxMaterial {
            state_at_fork: at_fork.clone(),
            blocks_by_hash: blocks,
            expected_state_at_old_tip: None,
        };

        let mut expected_final = at_fork.clone();
        apply_block_to_state_for_test(&mut expected_final, &b2);

        let r = ReplaySandbox::run_report(&plan, &index, &permissive_policy(), &material);
        match &r.replay {
            ReplaySandboxReplayPhase::Completed(Ok(ok)) => {
                assert_eq!(ok.rollback_blocks_verified, 0);
                assert_eq!(ok.apply_blocks_simulated, 1);
                assert_eq!(
                    ok.final_state_fingerprint,
                    sandbox_state_fingerprint(&expected_final)
                );
            }
            other => panic!("unexpected replay: {other:?}"),
        }
    }

    #[test]
    fn replay_forked_rollback_and_apply_sandbox() {
        let signing_key = SigningKey::from_bytes(&[24u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let sender = Address::new(Crypto::address_from_public_key(&verifying_key.to_bytes()));

        let genesis = Block::genesis();
        let ghash = genesis.block_hash.clone();

        let b1 = empty_block(1, &ghash, 1_700_000_500);
        let b2 = empty_block(2, &b1.block_hash, 1_700_000_501);

        let tx_o3 = make_signed_tx(
            &signing_key,
            Address::new("recv_o3"),
            3,
            0,
            0,
            1_700_000_300,
        );
        let blk_o3 = block_with_txs(3, &b2.block_hash, 1_700_000_301, vec![tx_o3]);
        let tx_o4 = make_signed_tx(
            &signing_key,
            Address::new("recv_o4"),
            1,
            0,
            1,
            1_700_000_302,
        );
        let blk_o4 = block_with_txs(4, &blk_o3.block_hash, 1_700_000_303, vec![tx_o4]);

        let tx_n3 = make_signed_tx(
            &signing_key,
            Address::new("recv_n3"),
            5,
            0,
            0,
            1_700_000_310,
        );
        let blk_n3 = block_with_txs(3, &b2.block_hash, 1_700_000_311, vec![tx_n3]);
        let tx_n4 = make_signed_tx(
            &signing_key,
            Address::new("recv_n4"),
            1,
            0,
            1,
            1_700_000_312,
        );
        let blk_n4 = block_with_txs(4, &blk_n3.block_hash, 1_700_000_313, vec![tx_n4]);
        let tx_n5 = make_signed_tx(
            &signing_key,
            Address::new("recv_n5"),
            1,
            0,
            2,
            1_700_000_314,
        );
        let blk_n5 = block_with_txs(5, &blk_n4.block_hash, 1_700_000_315, vec![tx_n5]);

        let mut index = BlockIndex::new();
        index.insert(ghash.clone(), genesis_index_entry());
        index.insert(b1.block_hash.clone(), child_entry(&ghash, 1));
        index.insert(b2.block_hash.clone(), child_entry(&b1.block_hash, 2));
        index.insert(
            blk_o3.block_hash.clone(),
            child_entry(&b2.block_hash, 3),
        );
        index.insert(
            blk_o4.block_hash.clone(),
            child_entry(&blk_o3.block_hash, 4),
        );
        index.insert(
            blk_n3.block_hash.clone(),
            child_entry(&b2.block_hash, 3),
        );
        index.insert(
            blk_n4.block_hash.clone(),
            child_entry(&blk_n3.block_hash, 4),
        );
        index.insert(
            blk_n5.block_hash.clone(),
            child_entry(&blk_n4.block_hash, 5),
        );

        let plan = ReorgPlan::try_from_tips(&index, &blk_o4.block_hash, &blk_n5.block_hash).unwrap();
        assert_eq!(plan.fork_hash, b2.block_hash);
        assert_eq!(plan.rollback_ordered, vec![
            blk_o4.block_hash.clone(),
            blk_o3.block_hash.clone()
        ]);
        assert_eq!(plan.apply_ordered, vec![
            blk_n3.block_hash.clone(),
            blk_n4.block_hash.clone(),
            blk_n5.block_hash.clone()
        ]);

        let mut base = State::from_genesis(&Genesis::empty()).unwrap();
        base.create_account(sender.clone(), 1_000);

        let mut at_fork = base.clone();
        apply_block_to_state_for_test(&mut at_fork, &b1);
        apply_block_to_state_for_test(&mut at_fork, &b2);

        let mut at_old_tip = at_fork.clone();
        apply_block_to_state_for_test(&mut at_old_tip, &blk_o3);
        apply_block_to_state_for_test(&mut at_old_tip, &blk_o4);

        let mut expected_new_tip = at_fork.clone();
        apply_block_to_state_for_test(&mut expected_new_tip, &blk_n3);
        apply_block_to_state_for_test(&mut expected_new_tip, &blk_n4);
        apply_block_to_state_for_test(&mut expected_new_tip, &blk_n5);

        let mut blocks = HashMap::new();
        blocks.insert(blk_o3.block_hash.clone(), blk_o3.clone());
        blocks.insert(blk_o4.block_hash.clone(), blk_o4.clone());
        blocks.insert(blk_n3.block_hash.clone(), blk_n3.clone());
        blocks.insert(blk_n4.block_hash.clone(), blk_n4.clone());
        blocks.insert(blk_n5.block_hash.clone(), blk_n5.clone());

        let material = ReplaySandboxMaterial {
            state_at_fork: at_fork.clone(),
            blocks_by_hash: blocks,
            expected_state_at_old_tip: Some(at_old_tip.clone()),
        };

        let r = ReplaySandbox::run_report(&plan, &index, &permissive_policy(), &material);
        let r2 = ReplaySandbox::run_report(&plan, &index, &permissive_policy(), &material);
        assert_eq!(r, r2);
        match &r.replay {
            ReplaySandboxReplayPhase::Completed(Ok(ok)) => {
                assert_eq!(ok.rollback_blocks_verified, 2);
                assert_eq!(ok.apply_blocks_simulated, 3);
                assert_eq!(
                    ok.final_state_fingerprint,
                    sandbox_state_fingerprint(&expected_new_tip)
                );
            }
            other => panic!("unexpected replay: {other:?}"),
        }
    }

    #[test]
    fn replay_rejects_invalid_block_body_during_apply() {
        let signing_key = SigningKey::from_bytes(&[25u8; 32]);
        let genesis = Block::genesis();
        let ghash = genesis.block_hash.clone();

        let tx0 = make_signed_tx(
            &signing_key,
            Address::new("recv_b1_bad_apply"),
            1,
            0,
            0,
            1_700_000_800,
        );
        let b1 = block_with_txs(1, &ghash, 1_700_000_801, vec![tx0]);

        let bad_tx = make_signed_tx(
            &signing_key,
            Address::new("recv_bad"),
            0,
            0,
            1,
            1_700_000_900,
        );
        let b2_bad = block_with_txs(2, &b1.block_hash, 1_700_000_901, vec![bad_tx]);

        let mut index = BlockIndex::new();
        index.insert(ghash.clone(), genesis_index_entry());
        index.insert(b1.block_hash.clone(), child_entry(&ghash, 1));
        index.insert(
            b2_bad.block_hash.clone(),
            child_entry(&b1.block_hash, 2),
        );

        let plan = ReorgPlan::try_from_tips(&index, &b1.block_hash, &b2_bad.block_hash).unwrap();

        let mut at_fork = fund_state_at_genesis(&signing_key, 100);
        apply_block_to_state_for_test(&mut at_fork, &b1);

        let mut blocks = HashMap::new();
        blocks.insert(b1.block_hash.clone(), b1.clone());
        blocks.insert(b2_bad.block_hash.clone(), b2_bad.clone());

        let material = ReplaySandboxMaterial {
            state_at_fork: at_fork,
            blocks_by_hash: blocks,
            expected_state_at_old_tip: None,
        };

        let r = ReplaySandbox::run_report(&plan, &index, &permissive_policy(), &material);
        match &r.replay {
            ReplaySandboxReplayPhase::Completed(Err(e)) => {
                assert!(matches!(e, ReplaySandboxReplayError::InvalidBlockBody { .. }));
            }
            other => panic!("unexpected replay: {other:?}"),
        }
    }

    #[test]
    fn replay_rejects_block_header_mismatch_vs_index() {
        let signing_key = SigningKey::from_bytes(&[26u8; 32]);
        let (index, b1, b2) = linear_index_with_two_blocks(&signing_key);
        let b2_hash = b2.block_hash.clone();

        let mut b2_wrong_height = b2.clone();
        b2_wrong_height.height = 9;
        b2_wrong_height.block_hash = b2_wrong_height.compute_block_hash();

        let plan = ReorgPlan::try_from_tips(&index, &b1.block_hash, &b2_hash).unwrap();

        let mut at_fork = fund_state_at_genesis(&signing_key, 100);
        apply_block_to_state_for_test(&mut at_fork, &b1);

        let mut blocks = HashMap::new();
        blocks.insert(b1.block_hash.clone(), b1);
        blocks.insert(b2_hash.clone(), b2_wrong_height);

        let material = ReplaySandboxMaterial {
            state_at_fork: at_fork,
            blocks_by_hash: blocks,
            expected_state_at_old_tip: None,
        };

        let r = ReplaySandbox::run_report(&plan, &index, &permissive_policy(), &material);
        match &r.replay {
            ReplaySandboxReplayPhase::Completed(Err(e)) => {
                assert!(matches!(e, ReplaySandboxReplayError::BlockHeaderMismatch { .. }));
            }
            other => panic!("unexpected replay: {other:?}"),
        }
    }

    #[test]
    fn replay_skipped_when_preflight_policy_rejects() {
        let signing_key = SigningKey::from_bytes(&[27u8; 32]);
        let (index, b1, b2) = linear_index_with_two_blocks(&signing_key);
        let b2_hash = b2.block_hash.clone();

        let plan = ReorgPlan::try_from_tips(&index, &b2_hash, &b1.block_hash).unwrap();
        assert_eq!(plan.rollback_ordered.len(), 1);

        let mut at_fork = fund_state_at_genesis(&signing_key, 100);
        apply_block_to_state_for_test(&mut at_fork, &b1);
        let mut at_tip = at_fork.clone();
        apply_block_to_state_for_test(&mut at_tip, &b2);

        let mut blocks = HashMap::new();
        blocks.insert(b1.block_hash.clone(), b1.clone());
        blocks.insert(b2.block_hash.clone(), b2.clone());

        let material = ReplaySandboxMaterial {
            state_at_fork: at_fork,
            blocks_by_hash: blocks,
            expected_state_at_old_tip: Some(at_tip),
        };

        let policy = ReorgPreflightPolicy {
            max_reorg_depth: 0,
        };
        let r = ReplaySandbox::run_report(&plan, &index, &policy, &material);
        assert_eq!(r.preflight.verdict, ReorgPreflightVerdict::Rejected);
        assert_eq!(r.replay, ReplaySandboxReplayPhase::SkippedDueToPolicy);
    }

    #[test]
    fn replay_rejects_old_tip_state_mismatch() {
        let signing_key = SigningKey::from_bytes(&[28u8; 32]);
        let (index, b1, b2) = linear_index_with_two_blocks(&signing_key);
        let b2_hash = b2.block_hash.clone();

        let plan = ReorgPlan::try_from_tips(&index, &b2_hash, &b1.block_hash).unwrap();

        let mut at_fork = fund_state_at_genesis(&signing_key, 100);
        apply_block_to_state_for_test(&mut at_fork, &b1);
        let mut wrong_expected = at_fork.clone();
        apply_block_to_state_for_test(&mut wrong_expected, &b2);
        wrong_expected
            .create_account(Address::new("extra_account_that_changes_fingerprint"), 1);

        let mut blocks = HashMap::new();
        blocks.insert(b1.block_hash.clone(), b1.clone());
        blocks.insert(b2.block_hash.clone(), b2.clone());

        let material = ReplaySandboxMaterial {
            state_at_fork: at_fork,
            blocks_by_hash: blocks,
            expected_state_at_old_tip: Some(wrong_expected),
        };

        let r = ReplaySandbox::run_report(&plan, &index, &permissive_policy(), &material);
        match &r.replay {
            ReplaySandboxReplayPhase::Completed(Err(
                ReplaySandboxReplayError::OldTipStateMismatch { .. },
            )) => {}
            other => panic!("unexpected replay: {other:?}"),
        }
    }

    #[test]
    fn replay_rejects_missing_expected_old_tip_when_rollback_non_empty() {
        let signing_key = SigningKey::from_bytes(&[30u8; 32]);
        let (index, b1, b2) = linear_index_with_two_blocks(&signing_key);
        let b2_hash = b2.block_hash.clone();
        let plan = ReorgPlan::try_from_tips(&index, &b2_hash, &b1.block_hash).unwrap();

        let mut at_fork = fund_state_at_genesis(&signing_key, 100);
        apply_block_to_state_for_test(&mut at_fork, &b1);

        let mut blocks = HashMap::new();
        blocks.insert(b1.block_hash.clone(), b1.clone());
        blocks.insert(b2.block_hash.clone(), b2.clone());

        let material = ReplaySandboxMaterial {
            state_at_fork: at_fork,
            blocks_by_hash: blocks,
            expected_state_at_old_tip: None,
        };

        let r = ReplaySandbox::run_report(&plan, &index, &permissive_policy(), &material);
        match &r.replay {
            ReplaySandboxReplayPhase::Completed(Err(
                ReplaySandboxReplayError::MissingExpectedOldTipState,
            )) => {}
            other => panic!("unexpected replay: {other:?}"),
        }
    }

    #[test]
    fn replay_skipped_when_structurally_invalid() {
        let signing_key = SigningKey::from_bytes(&[31u8; 32]);
        let (index, b1, b2) = linear_index_with_two_blocks(&signing_key);
        let plan = ReorgPlan {
            fork_hash: "not-in-index".into(),
            rollback_ordered: vec![],
            apply_ordered: vec![b2.block_hash.clone()],
        };

        let mut at_fork = fund_state_at_genesis(&signing_key, 100);
        apply_block_to_state_for_test(&mut at_fork, &b1);

        let mut blocks = HashMap::new();
        blocks.insert(b1.block_hash.clone(), b1.clone());
        blocks.insert(b2.block_hash.clone(), b2.clone());

        let material = ReplaySandboxMaterial {
            state_at_fork: at_fork,
            blocks_by_hash: blocks,
            expected_state_at_old_tip: None,
        };

        let r = ReplaySandbox::run_report(&plan, &index, &permissive_policy(), &material);
        assert!(r.structural.is_err());
        assert_eq!(r.replay, ReplaySandboxReplayPhase::SkippedDueToStructuralInvalid);
    }

    #[test]
    fn replay_does_not_mutate_caller_material_state() {
        let signing_key = SigningKey::from_bytes(&[29u8; 32]);
        let (index, b1, b2) = linear_index_with_two_blocks(&signing_key);
        let b2_hash = b2.block_hash.clone();
        let plan = ReorgPlan::try_from_tips(&index, &b2_hash, &b2_hash).unwrap();

        let mut at_tip = fund_state_at_genesis(&signing_key, 100);
        apply_block_to_state_for_test(&mut at_tip, &b1);
        apply_block_to_state_for_test(&mut at_tip, &b2);

        let material = ReplaySandboxMaterial {
            state_at_fork: at_tip.clone(),
            blocks_by_hash: HashMap::new(),
            expected_state_at_old_tip: None,
        };
        let fp_before = sandbox_state_fingerprint(&material.state_at_fork);
        let _ = ReplaySandbox::run_report(&plan, &index, &permissive_policy(), &material);
        assert_eq!(sandbox_state_fingerprint(&material.state_at_fork), fp_before);
    }
}
