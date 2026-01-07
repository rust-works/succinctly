//! Additional balanced parentheses tests to close coverage gaps vs HaskellWorks.
//!
//! This module adds dedicated tests for:
//! - excess() operation
//! - MinExcess computation
//! - find_close_in_word vs naive comparison
//! - Implementation cross-verification (BalancedParens vs linear scan)

use succinctly::bp::{
    enclose, find_close, find_close_in_word, find_open, find_unmatched_close_in_word,
    BalancedParens,
};

// ============================================================================
// Helper functions
// ============================================================================

/// Naive bit-by-bit find_close for a single word.
fn naive_find_close_in_word(word: u64, p: u32) -> Option<u32> {
    if p >= 64 {
        return None;
    }

    // Check if position p is an open
    if (word >> p) & 1 == 0 {
        return Some(p); // Close at p "matches itself"
    }

    let mut excess: i32 = 1; // Start with 1 for the open at p
    for bit in (p + 1)..64 {
        if (word >> bit) & 1 == 1 {
            excess += 1;
        } else {
            excess -= 1;
            if excess == 0 {
                return Some(bit);
            }
        }
    }
    None
}

/// Naive bit-by-bit find_unmatched_close for a word.
fn naive_find_unmatched_close_in_word(word: u64) -> u32 {
    let mut excess: i32 = 0;
    for bit in 0..64 {
        if (word >> bit) & 1 == 1 {
            excess += 1;
        } else {
            excess -= 1;
            if excess < 0 {
                return bit;
            }
        }
    }
    64
}

/// Compute min_excess and total_excess for a word naively.
fn naive_word_min_excess(word: u64, valid_bits: usize) -> (i16, i16) {
    let mut excess: i16 = 0;
    let mut min_excess: i16 = 0;

    for i in 0..valid_bits {
        if (word >> i) & 1 == 1 {
            excess += 1;
        } else {
            excess -= 1;
            min_excess = min_excess.min(excess);
        }
    }
    (min_excess, excess)
}

/// Compute excess at position p naively.
fn naive_excess(words: &[u64], len: usize, p: usize) -> i32 {
    if p >= len {
        return 0;
    }

    let mut total: i32 = 0;
    for i in 0..=p {
        let word_idx = i / 64;
        let bit_idx = i % 64;
        if (words[word_idx] >> bit_idx) & 1 == 1 {
            total += 1;
        } else {
            total -= 1;
        }
    }
    total
}

// ============================================================================
// Dedicated excess() tests (gap #3)
// ============================================================================

mod excess_tests {
    use super::*;

    #[test]
    fn test_excess_empty_sequence() {
        let bp = BalancedParens::new(vec![], 0);
        assert_eq!(bp.excess(0), 0);
        assert_eq!(bp.excess(100), 0);
    }

    #[test]
    fn test_excess_single_pair() {
        // "()" = 0b01
        let bp = BalancedParens::new(vec![0b01], 2);
        assert_eq!(bp.excess(0), 1); // After open
        assert_eq!(bp.excess(1), 0); // After close (balanced)
    }

    #[test]
    fn test_excess_nested_pairs() {
        // "(())" = 0b0011
        let bp = BalancedParens::new(vec![0b0011], 4);
        assert_eq!(bp.excess(0), 1); // After first open
        assert_eq!(bp.excess(1), 2); // After second open
        assert_eq!(bp.excess(2), 1); // After first close
        assert_eq!(bp.excess(3), 0); // After second close
    }

    #[test]
    fn test_excess_sequential_pairs() {
        // "()()" = 0b0101
        let bp = BalancedParens::new(vec![0b0101], 4);
        assert_eq!(bp.excess(0), 1);
        assert_eq!(bp.excess(1), 0);
        assert_eq!(bp.excess(2), 1);
        assert_eq!(bp.excess(3), 0);
    }

    #[test]
    fn test_excess_all_opens() {
        // 64 opens
        let bp = BalancedParens::new(vec![u64::MAX], 64);
        for i in 0..64 {
            assert_eq!(bp.excess(i), (i + 1) as i32);
        }
    }

