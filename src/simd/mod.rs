//! SIMD-accelerated operations.
//!
//! This module provides platform-specific SIMD implementations for
//! performance-critical operations like popcount.

#[cfg(target_arch = "aarch64")]
pub mod neon;

#[cfg(target_arch = "x86_64")]
pub mod x86;

/// Popcount of a 512-bit (64-byte) block.
///
/// Uses the best available implementation for the current platform.
#[inline]
pub fn popcount_512(data: &[u8; 64]) -> u32 {
    #[cfg(target_arch = "aarch64")]
    {
        // NEON is always available on aarch64
        unsafe { neon::popcount_512_neon(data.as_ptr()) }
    }

    #[cfg(target_arch = "x86_64")]
    {
        // Use POPCNT instruction if available
        if is_x86_feature_detected!("popcnt") {
            unsafe { x86::popcount_512_popcnt(data.as_ptr() as *const u64) }
        } else {
            popcount_512_scalar(data)
        }
    }

    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        popcount_512_scalar(data)
    }
}

/// Scalar fallback for 512-bit popcount.
#[inline]
pub fn popcount_512_scalar(data: &[u8; 64]) -> u32 {
    // Process byte-by-byte to avoid alignment issues
    let mut total = 0u32;
    for byte in data {
        total += byte.count_ones();
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_popcount_512_all_zeros() {
        let data = [0u8; 64];
        assert_eq!(popcount_512(&data), 0);
    }

    #[test]
    fn test_popcount_512_all_ones() {
        let data = [0xFFu8; 64];
        assert_eq!(popcount_512(&data), 512);
    }

    #[test]
    fn test_popcount_512_pattern() {
        let mut data = [0u8; 64];
        // Set alternating bits: 0b10101010 = 4 bits per byte
        for byte in &mut data {
            *byte = 0xAA;
        }
        assert_eq!(popcount_512(&data), 256);
    }

    #[test]
    fn test_popcount_512_matches_scalar() {
        let test_patterns: &[u8] = &[0x00, 0xFF, 0xAA, 0x55, 0x0F, 0xF0, 0x12, 0x34];
        for &pattern in test_patterns {
            let data = [pattern; 64];
            assert_eq!(
                popcount_512(&data),
                popcount_512_scalar(&data),
                "pattern={:#04x}",
                pattern
            );
        }
    }
}
