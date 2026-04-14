//! Pure **height-first branch selection** with TB1 tie-break (lexicographically larger
//! `block_hash` wins at equal height). See `docs/fork_choice.md`.
//!
//! **Inert:** nothing in this module observes peers, clocks, or I/O. **Not** wired into
//! the live node.

use std::cmp::Ordering;

/// Tip of a valid branch: height and tip block hash (hex string, same encoding as the rest of the node).
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct TipDescriptor {
    pub height: u64,
    pub block_hash: String,
}

/// Total ordering key: **greater** `BranchScore` ⇒ **more preferred** tip under V3 docs
/// (higher height, then larger `block_hash` ASCII lex order).
///
/// This is a **documentation / engineering** ordering only — not a security property.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct BranchScore {
    height: u64,
    tie_break_lex: String,
}

impl From<&TipDescriptor> for BranchScore {
    fn from(t: &TipDescriptor) -> Self {
        BranchScore {
            height: t.height,
            tie_break_lex: t.block_hash.clone(),
        }
    }
}

/// Compare two tips for **branch selection** (`a` vs `b`).
///
/// Returns `Ordering::Greater` if **`a` is preferred** over `b`, etc.
pub fn compare_tips_for_selection(a: &TipDescriptor, b: &TipDescriptor) -> Ordering {
    BranchScore::from(a).cmp(&BranchScore::from(b))
}

/// Prefer among **validated** tip candidates; order of `tips` does not matter.
pub fn select_preferred_tip<'a>(tips: &[&'a TipDescriptor]) -> Option<&'a TipDescriptor> {
    tips.iter().copied().max_by_key(|t| BranchScore::from(*t))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tip(h: u64, hash: &str) -> TipDescriptor {
        TipDescriptor {
            height: h,
            block_hash: hash.into(),
        }
    }

    #[test]
    fn higher_height_wins() {
        let a = tip(3, "aa");
        let b = tip(4, "00");
        assert_eq!(compare_tips_for_selection(&a, &b), Ordering::Less);
        assert_eq!(select_preferred_tip(&[&a, &b]).unwrap().height, 4);
    }

    #[test]
    fn tb1_larger_hash_wins_at_equal_height() {
        let low = tip(5, "0a");
        let high = tip(5, "0b");
        assert_eq!(compare_tips_for_selection(&low, &high), Ordering::Less);
        assert_eq!(
            select_preferred_tip(&[&low, &high]).unwrap().block_hash,
            "0b"
        );
    }

    #[test]
    fn single_candidate() {
        let t = tip(1, "abc");
        assert!(std::ptr::eq(select_preferred_tip(&[&t]).unwrap(), &t));
    }

    #[test]
    fn selection_order_in_slice_is_irrelevant() {
        let a = tip(2, "zz");
        let b = tip(3, "aa");
        let c = tip(2, "yy");
        let want = &b;
        assert!(std::ptr::eq(
            select_preferred_tip(&[&a, &b, &c]).unwrap(),
            want
        ));
        assert!(std::ptr::eq(
            select_preferred_tip(&[&c, &a, &b]).unwrap(),
            want
        ));
        assert!(std::ptr::eq(
            select_preferred_tip(&[&b, &c, &a]).unwrap(),
            want
        ));
    }

    #[test]
    fn transitivity_spot() {
        let x = tip(1, "ff");
        let y = tip(2, "00");
        let z = tip(2, "11");
        // y and z same height; z wins TB1
        assert_eq!(compare_tips_for_selection(&x, &y), Ordering::Less);
        assert_eq!(compare_tips_for_selection(&y, &z), Ordering::Less);
        assert_eq!(compare_tips_for_selection(&x, &z), Ordering::Less);
    }

    #[test]
    fn grinding_tb1_flips_winner_at_same_height() {
        // Same height, only tie-break differs — shows TB1 is grindable (not security).
        let g1 = tip(
            10,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        );
        let g2 = tip(
            10,
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        );
        assert_eq!(
            select_preferred_tip(&[&g1, &g2]).unwrap().block_hash,
            g2.block_hash
        );
    }
}
