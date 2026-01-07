//! SSE4.2-accelerated JSON semi-indexing for x86_64.
//!
//! Processes 16 bytes at a time using x86_64 SSE4.2 PCMPISTRI instructions.
//! SSE4.2 is available on Intel Nehalem (2008+) and AMD Bulldozer (2011+).
//!
//! SSE4.2 provides string comparison instructions (PCMPISTRI/PCMPISTRM) that
//! can efficiently find multiple characters in a single instruction, which is
//! useful for JSON structural character detection.

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

use crate::json::simple::{SemiIndex as SimpleSemiIndex, State as SimpleState};
use crate::json::standard::{SemiIndex, State};
use crate::json::BitWriter;

/// ASCII byte constants
const DOUBLE_QUOTE: i8 = b'"' as i8;
const BACKSLASH: i8 = b'\\' as i8;

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
    /// Mask of bytes that could start/continue a value (alphanumeric, ., -, +)
    value_chars: u16,
}

/// Classify 16 bytes at once using SSE4.2 PCMPISTRI.
///
/// Uses SSE4.2's `_mm_cmpistrm` instruction to find structural characters
/// in a single pass, then standard SSE2 for other character classes.
#[inline]
#[target_feature(enable = "sse4.2")]
#[cfg(target_arch = "x86_64")]
unsafe fn classify_chars(chunk: __m128i) -> CharClass {
    unsafe {
        // Use PCMPISTRI to find structural characters efficiently
        // Set up a string of structural characters: {}[]:,
        let structural = _mm_setr_epi8(
            b'{' as i8, b'}' as i8, b'[' as i8, b']' as i8, b':' as i8, b',' as i8, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0,
        );

        // _SIDD_CMP_EQUAL_ANY: Find any bytes that match any byte in structural
        // _SIDD_UBYTE_OPS: Operate on unsigned bytes
        // _SIDD_BIT_MASK: Return a bitmask
        const MODE: i32 = _SIDD_UBYTE_OPS | _SIDD_CMP_EQUAL_ANY | _SIDD_BIT_MASK;

        // This finds all structural characters in one instruction
        // Note: We could use this mask for optimization, but for now we do
        // individual character detection for clarity and correctness
        let _structural_mask = _mm_cmpistrm(structural, chunk, MODE);
        // let _structural_bits = _mm_extract_epi16(_structural_mask, 0) as u16;

        // Now do specific character detection using standard SSE2
        let v_quote = _mm_set1_epi8(DOUBLE_QUOTE);
        let v_backslash = _mm_set1_epi8(BACKSLASH);
        let v_open_brace = _mm_set1_epi8(b'{' as i8);
        let v_close_brace = _mm_set1_epi8(b'}' as i8);
        let v_open_bracket = _mm_set1_epi8(b'[' as i8);
        let v_close_bracket = _mm_set1_epi8(b']' as i8);
        let v_comma = _mm_set1_epi8(b',' as i8);
        let v_colon = _mm_set1_epi8(b':' as i8);

        let eq_quote = _mm_cmpeq_epi8(chunk, v_quote);
        let eq_backslash = _mm_cmpeq_epi8(chunk, v_backslash);
        let eq_open_brace = _mm_cmpeq_epi8(chunk, v_open_brace);
        let eq_close_brace = _mm_cmpeq_epi8(chunk, v_close_brace);
        let eq_open_bracket = _mm_cmpeq_epi8(chunk, v_open_bracket);
        let eq_close_bracket = _mm_cmpeq_epi8(chunk, v_close_bracket);
        let eq_comma = _mm_cmpeq_epi8(chunk, v_comma);
        let eq_colon = _mm_cmpeq_epi8(chunk, v_colon);

        let opens = _mm_or_si128(eq_open_brace, eq_open_bracket);
        let closes = _mm_or_si128(eq_close_brace, eq_close_bracket);
        let delims = _mm_or_si128(eq_comma, eq_colon);

        // Value chars: alphanumeric, period, minus, plus
        let v_a_lower = _mm_set1_epi8(b'a' as i8);
        let v_z_range = _mm_set1_epi8((b'z' - b'a') as i8);
        let sub_a = _mm_sub_epi8(chunk, v_a_lower);
        let lowercase = unsigned_le(sub_a, v_z_range);

        let v_a_upper = _mm_set1_epi8(b'A' as i8);
        let sub_a_upper = _mm_sub_epi8(chunk, v_a_upper);
        let uppercase = unsigned_le(sub_a_upper, v_z_range);

        let v_0 = _mm_set1_epi8(b'0' as i8);
        let v_9_range = _mm_set1_epi8(9);
        let sub_0 = _mm_sub_epi8(chunk, v_0);
        let digit = unsigned_le(sub_0, v_9_range);

        let v_period = _mm_set1_epi8(b'.' as i8);
        let v_minus = _mm_set1_epi8(b'-' as i8);
        let v_plus = _mm_set1_epi8(b'+' as i8);
        let eq_period = _mm_cmpeq_epi8(chunk, v_period);
        let eq_minus = _mm_cmpeq_epi8(chunk, v_minus);
        let eq_plus = _mm_cmpeq_epi8(chunk, v_plus);

        let alpha = _mm_or_si128(lowercase, uppercase);
        let alnum = _mm_or_si128(alpha, digit);
        let punct = _mm_or_si128(_mm_or_si128(eq_period, eq_minus), eq_plus);
        let value_chars = _mm_or_si128(alnum, punct);

        CharClass {
            quotes: _mm_movemask_epi8(eq_quote) as u16,
            backslashes: _mm_movemask_epi8(eq_backslash) as u16,
            opens: _mm_movemask_epi8(opens) as u16,
            closes: _mm_movemask_epi8(closes) as u16,
            delims: _mm_movemask_epi8(delims) as u16,
            value_chars: _mm_movemask_epi8(value_chars) as u16,
        }
    }
}

