//! x86_64 SIMD-accelerated string scanning for YAML parsing.
//!
//! Uses SSE2 (baseline, 16 bytes) with optional AVX2 (32 bytes) when available.

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

// ============================================================================
// Multi-Character Classification (P0 Optimization)
// ============================================================================

/// Character classification results for a 32-byte chunk (AVX2).
#[derive(Debug, Clone, Copy)]
pub struct YamlCharClass {
    /// Mask of bytes that are '\n'
    pub newlines: u32,
    /// Mask of bytes that are ':'
    pub colons: u32,
    /// Mask of bytes that are '-'
    pub hyphens: u32,
    /// Mask of bytes that are ' ' (space)
    pub spaces: u32,
    /// Mask of bytes that are '"'
    pub quotes_double: u32,
    /// Mask of bytes that are '\''
    pub quotes_single: u32,
    /// Mask of bytes that are '\\'
    pub backslashes: u32,
    /// Mask of bytes that are '#'
    pub hash: u32,
}

/// Classify YAML structural characters in a 32-byte chunk using AVX2.
///
/// This is the main P0 optimization - bulk classification of YAML characters.
/// Falls back to SSE2 (16 bytes) for inputs smaller than 32 bytes.
#[inline]
pub fn classify_yaml_chars(input: &[u8], offset: usize) -> Option<YamlCharClass> {
    // Require at least 16 bytes for SSE2
    if offset + 16 > input.len() {
        return None;
    }

    #[cfg(any(test, feature = "std"))]
    {
        if offset + 32 <= input.len() && is_x86_feature_detected!("avx2") {
            return Some(unsafe { classify_yaml_chars_avx2(input, offset) });
        }
    }

    // Fallback to SSE2 (requires 16 bytes minimum)
    if offset + 16 <= input.len() {
        return Some(unsafe { classify_yaml_chars_sse2(input, offset) });
    }

    None
}

/// AVX2 implementation of character classification (32 bytes at a time).
#[cfg(any(test, feature = "std"))]
#[target_feature(enable = "avx2")]
unsafe fn classify_yaml_chars_avx2(input: &[u8], offset: usize) -> YamlCharClass {
    let chunk = _mm256_loadu_si256(input.as_ptr().add(offset) as *const __m256i);

    // Create comparison vectors for each character
    let v_newline = _mm256_set1_epi8(b'\n' as i8);
    let v_colon = _mm256_set1_epi8(b':' as i8);
    let v_hyphen = _mm256_set1_epi8(b'-' as i8);
    let v_space = _mm256_set1_epi8(b' ' as i8);
    let v_quote_double = _mm256_set1_epi8(b'"' as i8);
    let v_quote_single = _mm256_set1_epi8(b'\'' as i8);
    let v_backslash = _mm256_set1_epi8(b'\\' as i8);
    let v_hash = _mm256_set1_epi8(b'#' as i8);

    // Compare and extract masks
    let eq_newline = _mm256_cmpeq_epi8(chunk, v_newline);
    let eq_colon = _mm256_cmpeq_epi8(chunk, v_colon);
    let eq_hyphen = _mm256_cmpeq_epi8(chunk, v_hyphen);
    let eq_space = _mm256_cmpeq_epi8(chunk, v_space);
    let eq_quote_double = _mm256_cmpeq_epi8(chunk, v_quote_double);
    let eq_quote_single = _mm256_cmpeq_epi8(chunk, v_quote_single);
    let eq_backslash = _mm256_cmpeq_epi8(chunk, v_backslash);
    let eq_hash = _mm256_cmpeq_epi8(chunk, v_hash);

    YamlCharClass {
        newlines: _mm256_movemask_epi8(eq_newline) as u32,
        colons: _mm256_movemask_epi8(eq_colon) as u32,
        hyphens: _mm256_movemask_epi8(eq_hyphen) as u32,
        spaces: _mm256_movemask_epi8(eq_space) as u32,
        quotes_double: _mm256_movemask_epi8(eq_quote_double) as u32,
        quotes_single: _mm256_movemask_epi8(eq_quote_single) as u32,
        backslashes: _mm256_movemask_epi8(eq_backslash) as u32,
        hash: _mm256_movemask_epi8(eq_hash) as u32,
    }
}

