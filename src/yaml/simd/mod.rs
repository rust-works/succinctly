//! SIMD-accelerated string scanning for YAML parsing.
//!
//! This module provides vectorized implementations for scanning inside
//! quoted strings, which is one of the main hot paths in YAML parsing.
//!
//! The key optimization is skipping to the next "interesting" character
//! (quote or escape) rather than checking each byte individually.
//!
//! ## Algorithm
//!
//! For double-quoted strings, find the next `"` or `\` character.
//! For single-quoted strings, find the next `'` character.
//!
//! Uses SIMD comparisons to check 16/32 bytes at a time, then
//! extracts a bitmask and uses `trailing_zeros()` to find the position.

#[cfg(target_arch = "aarch64")]
mod neon;

#[cfg(target_arch = "x86_64")]
mod x86;

// Re-export platform-specific types for P0 optimizations
#[cfg(target_arch = "x86_64")]
pub use x86::YamlCharClass;

// ============================================================================
// Public API - platform-dispatched functions
// ============================================================================

/// Find the next double-quote (`"`) or backslash (`\`) in the input.
///
/// Returns the offset from `start` to the found character, or `None` if
/// neither is found before `end`.
///
/// This is used for fast-path scanning inside double-quoted strings.
#[inline]
pub fn find_quote_or_escape(input: &[u8], start: usize, end: usize) -> Option<usize> {
    if start >= end || start >= input.len() {
        return None;
    }
    let end = end.min(input.len());

    #[cfg(target_arch = "aarch64")]
    {
        neon::find_quote_or_escape_neon(input, start, end)
    }

    #[cfg(target_arch = "x86_64")]
    {
        x86::find_quote_or_escape_x86(input, start, end)
    }

    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        find_quote_or_escape_scalar(input, start, end)
    }
}

/// Find the next single-quote (`'`) in the input.
///
/// Returns the offset from `start` to the found character, or `None` if
/// not found before `end`.
///
/// This is used for fast-path scanning inside single-quoted strings.
#[inline]
pub fn find_single_quote(input: &[u8], start: usize, end: usize) -> Option<usize> {
    if start >= end || start >= input.len() {
        return None;
    }
    let end = end.min(input.len());

    #[cfg(target_arch = "aarch64")]
    {
        neon::find_single_quote_neon(input, start, end)
    }

    #[cfg(target_arch = "x86_64")]
    {
        x86::find_single_quote_x86(input, start, end)
    }

    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        find_single_quote_scalar(input, start, end)
    }
}

/// Count leading spaces (indentation) from the given position.
///
/// Returns the number of consecutive space characters starting at `start`.
/// Stops at the first non-space character or end of input.
///
/// This is used for fast indentation counting in YAML block-style parsing.
#[inline]
pub fn count_leading_spaces(input: &[u8], start: usize) -> usize {
    if start >= input.len() {
        return 0;
    }

    #[cfg(target_arch = "aarch64")]
    {
        neon::count_leading_spaces_neon(input, start)
    }

    #[cfg(target_arch = "x86_64")]
    {
        x86::count_leading_spaces_x86(input, start)
    }

    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        count_leading_spaces_scalar(input, start)
    }
}

// ============================================================================
// P0 Optimizations - Multi-Character Classification
// ============================================================================

/// Classify YAML structural characters in a chunk (32 bytes on AVX2, 16 on SSE2).
///
/// Returns classification bitmasks for multiple character types at once.
/// This is the main P0 optimization for bulk character detection.
///
/// Currently only available on x86_64. Returns None on other platforms.
#[inline]
#[cfg(target_arch = "x86_64")]
pub fn classify_yaml_chars(input: &[u8], offset: usize) -> Option<YamlCharClass> {
    x86::classify_yaml_chars(input, offset)
}

/// Find the next newline (`\n`) from the given position.
///
/// Returns offset from `start` to the newline, or `None` if not found.
/// Uses SIMD for fast scanning on supported platforms.
#[inline]
pub fn find_newline(input: &[u8], start: usize) -> Option<usize> {
    if start >= input.len() {
        return None;
    }

    #[cfg(target_arch = "x86_64")]
    {
        x86::find_newline_x86(input, start)
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        find_newline_scalar(input, start)
    }
}

/// Scalar implementation of find_newline.
#[allow(dead_code)]
fn find_newline_scalar(input: &[u8], start: usize) -> Option<usize> {
    for (i, &b) in input[start..].iter().enumerate() {
        if b == b'\n' {
            return Some(i);
        }
    }
    None
}

// ============================================================================
// Scalar fallbacks (for non-SIMD platforms or testing)
// ============================================================================

/// Scalar implementation of find_quote_or_escape.
#[allow(dead_code)]
fn find_quote_or_escape_scalar(input: &[u8], start: usize, end: usize) -> Option<usize> {
    for (i, &b) in input[start..end].iter().enumerate() {
        if b == b'"' || b == b'\\' {
            return Some(i);
        }
    }
    None
}

/// Scalar implementation of find_single_quote.
#[allow(dead_code)]
fn find_single_quote_scalar(input: &[u8], start: usize, end: usize) -> Option<usize> {
    for (i, &b) in input[start..end].iter().enumerate() {
        if b == b'\'' {
            return Some(i);
        }
    }
    None
}

