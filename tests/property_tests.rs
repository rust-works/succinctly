//! Property-based tests for rank/select operations.
//!
//! These tests use proptest to verify invariants hold for arbitrary inputs.

use proptest::prelude::*;
use succinctly::{BitVec, RankSelect};

// ============================================================================
// Property-based tests
// ============================================================================

proptest! {
    /// rank1(0) is always 0
    #[test]
    fn rank1_at_zero_is_zero(words in prop::collection::vec(any::<u64>(), 0..50)) {
        let len = words.len() * 64;
        let bv = BitVec::from_words(words, len);
        prop_assert_eq!(bv.rank1(0), 0);
    }

    /// rank1(len) equals count_ones
    #[test]
    fn rank1_at_len_equals_count_ones(words in prop::collection::vec(any::<u64>(), 1..50)) {
        let len = words.len() * 64;
        let bv = BitVec::from_words(words, len);
        prop_assert_eq!(bv.rank1(bv.len()), bv.count_ones());
    }

    /// rank0(i) + rank1(i) == i for all valid i
    #[test]
    fn rank0_plus_rank1_equals_i(
        words in prop::collection::vec(any::<u64>(), 1..20),
        i_frac in 0.0..=1.0f64
    ) {
        let len = words.len() * 64;
        let bv = BitVec::from_words(words, len);
        let i = (i_frac * len as f64) as usize;
        prop_assert_eq!(bv.rank0(i) + bv.rank1(i), i);
    }

    /// rank1 is monotonically non-decreasing
    #[test]
    fn rank1_is_monotonic(
        words in prop::collection::vec(any::<u64>(), 1..20),
        i_frac in 0.0..1.0f64,
        j_frac in 0.0..1.0f64
    ) {
        let len = words.len() * 64;
        let bv = BitVec::from_words(words, len);
        let i = (i_frac * len as f64) as usize;
        let j = (j_frac * len as f64) as usize;
        if i <= j {
            prop_assert!(bv.rank1(i) <= bv.rank1(j));
        }
    }

    /// Galois connection: rank1(select1(k) + 1) == k + 1
    #[test]
    fn rank_select_galois_connection(words in prop::collection::vec(1u64..=u64::MAX, 1..30)) {
        let len = words.len() * 64;
        let bv = BitVec::from_words(words, len);
        let ones = bv.count_ones();

        for k in 0..ones.min(100) {
            if let Some(pos) = bv.select1(k) {
                // The bit at pos should be a 1
                prop_assert!(bv.get(pos), "select1({}) = {} but bit is 0", k, pos);
                // rank1(pos + 1) should equal k + 1
                prop_assert_eq!(bv.rank1(pos + 1), k + 1,
                    "rank1(select1({}) + 1) != {} + 1", k, k);
            }
        }
    }

    /// select1(rank1(i) - 1) <= i for positions with a 1-bit before them
    #[test]
    fn select_rank_inequality(
        words in prop::collection::vec(any::<u64>(), 1..20),
        i_frac in 0.01..1.0f64
    ) {
        let len = words.len() * 64;
        let bv = BitVec::from_words(words, len);
        let i = (i_frac * len as f64).max(1.0) as usize;

        let rank = bv.rank1(i);
        if rank > 0 && let Some(sel) = bv.select1(rank - 1) {
            prop_assert!(sel < i, "select1({}) = {} >= i = {}", rank - 1, sel, i);
        }
    }

    /// select1 returns positions in strictly increasing order
    #[test]
    fn select1_is_strictly_increasing(words in prop::collection::vec(1u64..=u64::MAX, 1..20)) {
        let len = words.len() * 64;
        let bv = BitVec::from_words(words, len);
        let ones = bv.count_ones();

        let mut prev = None;
        for k in 0..ones.min(100) {
            if let Some(pos) = bv.select1(k) {
                if let Some(p) = prev {
                    prop_assert!(pos > p, "select1({}) = {} <= select1({}) = {}", k, pos, k - 1, p);
                }
                prev = Some(pos);
            }
        }
    }

    /// select1(k) returns None iff k >= count_ones
    #[test]
    fn select1_none_iff_k_ge_ones(words in prop::collection::vec(any::<u64>(), 1..20)) {
        let len = words.len() * 64;
        let bv = BitVec::from_words(words, len);
        let ones = bv.count_ones();

        // Should succeed for k < ones
        for k in 0..ones.min(10) {
            prop_assert!(bv.select1(k).is_some(), "select1({}) should be Some", k);
        }

        // Should fail for k >= ones
        prop_assert!(bv.select1(ones).is_none(), "select1({}) should be None", ones);
        prop_assert!(bv.select1(ones + 1).is_none(), "select1({}) should be None", ones + 1);
    }

    /// Partial word masking works correctly
    #[test]
    fn partial_word_masking(
        word in any::<u64>(),
        valid_bits in 1usize..=64
    ) {
        let bv = BitVec::from_words(vec![word], valid_bits);
        let mask = if valid_bits == 64 { u64::MAX } else { (1u64 << valid_bits) - 1 };
        let expected_ones = (word & mask).count_ones() as usize;
        prop_assert_eq!(bv.count_ones(), expected_ones);
        prop_assert_eq!(bv.rank1(valid_bits), expected_ones);
    }
}

