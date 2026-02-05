//! SIMD-accelerated string validation for JSON validator.
//!
//! This module provides integrated 64-byte chunked validation that combines:
//! - UTF-8 validation using the Keiser-Lemire algorithm (< 1 instruction/byte)
//! - String terminator detection (`"`, `\`, control chars < 0x20)
//!
//! Both operations run on the same loaded data, avoiding redundant memory access.
//! The SIMD implementation processes 64 bytes at a time with AVX2.

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

/// Result of validating a string chunk.
///
/// Separates three categories:
/// - UTF-8 validation errors (invalid byte sequences)
/// - Control character errors (bytes < 0x20, illegal in JSON strings)
/// - Valid terminators (quote or backslash, need handling but aren't errors)
///
/// Both validations run to completion on the full chunk. After SIMD completes,
/// the caller checks for errors first (returning the earliest), then handles
/// valid terminators.
#[derive(Debug, Clone, Copy)]
pub struct ChunkResult {
    /// True if UTF-8 is valid for the entire chunk.
    pub utf8_valid: bool,
    /// Offset to first control character (< 0x20), or 64 if none found.
    pub first_control_char: usize,
    /// Offset to first quote or backslash, or 64 if none found.
    pub first_quote_or_backslash: usize,
}

/// Carry state for UTF-8 validation across chunk boundaries.
#[derive(Clone, Copy)]
pub struct Utf8Carry {
    /// Accumulated error (non-zero = invalid).
    pub error: __m256i,
    /// Previous incomplete sequence info.
    pub prev_incomplete: __m256i,
    /// Previous first_len info.
    pub prev_first_len: __m256i,
}

#[cfg(not(target_arch = "x86_64"))]
#[derive(Clone, Copy, Default)]
pub struct Utf8Carry {
    pub pending: u8,
}

#[cfg(target_arch = "x86_64")]
impl Utf8Carry {
    #[target_feature(enable = "avx2")]
    pub unsafe fn new() -> Self {
        Self {
            error: _mm256_setzero_si256(),
            prev_incomplete: _mm256_setzero_si256(),
            prev_first_len: _mm256_setzero_si256(),
        }
    }

    pub fn has_pending(&self) -> bool {
        unsafe { _mm256_testz_si256(self.prev_incomplete, self.prev_incomplete) == 0 }
    }
}

#[cfg(not(target_arch = "x86_64"))]
impl Utf8Carry {
    pub fn new() -> Self {
        Self { pending: 0 }
    }
    pub fn has_error(&self) -> bool {
        false
    }
    pub fn has_pending(&self) -> bool {
        self.pending > 0
    }
}

/// SIMD constant vectors for JSON string validation.
/// Create once at document start and reuse for all chunks.
#[cfg(target_arch = "x86_64")]
pub struct SimdConstants {
    // Terminator detection
    pub quote: __m256i,
    pub backslash: __m256i,

    // Keiser-Lemire UTF-8 lookup tables
    /// For each high nibble, how many continuation bytes follow (0, 1, 2, or 3).
    pub first_len_tbl: __m256i,

    // Useful constants
    pub nibble_mask: __m256i,
}

#[cfg(target_arch = "x86_64")]
impl SimdConstants {
    /// Create SIMD constants. Call once before processing document.
    #[target_feature(enable = "avx2")]
    pub unsafe fn new() -> Self {
        Self {
            // Terminator detection vectors
            quote: _mm256_set1_epi8(b'"' as i8),
            backslash: _mm256_set1_epi8(b'\\' as i8),

            // Keiser-Lemire: first_len_tbl
            // Maps high nibble (0-F) to number of continuation bytes expected.
            // Nibbles 0-7: ASCII (0 continuations)
            // Nibbles 8-B: continuation bytes (0, but these shouldn't start a sequence)
            // Nibble C-D: 2-byte sequences (1 continuation)
            // Nibble E: 3-byte sequences (2 continuations)
            // Nibble F: 4-byte sequences (3 continuations)
            first_len_tbl: _mm256_setr_epi8(
                0, 0, 0, 0, 0, 0, 0, 0, // 0x0-0x7: ASCII
                0, 0, 0, 0, // 0x8-0xB: continuations
                1, 1, // 0xC-0xD: 2-byte
                2, // 0xE: 3-byte
                3, // 0xF: 4-byte
                // Repeat for high lane
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 2, 3,
            ),

            nibble_mask: _mm256_set1_epi8(0x0F),
        }
    }
}

