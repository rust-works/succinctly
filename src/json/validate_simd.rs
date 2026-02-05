//! SIMD-accelerated validation for JSON validator.
//!
//! This module provides:
//! - Whole-document UTF-8 validation using the Keiser-Lemire algorithm
//! - String terminator scanning (`"`, `\`, control chars < 0x20)
//!
//! Architecture: UTF-8 is validated once for the entire document upfront,
//! then string scanning only needs to find terminators without re-validating UTF-8.
//! This is more efficient than per-string UTF-8 validation.

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

/// Result of validating a string chunk (legacy - kept for reference).
/// Now replaced by ScanResult since UTF-8 is validated for the entire document upfront.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct ChunkResult {
    /// True if UTF-8 is valid for the entire chunk.
    pub utf8_valid: bool,
    /// Offset to first control character (< 0x20), or 64 if none found.
    pub first_control_char: usize,
    /// Bitmap of all quote positions in the chunk (bit i = quote at offset i).
    pub quote_mask: u64,
    /// Bitmap of all backslash positions in the chunk (bit i = backslash at offset i).
    pub backslash_mask: u64,
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
///
/// Note: Only includes constants used on EVERY chunk (control char detection).
/// UTF-8 validation constants are created per-block to avoid cache pressure
/// on large files (benchmark showed 6-7% regression with larger struct).
#[cfg(target_arch = "x86_64")]
pub struct SimdConstants {
    // Terminator detection
    pub quote: __m256i,
    pub backslash: __m256i,

    // Keiser-Lemire UTF-8 lookup tables
    /// For each high nibble, how many continuation bytes follow (0, 1, 2, or 3).
    pub first_len_tbl: __m256i,

    // Shared constants (avoid repeated _mm256_set1_epi8 per chunk)
    pub nibble_mask: __m256i,
    /// 0x80 - used for sign flipping and high bit detection
    pub sign_flip: __m256i,
    /// 0xA0 = 0x20 ^ 0x80 - for control char detection (< 0x20)
    pub control_bound: __m256i,
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

            // Control char detection constants (used every chunk)
            nibble_mask: _mm256_set1_epi8(0x0F),
            sign_flip: _mm256_set1_epi8(0x80u8 as i8),
            control_bound: _mm256_set1_epi8((0x20u8 ^ 0x80u8) as i8), // 0xA0
        }
    }
}

/// Validate entire document as UTF-8 using SIMD.
/// Call once at the start of validation, before parsing.
/// Returns Ok(()) if valid, Err(offset) with byte offset of first invalid byte.
#[cfg(target_arch = "x86_64")]
pub fn validate_utf8_document(input: &[u8], constants: &SimdConstants) -> Result<(), usize> {
    if is_x86_feature_detected!("avx2") {
        unsafe { validate_utf8_document_avx2(input, constants) }
    } else {
        validate_utf8_document_scalar(input)
    }
}

#[cfg(not(target_arch = "x86_64"))]
pub fn validate_utf8_document(input: &[u8]) -> Result<(), usize> {
    validate_utf8_document_scalar(input)
}

/// AVX2 implementation of whole-document UTF-8 validation.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn validate_utf8_document_avx2(input: &[u8], c: &SimdConstants) -> Result<(), usize> {
    let mut offset = 0;
    let mut prev_incomplete = _mm256_setzero_si256();
    let mut prev_first_len = _mm256_setzero_si256();

    // Process 64 bytes at a time (two 32-byte AVX2 registers)
    while offset + 64 <= input.len() {
        let ptr = input.as_ptr().add(offset) as *const __m256i;
        let chunk0 = _mm256_loadu_si256(ptr);
        let chunk1 = _mm256_loadu_si256(ptr.add(1));

        let (error0, incomplete0, first_len0) =
            validate_utf8_block(chunk0, prev_incomplete, prev_first_len, c);

        let (error1, incomplete1, first_len1) =
            validate_utf8_block(chunk1, incomplete0, first_len0, c);

        // Check for errors
        let total_error = _mm256_or_si256(error0, error1);
        if _mm256_testz_si256(total_error, total_error) == 0 {
            // Find the first error position
            let err0_mask = _mm256_movemask_epi8(error0) as u32;
            let err1_mask = _mm256_movemask_epi8(error1) as u32;
            if err0_mask != 0 {
                return Err(offset + err0_mask.trailing_zeros() as usize);
            } else {
                return Err(offset + 32 + err1_mask.trailing_zeros() as usize);
            }
        }

        prev_incomplete = incomplete1;
        prev_first_len = first_len1;
        offset += 64;
    }

    // Process remaining 32-byte chunk if available
    if offset + 32 <= input.len() {
        let ptr = input.as_ptr().add(offset) as *const __m256i;
        let chunk = _mm256_loadu_si256(ptr);

        let (error, incomplete, _first_len) =
            validate_utf8_block(chunk, prev_incomplete, prev_first_len, c);

        if _mm256_testz_si256(error, error) == 0 {
            let err_mask = _mm256_movemask_epi8(error) as u32;
            return Err(offset + err_mask.trailing_zeros() as usize);
        }

        prev_incomplete = incomplete;
        // prev_first_len not needed after this point (no more chunks to process)
        offset += 32;
    }

    // Check for incomplete sequence at end
    if _mm256_testz_si256(prev_incomplete, prev_incomplete) == 0 {
        return Err(input.len());
    }

    // Validate remaining bytes with scalar
    validate_utf8_tail(&input[offset..]).map_err(|e| offset + e)
}