// ============================================================================
// Cross-implementation equivalence tests
// ============================================================================

/// Naive rank implementation for comparison
fn naive_rank1(words: &[u64], len: usize, i: usize) -> usize {
    let i = i.min(len);
    let mut count = 0;
    for bit_pos in 0..i {
        let word_idx = bit_pos / 64;
        let bit_idx = bit_pos % 64;
        if (words[word_idx] >> bit_idx) & 1 == 1 {
            count += 1;
        }
    }
    count
}

/// Naive select implementation for comparison
fn naive_select1(words: &[u64], len: usize, k: usize) -> Option<usize> {
    let mut count = 0;
    for bit_pos in 0..len {
        let word_idx = bit_pos / 64;
        let bit_idx = bit_pos % 64;
        if (words[word_idx] >> bit_idx) & 1 == 1 {
            if count == k {
                return Some(bit_pos);
            }
            count += 1;
        }
    }
    None
}

proptest! {
    /// BitVec rank1 matches naive implementation
    #[test]
    fn rank1_matches_naive(
        words in prop::collection::vec(any::<u64>(), 1..30),
        i_frac in 0.0..=1.0f64
    ) {
        let len = words.len() * 64;
        let bv = BitVec::from_words(words.clone(), len);
        let i = (i_frac * len as f64) as usize;

        let expected = naive_rank1(&words, len, i);
        let actual = bv.rank1(i);
        prop_assert_eq!(actual, expected, "rank1({}) mismatch", i);
    }

    /// BitVec select1 matches naive implementation
    #[test]
    fn select1_matches_naive(words in prop::collection::vec(any::<u64>(), 1..20)) {
        let len = words.len() * 64;
        let bv = BitVec::from_words(words.clone(), len);
        let ones = bv.count_ones();

        for k in 0..ones.min(50) {
            let expected = naive_select1(&words, len, k);
            let actual = bv.select1(k);
            prop_assert_eq!(actual, expected, "select1({}) mismatch", k);
        }
    }
}

// ============================================================================
// Large bitvector tests
// ============================================================================

#[test]
fn test_10k_bits_alternating() {
    // 10,240 bits with alternating pattern
    let words: Vec<u64> = (0..160).map(|_| 0xAAAA_AAAA_AAAA_AAAA).collect();
    let bv = BitVec::from_words(words, 10240);

    assert_eq!(bv.count_ones(), 5120);
    assert_eq!(bv.count_zeros(), 5120);

    // Test rank at various positions
    assert_eq!(bv.rank1(0), 0);
    assert_eq!(bv.rank1(64), 32);
    assert_eq!(bv.rank1(512), 256);
    assert_eq!(bv.rank1(1024), 512);
    assert_eq!(bv.rank1(5120), 2560);
    assert_eq!(bv.rank1(10240), 5120);

    // Test select
    assert_eq!(bv.select1(0), Some(1));
    assert_eq!(bv.select1(31), Some(63));
    assert_eq!(bv.select1(32), Some(65));
    assert_eq!(bv.select1(255), Some(511));
    assert_eq!(bv.select1(5119), Some(10239));
    assert_eq!(bv.select1(5120), None);
}

