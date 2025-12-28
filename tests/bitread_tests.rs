//! Tests using human-readable bit pattern strings.
//!
//! These tests mirror the haskell-works style of using bit strings like "10010010"
//! for more readable test cases.

use succinctly::{BitVec, RankSelect};

// ============================================================================
// Bit pattern parsing utilities
// ============================================================================

/// Parse a bit string like "1001 0010" into a bitvector.
///
/// Spaces are ignored. Bits are read left-to-right as positions 0, 1, 2, ...
/// (i.e., the leftmost bit is position 0).
fn bitread(s: &str) -> BitVec {
    let bits: Vec<bool> = s
        .chars()
        .filter(|c| *c == '0' || *c == '1')
        .map(|c| c == '1')
        .collect();

    if bits.is_empty() {
        return BitVec::new();
    }

    let num_words = bits.len().div_ceil(64);
    let mut words = vec![0u64; num_words];

    for (i, &bit) in bits.iter().enumerate() {
        if bit {
            let word_idx = i / 64;
            let bit_idx = i % 64;
            words[word_idx] |= 1u64 << bit_idx;
        }
    }

    BitVec::from_words(words, bits.len())
}

// ============================================================================
// Rank tests with readable bit patterns
// ============================================================================

#[test]
fn test_rank1_pattern_10010010() {
    // "10010010" - ones at positions 0, 3, 6
    let bv = bitread("10010010");

    assert_eq!(bv.len(), 8);
    assert_eq!(bv.count_ones(), 3);

    // rank1 over [0..8] should be 0,1,1,1,2,2,2,3,3
    let expected = [0, 1, 1, 1, 2, 2, 2, 3, 3];
    for (i, &exp) in expected.iter().enumerate() {
        assert_eq!(bv.rank1(i), exp, "rank1({}) mismatch", i);
    }
}

#[test]
fn test_rank1_pattern_11011010_00000000() {
    // "11011010 00000000" - ones at positions 0, 1, 3, 4, 6
    let bv = bitread("11011010 00000000");

    assert_eq!(bv.len(), 16);
    assert_eq!(bv.count_ones(), 5);

    // rank1 over [0..16]
    let expected = [0, 1, 2, 2, 3, 4, 4, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5];
    for (i, &exp) in expected.iter().enumerate() {
        assert_eq!(bv.rank1(i), exp, "rank1({}) mismatch", i);
    }
}

#[test]
fn test_rank1_pattern_11011010_10000000() {
    // "11011010 10000000" - ones at positions 0, 1, 3, 4, 6, 8
    let bv = bitread("11011010 10000000");

    assert_eq!(bv.len(), 16);
    assert_eq!(bv.count_ones(), 6);

    // rank1 over [0..16]
    let expected = [0, 1, 2, 2, 3, 4, 4, 5, 5, 6, 6, 6, 6, 6, 6, 6, 6];
    for (i, &exp) in expected.iter().enumerate() {
        assert_eq!(bv.rank1(i), exp, "rank1({}) mismatch", i);
    }
}

// ============================================================================
// Select tests with readable bit patterns
// ============================================================================

#[test]
fn test_select1_pattern_10010010() {
    // "10010010" - ones at positions 0, 3, 6
    let bv = bitread("10010010");

    // select1 over [0..3] should be 0, 3, 6
    assert_eq!(bv.select1(0), Some(0));
    assert_eq!(bv.select1(1), Some(3));
    assert_eq!(bv.select1(2), Some(6));
    assert_eq!(bv.select1(3), None);
}

#[test]
fn test_select1_pattern_11011010() {
    // "11011010" - ones at positions 0, 1, 3, 4, 6
    let bv = bitread("11011010");

    assert_eq!(bv.select1(0), Some(0));
    assert_eq!(bv.select1(1), Some(1));
    assert_eq!(bv.select1(2), Some(3));
    assert_eq!(bv.select1(3), Some(4));
    assert_eq!(bv.select1(4), Some(6));
    assert_eq!(bv.select1(5), None);
}

#[test]
fn test_select1_pattern_01000000_00000100() {
    // "01000000 00000100" - ones at positions 1, 13
    let bv = bitread("01000000 00000100");

    assert_eq!(bv.select1(0), Some(1));
    assert_eq!(bv.select1(1), Some(13));
    assert_eq!(bv.select1(2), None);
}

#[test]
fn test_select1_pattern_across_bytes() {
    // "11000001 10000000 01000000"
    // Positions: 01234567 89...15 16...23
    // Ones at: 0, 1, 7, 8, 17
    let bv = bitread("11000001 10000000 01000000");

    assert_eq!(bv.select1(0), Some(0));
    assert_eq!(bv.select1(1), Some(1));
    assert_eq!(bv.select1(2), Some(7));
    assert_eq!(bv.select1(3), Some(8));
    assert_eq!(bv.select1(4), Some(17));
}

