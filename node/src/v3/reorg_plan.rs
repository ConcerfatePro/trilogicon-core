//! **Inert** reorg **plan** derived from [`ForkSlices`](super::block_index::ForkSlices).
//!
//! This is **preparatory V3 scaffolding only**: it makes **rollback** and **apply** order explicit
//! for a future executor. It does **not** touch `State`, `Blockchain`, storage, network, or CLI.
//!
//! **Validation:** [`ReorgPlan::validate_against_index`] is a pure structural check against a
//! [`BlockIndex`](super::block_index::BlockIndex). Any future **replay / execution** path must
//! accept only plans that pass validation and were built from **hardened** index data (same
//! ancestry rules as [`BlockIndex::path_from_tip_toward_genesis`](super::block_index::BlockIndex::path_from_tip_toward_genesis)),
//! not ad-hoc hashes.
//!
//! ## Semantics
//!
//! * [`ForkSlices::old_suffix`](super::block_index::ForkSlices) is **tip-first** (current committed
//!   tip first, down toward the fork). **`rollback_ordered`** is the **same order**: unwind state
//!   **one block at a time** starting at the committed tip.
//! * [`ForkSlices::new_suffix`](super::block_index::ForkSlices) is **tip-first** away from the fork.
//!   **`apply_ordered`** is the **reverse**: **parent before child** so each next block extends the
//!   post-fork chain ending at the **new** tip.

use std::collections::HashSet;

use super::block_index::{BlockIndex, BlockPathError, ForkSlices};

