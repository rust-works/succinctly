//! Popcount implementations with compile-time switching.
//!
//! This module provides different popcount strategies that can be selected
//! via feature flags for benchmarking:
//!
//! - Default: Uses Rust's `count_ones()` which auto-vectorizes
//! - `simd`: Uses explicit SIMD intrinsics (NEON/POPCNT)
//! - `portable-popcount`: Uses portable bitwise algorithm (no intrinsics)

/// Popcount a single u64 word.
#[inline(always)]
pub fn popcount_word(word: u64) -> u32 {
    #[cfg(feature = "portable-popcount")]
    {
        popcount_word_portable(word)
    }

    #[cfg(all(feature = "simd", target_arch = "x86_64"))]
    {
        // Use POPCNT instruction directly
        #[cfg(target_feature = "popcnt")]
        {
            word.count_ones()
        }
        #[cfg(not(target_feature = "popcnt"))]
        {
            word.count_ones()
        }
    }

    #[cfg(all(feature = "simd", target_arch = "aarch64"))]
    {
        // On aarch64, count_ones compiles to CNT+ADDV
        word.count_ones()
    }

    #[cfg(not(any(feature = "portable-popcount", feature = "simd")))]
    {
        // Default: let Rust/LLVM choose the best implementation
        word.count_ones()
    }
}

/// Popcount multiple words, returning total.
#[inline]
pub fn popcount_words(words: &[u64]) -> u32 {
    #[cfg(all(feature = "simd", target_arch = "aarch64"))]
    {
        popcount_words_neon(words)
    }

    #[cfg(all(feature = "simd", target_arch = "x86_64"))]
    {
        popcount_words_x86(words)
    }

    #[cfg(feature = "portable-popcount")]
    {
        popcount_words_portable(words)
    }

    #[cfg(not(any(feature = "simd", feature = "portable-popcount")))]
    {
        // Default: simple loop, lets LLVM auto-vectorize
        popcount_words_default(words)
    }
}

/// Default implementation using Rust's count_ones.
#[inline]
#[cfg(not(any(feature = "simd", feature = "portable-popcount")))]
fn popcount_words_default(words: &[u64]) -> u32 {
    let mut total = 0u32;
    for &word in words {
        total += word.count_ones();
    }
    total
}

/// Portable bitwise popcount (no intrinsics).
///
/// Uses the classic parallel bit-counting algorithm.
#[inline(always)]
#[cfg(feature = "portable-popcount")]
pub fn popcount_word_portable(mut x: u64) -> u32 {
    // Parallel bit count using magic constants
    const M1: u64 = 0x5555_5555_5555_5555; // 01010101...
    const M2: u64 = 0x3333_3333_3333_3333; // 00110011...
    const M4: u64 = 0x0f0f_0f0f_0f0f_0f0f; // 00001111...
    const H01: u64 = 0x0101_0101_0101_0101; // sum helper

    x = x - ((x >> 1) & M1);
    x = (x & M2) + ((x >> 2) & M2);
    x = (x + (x >> 4)) & M4;
    ((x.wrapping_mul(H01)) >> 56) as u32
}

/// Portable popcount for word slice.
#[inline]
#[cfg(feature = "portable-popcount")]
fn popcount_words_portable(words: &[u64]) -> u32 {
    let mut total = 0u32;
    for &word in words {
        total += popcount_word_portable(word);
    }
    total
}

/// NEON-accelerated popcount for word slices.
#[cfg(all(feature = "simd", target_arch = "aarch64"))]
#[inline]
fn popcount_words_neon(words: &[u64]) -> u32 {
    use core::arch::aarch64::*;

    if words.is_empty() {
        return 0;
    }

    let mut total = 0u32;
    let ptr = words.as_ptr() as *const u8;
    let byte_len = words.len() * 8;
    let mut offset = 0;

    // Process 64-byte chunks with NEON
    while offset + 64 <= byte_len {
        // SAFETY: We verified bounds above
        let count = unsafe { popcount_64bytes_neon(ptr.add(offset)) };
        total += count;
        offset += 64;
    }

    // Handle remaining words
    let remaining_words = (byte_len - offset) / 8;
    for i in 0..remaining_words {
        total += words[offset / 8 + i].count_ones();
    }

    total
}

/// Popcount 64 bytes using NEON.
#[cfg(all(feature = "simd", target_arch = "aarch64"))]
#[inline]
unsafe fn popcount_64bytes_neon(ptr: *const u8) -> u32 {
    use core::arch::aarch64::*;

    unsafe {
        let v0 = vld1q_u8(ptr);
        let v1 = vld1q_u8(ptr.add(16));
        let v2 = vld1q_u8(ptr.add(32));
        let v3 = vld1q_u8(ptr.add(48));

        let c0 = vcntq_u8(v0);
        let c1 = vcntq_u8(v1);
        let c2 = vcntq_u8(v2);
        let c3 = vcntq_u8(v3);

        let sum01 = vaddq_u8(c0, c1);
        let sum23 = vaddq_u8(c2, c3);

        // Widen to u16 to avoid overflow
        let wide01 = vpaddlq_u8(sum01);
        let wide23 = vpaddlq_u8(sum23);
        let wide_sum = vaddq_u16(wide01, wide23);

        vaddvq_u16(wide_sum) as u32
    }
}

/// x86_64 popcount using POPCNT instruction.
#[cfg(all(feature = "simd", target_arch = "x86_64"))]
#[inline]
fn popcount_words_x86(words: &[u64]) -> u32 {
    use core::arch::x86_64::*;

    // Check for POPCNT support at runtime
    if is_x86_feature_detected!("popcnt") {
        let mut total = 0i64;
        for &word in words {
            // SAFETY: We checked for POPCNT support
            total += unsafe { _popcnt64(word as i64) };
        }
        total as u32
    } else {
        // Fallback to Rust's count_ones
        let mut total = 0u32;
        for &word in words {
            total += word.count_ones();
        }
        total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_popcount_word() {
        assert_eq!(popcount_word(0), 0);
        assert_eq!(popcount_word(1), 1);
        assert_eq!(popcount_word(u64::MAX), 64);
        assert_eq!(popcount_word(0xAAAA_AAAA_AAAA_AAAA), 32);
        assert_eq!(popcount_word(0x5555_5555_5555_5555), 32);
    }

    #[test]
    fn test_popcount_words() {
        let empty: &[u64] = &[];
        assert_eq!(popcount_words(empty), 0);

        let ones = [u64::MAX; 8];
        assert_eq!(popcount_words(&ones), 512);

        let pattern = [0xAAAA_AAAA_AAAA_AAAA; 16];
        assert_eq!(popcount_words(&pattern), 512);
    }

    #[test]
    fn test_popcount_words_various_lengths() {
        for len in 0..20 {
            let words: Vec<u64> = (0..len)
                .map(|i| (i as u64) | 0x8000_0000_0000_0001)
                .collect();
            let expected: u32 = words.iter().map(|w| w.count_ones()).sum();
            assert_eq!(popcount_words(&words), expected, "len={}", len);
        }
    }

    #[cfg(feature = "portable-popcount")]
    #[test]
    fn test_portable_matches_builtin() {
        for i in 0u64..1000 {
            let word = i.wrapping_mul(0x1234_5678_9ABC_DEF0_u64).wrapping_add(i);
            assert_eq!(
                popcount_word_portable(word),
                word.count_ones(),
                "word={:#x}",
                word
            );
        }
    }
}