/// Scalar UTF-8 validation for tail bytes or non-x86_64 platforms.
fn validate_utf8_document_scalar(input: &[u8]) -> Result<(), usize> {
    let mut i = 0;
    while i < input.len() {
        let b = input[i];
        if b < 0x80 {
            i += 1;
        } else if b < 0xC2 {
            // Invalid: 0x80-0xBF are continuations, 0xC0-0xC1 are overlong
            return Err(i);
        } else if b < 0xE0 {
            // 2-byte sequence
            if i + 1 >= input.len() || (input[i + 1] & 0xC0) != 0x80 {
                return Err(i);
            }
            i += 2;
        } else if b < 0xF0 {
            // 3-byte sequence
            if i + 2 >= input.len() {
                return Err(i);
            }
            let b1 = input[i + 1];
            let b2 = input[i + 2];
            if (b1 & 0xC0) != 0x80 || (b2 & 0xC0) != 0x80 {
                return Err(i);
            }
            // Check for overlong and surrogate
            let cp = ((b as u32 & 0x0F) << 12) | ((b1 as u32 & 0x3F) << 6) | (b2 as u32 & 0x3F);
            if cp < 0x800 || (0xD800..=0xDFFF).contains(&cp) {
                return Err(i);
            }
            i += 3;
        } else if b <= 0xF4 {
            // 4-byte sequence
            if i + 3 >= input.len() {
                return Err(i);
            }
            let b1 = input[i + 1];
            let b2 = input[i + 2];
            let b3 = input[i + 3];
            if (b1 & 0xC0) != 0x80 || (b2 & 0xC0) != 0x80 || (b3 & 0xC0) != 0x80 {
                return Err(i);
            }
            // Check for overlong and out-of-range
            let cp = ((b as u32 & 0x07) << 18)
                | ((b1 as u32 & 0x3F) << 12)
                | ((b2 as u32 & 0x3F) << 6)
                | (b3 as u32 & 0x3F);
            if !(0x10000..=0x10FFFF).contains(&cp) {
                return Err(i);
            }
            i += 4;
        } else {
            // 0xF5-0xFF are invalid
            return Err(i);
        }
    }
    Ok(())
}

/// Validate tail bytes (< 32) after SIMD processing.
fn validate_utf8_tail(input: &[u8]) -> Result<(), usize> {
    validate_utf8_document_scalar(input)
}

/// Result of scanning a string chunk (no UTF-8 validation).
#[derive(Debug, Clone, Copy)]
pub struct ScanResult {
    /// Offset to first control character (< 0x20), or 64 if none found.
    pub first_control_char: usize,
    /// Bitmap of all quote positions in the chunk.
    pub quote_mask: u64,
    /// Bitmap of all backslash positions in the chunk.
    pub backslash_mask: u64,
}

/// Scan a string chunk for terminators only (UTF-8 already validated).
/// This is faster than validate_string_chunk because it skips UTF-8 validation.
#[inline]
pub fn scan_string_chunk(
    input: &[u8],
    start: usize,
    #[cfg(target_arch = "x86_64")] constants: &SimdConstants,
) -> ScanResult {
    if start >= input.len() {
        return ScanResult {
            first_control_char: 0,
            quote_mask: 0,
            backslash_mask: 0,
        };
    }

    let remaining = input.len() - start;

    #[cfg(all(target_arch = "x86_64", any(test, feature = "std")))]
    if remaining >= 64 && is_x86_feature_detected!("avx2") {
        return unsafe { scan_chunk_64_avx2(input, start, constants) };
    }

    #[cfg(target_arch = "x86_64")]
    if remaining >= 16 {
        return unsafe { scan_chunk_16_sse2(input, start) };
    }

    scan_chunk_scalar(input, start)
}