/// Scalar implementation of count_leading_spaces.
#[allow(dead_code)]
fn count_leading_spaces_scalar(input: &[u8], start: usize) -> usize {
    input[start..].iter().take_while(|&&b| b == b' ').count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_quote_or_escape_basic() {
        let input = b"hello\"world";
        assert_eq!(find_quote_or_escape(input, 0, input.len()), Some(5));
    }

    #[test]
    fn test_find_quote_or_escape_backslash() {
        let input = b"hello\\world";
        assert_eq!(find_quote_or_escape(input, 0, input.len()), Some(5));
    }

    #[test]
    fn test_find_quote_or_escape_none() {
        let input = b"hello world";
        assert_eq!(find_quote_or_escape(input, 0, input.len()), None);
    }

    #[test]
    fn test_find_quote_or_escape_start_offset() {
        let input = b"ab\"cd\"ef";
        assert_eq!(find_quote_or_escape(input, 3, input.len()), Some(2)); // 'c' at 3, '"' at 5
    }

    #[test]
    fn test_find_quote_or_escape_long_string() {
        // Test with > 16 bytes to exercise SIMD path
        let mut input = vec![b'a'; 100];
        input[50] = b'"';
        assert_eq!(find_quote_or_escape(&input, 0, input.len()), Some(50));
    }

    #[test]
    fn test_find_single_quote_basic() {
        let input = b"hello'world";
        assert_eq!(find_single_quote(input, 0, input.len()), Some(5));
    }

    #[test]
    fn test_find_single_quote_none() {
        let input = b"hello world";
        assert_eq!(find_single_quote(input, 0, input.len()), None);
    }

    #[test]
    fn test_find_single_quote_long_string() {
        // Test with > 16 bytes to exercise SIMD path
        let mut input = vec![b'a'; 100];
        input[75] = b'\'';
        assert_eq!(find_single_quote(&input, 0, input.len()), Some(75));
    }

    #[test]
    fn test_empty_range() {
        let input = b"hello";
        assert_eq!(find_quote_or_escape(input, 5, 5), None);
        assert_eq!(find_quote_or_escape(input, 10, 5), None);
    }

    #[test]
    fn test_find_at_boundary() {
        // Quote exactly at 16-byte boundary
        let mut input = vec![b'a'; 32];
        input[16] = b'"';
        assert_eq!(find_quote_or_escape(&input, 0, input.len()), Some(16));
    }

    // Compare SIMD vs scalar for correctness
    #[test]
    fn test_simd_matches_scalar() {
        let test_cases: &[&[u8]] = &[
            b"",
            b"\"",
            b"\\",
            b"'",
            b"no special chars here",
            b"quote at end\"",
            b"\"quote at start",
            b"has\\backslash",
            b"has both \" and \\ chars",
            // Long strings
            &[b'x'; 100],
        ];

        for &input in test_cases {
            let scalar_dq = find_quote_or_escape_scalar(input, 0, input.len());
            let simd_dq = find_quote_or_escape(input, 0, input.len());
            assert_eq!(
                scalar_dq,
                simd_dq,
                "double-quote mismatch for {:?}",
                String::from_utf8_lossy(input)
            );

            let scalar_sq = find_single_quote_scalar(input, 0, input.len());
            let simd_sq = find_single_quote(input, 0, input.len());
            assert_eq!(
                scalar_sq,
                simd_sq,
                "single-quote mismatch for {:?}",
                String::from_utf8_lossy(input)
            );
        }
    }

    #[test]
    fn test_count_leading_spaces_basic() {
        assert_eq!(count_leading_spaces(b"  hello", 0), 2);
        assert_eq!(count_leading_spaces(b"    world", 0), 4);
        assert_eq!(count_leading_spaces(b"no spaces", 0), 0);
        assert_eq!(count_leading_spaces(b"", 0), 0);
    }

    #[test]
    fn test_count_leading_spaces_offset() {
        assert_eq!(count_leading_spaces(b"xx  hello", 2), 2);
        assert_eq!(count_leading_spaces(b"key:   value", 4), 3);
    }

    #[test]
    fn test_count_leading_spaces_all_spaces() {
        let spaces = vec![b' '; 100];
        assert_eq!(count_leading_spaces(&spaces, 0), 100);
    }

    #[test]
    fn test_count_leading_spaces_long() {
        // Test with > 16 bytes to exercise SIMD path
        let mut input = vec![b' '; 50];
        input.extend_from_slice(b"content");
        assert_eq!(count_leading_spaces(&input, 0), 50);
    }

    #[test]
    fn test_count_leading_spaces_at_boundary() {
        // Spaces ending exactly at 16-byte boundary
        let mut input = vec![b' '; 16];
        input.push(b'x');
        assert_eq!(count_leading_spaces(&input, 0), 16);

        // Spaces ending at 32-byte boundary
        let mut input32 = vec![b' '; 32];
        input32.push(b'x');
        assert_eq!(count_leading_spaces(&input32, 0), 32);
    }

    #[test]
    fn test_count_leading_spaces_simd_matches_scalar() {
        let test_cases: &[&[u8]] = &[
            b"",
            b" ",
            b"  ",
            b"   ",
            b"    ",
            b"        ",                         // 8 spaces
            b"                ",                 // 16 spaces
            b"                                ", // 32 spaces
            b"no spaces",
            b" one",
            b"  two",
            b"                x",  // 16 spaces + char
            b"                 x", // 17 spaces + char
        ];

        for &input in test_cases {
            let scalar = count_leading_spaces_scalar(input, 0);
            let simd = count_leading_spaces(input, 0);
            assert_eq!(
                scalar,
                simd,
                "leading spaces mismatch for {:?}",
                String::from_utf8_lossy(input)
            );
        }
    }
}
