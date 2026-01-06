//! AVX-512-accelerated JSON semi-indexing for x86_64.
//!
//! Processes 64 bytes at a time using x86_64 AVX-512 SIMD instructions.
//! AVX-512 is available on Intel Ice Lake (2019+) and AMD Zen 4 (2022+).
//!
//! This module doubles the throughput of AVX2 by processing 64 bytes per iteration
//! instead of 32 bytes. On CPUs without AVX-512, the runtime dispatcher will
//! automatically fall back to AVX2, SSE4.2, or SSE2.

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

use crate::json::BitWriter;
use crate::json::simple::{SemiIndex as SimpleSemiIndex, State as SimpleState};
use crate::json::standard::{SemiIndex, State};

/// ASCII byte constants
const DOUBLE_QUOTE: i8 = b'"' as i8;
const BACKSLASH: i8 = b'\\' as i8;
const OPEN_BRACE: i8 = b'{' as i8;
const CLOSE_BRACE: i8 = b'}' as i8;
const OPEN_BRACKET: i8 = b'[' as i8;
const CLOSE_BRACKET: i8 = b']' as i8;
const COMMA: i8 = b',' as i8;
const COLON: i8 = b':' as i8;

/// Character classification results for a 64-byte chunk.
#[derive(Debug, Clone, Copy)]
struct CharClass {
    /// Mask of bytes that are '"'
    quotes: u64,
    /// Mask of bytes that are '\'
    backslashes: u64,
    /// Mask of bytes that are '{' or '['
    opens: u64,
    /// Mask of bytes that are '}' or ']'
    closes: u64,
    /// Mask of bytes that are ',' or ':'
    delims: u64,
    /// Mask of bytes that could start/continue a value (alphanumeric, ., -, +)
    value_chars: u64,
}

/// Classify 64 bytes at once using AVX-512.
#[inline]
#[target_feature(enable = "avx512f,avx512bw")]
#[cfg(target_arch = "x86_64")]
unsafe fn classify_chars(chunk: __m512i) -> CharClass {
    unsafe {
        // Create comparison vectors for each character
        let v_quote = _mm512_set1_epi8(DOUBLE_QUOTE);
        let v_backslash = _mm512_set1_epi8(BACKSLASH);
        let v_open_brace = _mm512_set1_epi8(OPEN_BRACE);
        let v_close_brace = _mm512_set1_epi8(CLOSE_BRACE);
        let v_open_bracket = _mm512_set1_epi8(OPEN_BRACKET);
        let v_close_bracket = _mm512_set1_epi8(CLOSE_BRACKET);
        let v_comma = _mm512_set1_epi8(COMMA);
        let v_colon = _mm512_set1_epi8(COLON);

        // Compare and get masks using AVX-512 mask registers
        let eq_quote = _mm512_cmpeq_epi8_mask(chunk, v_quote);
        let eq_backslash = _mm512_cmpeq_epi8_mask(chunk, v_backslash);
        let eq_open_brace = _mm512_cmpeq_epi8_mask(chunk, v_open_brace);
        let eq_close_brace = _mm512_cmpeq_epi8_mask(chunk, v_close_brace);
        let eq_open_bracket = _mm512_cmpeq_epi8_mask(chunk, v_open_bracket);
        let eq_close_bracket = _mm512_cmpeq_epi8_mask(chunk, v_close_bracket);
        let eq_comma = _mm512_cmpeq_epi8_mask(chunk, v_comma);
        let eq_colon = _mm512_cmpeq_epi8_mask(chunk, v_colon);

        // Combine opens and closes using bitwise OR on masks
        let opens = eq_open_brace | eq_open_bracket;
        let closes = eq_close_brace | eq_close_bracket;
        let delims = eq_comma | eq_colon;

        // Value chars: alphanumeric, period, minus, plus
        // a-z: 0x61-0x7a, A-Z: 0x41-0x5a, 0-9: 0x30-0x39
        // period: 0x2e, minus: 0x2d, plus: 0x2b

        // Check for lowercase: c >= 'a' && c <= 'z'
        let v_a_lower = _mm512_set1_epi8(b'a' as i8);
        let v_z_range = _mm512_set1_epi8((b'z' - b'a') as i8);
        let sub_a = _mm512_sub_epi8(chunk, v_a_lower);
        let lowercase = unsigned_le(sub_a, v_z_range);

        // Check for uppercase: c >= 'A' && c <= 'Z'
        let v_a_upper = _mm512_set1_epi8(b'A' as i8);
        let sub_a_upper = _mm512_sub_epi8(chunk, v_a_upper);
        let uppercase = unsigned_le(sub_a_upper, v_z_range);

        // Check for digit: c >= '0' && c <= '9'
        let v_0 = _mm512_set1_epi8(b'0' as i8);
        let v_9_range = _mm512_set1_epi8(9); // '9' - '0' = 9
        let sub_0 = _mm512_sub_epi8(chunk, v_0);
        let digit = unsigned_le(sub_0, v_9_range);

        // Check for period, minus, plus
        let v_period = _mm512_set1_epi8(b'.' as i8);
        let v_minus = _mm512_set1_epi8(b'-' as i8);
        let v_plus = _mm512_set1_epi8(b'+' as i8);
        let eq_period = _mm512_cmpeq_epi8_mask(chunk, v_period);
        let eq_minus = _mm512_cmpeq_epi8_mask(chunk, v_minus);
        let eq_plus = _mm512_cmpeq_epi8_mask(chunk, v_plus);

        // Combine all value chars using bitwise OR on masks
        let alpha = lowercase | uppercase;
        let alnum = alpha | digit;
        let punct = eq_period | eq_minus | eq_plus;
        let value_chars = alnum | punct;

        CharClass {
            quotes: eq_quote,
            backslashes: eq_backslash,
            opens,
            closes,
            delims,
            value_chars,
        }
    }
}

