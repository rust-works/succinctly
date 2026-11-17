#![allow(unsafe_code)] // x86_64 SSE2 SIMD intrinsics
//! SSE2-accelerated JSON semi-indexing for x86_64.
//!
//! Processes 16 bytes at a time using x86_64 SSE2 SIMD instructions.
//! SSE2 is universally available on all x86_64 processors.

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

use super::step::BlockMasks;
use crate::json::simple::{SemiIndex as SimpleSemiIndex, State as SimpleState};
use crate::json::standard::SemiIndex;
use crate::json::BitWriter;

/// ASCII byte constants
const DOUBLE_QUOTE: i8 = b'"' as i8;
const BACKSLASH: i8 = b'\\' as i8;
const OPEN_BRACE: i8 = b'{' as i8;
const CLOSE_BRACE: i8 = b'}' as i8;
const OPEN_BRACKET: i8 = b'[' as i8;
const CLOSE_BRACKET: i8 = b']' as i8;
const COMMA: i8 = b',' as i8;
const COLON: i8 = b':' as i8;

/// Character classification results for a 16-byte chunk.
#[derive(Debug, Clone, Copy)]
struct CharClass {
    /// Mask of bytes that are '"'
    quotes: u16,
    /// Mask of bytes that are '\'
    backslashes: u16,
    /// Mask of bytes that are '{' or '['
    opens: u16,
    /// Mask of bytes that are '}' or ']'
    closes: u16,
    /// Mask of bytes that are ',' or ':'
    delims: u16,
}

/// Classify 16 bytes at once using SSE2.
#[inline]
#[target_feature(enable = "sse2")]
#[cfg(target_arch = "x86_64")]
unsafe fn classify_chars(chunk: __m128i) -> CharClass {
    // Create comparison vectors for each character
    let v_quote = _mm_set1_epi8(DOUBLE_QUOTE);
    let v_backslash = _mm_set1_epi8(BACKSLASH);
    let v_open_brace = _mm_set1_epi8(OPEN_BRACE);
    let v_close_brace = _mm_set1_epi8(CLOSE_BRACE);
    let v_open_bracket = _mm_set1_epi8(OPEN_BRACKET);
    let v_close_bracket = _mm_set1_epi8(CLOSE_BRACKET);
    let v_comma = _mm_set1_epi8(COMMA);
    let v_colon = _mm_set1_epi8(COLON);

    // Compare and get masks using _mm_cmpeq_epi8
    let eq_quote = _mm_cmpeq_epi8(chunk, v_quote);
    let eq_backslash = _mm_cmpeq_epi8(chunk, v_backslash);
    let eq_open_brace = _mm_cmpeq_epi8(chunk, v_open_brace);
    let eq_close_brace = _mm_cmpeq_epi8(chunk, v_close_brace);
    let eq_open_bracket = _mm_cmpeq_epi8(chunk, v_open_bracket);
    let eq_close_bracket = _mm_cmpeq_epi8(chunk, v_close_bracket);
    let eq_comma = _mm_cmpeq_epi8(chunk, v_comma);
    let eq_colon = _mm_cmpeq_epi8(chunk, v_colon);

    // Combine opens and closes
    let opens = _mm_or_si128(eq_open_brace, eq_open_bracket);
    let closes = _mm_or_si128(eq_close_brace, eq_close_bracket);
    let delims = _mm_or_si128(eq_comma, eq_colon);

    // Convert to bitmasks using _mm_movemask_epi8
    CharClass {
        quotes: _mm_movemask_epi8(eq_quote) as u16,
        backslashes: _mm_movemask_epi8(eq_backslash) as u16,
        opens: _mm_movemask_epi8(opens) as u16,
        closes: _mm_movemask_epi8(closes) as u16,
        delims: _mm_movemask_epi8(delims) as u16,
    }
}

/// The SSE2 answers to the shared kernel's per-block questions: two 16-byte
/// `movemask`s each.
struct Sse2;

impl BlockMasks for Sse2 {
    #[inline]
    unsafe fn specials(block: &[u8; 32]) -> u32 {
        unsafe {
            let (quote, backslash) = (_mm_set1_epi8(DOUBLE_QUOTE), _mm_set1_epi8(BACKSLASH));
            let lo = _mm_loadu_si128(block.as_ptr().cast::<__m128i>());
            let hi = _mm_loadu_si128(block.as_ptr().add(16).cast::<__m128i>());
            let lo = _mm_or_si128(_mm_cmpeq_epi8(lo, quote), _mm_cmpeq_epi8(lo, backslash));
            let hi = _mm_or_si128(_mm_cmpeq_epi8(hi, quote), _mm_cmpeq_epi8(hi, backslash));
            join(lo, hi)
        }
    }

