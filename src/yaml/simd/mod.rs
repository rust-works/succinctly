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
//!
//! ## Broadword Fallback
//!
//! The `broadword` module provides a portable implementation using pure u64
//! arithmetic (SWAR = SIMD Within A Register). This works on any platform
//! without CPU-specific intrinsics and can be enabled with `--features broadword-yaml`
//! to use instead of NEON on ARM64 for comparison testing.
//!
//! ## Scalar Fallback
//!
//! Use `--features scalar-yaml` to force pure scalar (byte-by-byte) implementations.
//! This is useful for benchmarking baseline performance without any SIMD or broadword.

// NEON module only when not using broadword or scalar fallback
#[cfg(all(
    target_arch = "aarch64",
    not(feature = "broadword-yaml"),
    not(feature = "scalar-yaml")
))]
mod neon;

// x86 module only when not using scalar fallback
#[cfg(all(target_arch = "x86_64", not(feature = "scalar-yaml")))]
mod x86;

// Portable broadword module - compiled when:
// 1. broadword-yaml feature is enabled on ARM64 (explicit opt-in), OR
// 2. On non-SIMD platforms (automatic fallback)
// NOT compiled on x86_64 (uses native SIMD) or ARM64 without broadword-yaml (uses NEON)
#[cfg(all(
    not(feature = "scalar-yaml"),
    any(
        all(target_arch = "aarch64", feature = "broadword-yaml"),
        not(any(target_arch = "aarch64", target_arch = "x86_64"))
    )
))]
mod broadword;

// Re-export platform-specific types for P0 optimizations
#[cfg(all(target_arch = "x86_64", not(feature = "scalar-yaml")))]
pub use x86::YamlCharClass;

// Re-export broadword types when broadword module is compiled
#[cfg(all(
    not(feature = "scalar-yaml"),
    any(
        all(target_arch = "aarch64", feature = "broadword-yaml"),
        not(any(target_arch = "aarch64", target_arch = "x86_64"))
    )
))]
pub use broadword::{YamlCharClass16, YamlCharClassBroadword};

// Re-export NEON types on ARM64 when using NEON (not broadword)
// Only YamlCharClass16 is used; YamlCharClassBroadword is broadword-specific
#[cfg(all(
    target_arch = "aarch64",
    not(feature = "broadword-yaml"),
    not(feature = "scalar-yaml")
))]
pub use neon::YamlCharClass16;

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

    // Scalar fallback: when scalar-yaml feature is enabled
    #[cfg(feature = "scalar-yaml")]
    {
        find_quote_or_escape_scalar(input, start, end)
    }

    // ARM64 with NEON (default, when not scalar or broadword)
    #[cfg(all(
        target_arch = "aarch64",
        not(feature = "broadword-yaml"),
        not(feature = "scalar-yaml")
    ))]
    {
        neon::find_quote_or_escape_neon(input, start, end)
    }

    // x86_64 uses native SIMD (when not scalar)
    #[cfg(all(target_arch = "x86_64", not(feature = "scalar-yaml")))]
    {
        x86::find_quote_or_escape_x86(input, start, end)
    }

    // Broadword fallback: when feature enabled on ARM64, or on non-SIMD platforms (when not scalar)
    #[cfg(all(
        not(feature = "scalar-yaml"),
        any(
            all(target_arch = "aarch64", feature = "broadword-yaml"),
            not(any(target_arch = "aarch64", target_arch = "x86_64"))
        )
    ))]
    {
        broadword::find_quote_or_escape_broadword(input, start, end)
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

    // Scalar fallback: when scalar-yaml feature is enabled
    #[cfg(feature = "scalar-yaml")]
    {
        find_single_quote_scalar(input, start, end)
    }

    // ARM64 with NEON (default, when not scalar or broadword)
    #[cfg(all(
        target_arch = "aarch64",
        not(feature = "broadword-yaml"),
        not(feature = "scalar-yaml")
    ))]
    {
        neon::find_single_quote_neon(input, start, end)
    }

    // x86_64 uses native SIMD (when not scalar)
    #[cfg(all(target_arch = "x86_64", not(feature = "scalar-yaml")))]
    {
        x86::find_single_quote_x86(input, start, end)
    }

    // Broadword fallback: when feature enabled on ARM64, or on non-SIMD platforms (when not scalar)
    #[cfg(all(
        not(feature = "scalar-yaml"),
        any(
            all(target_arch = "aarch64", feature = "broadword-yaml"),
            not(any(target_arch = "aarch64", target_arch = "x86_64"))
        )
    ))]
    {
        broadword::find_single_quote_broadword(input, start, end)
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

    // Scalar fallback: when scalar-yaml feature is enabled
    #[cfg(feature = "scalar-yaml")]
    {
        count_leading_spaces_scalar(input, start)
    }

    // ARM64 with NEON (default, when not scalar or broadword)
    #[cfg(all(
        target_arch = "aarch64",
        not(feature = "broadword-yaml"),
        not(feature = "scalar-yaml")
    ))]
    {
        neon::count_leading_spaces_neon(input, start)
    }

    // x86_64 uses native SIMD (when not scalar)
    #[cfg(all(target_arch = "x86_64", not(feature = "scalar-yaml")))]
    {
        x86::count_leading_spaces_x86(input, start)
    }

    // Broadword fallback: when feature enabled on ARM64, or on non-SIMD platforms (when not scalar)
    #[cfg(all(
        not(feature = "scalar-yaml"),
        any(
            all(target_arch = "aarch64", feature = "broadword-yaml"),
            not(any(target_arch = "aarch64", target_arch = "x86_64"))
        )
    ))]
    {
        broadword::count_leading_spaces_broadword(input, start)
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
#[cfg(all(target_arch = "x86_64", not(feature = "scalar-yaml")))]
pub fn classify_yaml_chars(input: &[u8], offset: usize) -> Option<YamlCharClass> {
    x86::classify_yaml_chars(input, offset)
}

/// Classify 16 bytes of YAML structural characters.
///
/// On ARM64: Uses NEON or broadword depending on feature flags.
/// On other platforms: Uses broadword (SWAR).
/// Returns classification bitmasks for 8 character types at once.
///
/// Available on ARM64 and non-SIMD platforms.
#[inline]
#[cfg(all(
    not(feature = "scalar-yaml"),
    any(target_arch = "aarch64", not(target_arch = "x86_64"))
))]
#[allow(dead_code)]
pub fn classify_yaml_chars_16(input: &[u8], offset: usize) -> Option<YamlCharClass16> {
    // ARM64 with NEON (default)
    #[cfg(all(target_arch = "aarch64", not(feature = "broadword-yaml")))]
    {
        neon::classify_yaml_chars_16(input, offset)
    }

    // Broadword: when feature enabled on ARM64, or on non-SIMD platforms
    #[cfg(any(
        all(target_arch = "aarch64", feature = "broadword-yaml"),
        not(any(target_arch = "aarch64", target_arch = "x86_64"))
    ))]
    {
        broadword::classify_yaml_chars_16(input, offset)
    }
}