/// Unsigned less-than-or-equal comparison for AVX-512.
/// Returns mask where a <= b (unsigned).
#[inline]
#[target_feature(enable = "avx512f,avx512bw")]
#[cfg(target_arch = "x86_64")]
unsafe fn unsigned_le(a: __m512i, b: __m512i) -> u64 {
    // For unsigned comparison a <= b:
    // We use: min(a, b) == a
    // AVX-512 has _mm512_min_epu8 for unsigned byte minimum
    let min_ab = _mm512_min_epu8(a, b);
    _mm512_cmpeq_epi8_mask(min_ab, a)
}

/// Process a 64-byte chunk and update IB/BP writers.
/// Returns the new state after processing all 64 bytes.
#[inline]
fn process_chunk_standard(
    class: CharClass,
    mut state: State,
    ib: &mut BitWriter,
    bp: &mut BitWriter,
    bytes: &[u8],
) -> State {
    // Process each byte using the classification masks
    for i in 0..bytes.len().min(64) {
        let bit = 1u64 << i;

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
                    // state stays InJson
                } else if is_close {
                    ib.write_0();
                    bp.write_0();
                    // state stays InJson
                } else if is_delim {
                    ib.write_0();
                    // no BP output
                    // state stays InJson
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
                    // whitespace or other
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
                    // state stays InValue
                } else {
                    // whitespace ends value
                    ib.write_0();
                    state = State::InJson;
                }
            }
        }
    }

    state
}

