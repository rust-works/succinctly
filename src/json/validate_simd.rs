//! SIMD-accelerated string validation for JSON validator.
//!
//! This module provides integrated scanning that combines:
//! - String terminator detection (`"`, `\`, control chars < 0x20)
//! - UTF-8 validation (ASCII fast path + scalar fallback for multi-byte)
//!
//! Both operations run on the same loaded chunk, avoiding redundant memory access.
//! The SIMD implementation processes 32 bytes at a time with AVX2.

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

/// Result of validating a string chunk.
#[derive(Debug, Clone, Copy)]
pub struct ChunkResult {
    /// Offset to first terminator (relative to chunk start), or 32 if none found.
    pub terminator_offset: usize,
    /// True if UTF-8 is valid up to the terminator (or entire chunk if no terminator).
    pub utf8_valid: bool,
    /// Number of continuation bytes expected at start of next chunk (0-3).
    /// Non-zero means the chunk ended mid-sequence.
    pub pending_continuations: u8,
}

/// State for tracking UTF-8 validation across chunk boundaries.
#[derive(Debug, Clone, Copy, Default)]
pub struct Utf8State {
    /// Number of continuation bytes expected (0-3).
    pub pending_continuations: u8,
}

impl Utf8State {
    pub fn new() -> Self {
        Self { pending_continuations: 0 }
    }
}

/// Validate a string chunk: find terminators and validate UTF-8 in one pass.
///
/// This is the main entry point for chunked string validation.
#[inline]
pub fn validate_string_chunk(
    input: &[u8],
    start: usize,
    utf8_state: &mut Utf8State,
) -> ChunkResult {
    if start >= input.len() {
        return ChunkResult {
            terminator_offset: 0,
            utf8_valid: true,
            pending_continuations: utf8_state.pending_continuations,
        };
    }

    let remaining = input.len() - start;

    // Use AVX2 for 32+ bytes
    #[cfg(all(target_arch = "x86_64", any(test, feature = "std")))]
    if remaining >= 32 && is_x86_feature_detected!("avx2") {
        return unsafe { validate_string_chunk_avx2(input, start, utf8_state) };
    }

    // Scalar fallback
    validate_string_chunk_scalar(input, start, utf8_state)
}

/// AVX2 implementation - processes 32 bytes for terminators and UTF-8.
///
/// Strategy:
/// 1. Load 32 bytes into SIMD register (single memory access)
/// 2. Find terminators using SIMD comparisons
/// 3. Check if chunk is all-ASCII using high bit test
/// 4. If ASCII: return terminator position (UTF-8 trivially valid)
/// 5. If non-ASCII: use scalar to validate UTF-8 up to terminator
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn validate_string_chunk_avx2(
    input: &[u8],
    start: usize,
    utf8_state: &mut Utf8State,
) -> ChunkResult {
    let data = &input[start..];
    let chunk = _mm256_loadu_si256(data.as_ptr() as *const __m256i);

    // Set up comparison vectors (these get hoisted by the compiler in hot loops)
    let quote_vec = _mm256_set1_epi8(b'"' as i8);
    let backslash_vec = _mm256_set1_epi8(b'\\' as i8);
    let control_threshold = _mm256_set1_epi8(0x20);
    let high_bit_vec = _mm256_set1_epi8(0x80u8 as i8);

    // Find terminators: ", \, or control chars (< 0x20)
    let is_quote = _mm256_cmpeq_epi8(chunk, quote_vec);
    let is_backslash = _mm256_cmpeq_epi8(chunk, backslash_vec);
    let is_control = _mm256_cmpgt_epi8(control_threshold, chunk);

    let terminator_mask = _mm256_or_si256(_mm256_or_si256(is_quote, is_backslash), is_control);
    let term_bits = _mm256_movemask_epi8(terminator_mask) as u32;

    let terminator_offset = if term_bits != 0 {
        term_bits.trailing_zeros() as usize
    } else {
        32
    };

    // Check for non-ASCII bytes (high bit set)
    let has_high_bit = _mm256_and_si256(chunk, high_bit_vec);
    let high_bit_mask = _mm256_movemask_epi8(has_high_bit) as u32;

    // ASCII fast path: if no high bits and no pending continuations, UTF-8 is trivially valid
    if high_bit_mask == 0 && utf8_state.pending_continuations == 0 {
        return ChunkResult {
            terminator_offset,
            utf8_valid: true,
            pending_continuations: 0,
        };
    }

    // Non-ASCII path: validate UTF-8 using scalar up to terminator
    // This handles multi-byte sequences correctly including cross-chunk boundaries
    let validate_len = terminator_offset.min(32);
    validate_utf8_scalar(&data[..validate_len], utf8_state, terminator_offset)
}

