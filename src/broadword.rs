//! Broadword (SWAR) algorithms for bit manipulation.
//!
//! These algorithms operate on 64-bit words using SIMD-within-a-register techniques,
//! providing efficient bit operations without requiring hardware SIMD support.

use crate::table::select_in_byte;

/// Constant with 1 in each byte's LSB position.
pub const L8: u64 = 0x0101_0101_0101_0101;

/// Constant with 1 in each byte's MSB position.
pub const H8: u64 = 0x8080_8080_8080_8080;

/// Select the k-th set bit (0-indexed) in a 64-bit word.
///
/// Returns the bit position (0-63), or 64 if there are fewer than k+1 set bits.
///
/// This implementation uses the CTZ (count trailing zeros) loop approach,
/// which is simple and very efficient on modern CPUs.
#[inline]
pub fn select_in_word(x: u64, k: u32) -> u32 {
    let mut val = x;
    let mut remaining = k;

    loop {
        if val == 0 {
            return 64;
        }
        let t = val.trailing_zeros();
        if remaining == 0 {
            return t;
        }
        remaining -= 1;
        val &= val - 1; // Clear lowest set bit
    }
}

/// Select the k-th set bit using broadword/SWAR algorithm.
///
/// This implementation uses the broadword/SWAR technique from Vigna's paper.
/// It computes byte popcounts and prefix sums in parallel, then uses a lookup
/// table for the final byte.
///
/// Note: The CTZ loop (`select_in_word`) is often faster on modern CPUs due
/// to the efficient `tzcnt` instruction.
#[inline]
#[allow(dead_code)]
pub fn select_in_word_broadword(x: u64, k: u32) -> u32 {
    if x == 0 {
        return 64;
    }

    let pop = x.count_ones();
    if k >= pop {
        return 64;
    }

    // Byte popcounts via SWAR (parallel popcount per byte)
    let byte_counts = {
        let t = x - ((x >> 1) & 0x5555_5555_5555_5555);
        let t = (t & 0x3333_3333_3333_3333) + ((t >> 2) & 0x3333_3333_3333_3333);
        (t + (t >> 4)) & 0x0F0F_0F0F_0F0F_0F0F
    };

    // Find which byte contains the k-th bit by scanning prefix sums
    let mut cumulative = 0u32;
    let mut byte_idx = 0u32;

    for i in 0..8u32 {
        let byte_pop = ((byte_counts >> (i * 8)) & 0xFF) as u32;
        if cumulative + byte_pop > k {
            byte_idx = i;
            break;
        }
        cumulative += byte_pop;
    }

    // Extract the target byte
    let byte_offset = byte_idx * 8;
    let target_byte = ((x >> byte_offset) & 0xFF) as u8;

    // Find position within byte
    let k_in_byte = k - cumulative;

    byte_offset + select_in_byte(target_byte, k_in_byte)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_select_in_word_first_bit() {
        assert_eq!(select_in_word(0b1, 0), 0);
        assert_eq!(select_in_word(0b10, 0), 1);
        assert_eq!(select_in_word(0b100, 0), 2);
        assert_eq!(select_in_word(1 << 63, 0), 63);
    }

    #[test]
    fn test_select_in_word_multiple() {
        let word = 0b1010_1010u64;
        assert_eq!(select_in_word(word, 0), 1);
        assert_eq!(select_in_word(word, 1), 3);
        assert_eq!(select_in_word(word, 2), 5);
        assert_eq!(select_in_word(word, 3), 7);
        assert_eq!(select_in_word(word, 4), 64); // Not found
    }

    #[test]
    fn test_select_in_word_all_ones() {
        let word = u64::MAX;
        for k in 0..64 {
            assert_eq!(select_in_word(word, k), k, "k={}", k);
        }
        assert_eq!(select_in_word(word, 64), 64);
    }

    #[test]
    fn test_select_in_word_zero() {
        assert_eq!(select_in_word(0, 0), 64);
    }

    #[test]
    fn test_select_in_word_high_bits() {
        let word = 1u64 << 63;
        assert_eq!(select_in_word(word, 0), 63);
        assert_eq!(select_in_word(word, 1), 64);
    }

    #[test]
    fn test_select_in_word_cross_byte_boundary() {
        // Bits at positions 7 and 8 (across first byte boundary)
        let word = 0b1_1000_0000u64;
        assert_eq!(select_in_word(word, 0), 7);
        assert_eq!(select_in_word(word, 1), 8);
    }

    #[test]
    fn test_select_in_word_broadword_matches() {
        // Verify broadword algorithm matches CTZ loop
        for &word in &[
            0u64,
            1,
            0xFF,
            0x8000_0000_0000_0000,
            0xFFFF_FFFF_FFFF_FFFF,
            0xAAAA_AAAA_AAAA_AAAA,
            0x1234_5678_9ABC_DEF0,
        ] {
            let pop = word.count_ones();
            for k in 0..=pop {
                assert_eq!(
                    select_in_word(word, k),
                    select_in_word_broadword(word, k),
                    "word={:#x}, k={}",
                    word,
                    k
                );
            }
        }
    }

    #[test]
    fn test_select_in_word_exhaustive_small() {
        // Test all 16-bit patterns
        for word in 0u64..=0xFFFF {
            let pop = word.count_ones();
            for k in 0..pop {
                let pos = select_in_word(word, k);
                assert!(pos < 64, "word={:#x}, k={}", word, k);
                // Verify this is actually the k-th bit
                let bits_before = (word & ((1 << pos) - 1)).count_ones();
                assert_eq!(bits_before, k, "word={:#x}, k={}, pos={}", word, k, pos);
                // Verify bit is set
                assert!((word >> pos) & 1 == 1);
            }
            // Beyond popcount should return 64
            assert_eq!(select_in_word(word, pop), 64);
        }
    }
}
