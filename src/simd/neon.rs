//! ARM NEON SIMD implementations.
//!
//! These implementations use NEON intrinsics for accelerated bit operations
//! on ARM/AArch64 platforms.

#[cfg(target_arch = "aarch64")]
use core::arch::aarch64::*;

/// Popcount of 64 bytes (512 bits) using NEON.
///
/// # Safety
///
/// - `ptr` must be valid for reading 64 bytes
/// - `ptr` should be 16-byte aligned for optimal performance (but works unaligned)
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
pub unsafe fn popcount_512_neon(ptr: *const u8) -> u32 {
    unsafe {
        // Load 4 x 128-bit chunks
        let v0 = vld1q_u8(ptr);
        let v1 = vld1q_u8(ptr.add(16));
        let v2 = vld1q_u8(ptr.add(32));
        let v3 = vld1q_u8(ptr.add(48));

        // Per-byte popcount using CNT instruction
        let c0 = vcntq_u8(v0);
        let c1 = vcntq_u8(v1);
        let c2 = vcntq_u8(v2);
        let c3 = vcntq_u8(v3);

        // Add pairs (max 16 per byte position)
        let sum01 = vaddq_u8(c0, c1);
        let sum23 = vaddq_u8(c2, c3);

        // Widen to u16 before final sum to avoid overflow
        // vpaddlq_u8 adds pairs of adjacent u8 -> u16 (8 elements)
        let wide01 = vpaddlq_u8(sum01);
        let wide23 = vpaddlq_u8(sum23);

        // Add the u16 vectors
        let wide_sum = vaddq_u16(wide01, wide23);

        // Horizontal sum of u16
        vaddvq_u16(wide_sum) as u32
    }
}

/// Popcount of arbitrary-length data using NEON.
///
/// Handles data that isn't a multiple of 64 bytes by using scalar
/// fallback for the tail.
///
/// # Safety
///
/// - `ptr` must be valid for reading `len` bytes
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
pub unsafe fn popcount_neon(ptr: *const u8, len: usize) -> u32 {
    unsafe {
        let mut total = 0u32;
        let mut offset = 0;

        // Process 64-byte chunks
        while offset + 64 <= len {
            total += popcount_512_neon(ptr.add(offset));
            offset += 64;
        }

        // Handle remaining bytes with scalar
        while offset < len {
            total += (*ptr.add(offset)).count_ones();
            offset += 1;
        }

        total
    }
}

#[cfg(all(test, target_arch = "aarch64"))]
mod tests {
    use super::*;

    #[test]
    fn test_popcount_512_neon_all_zeros() {
        let data = [0u8; 64];
        let result = unsafe { popcount_512_neon(data.as_ptr()) };
        assert_eq!(result, 0);
    }

    #[test]
    fn test_popcount_512_neon_all_ones() {
        let data = [0xFFu8; 64];
        let result = unsafe { popcount_512_neon(data.as_ptr()) };
        assert_eq!(result, 512);
    }

    #[test]
    fn test_popcount_512_neon_pattern() {
        // Alternating bits: 4 bits per byte
        let data = [0xAAu8; 64];
        let result = unsafe { popcount_512_neon(data.as_ptr()) };
        assert_eq!(result, 256);
    }

    #[test]
    fn test_popcount_neon_various_lengths() {
        let data = [0xFFu8; 100];

        unsafe {
            assert_eq!(popcount_neon(data.as_ptr(), 0), 0);
            assert_eq!(popcount_neon(data.as_ptr(), 1), 8);
            assert_eq!(popcount_neon(data.as_ptr(), 64), 512);
            assert_eq!(popcount_neon(data.as_ptr(), 65), 520);
            assert_eq!(popcount_neon(data.as_ptr(), 100), 800);
        }
    }
}