/// Validate a string chunk: find terminators and validate UTF-8 in one pass.
/// Pass pre-created SimdConstants from the caller to avoid per-chunk allocation.
///
/// Returns a `ChunkResult` with separate positions for:
/// - `first_control_char`: ERRORS that must be reported
/// - `first_quote_or_backslash`: Valid terminators that need handling
/// - `utf8_valid`: Whether UTF-8 is valid for the chunk
#[inline]
pub fn validate_string_chunk(
    input: &[u8],
    start: usize,
    carry: &mut Utf8Carry,
    #[cfg(target_arch = "x86_64")] constants: &SimdConstants,
) -> ChunkResult {
    if start >= input.len() {
        return ChunkResult {
            utf8_valid: true,
            first_control_char: 0,
            first_quote_or_backslash: 0,
        };
    }

    let remaining = input.len() - start;

    // Use AVX2 for 64+ bytes
    #[cfg(all(target_arch = "x86_64", any(test, feature = "std")))]
    if remaining >= 64 && is_x86_feature_detected!("avx2") {
        return unsafe { validate_chunk_64_avx2(input, start, carry, constants) };
    }

    // Scalar fallback
    validate_string_chunk_scalar(input, start, carry)
}

/// AVX2 implementation - processes 64 bytes with full Keiser-Lemire UTF-8 validation.
///
/// Both UTF-8 validation and terminator detection run to completion on the full chunk.
/// Returns separate positions for:
/// - Control characters (errors)
/// - Quote/backslash (valid terminators that need handling)
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn validate_chunk_64_avx2(
    input: &[u8],
    start: usize,
    carry: &mut Utf8Carry,
    c: &SimdConstants,
) -> ChunkResult {
    let data = &input[start..];
    let ptr = data.as_ptr() as *const __m256i;

    // Load 64 bytes (two 256-bit registers)
    let chunk0 = _mm256_loadu_si256(ptr);
    let chunk1 = _mm256_loadu_si256(ptr.add(1));

    // === UTF-8 VALIDATION (Keiser-Lemire) ===
    // Validate the full 64-byte chunk
    let (error0, incomplete0, first_len0) =
        validate_utf8_block(chunk0, carry.prev_incomplete, carry.prev_first_len, c);

    let (error1, incomplete1, first_len1) = validate_utf8_block(chunk1, incomplete0, first_len0, c);

    // Accumulate errors
    let total_error = _mm256_or_si256(_mm256_or_si256(carry.error, error0), error1);
    let has_error = _mm256_testz_si256(total_error, total_error) == 0;

    // === CONTROL CHARACTER DETECTION (ERRORS) ===
    // Control chars (< 0x20) are validation errors
    let sign_flip = _mm256_set1_epi8(0x80u8 as i8);
    let bound_unsigned = _mm256_set1_epi8((0x20u8 ^ 0x80u8) as i8);
    let chunk0_unsigned = _mm256_xor_si256(chunk0, sign_flip);
    let chunk1_unsigned = _mm256_xor_si256(chunk1, sign_flip);
    let is_control0 = _mm256_cmpgt_epi8(bound_unsigned, chunk0_unsigned);
    let is_control1 = _mm256_cmpgt_epi8(bound_unsigned, chunk1_unsigned);

    let ctrl0 = _mm256_movemask_epi8(is_control0) as u32;
    let ctrl1 = _mm256_movemask_epi8(is_control1) as u32;
    let control_mask: u64 = (ctrl0 as u64) | ((ctrl1 as u64) << 32);

    // Branchless: trailing_zeros() returns 64 for u64 when all bits are zero
    let first_control_char = control_mask.trailing_zeros() as usize;

    // === TERMINATOR DETECTION (NOT ERRORS) ===
    // Quote and backslash are valid terminators that need handling
    let is_quote0 = _mm256_cmpeq_epi8(chunk0, c.quote);
    let is_backslash0 = _mm256_cmpeq_epi8(chunk0, c.backslash);
    let is_quote1 = _mm256_cmpeq_epi8(chunk1, c.quote);
    let is_backslash1 = _mm256_cmpeq_epi8(chunk1, c.backslash);

    // Combine quote and backslash before movemask to minimize expensive operations
    let terminator0 = _mm256_or_si256(is_quote0, is_backslash0);
    let terminator1 = _mm256_or_si256(is_quote1, is_backslash1);

    let t0 = _mm256_movemask_epi8(terminator0) as u32;
    let t1 = _mm256_movemask_epi8(terminator1) as u32;
    let terminator_mask: u64 = (t0 as u64) | ((t1 as u64) << 32);

    // Branchless: trailing_zeros() returns 64 for u64 when all bits are zero
    let first_quote_or_backslash = terminator_mask.trailing_zeros() as usize;

    // Update carry state for next chunk
    carry.error = total_error;
    carry.prev_incomplete = incomplete1;
    carry.prev_first_len = first_len1;

    ChunkResult {
        utf8_valid: !has_error,
        first_control_char,
        first_quote_or_backslash,
    }
}

