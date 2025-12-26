//! x86/x86_64 SIMD implementations.
//!
//! These implementations use POPCNT and other x86 instructions for
//! accelerated bit operations.

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

/// Popcount of 64 bytes (512 bits) using POPCNT instruction.
///
/// # Safety
///
/// - `ptr` must be valid for reading 8 x u64 = 64 bytes
/// - `ptr` should be 8-byte aligned
/// - CPU must support POPCNT instruction (caller should check)
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "popcnt")]
#[inline]
pub unsafe fn popcount_512_popcnt(ptr: *const u64) -> u32 {
    unsafe {
        let mut sum = 0i64;
        sum += _popcnt64(*ptr);
        sum += _popcnt64(*ptr.add(1));
        sum += _popcnt64(*ptr.add(2));
        sum += _popcnt64(*ptr.add(3));
        sum += _popcnt64(*ptr.add(4));
        sum += _popcnt64(*ptr.add(5));
        sum += _popcnt64(*ptr.add(6));
        sum += _popcnt64(*ptr.add(7));
        sum as u32
    }
}

/// Popcount of arbitrary-length word data using POPCNT.
///
/// # Safety
///
/// - `ptr` must be valid for reading `word_count` x u64 bytes
/// - CPU must support POPCNT instruction
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "popcnt")]
#[inline]
pub unsafe fn popcount_words_popcnt(ptr: *const u64, word_count: usize) -> u32 {
    unsafe {
        let mut total = 0i64;
        for i in 0..word_count {
            total += _popcnt64(*ptr.add(i));
        }
        total as u32
    }
}

#[cfg(all(test, target_arch = "x86_64"))]
mod tests {
    use super::*;

    #[test]
    fn test_popcount_512_popcnt() {
        if !is_x86_feature_detected!("popcnt") {
            return;
        }

        let data = [0xFFu64; 8];
        let result = unsafe { popcount_512_popcnt(data.as_ptr()) };
        assert_eq!(result, 512);

        let data = [0u64; 8];
        let result = unsafe { popcount_512_popcnt(data.as_ptr()) };
        assert_eq!(result, 0);

        let data = [0xAAAA_AAAA_AAAA_AAAAu64; 8];
        let result = unsafe { popcount_512_popcnt(data.as_ptr()) };
        assert_eq!(result, 256);
    }

    #[test]
    fn test_popcount_words_popcnt() {
        if !is_x86_feature_detected!("popcnt") {
            return;
        }

        let data = [0xFFu64; 16];
        unsafe {
            assert_eq!(popcount_words_popcnt(data.as_ptr(), 0), 0);
            assert_eq!(popcount_words_popcnt(data.as_ptr(), 1), 64);
            assert_eq!(popcount_words_popcnt(data.as_ptr(), 8), 512);
            assert_eq!(popcount_words_popcnt(data.as_ptr(), 16), 1024);
        }
    }
}
