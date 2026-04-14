//! Minimal **block header facts** indexed by `block_hash`.
//!
//! **Inert:** not backed by `chain.blocks`, not used for canonical tip or sync.
//! A future wired implementation may replace this with disk-backed structures;
//! the shape (`hash` → `parent_hash`, `height`) matches `docs/reorg_model.md` §9.

use std::collections::HashMap;

/// Parent linkage and height for one block, keyed elsewhere by that block’s `block_hash`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockIndexEntry {
    pub parent_hash: String,
    pub height: u64,
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
        idx.insert(
            "gen".into(),
            BlockIndexEntry {
                parent_hash: "".into(),
                height: 0,
            },
        );
        idx.insert(
            "b1".into(),
            BlockIndexEntry {
                parent_hash: "gen".into(),
                height: 1,
            },
        );
        idx.insert(
            "tip".into(),
            BlockIndexEntry {
                parent_hash: "b1".into(),
                height: 2,
            },
        );
        let chain = idx.ancestors_to_height_zero("tip");
        assert_eq!(chain, vec!["tip", "b1", "gen"]);
    }
}