/// SSE2 implementation of character classification (16 bytes at a time).
#[target_feature(enable = "sse2")]
unsafe fn classify_yaml_chars_sse2(input: &[u8], offset: usize) -> YamlCharClass {
    let chunk = _mm_loadu_si128(input.as_ptr().add(offset) as *const __m128i);

    // Create comparison vectors for each character
    let v_newline = _mm_set1_epi8(b'\n' as i8);
    let v_colon = _mm_set1_epi8(b':' as i8);
    let v_hyphen = _mm_set1_epi8(b'-' as i8);
    let v_space = _mm_set1_epi8(b' ' as i8);
    let v_quote_double = _mm_set1_epi8(b'"' as i8);
    let v_quote_single = _mm_set1_epi8(b'\'' as i8);
    let v_backslash = _mm_set1_epi8(b'\\' as i8);
    let v_hash = _mm_set1_epi8(b'#' as i8);

    // Compare and extract masks
    let eq_newline = _mm_cmpeq_epi8(chunk, v_newline);
    let eq_colon = _mm_cmpeq_epi8(chunk, v_colon);
    let eq_hyphen = _mm_cmpeq_epi8(chunk, v_hyphen);
    let eq_space = _mm_cmpeq_epi8(chunk, v_space);
    let eq_quote_double = _mm_cmpeq_epi8(chunk, v_quote_double);
    let eq_quote_single = _mm_cmpeq_epi8(chunk, v_quote_single);
    let eq_backslash = _mm_cmpeq_epi8(chunk, v_backslash);
    let eq_hash = _mm_cmpeq_epi8(chunk, v_hash);

    YamlCharClass {
        newlines: _mm_movemask_epi8(eq_newline) as u32,
        colons: _mm_movemask_epi8(eq_colon) as u32,
        hyphens: _mm_movemask_epi8(eq_hyphen) as u32,
        spaces: _mm_movemask_epi8(eq_space) as u32,
        quotes_double: _mm_movemask_epi8(eq_quote_double) as u32,
        quotes_single: _mm_movemask_epi8(eq_quote_single) as u32,
        backslashes: _mm_movemask_epi8(eq_backslash) as u32,
        hash: _mm_movemask_epi8(eq_hash) as u32,
    }
}

/// Find the next newline using SIMD.
///
/// Returns offset from `start` to the newline, or `None` if not found.
#[inline]
pub fn find_newline_x86(input: &[u8], start: usize) -> Option<usize> {
    #[cfg(any(test, feature = "std"))]
    {
        if is_x86_feature_detected!("avx2") {
            return unsafe { find_newline_avx2(input, start) };
        }
    }

    unsafe { find_newline_sse2(input, start) }
}

#[target_feature(enable = "sse2")]
unsafe fn find_newline_sse2(input: &[u8], start: usize) -> Option<usize> {
    let data = &input[start..];
    let len = data.len();
    let mut offset = 0;

    let newline_vec = _mm_set1_epi8(b'\n' as i8);

    while offset + 16 <= len {
        let chunk = _mm_loadu_si128(data.as_ptr().add(offset) as *const __m128i);
        let matches = _mm_cmpeq_epi8(chunk, newline_vec);
        let mask = _mm_movemask_epi8(matches) as u32;

        if mask != 0 {
            return Some(offset + mask.trailing_zeros() as usize);
        }

        offset += 16;
    }

    // Handle remaining bytes
    for i in offset..len {
        if data[i] == b'\n' {
            return Some(i);
        }
    }

    None
}

#[cfg(any(test, feature = "std"))]
#[target_feature(enable = "avx2")]
unsafe fn find_newline_avx2(input: &[u8], start: usize) -> Option<usize> {
    let data = &input[start..];
    let len = data.len();
    let mut offset = 0;

    let newline_vec = _mm256_set1_epi8(b'\n' as i8);

    while offset + 32 <= len {
        let chunk = _mm256_loadu_si256(data.as_ptr().add(offset) as *const __m256i);
        let matches = _mm256_cmpeq_epi8(chunk, newline_vec);
        let mask = _mm256_movemask_epi8(matches) as u32;

        if mask != 0 {
            return Some(offset + mask.trailing_zeros() as usize);
        }

        offset += 32;
    }

    // Handle remaining bytes with SSE2
    if offset + 16 <= len {
        let newline_vec_sse = _mm_set1_epi8(b'\n' as i8);
        let chunk = _mm_loadu_si128(data.as_ptr().add(offset) as *const __m128i);
        let matches = _mm_cmpeq_epi8(chunk, newline_vec_sse);
        let mask = _mm_movemask_epi8(matches) as u32;

        if mask != 0 {
            return Some(offset + mask.trailing_zeros() as usize);
        }
        offset += 16;
    }

    // Handle remaining bytes
    for i in offset..len {
        if data[i] == b'\n' {
            return Some(i);
        }
    }

    None
}