/// Build a semi-index from JSON bytes using SIMD-accelerated Standard Cursor algorithm.
///
/// Processes 64 bytes at a time using x86_64 AVX-512 instructions for character
/// classification, then processes the state machine transitions.
#[cfg(target_arch = "x86_64")]
pub fn build_semi_index_standard(json: &[u8]) -> SemiIndex {
    // SAFETY: Caller must ensure AVX-512 is available
    unsafe { build_semi_index_standard_avx512(json) }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn build_semi_index_standard_avx512(json: &[u8]) -> SemiIndex {
    unsafe {
        let word_capacity = json.len().div_ceil(64);
        let mut ib = BitWriter::with_capacity(word_capacity);
        let mut bp = BitWriter::with_capacity(word_capacity * 2);
        let mut state = State::InJson;

        let mut offset = 0;

        // Process 64-byte chunks
        while offset + 64 <= json.len() {
            let chunk = _mm512_loadu_si512(json.as_ptr().add(offset) as *const __m512i);
            let class = classify_chars(chunk);
            state =
                process_chunk_standard(class, state, &mut ib, &mut bp, &json[offset..offset + 64]);
            offset += 64;
        }

        // Process remaining bytes (less than 64)
        if offset < json.len() {
            // Pad with zeros and process
            let mut padded = [0u8; 64];
            let remaining = json.len() - offset;
            padded[..remaining].copy_from_slice(&json[offset..]);

            let chunk = _mm512_loadu_si512(padded.as_ptr() as *const __m512i);
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
// Simple Cursor SIMD Implementation
// ============================================================================

/// Process a 64-byte chunk for simple cursor and update IB/BP writers.
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
    for i in 0..bytes.len().min(64) {
        let bit = 1u64 << i;

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
/// Processes 64 bytes at a time using x86_64 AVX-512 instructions for character
/// classification, then processes the state machine transitions.
#[cfg(target_arch = "x86_64")]
pub fn build_semi_index_simple(json: &[u8]) -> SimpleSemiIndex {
    // SAFETY: Caller must ensure AVX-512 is available
    unsafe { build_semi_index_simple_avx512(json) }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn build_semi_index_simple_avx512(json: &[u8]) -> SimpleSemiIndex {
    unsafe {
        let word_capacity = json.len().div_ceil(64);
        let mut ib = BitWriter::with_capacity(word_capacity);
        let mut bp = BitWriter::with_capacity(word_capacity * 2);
        let mut state = SimpleState::InJson;

        let mut offset = 0;

        // Process 64-byte chunks
        while offset + 64 <= json.len() {
            let chunk = _mm512_loadu_si512(json.as_ptr().add(offset) as *const __m512i);
            let class = classify_chars(chunk);
            state =
                process_chunk_simple(class, state, &mut ib, &mut bp, &json[offset..offset + 64]);
            offset += 64;
        }

        // Process remaining bytes (less than 64)
        if offset < json.len() {
            // Pad with zeros and process
            let mut padded = [0u8; 64];
            let remaining = json.len() - offset;
            padded[..remaining].copy_from_slice(&json[offset..]);

            let chunk = _mm512_loadu_si512(padded.as_ptr() as *const __m512i);
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
    fn test_avx512_matches_scalar_empty_object() {
        if !is_x86_feature_detected!("avx512f") || !is_x86_feature_detected!("avx512bw") {
            eprintln!("Skipping AVX-512 test: CPU doesn't support AVX512F+AVX512BW");
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
    fn test_avx512_matches_scalar_simple_object() {
        if !is_x86_feature_detected!("avx512f") || !is_x86_feature_detected!("avx512bw") {
            eprintln!("Skipping AVX-512 test: CPU doesn't support AVX512F+AVX512BW");
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
    fn test_avx512_matches_scalar_long_input() {
        if !is_x86_feature_detected!("avx512f") || !is_x86_feature_detected!("avx512bw") {
            eprintln!("Skipping AVX-512 test: CPU doesn't support AVX512F+AVX512BW");
            return;
        }

        // Test with input > 64 bytes to exercise chunk processing
        let json = br#"{"name":"value","number":12345,"array":[1,2,3],"nested":{"key":"val"}}"#;
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
    fn test_avx512_matches_scalar_exact_64_bytes() {
        if !is_x86_feature_detected!("avx512f") || !is_x86_feature_detected!("avx512bw") {
            eprintln!("Skipping AVX-512 test: CPU doesn't support AVX512F+AVX512BW");
            return;
        }

        // Exactly 64 bytes - one full chunk
        let json = br#"{"a":"b","c":"d","e":"f","g":"h","i":"j","k":"l","m":"nopqrstu"}"#;
        assert_eq!(json.len(), 64, "Test JSON should be exactly 64 bytes");

        let simd_result = build_semi_index_standard(json);
        let scalar_result = crate::json::standard::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch for 64-byte input"
        );
    }

    #[test]
    fn test_avx512_matches_scalar_128_bytes() {
        if !is_x86_feature_detected!("avx512f") || !is_x86_feature_detected!("avx512bw") {
            eprintln!("Skipping AVX-512 test: CPU doesn't support AVX512F+AVX512BW");
            return;
        }

        // 128 bytes - two full chunks
        let json = br#"{"k1":"v1","k2":"v2","k3":"v3","k4":"v4","k5":"v5","k6":"v6","k7":"v7","k8":"v8","k9":"v9","k10":"v10","k11":"v11","k12":"vals"}"#;
        assert_eq!(json.len(), 128, "Test JSON should be exactly 128 bytes");

        let simd_result = build_semi_index_standard(json);
        let scalar_result = crate::json::standard::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch for 128-byte input"
        );
    }

    // ========================================================================
    // Simple Cursor AVX-512 Tests
    // ========================================================================

    #[test]
    fn test_simple_avx512_matches_scalar_empty_object() {
        if !is_x86_feature_detected!("avx512f") || !is_x86_feature_detected!("avx512bw") {
            eprintln!("Skipping AVX-512 test: CPU doesn't support AVX512F+AVX512BW");
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
    fn test_simple_avx512_matches_scalar_simple_object() {
        if !is_x86_feature_detected!("avx512f") || !is_x86_feature_detected!("avx512bw") {
            eprintln!("Skipping AVX-512 test: CPU doesn't support AVX512F+AVX512BW");
            return;
        }

        let json = br#"{"a":"b"}"#;
        let simd_result = build_semi_index_simple(json);
        let scalar_result = crate::json::simple::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch"
        );
        assert_eq!(
            bits_to_string(&simd_result.bp, 6),
            bits_to_string(&scalar_result.bp, 6),
            "BP mismatch"
        );
    }

    #[test]
    fn test_simple_avx512_matches_scalar_long_input() {
        if !is_x86_feature_detected!("avx512f") || !is_x86_feature_detected!("avx512bw") {
            eprintln!("Skipping AVX-512 test: CPU doesn't support AVX512F+AVX512BW");
            return;
        }

        // Test with input > 64 bytes
        let json = br#"{"name":"value","number":12345,"array":[1,2,3],"nested":{"key":"val"}}"#;
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
    fn test_simple_avx512_matches_scalar_exact_64_bytes() {
        if !is_x86_feature_detected!("avx512f") || !is_x86_feature_detected!("avx512bw") {
            eprintln!("Skipping AVX-512 test: CPU doesn't support AVX512F+AVX512BW");
            return;
        }

        let json = br#"{"a":"b","c":"d","e":"f","g":"h","i":"j","k":"l","m":"nopqrstu"}"#;
        assert_eq!(json.len(), 64, "Test JSON should be exactly 64 bytes");

        let simd_result = build_semi_index_simple(json);
        let scalar_result = crate::json::simple::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch for 64-byte input"
        );
    }

    #[test]
    fn test_simple_avx512_matches_scalar_escaped() {
        if !is_x86_feature_detected!("avx512f") || !is_x86_feature_detected!("avx512bw") {
            eprintln!("Skipping AVX-512 test: CPU doesn't support AVX512F+AVX512BW");
            return;
        }

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
}