/// Shift a 256-bit register right by 1 byte, bringing prev[31] to position 0.
/// This is for cross-chunk validation where we need the last byte(s) of the previous chunk.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
unsafe fn shift_right_1(input: __m256i, prev: __m256i) -> __m256i {
    // We want: result[0] = prev[31], result[1..32] = input[0..31]
    // _mm256_alignr_epi8 works within lanes, so we need a different approach.

    // Step 1: Use permute to bring prev's high lane to low position
    // permute2x128(prev, input, 0x21) gives: [prev_high, input_low]
    let prev_high_input_low = _mm256_permute2x128_si256(prev, input, 0x21);

    // Step 2: alignr within this rearranged vector
    // For low lane: need prev_high[15] (=prev[31]) at pos 0, input_low[0..14] at pos 1..15
    // For high lane: need input_low[15] at pos 16, input_high[0..14] (=input[16..30]) at 17..31
    _mm256_alignr_epi8(input, prev_high_input_low, 15)
}

/// Shift a 256-bit register right by 2 bytes.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
unsafe fn shift_right_2(input: __m256i, prev: __m256i) -> __m256i {
    // result[0..2] = prev[30..32], result[2..32] = input[0..30]
    let prev_high_input_low = _mm256_permute2x128_si256(prev, input, 0x21);
    _mm256_alignr_epi8(input, prev_high_input_low, 14)
}

/// Shift a 256-bit register right by 3 bytes.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
unsafe fn shift_right_3(input: __m256i, prev: __m256i) -> __m256i {
    // result[0..3] = prev[29..32], result[3..32] = input[0..29]
    let prev_high_input_low = _mm256_permute2x128_si256(prev, input, 0x21);
    _mm256_alignr_epi8(input, prev_high_input_low, 13)
}