#[test]
fn test_10k_bits_sparse() {
    // 10,240 bits with only first bit of each word set
    let words: Vec<u64> = (0..160).map(|_| 1u64).collect();
    let bv = BitVec::from_words(words, 10240);

    assert_eq!(bv.count_ones(), 160);
    assert_eq!(bv.count_zeros(), 10080);

    // Test rank
    assert_eq!(bv.rank1(0), 0);
    assert_eq!(bv.rank1(1), 1);
    assert_eq!(bv.rank1(64), 1);
    assert_eq!(bv.rank1(65), 2);
    assert_eq!(bv.rank1(10240), 160);

    // Test select
    assert_eq!(bv.select1(0), Some(0));
    assert_eq!(bv.select1(1), Some(64));
    assert_eq!(bv.select1(2), Some(128));
    assert_eq!(bv.select1(159), Some(159 * 64));
    assert_eq!(bv.select1(160), None);
}

#[test]
fn test_10k_bits_dense() {
    // 10,240 bits all set to 1
    let words: Vec<u64> = (0..160).map(|_| u64::MAX).collect();
    let bv = BitVec::from_words(words, 10240);

    assert_eq!(bv.count_ones(), 10240);
    assert_eq!(bv.count_zeros(), 0);

    // Test rank
    for i in 0..=10240 {
        assert_eq!(bv.rank1(i), i);
    }

    // Test select
    for k in 0..100 {
        assert_eq!(bv.select1(k), Some(k));
    }
    assert_eq!(bv.select1(10239), Some(10239));
    assert_eq!(bv.select1(10240), None);
}