#[test]
fn test_select1_pattern_32bits() {
    // "10000010 00000000 00100000 00010000"
    // ones at positions 0, 6, 18, 27
    let bv = bitread("10000010 00000000 00100000 00010000");

    assert_eq!(bv.select1(0), Some(0));
    assert_eq!(bv.select1(1), Some(6));
    assert_eq!(bv.select1(2), Some(18));
    assert_eq!(bv.select1(3), Some(27));
    assert_eq!(bv.select1(4), None);
}

// ============================================================================
// Select0 tests with readable bit patterns
// ============================================================================

#[test]
fn test_select0_pattern_10010010() {
    // "10010010" - zeros at positions 1, 2, 4, 5, 7
    let bv = bitread("10010010");

    assert_eq!(bv.select0(0), Some(1));
    assert_eq!(bv.select0(1), Some(2));
    assert_eq!(bv.select0(2), Some(4));
    assert_eq!(bv.select0(3), Some(5));
    assert_eq!(bv.select0(4), Some(7));
    assert_eq!(bv.select0(5), None);
}

#[test]
fn test_select0_all_ones() {
    let bv = bitread("11111111");
    assert_eq!(bv.count_zeros(), 0);
    assert_eq!(bv.select0(0), None);
}

#[test]
fn test_select0_all_zeros() {
    let bv = bitread("00000000");
    assert_eq!(bv.count_ones(), 0);
    for i in 0..8 {
        assert_eq!(bv.select0(i), Some(i));
    }
    assert_eq!(bv.select0(8), None);
}

// ============================================================================
// Rank/Select roundtrip tests
// ============================================================================

#[test]
fn test_roundtrip_various_patterns() {
    let patterns = [
        "10010010",
        "11111111",
        "00000000",
        "10101010",
        "01010101",
        "11110000",
        "00001111",
        "11011010 00000000",
        "11011010 10000000",
        "01000000 00000100",
        "10000010 00000000 00100000 00010000",
    ];

    for pattern in patterns {
        let bv = bitread(pattern);

        // For every set bit, verify roundtrip
        for k in 0..bv.count_ones() {
            let pos = bv.select1(k).unwrap();
            assert!(
                bv.get(pos),
                "pattern '{}': select1({}) = {} but bit is not set",
                pattern,
                k,
                pos
            );
            assert_eq!(
                bv.rank1(pos + 1),
                k + 1,
                "pattern '{}': rank1(select1({}) + 1) != {} + 1",
                pattern,
                k,
                k
            );
        }

        // For every zero bit, verify roundtrip
        for k in 0..bv.count_zeros() {
            let pos = bv.select0(k).unwrap();
            assert!(
                !bv.get(pos),
                "pattern '{}': select0({}) = {} but bit is set",
                pattern,
                k,
                pos
            );
            assert_eq!(
                bv.rank0(pos + 1),
                k + 1,
                "pattern '{}': rank0(select0({}) + 1) != {} + 1",
                pattern,
                k,
                k
            );
        }
    }
}

// ============================================================================
// Long pattern tests (mimicking haskell-works corpus tests)
// ============================================================================

#[test]
fn test_long_alternating_pattern() {
    // 4096 bits of alternating "10" pattern
    let pattern: String = "10".repeat(2048);
    let bv = bitread(&pattern);

    assert_eq!(bv.len(), 4096);
    assert_eq!(bv.count_ones(), 2048);

    // rank1 at position x should be (x-1)/2 + 1 for x > 0
    for x in 1..=4096 {
        let expected = (x - 1) / 2 + 1;
        assert_eq!(bv.rank1(x), expected, "rank1({}) mismatch", x);
    }
}

#[test]
fn test_long_sparse_pattern() {
    // One bit set every 64 positions
    let mut pattern = String::new();
    for _ in 0..64 {
        pattern.push('1');
        pattern.push_str(&"0".repeat(63));
    }

    let bv = bitread(&pattern);

    assert_eq!(bv.len(), 4096);
    assert_eq!(bv.count_ones(), 64);

    // select1(k) should be k * 64
    for k in 0..64 {
        assert_eq!(bv.select1(k), Some(k * 64), "select1({}) mismatch", k);
    }
}

// ============================================================================
// Edge cases from haskell-works test suite
// ============================================================================

#[test]
fn test_empty_bitvector() {
    let bv = bitread("");
    assert_eq!(bv.len(), 0);
    assert_eq!(bv.count_ones(), 0);
    assert_eq!(bv.rank1(0), 0);
    assert_eq!(bv.select1(0), None);
}

#[test]
fn test_single_one() {
    let bv = bitread("1");
    assert_eq!(bv.len(), 1);
    assert_eq!(bv.count_ones(), 1);
    assert_eq!(bv.rank1(0), 0);
    assert_eq!(bv.rank1(1), 1);
    assert_eq!(bv.select1(0), Some(0));
    assert_eq!(bv.select1(1), None);
}

#[test]
fn test_single_zero() {
    let bv = bitread("0");
    assert_eq!(bv.len(), 1);
    assert_eq!(bv.count_ones(), 0);
    assert_eq!(bv.rank1(0), 0);
    assert_eq!(bv.rank1(1), 0);
    assert_eq!(bv.select1(0), None);
    assert_eq!(bv.select0(0), Some(0));
}