/// Classify 8 bytes of YAML structural characters using broadword operations.
///
/// Uses pure u64 arithmetic instead of SIMD intrinsics.
///
/// Available on ARM64 (with broadword-yaml feature) and non-SIMD platforms.
#[inline]
#[cfg(all(
    not(feature = "scalar-yaml"),
    any(
        all(target_arch = "aarch64", feature = "broadword-yaml"),
        not(any(target_arch = "aarch64", target_arch = "x86_64"))
    )
))]
#[allow(dead_code)]
pub fn classify_yaml_chars_8(input: &[u8], offset: usize) -> Option<YamlCharClassBroadword> {
    broadword::classify_yaml_chars_broadword(input, offset)
}

/// Find the next newline (`\n`) from the given position.
///
/// Returns offset from `start` to the newline, or `None` if not found.
/// Uses SIMD/broadword for fast scanning on supported platforms.
#[inline]
#[allow(dead_code)]
pub fn find_newline(input: &[u8], start: usize) -> Option<usize> {
    if start >= input.len() {
        return None;
    }

    // Scalar fallback: when scalar-yaml feature is enabled
    #[cfg(feature = "scalar-yaml")]
    {
        find_newline_scalar(input, start)
    }

    // x86_64 uses native SIMD (when not scalar)
    #[cfg(all(target_arch = "x86_64", not(feature = "scalar-yaml")))]
    {
        x86::find_newline_x86(input, start)
    }

    // ARM64 with NEON (default, when not scalar or broadword)
    #[cfg(all(
        target_arch = "aarch64",
        not(feature = "broadword-yaml"),
        not(feature = "scalar-yaml")
    ))]
    {
        neon::find_newline_broadword(input, start)
    }

    // Broadword: when feature enabled on ARM64, or on non-SIMD platforms (when not scalar)
    #[cfg(all(
        not(feature = "scalar-yaml"),
        any(
            all(target_arch = "aarch64", feature = "broadword-yaml"),
            not(any(target_arch = "aarch64", target_arch = "x86_64"))
        )
    ))]
    {
        broadword::find_newline_broadword(input, start)
    }
}

/// Find the end of a block scalar by scanning for a line with insufficient indentation.
///
/// Uses SIMD to find newlines and check indentation efficiently.
/// Returns the position where the block ends (start of line with insufficient indent),
/// or input.len() if EOF is reached.
///
/// This is used to quickly skip to the end of `|` (literal) or `>` (folded) block scalars.
#[inline]
pub fn find_block_scalar_end(input: &[u8], start: usize, min_indent: usize) -> Option<usize> {
    #[cfg(target_arch = "x86_64")]
    {
        x86::find_block_scalar_end(input, start, min_indent)
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        find_block_scalar_end_scalar(input, start, min_indent)
    }
}

/// Scalar fallback for find_block_scalar_end
#[allow(dead_code)]
fn find_block_scalar_end_scalar(input: &[u8], start: usize, min_indent: usize) -> Option<usize> {
    let mut pos = start;

    while pos < input.len() {
        if input[pos] == b'\n' {
            let line_start = pos + 1;

            if line_start >= input.len() {
                return Some(input.len());
            }

            // Count leading spaces
            let mut indent = 0;
            while line_start + indent < input.len() && input[line_start + indent] == b' ' {
                indent += 1;
            }

            // Check if this line has content at insufficient indent
            if line_start + indent < input.len() {
                let next_char = input[line_start + indent];
                if next_char != b'\n' && next_char != b'\r' && indent < min_indent {
                    return Some(line_start);
                }
            }
        }
        pos += 1;
    }

    Some(input.len())
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
