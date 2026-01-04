//! Property-based tests for balanced parentheses operations.
//!
//! Tests invariants like:
//! - find_close(find_open(i)) == i for close positions
//! - find_open(find_close(i)) == i for open positions
//! - RangeMin matches linear scan
//! - Tree navigation consistency

use proptest::prelude::*;
use succinctly::bp::{self, BalancedParens};

/// Strategy for generating valid balanced parentheses sequences.
///
/// Generates sequences with specified approximate node count and max depth.
fn balanced_parens_strategy(
    max_nodes: usize,
    max_depth: usize,
) -> impl Strategy<Value = (Vec<u64>, usize)> {
    (1..=max_nodes, any::<u64>()).prop_map(move |(target_nodes, seed)| {
        generate_balanced_parens(target_nodes, max_depth, seed)
    })
}

/// Generate a balanced parentheses sequence deterministically from seed.
fn generate_balanced_parens(node_count: usize, max_depth: usize, seed: u64) -> (Vec<u64>, usize) {
    use std::num::Wrapping;

    // Simple PRNG for determinism
    let mut state = Wrapping(seed);
    let mut next_rand = || {
        state = state * Wrapping(6364136223846793005u64) + Wrapping(1);
        state.0
    };

    let mut bits = Vec::with_capacity(node_count * 2);
    let mut depth = 0;

    while bits.len() < node_count * 2 {
        if depth == 0 {
            bits.push(true);
            depth += 1;
        } else if depth >= max_depth {
            bits.push(false);
            depth -= 1;
        } else {
            // Bias towards opening to create varied structures
            if next_rand() % 100 < 55 {
                bits.push(true);
                depth += 1;
            } else {
                bits.push(false);
                depth -= 1;
            }
        }
    }

    // Close any remaining open parens
    while depth > 0 {
        bits.push(false);
        depth -= 1;
    }

    let len = bits.len();

    // Pack into words
    let word_count = len.div_ceil(64);
    let mut words = vec![0u64; word_count];
    for (i, &bit) in bits.iter().enumerate() {
        if bit {
            words[i / 64] |= 1 << (i % 64);
        }
    }

    (words, len)
}

/// Collect all open positions in a BP sequence.
fn collect_open_positions(bp: &BalancedParens) -> Vec<usize> {
    (0..bp.len()).filter(|&p| bp.is_open(p)).collect()
}

