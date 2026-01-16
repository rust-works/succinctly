//! x86_64 SIMD-accelerated string scanning for YAML parsing.
//!
//! Uses SSE2 (baseline, 16 bytes) with optional AVX2 (32 bytes) when available.

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

/// Find the next double-quote or backslash using x86 SIMD.
///
/// Returns offset from `start` to the found character, or `None` if not found.
#[inline]
pub fn find_quote_or_escape_x86(input: &[u8], start: usize, end: usize) -> Option<usize> {
    // Runtime dispatch to best available implementation
    #[cfg(any(test, feature = "std"))]
    {
        if is_x86_feature_detected!("avx2") {
            // SAFETY: We just checked for AVX2 support
            return unsafe { find_quote_or_escape_avx2(input, start, end) };
        }
    }

    // SAFETY: SSE2 is guaranteed on x86_64
    unsafe { find_quote_or_escape_sse2(input, start, end) }
}

/// Find the next single-quote using x86 SIMD.
///
/// Returns offset from `start` to the found character, or `None` if not found.
#[inline]
pub fn find_single_quote_x86(input: &[u8], start: usize, end: usize) -> Option<usize> {
    // Runtime dispatch to best available implementation
    #[cfg(any(test, feature = "std"))]
    {
        if is_x86_feature_detected!("avx2") {
            // SAFETY: We just checked for AVX2 support
            return unsafe { find_single_quote_avx2(input, start, end) };
        }
    }

    // SAFETY: SSE2 is guaranteed on x86_64
    unsafe { find_single_quote_sse2(input, start, end) }
}

// ============================================================================
// SSE2 implementations (baseline, 16 bytes at a time)
// ============================================================================

#[target_feature(enable = "sse2")]
unsafe fn find_quote_or_escape_sse2(input: &[u8], start: usize, end: usize) -> Option<usize> {
    let len = end - start;
    let data = &input[start..end];
    let mut offset = 0;

    let quote_vec = _mm_set1_epi8(b'"' as i8);
    let backslash_vec = _mm_set1_epi8(b'\\' as i8);

    while offset + 16 <= len {
        let chunk = _mm_loadu_si128(data.as_ptr().add(offset) as *const __m128i);

        // Compare against both targets
        let quotes = _mm_cmpeq_epi8(chunk, quote_vec);
        let backslashes = _mm_cmpeq_epi8(chunk, backslash_vec);

        // OR the results
        let matches = _mm_or_si128(quotes, backslashes);

        // Extract bitmask (one bit per byte)
        let mask = _mm_movemask_epi8(matches) as u32;

        if mask != 0 {
            return Some(offset + mask.trailing_zeros() as usize);
        }

        offset += 16;
    }

    // Handle remaining bytes
    for i in offset..len {
        let b = data[i];
        if b == b'"' || b == b'\\' {
            return Some(i);
        }
    }

    None
}

#[target_feature(enable = "sse2")]
unsafe fn find_single_quote_sse2(input: &[u8], start: usize, end: usize) -> Option<usize> {
    let len = end - start;
    let data = &input[start..end];
    let mut offset = 0;

    let quote_vec = _mm_set1_epi8(b'\'' as i8);

    while offset + 16 <= len {
        let chunk = _mm_loadu_si128(data.as_ptr().add(offset) as *const __m128i);

        // Compare against single quote
        let matches = _mm_cmpeq_epi8(chunk, quote_vec);

        // Extract bitmask
        let mask = _mm_movemask_epi8(matches) as u32;

        if mask != 0 {
            return Some(offset + mask.trailing_zeros() as usize);
        }

        offset += 16;
    }

    // Handle remaining bytes
    for i in offset..len {
        if data[i] == b'\'' {
            return Some(i);
        }
    }

    None
}

// ============================================================================
// AVX2 implementations (32 bytes at a time)
// ============================================================================