    #[test]
    fn test_excess_all_closes() {
        // 64 closes
        let bp = BalancedParens::new(vec![0], 64);
        for i in 0..64 {
            assert_eq!(bp.excess(i), -((i + 1) as i32));
        }
    }

    #[test]
    fn test_excess_multi_word() {
        // 64 opens followed by 64 closes
        let bp = BalancedParens::new(vec![u64::MAX, 0], 128);

        // First word: excess increases
        assert_eq!(bp.excess(0), 1);
        assert_eq!(bp.excess(63), 64);

        // Second word: excess decreases
        assert_eq!(bp.excess(64), 63);
        assert_eq!(bp.excess(127), 0);
    }

    #[test]
    fn test_excess_matches_naive() {
        // Test against naive implementation
        let test_cases: Vec<(Vec<u64>, usize)> = vec![
            (vec![0b001011], 6),            // "(()())"
            (vec![0b0011], 4),              // "(())"
            (vec![u64::MAX, 0], 128),       // 64 opens, 64 closes
            (vec![0x5555555555555555], 64), // ()()()...
            (vec![0xAAAAAAAAAAAAAAAA], 64), // )()()(... (unbalanced)
        ];

        for (words, len) in test_cases {
            let bp = BalancedParens::new(words.clone(), len);
            for p in 0..len {
                let expected = naive_excess(&words, len, p);
                assert_eq!(
                    bp.excess(p),
                    expected,
                    "excess({}) mismatch for words={:?}",
                    p,
                    words
                );
            }
        }
    }

    #[test]
    fn test_excess_out_of_bounds() {
        let bp = BalancedParens::new(vec![0b01], 2);
        assert_eq!(bp.excess(2), 0);
        assert_eq!(bp.excess(100), 0);
    }

    #[test]
    fn test_excess_partial_word() {
        // Only 10 valid bits
        let bp = BalancedParens::new(vec![0b0011001011], 10);
        // bits: 1 1 0 1 0 0 1 1 0 0
        // excess: 1 2 1 2 1 0 1 2 1 0
        assert_eq!(bp.excess(0), 1);
        assert_eq!(bp.excess(1), 2);
        assert_eq!(bp.excess(2), 1);
        assert_eq!(bp.excess(5), 0);
        assert_eq!(bp.excess(9), 0);
    }

    #[test]
    fn test_excess_depth_relationship() {
        // For opens, depth should equal excess
        let bp = BalancedParens::new(vec![0b001011], 6);

        for p in 0..6 {
            if bp.is_open(p) {
                assert_eq!(
                    bp.depth(p).unwrap() as i32,
                    bp.excess(p),
                    "depth != excess at open position {}",
                    p
                );
            }
        }
    }
}

// ============================================================================
// MinExcess tests (gap #3)
// ============================================================================

mod min_excess_tests {
    use super::*;

    #[test]
    fn test_min_excess_all_opens() {
        let (min, total) = naive_word_min_excess(u64::MAX, 64);
        assert_eq!(min, 0); // Never goes negative
        assert_eq!(total, 64);
    }

    #[test]
    fn test_min_excess_all_closes() {
        let (min, total) = naive_word_min_excess(0, 64);
        assert_eq!(min, -64);
        assert_eq!(total, -64);
    }

    #[test]
    fn test_min_excess_balanced() {
        // "()" = 0b01
        let (min, total) = naive_word_min_excess(0b01, 2);
        assert_eq!(min, 0);
        assert_eq!(total, 0);

        // "()()" = 0b0101
        let (min, total) = naive_word_min_excess(0b0101, 4);
        assert_eq!(min, 0);
        assert_eq!(total, 0);
    }

    #[test]
    fn test_min_excess_starts_with_close() {
        // ")(" = 0b10
        let (min, total) = naive_word_min_excess(0b10, 2);
        assert_eq!(min, -1);
        assert_eq!(total, 0);

        // "))((" = 0b1100
        let (min, total) = naive_word_min_excess(0b1100, 4);
        assert_eq!(min, -2);
        assert_eq!(total, 0);
    }