/// Collect all close positions in a BP sequence.
fn collect_close_positions(bp: &BalancedParens) -> Vec<usize> {
    (0..bp.len()).filter(|&p| bp.is_close(p)).collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// find_open(find_close(i)) == i for all open positions
    #[test]
    fn prop_find_close_find_open_roundtrip(
        (words, len) in balanced_parens_strategy(500, 50)
    ) {
        let bp = BalancedParens::new(words, len);

        for p in collect_open_positions(&bp) {
            if let Some(close) = bp.find_close(p) {
                let open = bp.find_open(close);
                prop_assert_eq!(open, Some(p),
                    "find_open(find_close({})) = {:?}, expected Some({})", p, open, p);
            }
        }
    }

    /// find_close(find_open(i)) == i for all close positions
    #[test]
    fn prop_find_open_find_close_roundtrip(
        (words, len) in balanced_parens_strategy(500, 50)
    ) {
        let bp = BalancedParens::new(words, len);

        for p in collect_close_positions(&bp) {
            if let Some(open) = bp.find_open(p) {
                let close = bp.find_close(open);
                prop_assert_eq!(close, Some(p),
                    "find_close(find_open({})) = {:?}, expected Some({})", p, close, p);
            }
        }
    }

    /// RangeMin find_close matches linear scan
    #[test]
    fn prop_rangemin_matches_linear_find_close(
        (words, len) in balanced_parens_strategy(500, 50)
    ) {
        let bp = BalancedParens::new(words.clone(), len);

        for p in collect_open_positions(&bp) {
            let rangemin_result = bp.find_close(p);
            let linear_result = bp::find_close(&words, len, p);
            prop_assert_eq!(rangemin_result, linear_result,
                "find_close({}) mismatch: rangemin={:?}, linear={:?}", p, rangemin_result, linear_result);
        }
    }

    /// RangeMin find_open matches linear scan
    #[test]
    fn prop_rangemin_matches_linear_find_open(
        (words, len) in balanced_parens_strategy(500, 50)
    ) {
        let bp = BalancedParens::new(words.clone(), len);

        for p in collect_close_positions(&bp) {
            let rangemin_result = bp.find_open(p);
            let linear_result = bp::find_open(&words, len, p);
            prop_assert_eq!(rangemin_result, linear_result,
                "find_open({}) mismatch: rangemin={:?}, linear={:?}", p, rangemin_result, linear_result);
        }
    }

    /// RangeMin enclose matches linear scan
    #[test]
    fn prop_rangemin_matches_linear_enclose(
        (words, len) in balanced_parens_strategy(500, 50)
    ) {
        let bp = BalancedParens::new(words.clone(), len);

        for p in collect_open_positions(&bp) {
            let rangemin_result = bp.enclose(p);
            let linear_result = bp::enclose(&words, len, p);
            prop_assert_eq!(rangemin_result, linear_result,
                "enclose({}) mismatch: rangemin={:?}, linear={:?}", p, rangemin_result, linear_result);
        }
    }

    /// find_close(i) > i for open positions
    #[test]
    fn prop_find_close_greater_than_position(
        (words, len) in balanced_parens_strategy(500, 50)
    ) {
        let bp = BalancedParens::new(words, len);

        for p in collect_open_positions(&bp) {
            if let Some(close) = bp.find_close(p) {
                prop_assert!(close > p,
                    "find_close({}) = {} should be > {}", p, close, p);
            }
        }
    }

    /// find_open(i) < i for close positions
    #[test]
    fn prop_find_open_less_than_position(
        (words, len) in balanced_parens_strategy(500, 50)
    ) {
        let bp = BalancedParens::new(words, len);

        for p in collect_close_positions(&bp) {
            if let Some(open) = bp.find_open(p) {
                prop_assert!(open < p,
                    "find_open({}) = {} should be < {}", p, open, p);
            }
        }
    }

    /// excess at close position equals excess at open position minus 1
    ///
    /// If open is at position p with excess e, the matching close is at position c
    /// where excess(c) = e - 1 (the close brings excess back down by 1).
    #[test]
    fn prop_excess_at_close_is_one_less(
        (words, len) in balanced_parens_strategy(500, 50)
    ) {
        let bp = BalancedParens::new(words, len);

        for p in collect_open_positions(&bp) {
            if let Some(close) = bp.find_close(p) {
                let excess_at_open = bp.excess(p);
                let excess_at_close = bp.excess(close);
                // After open at p: excess = e
                // After close at c: excess = e - 1 (one open matched by one close)
                prop_assert_eq!(excess_at_close, excess_at_open - 1,
                    "excess({}) = {}, excess({}) = {}, expected excess_close = excess_open - 1",
                    p, excess_at_open, close, excess_at_close);
            }
        }
    }

    /// first_child returns position p+1 if it's an open
    #[test]
    fn prop_first_child_is_next_open(
        (words, len) in balanced_parens_strategy(500, 50)
    ) {
        let bp = BalancedParens::new(words, len);

        for p in collect_open_positions(&bp) {
            let child = bp.first_child(p);
            if p + 1 < len && bp.is_open(p + 1) {
                prop_assert_eq!(child, Some(p + 1),
                    "first_child({}) should be Some({})", p, p + 1);
            } else {
                prop_assert_eq!(child, None,
                    "first_child({}) should be None", p);
            }
        }
    }

    /// next_sibling(p) == find_close(p) + 1 if that's an open
    #[test]
    fn prop_next_sibling_after_close(
        (words, len) in balanced_parens_strategy(500, 50)
    ) {
        let bp = BalancedParens::new(words, len);

        for p in collect_open_positions(&bp) {
            if let Some(close) = bp.find_close(p) {
                let sibling = bp.next_sibling(p);
                if close + 1 < len && bp.is_open(close + 1) {
                    prop_assert_eq!(sibling, Some(close + 1),
                        "next_sibling({}) should be Some({})", p, close + 1);
                } else {
                    prop_assert_eq!(sibling, None,
                        "next_sibling({}) should be None", p);
                }
            }
        }
    }

    /// parent(p) == enclose(p) for open positions
    #[test]
    fn prop_parent_equals_enclose(
        (words, len) in balanced_parens_strategy(500, 50)
    ) {
        let bp = BalancedParens::new(words, len);

        for p in collect_open_positions(&bp) {
            let parent = bp.parent(p);
            let enclose = bp.enclose(p);
            prop_assert_eq!(parent, enclose,
                "parent({}) = {:?}, enclose({}) = {:?}", p, parent, p, enclose);
        }
    }

    /// Number of opens equals number of closes (balanced sequence)
    #[test]
    fn prop_balanced_open_close_count(
        (words, len) in balanced_parens_strategy(500, 50)
    ) {
        let bp = BalancedParens::new(words, len);
        let opens = collect_open_positions(&bp).len();
        let closes = collect_close_positions(&bp).len();
        prop_assert_eq!(opens, closes,
            "opens ({}) should equal closes ({})", opens, closes);
    }

    /// Depth-first traversal visits all nodes
    #[test]
    fn prop_dfs_visits_all_nodes(
        (words, len) in balanced_parens_strategy(200, 30)
    ) {
        let bp = BalancedParens::new(words, len);

        if len == 0 {
            return Ok(());
        }

        let mut visited = vec![false; len];
        let mut stack = vec![0usize];

        while let Some(pos) = stack.pop() {
            if pos >= len || !bp.is_open(pos) {
                continue;
            }

            visited[pos] = true;

            // Push next sibling (process after current subtree)
            if let Some(sib) = bp.next_sibling(pos) {
                stack.push(sib);
            }

            // Push first child (process next)
            if let Some(child) = bp.first_child(pos) {
                stack.push(child);
            }
        }

        // All open positions should be visited
        for p in collect_open_positions(&bp) {
            prop_assert!(visited[p], "position {} not visited during DFS", p);
        }
    }
}

