//! Popcount implementations with compile-time switching.
//!
//! This module provides different popcount strategies that can be selected
//! via feature flags for benchmarking:
//!
//! - Default: Uses Rust's `count_ones()` which auto-vectorizes
//! - `simd`: Uses explicit SIMD intrinsics (NEON/POPCNT)
//! - `portable-popcount`: Uses portable bitwise algorithm (no intrinsics)
//!
//! Feature priority (when multiple enabled): portable-popcount > simd > default

/// Popcount a single u64 word.
#[inline(always)]
pub fn popcount_word(word: u64) -> u32 {
    // Priority: portable-popcount > simd > default
    #[cfg(feature = "portable-popcount")]
    {
        popcount_word_portable(word)
    }

    #[cfg(all(feature = "simd", not(feature = "portable-popcount")))]
    {
        // On both x86_64 and aarch64, count_ones compiles to efficient instructions
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
    // Priority: portable-popcount > simd > default
    #[cfg(feature = "portable-popcount")]
    {
        popcount_words_portable(words)
    }

    #[cfg(all(
        feature = "simd",
        target_arch = "aarch64",
        not(feature = "portable-popcount")
    ))]
    {
        popcount_words_neon(words)
    }

    #[cfg(all(
        feature = "simd",
        target_arch = "x86_64",
        not(feature = "portable-popcount")
    ))]
    {
        popcount_words_x86(words)
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
#[cfg(all(
    feature = "simd",
    target_arch = "aarch64",
    not(feature = "portable-popcount")
))]
#[inline]
fn popcount_words_neon(words: &[u64]) -> u32 {
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
#[cfg(all(
    feature = "simd",
    target_arch = "aarch64",
    not(feature = "portable-popcount")
))]
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

/// AVX-512 VPOPCNTDQ: 8x parallel u64 popcount.
///
/// Processes 8 u64 words (512 bits) at once using AVX-512 VPOPCNTDQ instruction.
/// Available on Intel Ice Lake+ (2019) and AMD Zen 4+ (2022).
#[cfg(all(
    feature = "simd",
    target_arch = "x86_64",
    not(feature = "portable-popcount")
))]
#[inline]
#[target_feature(enable = "avx512f,avx512vpopcntdq")]
unsafe fn popcount_words_avx512vpopcntdq(words: &[u64]) -> u32 {
    use core::arch::x86_64::*;

    if words.is_empty() {
        return 0;
    }

    let mut total = 0u32;
    let mut offset = 0;

    // Process 8 u64 words (512 bits) at a time
    while offset + 8 <= words.len() {
        unsafe {
            let ptr = words.as_ptr().add(offset) as *const __m512i;
            let v = _mm512_loadu_si512(ptr);

            // _mm512_popcnt_epi64: Count bits in each of 8 u64 lanes in parallel
            let counts = _mm512_popcnt_epi64(v);

            // Sum all 8 counts into a single value
            total += _mm512_reduce_add_epi64(counts) as u32;
        }
        offset += 8;
    }

    // Handle remaining words (< 8)
    for &word in &words[offset..] {
        total += word.count_ones();
    }

    total
}

/// x86_64 popcount with runtime dispatch to best available implementation.
///
/// Dispatches to:
/// - AVX-512 VPOPCNTDQ (8x u64 parallel) if available
/// - Scalar POPCNT otherwise
#[cfg(all(
    feature = "simd",
    target_arch = "x86_64",
    not(feature = "portable-popcount")
))]
#[inline]
fn popcount_words_x86(words: &[u64]) -> u32 {
    // Runtime dispatch to AVX-512 VPOPCNTDQ if available (requires std)
    #[cfg(feature = "std")]
    {
        if is_x86_feature_detected!("avx512vpopcntdq") {
            return unsafe { popcount_words_avx512vpopcntdq(words) };
        }
    }

    // Fallback: scalar POPCNT (count_ones compiles to POPCNT on x86_64)
    let mut total = 0u32;
    for &word in words {
        total += word.count_ones();
    }
    total
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

    #[cfg(all(
        feature = "simd",
        target_arch = "x86_64",
        not(feature = "portable-popcount")
    ))]
    #[test]
    fn test_avx512_vpopcntdq_matches_scalar() {
        // Test AVX-512 VPOPCNTDQ implementation against scalar
        if !is_x86_feature_detected!("avx512vpopcntdq") {
            eprintln!("Skipping AVX-512 VPOPCNTDQ test: CPU doesn't support it");
            return;
        }

        // Test various sizes
        for len in [0, 1, 7, 8, 9, 15, 16, 17, 64, 100, 1000] {
            let words: Vec<u64> = (0..len)
                .map(|i: u64| {
                    // Mix of patterns
                    match i % 4 {
                        0 => u64::MAX,
                        1 => 0,
                        2 => 0xAAAA_AAAA_AAAA_AAAA,
                        _ => i.wrapping_mul(0x0123_4567_89AB_CDEF),
                    }
                })
                .collect();

            let expected: u32 = words.iter().map(|w: &u64| w.count_ones()).sum();
            let avx512_result = unsafe { popcount_words_avx512vpopcntdq(&words) };

            assert_eq!(
                avx512_result, expected,
                "AVX-512 VPOPCNTDQ mismatch for {} words",
                len
            );
        }
    }

    #[cfg(all(
        feature = "simd",
        target_arch = "x86_64",
        not(feature = "portable-popcount")
    ))]
    #[test]
    fn test_avx512_edge_cases() {
        if !is_x86_feature_detected!("avx512vpopcntdq") {
            return;
        }

        // All zeros
        let zeros = vec![0u64; 100];
        assert_eq!(unsafe { popcount_words_avx512vpopcntdq(&zeros) }, 0);

        // All ones
        let ones = vec![u64::MAX; 100];
        assert_eq!(unsafe { popcount_words_avx512vpopcntdq(&ones) }, 100 * 64);

        // Alternating pattern
        let alt = vec![0xAAAA_AAAA_AAAA_AAAA; 100];
        assert_eq!(unsafe { popcount_words_avx512vpopcntdq(&alt) }, 100 * 32);
    }
}
