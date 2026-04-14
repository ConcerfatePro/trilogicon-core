//! **Inert** reorg **plan** derived from [`ForkSlices`](super::block_index::ForkSlices).
//!
//! This is **preparatory V3 scaffolding only**: it makes **rollback** and **apply** order explicit
//! for a future executor. It does **not** touch `State`, `Blockchain`, storage, network, or CLI.
//!
//! ## Semantics
//!
//! * [`ForkSlices::old_suffix`](super::block_index::ForkSlices) is **tip-first** (current committed
//!   tip first, down toward the fork). **`rollback_ordered`** is the **same order**: unwind state
//!   **one block at a time** starting at the committed tip.
//! * [`ForkSlices::new_suffix`](super::block_index::ForkSlices) is **tip-first** away from the fork.
//!   **`apply_ordered`** is the **reverse**: **parent before child** so each next block extends the
//!   post-fork chain ending at the **new** tip.

use super::block_index::{BlockIndex, BlockPathError, ForkSlices};

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
}

#[cfg(test)]
mod tests {
    use super::super::block_index::{BlockIndex, BlockIndexEntry, BlockPathError};
    use super::ReorgPlan;

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
}