    #[inline]
    unsafe fn phi_masks(fields: &[u8; 32]) -> [u32; 3] {
        // SAFETY: SSE2 is part of the x86_64 baseline.
        unsafe { phi_masks_in_halves(fields) }
    }
}

/// Bits 7, 6 and 5 of each of the 32 stored fields, as the IB, BP-open and
/// BP-close masks, read as two 16-byte halves.
///
/// The AVX2 kernel answers this question with it too. On a Ryzen 9 7950X its
/// kernel ran 15-20% faster on every input that is not mostly long strings with
/// these two halves than with one 32-byte load, shift and `movemask` (#7305).
///
/// # Safety
///
/// None beyond SSE2, which is part of the x86_64 baseline; `unsafe` only because
/// the loads are raw-pointer intrinsics.
#[inline(always)]
pub(super) unsafe fn phi_masks_in_halves(fields: &[u8; 32]) -> [u32; 3] {
    unsafe {
        // `movemask` reads bit 7 of each byte, so shift the open (bit 6) and
        // close (bit 5) bits up to it; a 16-bit lane shift is fine because
        // only each byte's own lower bits move into its bit 7.
        let lo = _mm_loadu_si128(fields.as_ptr().cast::<__m128i>());
        let hi = _mm_loadu_si128(fields.as_ptr().add(16).cast::<__m128i>());
        [
            join(lo, hi),
            join(_mm_slli_epi16::<1>(lo), _mm_slli_epi16::<1>(hi)),
            join(_mm_slli_epi16::<2>(lo), _mm_slli_epi16::<2>(hi)),
        ]
    }
}

/// Bit 7 of each byte of `lo` then `hi`, as one 32-bit mask.
#[inline(always)]
fn join(lo: __m128i, hi: __m128i) -> u32 {
    // SAFETY: SSE2 is part of the x86_64 baseline.
    unsafe { _mm_movemask_epi8(lo) as u32 | (_mm_movemask_epi8(hi) as u32) << 16 }
}

/// Build a semi-index from JSON bytes using SIMD-accelerated Standard Cursor algorithm.
///
/// The kernel is `json::simd::step`'s, shared with the AVX2 builder: one
/// `STEP_TABLE` load and one shift per byte with no branch on the input, each
/// 32-byte block's IB, BP-open and BP-close bits pulled out of the stored fields
/// with two 16-byte `movemask`s apiece, and a block that lies wholly inside a
/// string skipped. It replaced a per-byte `match` over 16-byte classification
/// masks (#7305).
#[cfg(target_arch = "x86_64")]
pub fn build_semi_index_standard(json: &[u8]) -> SemiIndex {
    // SAFETY: SSE2 is guaranteed to be available on all x86_64 processors
    unsafe { super::step::build_semi_index_standard::<Sse2>(json) }
}

// ============================================================================
// Simple Cursor SIMD Implementation
// ============================================================================

/// Process a 16-byte chunk for simple cursor and update IB/BP writers.
/// Returns the new state after processing all bytes.
///
/// Simple cursor differences from standard:
/// - 3 states: InJson, InString, InEscape (no InValue)
/// - IB marks all structural chars: { } [ ] : ,
/// - BP encoding: { or [ → 11, } or ] → 00, : or , → 01
#[inline]
fn process_chunk_simple(
    class: CharClass,
    mut state: SimpleState,
    ib: &mut BitWriter,
    bp: &mut BitWriter,
    bytes: &[u8],
) -> SimpleState {
    for i in 0..bytes.len().min(16) {
        let bit = 1u16 << i;

        let is_quote = (class.quotes & bit) != 0;
        let is_backslash = (class.backslashes & bit) != 0;
        let is_open = (class.opens & bit) != 0;
        let is_close = (class.closes & bit) != 0;
        let is_delim = (class.delims & bit) != 0;

        match state {
            SimpleState::InJson => {
                if is_open {
                    // { or [: write BP=11, IB=1
                    bp.write_1();
                    bp.write_1();
                    ib.write_1();
                } else if is_close {
                    // } or ]: write BP=00, IB=1
                    bp.write_0();
                    bp.write_0();
                    ib.write_1();
                } else if is_delim {
                    // , or :: write BP=01, IB=1
                    bp.write_0();
                    bp.write_1();
                    ib.write_1();
                } else if is_quote {
                    // Start of string: IB=0, transition to InString
                    ib.write_0();
                    state = SimpleState::InString;
                } else {
                    // Whitespace or other: IB=0
                    ib.write_0();
                }
            }
            SimpleState::InString => {
                // Always IB=0 inside strings
                ib.write_0();
                if is_quote {
                    state = SimpleState::InJson;
                } else if is_backslash {
                    state = SimpleState::InEscape;
                }
            }
            SimpleState::InEscape => {
                // Escaped character: IB=0, return to InString
                ib.write_0();
                state = SimpleState::InString;
            }
        }
    }

    state
}