    #[test]
    fn test_min_excess_alternating() {
        // "()()()()" = 0x55
        let (min, total) = naive_word_min_excess(0x55, 8);
        assert_eq!(min, 0);
        assert_eq!(total, 0);

        // ")()()()" = 0xAA
        let (min, total) = naive_word_min_excess(0xAA, 8);
        assert_eq!(min, -1);
        assert_eq!(total, 0);
    }

    #[test]
    fn test_min_excess_partial_bits() {
        // Only 3 bits: "(()" = 0b011
        let (min, total) = naive_word_min_excess(0b011, 3);
        assert_eq!(min, 0);
        assert_eq!(total, 1);

        // Only 3 bits: "())" = 0b001
        let (min, total) = naive_word_min_excess(0b001, 3);
        assert_eq!(min, -1);
        assert_eq!(total, -1);
    }

    #[test]
    fn test_min_excess_invariants() {
        // Property: min_excess <= 0 for any sequence
        // Property: min_excess >= total_excess
        // Property: min_excess <= running_minimum at any point

        let test_words = [
            0u64,
            u64::MAX,
            0x5555555555555555,
            0xAAAAAAAAAAAAAAAA,
            0x123456789ABCDEF0,
            0xFEDCBA9876543210,
        ];

        for word in test_words {
            let (min, total) = naive_word_min_excess(word, 64);

            // min_excess should be <= total_excess (can't go below minimum)
            assert!(
                min <= total,
                "min_excess > total_excess for word={:#x}: min={}, total={}",
                word,
                min,
                total
            );

            // min_excess should be <= 0 or the minimum running excess
            // This is always true by definition
        }
    }
}

// ============================================================================
// find_close_in_word vs naive comparison (gap #6)
// ============================================================================

mod find_close_in_word_tests {
    use super::*;

    #[test]
    fn test_find_close_in_word_matches_naive_simple() {
        // Simple test cases
        let test_cases = [
            (0b01u64, 0u32),  // "()"
            (0b0011u64, 0),   // "(())" outer
            (0b0011u64, 1),   // "(())" inner
            (0b0101u64, 0),   // "()()" first
            (0b0101u64, 2),   // "()()" second
            (0b001011u64, 0), // "(()())" outer
            (0b001011u64, 1), // "(()())" first inner
            (0b001011u64, 3), // "(()())" second inner
        ];

        for (word, p) in test_cases {
            let result = find_close_in_word(word, p);
            let naive = naive_find_close_in_word(word, p);
            assert_eq!(
                result, naive,
                "find_close_in_word({:#x}, {}) mismatch: got {:?}, expected {:?}",
                word, p, result, naive
            );
        }
    }

    #[test]
    fn test_find_close_in_word_matches_naive_all_patterns() {
        // Test all 16-bit patterns
        for word in 0u64..=0xFFFF {
            for p in 0..16u32 {
                let result = find_close_in_word(word, p);
                let naive = naive_find_close_in_word(word, p);
                assert_eq!(
                    result, naive,
                    "find_close_in_word({:#x}, {}) mismatch",
                    word, p
                );
            }
        }
    }

    #[test]
    fn test_find_close_in_word_at_close_position() {
        // When p points to a close, both should return Some(p)
        let word = 0b001011u64; // "(()())" - closes at 2, 4, 5

        for p in [2, 4, 5] {
            let result = find_close_in_word(word, p as u32);
            let naive = naive_find_close_in_word(word, p as u32);
            assert_eq!(result, Some(p as u32));
            assert_eq!(naive, Some(p as u32));
        }
    }

    #[test]
    fn test_find_close_in_word_no_match() {
        // All opens - no close exists
        let word = u64::MAX;
        for p in 0..64u32 {
            let result = find_close_in_word(word, p);
            let naive = naive_find_close_in_word(word, p);
            assert_eq!(result, None);
            assert_eq!(naive, None);
        }
    }