/// AVX2 string scanning - terminators only, no UTF-8.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn scan_chunk_64_avx2(input: &[u8], start: usize, c: &SimdConstants) -> ScanResult {
    let data = &input[start..];
    let ptr = data.as_ptr() as *const __m256i;

    let chunk0 = _mm256_loadu_si256(ptr);
    let chunk1 = _mm256_loadu_si256(ptr.add(1));

    // Control character detection
    let chunk0_unsigned = _mm256_xor_si256(chunk0, c.sign_flip);
    let chunk1_unsigned = _mm256_xor_si256(chunk1, c.sign_flip);
    let is_control0 = _mm256_cmpgt_epi8(c.control_bound, chunk0_unsigned);
    let is_control1 = _mm256_cmpgt_epi8(c.control_bound, chunk1_unsigned);

    let ctrl0 = _mm256_movemask_epi8(is_control0) as u32;
    let ctrl1 = _mm256_movemask_epi8(is_control1) as u32;
    let control_mask: u64 = (ctrl0 as u64) | ((ctrl1 as u64) << 32);
    let first_control_char = control_mask.trailing_zeros() as usize;

    // Quote and backslash detection
    let is_quote0 = _mm256_cmpeq_epi8(chunk0, c.quote);
    let is_backslash0 = _mm256_cmpeq_epi8(chunk0, c.backslash);
    let is_quote1 = _mm256_cmpeq_epi8(chunk1, c.quote);
    let is_backslash1 = _mm256_cmpeq_epi8(chunk1, c.backslash);

    let q0 = _mm256_movemask_epi8(is_quote0) as u32;
    let q1 = _mm256_movemask_epi8(is_quote1) as u32;
    let quote_mask: u64 = (q0 as u64) | ((q1 as u64) << 32);

    let b0 = _mm256_movemask_epi8(is_backslash0) as u32;
    let b1 = _mm256_movemask_epi8(is_backslash1) as u32;
    let backslash_mask: u64 = (b0 as u64) | ((b1 as u64) << 32);

    ScanResult {
        first_control_char,
        quote_mask,
        backslash_mask,
    }
}

/// SSE2 string scanning for 16-63 bytes.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn scan_chunk_16_sse2(input: &[u8], start: usize) -> ScanResult {
    use core::arch::x86_64::*;

    let data = &input[start..];
    let remaining = data.len().min(64);

    let quote_vec = _mm_set1_epi8(b'"' as i8);
    let backslash_vec = _mm_set1_epi8(b'\\' as i8);
    let sign_flip = _mm_set1_epi8(0x80u8 as i8);
    let control_bound = _mm_set1_epi8((0x20u8 ^ 0x80) as i8);

    let mut quote_mask: u64 = 0;
    let mut backslash_mask: u64 = 0;
    let mut first_control_char = remaining;
    let mut offset = 0;

    while offset + 16 <= remaining {
        let ptr = data.as_ptr().add(offset) as *const __m128i;
        let chunk = _mm_loadu_si128(ptr);

        let is_quote = _mm_cmpeq_epi8(chunk, quote_vec);
        let is_backslash = _mm_cmpeq_epi8(chunk, backslash_vec);
        let q = _mm_movemask_epi8(is_quote) as u16;
        let b = _mm_movemask_epi8(is_backslash) as u16;

        quote_mask |= (q as u64) << offset;
        backslash_mask |= (b as u64) << offset;

        let chunk_unsigned = _mm_xor_si128(chunk, sign_flip);
        let is_control = _mm_cmpgt_epi8(control_bound, chunk_unsigned);
        let ctrl = _mm_movemask_epi8(is_control) as u16;

        if ctrl != 0 && first_control_char == remaining {
            first_control_char = offset + ctrl.trailing_zeros() as usize;
        }

        offset += 16;
    }

    // Handle remaining bytes
    for (i, &b) in data.iter().enumerate().take(remaining).skip(offset) {
        if b == b'"' {
            quote_mask |= 1u64 << i;
        }
        if b == b'\\' {
            backslash_mask |= 1u64 << i;
        }
        if b < 0x20 && first_control_char == remaining {
            first_control_char = i;
        }
    }

    ScanResult {
        first_control_char,
        quote_mask,
        backslash_mask,
    }
}

