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
#[derive(Debug, Clone, Copy)]
pub struct ChunkResult {
    /// Offset to first terminator (relative to chunk start), or chunk size if none found.
    pub terminator_offset: usize,
    /// True if UTF-8 is valid up to the terminator (or entire chunk if no terminator).
    pub utf8_valid: bool,
    /// Error state from UTF-8 validation (non-zero means error detected).
    pub error: u32,
    /// Previous block state for cross-chunk validation.
    pub prev_incomplete: __m256i,
    /// Previous block state (high bytes).
    pub prev_first_len: __m256i,
}

#[cfg(not(target_arch = "x86_64"))]
#[derive(Debug, Clone, Copy)]
pub struct ChunkResult {
    pub terminator_offset: usize,
    pub utf8_valid: bool,
    pub error: u32,
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

    pub fn has_error(&self) -> bool {
        unsafe { _mm256_testz_si256(self.error, self.error) == 0 }
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

// Compatibility alias
pub type Utf8State = Utf8Carry;

/// SIMD constant vectors for Keiser-Lemire algorithm.
/// Create once at document start and reuse for all chunks.
#[cfg(target_arch = "x86_64")]
pub struct SimdConstants {
    // Terminator detection
    pub quote: __m256i,
    pub backslash: __m256i,
    pub control_bound: __m256i,

    // Keiser-Lemire UTF-8 lookup tables
    /// For each high nibble, how many continuation bytes follow (0, 1, 2, or 3).
    pub first_len_tbl: __m256i,
    /// For each high nibble, the "range" index for validating the second byte.
    pub first_range_tbl: __m256i,
    /// For adjusting the range based on low nibble of first byte.
    pub range_adjust_tbl: __m256i,
    /// Minimum valid values for second byte (indexed by range).
    pub range_min_tbl: __m256i,
    /// Maximum valid values for second byte (indexed by range, as signed comparison).
    pub range_max_tbl: __m256i,

    // Useful constants
    pub nibble_mask: __m256i,
    pub continuation_mask: __m256i,
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
            control_bound: _mm256_set1_epi8(0x20),

            // Keiser-Lemire: first_len_tbl
            // Maps high nibble (0-F) to number of continuation bytes expected.
            // Nibbles 0-7: ASCII (0 continuations)
            // Nibbles 8-B: continuation bytes (0, but these shouldn't start a sequence)
            // Nibble C-D: 2-byte sequences (1 continuation)
            // Nibble E: 3-byte sequences (2 continuations)
            // Nibble F: 4-byte sequences (3 continuations)
            first_len_tbl: _mm256_setr_epi8(
                0, 0, 0, 0, 0, 0, 0, 0,  // 0x0-0x7: ASCII
                0, 0, 0, 0,              // 0x8-0xB: continuations
                1, 1,                     // 0xC-0xD: 2-byte
                2,                        // 0xE: 3-byte
                3,                        // 0xF: 4-byte
                // Repeat for high lane
                0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0,
                1, 1,
                2,
                3,
            ),

            // first_range_tbl: Maps high nibble to range index (0-7).
            // Used to look up valid second byte range.
            first_range_tbl: _mm256_setr_epi8(
                0, 0, 0, 0, 0, 0, 0, 0,  // ASCII: range 0
                0, 0, 0, 0,              // Continuations: range 0
                0, 0,                     // 2-byte (C0-DF): range 0 (adjusted for C0-C1)
                0,                        // 3-byte (E0-EF): range 0 (adjusted for E0, ED)
                0,                        // 4-byte (F0-FF): range 0 (adjusted for F0, F4)
                // Repeat
                0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0,
                0, 0,
                0,
                0,
            ),

            // range_adjust_tbl: Adjusts range based on low nibble of first byte.
            // This handles special cases: E0->3, ED->3, F0->3, F4->4
            range_adjust_tbl: _mm256_setr_epi8(
                // For 3-byte (E0-EF), indexed by low nibble:
                // E0: need range 3 (second byte A0-BF to avoid overlong)
                // E1-EC: need range 0 (second byte 80-BF)
                // ED: need range 4 (second byte 80-9F to avoid surrogates)
                // EE-EF: need range 0 (second byte 80-BF)
                3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 0, 0,
                // For 4-byte (F0-F7), indexed by low nibble:
                // F0: need range 3 (second byte 90-BF to avoid overlong)
                // F1-F3: need range 0 (second byte 80-BF)
                // F4: need range 4 (second byte 80-8F to avoid > U+10FFFF)
                3, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ),

            // range_min_tbl: Minimum valid second byte for each range (unsigned).
            // Range 0: 0x80 (normal continuation)
            // Range 3: 0x90 (after E0) or 0xA0 (after F0) to avoid overlong
            // Range 4: 0x80 (after ED) or 0x80 (after F4)
            range_min_tbl: _mm256_setr_epi8(
                0x80u8 as i8, 0x80u8 as i8, 0x80u8 as i8, 0xA0u8 as i8,
                0x80u8 as i8, 0x80u8 as i8, 0x80u8 as i8, 0x80u8 as i8,
                0, 0, 0, 0, 0, 0, 0, 0,
                // Repeat
                0x80u8 as i8, 0x80u8 as i8, 0x80u8 as i8, 0xA0u8 as i8,
                0x80u8 as i8, 0x80u8 as i8, 0x80u8 as i8, 0x80u8 as i8,
                0, 0, 0, 0, 0, 0, 0, 0,
            ),

            // range_max_tbl: Maximum valid second byte for each range.
            // Stored as signed for use with _mm256_cmpgt_epi8.
            range_max_tbl: _mm256_setr_epi8(
                0xBFu8 as i8, 0xBFu8 as i8, 0xBFu8 as i8, 0xBFu8 as i8,
                0x9Fu8 as i8, 0xBFu8 as i8, 0xBFu8 as i8, 0x8Fu8 as i8,
                0, 0, 0, 0, 0, 0, 0, 0,
                // Repeat
                0xBFu8 as i8, 0xBFu8 as i8, 0xBFu8 as i8, 0xBFu8 as i8,
                0x9Fu8 as i8, 0xBFu8 as i8, 0xBFu8 as i8, 0x8Fu8 as i8,
                0, 0, 0, 0, 0, 0, 0, 0,
            ),

            nibble_mask: _mm256_set1_epi8(0x0F),
            continuation_mask: _mm256_set1_epi8(0xC0u8 as i8),
        }
    }
}