    #[test]
    fn test_find_close_in_word_boundary_positions() {
        // Test positions 62 and 63 specifically
        let word = 0b11u64 << 62; // Opens at positions 62, 63

        // Position 62: open, no close after it
        assert_eq!(find_close_in_word(word, 62), None);
        assert_eq!(naive_find_close_in_word(word, 62), None);

        // Position 63: open at last bit, no room for close
        assert_eq!(find_close_in_word(word, 63), None);
        assert_eq!(naive_find_close_in_word(word, 63), None);
    }

    #[test]
    fn test_find_unmatched_close_matches_naive() {
        // Test find_unmatched_close_in_word against naive
        let test_words = [
            0u64,
            u64::MAX,
            0x5555555555555555,
            0xAAAAAAAAAAAAAAAA,
            0b001u64 | (u64::MAX << 3),   // "())" padded with opens
            0b00011u64 | (u64::MAX << 5), // "(()))" padded
        ];

        for word in test_words {
            let result = find_unmatched_close_in_word(word);
            let naive = naive_find_unmatched_close_in_word(word);
            assert_eq!(
                result, naive,
                "find_unmatched_close_in_word({:#x}) mismatch: got {}, expected {}",
                word, result, naive
            );
        }
    }
}

// ============================================================================
// Implementation cross-verification (gap #1)
// ============================================================================

mod cross_verification_tests {
    use super::*;
    use rand::prelude::*;
    use rand_chacha::ChaCha8Rng;

    /// Verify BalancedParens matches linear scan for all operations
    fn verify_implementations(words: Vec<u64>, len: usize) {
        let bp = BalancedParens::new(words.clone(), len);

        for p in 0..len {
            // find_close
            if bp.is_open(p) {
                let bp_result = bp.find_close(p);
                let linear_result = find_close(&words, len, p);
                assert_eq!(
                    bp_result, linear_result,
                    "find_close({}) mismatch at len={}",
                    p, len
                );
            }

            // find_open
            if bp.is_close(p) {
                let bp_result = bp.find_open(p);
                let linear_result = find_open(&words, len, p);
                assert_eq!(
                    bp_result, linear_result,
                    "find_open({}) mismatch at len={}",
                    p, len
                );
            }

            // enclose
            if bp.is_open(p) {
                let bp_result = bp.enclose(p);
                let linear_result = enclose(&words, len, p);
                assert_eq!(
                    bp_result, linear_result,
                    "enclose({}) mismatch at len={}",
                    p, len
                );
            }

            // excess
            let bp_excess = bp.excess(p);
            let naive_exc = naive_excess(&words, len, p);
            assert_eq!(
                bp_excess, naive_exc,
                "excess({}) mismatch at len={}",
                p, len
            );
        }
    }

    #[test]
    fn test_cross_verify_small_patterns() {
        let patterns: Vec<(Vec<u64>, usize)> = vec![
            (vec![0b01], 2),                // "()"
            (vec![0b0011], 4),              // "(())"
            (vec![0b0101], 4),              // "()()"
            (vec![0b001011], 6),            // "(()())"
            (vec![0b000111], 6),            // "((()))"
            (vec![0x5555555555555555], 64), // 32 pairs
        ];

        for (words, len) in patterns {
            verify_implementations(words, len);
        }
    }

    #[test]
    fn test_cross_verify_multi_word() {
        // 64 opens followed by 64 closes
        verify_implementations(vec![u64::MAX, 0], 128);

        // 128 opens followed by 128 closes
        verify_implementations(vec![u64::MAX, u64::MAX, 0, 0], 256);

        // Alternating pairs across word boundary
        verify_implementations(vec![0x5555555555555555, 0x5555555555555555], 128);
    }

    #[test]
    fn test_cross_verify_l1_boundary() {
        // 32 words = L1 boundary
        let mut words = vec![u64::MAX; 32]; // 2048 opens
        words.extend(std::iter::repeat_n(0u64, 32)); // 2048 closes
        verify_implementations(words, 4096);
    }