// ============================================================================
// Original SIMD Functions (Enhanced)
// ============================================================================

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

/// Count leading spaces (indentation) using x86 SIMD.
///
/// Returns the number of consecutive space characters starting at `start`.
#[inline]
pub fn count_leading_spaces_x86(input: &[u8], start: usize) -> usize {
    // Runtime dispatch to best available implementation
    #[cfg(any(test, feature = "std"))]
    {
        if is_x86_feature_detected!("avx2") {
            // SAFETY: We just checked for AVX2 support
            return unsafe { count_leading_spaces_avx2(input, start) };
        }
    }

    // SAFETY: SSE2 is guaranteed on x86_64
    unsafe { count_leading_spaces_sse2(input, start) }
}

#[target_feature(enable = "sse2")]
unsafe fn count_leading_spaces_sse2(input: &[u8], start: usize) -> usize {
    let data = &input[start..];
    let len = data.len();
    let mut offset = 0;

    let space_vec = _mm_set1_epi8(b' ' as i8);

    // Process 16-byte chunks
    while offset + 16 <= len {
        let chunk = _mm_loadu_si128(data.as_ptr().add(offset) as *const __m128i);

        // Compare against space
        let matches = _mm_cmpeq_epi8(chunk, space_vec);

        // Extract bitmask (one bit per byte)
        let mask = _mm_movemask_epi8(matches) as u32;

        if mask != 0xFFFF {
            // Found a non-space - count trailing ones (consecutive spaces from start)
            return offset + (!mask).trailing_zeros() as usize;
        }

        offset += 16;
    }

    // Handle remaining bytes
    offset + data[offset..].iter().take_while(|&&b| b == b' ').count()
}

