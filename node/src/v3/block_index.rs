//! Minimal **block header facts** indexed by `block_hash`.
//!
//! **Inert:** not backed by `chain.blocks`, not used for canonical tip or sync.
//! A future wired implementation may replace this with disk-backed structures;
//! the shape (`hash` → `parent_hash`, `height`) matches `docs/reorg_model.md` §9.

use std::collections::{HashMap, HashSet};

/// Parent linkage and height for one block, keyed elsewhere by that block’s `block_hash`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockIndexEntry {
    pub parent_hash: String,
    pub height: u64,
}

/// Result of [`BlockIndex::fork_slices_between_tips`]: common ancestor and both branch suffixes.
///
/// Suffixes are ordered **tip-first** (first element is the branch tip, last is the block
/// whose parent is `fork_hash` on that branch). `fork_hash` itself is **not** included in either suffix.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForkSlices {
    pub fork_hash: String,
    pub old_suffix: Vec<String>,
    pub new_suffix: Vec<String>,
}

/// Hardened path traversal failures (cycles, gaps, inconsistent height).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BlockPathError {
    /// Hash not present in the index.
    UnknownBlock(String),
    /// `height > 0` but `parent_hash` empty or parent not in index.
    MissingParentLink { block: String },
    /// Same hash seen twice while walking toward genesis.
    Cycle { block: String },
    /// Walked `new_tip` to genesis without intersecting the `old_tip` ancestor set.
    NoCommonAncestor,
    /// Parent exists but `parent.height + 1 != child.height`.
    MalformedAncestry { block: String, detail: &'static str },
}

/// Library-only / test-oriented map: `block_hash` → `(parent_hash, height)`.
#[derive(Debug, Default, Clone)]
pub struct BlockIndex {
    by_hash: HashMap<String, BlockIndexEntry>,
}