#[cfg(any(test, feature = "std"))]
#[target_feature(enable = "avx2")]
unsafe fn find_quote_or_escape_avx2(input: &[u8], start: usize, end: usize) -> Option<usize> {
    let len = end - start;
    let data = &input[start..end];
    let mut offset = 0;

    let quote_vec = _mm256_set1_epi8(b'"' as i8);
    let backslash_vec = _mm256_set1_epi8(b'\\' as i8);

    while offset + 32 <= len {
        let chunk = _mm256_loadu_si256(data.as_ptr().add(offset) as *const __m256i);

        // Compare against both targets
        let quotes = _mm256_cmpeq_epi8(chunk, quote_vec);
        let backslashes = _mm256_cmpeq_epi8(chunk, backslash_vec);

        // OR the results
        let matches = _mm256_or_si256(quotes, backslashes);

        // Extract bitmask (one bit per byte)
        let mask = _mm256_movemask_epi8(matches) as u32;

        if mask != 0 {
            return Some(offset + mask.trailing_zeros() as usize);
        }

        offset += 32;
    }

    // Handle remaining bytes (16-31 bytes) with SSE2
    if offset + 16 <= len {
        let quote_vec_sse = _mm_set1_epi8(b'"' as i8);
        let backslash_vec_sse = _mm_set1_epi8(b'\\' as i8);

        let chunk = _mm_loadu_si128(data.as_ptr().add(offset) as *const __m128i);
        let quotes = _mm_cmpeq_epi8(chunk, quote_vec_sse);
        let backslashes = _mm_cmpeq_epi8(chunk, backslash_vec_sse);
        let matches = _mm_or_si128(quotes, backslashes);
        let mask = _mm_movemask_epi8(matches) as u32;

        if mask != 0 {
            return Some(offset + mask.trailing_zeros() as usize);
        }
        offset += 16;
    }

    // Handle remaining bytes (< 16)
    for i in offset..len {
        let b = data[i];
        if b == b'"' || b == b'\\' {
            return Some(i);
        }
    }

    None
}

#[cfg(any(test, feature = "std"))]
#[target_feature(enable = "avx2")]
unsafe fn find_single_quote_avx2(input: &[u8], start: usize, end: usize) -> Option<usize> {
    let len = end - start;
    let data = &input[start..end];
    let mut offset = 0;

    let quote_vec = _mm256_set1_epi8(b'\'' as i8);

    while offset + 32 <= len {
        let chunk = _mm256_loadu_si256(data.as_ptr().add(offset) as *const __m256i);

        // Compare against single quote
        let matches = _mm256_cmpeq_epi8(chunk, quote_vec);

        // Extract bitmask
        let mask = _mm256_movemask_epi8(matches) as u32;

        if mask != 0 {
            return Some(offset + mask.trailing_zeros() as usize);
        }

        offset += 32;
    }

    // Handle remaining bytes (16-31 bytes) with SSE2
    if offset + 16 <= len {
        let quote_vec_sse = _mm_set1_epi8(b'\'' as i8);

        let chunk = _mm_loadu_si128(data.as_ptr().add(offset) as *const __m128i);
        let matches = _mm_cmpeq_epi8(chunk, quote_vec_sse);
        let mask = _mm_movemask_epi8(matches) as u32;

        if mask != 0 {
            return Some(offset + mask.trailing_zeros() as usize);
        }
        offset += 16;
    }

    // Handle remaining bytes (< 16)
    for i in offset..len {
        if data[i] == b'\'' {
            return Some(i);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sse2_find_quote_basic() {
        let input = b"hello\"world";
        unsafe {
            assert_eq!(find_quote_or_escape_sse2(input, 0, input.len()), Some(5));
        }
    }

    #[test]
    fn test_sse2_find_backslash() {
        let input = b"hello\\world";
        unsafe {
            assert_eq!(find_quote_or_escape_sse2(input, 0, input.len()), Some(5));
        }
    }

    #[test]
    fn test_sse2_find_single_quote() {
        let input = b"hello'world";
        unsafe {
            assert_eq!(find_single_quote_sse2(input, 0, input.len()), Some(5));
        }
    }

    #[test]
    fn test_sse2_long_string() {
        let mut input = vec![b'a'; 100];
        input[50] = b'"';
        unsafe {
            assert_eq!(find_quote_or_escape_sse2(&input, 0, input.len()), Some(50));
        }
    }

    #[test]
    fn test_dispatched_find_quote() {
        let input = b"hello\"world";
        assert_eq!(find_quote_or_escape_x86(input, 0, input.len()), Some(5));
    }

    #[test]
    fn test_dispatched_find_single_quote() {
        let input = b"hello'world";
        assert_eq!(find_single_quote_x86(input, 0, input.len()), Some(5));
    }
}