/// Scalar string scanning fallback.
fn scan_chunk_scalar(input: &[u8], start: usize) -> ScanResult {
    let data = &input[start..];
    let chunk_len = data.len().min(64);

    let mut first_control_char = chunk_len;
    let mut quote_mask: u64 = 0;
    let mut backslash_mask: u64 = 0;

    for (i, &b) in data.iter().take(chunk_len).enumerate() {
        if b < 0x20 && first_control_char == chunk_len {
            first_control_char = i;
        }
        if b == b'"' {
            quote_mask |= 1u64 << i;
        }
        if b == b'\\' {
            backslash_mask |= 1u64 << i;
        }
    }

    ScanResult {
        first_control_char,
        quote_mask,
        backslash_mask,
    }
}

/// Validate a string chunk: find terminators and validate UTF-8 in one pass.
/// Pass pre-created SimdConstants from the caller to avoid per-chunk allocation.
///
/// Returns a `ChunkResult` with:
/// - `utf8_valid`: Whether UTF-8 is valid for the chunk
/// - `first_control_char`: Position of first control char error, or 64 if none
/// - `quote_mask`: Bitmap of all quote positions (bit i = quote at offset i)
/// - `backslash_mask`: Bitmap of all backslash positions for batch escape processing
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
            quote_mask: 0,
            backslash_mask: 0,
        };
    }

    let remaining = input.len() - start;

    // Use AVX2 for 64+ bytes
    #[cfg(all(target_arch = "x86_64", any(test, feature = "std")))]
    if remaining >= 64 && is_x86_feature_detected!("avx2") {
        return unsafe { validate_chunk_64_avx2(input, start, carry, constants) };
    }

    // Use SSE2 for 16-63 bytes (baseline x86_64, always available)
    // Only use SSE2 for ASCII content (no pending UTF-8) to keep it simple
    #[cfg(target_arch = "x86_64")]
    if remaining >= 16 && !carry.has_pending() {
        return unsafe { validate_chunk_16_sse2(input, start) };
    }

    // Scalar fallback for <16 bytes or pending UTF-8 continuations
    validate_string_chunk_scalar(input, start, carry)
}