/// Keiser-Lemire UTF-8 validation for one 32-byte block.
/// Returns (error, prev_incomplete for next block, prev_first_len for next block).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn validate_utf8_block(
    input: __m256i,
    prev_incomplete: __m256i,
    prev_first_len: __m256i,
    c: &SimdConstants,
) -> (__m256i, __m256i, __m256i) {
    // Check if entire block is ASCII (all bytes < 0x80)
    let high_bit = _mm256_set1_epi8(0x80u8 as i8);
    let has_high = _mm256_and_si256(input, high_bit);
    let any_high = _mm256_movemask_epi8(has_high);

    // Also check if we have pending continuations from previous block
    let has_prev_incomplete = _mm256_testz_si256(prev_incomplete, prev_incomplete) == 0;

    if any_high == 0 && !has_prev_incomplete {
        // All ASCII, no pending - trivially valid
        return (
            _mm256_setzero_si256(),
            _mm256_setzero_si256(),
            _mm256_setzero_si256(),
        );
    }

    // Get high nibbles for classification
    let high_nibbles = _mm256_and_si256(_mm256_srli_epi16(input, 4), c.nibble_mask);

    // Look up how many continuation bytes each byte expects (0, 1, 2, or 3)
    let first_len = _mm256_shuffle_epi8(c.first_len_tbl, high_nibbles);

    // === Check for invalid leading bytes ===
    // C0, C1 are overlong 2-byte; F5-FF are invalid
    let is_continuation = _mm256_cmpeq_epi8(
        _mm256_and_si256(input, _mm256_set1_epi8(0xC0u8 as i8)),
        _mm256_set1_epi8(0x80u8 as i8),
    );

    // Check for C0, C1 (overlong 2-byte)
    let byte_eq_c0 = _mm256_cmpeq_epi8(input, _mm256_set1_epi8(0xC0u8 as i8));
    let byte_eq_c1 = _mm256_cmpeq_epi8(input, _mm256_set1_epi8(0xC1u8 as i8));
    let invalid_c0c1 = _mm256_or_si256(byte_eq_c0, byte_eq_c1);

    // Check for F5-FF (invalid)
    // Use unsigned comparison by XORing with 0x80 to flip sign bit
    let sign_flip = _mm256_set1_epi8(0x80u8 as i8);
    let input_unsigned = _mm256_xor_si256(input, sign_flip);
    let f4_unsigned = _mm256_set1_epi8((0xF4u8 ^ 0x80u8) as i8); // 0x74
    let byte_gt_f4 = _mm256_cmpgt_epi8(input_unsigned, f4_unsigned);

    // === Check continuation byte placement ===
    // Use proper cross-lane shifting
    let first_len_shifted_1 = shift_right_1(first_len, prev_first_len);
    let first_len_shifted_2 = shift_right_2(first_len, prev_first_len);
    let first_len_shifted_3 = shift_right_3(first_len, prev_first_len);

    // Expected continuation at position i if first_len[i-1] >= 1
    let expect_cont_1 = _mm256_cmpgt_epi8(first_len_shifted_1, _mm256_setzero_si256());
    // Expected second continuation if first_len[i-2] >= 2
    let expect_cont_2 = _mm256_cmpgt_epi8(first_len_shifted_2, _mm256_set1_epi8(1));
    // Expected third continuation if first_len[i-3] >= 3
    let expect_cont_3 = _mm256_cmpgt_epi8(first_len_shifted_3, _mm256_set1_epi8(2));

    // Combine: we expect a continuation byte if any of the above
    let expect_cont = _mm256_or_si256(expect_cont_1, _mm256_or_si256(expect_cont_2, expect_cont_3));

    // Error: is_continuation XOR expect_cont
    // (unexpected continuation OR missing continuation)
    let cont_error = _mm256_xor_si256(is_continuation, expect_cont);

    // === Combine all errors ===
    let error = _mm256_or_si256(_mm256_or_si256(invalid_c0c1, byte_gt_f4), cont_error);

    // === Calculate incomplete state for next block ===
    // Check last 3 bytes for sequences that continue into next block.
    // Extract only the high 128 bits (bytes 16-31) to avoid full 32-byte store.
    let high128 = _mm256_extracti128_si256::<1>(first_len);

    // Bytes 29, 30, 31 are at positions 13, 14, 15 in the high 128-bit lane
    let byte29 = _mm_extract_epi8::<13>(high128) as u8;
    let byte30 = _mm_extract_epi8::<14>(high128) as u8;
    let byte31 = _mm_extract_epi8::<15>(high128) as u8;

    let incomplete_count = if byte31 >= 1 {
        byte31
    } else if byte30 >= 2 {
        byte30 - 1
    } else if byte29 >= 3 {
        byte29 - 2
    } else {
        0
    };

    let next_incomplete = if incomplete_count > 0 {
        _mm256_set1_epi8(incomplete_count as i8)
    } else {
        _mm256_setzero_si256()
    };

    (error, next_incomplete, first_len)
}

