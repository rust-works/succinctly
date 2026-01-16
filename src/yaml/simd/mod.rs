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
}