/// Build a semi-index from JSON bytes using SIMD-accelerated Simple Cursor algorithm.
///
/// Processes 16 bytes at a time using x86_64 SSE2 instructions for character
/// classification, then processes the state machine transitions.
#[cfg(target_arch = "x86_64")]
pub fn build_semi_index_simple(json: &[u8]) -> SimpleSemiIndex {
    // SAFETY: SSE2 is guaranteed to be available on all x86_64 processors
    unsafe { build_semi_index_simple_sse2(json) }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn build_semi_index_simple_sse2(json: &[u8]) -> SimpleSemiIndex {
    unsafe {
        let word_capacity = json.len().div_ceil(64);
        let mut ib = BitWriter::with_capacity(word_capacity);
        let mut bp = BitWriter::with_capacity(word_capacity * 2);
        let mut state = SimpleState::InJson;

        let mut offset = 0;

        // Process 16-byte chunks
        while offset + 16 <= json.len() {
            let chunk = _mm_loadu_si128(json.as_ptr().add(offset).cast::<__m128i>());
            let class = classify_chars(chunk);
            state =
                process_chunk_simple(class, state, &mut ib, &mut bp, &json[offset..offset + 16]);
            offset += 16;
        }

        // Process remaining bytes (less than 16)
        if offset < json.len() {
            // Pad with zeros and process
            let mut padded = [0u8; 16];
            let remaining = json.len() - offset;
            padded[..remaining].copy_from_slice(&json[offset..]);

            let chunk = _mm_loadu_si128(padded.as_ptr().cast::<__m128i>());
            let class = classify_chars(chunk);
            state = process_chunk_simple(class, state, &mut ib, &mut bp, &json[offset..]);
        }

        SimpleSemiIndex {
            state,
            ib: ib.finish(),
            bp: bp.finish(),
        }
    }
}

#[cfg(all(test, target_arch = "x86_64"))]
mod tests {
    use super::*;

    /// Extract bit at position i from a word vector (LSB-first within each word)
    fn get_bit(words: &[u64], i: usize) -> bool {
        let word_idx = i / 64;
        let bit_idx = i % 64;
        if word_idx < words.len() {
            (words[word_idx] >> bit_idx) & 1 == 1
        } else {
            false
        }
    }

    /// Convert bit vector to string of '0' and '1' for first n bits
    fn bits_to_string(words: &[u64], n: usize) -> String {
        (0..n)
            .map(|i| if get_bit(words, i) { '1' } else { '0' })
            .collect()
    }

    #[test]
    fn test_classify_chars() {
        unsafe {
            let input = br#"{"hello":123}   "#;
            let chunk = _mm_loadu_si128(input.as_ptr().cast::<__m128i>());
            let class = classify_chars(chunk);

            // Position 0 is '{', position 12 is '}'
            assert_ne!(class.opens & (1 << 0), 0, "'{{' at position 0");
            assert_ne!(class.closes & (1 << 12), 0, "'}}' at position 12");

            // Position 1 and 7 are '"'
            assert_ne!(class.quotes & (1 << 1), 0, "'\"' at position 1");
            assert_ne!(class.quotes & (1 << 7), 0, "'\"' at position 7");

            // Position 8 is ':'
            assert_ne!(class.delims & (1 << 8), 0, "':' at position 8");
        }
    }

    /// The shared kernel's differential suite, over the SSE2 builder (#7305).
    #[test]
    fn test_sse2_standard_kernel_suite() {
        crate::json::simd::step::suite::all(build_semi_index_standard);
    }

    /// SSE2's answers to the shared kernel's per-block questions are the portable
    /// reference's (#7305).
    #[test]
    fn test_sse2_block_masks_agree_with_portable() {
        // SAFETY: SSE2 is part of the x86_64 baseline.
        unsafe { crate::json::simd::step::suite::masks_agree::<Sse2>() };
    }

    #[test]
    fn test_sse2_matches_scalar_empty_object() {
        let json = b"{}";
        let simd_result = build_semi_index_standard(json);
        let scalar_result = crate::json::standard::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len())
        );
        assert_eq!(simd_result.state, scalar_result.state);
    }

    #[test]
    fn test_sse2_matches_scalar_simple_object() {
        let json = br#"{"a":"b"}"#;
        let simd_result = build_semi_index_standard(json);
        let scalar_result = crate::json::standard::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch"
        );
        assert_eq!(simd_result.state, scalar_result.state);
    }

    #[test]
    fn test_sse2_matches_scalar_array() {
        let json = b"[1,2,3]";
        let simd_result = build_semi_index_standard(json);
        let scalar_result = crate::json::standard::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch"
        );
    }

    #[test]
    fn test_sse2_matches_scalar_nested() {
        let json = br#"{"a":{"b":1}}"#;
        let simd_result = build_semi_index_standard(json);
        let scalar_result = crate::json::standard::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch"
        );
    }

    #[test]
    fn test_sse2_matches_scalar_escaped() {
        let json = br#"{"a":"b\"c"}"#;
        let simd_result = build_semi_index_standard(json);
        let scalar_result = crate::json::standard::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch"
        );
    }

    #[test]
    fn test_sse2_matches_scalar_long_input() {
        // Test with input > 16 bytes to exercise chunk processing
        let json = br#"{"name":"value","number":12345,"array":[1,2,3]}"#;
        let simd_result = build_semi_index_standard(json);
        let scalar_result = crate::json::standard::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch for long input"
        );
        assert_eq!(simd_result.state, scalar_result.state);
    }

    #[test]
    fn test_sse2_matches_scalar_booleans() {
        let json = br#"{"t":true,"f":false,"n":null}"#;
        let simd_result = build_semi_index_standard(json);
        let scalar_result = crate::json::standard::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch for booleans"
        );
    }

    #[test]
    fn test_sse2_matches_scalar_whitespace() {
        let json = b"{ \"a\" : 1 }";
        let simd_result = build_semi_index_standard(json);
        let scalar_result = crate::json::standard::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch for whitespace"
        );
    }

    #[test]
    fn test_sse2_matches_scalar_exact_16_bytes() {
        // Exactly 16 bytes - one full chunk
        let json = br#"{"abc":"defghi"}"#; // 16 bytes
        let simd_result = build_semi_index_standard(json);
        let scalar_result = crate::json::standard::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch for 16-byte input"
        );
    }

    #[test]
    fn test_sse2_matches_scalar_32_bytes() {
        // 32 bytes - two full chunks
        let json = br#"{"abcdefghij":"klmnopqrst"}"#;
        let simd_result = build_semi_index_standard(json);
        let scalar_result = crate::json::standard::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch for 32-byte input"
        );
    }

    #[test]
    fn test_sse2_matches_scalar_negative_numbers() {
        let json = b"[-1,-2.5,-3e10]";
        let simd_result = build_semi_index_standard(json);
        let scalar_result = crate::json::standard::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch for negative numbers"
        );
    }

    #[test]
    fn test_sse2_matches_scalar_scientific_notation() {
        let json = b"[1e10,2E-5,3.14e+2]";
        let simd_result = build_semi_index_standard(json);
        let scalar_result = crate::json::standard::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch for scientific notation"
        );
    }

    // ========================================================================
    // Simple Cursor SIMD Tests
    // ========================================================================

    #[test]
    fn test_simple_sse2_matches_scalar_empty_object() {
        let json = b"{}";
        let simd_result = build_semi_index_simple(json);
        let scalar_result = crate::json::simple::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch"
        );
        assert_eq!(
            bits_to_string(&simd_result.bp, 4),
            bits_to_string(&scalar_result.bp, 4),
            "BP mismatch"
        );
        assert_eq!(simd_result.state, scalar_result.state);
    }

    #[test]
    fn test_simple_sse2_matches_scalar_empty_array() {
        let json = b"[]";
        let simd_result = build_semi_index_simple(json);
        let scalar_result = crate::json::simple::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch"
        );
        assert_eq!(
            bits_to_string(&simd_result.bp, 4),
            bits_to_string(&scalar_result.bp, 4),
            "BP mismatch"
        );
    }

    #[test]
    fn test_simple_sse2_matches_scalar_simple_object() {
        let json = br#"{"a":"b"}"#;
        let simd_result = build_semi_index_simple(json);
        let scalar_result = crate::json::simple::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch"
        );
        // BP for simple: { → 11, : → 01, } → 00 = 110100
        assert_eq!(
            bits_to_string(&simd_result.bp, 6),
            bits_to_string(&scalar_result.bp, 6),
            "BP mismatch"
        );
    }

    #[test]
    fn test_simple_sse2_matches_scalar_array_with_values() {
        let json = b"[1,2,3]";
        let simd_result = build_semi_index_simple(json);
        let scalar_result = crate::json::simple::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch"
        );
        // BP for simple: [ → 11, , → 01, , → 01, ] → 00 = 11010100
        assert_eq!(
            bits_to_string(&simd_result.bp, 8),
            bits_to_string(&scalar_result.bp, 8),
            "BP mismatch"
        );
    }

    #[test]
    fn test_simple_sse2_matches_scalar_nested() {
        let json = br#"{"a":{"b":1}}"#;
        let simd_result = build_semi_index_simple(json);
        let scalar_result = crate::json::simple::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch"
        );
    }

    #[test]
    fn test_simple_sse2_matches_scalar_escaped() {
        let json = br#"{"a":"b\"c"}"#;
        let simd_result = build_semi_index_simple(json);
        let scalar_result = crate::json::simple::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch"
        );
        assert_eq!(simd_result.state, scalar_result.state);
    }

    #[test]
    fn test_simple_sse2_matches_scalar_escaped_backslash() {
        let json = br#""a\\b""#;
        let simd_result = build_semi_index_simple(json);
        let scalar_result = crate::json::simple::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch"
        );
        assert_eq!(simd_result.state, scalar_result.state);
    }

    #[test]
    fn test_simple_sse2_matches_scalar_long_input() {
        // Test with input > 16 bytes to exercise chunk processing
        let json = br#"{"name":"value","number":12345,"array":[1,2,3]}"#;
        let simd_result = build_semi_index_simple(json);
        let scalar_result = crate::json::simple::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch for long input"
        );
        assert_eq!(simd_result.state, scalar_result.state);
    }

    #[test]
    fn test_simple_sse2_matches_scalar_whitespace() {
        let json = b"{ \"a\" : 1 }";
        let simd_result = build_semi_index_simple(json);
        let scalar_result = crate::json::simple::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch for whitespace"
        );
    }

    #[test]
    fn test_simple_sse2_matches_scalar_exact_16_bytes() {
        // Exactly 16 bytes - one full chunk
        let json = br#"{"abc":"defghi"}"#; // 16 bytes
        let simd_result = build_semi_index_simple(json);
        let scalar_result = crate::json::simple::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch for 16-byte input"
        );
    }

    #[test]
    fn test_simple_sse2_matches_scalar_32_bytes() {
        // 32 bytes - two full chunks
        let json = br#"{"abcdefghij":"klmnopqrst"}"#;
        let simd_result = build_semi_index_simple(json);
        let scalar_result = crate::json::simple::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch for 32-byte input"
        );
    }

    #[test]
    fn test_simple_sse2_matches_scalar_unterminated_string() {
        let json = br#"{"a"#;
        let simd_result = build_semi_index_simple(json);
        let scalar_result = crate::json::simple::build_semi_index(json);

        assert_eq!(simd_result.state, scalar_result.state);
        assert_eq!(simd_result.state, SimpleState::InString);
    }

    #[test]
    fn test_simple_sse2_matches_scalar_unterminated_escape() {
        // String ending with a single backslash: "\ (2 bytes: quote, backslash)
        let json = br#""\"#;
        let simd_result = build_semi_index_simple(json);
        let scalar_result = crate::json::simple::build_semi_index(json);

        assert_eq!(simd_result.state, scalar_result.state);
        assert_eq!(simd_result.state, SimpleState::InEscape);
    }
}