/// AVX2 implementation - processes 64 bytes with full Keiser-Lemire UTF-8 validation.
///
/// Validates the full 64-byte chunk and returns:
/// - Control character position (errors)
/// - Quote position (string terminator)
/// - Backslash bitmap (all escape positions for batch processing)
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
    // Use precomputed constants to avoid per-chunk overhead
    let chunk0_unsigned = _mm256_xor_si256(chunk0, c.sign_flip);
    let chunk1_unsigned = _mm256_xor_si256(chunk1, c.sign_flip);
    let is_control0 = _mm256_cmpgt_epi8(c.control_bound, chunk0_unsigned);
    let is_control1 = _mm256_cmpgt_epi8(c.control_bound, chunk1_unsigned);

    let ctrl0 = _mm256_movemask_epi8(is_control0) as u32;
    let ctrl1 = _mm256_movemask_epi8(is_control1) as u32;
    let control_mask: u64 = (ctrl0 as u64) | ((ctrl1 as u64) << 32);

    // Branchless: trailing_zeros() returns 64 for u64 when all bits are zero
    let first_control_char = control_mask.trailing_zeros() as usize;

    // === QUOTE AND BACKSLASH DETECTION ===
    // Quote = string terminator, Backslash = escape sequence start
    let is_quote0 = _mm256_cmpeq_epi8(chunk0, c.quote);
    let is_backslash0 = _mm256_cmpeq_epi8(chunk0, c.backslash);
    let is_quote1 = _mm256_cmpeq_epi8(chunk1, c.quote);
    let is_backslash1 = _mm256_cmpeq_epi8(chunk1, c.backslash);

    // Build separate masks for quote and backslash
    let q0 = _mm256_movemask_epi8(is_quote0) as u32;
    let q1 = _mm256_movemask_epi8(is_quote1) as u32;
    let quote_mask: u64 = (q0 as u64) | ((q1 as u64) << 32);

    let b0 = _mm256_movemask_epi8(is_backslash0) as u32;
    let b1 = _mm256_movemask_epi8(is_backslash1) as u32;
    let backslash_mask: u64 = (b0 as u64) | ((b1 as u64) << 32);

    // Update carry state for next chunk
    carry.error = total_error;
    carry.prev_incomplete = incomplete1;
    carry.prev_first_len = first_len1;

    ChunkResult {
        utf8_valid: !has_error,
        first_control_char,
        quote_mask,
        backslash_mask,
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
    // Use precomputed sign_flip (0x80) for high bit detection
    let has_high = _mm256_and_si256(input, c.sign_flip);
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
    // Note: These constants are created inline rather than precomputed because
    // UTF-8 validation only runs for non-ASCII content, and precomputing them
    // caused 6-7% cache pressure regression on large unicode files.
    let is_continuation = _mm256_cmpeq_epi8(
        _mm256_and_si256(input, _mm256_set1_epi8(0xC0u8 as i8)),
        _mm256_set1_epi8(0x80u8 as i8),
    );

    // Check for C0, C1 (overlong 2-byte)
    let byte_eq_c0 = _mm256_cmpeq_epi8(input, _mm256_set1_epi8(0xC0u8 as i8));
    let byte_eq_c1 = _mm256_cmpeq_epi8(input, _mm256_set1_epi8(0xC1u8 as i8));
    let invalid_c0c1 = _mm256_or_si256(byte_eq_c0, byte_eq_c1);

    // Check for F5-FF (invalid)
    // Use sign_flip (precomputed) for unsigned comparison
    let input_unsigned = _mm256_xor_si256(input, c.sign_flip);
    let f4_unsigned = _mm256_set1_epi8((0xF4u8 ^ 0x80) as i8);
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

/// SSE2 implementation for 16-63 byte chunks.
/// Processes up to 4 x 16-byte chunks to match the 64-byte interface.
/// Falls back to scalar if non-ASCII bytes detected (for proper UTF-8 validation).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn validate_chunk_16_sse2(input: &[u8], start: usize) -> ChunkResult {
    use core::arch::x86_64::*;

    let data = &input[start..];
    let remaining = data.len().min(64);

    // Preload vectors once
    let quote_vec = _mm_set1_epi8(b'"' as i8);
    let backslash_vec = _mm_set1_epi8(b'\\' as i8);
    let sign_flip = _mm_set1_epi8(0x80u8 as i8);
    let control_bound = _mm_set1_epi8((0x20u8 ^ 0x80) as i8);

    let mut quote_mask: u64 = 0;
    let mut backslash_mask: u64 = 0;
    let mut first_control_char = remaining;
    let mut offset = 0;

    // Process 16-byte chunks
    while offset + 16 <= remaining {
        let ptr = data.as_ptr().add(offset) as *const __m128i;
        let chunk = _mm_loadu_si128(ptr);

        // Check for high bytes (non-ASCII)
        let has_high = _mm_and_si128(chunk, sign_flip);
        let any_high = _mm_movemask_epi8(has_high);

        if any_high != 0 {
            // Contains non-ASCII - fall back to scalar for proper UTF-8 validation
            let mut temp_carry = Utf8Carry::new();
            return validate_string_chunk_scalar(input, start, &mut temp_carry);
        }

        // Detect terminators
        let is_quote = _mm_cmpeq_epi8(chunk, quote_vec);
        let is_backslash = _mm_cmpeq_epi8(chunk, backslash_vec);
        let q = _mm_movemask_epi8(is_quote) as u16;
        let b = _mm_movemask_epi8(is_backslash) as u16;

        quote_mask |= (q as u64) << offset;
        backslash_mask |= (b as u64) << offset;

        // Control char detection
        let chunk_unsigned = _mm_xor_si128(chunk, sign_flip);
        let is_control = _mm_cmpgt_epi8(control_bound, chunk_unsigned);
        let ctrl = _mm_movemask_epi8(is_control) as u16;

        if ctrl != 0 && first_control_char == remaining {
            first_control_char = offset + ctrl.trailing_zeros() as usize;
        }

        offset += 16;
    }

    // Handle remaining bytes (0-15) with scalar
    for (i, &b) in data.iter().enumerate().take(remaining).skip(offset) {
        if b >= 0x80 {
            // Non-ASCII detected in tail - fall back to scalar
            let mut temp_carry = Utf8Carry::new();
            return validate_string_chunk_scalar(input, start, &mut temp_carry);
        }

        if b == b'"' {
            quote_mask |= 1u64 << i;
        }
        if b == b'\\' {
            backslash_mask |= 1u64 << i;
        }
        if b < 0x20 && first_control_char == remaining {
            first_control_char = i;
        }
    }

    ChunkResult {
        utf8_valid: true, // All ASCII is valid UTF-8
        first_control_char,
        quote_mask,
        backslash_mask,
    }
}

