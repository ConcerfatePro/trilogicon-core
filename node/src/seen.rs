//! Bounded duplicate-suppression for gossiped tx/block ids (not consensus truth).

use std::collections::{HashSet, VecDeque};

/// FIFO-evicted set: insert returns `false` if `id` was already present.
pub struct SeenCache {
    cap: usize,
    order: VecDeque<String>,
    set: HashSet<String>,
}

impl SeenCache {
    pub fn new(cap: usize) -> Self {
        Self {
            cap: cap.max(1),
            order: VecDeque::new(),
            set: HashSet::new(),
        }
    }

    /// Returns `true` if this id is new (inserted), `false` if duplicate.
    pub fn insert(&mut self, id: String) -> bool {
        if self.set.contains(&id) {
            return false;
        }
        while self.order.len() >= self.cap {
            if let Some(old) = self.order.pop_front() {
                self.set.remove(&old);
            }
        }
        self.set.insert(id.clone());
        self.order.push_back(id);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_evicts_oldest() {
        let mut c = SeenCache::new(2);
        assert!(c.insert("a".into()));
        assert!(c.insert("b".into()));
        assert!(c.insert("c".into()));
        assert!(!c.insert("b".into()));
        assert!(c.insert("a".into()));
    }
}