/// Scalar fallback for chunks smaller than 64 bytes.
/// Scans the full chunk and returns separate positions for control chars and quote/backslash.
fn validate_string_chunk_scalar(input: &[u8], start: usize, carry: &mut Utf8Carry) -> ChunkResult {
    let data = &input[start..];
    let chunk_len = data.len().min(64);

    #[cfg(target_arch = "x86_64")]
    let mut pending = if carry.has_pending() { 1u8 } else { 0u8 };
    #[cfg(not(target_arch = "x86_64"))]
    let mut pending = carry.pending;

    let mut utf8_valid = true;
    let mut first_control_char = chunk_len;
    let mut first_quote_or_backslash = chunk_len;

    for (i, &b) in data.iter().take(chunk_len).enumerate() {
        // Track control chars (errors)
        if b < 0x20 && first_control_char == chunk_len {
            first_control_char = i;
        }

        // Track quote/backslash (valid terminators)
        if (b == b'"' || b == b'\\') && first_quote_or_backslash == chunk_len {
            first_quote_or_backslash = i;
        }

        // UTF-8 validation
        if pending > 0 {
            if (b & 0xC0) == 0x80 {
                pending -= 1;
            } else {
                utf8_valid = false;
                pending = 0;
            }
        } else if b >= 0x80 {
            if b < 0xC2 {
                utf8_valid = false;
            } else if b < 0xE0 {
                pending = 1;
            } else if b < 0xF0 {
                pending = 2;
            } else if b <= 0xF4 {
                pending = 3;
            } else {
                utf8_valid = false;
            }
        }
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        carry.pending = pending;
    }

    ChunkResult {
        utf8_valid,
        first_control_char,
        first_quote_or_backslash,
    }
}

