//! **Inert** read-only **reorg preflight** — preparatory V3 scaffolding only.
//!
//! Combines structural [`ReorgPlan`](super::reorg_plan::ReorgPlan) validation against a
//! [`BlockIndex`](super::block_index::BlockIndex) with **local operator policy** (for example
//! `MAX_REORG_DEPTH`-style bounds). Produces a [`ReorgPreflightReport`] only; **no** execution,
//! **no** state or storage mutation, **no** network or CLI wiring.
//!
//! Structural checks and policy checks are **explicitly separated** in the report so a future
//! executor can distinguish “invalid plan / index” from “valid but locally refused”.

use super::block_index::BlockIndex;
use super::reorg_plan::{ReorgPlan, ReorgPlanValidationError};

/// Local fail-closed rules applied **after** structural validation succeeds.
///
/// Aligns with `docs/reorg_model.md` §4.1: depth is measured on the **old canonical suffix**
/// (`rollback_ordered.len()`), not apply length.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReorgPreflightPolicy {
    /// Maximum allowed `rollback_ordered.len()` for a non–no-op plan. `usize::MAX` effectively
    /// disables the check.
    pub max_reorg_depth: usize,
}

impl Default for ReorgPreflightPolicy {
    fn default() -> Self {
        Self {
            max_reorg_depth: usize::MAX,
        }
    }
}

/// Policy violation (structural validation already passed).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReorgPreflightPolicyError {
    /// `rollback_ordered.len()` exceeds [`ReorgPreflightPolicy::max_reorg_depth`].
    MaxReorgDepthExceeded {
        rollback_depth: usize,
        max_allowed: usize,
    },
}

/// Outcome bucket for operators and future wiring.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReorgPreflightVerdict {
    /// Structurally valid plan with nothing to roll back or apply.
    NoOp,
    /// Structurally valid and policy allows this non–no-op reorg.
    Acceptable,
    /// Structurally invalid **or** structurally valid but policy rejected **or** both (see report fields).
    Rejected,
}

/// Whether local policy was evaluated and its result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReorgPreflightPolicyPhase {
    /// [`ReorgPlan::validate_against_index`] failed; policy is not applied.
    NotEvaluated,
    /// Policy ran and accepted the plan (including trivial acceptance when `rollback_depth == 0`).
    Ok,
    /// Policy ran and rejected the plan.
    Failed(ReorgPreflightPolicyError),
}

/// Read-only preflight result: depths, structural vs policy phases, and a coarse verdict.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReorgPreflightReport {
    pub rollback_depth: usize,
    pub apply_depth: usize,
    /// Full structural validation against `index` (fork, segments, ancestry).
    pub structural: Result<(), ReorgPlanValidationError>,
    /// Local policy relative to structural success.
    pub policy: ReorgPreflightPolicyPhase,
    pub verdict: ReorgPreflightVerdict,
}

/// Namespace for the preflight entrypoint (no instance state).
pub struct ReorgPreflight;

impl ReorgPreflight {
    /// Validate `plan` against `index`, then apply `policy` when structural validation succeeds.
    pub fn evaluate(
        plan: &ReorgPlan,
        index: &BlockIndex,
        policy: &ReorgPreflightPolicy,
    ) -> ReorgPreflightReport {
        let rollback_depth = plan.rollback_ordered.len();
        let apply_depth = plan.apply_ordered.len();
        let structural = plan.validate_against_index(index);

        let (policy_phase, verdict) = match &structural {
            Err(_) => (
                ReorgPreflightPolicyPhase::NotEvaluated,
                ReorgPreflightVerdict::Rejected,
            ),
            Ok(()) if plan.is_noop() => {
                (ReorgPreflightPolicyPhase::Ok, ReorgPreflightVerdict::NoOp)
            }
            Ok(()) => match check_policy(rollback_depth, policy) {
                Ok(()) => (
                    ReorgPreflightPolicyPhase::Ok,
                    ReorgPreflightVerdict::Acceptable,
                ),
                Err(e) => (
                    ReorgPreflightPolicyPhase::Failed(e),
                    ReorgPreflightVerdict::Rejected,
                ),
            },
        };

        ReorgPreflightReport {
            rollback_depth,
            apply_depth,
            structural,
            policy: policy_phase,
            verdict,
        }
    }
}

