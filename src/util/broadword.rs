//! Broadword (SWAR) algorithms for bit manipulation.
//!
//! These algorithms operate on 64-bit words using SIMD-within-a-register techniques,
//! providing efficient bit operations without requiring hardware SIMD support.

use crate::util::table::select_in_byte;

/// Constant with 1 in each byte's LSB position.
#[allow(dead_code)]
pub const L8: u64 = 0x0101_0101_0101_0101;

/// Constant with 1 in each byte's MSB position.
#[allow(dead_code)]
pub const H8: u64 = 0x8080_8080_8080_8080;

/// Select the k-th set bit (0-indexed) in a 64-bit word.
///
/// Returns the bit position (0-63), or 64 if there are fewer than k+1 set bits.
///
/// On ARM64 with SVE2-BITPERM support (Graviton 4, Azure Cobalt 100), this uses
/// the BDEP instruction for O(1) performance. Otherwise falls back to the CTZ loop.
///
/// # Performance (Graviton 4)
///
/// | Method | k=0 | k=31 | k=63 |
/// |--------|-----|------|------|
/// | CTZ loop | 0.9 ns | 10 ns | 31 ns |
/// | BDEP | 1.8 ns | 1.8 ns | 1.8 ns |
///
/// BDEP is 5-17x faster for k > 0.
#[inline]
pub fn select_in_word(x: u64, k: u32) -> u32 {
    #[cfg(all(target_arch = "aarch64", feature = "std"))]
    {
        use core::sync::atomic::{AtomicU8, Ordering};

        // Cache the feature detection result
        // 0 = unknown, 1 = BDEP available, 2 = BDEP not available
        static HAS_BDEP: AtomicU8 = AtomicU8::new(0);

        let has_bdep = match HAS_BDEP.load(Ordering::Relaxed) {
            1 => true,
            2 => false,
            _ => {
                let detected = std::arch::is_aarch64_feature_detected!("sve2-bitperm");
                HAS_BDEP.store(if detected { 1 } else { 2 }, Ordering::Relaxed);
                detected
            }
        };

        if has_bdep {
            // SAFETY: We just verified sve2-bitperm is available
            return unsafe { crate::util::simd::sve2::select_in_word_bdep(x, k) };
        }
    }

    select_in_word_ctz(x, k)
}

/// Select using CTZ (count trailing zeros) loop.
///
/// This is the fallback implementation when BDEP is not available.
/// It's O(k) but very efficient for small k values.
#[inline]
fn select_in_word_ctz(x: u64, k: u32) -> u32 {
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
/// # Performance (Apple M1 Max)
///
/// Broadword is faster than CTZ loop for k≥4:
/// - k=0-1: CTZ wins (0.7-1.0 ns vs 1.5-1.8 ns)
/// - k=4+: Broadword wins, gap widens with k
/// - k=31: 12.4 ns (CTZ) vs 3.6 ns (Broadword) = 3.4× faster
/// - k=63: 28.1 ns (CTZ) vs 4.6 ns (Broadword) = 6.1× faster
///
/// For Elias-Fano `select1` where k averages ~32, broadword is ~3× faster.
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
