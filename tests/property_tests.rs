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

    /// select0 returns positions in strictly increasing order
    #[test]
    fn select0_is_strictly_increasing(words in prop::collection::vec(0u64..u64::MAX, 1..20)) {
        let len = words.len() * 64;
        let bv = BitVec::from_words(words, len);
        let zeros = bv.count_zeros();

        let mut prev = None;
        for k in 0..zeros.min(100) {
            if let Some(pos) = bv.select0(k) {
                if let Some(p) = prev {
                    prop_assert!(pos > p, "select0({}) = {} <= select0({}) = {}", k, pos, k - 1, p);
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

#[test]
fn test_select0_comprehensive() {
    // Bits: 11110000 pattern repeated
    let word = 0x0F0F_0F0F_0F0F_0F0Fu64;
    let bv = BitVec::from_words(vec![word; 4], 256);

    // 32 ones and 32 zeros per word
    assert_eq!(bv.count_ones(), 128);
    assert_eq!(bv.count_zeros(), 128);

    // First zero is at position 4
    assert_eq!(bv.select0(0), Some(4));
    // Second zero is at position 5
    assert_eq!(bv.select0(1), Some(5));

    // Verify select0 roundtrip
    for k in 0..128 {
        if let Some(pos) = bv.select0(k) {
            assert!(!bv.get(pos), "select0({}) = {} should be a 0-bit", k, pos);
            assert_eq!(bv.rank0(pos + 1), k + 1);
        }
    }
}