/// Structural problems when checking a [`ReorgPlan`] against a [`BlockIndex`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReorgPlanValidationError {
    /// Underlying index / ancestry rule violation (unknown hash, malformed link, etc.).
    BlockPath(BlockPathError),
    /// `rollback_ordered` parent chain does not match the fork or the next rollback step.
    BrokenRollbackChain { block: String, detail: &'static str },
    /// `apply_ordered` parent chain does not start at the fork or extend contiguously.
    BrokenApplyChain { block: String, detail: &'static str },
    /// Same hash appears twice in `rollback_ordered` or `apply_ordered`.
    DuplicateBlock { block: String, side: &'static str },
    /// `fork_hash` must not appear inside rollback or apply segments.
    ForkInsideSegments { block: String },
}

/// Explicit rollback (tip → fork) and apply (fork-child → new tip) order for a planned reorg.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReorgPlan {
    /// Deepest common block hash (`ForkSlices::fork_hash`).
    pub fork_hash: String,
    /// Blocks to **roll back** from the old committed tip toward `fork_hash`, **excluding** the fork.
    /// Order: **old tip first**, then its parent on the old branch, … until the block whose parent is `fork_hash`.
    pub rollback_ordered: Vec<String>,
    /// Blocks to **apply** on top of `fork_hash` state after rollback, **excluding** the fork.
    /// Order: **chain order** — first block whose parent is `fork_hash`, then its child, … ending at the **new** tip.
    pub apply_ordered: Vec<String>,
}

impl ReorgPlan {
    /// Build a plan from fork slices (pure; does not read the index again).
    pub fn from_fork_slices(slices: ForkSlices) -> Self {
        let ForkSlices {
            fork_hash,
            old_suffix,
            new_suffix,
        } = slices;
        let apply_ordered: Vec<String> = new_suffix.into_iter().rev().collect();
        Self {
            fork_hash,
            rollback_ordered: old_suffix,
            apply_ordered,
        }
    }

    /// [`fork_slices_between_tips`](BlockIndex::fork_slices_between_tips) then [`from_fork_slices`](Self::from_fork_slices).
    pub fn try_from_tips(
        index: &BlockIndex,
        old_tip: &str,
        new_tip: &str,
    ) -> Result<Self, BlockPathError> {
        let slices = index.fork_slices_between_tips(old_tip, new_tip)?;
        Ok(Self::from_fork_slices(slices))
    }

    /// `true` when there is nothing to roll back or apply (same tip, or empty deltas).
    pub fn is_noop(&self) -> bool {
        self.rollback_ordered.is_empty() && self.apply_ordered.is_empty()
    }

    /// Fail-closed structural validation: fork exists with valid ancestry, every listed hash exists
    /// with valid ancestry, rollback is **tip-first** contiguous back to `fork_hash`, apply is
    /// **fork-forward** contiguous through the new tip.
    pub fn validate_against_index(
        &self,
        index: &BlockIndex,
    ) -> Result<(), ReorgPlanValidationError> {
        index
            .validate_block_ancestry(&self.fork_hash)
            .map_err(ReorgPlanValidationError::BlockPath)?;

        ensure_unique_hashes(&self.rollback_ordered, "rollback")?;
        ensure_unique_hashes(&self.apply_ordered, "apply")?;

        for h in &self.rollback_ordered {
            if h == &self.fork_hash {
                return Err(ReorgPlanValidationError::ForkInsideSegments { block: h.clone() });
            }
        }
        for h in &self.apply_ordered {
            if h == &self.fork_hash {
                return Err(ReorgPlanValidationError::ForkInsideSegments { block: h.clone() });
            }
        }

        for (i, h) in self.rollback_ordered.iter().enumerate() {
            index
                .validate_block_ancestry(h)
                .map_err(ReorgPlanValidationError::BlockPath)?;
            let e = index.get(h).ok_or_else(|| {
                ReorgPlanValidationError::BlockPath(BlockPathError::UnknownBlock(h.clone()))
            })?;
            let want_parent = if i + 1 < self.rollback_ordered.len() {
                self.rollback_ordered[i + 1].as_str()
            } else {
                self.fork_hash.as_str()
            };
            if e.parent_hash != want_parent {
                return Err(ReorgPlanValidationError::BrokenRollbackChain {
                    block: h.clone(),
                    detail: "parent must equal next rollback entry or fork_hash",
                });
            }
        }

        for (i, h) in self.apply_ordered.iter().enumerate() {
            index
                .validate_block_ancestry(h)
                .map_err(ReorgPlanValidationError::BlockPath)?;
            let e = index.get(h).ok_or_else(|| {
                ReorgPlanValidationError::BlockPath(BlockPathError::UnknownBlock(h.clone()))
            })?;
            let want_parent = if i == 0 {
                self.fork_hash.as_str()
            } else {
                self.apply_ordered[i - 1].as_str()
            };
            if e.parent_hash != want_parent {
                return Err(ReorgPlanValidationError::BrokenApplyChain {
                    block: h.clone(),
                    detail: "parent must equal fork_hash or previous apply entry",
                });
            }
        }

        Ok(())
    }
}

fn ensure_unique_hashes(
    hashes: &[String],
    side: &'static str,
) -> Result<(), ReorgPlanValidationError> {
    let mut seen = HashSet::new();
    for h in hashes {
        if !seen.insert(h.clone()) {
            return Err(ReorgPlanValidationError::DuplicateBlock {
                block: h.clone(),
                side,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::block_index::{BlockIndex, BlockIndexEntry, BlockPathError};
    use super::{ReorgPlan, ReorgPlanValidationError};

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

    #[test]
    fn same_tip_is_noop_plan() {
        let idx = linear_chain();
        let p = ReorgPlan::try_from_tips(&idx, "a2", "a2").unwrap();
        assert!(p.is_noop());
        assert_eq!(p.fork_hash, "a2");
        assert!(p.rollback_ordered.is_empty());
        assert!(p.apply_ordered.is_empty());
    }

    #[test]
    fn direct_parent_child_extension_plan() {
        let idx = linear_chain();
        let p = ReorgPlan::try_from_tips(&idx, "a1", "a2").unwrap();
        assert_eq!(p.fork_hash, "a1");
        assert!(p.rollback_ordered.is_empty());
        assert_eq!(p.apply_ordered, vec!["a2"]);
    }

    #[test]
    fn simple_fork_plan() {
        let mut idx = BlockIndex::new();
        idx.insert("g".into(), genesis());
        idx.insert("b1".into(), child("g", 1));
        idx.insert("a2".into(), child("b1", 2));
        idx.insert("c2".into(), child("b1", 2));
        let p = ReorgPlan::try_from_tips(&idx, "a2", "c2").unwrap();
        assert_eq!(p.fork_hash, "b1");
        assert_eq!(p.rollback_ordered, vec!["a2"]);
        assert_eq!(p.apply_ordered, vec!["c2"]);
    }

    #[test]
    fn deeper_fork_plan_orders() {
        let idx = deeper_fork_index();
        let p = ReorgPlan::try_from_tips(&idx, "o4", "n5").unwrap();
        assert_eq!(p.fork_hash, "b2");
        assert_eq!(p.rollback_ordered, vec!["o4", "o3"]);
        assert_eq!(p.apply_ordered, vec!["n3", "n4", "n5"]);
    }

    #[test]
    fn rollback_then_apply_ordering_is_deterministic() {
        let idx = deeper_fork_index();
        let a = ReorgPlan::try_from_tips(&idx, "o4", "n5").unwrap();
        let b = ReorgPlan::try_from_tips(&idx, "o4", "n5").unwrap();
        assert_eq!(a, b);
        // Rollback: tip-first along old branch; apply: reverse of tip-first new_suffix.
        assert_eq!(a.rollback_ordered, vec!["o4", "o3"]);
        assert_eq!(a.apply_ordered, vec!["n3", "n4", "n5"]);
    }

    #[test]
    fn unknown_old_tip_fails() {
        let idx = linear_chain();
        assert!(matches!(
            ReorgPlan::try_from_tips(&idx, "missing", "a2"),
            Err(BlockPathError::UnknownBlock(_))
        ));
    }

    #[test]
    fn unknown_new_tip_fails() {
        let idx = linear_chain();
        assert!(matches!(
            ReorgPlan::try_from_tips(&idx, "a2", "nope"),
            Err(BlockPathError::UnknownBlock(_))
        ));
    }

    #[test]
    fn apply_order_parent_before_child() {
        // apply_ordered[0]'s parent on the new branch must be fork_hash; each next extends the chain.
        let idx = deeper_fork_index();
        let p = ReorgPlan::try_from_tips(&idx, "o4", "n5").unwrap();
        let slices = idx.fork_slices_between_tips("o4", "n5").unwrap();
        assert_eq!(slices.new_suffix, vec!["n5", "n4", "n3"]);
        assert_eq!(p.apply_ordered, vec!["n3", "n4", "n5"]);
        // n3 parent is b2 (fork)
        let n3 = idx.get("n3").unwrap();
        assert_eq!(n3.parent_hash, p.fork_hash);
        let n4 = idx.get("n4").unwrap();
        assert_eq!(n4.parent_hash, "n3");
        let n5 = idx.get("n5").unwrap();
        assert_eq!(n5.parent_hash, "n4");
    }

    #[test]
    fn malformed_ancestry_rejects_plan() {
        let mut idx = BlockIndex::new();
        idx.insert("g".into(), genesis());
        idx.insert(
            "bad".into(),
            BlockIndexEntry {
                parent_hash: "g".into(),
                height: 99,
            },
        );
        assert!(matches!(
            ReorgPlan::try_from_tips(&idx, "bad", "bad"),
            Err(BlockPathError::MalformedAncestry { .. })
        ));
    }

    #[test]
    fn validate_noop_plan_ok() {
        let idx = linear_chain();
        let p = ReorgPlan::try_from_tips(&idx, "a2", "a2").unwrap();
        assert_eq!(p.validate_against_index(&idx), Ok(()));
    }

    #[test]
    fn validate_rollback_only_new_is_ancestor_of_old() {
        let idx = linear_chain();
        let p = ReorgPlan::try_from_tips(&idx, "a2", "a1").unwrap();
        assert_eq!(p.rollback_ordered, vec!["a2"]);
        assert!(p.apply_ordered.is_empty());
        assert_eq!(p.validate_against_index(&idx), Ok(()));
    }

    #[test]
    fn validate_apply_only_old_is_ancestor_of_new() {
        let idx = linear_chain();
        let p = ReorgPlan::try_from_tips(&idx, "a1", "a2").unwrap();
        assert!(p.rollback_ordered.is_empty());
        assert_eq!(p.apply_ordered, vec!["a2"]);
        assert_eq!(p.validate_against_index(&idx), Ok(()));
    }

    #[test]
    fn validate_forked_rollback_and_apply_ok() {
        let idx = deeper_fork_index();
        let p = ReorgPlan::try_from_tips(&idx, "o4", "n5").unwrap();
        assert_eq!(p.validate_against_index(&idx), Ok(()));
    }

    #[test]
    fn validate_unknown_hash_in_rollback_rejected() {
        let idx = linear_chain();
        let p = ReorgPlan {
            fork_hash: "g".into(),
            rollback_ordered: vec!["ghost".into()],
            apply_ordered: vec![],
        };
        assert!(matches!(
            p.validate_against_index(&idx),
            Err(ReorgPlanValidationError::BlockPath(BlockPathError::UnknownBlock(h))) if h == "ghost"
        ));
    }

    #[test]
    fn validate_unknown_hash_in_apply_rejected() {
        let idx = linear_chain();
        let p = ReorgPlan {
            fork_hash: "g".into(),
            rollback_ordered: vec![],
            apply_ordered: vec!["ghost".into()],
        };
        assert!(matches!(
            p.validate_against_index(&idx),
            Err(ReorgPlanValidationError::BlockPath(BlockPathError::UnknownBlock(h))) if h == "ghost"
        ));
    }

    #[test]
    fn validate_unknown_fork_rejected() {
        let idx = linear_chain();
        let p = ReorgPlan {
            fork_hash: "ghost".into(),
            rollback_ordered: vec![],
            apply_ordered: vec![],
        };
        assert!(matches!(
            p.validate_against_index(&idx),
            Err(ReorgPlanValidationError::BlockPath(BlockPathError::UnknownBlock(h))) if h == "ghost"
        ));
    }

    #[test]
    fn validate_broken_rollback_adjacency_rejected() {
        let idx = linear_chain();
        // a2's parent is a1, not g — chain breaks toward fork.
        let p = ReorgPlan {
            fork_hash: "g".into(),
            rollback_ordered: vec!["a2".into()],
            apply_ordered: vec![],
        };
        assert!(matches!(
            p.validate_against_index(&idx),
            Err(ReorgPlanValidationError::BrokenRollbackChain { block, .. }) if block == "a2"
        ));
    }

    #[test]
    fn validate_wrong_fork_rejected() {
        let idx = linear_chain();
        // Correct fork for single-step rollback a2 → a1 is a1.
        let correct = ReorgPlan {
            fork_hash: "a1".into(),
            rollback_ordered: vec!["a2".into()],
            apply_ordered: vec![],
        };
        assert_eq!(correct.validate_against_index(&idx), Ok(()));
        // Wrong fork: claim g while rolling only a2 (parent of a2 is a1, not g).
        let wrong = ReorgPlan {
            fork_hash: "g".into(),
            rollback_ordered: vec!["a2".into()],
            apply_ordered: vec![],
        };
        assert!(matches!(
            wrong.validate_against_index(&idx),
            Err(ReorgPlanValidationError::BrokenRollbackChain { block, .. }) if block == "a2"
        ));
    }

    #[test]
    fn validate_broken_apply_adjacency_rejected() {
        let idx = linear_chain();
        // After fork g, apply must be a1 then a2; reversed order breaks parent link.
        let p = ReorgPlan {
            fork_hash: "g".into(),
            rollback_ordered: vec![],
            apply_ordered: vec!["a2".into(), "a1".into()],
        };
        assert!(matches!(
            p.validate_against_index(&idx),
            Err(ReorgPlanValidationError::BrokenApplyChain { block, .. }) if block == "a2"
        ));
    }

    #[test]
    fn validate_malformed_block_in_apply_rejected() {
        let mut idx = BlockIndex::new();
        idx.insert("g".into(), genesis());
        idx.insert(
            "bad".into(),
            BlockIndexEntry {
                parent_hash: "g".into(),
                height: 7,
            },
        );
        let p = ReorgPlan {
            fork_hash: "g".into(),
            rollback_ordered: vec![],
            apply_ordered: vec!["bad".into()],
        };
        assert!(matches!(
            p.validate_against_index(&idx),
            Err(ReorgPlanValidationError::BlockPath(
                BlockPathError::MalformedAncestry { .. }
            ))
        ));
    }

    #[test]
    fn validate_duplicate_in_rollback_rejected() {
        let idx = linear_chain();
        let p = ReorgPlan {
            fork_hash: "g".into(),
            rollback_ordered: vec!["a2".into(), "a2".into()],
            apply_ordered: vec![],
        };
        assert!(matches!(
            p.validate_against_index(&idx),
            Err(ReorgPlanValidationError::DuplicateBlock { side, .. }) if side == "rollback"
        ));
    }

    #[test]
    fn validate_fork_inside_segments_rejected() {
        let idx = linear_chain();
        let p = ReorgPlan {
            fork_hash: "a1".into(),
            rollback_ordered: vec!["a1".into()],
            apply_ordered: vec![],
        };
        assert!(matches!(
            p.validate_against_index(&idx),
            Err(ReorgPlanValidationError::ForkInsideSegments { block }) if block == "a1"
        ));
    }
}
