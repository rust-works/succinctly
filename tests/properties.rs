//! Property-based tests for rank/select operations.

use proptest::prelude::*;
use succinctly::{BitVec, RankSelect};

proptest! {
    /// rank1(i) + rank0(i) == i
    #[test]
    fn prop_rank_sum(
        words in prop::collection::vec(any::<u64>(), 1..50),
        i_ratio in 0.0..1.0f64
    ) {
        let len = words.len() * 64;
        let bv = BitVec::from_words(words, len);
        let i = (i_ratio * len as f64) as usize;

        if i <= len {
            prop_assert_eq!(bv.rank1(i) + bv.rank0(i), i);
        }
    }

    /// rank1 is monotonically increasing
    #[test]
    fn prop_rank_monotonic(
        words in prop::collection::vec(any::<u64>(), 1..20),
    ) {
        let len = words.len() * 64;
        let bv = BitVec::from_words(words, len);

        let mut prev_rank = 0;
        for i in 0..=len {
            let rank = bv.rank1(i);
            prop_assert!(rank >= prev_rank,
                "rank1({}) = {} < rank1({}) = {}", i, rank, i.saturating_sub(1), prev_rank);
            prop_assert!(rank <= prev_rank + 1,
                "rank1 jumped by more than 1 at position {}", i);
            prev_rank = rank;
        }
    }

    /// select1(k) returns strictly increasing positions
    #[test]
    fn prop_select_monotonic(
        words in prop::collection::vec(any::<u64>(), 1..20),
    ) {
        let len = words.len() * 64;
        let bv = BitVec::from_words(words, len);
        let ones = bv.count_ones();

        let mut prev_pos: Option<usize> = None;
        for k in 0..ones {
            let pos = bv.select1(k);
            if let (Some(prev), Some(curr)) = (prev_pos, pos) {
                prop_assert!(curr > prev,
                    "select1({}) = {} <= select1({}) = {}", k, curr, k.saturating_sub(1), prev);
            }
            prev_pos = pos;
        }
    }

    /// rank1(select1(k) + 1) - 1 == k for valid k
    #[test]
    fn prop_rank_of_select(
        words in prop::collection::vec(any::<u64>(), 1..50),
        k_ratio in 0.0..1.0f64
    ) {
        let len = words.len() * 64;
        let bv = BitVec::from_words(words, len);
        let ones = bv.count_ones();

        if ones > 0 {
            let k = ((k_ratio * ones as f64) as usize).min(ones - 1);
            if let Some(pos) = bv.select1(k) {
                // rank1(pos) should be k (counting bits before pos)
                // Actually, rank1(pos) counts bits in [0, pos), so if bit at pos is set,
                // rank1(pos+1) - 1 == k
                prop_assert_eq!(bv.rank1(pos + 1).saturating_sub(1), k,
                    "rank1(select1({}) + 1) - 1 should equal {}", k, k);
            }
        }
    }

    /// select1(rank1(i)) == i when bit at i is set
    #[test]
    fn prop_select_of_rank(
        words in prop::collection::vec(any::<u64>(), 1..50),
        i_ratio in 0.0..1.0f64
    ) {
        let len = words.len() * 64;
        let bv = BitVec::from_words(words.clone(), len);
        let i = (i_ratio * len as f64) as usize;

        if i < len && bv.get(i) {
            let rank = bv.rank1(i);
            prop_assert_eq!(bv.select1(rank), Some(i),
                "select1(rank1({})) should equal {}", i, i);
        }
    }

    /// count_ones matches word-by-word count
    #[test]
    fn prop_count_ones(
        words in prop::collection::vec(any::<u64>(), 0..100),
    ) {
        let len = words.len() * 64;
        let expected: usize = words.iter().map(|w| w.count_ones() as usize).sum();
        let bv = BitVec::from_words(words, len);
        prop_assert_eq!(bv.count_ones(), expected);
    }

    /// rank1(len) == count_ones
    #[test]
    fn prop_rank_at_end(
        words in prop::collection::vec(any::<u64>(), 0..100),
    ) {
        let len = words.len() * 64;
        let bv = BitVec::from_words(words, len);
        prop_assert_eq!(bv.rank1(len), bv.count_ones());
    }

    /// get(i) matches direct word access
    #[test]
    fn prop_get_matches_words(
        words in prop::collection::vec(any::<u64>(), 1..20),
        i_ratio in 0.0..1.0f64
    ) {
        let len = words.len() * 64;
        let bv = BitVec::from_words(words.clone(), len);
        let i = (i_ratio * len as f64) as usize;

        if i < len {
            let word_idx = i / 64;
            let bit_idx = i % 64;
            let expected = (words[word_idx] >> bit_idx) & 1 == 1;
            prop_assert_eq!(bv.get(i), expected);
        }
    }

    /// select0 + select1 cover all positions
    #[test]
    fn prop_select_coverage(
        words in prop::collection::vec(any::<u64>(), 1..20),
    ) {
        let len = words.len() * 64;
        let bv = BitVec::from_words(words, len);
        let ones = bv.count_ones();
        let zeros = bv.count_zeros();

        // Every select1 position should have bit set
        for k in 0..ones {
            if let Some(pos) = bv.select1(k) {
                prop_assert!(bv.get(pos), "select1({}) = {} but bit not set", k, pos);
            }
        }

        // Every select0 position should have bit unset
        for k in 0..zeros {
            if let Some(pos) = bv.select0(k) {
                prop_assert!(!bv.get(pos), "select0({}) = {} but bit is set", k, pos);
            }
        }
    }
}

/// Reference implementation for comparison
fn reference_rank1(words: &[u64], len: usize, i: usize) -> usize {
    let mut count = 0;
    for bit_pos in 0..i.min(len) {
        let word_idx = bit_pos / 64;
        let bit_idx = bit_pos % 64;
        if word_idx < words.len() && (words[word_idx] >> bit_idx) & 1 == 1 {
            count += 1;
        }
    }
    count
}

fn reference_select1(words: &[u64], len: usize, k: usize) -> Option<usize> {
    let mut count = 0;
    for bit_pos in 0..len {
        let word_idx = bit_pos / 64;
        let bit_idx = bit_pos % 64;
        if word_idx < words.len() && (words[word_idx] >> bit_idx) & 1 == 1 {
            if count == k {
                return Some(bit_pos);
            }
            count += 1;
        }
    }
    None
}

proptest! {
    /// rank1 matches reference implementation
    #[test]
    fn prop_rank_matches_reference(
        words in prop::collection::vec(any::<u64>(), 1..30),
    ) {
        let len = words.len() * 64;
        let bv = BitVec::from_words(words.clone(), len);

        // Test at various positions
        for i in (0..=len).step_by(7) {
            let expected = reference_rank1(&words, len, i);
            let actual = bv.rank1(i);
            prop_assert_eq!(actual, expected, "rank1({}) mismatch", i);
        }
    }

    /// select1 matches reference implementation
    #[test]
    fn prop_select_matches_reference(
        words in prop::collection::vec(any::<u64>(), 1..30),
    ) {
        let len = words.len() * 64;
        let bv = BitVec::from_words(words.clone(), len);
        let ones = bv.count_ones();

        // Test at various k values
        for k in (0..ones).step_by(7) {
            let expected = reference_select1(&words, len, k);
            let actual = bv.select1(k);
            prop_assert_eq!(actual, expected, "select1({}) mismatch", k);
        }
    }
}