/// Validate UTF-8 in a slice using scalar code.
/// Returns ChunkResult with the given terminator_offset and UTF-8 validity.
fn validate_utf8_scalar(data: &[u8], utf8_state: &mut Utf8State, terminator_offset: usize) -> ChunkResult {
    let mut pending = utf8_state.pending_continuations;
    let mut utf8_valid = true;

    for &b in data.iter() {
        if pending > 0 {
            // Expecting continuation byte (10xxxxxx)
            if (b & 0xC0) == 0x80 {
                pending -= 1;
            } else {
                utf8_valid = false;
                pending = 0;
            }
        } else if b >= 0x80 {
            // Start of multi-byte sequence
            if (b & 0xE0) == 0xC0 {
                pending = 1; // 2-byte sequence
            } else if (b & 0xF0) == 0xE0 {
                pending = 2; // 3-byte sequence
            } else if (b & 0xF8) == 0xF0 {
                pending = 3; // 4-byte sequence
            } else {
                // Invalid leading byte (10xxxxxx or 11111xxx)
                utf8_valid = false;
            }
        }
        // ASCII bytes (< 0x80) need no validation
    }

    utf8_state.pending_continuations = pending;

    ChunkResult {
        terminator_offset,
        utf8_valid,
        pending_continuations: pending,
    }
}

/// Scalar fallback for chunks smaller than 32 bytes.
fn validate_string_chunk_scalar(
    input: &[u8],
    start: usize,
    utf8_state: &mut Utf8State,
) -> ChunkResult {
    let data = &input[start..];
    let mut pending = utf8_state.pending_continuations;
    let mut utf8_valid = true;

    for (i, &b) in data.iter().enumerate() {
        // Check for terminator
        if b == b'"' || b == b'\\' || b < 0x20 {
            utf8_state.pending_continuations = pending;
            return ChunkResult {
                terminator_offset: i,
                utf8_valid,
                pending_continuations: pending,
            };
        }

        // UTF-8 validation
        if pending > 0 {
            // Expecting continuation byte
            if (b & 0xC0) == 0x80 {
                pending -= 1;
            } else {
                utf8_valid = false;
                pending = 0;
            }
        } else if b >= 0x80 {
            // Start of multi-byte sequence
            if (b & 0xE0) == 0xC0 {
                pending = 1;
            } else if (b & 0xF0) == 0xE0 {
                pending = 2;
            } else if (b & 0xF8) == 0xF0 {
                pending = 3;
            } else {
                // Invalid leading byte or unexpected continuation
                utf8_valid = false;
            }
        }
    }

    utf8_state.pending_continuations = pending;

    ChunkResult {
        terminator_offset: data.len(),
        utf8_valid,
        pending_continuations: pending,
    }
}