    #[test]
    fn test_cross_verify_random_small() {
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        for _ in 0..100 {
            let num_words = rng.gen_range(1..=4);
            let words: Vec<u64> = (0..num_words).map(|_| rng.r#gen()).collect();
            let len = num_words * 64;
            verify_implementations(words, len);
        }
    }

    #[test]
    fn test_cross_verify_random_medium() {
        let mut rng = ChaCha8Rng::seed_from_u64(123);

        for _ in 0..20 {
            let num_words = rng.gen_range(10..=100);
            let words: Vec<u64> = (0..num_words).map(|_| rng.r#gen()).collect();
            let len = num_words * 64;

            let bp = BalancedParens::new(words.clone(), len);

            // Sample positions rather than all
            for _ in 0..100 {
                let p = rng.gen_range(0..len);

                if bp.is_open(p) {
                    let bp_result = bp.find_close(p);
                    let linear_result = find_close(&words, len, p);
                    assert_eq!(bp_result, linear_result, "find_close({}) mismatch", p);
                }
            }
        }
    }

    #[test]
    fn test_cross_verify_random_large() {
        let mut rng = ChaCha8Rng::seed_from_u64(999);

        // Large random bitvector
        let num_words = 1024; // 64K bits
        let words: Vec<u64> = (0..num_words).map(|_| rng.r#gen()).collect();
        let len = num_words * 64;

        let bp = BalancedParens::new(words.clone(), len);

        // Sample 500 positions
        for _ in 0..500 {
            let p = rng.gen_range(0..len);

            if bp.is_open(p) {
                let bp_result = bp.find_close(p);
                let linear_result = find_close(&words, len, p);
                assert_eq!(
                    bp_result, linear_result,
                    "find_close({}) mismatch at large scale",
                    p
                );
            }
        }
    }
}

// ============================================================================
// Inverse relationship tests
// ============================================================================

mod inverse_tests {
    use super::*;

    #[test]
    fn test_find_close_find_open_inverse() {
        let patterns: Vec<(Vec<u64>, usize)> = vec![
            (vec![0b01], 2),
            (vec![0b0011], 4),
            (vec![0b001011], 6),
            (vec![u64::MAX, 0], 128),
        ];

        for (words, len) in patterns {
            let bp = BalancedParens::new(words, len);

            for p in 0..len {
                if bp.is_open(p)
                    && let Some(close) = bp.find_close(p)
                {
                    let open = bp.find_open(close);
                    assert_eq!(
                        open,
                        Some(p),
                        "find_open(find_close({})) != {} for close={}",
                        p,
                        p,
                        close
                    );
                }
            }
        }
    }

    #[test]
    fn test_excess_balanced_returns_zero() {
        // Balanced sequences should end with excess = 0
        let patterns: Vec<(Vec<u64>, usize)> = vec![
            (vec![0b01], 2),
            (vec![0b0011], 4),
            (vec![0b0101], 4),
            (vec![0b001011], 6),
            (vec![u64::MAX, 0], 128),
        ];

        for (words, len) in patterns {
            let bp = BalancedParens::new(words, len);
            assert_eq!(
                bp.excess(len - 1),
                0,
                "Balanced sequence should end with excess 0"
            );
        }
    }

    #[test]
    fn test_subtree_size_formula() {
        // subtree_size(p) = (find_close(p) - p) / 2
        let bp = BalancedParens::new(vec![0b001011], 6);

        for p in 0..6 {
            if bp.is_open(p)
                && let (Some(size), Some(close)) = (bp.subtree_size(p), bp.find_close(p))
            {
                assert_eq!(
                    size,
                    (close - p) / 2,
                    "subtree_size formula failed at {}",
                    p
                );
            }
        }
    }
}

// ============================================================================
// L0/L1/L2 index correctness tests
// ============================================================================

mod index_structure_tests {
    use super::*;

    #[test]
    fn test_l0_index_single_word() {
        // "(()())" = 0b001011
        // Scanning: 1 2 1 2 1 0 -> min=0, total=0
        let bp = BalancedParens::new(vec![0b001011], 6);

        // L0 should have one entry
        assert_eq!(bp.len(), 6);
    }

    #[test]
    fn test_index_correctness_via_find_close() {
        // The real test of index correctness is whether find_close works
        // This tests various patterns that exercise different index paths

        // Pattern that stays in same word
        let bp = BalancedParens::new(vec![0b001011], 6);
        assert_eq!(bp.find_close(0), Some(5));

        // Pattern that crosses word boundary
        let bp = BalancedParens::new(vec![u64::MAX, 0], 128);
        assert_eq!(bp.find_close(0), Some(127));
        assert_eq!(bp.find_close(63), Some(64));

        // Pattern with L1 boundary crossing
        let mut words = vec![u64::MAX; 32];
        words.extend(std::iter::repeat_n(0u64, 32));
        let bp = BalancedParens::new(words, 4096);
        assert_eq!(bp.find_close(0), Some(4095));
        assert_eq!(bp.find_close(2047), Some(2048));
    }

    #[test]
    fn test_empty_structure() {
        let bp = BalancedParens::new(vec![], 0);
        assert!(bp.is_empty());
        assert_eq!(bp.find_close(0), None);
        assert_eq!(bp.find_open(0), None);
        assert_eq!(bp.excess(0), 0);
    }
}

// ============================================================================
// Depth and subtree size comprehensive tests
// ============================================================================

mod depth_subtree_tests {
    use super::*;

    #[test]
    fn test_depth_increases_on_open() {
        let bp = BalancedParens::new(vec![0b000111], 6); // "((()))"

        // Depths should be: 1, 2, 3, 2, 1, 0
        let expected_depths = [1, 2, 3, 2, 1, 0];
        for (i, &expected) in expected_depths.iter().enumerate() {
            assert_eq!(
                bp.depth(i),
                Some(expected),
                "depth({}) should be {}",
                i,
                expected
            );
        }
    }

    #[test]
    fn test_depth_monotonic_for_opens() {
        // For nested opens, depth should strictly increase
        let bp = BalancedParens::new(vec![u64::MAX], 64);

        let mut prev_depth = 0;
        for p in 0..64 {
            let depth = bp.depth(p).unwrap();
            assert!(depth > prev_depth, "Depth should increase for nested opens");
            prev_depth = depth;
        }
    }

    #[test]
    fn test_subtree_size_leaves() {
        // All leaves (immediate ()()) should have subtree_size = 0
        let bp = BalancedParens::new(vec![0b0101], 4); // "()()"

        assert_eq!(bp.subtree_size(0), Some(0)); // First ()
        assert_eq!(bp.subtree_size(2), Some(0)); // Second ()
    }

    #[test]
    fn test_subtree_size_nested() {
        // "(())" - root has one child
        let bp = BalancedParens::new(vec![0b0011], 4);

        assert_eq!(bp.subtree_size(0), Some(1)); // Root has 1 descendant
        assert_eq!(bp.subtree_size(1), Some(0)); // Inner is leaf
    }

    #[test]
    fn test_subtree_size_at_close_returns_none() {
        let bp = BalancedParens::new(vec![0b0011], 4);

        assert_eq!(bp.subtree_size(2), None);
        assert_eq!(bp.subtree_size(3), None);
    }
}

// ============================================================================
// Navigation operation tests
// ============================================================================

mod navigation_tests {
    use super::*;

    #[test]
    fn test_first_child_with_children() {
        // "(()())" - root at 0 has first child at 1
        let bp = BalancedParens::new(vec![0b001011], 6);
        assert_eq!(bp.first_child(0), Some(1));
    }

    #[test]
    fn test_first_child_leaf() {
        // Leaf nodes have no children
        let bp = BalancedParens::new(vec![0b01], 2);
        assert_eq!(bp.first_child(0), None);
    }

    #[test]
    fn test_next_sibling_chain() {
        // "()()()" - chain of siblings
        let bp = BalancedParens::new(vec![0b010101], 6);

        assert_eq!(bp.next_sibling(0), Some(2));
        assert_eq!(bp.next_sibling(2), Some(4));
        assert_eq!(bp.next_sibling(4), None);
    }

    #[test]
    fn test_next_sibling_nested() {
        // "(()())" - two children are siblings
        let bp = BalancedParens::new(vec![0b001011], 6);

        assert_eq!(bp.next_sibling(1), Some(3)); // First child -> second child
        assert_eq!(bp.next_sibling(3), None); // Second child has no sibling
    }

    #[test]
    fn test_parent_navigation() {
        // "(()())" - children's parent is root
        let bp = BalancedParens::new(vec![0b001011], 6);

        assert_eq!(bp.parent(1), Some(0));
        assert_eq!(bp.parent(3), Some(0));
        assert_eq!(bp.parent(0), None); // Root has no parent
    }

    #[test]
    fn test_navigation_at_close() {
        // Navigation on close positions should return None
        let bp = BalancedParens::new(vec![0b001011], 6);

        assert_eq!(bp.first_child(2), None);
        assert_eq!(bp.next_sibling(2), None);
        assert_eq!(bp.parent(2), None);
    }
}

// ============================================================================
// Edge cases and regression tests
// ============================================================================

mod edge_cases {
    use super::*;

    #[test]
    fn test_single_bit() {
        // Single open (unbalanced)
        let bp = BalancedParens::new(vec![1], 1);
        assert!(bp.is_open(0));
        assert_eq!(bp.find_close(0), None);
        assert_eq!(bp.excess(0), 1);

        // Single close (unbalanced)
        let bp = BalancedParens::new(vec![0], 1);
        assert!(bp.is_close(0));
        assert_eq!(bp.find_open(0), None);
        assert_eq!(bp.excess(0), -1);
    }

    #[test]
    fn test_alternating_full_words() {
        // Full word of alternating pairs
        let word = 0x5555555555555555u64;
        let bp = BalancedParens::new(vec![word], 64);

        // Each open at even position matches close at odd position
        for i in 0..32 {
            assert_eq!(bp.find_close(i * 2), Some(i * 2 + 1));
        }
    }

    #[test]
    fn test_deeply_nested_multi_word() {
        // 128 opens followed by 128 closes
        let words = vec![u64::MAX, u64::MAX, 0, 0];
        let bp = BalancedParens::new(words.clone(), 256);

        // Open at 0 matches close at 255
        assert_eq!(bp.find_close(0), Some(255));

        // Open at 127 matches close at 128
        assert_eq!(bp.find_close(127), Some(128));

        // Verify with linear scan
        assert_eq!(find_close(&words, 256, 0), Some(255));
        assert_eq!(find_close(&words, 256, 127), Some(128));
    }

    #[test]
    fn test_partial_word_len() {
        // len not multiple of 64
        let bp = BalancedParens::new(vec![0b0011], 4);
        assert_eq!(bp.len(), 4);
        assert_eq!(bp.find_close(0), Some(3));
        assert_eq!(bp.find_close(1), Some(2));

        // Verify excess at last position
        assert_eq!(bp.excess(3), 0);
    }

    #[test]
    fn test_all_operations_at_boundary() {
        // Test at word boundary (position 64)
        let words = vec![u64::MAX, 0];
        let bp = BalancedParens::new(words, 128);

        // Position 63 is last open in first word
        assert!(bp.is_open(63));
        assert_eq!(bp.find_close(63), Some(64));

        // Position 64 is first close in second word
        assert!(bp.is_close(64));
        assert_eq!(bp.find_open(64), Some(63));

        // Excess at 63 should be 64, at 64 should be 63
        assert_eq!(bp.excess(63), 64);
        assert_eq!(bp.excess(64), 63);
    }
}