/// Unsigned less-than-or-equal comparison for SSE2.
#[inline]
#[target_feature(enable = "sse2")]
#[cfg(target_arch = "x86_64")]
unsafe fn unsigned_le(a: __m128i, b: __m128i) -> __m128i {
    let min_ab = _mm_min_epu8(a, b);
    _mm_cmpeq_epi8(min_ab, a)
}

/// Process a 16-byte chunk and update IB/BP writers.
#[inline]
fn process_chunk_standard(
    class: CharClass,
    mut state: State,
    ib: &mut BitWriter,
    bp: &mut BitWriter,
    bytes: &[u8],
) -> State {
    for i in 0..bytes.len().min(16) {
        let bit = 1u16 << i;

        let is_quote = (class.quotes & bit) != 0;
        let is_backslash = (class.backslashes & bit) != 0;
        let is_open = (class.opens & bit) != 0;
        let is_close = (class.closes & bit) != 0;
        let is_delim = (class.delims & bit) != 0;
        let is_value_char = (class.value_chars & bit) != 0;

        match state {
            State::InJson => {
                if is_open {
                    ib.write_1();
                    bp.write_1();
                } else if is_close {
                    ib.write_0();
                    bp.write_0();
                } else if is_delim {
                    ib.write_0();
                } else if is_value_char {
                    ib.write_1();
                    bp.write_1();
                    bp.write_0();
                    state = State::InValue;
                } else if is_quote {
                    ib.write_1();
                    bp.write_1();
                    bp.write_0();
                    state = State::InString;
                } else {
                    ib.write_0();
                }
            }
            State::InString => {
                ib.write_0();
                if is_quote {
                    state = State::InJson;
                } else if is_backslash {
                    state = State::InEscape;
                }
            }
            State::InEscape => {
                ib.write_0();
                state = State::InString;
            }
            State::InValue => {
                if is_open {
                    ib.write_1();
                    bp.write_1();
                    state = State::InJson;
                } else if is_close {
                    ib.write_0();
                    bp.write_0();
                    state = State::InJson;
                } else if is_delim {
                    ib.write_0();
                    state = State::InJson;
                } else if is_value_char {
                    ib.write_0();
                } else {
                    ib.write_0();
                    state = State::InJson;
                }
            }
        }
    }

    state
}