/// Tests for edge cases
#[cfg(test)]
mod edge_cases {
    use super::*;

    #[test]
    fn test_single_pair() {
        // "()" = 0b01
        let bp = BalancedParens::new(vec![0b01], 2);
        assert_eq!(bp.find_close(0), Some(1));
        assert_eq!(bp.find_open(1), Some(0));
        assert_eq!(bp.enclose(0), None); // root has no parent
        assert_eq!(bp.first_child(0), None);
        assert_eq!(bp.next_sibling(0), None);
    }

    #[test]
    fn test_two_pairs() {
        // "()()" = 0b0101
        let bp = BalancedParens::new(vec![0b0101], 4);
        assert_eq!(bp.find_close(0), Some(1));
        assert_eq!(bp.find_close(2), Some(3));
        assert_eq!(bp.next_sibling(0), Some(2));
        assert_eq!(bp.next_sibling(2), None);
    }

    #[test]
    fn test_nested_pairs() {
        // "(())" = 0b0011
        let bp = BalancedParens::new(vec![0b0011], 4);
        assert_eq!(bp.find_close(0), Some(3));
        assert_eq!(bp.find_close(1), Some(2));
        assert_eq!(bp.first_child(0), Some(1));
        assert_eq!(bp.enclose(1), Some(0));
    }

    #[test]
    fn test_deep_nesting() {
        // "((((...)))) with depth 32
        let depth: usize = 32;
        let len = depth * 2;
        let mut words = vec![0u64; len.div_ceil(64)];

        // First `depth` bits are opens
        for i in 0..depth {
            words[i / 64] |= 1 << (i % 64);
        }

        let bp = BalancedParens::new(words.clone(), len);

        // Root matches last close
        assert_eq!(bp.find_close(0), Some(len - 1));

        // Each inner open matches its corresponding close
        for i in 0..depth {
            assert_eq!(
                bp.find_close(i),
                Some(len - 1 - i),
                "find_close({}) should be {}",
                i,
                len - 1 - i
            );
        }
    }

    #[test]
    fn test_multi_word_sequence() {
        // Create sequence spanning multiple words
        let (words, len) = generate_balanced_parens(100, 20, 12345);
        let bp = BalancedParens::new(words.clone(), len);

        // Verify roundtrip for all positions
        for p in 0..len {
            if bp.is_open(p) {
                if let Some(close) = bp.find_close(p) {
                    assert_eq!(
                        bp.find_open(close),
                        Some(p),
                        "roundtrip failed at open position {}",
                        p
                    );
                }
            }
        }
    }

    #[test]
    fn test_excess_monotonicity_within_subtree() {
        // "(()())" = 0b001011
        let bp = BalancedParens::new(vec![0b001011], 6);

        // Excess at each position:
        // pos 0 (open): 1
        // pos 1 (open): 2
        // pos 2 (close): 1
        // pos 3 (open): 2
        // pos 4 (close): 1
        // pos 5 (close): 0
        assert_eq!(bp.excess(0), 1);
        assert_eq!(bp.excess(1), 2);
        assert_eq!(bp.excess(2), 1);
        assert_eq!(bp.excess(3), 2);
        assert_eq!(bp.excess(4), 1);
        assert_eq!(bp.excess(5), 0);
    }
}