fn check_policy(
    rollback_depth: usize,
    policy: &ReorgPreflightPolicy,
) -> Result<(), ReorgPreflightPolicyError> {
    if rollback_depth > policy.max_reorg_depth {
        return Err(ReorgPreflightPolicyError::MaxReorgDepthExceeded {
            rollback_depth,
            max_allowed: policy.max_reorg_depth,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::block_index::{BlockIndex, BlockIndexEntry};
    use super::super::reorg_plan::ReorgPlan;
    use super::{
        ReorgPreflight, ReorgPreflightPolicy, ReorgPreflightPolicyError, ReorgPreflightPolicyPhase,
        ReorgPreflightReport, ReorgPreflightVerdict,
    };

    fn genesis() -> BlockIndexEntry {
        BlockIndexEntry {
            parent_hash: String::new(),
            height: 0,
        }
    }

    fn child(parent: &str, h: u64) -> BlockIndexEntry {
        BlockIndexEntry {
            parent_hash: parent.into(),
            height: h,
        }
    }

    fn linear_chain() -> BlockIndex {
        let mut idx = BlockIndex::new();
        idx.insert("g".into(), genesis());
        idx.insert("a1".into(), child("g", 1));
        idx.insert("a2".into(), child("a1", 2));
        idx
    }

    fn deeper_fork_index() -> BlockIndex {
        let mut idx = BlockIndex::new();
        idx.insert("g".into(), genesis());
        idx.insert("b1".into(), child("g", 1));
        idx.insert("b2".into(), child("b1", 2));
        idx.insert("o3".into(), child("b2", 3));
        idx.insert("o4".into(), child("o3", 4));
        idx.insert("n3".into(), child("b2", 3));
        idx.insert("n4".into(), child("n3", 4));
        idx.insert("n5".into(), child("n4", 5));
        idx
    }

    fn permissive_policy() -> ReorgPreflightPolicy {
        ReorgPreflightPolicy {
            max_reorg_depth: usize::MAX,
        }
    }

    #[test]
    fn preflight_noop_ok() {
        let idx = linear_chain();
        let plan = ReorgPlan::try_from_tips(&idx, "a2", "a2").unwrap();
        let r = ReorgPreflight::evaluate(&plan, &idx, &permissive_policy());
        assert_eq!(r.rollback_depth, 0);
        assert_eq!(r.apply_depth, 0);
        assert_eq!(r.structural, Ok(()));
        assert_eq!(r.policy, ReorgPreflightPolicyPhase::Ok);
        assert_eq!(r.verdict, ReorgPreflightVerdict::NoOp);
    }

    #[test]
    fn preflight_shallow_reorg_acceptable() {
        let idx = deeper_fork_index();
        let plan = ReorgPlan::try_from_tips(&idx, "o4", "n5").unwrap();
        let r = ReorgPreflight::evaluate(&plan, &idx, &ReorgPreflightPolicy { max_reorg_depth: 2 });
        assert_eq!(r.rollback_depth, 2);
        assert_eq!(r.apply_depth, 3);
        assert_eq!(r.structural, Ok(()));
        assert_eq!(r.policy, ReorgPreflightPolicyPhase::Ok);
        assert_eq!(r.verdict, ReorgPreflightVerdict::Acceptable);
    }

    #[test]
    fn preflight_rollback_only_acceptable() {
        let idx = linear_chain();
        let plan = ReorgPlan::try_from_tips(&idx, "a2", "a1").unwrap();
        let r = ReorgPreflight::evaluate(&plan, &idx, &ReorgPreflightPolicy { max_reorg_depth: 1 });
        assert_eq!(r.rollback_depth, 1);
        assert_eq!(r.apply_depth, 0);
        assert_eq!(r.structural, Ok(()));
        assert_eq!(r.policy, ReorgPreflightPolicyPhase::Ok);
        assert_eq!(r.verdict, ReorgPreflightVerdict::Acceptable);
    }

    #[test]
    fn preflight_apply_only_acceptable() {
        let idx = linear_chain();
        let plan = ReorgPlan::try_from_tips(&idx, "a1", "a2").unwrap();
        let r = ReorgPreflight::evaluate(&plan, &idx, &ReorgPreflightPolicy { max_reorg_depth: 0 });
        assert_eq!(r.rollback_depth, 0);
        assert_eq!(r.apply_depth, 1);
        assert_eq!(r.structural, Ok(()));
        assert_eq!(r.policy, ReorgPreflightPolicyPhase::Ok);
        assert_eq!(r.verdict, ReorgPreflightVerdict::Acceptable);
    }

    #[test]
    fn preflight_rejects_when_max_reorg_depth_exceeded() {
        let idx = deeper_fork_index();
        let plan = ReorgPlan::try_from_tips(&idx, "o4", "n5").unwrap();
        let r = ReorgPreflight::evaluate(&plan, &idx, &ReorgPreflightPolicy { max_reorg_depth: 1 });
        assert_eq!(r.structural, Ok(()));
        assert_eq!(
            r.policy,
            ReorgPreflightPolicyPhase::Failed(ReorgPreflightPolicyError::MaxReorgDepthExceeded {
                rollback_depth: 2,
                max_allowed: 1,
            })
        );
        assert_eq!(r.verdict, ReorgPreflightVerdict::Rejected);
    }

    #[test]
    fn preflight_rejects_structurally_invalid_plan() {
        let idx = linear_chain();
        let plan = ReorgPlan {
            fork_hash: "g".into(),
            rollback_ordered: vec!["a2".into()],
            apply_ordered: vec![],
        };
        let r = ReorgPreflight::evaluate(&plan, &idx, &permissive_policy());
        assert!(r.structural.is_err());
        assert_eq!(r.policy, ReorgPreflightPolicyPhase::NotEvaluated);
        assert_eq!(r.verdict, ReorgPreflightVerdict::Rejected);
    }

    #[test]
    fn preflight_report_is_deterministic() {
        let idx = deeper_fork_index();
        let plan = ReorgPlan::try_from_tips(&idx, "o4", "n5").unwrap();
        let policy = ReorgPreflightPolicy { max_reorg_depth: 1 };
        let a = ReorgPreflight::evaluate(&plan, &idx, &policy);
        let b = ReorgPreflight::evaluate(&plan, &idx, &policy);
        assert_eq!(a, b);
        assert_eq!(
            a,
            ReorgPreflightReport {
                rollback_depth: 2,
                apply_depth: 3,
                structural: Ok(()),
                policy: ReorgPreflightPolicyPhase::Failed(
                    ReorgPreflightPolicyError::MaxReorgDepthExceeded {
                        rollback_depth: 2,
                        max_allowed: 1,
                    }
                ),
                verdict: ReorgPreflightVerdict::Rejected,
            }
        );
    }
}