/// Validate a string chunk: find terminators and validate UTF-8 in one pass.
/// Pass pre-created SimdConstants from the caller to avoid per-chunk allocation.
#[inline]
pub fn validate_string_chunk(
    input: &[u8],
    start: usize,
    carry: &mut Utf8Carry,
    #[cfg(target_arch = "x86_64")] constants: &SimdConstants,
) -> ChunkResult {
    if start >= input.len() {
        return ChunkResult {
            terminator_offset: 0,
            utf8_valid: true,
            #[cfg(target_arch = "x86_64")]
            error: 0,
            #[cfg(target_arch = "x86_64")]
            prev_incomplete: unsafe { _mm256_setzero_si256() },
            #[cfg(target_arch = "x86_64")]
            prev_first_len: unsafe { _mm256_setzero_si256() },
            #[cfg(not(target_arch = "x86_64"))]
            error: 0,
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
/// Terminators (", \, control chars) are valid ASCII, so we validate the full chunk
/// and return terminator position separately.
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
    // Always validate the full 64-byte chunk - terminators are valid ASCII
    let (error0, incomplete0, first_len0) = validate_utf8_block(
        chunk0,
        carry.prev_incomplete,
        carry.prev_first_len,
        c,
    );

    let (error1, incomplete1, first_len1) = validate_utf8_block(
        chunk1,
        incomplete0,
        first_len0,
        c,
    );

    // Accumulate errors
    let total_error = _mm256_or_si256(_mm256_or_si256(carry.error, error0), error1);
    let has_error = _mm256_testz_si256(total_error, total_error) == 0;

    // === TERMINATOR DETECTION ===
    let term0 = find_terminators_avx2(chunk0, c);
    let term1 = find_terminators_avx2(chunk1, c);
    let term_mask: u64 = (term0 as u64) | ((term1 as u64) << 32);

    let terminator_offset = if term_mask != 0 {
        term_mask.trailing_zeros() as usize
    } else {
        64
    };

    // Update carry state for next chunk
    carry.error = total_error;
    carry.prev_incomplete = incomplete1;
    carry.prev_first_len = first_len1;

    ChunkResult {
        terminator_offset,
        utf8_valid: !has_error,
        error: if has_error { 1 } else { 0 },
        prev_incomplete: incomplete1,
        prev_first_len: first_len1,
    }
}

/// Find terminators (", \, control chars) in a 256-bit chunk.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
unsafe fn find_terminators_avx2(chunk: __m256i, c: &SimdConstants) -> u32 {
    let is_quote = _mm256_cmpeq_epi8(chunk, c.quote);
    let is_backslash = _mm256_cmpeq_epi8(chunk, c.backslash);
    // Control char detection needs unsigned comparison (bytes < 0x20)
    // Use XOR with 0x80 to convert to unsigned-like comparison
    let sign_flip = _mm256_set1_epi8(0x80u8 as i8);
    let chunk_unsigned = _mm256_xor_si256(chunk, sign_flip);
    let bound_unsigned = _mm256_set1_epi8((0x20u8 ^ 0x80u8) as i8);  // 0xA0
    let is_control = _mm256_cmpgt_epi8(bound_unsigned, chunk_unsigned);
    let terminators = _mm256_or_si256(_mm256_or_si256(is_quote, is_backslash), is_control);
    _mm256_movemask_epi8(terminators) as u32
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
    let shifted = _mm256_alignr_epi8(input, prev_high_input_low, 15);

    shifted
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
        return (_mm256_setzero_si256(), _mm256_setzero_si256(), _mm256_setzero_si256());
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
    let f4_unsigned = _mm256_set1_epi8((0xF4u8 ^ 0x80u8) as i8);  // 0x74
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
    let expect_cont = _mm256_or_si256(
        expect_cont_1,
        _mm256_or_si256(expect_cont_2, expect_cont_3),
    );

    // Error: is_continuation XOR expect_cont
    // (unexpected continuation OR missing continuation)
    let cont_error = _mm256_xor_si256(is_continuation, expect_cont);

    // === Combine all errors ===
    let error = _mm256_or_si256(
        _mm256_or_si256(invalid_c0c1, byte_gt_f4),
        cont_error,
    );

    // === Calculate incomplete state for next block ===
    // Check last 3 bytes for sequences that continue into next block
    let mut first_len_bytes = [0u8; 32];
    _mm256_storeu_si256(first_len_bytes.as_mut_ptr() as *mut __m256i, first_len);

    let mut incomplete_count = 0u8;
    // Check if last byte starts a multi-byte sequence
    if first_len_bytes[31] >= 1 {
        incomplete_count = first_len_bytes[31];
    } else if first_len_bytes[30] >= 2 {
        incomplete_count = first_len_bytes[30] - 1;
    } else if first_len_bytes[29] >= 3 {
        incomplete_count = first_len_bytes[29] - 2;
    }

    let next_incomplete = if incomplete_count > 0 {
        _mm256_set1_epi8(incomplete_count as i8)
    } else {
        _mm256_setzero_si256()
    };

    (error, next_incomplete, first_len)
}

/// Scalar UTF-8 state for fallback.
struct ScalarUtf8State {
    pending: u8,
}

/// Scalar UTF-8 validation for partial chunks.
fn validate_utf8_scalar_slice(data: &[u8], state: &mut ScalarUtf8State) -> bool {
    let mut pending = state.pending;

    for &b in data {
        if pending > 0 {
            if (b & 0xC0) == 0x80 {
                pending -= 1;
            } else {
                return false;
            }
        } else if b >= 0x80 {
            if b < 0xC2 {
                return false; // Unexpected continuation or overlong
            } else if b < 0xE0 {
                pending = 1;
            } else if b < 0xF0 {
                pending = 2;
            } else if b <= 0xF4 {
                pending = 3;
            } else {
                return false; // Invalid byte
            }
        }
    }

    state.pending = pending;
    true
}

/// Scalar fallback for chunks smaller than 64 bytes.
fn validate_string_chunk_scalar(
    input: &[u8],
    start: usize,
    carry: &mut Utf8Carry,
) -> ChunkResult {
    let data = &input[start..];

    #[cfg(target_arch = "x86_64")]
    let mut pending = if carry.has_pending() { 1u8 } else { 0u8 };
    #[cfg(not(target_arch = "x86_64"))]
    let mut pending = carry.pending;

    let mut utf8_valid = true;

    for (i, &b) in data.iter().enumerate() {
        if b == b'"' || b == b'\\' || b < 0x20 {
            #[cfg(target_arch = "x86_64")]
            {
                return ChunkResult {
                    terminator_offset: i,
                    utf8_valid,
                    error: if utf8_valid { 0 } else { 1 },
                    prev_incomplete: unsafe { _mm256_setzero_si256() },
                    prev_first_len: unsafe { _mm256_setzero_si256() },
                };
            }
            #[cfg(not(target_arch = "x86_64"))]
            {
                carry.pending = pending;
                return ChunkResult {
                    terminator_offset: i,
                    utf8_valid,
                    error: if utf8_valid { 0 } else { 1 },
                };
            }
        }

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

    #[cfg(target_arch = "x86_64")]
    {
        ChunkResult {
            terminator_offset: data.len(),
            utf8_valid,
            error: if utf8_valid { 0 } else { 1 },
            prev_incomplete: unsafe { _mm256_setzero_si256() },
            prev_first_len: unsafe { _mm256_setzero_si256() },
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        carry.pending = pending;
        ChunkResult {
            terminator_offset: data.len(),
            utf8_valid,
            error: if utf8_valid { 0 } else { 1 },
        }
    }
}

// Compatibility wrapper
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

    if result.terminator_offset < input.len() - start {
        Some(result.terminator_offset)
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
        assert_eq!(result.terminator_offset, 5);
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
        assert_eq!(result.terminator_offset, 31);
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
        assert_eq!(input[result.terminator_offset], b'"');
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
        assert_eq!(input[result.terminator_offset], b'"');
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
        assert_eq!(input[result.terminator_offset], b'"');
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
        assert_eq!(result.terminator_offset, 5);
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
        assert_eq!(result.terminator_offset, 5);
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
        assert_eq!(result.terminator_offset, 64);

        // Second chunk (starting at 64) should find the quote
        #[cfg(target_arch = "x86_64")]
        let result2 = validate_string_chunk(input, 64, &mut carry, &constants);
        #[cfg(not(target_arch = "x86_64"))]
        let result2 = validate_string_chunk(input, 64, &mut carry);
        assert!(result2.utf8_valid);
        assert_eq!(result2.terminator_offset, 27);
        assert_eq!(input[64 + result2.terminator_offset], b'"');
    }

    #[test]
    fn test_long_utf8_string() {
        let mut carry = unsafe { Utf8Carry::new() };
        #[cfg(target_arch = "x86_64")]
        let constants = unsafe { test_constants() };
        // Mixed UTF-8 longer than 64 bytes
        let input = "Hello 世界 Привет мир مرحبا العالم 🌍🌎🌏 end of test string here\"".as_bytes();
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
}