// Keep the old function for compatibility during transition
#[inline]
pub fn find_string_terminator(input: &[u8], start: usize) -> Option<usize> {
    let mut state = Utf8State::new();
    let result = validate_string_chunk(input, start, &mut state);
    if result.terminator_offset < input.len() - start {
        Some(result.terminator_offset)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_find_quote() {
        let mut state = Utf8State::new();
        let input = b"hello\"world";
        let result = validate_string_chunk(input, 0, &mut state);
        assert_eq!(result.terminator_offset, 5);
        assert!(result.utf8_valid);
    }

    #[test]
    fn test_chunk_ascii_valid() {
        let mut state = Utf8State::new();
        let input = b"hello world this is a test!!!!!\"";
        let result = validate_string_chunk(input, 0, &mut state);
        assert_eq!(result.terminator_offset, 31); // quote is at position 31
        assert!(result.utf8_valid);
    }

    #[test]
    fn test_chunk_utf8_2byte() {
        let mut state = Utf8State::new();
        let input = "Café\"".as_bytes();
        let result = validate_string_chunk(input, 0, &mut state);
        assert!(result.utf8_valid);
        assert_eq!(input[result.terminator_offset], b'"');
    }

    #[test]
    fn test_chunk_utf8_3byte() {
        let mut state = Utf8State::new();
        let input = "Hello 世界\"".as_bytes();
        let result = validate_string_chunk(input, 0, &mut state);
        assert!(result.utf8_valid);
        assert_eq!(input[result.terminator_offset], b'"');
    }

    #[test]
    fn test_chunk_utf8_4byte() {
        let mut state = Utf8State::new();
        let input = "Emoji 🎉 test\"".as_bytes();
        let result = validate_string_chunk(input, 0, &mut state);
        assert!(result.utf8_valid);
        assert_eq!(input[result.terminator_offset], b'"');
    }

    #[test]
    fn test_chunk_control_char() {
        let mut state = Utf8State::new();
        let input = b"hello\x00world";
        let result = validate_string_chunk(input, 0, &mut state);
        assert_eq!(result.terminator_offset, 5);
    }

    #[test]
    fn test_chunk_backslash() {
        let mut state = Utf8State::new();
        let input = b"hello\\nworld";
        let result = validate_string_chunk(input, 0, &mut state);
        assert_eq!(result.terminator_offset, 5);
    }

    #[test]
    fn test_find_string_terminator_compat() {
        // Test that the compatibility wrapper still works
        let input = b"hello\"world";
        assert_eq!(find_string_terminator(input, 0), Some(5));

        let input = b"hello world";
        assert_eq!(find_string_terminator(input, 0), None);
    }

    #[test]
    fn test_long_utf8_string() {
        let mut state = Utf8State::new();
        // A string with mixed UTF-8 that's longer than 32 bytes
        let input = "Hello 世界 Привет мир مرحبا 🌍🌎🌏 end\"".as_bytes();
        let result = validate_string_chunk(input, 0, &mut state);
        assert!(result.utf8_valid);
    }

    #[test]
    fn test_long_ascii_string() {
        let mut state = Utf8State::new();
        // ASCII string longer than 32 bytes should use SIMD fast path
        // First chunk (32 bytes) has no terminator
        let input = b"This is a long ASCII string that is definitely more than 32 bytes long!\"";
        let result = validate_string_chunk(input, 0, &mut state);
        assert!(result.utf8_valid);
        // No terminator in first 32-byte chunk
        assert_eq!(result.terminator_offset, 32);
        assert_eq!(result.pending_continuations, 0);

        // Second chunk (starting at 32) also has no terminator in first 32 bytes
        let result2 = validate_string_chunk(input, 32, &mut state);
        assert!(result2.utf8_valid);
        assert_eq!(result2.terminator_offset, 32);

        // Third chunk (starting at 64) should find the quote at position 7 (64+7=71)
        let result3 = validate_string_chunk(input, 64, &mut state);
        assert!(result3.utf8_valid);
        assert_eq!(result3.terminator_offset, 7);
        assert_eq!(input[64 + result3.terminator_offset], b'"');
    }

    #[test]
    fn test_invalid_utf8_standalone_continuation() {
        let mut state = Utf8State::new();
        // Standalone continuation byte (invalid UTF-8)
        let input = b"hello\x80world\"";
        let result = validate_string_chunk(input, 0, &mut state);
        assert!(!result.utf8_valid);
    }

    #[test]
    fn test_invalid_utf8_truncated_sequence() {
        let mut state = Utf8State::new();
        // Start of 2-byte sequence without continuation
        let input = b"hello\xC2\"";
        let result = validate_string_chunk(input, 0, &mut state);
        // The terminator is found at position 6 (the quote)
        assert_eq!(result.terminator_offset, 6);
        // pending_continuations should be 1 (expecting a continuation byte)
        assert_eq!(result.pending_continuations, 1);
    }

    #[test]
    fn test_utf8_cross_chunk_boundary() {
        // Simulate a 2-byte UTF-8 character split across chunks
        let mut state = Utf8State::new();

        // First "chunk": ends with start of 2-byte sequence
        let input1 = b"hello\xC3";
        let result1 = validate_string_chunk_scalar(input1, 0, &mut state);
        assert!(result1.utf8_valid);
        assert_eq!(result1.pending_continuations, 1);

        // Second "chunk": starts with continuation byte
        let input2 = b"\xA9 world\""; // é continuation + more
        let result2 = validate_string_chunk_scalar(input2, 0, &mut state);
        assert!(result2.utf8_valid);
        assert_eq!(result2.pending_continuations, 0);
    }
}