/// Scalar fallback for chunks smaller than 16 bytes or with pending UTF-8.
/// Scans the full chunk and returns control char position, quote mask, and backslash mask.
fn validate_string_chunk_scalar(input: &[u8], start: usize, carry: &mut Utf8Carry) -> ChunkResult {
    let data = &input[start..];
    let chunk_len = data.len().min(64);

    #[cfg(target_arch = "x86_64")]
    let mut pending = if carry.has_pending() { 1u8 } else { 0u8 };
    #[cfg(not(target_arch = "x86_64"))]
    let mut pending = carry.pending;

    let mut utf8_valid = true;
    let mut first_control_char = chunk_len;
    let mut quote_mask: u64 = 0;
    let mut backslash_mask: u64 = 0;

    for (i, &b) in data.iter().take(chunk_len).enumerate() {
        // Track control chars (errors)
        if b < 0x20 && first_control_char == chunk_len {
            first_control_char = i;
        }

        // Track all quotes in bitmap
        if b == b'"' {
            quote_mask |= 1u64 << i;
        }

        // Track all backslashes in bitmap
        if b == b'\\' {
            backslash_mask |= 1u64 << i;
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
        quote_mask,
        backslash_mask,
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

    // Return earliest terminator (control char, quote, or first backslash)
    let chunk_len = (input.len() - start).min(64);
    let first_quote = result.quote_mask.trailing_zeros() as usize;
    let first_backslash = result.backslash_mask.trailing_zeros() as usize;
    let terminator_offset = result
        .first_control_char
        .min(first_quote)
        .min(first_backslash);

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
        assert_eq!(result.quote_mask.trailing_zeros() as usize, 5);
        assert_eq!(result.backslash_mask, 0); // No backslash
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
        assert_eq!(result.quote_mask.trailing_zeros() as usize, 31);
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
        assert_eq!(input[result.quote_mask.trailing_zeros() as usize], b'"');
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
        assert_eq!(input[result.quote_mask.trailing_zeros() as usize], b'"');
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
        assert_eq!(input[result.quote_mask.trailing_zeros() as usize], b'"');
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
        // No quote in this input (trailing_zeros returns 64 for empty mask)
        assert_eq!(result.quote_mask, 0);
        assert_eq!(result.backslash_mask, 0);
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
        // Backslash at position 5 in bitmap
        assert_eq!(result.backslash_mask, 1u64 << 5);
        // No quote in this input
        assert_eq!(result.quote_mask, 0);
        // No control char in this input (returns chunk_len for short input)
        assert_eq!(result.first_control_char, 12);
    }

    #[test]
    fn test_chunk_multiple_backslashes() {
        let mut carry = unsafe { Utf8Carry::new() };
        #[cfg(target_arch = "x86_64")]
        let constants = unsafe { test_constants() };
        let input = b"a\\b\\c\\d\"";
        #[cfg(target_arch = "x86_64")]
        let result = validate_string_chunk(input, 0, &mut carry, &constants);
        #[cfg(not(target_arch = "x86_64"))]
        let result = validate_string_chunk(input, 0, &mut carry);
        // Backslashes at positions 1, 3, 5
        assert_eq!(
            result.backslash_mask,
            (1u64 << 1) | (1u64 << 3) | (1u64 << 5)
        );
        assert_eq!(result.quote_mask.trailing_zeros() as usize, 7);
        assert!(result.utf8_valid);
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
        assert_eq!(result.quote_mask.trailing_zeros() as usize, 64); // No quote in first 64 bytes
        assert_eq!(result.first_control_char, 64); // No control char
        assert_eq!(result.backslash_mask, 0);

        // Second chunk (starting at 64) should find the quote
        #[cfg(target_arch = "x86_64")]
        let result2 = validate_string_chunk(input, 64, &mut carry, &constants);
        #[cfg(not(target_arch = "x86_64"))]
        let result2 = validate_string_chunk(input, 64, &mut carry);
        assert!(result2.utf8_valid);
        assert_eq!(result2.quote_mask.trailing_zeros() as usize, 27);
        assert_eq!(
            input[64 + result2.quote_mask.trailing_zeros() as usize],
            b'"'
        );
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
        assert_eq!(result.quote_mask.trailing_zeros() as usize, 7); // Quote at position 7
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
        assert_eq!(result.quote_mask.trailing_zeros() as usize, 3); // Quote at position 3
        assert_eq!(result.first_control_char, 7); // Control at position 7
        assert!(result.utf8_valid);
    }
}