#[cfg(any(test, feature = "std"))]
#[target_feature(enable = "avx2")]
unsafe fn count_leading_spaces_avx2(input: &[u8], start: usize) -> usize {
    let data = &input[start..];
    let len = data.len();
    let mut offset = 0;

    let space_vec = _mm256_set1_epi8(b' ' as i8);

    // Process 32-byte chunks
    while offset + 32 <= len {
        let chunk = _mm256_loadu_si256(data.as_ptr().add(offset) as *const __m256i);

        // Compare against space
        let matches = _mm256_cmpeq_epi8(chunk, space_vec);

        // Extract bitmask (one bit per byte)
        let mask = _mm256_movemask_epi8(matches) as u32;

        if mask != 0xFFFF_FFFF {
            // Found a non-space - count trailing ones (consecutive spaces from start)
            return offset + (!mask).trailing_zeros() as usize;
        }

        offset += 32;
    }

    // Handle remaining bytes (16-31 bytes) with SSE2
    if offset + 16 <= len {
        let space_vec_sse = _mm_set1_epi8(b' ' as i8);
        let chunk = _mm_loadu_si128(data.as_ptr().add(offset) as *const __m128i);
        let matches = _mm_cmpeq_epi8(chunk, space_vec_sse);
        let mask = _mm_movemask_epi8(matches) as u32;

        if mask != 0xFFFF {
            return offset + (!mask).trailing_zeros() as usize;
        }
        offset += 16;
    }

    // Handle remaining bytes (< 16)
    offset + data[offset..].iter().take_while(|&&b| b == b' ').count()
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

    #[test]
    fn test_sse2_count_leading_spaces_basic() {
        unsafe {
            assert_eq!(count_leading_spaces_sse2(b"  hello", 0), 2);
            assert_eq!(count_leading_spaces_sse2(b"    world", 0), 4);
            assert_eq!(count_leading_spaces_sse2(b"no spaces", 0), 0);
        }
    }

    #[test]
    fn test_sse2_count_leading_spaces_long() {
        // Test with > 16 bytes to exercise SIMD path
        let mut input = vec![b' '; 50];
        input.extend_from_slice(b"content");
        unsafe {
            assert_eq!(count_leading_spaces_sse2(&input, 0), 50);
        }
    }

    #[test]
    fn test_dispatched_count_leading_spaces() {
        assert_eq!(count_leading_spaces_x86(b"  hello", 0), 2);
        assert_eq!(count_leading_spaces_x86(b"    world", 0), 4);
        assert_eq!(count_leading_spaces_x86(b"no spaces", 0), 0);

        // Test long string
        let mut input = vec![b' '; 50];
        input.extend_from_slice(b"content");
        assert_eq!(count_leading_spaces_x86(&input, 0), 50);
    }

    // ========================================================================
    // Tests for P0 optimizations
    // ========================================================================

    #[test]
    fn test_classify_yaml_chars_basic() {
        // Keep input under 16 bytes for SSE2 test
        let input = b"key: #val-item\nmore";
        let class = classify_yaml_chars(input, 0).unwrap();

        // Check colon at position 3
        assert_ne!(class.colons & (1 << 3), 0, "Colon not found at position 3");

        // Check space at position 4
        assert_ne!(class.spaces & (1 << 4), 0, "Space not found at position 4");

        // Check hash at position 5
        assert_ne!(class.hash & (1 << 5), 0, "Hash not found at position 5");

        // Check hyphen at position 9
        assert_ne!(
            class.hyphens & (1 << 9),
            0,
            "Hyphen not found at position 9"
        );

        // Check newline at position 14
        assert_ne!(
            class.newlines & (1 << 14),
            0,
            "Newline not found at position 14"
        );
    }

    #[test]
    fn test_classify_yaml_chars_quotes() {
        let input = b"a: \"val\" 'x'end."; // 16 bytes
        let class = classify_yaml_chars(input, 0).unwrap();

        // Check double quote at position 3
        assert_ne!(
            class.quotes_double & (1 << 3),
            0,
            "Double quote not found at position 3"
        );

        // Check double quote at position 7
        assert_ne!(
            class.quotes_double & (1 << 7),
            0,
            "Double quote not found at position 7"
        );

        // Check single quote at position 9
        assert_ne!(
            class.quotes_single & (1 << 9),
            0,
            "Single quote not found at position 9"
        );

        // Check single quote at position 11
        assert_ne!(
            class.quotes_single & (1 << 11),
            0,
            "Single quote not found at position 11"
        );
    }

    #[test]
    fn test_classify_yaml_chars_backslash() {
        let input = b"text: \"esc\\n\"ok."; // 16 bytes
        let class = classify_yaml_chars(input, 0).unwrap();

        // Check backslash at position 10
        assert_ne!(
            class.backslashes & (1 << 10),
            0,
            "Backslash not found at position 10"
        );
    }

    #[test]
    fn test_find_newline_basic() {
        let input = b"line1\nline2\nline3";
        assert_eq!(find_newline_x86(input, 0), Some(5));
        assert_eq!(find_newline_x86(input, 6), Some(5)); // 5 bytes from offset 6 = position 11
    }

    #[test]
    fn test_find_newline_long() {
        let mut input = vec![b'x'; 100];
        input[50] = b'\n';
        assert_eq!(find_newline_x86(&input, 0), Some(50));
    }

    #[test]
    fn test_find_newline_not_found() {
        let input = b"no newline here";
        assert_eq!(find_newline_x86(input, 0), None);
    }

    #[test]
    fn test_classify_context_sensitive() {
        // Test ": " pattern (colon followed by space)
        let input = b"key: val t:1:2x."; // 16 bytes
        let class = classify_yaml_chars(input, 0).unwrap();

        // Colon at position N followed by space at N+1 means:
        // - Colon bit is set at position N
        // - Space bit is set at position N+1
        // - Shift space mask right by 1 to align with colon position
        // - AND with colon mask: colons & (spaces >> 1)
        let colon_space_pattern = class.colons & (class.spaces >> 1);

        // Position 3: colon followed by space at 4 should match
        assert_ne!(
            colon_space_pattern & (1 << 3),
            0,
            "Colon-space pattern not found at position 3"
        );

        // Positions 11, 13: colons not followed by space should not match
        assert_eq!(
            colon_space_pattern & (1 << 11),
            0,
            "False positive: colon-space at position 11"
        );
        assert_eq!(
            colon_space_pattern & (1 << 13),
            0,
            "False positive: colon-space at position 13"
        );
    }

    #[test]
    fn test_classify_hyphen_space_pattern() {
        // Test "- " pattern (hyphen followed by space)
        let input = b"- item\nval-nosp."; // 16 bytes
        let class = classify_yaml_chars(input, 0).unwrap();

        // Hyphen at position N followed by space at N+1
        let hyphen_space_pattern = class.hyphens & (class.spaces >> 1);

        // Position 0: hyphen followed by space at 1 should match
        assert_ne!(
            hyphen_space_pattern & (1 << 0),
            0,
            "Hyphen-space pattern not found at position 0"
        );

        // Position 11: hyphen not followed by space should not match
        assert_eq!(
            hyphen_space_pattern & (1 << 11),
            0,
            "False positive: hyphen-space at position 11"
        );
    }
}