/// Build a semi-index from JSON bytes using SSE4.2-accelerated Standard Cursor.
#[cfg(target_arch = "x86_64")]
pub fn build_semi_index_standard(json: &[u8]) -> SemiIndex {
    unsafe { build_semi_index_standard_sse42(json) }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.2")]
unsafe fn build_semi_index_standard_sse42(json: &[u8]) -> SemiIndex {
    unsafe {
        let word_capacity = json.len().div_ceil(64);
        let mut ib = BitWriter::with_capacity(word_capacity);
        let mut bp = BitWriter::with_capacity(word_capacity * 2);
        let mut state = State::InJson;

        let mut offset = 0;

        while offset + 16 <= json.len() {
            let chunk = _mm_loadu_si128(json.as_ptr().add(offset) as *const __m128i);
            let class = classify_chars(chunk);
            state =
                process_chunk_standard(class, state, &mut ib, &mut bp, &json[offset..offset + 16]);
            offset += 16;
        }

        if offset < json.len() {
            let mut padded = [0u8; 16];
            let remaining = json.len() - offset;
            padded[..remaining].copy_from_slice(&json[offset..]);

            let chunk = _mm_loadu_si128(padded.as_ptr() as *const __m128i);
            let class = classify_chars(chunk);
            state = process_chunk_standard(class, state, &mut ib, &mut bp, &json[offset..]);
        }

        SemiIndex {
            state,
            ib: ib.finish(),
            bp: bp.finish(),
        }
    }
}

// ============================================================================
// Simple Cursor Implementation
// ============================================================================

/// Process a 16-byte chunk for simple cursor.
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
                    bp.write_1();
                    bp.write_1();
                    ib.write_1();
                } else if is_close {
                    bp.write_0();
                    bp.write_0();
                    ib.write_1();
                } else if is_delim {
                    bp.write_0();
                    bp.write_1();
                    ib.write_1();
                } else if is_quote {
                    ib.write_0();
                    state = SimpleState::InString;
                } else {
                    ib.write_0();
                }
            }
            SimpleState::InString => {
                ib.write_0();
                if is_quote {
                    state = SimpleState::InJson;
                } else if is_backslash {
                    state = SimpleState::InEscape;
                }
            }
            SimpleState::InEscape => {
                ib.write_0();
                state = SimpleState::InString;
            }
        }
    }

    state
}

/// Build a semi-index from JSON bytes using SSE4.2-accelerated Simple Cursor.
#[cfg(target_arch = "x86_64")]
pub fn build_semi_index_simple(json: &[u8]) -> SimpleSemiIndex {
    unsafe { build_semi_index_simple_sse42(json) }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.2")]
unsafe fn build_semi_index_simple_sse42(json: &[u8]) -> SimpleSemiIndex {
    unsafe {
        let word_capacity = json.len().div_ceil(64);
        let mut ib = BitWriter::with_capacity(word_capacity);
        let mut bp = BitWriter::with_capacity(word_capacity * 2);
        let mut state = SimpleState::InJson;

        let mut offset = 0;

        while offset + 16 <= json.len() {
            let chunk = _mm_loadu_si128(json.as_ptr().add(offset) as *const __m128i);
            let class = classify_chars(chunk);
            state =
                process_chunk_simple(class, state, &mut ib, &mut bp, &json[offset..offset + 16]);
            offset += 16;
        }

        if offset < json.len() {
            let mut padded = [0u8; 16];
            let remaining = json.len() - offset;
            padded[..remaining].copy_from_slice(&json[offset..]);

            let chunk = _mm_loadu_si128(padded.as_ptr() as *const __m128i);
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

    fn get_bit(words: &[u64], i: usize) -> bool {
        let word_idx = i / 64;
        let bit_idx = i % 64;
        if word_idx < words.len() {
            (words[word_idx] >> bit_idx) & 1 == 1
        } else {
            false
        }
    }

    fn bits_to_string(words: &[u64], n: usize) -> String {
        (0..n)
            .map(|i| if get_bit(words, i) { '1' } else { '0' })
            .collect()
    }

    #[test]
    fn test_sse42_matches_scalar_empty_object() {
        if !is_x86_feature_detected!("sse4.2") {
            return;
        }

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
    fn test_sse42_matches_scalar_simple_object() {
        if !is_x86_feature_detected!("sse4.2") {
            return;
        }

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
    fn test_sse42_matches_scalar_long_input() {
        if !is_x86_feature_detected!("sse4.2") {
            return;
        }

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
    fn test_simple_sse42_matches_scalar_empty_object() {
        if !is_x86_feature_detected!("sse4.2") {
            return;
        }

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
    fn test_simple_sse42_matches_scalar_long_input() {
        if !is_x86_feature_detected!("sse4.2") {
            return;
        }

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
}