#[test]
fn test_100k_bits_random_pattern() {
    use rand::prelude::*;
    use rand_chacha::ChaCha8Rng;

    // Use a fixed seed for reproducibility
    let mut rng = ChaCha8Rng::seed_from_u64(42);

    // 102,400 bits
    let words: Vec<u64> = (0..1600).map(|_| rng.r#gen()).collect();
    let bv = BitVec::from_words(words.clone(), 102400);

    // Verify count_ones matches manual count
    let expected_ones: usize = words.iter().map(|w| w.count_ones() as usize).sum();
    assert_eq!(bv.count_ones(), expected_ones);

    // Sample some rank queries
    for i in (0..102400).step_by(1000) {
        let expected = naive_rank1(&words, 102400, i);
        assert_eq!(bv.rank1(i), expected, "rank1({}) mismatch", i);
    }

    // Sample some select queries
    let ones = bv.count_ones();
    for k in (0..ones).step_by(1000) {
        let expected = naive_select1(&words, 102400, k);
        assert_eq!(bv.select1(k), expected, "select1({}) mismatch", k);
    }
}

// ============================================================================
// Multi-block boundary tests
// ============================================================================

#[test]
fn test_block_boundary_512_bits() {
    // 512 bits = 8 words = exactly 1 rank block
    let words: Vec<u64> = vec![u64::MAX; 8];
    let bv = BitVec::from_words(words, 512);

    assert_eq!(bv.count_ones(), 512);

    // Test at block boundary
    assert_eq!(bv.rank1(511), 511);
    assert_eq!(bv.rank1(512), 512);
}

#[test]
fn test_block_boundary_crossing() {
    // 1024 bits = 16 words = 2 rank blocks
    let mut words = vec![0u64; 16];
    // Put ones at the boundary: last bit of block 0, first bit of block 1
    words[7] = 1u64 << 63; // bit 511
    words[8] = 1u64; // bit 512

    let bv = BitVec::from_words(words, 1024);

    assert_eq!(bv.count_ones(), 2);
    assert_eq!(bv.rank1(511), 0);
    assert_eq!(bv.rank1(512), 1);
    assert_eq!(bv.rank1(513), 2);

    assert_eq!(bv.select1(0), Some(511));
    assert_eq!(bv.select1(1), Some(512));
}

#[test]
fn test_many_blocks() {
    // 8192 bits = 128 words = 16 rank blocks
    let words: Vec<u64> = (0..128).map(|i| 1u64 << (i % 64)).collect();
    let bv = BitVec::from_words(words, 8192);

    assert_eq!(bv.count_ones(), 128);

    // Verify rank at each block boundary
    for block in 0..16 {
        let pos = block * 512;
        let expected = block * 8; // 8 ones per block
        assert_eq!(bv.rank1(pos), expected, "rank1({}) at block {}", pos, block);
    }
}

// ============================================================================
// Word boundary tests
// ============================================================================

#[test]
fn test_word_boundaries_rank() {
    // Test rank queries right at word boundaries
    let words: Vec<u64> = vec![0xFF; 16]; // 8 ones per word

    let bv = BitVec::from_words(words, 1024);

    for word_idx in 0..16 {
        let pos = word_idx * 64;
        let expected = word_idx * 8;
        assert_eq!(bv.rank1(pos), expected, "rank1({}) at word boundary", pos);
    }
}

#[test]
fn test_word_boundaries_select() {
    // Ones at positions 63, 127, 191, ... (last bit of each word)
    let words: Vec<u64> = vec![1u64 << 63; 16];
    let bv = BitVec::from_words(words, 1024);

    for k in 0..16 {
        let expected = k * 64 + 63;
        assert_eq!(
            bv.select1(k),
            Some(expected),
            "select1({}) at word boundary",
            k
        );
    }
}

// ============================================================================
// Edge cases
// ============================================================================

#[test]
fn test_single_bit_set() {
    let bv = BitVec::from_words(vec![1], 64);
    assert_eq!(bv.count_ones(), 1);
    assert_eq!(bv.rank1(0), 0);
    assert_eq!(bv.rank1(1), 1);
    assert_eq!(bv.rank1(64), 1);
    assert_eq!(bv.select1(0), Some(0));
    assert_eq!(bv.select1(1), None);
}

#[test]
fn test_last_bit_set() {
    let bv = BitVec::from_words(vec![1u64 << 63], 64);
    assert_eq!(bv.count_ones(), 1);
    assert_eq!(bv.rank1(63), 0);
    assert_eq!(bv.rank1(64), 1);
    assert_eq!(bv.select1(0), Some(63));
}

#[test]
fn test_partial_last_word() {
    // 100 bits with all ones
    let words = vec![u64::MAX, u64::MAX];
    let bv = BitVec::from_words(words, 100);

    assert_eq!(bv.count_ones(), 100);
    assert_eq!(bv.rank1(100), 100);
    assert_eq!(bv.select1(99), Some(99));
    assert_eq!(bv.select1(100), None);
}

// ============================================================================
// Balanced Parentheses Property Tests
// ============================================================================

use succinctly::bp::{enclose, find_close, find_open, BalancedParens};

/// Generate a valid balanced parentheses sequence.
///
/// This generator builds balanced sequences by randomly choosing to:
/// - Add an open paren (if we haven't reached the target length)
/// - Add a close paren (if there are unmatched opens)
///
/// Based on the Haskell hw-balancedparens generator.
fn balanced_parens_strategy(max_pairs: usize) -> impl Strategy<Value = (Vec<u64>, usize)> {
    (1..=max_pairs).prop_flat_map(|num_pairs| {
        let len = num_pairs * 2;
        // Generate a sequence of decisions: true = open, false = close
        // We need exactly num_pairs opens and num_pairs closes
        prop::collection::vec(any::<bool>(), len).prop_map(move |decisions| {
            let mut bits = Vec::new();
            let mut open_count = 0usize;
            let mut close_count = 0usize;

            for decision in decisions {
                let need_opens = num_pairs - open_count;
                let need_closes = num_pairs - close_count;
                let unmatched_opens = open_count - close_count;

                if need_opens == 0 {
                    // Must close
                    bits.push(false);
                    close_count += 1;
                } else if unmatched_opens == 0 {
                    // Must open (can't close with no unmatched opens)
                    bits.push(true);
                    open_count += 1;
                } else if need_closes == 0 {
                    // Shouldn't happen, but just open
                    bits.push(true);
                    open_count += 1;
                } else if decision {
                    // Choose to open
                    bits.push(true);
                    open_count += 1;
                } else {
                    // Choose to close
                    bits.push(false);
                    close_count += 1;
                }
            }

            // Convert bits to words
            let len = bits.len();
            let num_words = len.div_ceil(64);
            let mut words = vec![0u64; num_words];

            for (i, &bit) in bits.iter().enumerate() {
                if bit {
                    let word_idx = i / 64;
                    let bit_idx = i % 64;
                    words[word_idx] |= 1u64 << bit_idx;
                }
            }

            (words, len)
        })
    })
}

/// Naive linear scan implementation of find_close for comparison
fn naive_find_close(words: &[u64], len: usize, p: usize) -> Option<usize> {
    if p >= len {
        return None;
    }

    let word_idx = p / 64;
    let bit_idx = p % 64;

    // Check if position p is an open paren (bit = 1)
    if (words[word_idx] >> bit_idx) & 1 == 0 {
        return None; // Not an open paren
    }

    // Linear scan to find matching close
    let mut excess = 1i32; // Start after the open at p
    for pos in (p + 1)..len {
        let w_idx = pos / 64;
        let b_idx = pos % 64;
        if (words[w_idx] >> b_idx) & 1 == 1 {
            excess += 1;
        } else {
            excess -= 1;
            if excess == 0 {
                return Some(pos);
            }
        }
    }
    None
}

/// Naive linear scan implementation of find_open for comparison
fn naive_find_open(words: &[u64], len: usize, p: usize) -> Option<usize> {
    if p >= len || p == 0 {
        return None;
    }

    let word_idx = p / 64;
    let bit_idx = p % 64;

    // Check if position p is a close paren (bit = 0)
    if (words[word_idx] >> bit_idx) & 1 == 1 {
        return None; // Not a close paren
    }

    // Linear scan backwards to find matching open
    let mut excess = 1i32; // Start before the close at p
    for pos in (0..p).rev() {
        let w_idx = pos / 64;
        let b_idx = pos % 64;
        if (words[w_idx] >> b_idx) & 1 == 0 {
            excess += 1;
        } else {
            excess -= 1;
            if excess == 0 {
                return Some(pos);
            }
        }
    }
    None
}

proptest! {
    /// BalancedParens.find_close matches linear scan for all positions
    #[test]
    fn bp_find_close_matches_linear(
        (words, len) in balanced_parens_strategy(64)
    ) {
        let bp = BalancedParens::new(words.clone(), len);

        for p in 0..len {
            let bp_result = bp.find_close(p);
            let linear_result = find_close(&words, len, p);
            let naive_result = naive_find_close(&words, len, p);

            prop_assert_eq!(bp_result, linear_result,
                "BalancedParens.find_close({}) != find_close() at len={}", p, len);
            prop_assert_eq!(bp_result, naive_result,
                "BalancedParens.find_close({}) != naive at len={}", p, len);
        }
    }

    /// BalancedParens.find_open matches linear scan for all positions
    #[test]
    fn bp_find_open_matches_linear(
        (words, len) in balanced_parens_strategy(64)
    ) {
        let bp = BalancedParens::new(words.clone(), len);

        for p in 0..len {
            let bp_result = bp.find_open(p);
            let linear_result = find_open(&words, len, p);
            let naive_result = naive_find_open(&words, len, p);

            prop_assert_eq!(bp_result, linear_result,
                "BalancedParens.find_open({}) != find_open() at len={}", p, len);
            prop_assert_eq!(bp_result, naive_result,
                "BalancedParens.find_open({}) != naive at len={}", p, len);
        }
    }

    /// find_close and find_open are inverses
    #[test]
    fn bp_find_close_open_inverse(
        (words, len) in balanced_parens_strategy(64)
    ) {
        let bp = BalancedParens::new(words, len);

        for p in 0..len {
            if bp.is_open(p) && let Some(close) = bp.find_close(p) {
                let open = bp.find_open(close);
                prop_assert_eq!(open, Some(p),
                    "find_open(find_close({})) != {} at len={}", p, p, len);
            }
        }
    }

    /// next_sibling returns valid open positions
    #[test]
    fn bp_next_sibling_valid(
        (words, len) in balanced_parens_strategy(64)
    ) {
        let bp = BalancedParens::new(words, len);

        for p in 0..len {
            if let Some(sibling) = bp.next_sibling(p) {
                prop_assert!(bp.is_open(sibling),
                    "next_sibling({}) = {} should be an open at len={}", p, sibling, len);
                prop_assert!(sibling > p,
                    "next_sibling({}) = {} should be > p at len={}", p, sibling, len);
            }
        }
    }

    /// first_child returns valid open positions
    #[test]
    fn bp_first_child_valid(
        (words, len) in balanced_parens_strategy(64)
    ) {
        let bp = BalancedParens::new(words, len);

        for p in 0..len {
            if let Some(child) = bp.first_child(p) {
                prop_assert!(bp.is_open(child),
                    "first_child({}) = {} should be an open at len={}", p, child, len);
                prop_assert_eq!(child, p + 1,
                    "first_child({}) should be p+1 at len={}", p, len);
            }
        }
    }

    /// parent/enclose returns valid open positions
    #[test]
    fn bp_parent_valid(
        (words, len) in balanced_parens_strategy(64)
    ) {
        let bp = BalancedParens::new(words.clone(), len);

        for p in 0..len {
            if bp.is_open(p) {
                if let Some(parent) = bp.parent(p) {
                    prop_assert!(bp.is_open(parent),
                        "parent({}) = {} should be an open at len={}", p, parent, len);
                    prop_assert!(parent < p,
                        "parent({}) = {} should be < p at len={}", p, parent, len);

                    // Verify parent encloses p
                    let parent_close = bp.find_close(parent);
                    let p_close = bp.find_close(p);
                    prop_assert!(parent_close > p_close,
                        "parent({}) should have close after p's close at len={}", p, len);
                }

                // Verify parent matches enclose
                let enclose_result = enclose(&words, len, p);
                prop_assert_eq!(bp.parent(p), enclose_result,
                    "parent({}) != enclose() at len={}", p, len);
            }
        }
    }

    /// depth is always positive for valid positions
    #[test]
    fn bp_depth_positive(
        (words, len) in balanced_parens_strategy(64)
    ) {
        let bp = BalancedParens::new(words, len);

        for p in 0..len {
            if let Some(d) = bp.depth(p) {
                // Depth at open should be >= 1, at close can be 0
                if bp.is_open(p) {
                    prop_assert!(d >= 1, "depth({}) = {} should be >= 1 for open at len={}", p, d, len);
                }
            }
        }
    }

    /// subtree_size is consistent with find_close
    #[test]
    fn bp_subtree_size_consistent(
        (words, len) in balanced_parens_strategy(64)
    ) {
        let bp = BalancedParens::new(words, len);

        for p in 0..len {
            if bp.is_open(p) {
                if let (Some(size), Some(close)) = (bp.subtree_size(p), bp.find_close(p)) {
                    // subtree_size = (close - p) / 2
                    let expected = (close - p) / 2;
                    prop_assert_eq!(size, expected,
                        "subtree_size({}) = {} != (close - p) / 2 = {} at len={}", p, size, expected, len);
                }
            } else {
                prop_assert_eq!(bp.subtree_size(p), None,
                    "subtree_size({}) should be None for close at len={}", p, len);
            }
        }
    }
}

// ============================================================================
// Larger scale balanced parentheses tests (matching Haskell's factor = 16384)
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10))]

    /// Test with larger vectors (up to 1024 words = 65536 bits)
    #[test]
    fn bp_large_vectors(
        (words, len) in balanced_parens_strategy(512)
    ) {
        let bp = BalancedParens::new(words.clone(), len);

        // Test a sample of positions
        let step = (len / 100).max(1);
        for p in (0..len).step_by(step) {
            if bp.is_open(p) {
                let bp_result = bp.find_close(p);
                let linear_result = find_close(&words, len, p);
                prop_assert_eq!(bp_result, linear_result,
                    "find_close mismatch at p={} len={}", p, len);
            }
        }
    }

    /// BalancedParens matches random word vectors (like Haskell RangeMinSpec)
    #[test]
    fn bp_random_words_find_close(
        words in prop::collection::vec(any::<u64>(), 1..64),
        p_frac in 0.0..1.0f64
    ) {
        let len = words.len() * 64;
        let bp = BalancedParens::new(words.clone(), len);
        let p = (p_frac * len as f64) as usize;

        if p < len {
            // Even for non-balanced sequences, the implementations should match
            let bp_result = bp.find_close(p);
            let linear_result = find_close(&words, len, p);
            prop_assert_eq!(bp_result, linear_result,
                "find_close({}) mismatch for random words at len={}", p, len);
        }
    }

    /// next_sibling matches between BalancedParens and SimpleBalancedParens
    #[test]
    fn bp_next_sibling_matches(
        words in prop::collection::vec(any::<u64>(), 1..64),
        p_frac in 0.0..1.0f64
    ) {
        let len = words.len() * 64;
        let bp = BalancedParens::new(words.clone(), len);
        let p = (p_frac * len as f64) as usize;

        if p < len {
            // Compute next_sibling using linear scan
            let naive_next_sibling = if bp.is_open(p) {
                find_close(&words, len, p).and_then(|close| {
                    if close + 1 < len {
                        let w_idx = (close + 1) / 64;
                        let b_idx = (close + 1) % 64;
                        if (words[w_idx] >> b_idx) & 1 == 1 {
                            Some(close + 1)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
            } else {
                None
            };

            prop_assert_eq!(bp.next_sibling(p), naive_next_sibling,
                "next_sibling({}) mismatch at len={}", p, len);
        }
    }
}

// ============================================================================
// Very large scale tests (matching Haskell's factor = 16384)
// These test up to 16K words = 1M+ bits to match hw-balancedparens coverage
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(5))]

    /// Test with very large random word vectors (16K words = 1M bits)
    /// Matches Haskell RangeMinSpec's factor = 16384
    #[test]
    fn bp_very_large_random_words(
        words in prop::collection::vec(any::<u64>(), 1..16384),
        p_frac in 0.0..1.0f64
    ) {
        let len = words.len() * 64;
        let bp = BalancedParens::new(words.clone(), len);
        let p = (p_frac * len as f64) as usize;

        if p < len {
            let bp_result = bp.find_close(p);
            let linear_result = find_close(&words, len, p);
            prop_assert_eq!(bp_result, linear_result,
                "find_close({}) mismatch for random words at len={}", p, len);
        }
    }

    /// next_sibling at large scale
    #[test]
    fn bp_very_large_next_sibling(
        words in prop::collection::vec(any::<u64>(), 1..16384),
        p_frac in 0.0..1.0f64
    ) {
        let len = words.len() * 64;
        let bp = BalancedParens::new(words.clone(), len);
        let p = (p_frac * len as f64) as usize;

        if p < len {
            let naive_next_sibling = if bp.is_open(p) {
                find_close(&words, len, p).and_then(|close| {
                    if close + 1 < len {
                        let w_idx = (close + 1) / 64;
                        let b_idx = (close + 1) % 64;
                        if (words[w_idx] >> b_idx) & 1 == 1 {
                            Some(close + 1)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
            } else {
                None
            };

            prop_assert_eq!(bp.next_sibling(p), naive_next_sibling,
                "next_sibling({}) mismatch at len={}", p, len);
        }
    }
}

// ============================================================================
// L1/L2 boundary crossing tests at scale
// L1 blocks = 32 words = 2048 bits
// L2 blocks = 1024 words = 65536 bits
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(20))]

    /// Test find_close that spans multiple L1 blocks (> 32 words)
    #[test]
    fn bp_spanning_l1_blocks(
        words in prop::collection::vec(any::<u64>(), 64..256)
    ) {
        let len = words.len() * 64;
        let bp = BalancedParens::new(words.clone(), len);

        // Test positions near L1 boundaries (every 2048 bits)
        for l1_block in 0..(len / 2048) {
            let boundary = l1_block * 2048;
            // Test positions around boundary
            for offset in [0, 1, 31, 32, 33, 63, 64, 65].iter() {
                let p = boundary.saturating_add(*offset);
                if p < len && bp.is_open(p) {
                    let bp_result = bp.find_close(p);
                    let linear_result = find_close(&words, len, p);
                    prop_assert_eq!(bp_result, linear_result,
                        "find_close({}) mismatch near L1 boundary {} at len={}", p, boundary, len);
                }
            }
        }
    }

    /// Test find_close that spans L2 blocks (> 1024 words)
    #[test]
    fn bp_spanning_l2_blocks(
        words in prop::collection::vec(any::<u64>(), 1024..2048)
    ) {
        let len = words.len() * 64;
        let bp = BalancedParens::new(words.clone(), len);

        // Test positions near L2 boundaries (every 65536 bits)
        for l2_block in 0..(len / 65536).max(1) {
            let boundary = l2_block * 65536;
            // Test positions around boundary
            for offset in [0, 1, 63, 64, 65, 2047, 2048, 2049].iter() {
                let p = boundary.saturating_add(*offset);
                if p < len && bp.is_open(p) {
                    let bp_result = bp.find_close(p);
                    let linear_result = find_close(&words, len, p);
                    prop_assert_eq!(bp_result, linear_result,
                        "find_close({}) mismatch near L2 boundary {} at len={}", p, boundary, len);
                }
            }
        }
    }
}

// ============================================================================
// Specific pattern tests at large scale
// ============================================================================

#[test]
fn test_large_deeply_nested() {
    // Create deeply nested structure: ((((....))))
    // 8K opens followed by 8K closes = 16K bits
    let num_pairs: usize = 8192;
    let len = num_pairs * 2;
    let num_words = len.div_ceil(64);
    let mut words = vec![0u64; num_words];

    // Set first num_pairs bits to 1 (opens)
    for i in 0..num_pairs {
        let word_idx = i / 64;
        let bit_idx = i % 64;
        words[word_idx] |= 1u64 << bit_idx;
    }

    let bp = BalancedParens::new(words.clone(), len);

    // First open should match last close
    assert_eq!(bp.find_close(0), Some(len - 1));

    // Each open at position i should match close at position (len - 1 - i)
    for i in 0..100 {
        let expected_close = len - 1 - i;
        assert_eq!(
            bp.find_close(i),
            Some(expected_close),
            "find_close({}) should be {}",
            i,
            expected_close
        );
    }

    // Test from the middle
    let mid = num_pairs - 1;
    assert_eq!(bp.find_close(mid), Some(mid + 1));
}

#[test]
fn test_large_flat_sequence() {
    // Create flat sequence: ()()()()...
    // Alternating 1,0,1,0,... = 0x5555...
    let num_words = 1024; // 64K bits = 32K pairs
    let len = num_words * 64;
    let words: Vec<u64> = vec![0x5555_5555_5555_5555; num_words];

    let bp = BalancedParens::new(words.clone(), len);

    // Each open at even position should match the next odd position
    for i in (0..1000).step_by(2) {
        assert_eq!(
            bp.find_close(i),
            Some(i + 1),
            "find_close({}) should be {}",
            i,
            i + 1
        );
    }

    // Test at various block boundaries
    for block in [0, 32, 64, 128, 512, 1023] {
        let p = block * 64;
        if p < len {
            assert_eq!(
                bp.find_close(p),
                Some(p + 1),
                "find_close({}) at block {} boundary",
                p,
                block
            );
        }
    }
}

#[test]
fn test_large_mixed_structure() {
    // Create a sequence with varied depth
    // Pattern: ()((()))(())((())) repeated
    use rand::prelude::*;
    use rand_chacha::ChaCha8Rng;

    let mut rng = ChaCha8Rng::seed_from_u64(12345);

    // Generate random balanced parens
    let num_pairs = 16384;
    let mut bits = Vec::with_capacity(num_pairs * 2);
    let mut open_count = 0usize;
    let mut close_count = 0usize;

    while open_count < num_pairs || close_count < num_pairs {
        let can_open = open_count < num_pairs;
        let can_close = close_count < open_count;

        if can_open && can_close {
            if rng.r#gen::<bool>() {
                bits.push(true);
                open_count += 1;
            } else {
                bits.push(false);
                close_count += 1;
            }
        } else if can_open {
            bits.push(true);
            open_count += 1;
        } else {
            bits.push(false);
            close_count += 1;
        }
    }

    // Convert to words
    let len = bits.len();
    let num_words = len.div_ceil(64);
    let mut words = vec![0u64; num_words];
    for (i, &bit) in bits.iter().enumerate() {
        if bit {
            words[i / 64] |= 1u64 << (i % 64);
        }
    }

    let bp = BalancedParens::new(words.clone(), len);

    // Verify find_close matches linear scan at many positions
    for p in (0..len).step_by(100) {
        if bp.is_open(p) {
            let bp_result = bp.find_close(p);
            let linear_result = find_close(&words, len, p);
            assert_eq!(
                bp_result, linear_result,
                "find_close({}) mismatch in mixed structure",
                p
            );
        }
    }

    // Test all L1 boundaries
    for l1_block in 0..(len / 2048) {
        let boundary = l1_block * 2048;
        if boundary < len && bp.is_open(boundary) {
            let bp_result = bp.find_close(boundary);
            let linear_result = find_close(&words, len, boundary);
            assert_eq!(
                bp_result, linear_result,
                "find_close({}) mismatch at L1 boundary",
                boundary
            );
        }
    }
}

#[test]
fn test_million_bit_random() {
    // 1M bits = ~15625 words
    use rand::prelude::*;
    use rand_chacha::ChaCha8Rng;

    let mut rng = ChaCha8Rng::seed_from_u64(99999);
    let num_words = 15625;
    let len = num_words * 64;
    let words: Vec<u64> = (0..num_words).map(|_| rng.r#gen()).collect();

    let bp = BalancedParens::new(words.clone(), len);

    // Sample 1000 random positions
    for _ in 0..1000 {
        let p = rng.gen_range(0..len);
        let bp_result = bp.find_close(p);
        let linear_result = find_close(&words, len, p);
        assert_eq!(
            bp_result, linear_result,
            "find_close({}) mismatch in 1M bit test",
            p
        );
    }
}