// Compatibility wrapper - returns first terminator (control char, quote, or backslash)
// This function is provided for external callers and tests.
#[allow(dead_code)]
#[inline]
pub fn find_string_terminator(input: &[u8], start: usize) -> Option<usize> {
    #[cfg(target_arch = "x86_64")]
    let mut carry = unsafe { Utf8Carry::new() };
    #[cfg(not(target_arch = "x86_64"))]
    let mut carry = Utf8Carry::new();

    #[cfg(target_arch = "x86_64")]
    let constants = unsafe { SimdConstants::new() };

    #[cfg(target_arch = "x86_64")]
    let result = validate_string_chunk(input, start, &mut carry, &constants);
    #[cfg(not(target_arch = "x86_64"))]
    let result = validate_string_chunk(input, start, &mut carry);

    // Return earliest terminator (control char or quote/backslash)
    let chunk_len = (input.len() - start).min(64);
    let terminator_offset = result
        .first_control_char
        .min(result.first_quote_or_backslash);

    if terminator_offset < chunk_len {
        Some(terminator_offset)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to create constants for tests
    #[cfg(target_arch = "x86_64")]
    unsafe fn test_constants() -> SimdConstants {
        SimdConstants::new()
    }

    #[test]
    fn test_chunk_find_quote() {
        let mut carry = unsafe { Utf8Carry::new() };
        #[cfg(target_arch = "x86_64")]
        let constants = unsafe { test_constants() };
        let input = b"hello\"world";
        #[cfg(target_arch = "x86_64")]
        let result = validate_string_chunk(input, 0, &mut carry, &constants);
        #[cfg(not(target_arch = "x86_64"))]
        let result = validate_string_chunk(input, 0, &mut carry);
        assert_eq!(result.first_quote_or_backslash, 5);
        assert_eq!(result.first_control_char, 11); // No control char
        assert!(result.utf8_valid);
    }

    #[test]
    fn test_chunk_ascii_valid() {
        let mut carry = unsafe { Utf8Carry::new() };
        #[cfg(target_arch = "x86_64")]
        let constants = unsafe { test_constants() };
        let input = b"hello world this is a test!!!!!\"";
        #[cfg(target_arch = "x86_64")]
        let result = validate_string_chunk(input, 0, &mut carry, &constants);
        #[cfg(not(target_arch = "x86_64"))]
        let result = validate_string_chunk(input, 0, &mut carry);
        assert_eq!(result.first_quote_or_backslash, 31);
        assert!(result.utf8_valid);
    }

    #[test]
    fn test_chunk_utf8_2byte() {
        let mut carry = unsafe { Utf8Carry::new() };
        #[cfg(target_arch = "x86_64")]
        let constants = unsafe { test_constants() };
        let input = "Café\"".as_bytes();
        #[cfg(target_arch = "x86_64")]
        let result = validate_string_chunk(input, 0, &mut carry, &constants);
        #[cfg(not(target_arch = "x86_64"))]
        let result = validate_string_chunk(input, 0, &mut carry);
        assert!(result.utf8_valid);
        assert_eq!(input[result.first_quote_or_backslash], b'"');
    }

    #[test]
    fn test_chunk_utf8_3byte() {
        let mut carry = unsafe { Utf8Carry::new() };
        #[cfg(target_arch = "x86_64")]
        let constants = unsafe { test_constants() };
        let input = "Hello 世界\"".as_bytes();
        #[cfg(target_arch = "x86_64")]
        let result = validate_string_chunk(input, 0, &mut carry, &constants);
        #[cfg(not(target_arch = "x86_64"))]
        let result = validate_string_chunk(input, 0, &mut carry);
        assert!(result.utf8_valid);
        assert_eq!(input[result.first_quote_or_backslash], b'"');
    }

    #[test]
    fn test_chunk_utf8_4byte() {
        let mut carry = unsafe { Utf8Carry::new() };
        #[cfg(target_arch = "x86_64")]
        let constants = unsafe { test_constants() };
        let input = "Emoji 🎉 test\"".as_bytes();
        #[cfg(target_arch = "x86_64")]
        let result = validate_string_chunk(input, 0, &mut carry, &constants);
        #[cfg(not(target_arch = "x86_64"))]
        let result = validate_string_chunk(input, 0, &mut carry);
        assert!(result.utf8_valid);
        assert_eq!(input[result.first_quote_or_backslash], b'"');
    }

    #[test]
    fn test_chunk_control_char() {
        let mut carry = unsafe { Utf8Carry::new() };
        #[cfg(target_arch = "x86_64")]
        let constants = unsafe { test_constants() };
        let input = b"hello\x00world";
        #[cfg(target_arch = "x86_64")]
        let result = validate_string_chunk(input, 0, &mut carry, &constants);
        #[cfg(not(target_arch = "x86_64"))]
        let result = validate_string_chunk(input, 0, &mut carry);
        // Control char at position 5 is an ERROR
        assert_eq!(result.first_control_char, 5);
        // No quote or backslash in this input
        assert_eq!(result.first_quote_or_backslash, 11);
    }

    #[test]
    fn test_chunk_backslash() {
        let mut carry = unsafe { Utf8Carry::new() };
        #[cfg(target_arch = "x86_64")]
        let constants = unsafe { test_constants() };
        let input = b"hello\\nworld";
        #[cfg(target_arch = "x86_64")]
        let result = validate_string_chunk(input, 0, &mut carry, &constants);
        #[cfg(not(target_arch = "x86_64"))]
        let result = validate_string_chunk(input, 0, &mut carry);
        // Backslash at position 5 is a valid terminator
        assert_eq!(result.first_quote_or_backslash, 5);
        // No control char in this input
        assert_eq!(result.first_control_char, 12);
    }

    #[test]
    fn test_long_ascii_string() {
        let mut carry = unsafe { Utf8Carry::new() };
        #[cfg(target_arch = "x86_64")]
        let constants = unsafe { test_constants() };
        // ASCII string longer than 64 bytes
        let input = b"This is a long ASCII string that is definitely more than sixty-four bytes long for testing!\"";

        // First chunk (64 bytes) has no terminator
        #[cfg(target_arch = "x86_64")]
        let result = validate_string_chunk(input, 0, &mut carry, &constants);
        #[cfg(not(target_arch = "x86_64"))]
        let result = validate_string_chunk(input, 0, &mut carry);
        assert!(result.utf8_valid);
        assert_eq!(result.first_quote_or_backslash, 64); // No quote in first 64 bytes
        assert_eq!(result.first_control_char, 64); // No control char

        // Second chunk (starting at 64) should find the quote
        #[cfg(target_arch = "x86_64")]
        let result2 = validate_string_chunk(input, 64, &mut carry, &constants);
        #[cfg(not(target_arch = "x86_64"))]
        let result2 = validate_string_chunk(input, 64, &mut carry);
        assert!(result2.utf8_valid);
        assert_eq!(result2.first_quote_or_backslash, 27);
        assert_eq!(input[64 + result2.first_quote_or_backslash], b'"');
    }

    #[test]
    fn test_long_utf8_string() {
        let mut carry = unsafe { Utf8Carry::new() };
        #[cfg(target_arch = "x86_64")]
        let constants = unsafe { test_constants() };
        // Mixed UTF-8 longer than 64 bytes
        let input =
            "Hello 世界 Привет мир مرحبا العالم 🌍🌎🌏 end of test string here\"".as_bytes();
        #[cfg(target_arch = "x86_64")]
        let result = validate_string_chunk(input, 0, &mut carry, &constants);
        #[cfg(not(target_arch = "x86_64"))]
        let result = validate_string_chunk(input, 0, &mut carry);
        assert!(result.utf8_valid);
    }

    #[test]
    fn test_invalid_utf8_standalone_continuation() {
        let mut carry = unsafe { Utf8Carry::new() };
        #[cfg(target_arch = "x86_64")]
        let constants = unsafe { test_constants() };
        let input = b"hello\x80world\"";
        #[cfg(target_arch = "x86_64")]
        let result = validate_string_chunk(input, 0, &mut carry, &constants);
        #[cfg(not(target_arch = "x86_64"))]
        let result = validate_string_chunk(input, 0, &mut carry);
        assert!(!result.utf8_valid);
    }

    #[test]
    fn test_invalid_utf8_overlong() {
        let mut carry = unsafe { Utf8Carry::new() };
        #[cfg(target_arch = "x86_64")]
        let constants = unsafe { test_constants() };
        let input = b"hello\xC0\x80world\"";
        #[cfg(target_arch = "x86_64")]
        let result = validate_string_chunk(input, 0, &mut carry, &constants);
        #[cfg(not(target_arch = "x86_64"))]
        let result = validate_string_chunk(input, 0, &mut carry);
        assert!(!result.utf8_valid);
    }

    #[test]
    fn test_invalid_utf8_f5_and_above() {
        let mut carry = unsafe { Utf8Carry::new() };
        #[cfg(target_arch = "x86_64")]
        let constants = unsafe { test_constants() };
        let input = b"hello\xF5\x80\x80\x80world\"";
        #[cfg(target_arch = "x86_64")]
        let result = validate_string_chunk(input, 0, &mut carry, &constants);
        #[cfg(not(target_arch = "x86_64"))]
        let result = validate_string_chunk(input, 0, &mut carry);
        assert!(!result.utf8_valid);
    }

    #[test]
    fn test_find_string_terminator_compat() {
        let input = b"hello\"world";
        assert_eq!(find_string_terminator(input, 0), Some(5));

        let input = b"hello world";
        assert_eq!(find_string_terminator(input, 0), None);
    }

    #[test]
    fn test_control_char_before_quote() {
        // Control char should be detected as an error even if quote comes later
        let mut carry = unsafe { Utf8Carry::new() };
        #[cfg(target_arch = "x86_64")]
        let constants = unsafe { test_constants() };
        let input = b"abc\x01def\"ghi";
        #[cfg(target_arch = "x86_64")]
        let result = validate_string_chunk(input, 0, &mut carry, &constants);
        #[cfg(not(target_arch = "x86_64"))]
        let result = validate_string_chunk(input, 0, &mut carry);
        assert_eq!(result.first_control_char, 3); // Control at position 3
        assert_eq!(result.first_quote_or_backslash, 7); // Quote at position 7
        assert!(result.utf8_valid);
    }

    #[test]
    fn test_quote_before_control_char() {
        // Quote before control char - both should be reported separately
        let mut carry = unsafe { Utf8Carry::new() };
        #[cfg(target_arch = "x86_64")]
        let constants = unsafe { test_constants() };
        let input = b"abc\"def\x01ghi";
        #[cfg(target_arch = "x86_64")]
        let result = validate_string_chunk(input, 0, &mut carry, &constants);
        #[cfg(not(target_arch = "x86_64"))]
        let result = validate_string_chunk(input, 0, &mut carry);
        assert_eq!(result.first_quote_or_backslash, 3); // Quote at position 3
        assert_eq!(result.first_control_char, 7); // Control at position 7
        assert!(result.utf8_valid);
    }
}