impl BlockIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(
        &mut self,
        block_hash: String,
        entry: BlockIndexEntry,
    ) -> Option<BlockIndexEntry> {
        self.by_hash.insert(block_hash, entry)
    }

    pub fn get(&self, block_hash: &str) -> Option<&BlockIndexEntry> {
        self.by_hash.get(block_hash)
    }

    pub fn len(&self) -> usize {
        self.by_hash.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_hash.is_empty()
    }

    /// Walk from `tip_hash` toward genesis following `parent_hash` until missing or height 0.
    ///
    /// **Soft** helper: does **not** detect cycles or height consistency — prefer
    /// [`path_from_tip_toward_genesis`](Self::path_from_tip_toward_genesis) for new code.
    pub fn ancestors_to_height_zero(&self, tip_hash: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut cur = tip_hash.to_string();
        loop {
            out.push(cur.clone());
            let Some(e) = self.by_hash.get(&cur) else {
                break;
            };
            if e.height == 0 {
                break;
            }
            cur = e.parent_hash.clone();
        }
        out
    }

    /// Ordered `[tip, parent, …, genesis]` with cycle, missing-link, and height checks.
    pub fn path_from_tip_toward_genesis(&self, tip: &str) -> Result<Vec<String>, BlockPathError> {
        let mut chain = Vec::new();
        let mut cur = tip.to_string();
        let mut visited = HashSet::new();
        let limit = self.by_hash.len().saturating_add(2);
        for _ in 0..limit {
            if visited.contains(&cur) {
                return Err(BlockPathError::Cycle { block: cur });
            }
            visited.insert(cur.clone());
            let e = self
                .get(&cur)
                .ok_or_else(|| BlockPathError::UnknownBlock(cur.clone()))?;
            chain.push(cur.clone());
            if e.height == 0 {
                return Ok(chain);
            }
            cur = self.validated_parent_hash(&cur, e)?;
        }
        Err(BlockPathError::Cycle { block: cur })
    }

    /// [`ForkSlices`] between `old_tip` and `new_tip` for in-memory reorg planning only.
    ///
    /// **Inert:** not used by commit, storage, or network.
    pub fn fork_slices_between_tips(
        &self,
        old_tip: &str,
        new_tip: &str,
    ) -> Result<ForkSlices, BlockPathError> {
        let path_old = self.path_from_tip_toward_genesis(old_tip)?;
        let old_set: HashSet<_> = path_old.iter().cloned().collect();

        let mut new_suffix = Vec::new();
        let mut cur = new_tip.to_string();
        let mut visited_new = HashSet::new();
        let limit = self.by_hash.len().saturating_add(2);
        let mut fork_hash_opt: Option<String> = None;

        for _ in 0..limit {
            if visited_new.contains(&cur) {
                return Err(BlockPathError::Cycle { block: cur });
            }
            visited_new.insert(cur.clone());

            if old_set.contains(&cur) {
                fork_hash_opt = Some(cur.clone());
                break;
            }

            let e = self
                .get(&cur)
                .ok_or_else(|| BlockPathError::UnknownBlock(cur.clone()))?;
            new_suffix.push(cur.clone());

            if e.height == 0 {
                return Err(BlockPathError::NoCommonAncestor);
            }
            cur = self.validated_parent_hash(&cur, e)?;
        }

        let fork_hash =
            fork_hash_opt.ok_or_else(|| BlockPathError::Cycle { block: cur.clone() })?;

        let mut old_suffix = Vec::new();
        cur = old_tip.to_string();
        let mut visited_old = HashSet::new();
        for _ in 0..limit {
            if cur == fork_hash {
                break;
            }
            if visited_old.contains(&cur) {
                return Err(BlockPathError::Cycle { block: cur });
            }
            visited_old.insert(cur.clone());
            old_suffix.push(cur.clone());
            let e = self
                .get(&cur)
                .ok_or_else(|| BlockPathError::UnknownBlock(cur.clone()))?;
            if e.height == 0 {
                return Err(BlockPathError::NoCommonAncestor);
            }
            cur = self.validated_parent_hash(&cur, e)?;
        }

        if cur != fork_hash {
            return Err(BlockPathError::NoCommonAncestor);
        }

        Ok(ForkSlices {
            fork_hash,
            old_suffix,
            new_suffix,
        })
    }

    fn validated_parent_hash(
        &self,
        block: &str,
        e: &BlockIndexEntry,
    ) -> Result<String, BlockPathError> {
        let p = &e.parent_hash;
        if p == block {
            return Err(BlockPathError::Cycle {
                block: block.to_string(),
            });
        }
        if p.is_empty() {
            return Err(BlockPathError::MissingParentLink {
                block: block.to_string(),
            });
        }
        let pe = self
            .get(p)
            .ok_or_else(|| BlockPathError::MissingParentLink {
                block: block.to_string(),
            })?;
        if pe.height + 1 != e.height {
            return Err(BlockPathError::MalformedAncestry {
                block: block.to_string(),
                detail: "parent.height + 1 must equal child.height",
            });
        }
        Ok(p.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn genesis(_hash: &str) -> BlockIndexEntry {
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
        idx.insert("g".into(), genesis("g"));
        idx.insert("a1".into(), child("g", 1));
        idx.insert("a2".into(), child("a1", 2));
        idx
    }

    #[test]
    fn insert_get_roundtrip() {
        let mut idx = BlockIndex::new();
        idx.insert(
            "tip".into(),
            BlockIndexEntry {
                parent_hash: "p".into(),
                height: 2,
            },
        );
        let e = idx.get("tip").unwrap();
        assert_eq!(e.height, 2);
        assert_eq!(e.parent_hash, "p");
    }

    #[test]
    fn parent_walk_from_tip() {
        let mut idx = BlockIndex::new();
        idx.insert("gen".into(), genesis("gen"));
        idx.insert("b1".into(), child("gen", 1));
        idx.insert("tip".into(), child("b1", 2));
        let chain = idx.ancestors_to_height_zero("tip");
        assert_eq!(chain, vec!["tip", "b1", "gen"]);
    }

    #[test]
    fn fork_same_tip() {
        let idx = linear_chain();
        let f = idx.fork_slices_between_tips("a2", "a2").unwrap();
        assert_eq!(f.fork_hash, "a2");
        assert!(f.old_suffix.is_empty());
        assert!(f.new_suffix.is_empty());
    }

    #[test]
    fn fork_direct_parent_child() {
        let idx = linear_chain();
        let f = idx.fork_slices_between_tips("a1", "a2").unwrap();
        assert_eq!(f.fork_hash, "a1");
        assert!(f.old_suffix.is_empty());
        assert_eq!(f.new_suffix, vec!["a2"]);
    }

    #[test]
    fn fork_simple_two_tips() {
        let mut idx = BlockIndex::new();
        idx.insert("g".into(), genesis("g"));
        idx.insert("b1".into(), child("g", 1));
        idx.insert("a2".into(), child("b1", 2));
        idx.insert("c2".into(), child("b1", 2));
        let f = idx.fork_slices_between_tips("a2", "c2").unwrap();
        assert_eq!(f.fork_hash, "b1");
        assert_eq!(f.old_suffix, vec!["a2"]);
        assert_eq!(f.new_suffix, vec!["c2"]);
    }

    #[test]
    fn fork_deeper() {
        let mut idx = BlockIndex::new();
        idx.insert("g".into(), genesis("g"));
        idx.insert("b1".into(), child("g", 1));
        idx.insert("b2".into(), child("b1", 2));
        idx.insert("o3".into(), child("b2", 3));
        idx.insert("o4".into(), child("o3", 4));
        idx.insert("n3".into(), child("b2", 3));
        idx.insert("n4".into(), child("n3", 4));
        idx.insert("n5".into(), child("n4", 5));
        let f = idx.fork_slices_between_tips("o4", "n5").unwrap();
        assert_eq!(f.fork_hash, "b2");
        assert_eq!(f.old_suffix, vec!["o4", "o3"]);
        assert_eq!(f.new_suffix, vec!["n5", "n4", "n3"]);
    }

    #[test]
    fn missing_parent_fails() {
        let mut idx = BlockIndex::new();
        idx.insert(
            "orphan".into(),
            BlockIndexEntry {
                parent_hash: "missing".into(),
                height: 1,
            },
        );
        assert!(matches!(
            idx.fork_slices_between_tips("orphan", "orphan"),
            Err(BlockPathError::MissingParentLink { .. })
        ));
    }

    #[test]
    fn cycle_detection_fails() {
        // Self-parent: only consistent "cycle" for height validation; must error as Cycle.
        let mut idx = BlockIndex::new();
        idx.insert(
            "loop".into(),
            BlockIndexEntry {
                parent_hash: "loop".into(),
                height: 1,
            },
        );
        assert!(matches!(
            idx.path_from_tip_toward_genesis("loop"),
            Err(BlockPathError::Cycle { .. })
        ));
        assert!(matches!(
            idx.fork_slices_between_tips("loop", "loop"),
            Err(BlockPathError::Cycle { .. })
        ));
    }

    #[test]
    fn two_node_swap_is_malformed_not_consistent_cycle() {
        // Height-consistent 2-cycles are impossible; inconsistent graph hits Malformed first.
        let mut idx = BlockIndex::new();
        idx.insert(
            "a".into(),
            BlockIndexEntry {
                parent_hash: "b".into(),
                height: 1,
            },
        );
        idx.insert(
            "b".into(),
            BlockIndexEntry {
                parent_hash: "a".into(),
                height: 1,
            },
        );
        assert!(matches!(
            idx.path_from_tip_toward_genesis("a"),
            Err(BlockPathError::MalformedAncestry { .. })
        ));
    }

    #[test]
    fn no_common_ancestor() {
        let mut idx = BlockIndex::new();
        idx.insert("g1".into(), genesis("g1"));
        idx.insert("g2".into(), genesis("g2"));
        idx.insert("t1".into(), child("g1", 1));
        idx.insert("t2".into(), child("g2", 1));
        assert_eq!(
            idx.fork_slices_between_tips("t1", "t2"),
            Err(BlockPathError::NoCommonAncestor)
        );
    }

    #[test]
    fn malformed_height_fails() {
        let mut idx = BlockIndex::new();
        idx.insert("g".into(), genesis("g"));
        idx.insert(
            "bad".into(),
            BlockIndexEntry {
                parent_hash: "g".into(),
                height: 5,
            },
        );
        assert!(matches!(
            idx.path_from_tip_toward_genesis("bad"),
            Err(BlockPathError::MalformedAncestry { .. })
        ));
    }

    #[test]
    fn suffix_ordering_is_tip_first_deterministic() {
        let mut idx = BlockIndex::new();
        idx.insert("g".into(), genesis("g"));
        idx.insert("x".into(), child("g", 1));
        idx.insert("y".into(), child("x", 2));
        idx.insert("z".into(), child("y", 3));
        let f = idx.fork_slices_between_tips("z", "z").unwrap();
        assert_eq!(f.old_suffix, Vec::<String>::new());
        assert_eq!(f.new_suffix, Vec::<String>::new());
        let p = idx.path_from_tip_toward_genesis("z").unwrap();
        assert_eq!(p, vec!["z", "y", "x", "g"]);
    }
}
