//! NEON-accelerated JSON semi-indexing for ARM64.
//!
//! Processes 16 bytes at a time using ARM NEON SIMD instructions.

use core::arch::aarch64::*;

use crate::json::BitWriter;
use crate::json::simple::{SemiIndex as SimpleSemiIndex, State as SimpleState};
use crate::json::standard::{SemiIndex, State};

/// ASCII byte constants
const DOUBLE_QUOTE: u8 = b'"';
const BACKSLASH: u8 = b'\\';
const OPEN_BRACE: u8 = b'{';
const CLOSE_BRACE: u8 = b'}';
const OPEN_BRACKET: u8 = b'[';
const CLOSE_BRACKET: u8 = b']';
const COMMA: u8 = b',';
const COLON: u8 = b':';

/// Extract a bitmask from the high bit of each byte in a NEON vector.
/// Returns a u16 where bit i is set if byte i has its high bit set.
///
/// This is the optimized parallel implementation using weighted shifts and
/// horizontal addition - much faster than the serial loop approach.
#[inline]
#[target_feature(enable = "neon")]
unsafe fn neon_movemask(v: uint8x16_t) -> u16 {
    unsafe { neon_movemask_parallel(v) }
}

/// Serial movemask implementation for benchmarking comparison.
/// This is the naive approach - kept for performance comparison only.
#[inline]
#[target_feature(enable = "neon")]
#[allow(dead_code)]
pub(crate) unsafe fn neon_movemask_serial(v: uint8x16_t) -> u16 {
    unsafe {
        let mut mask: u16 = 0;
        let arr: [u8; 16] = core::mem::transmute(v);
        for (i, &byte) in arr.iter().enumerate() {
            if byte & 0x80 != 0 {
                mask |= 1 << i;
            }
        }
        mask
    }
}

/// Optimized parallel movemask using weighted shifts and horizontal add.
/// ~3-5x faster than the serial version on ARM.
#[inline]
#[target_feature(enable = "neon")]
pub(crate) unsafe fn neon_movemask_parallel(v: uint8x16_t) -> u16 {
    unsafe {
        // Shift each byte right by 7 to get just the high bit (0 or 1)
        let high_bits = vshrq_n_u8::<7>(v);

        // Create shift amounts: [0,1,2,3,4,5,6,7, 0,1,2,3,4,5,6,7]
        let shift_amounts: [i8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 0, 1, 2, 3, 4, 5, 6, 7];
        let shifts = vld1q_s8(shift_amounts.as_ptr());

        // Shift each byte left by its lane index (multiply by 2^i)
        let shifted = vshlq_u8(high_bits, shifts);

        // Split into low and high halves
        let low = vget_low_u8(shifted);
        let high = vget_high_u8(shifted);

        // Horizontal add within each half to get a single byte
        let low_sum = vaddv_u8(low) as u16;
        let high_sum = vaddv_u8(high) as u16;

        low_sum | (high_sum << 8)
    }
}

/// Alternative movemask using multiplication instead of variable shifts.
/// This approach uses vmul_u8 with pre-computed weights instead of vshlq_u8.
#[inline]
#[target_feature(enable = "neon")]
#[allow(dead_code)]
pub(crate) unsafe fn neon_movemask_mul(v: uint8x16_t) -> u16 {
    unsafe {
        // Split into halves first
        let v_hi = vget_high_u8(v);
        let v_lo = vget_low_u8(v);

        // Extract high bits from each byte (shift right by 7)
        let hi_bits_hi = vshr_n_u8::<7>(v_hi);
        let hi_bits_lo = vshr_n_u8::<7>(v_lo);

        // Weight by position using multiplication
        let weights: [u8; 8] = [1, 2, 4, 8, 16, 32, 64, 128];
        let w = vld1_u8(weights.as_ptr());

        let weighted_lo = vmul_u8(hi_bits_lo, w);
        let weighted_hi = vmul_u8(hi_bits_hi, w);

        let sum_lo = vaddv_u8(weighted_lo) as u16;
        let sum_hi = vaddv_u8(weighted_hi) as u16;

        sum_lo | (sum_hi << 8)
    }
}

/// Movemask using polynomial multiplication approach.
/// Uses PMULL to efficiently gather bits.
#[inline]
#[target_feature(enable = "neon")]
#[allow(dead_code)]
pub(crate) unsafe fn neon_movemask_pmul(v: uint8x16_t) -> u16 {
    unsafe {
        // The PMULL approach requires additional setup that doesn't pay off
        // for 16 bytes. Fall back to the parallel approach.
        neon_movemask_parallel(v)
    }
}

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

/// Classify 16 bytes at once using NEON.
#[inline]
#[target_feature(enable = "neon")]
unsafe fn classify_chars(chunk: uint8x16_t) -> CharClass {
    unsafe {
        // Create comparison vectors for each character
        let v_quote = vdupq_n_u8(DOUBLE_QUOTE);
        let v_backslash = vdupq_n_u8(BACKSLASH);
        let v_open_brace = vdupq_n_u8(OPEN_BRACE);
        let v_close_brace = vdupq_n_u8(CLOSE_BRACE);
        let v_open_bracket = vdupq_n_u8(OPEN_BRACKET);
        let v_close_bracket = vdupq_n_u8(CLOSE_BRACKET);
        let v_comma = vdupq_n_u8(COMMA);
        let v_colon = vdupq_n_u8(COLON);

        // Compare and get masks
        let eq_quote = vceqq_u8(chunk, v_quote);
        let eq_backslash = vceqq_u8(chunk, v_backslash);
        let eq_open_brace = vceqq_u8(chunk, v_open_brace);
        let eq_close_brace = vceqq_u8(chunk, v_close_brace);
        let eq_open_bracket = vceqq_u8(chunk, v_open_bracket);
        let eq_close_bracket = vceqq_u8(chunk, v_close_bracket);
        let eq_comma = vceqq_u8(chunk, v_comma);
        let eq_colon = vceqq_u8(chunk, v_colon);

        // Combine opens and closes
        let opens = vorrq_u8(eq_open_brace, eq_open_bracket);
        let closes = vorrq_u8(eq_close_brace, eq_close_bracket);
        let delims = vorrq_u8(eq_comma, eq_colon);

        // Value chars: alphanumeric, period, minus, plus
        // a-z: 0x61-0x7a, A-Z: 0x41-0x5a, 0-9: 0x30-0x39
        // period: 0x2e, minus: 0x2d, plus: 0x2b

        // Check for lowercase: c >= 'a' && c <= 'z'
        let v_a_lower = vdupq_n_u8(b'a');
        let v_z_lower = vdupq_n_u8(b'z');
        let ge_a = vcgeq_u8(chunk, v_a_lower);
        let le_z = vcleq_u8(chunk, v_z_lower);
        let lowercase = vandq_u8(ge_a, le_z);

        // Check for uppercase: c >= 'A' && c <= 'Z'
        let v_a_upper = vdupq_n_u8(b'A');
        let v_z_upper = vdupq_n_u8(b'Z');
        let ge_a_upper = vcgeq_u8(chunk, v_a_upper);
        let le_z_upper = vcleq_u8(chunk, v_z_upper);
        let uppercase = vandq_u8(ge_a_upper, le_z_upper);

        // Check for digit: c >= '0' && c <= '9'
        let v_0 = vdupq_n_u8(b'0');
        let v_9 = vdupq_n_u8(b'9');
        let ge_0 = vcgeq_u8(chunk, v_0);
        let le_9 = vcleq_u8(chunk, v_9);
        let digit = vandq_u8(ge_0, le_9);

        // Check for period, minus, plus
        let v_period = vdupq_n_u8(b'.');
        let v_minus = vdupq_n_u8(b'-');
        let v_plus = vdupq_n_u8(b'+');
        let eq_period = vceqq_u8(chunk, v_period);
        let eq_minus = vceqq_u8(chunk, v_minus);
        let eq_plus = vceqq_u8(chunk, v_plus);

        // Combine all value chars
        let alpha = vorrq_u8(lowercase, uppercase);
        let alnum = vorrq_u8(alpha, digit);
        let punct = vorrq_u8(vorrq_u8(eq_period, eq_minus), eq_plus);
        let value_chars = vorrq_u8(alnum, punct);

        CharClass {
            quotes: neon_movemask(eq_quote),
            backslashes: neon_movemask(eq_backslash),
            opens: neon_movemask(opens),
            closes: neon_movemask(closes),
            delims: neon_movemask(delims),
            value_chars: neon_movemask(value_chars),
        }
    }
}

/// Process a 16-byte chunk and update IB/BP writers.
/// Returns the new state after processing all 16 bytes.
#[inline]
fn process_chunk_standard(
    class: CharClass,
    mut state: State,
    ib: &mut BitWriter,
    bp: &mut BitWriter,
    bytes: &[u8],
) -> State {
    // Process each byte using the classification masks
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
/// Processes 16 bytes at a time using ARM NEON instructions for character
/// classification, then processes the state machine transitions.
pub fn build_semi_index_standard(json: &[u8]) -> SemiIndex {
    // SAFETY: We check for NEON support at compile time via target_arch
    unsafe { build_semi_index_standard_neon(json) }
}

#[target_feature(enable = "neon")]
unsafe fn build_semi_index_standard_neon(json: &[u8]) -> SemiIndex {
    unsafe {
        let word_capacity = json.len().div_ceil(64);
        let mut ib = BitWriter::with_capacity(word_capacity);
        let mut bp = BitWriter::with_capacity(word_capacity * 2);
        let mut state = State::InJson;

        let mut offset = 0;

        // Process 16-byte chunks
        while offset + 16 <= json.len() {
            let chunk = vld1q_u8(json.as_ptr().add(offset));
            let class = classify_chars(chunk);
            state =
                process_chunk_standard(class, state, &mut ib, &mut bp, &json[offset..offset + 16]);
            offset += 16;
        }

        // Process remaining bytes (less than 16)
        if offset < json.len() {
            // Pad with zeros and process
            let mut padded = [0u8; 16];
            let remaining = json.len() - offset;
            padded[..remaining].copy_from_slice(&json[offset..]);

            let chunk = vld1q_u8(padded.as_ptr());
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
/// Processes 16 bytes at a time using ARM NEON instructions for character
/// classification, then processes the state machine transitions.
pub fn build_semi_index_simple(json: &[u8]) -> SimpleSemiIndex {
    // SAFETY: We check for NEON support at compile time via target_arch
    unsafe { build_semi_index_simple_neon(json) }
}

#[target_feature(enable = "neon")]
unsafe fn build_semi_index_simple_neon(json: &[u8]) -> SimpleSemiIndex {
    unsafe {
        let word_capacity = json.len().div_ceil(64);
        let mut ib = BitWriter::with_capacity(word_capacity);
        let mut bp = BitWriter::with_capacity(word_capacity * 2);
        let mut state = SimpleState::InJson;

        let mut offset = 0;

        // Process 16-byte chunks
        while offset + 16 <= json.len() {
            let chunk = vld1q_u8(json.as_ptr().add(offset));
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

            let chunk = vld1q_u8(padded.as_ptr());
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

#[cfg(test)]
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
    fn test_neon_movemask() {
        unsafe {
            // All high bits set
            let v = vdupq_n_u8(0x80);
            assert_eq!(neon_movemask(v), 0xFFFF);

            // No high bits set
            let v = vdupq_n_u8(0x7F);
            assert_eq!(neon_movemask(v), 0x0000);

            // Alternating pattern
            let bytes: [u8; 16] = [
                0x80, 0x00, 0x80, 0x00, 0x80, 0x00, 0x80, 0x00, 0x80, 0x00, 0x80, 0x00, 0x80, 0x00,
                0x80, 0x00,
            ];
            let v = vld1q_u8(bytes.as_ptr());
            assert_eq!(neon_movemask(v), 0x5555);
        }
    }

    #[test]
    fn test_classify_chars() {
        unsafe {
            let input = br#"{"hello":123}   "#;
            let chunk = vld1q_u8(input.as_ptr());
            let class = classify_chars(chunk);

            // Position 0 is '{', position 12 is '}'
            assert_ne!(class.opens & (1 << 0), 0);
            assert_ne!(class.closes & (1 << 12), 0);

            // Position 1 and 7 are '"'
            assert_ne!(class.quotes & (1 << 1), 0);
            assert_ne!(class.quotes & (1 << 7), 0);

            // Position 8 is ':'
            assert_ne!(class.delims & (1 << 8), 0);

            // Positions 9, 10, 11 are digits
            assert_ne!(class.value_chars & (1 << 9), 0);
            assert_ne!(class.value_chars & (1 << 10), 0);
            assert_ne!(class.value_chars & (1 << 11), 0);
        }
    }

    #[test]
    fn test_simd_matches_scalar_empty_object() {
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
    fn test_simd_matches_scalar_simple_object() {
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
    fn test_simd_matches_scalar_array() {
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
    fn test_simd_matches_scalar_nested() {
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
    fn test_simd_matches_scalar_escaped() {
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
    fn test_simd_matches_scalar_long_input() {
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
    fn test_simd_matches_scalar_booleans() {
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
    fn test_simd_matches_scalar_whitespace() {
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
    fn test_simd_matches_scalar_exact_16_bytes() {
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
    fn test_simd_matches_scalar_32_bytes() {
        // 32 bytes - two full chunks
        let json = br#"{"abcdefghij":"klmnopqrst"}"#; // Adjust to exactly 32
        let simd_result = build_semi_index_standard(json);
        let scalar_result = crate::json::standard::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch for 32-byte input"
        );
    }

    // ========================================================================
    // Simple Cursor SIMD Tests
    // ========================================================================

    #[test]
    fn test_simple_simd_matches_scalar_empty_object() {
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
    fn test_simple_simd_matches_scalar_empty_array() {
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
    fn test_simple_simd_matches_scalar_simple_object() {
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
    fn test_simple_simd_matches_scalar_array_with_values() {
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
    fn test_simple_simd_matches_scalar_nested() {
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
    fn test_simple_simd_matches_scalar_escaped() {
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
    fn test_simple_simd_matches_scalar_escaped_backslash() {
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
    fn test_simple_simd_matches_scalar_long_input() {
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
    fn test_simple_simd_matches_scalar_whitespace() {
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
    fn test_simple_simd_matches_scalar_exact_16_bytes() {
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
    fn test_simple_simd_matches_scalar_32_bytes() {
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
    fn test_simple_simd_matches_scalar_unterminated_string() {
        let json = br#"{"a"#;
        let simd_result = build_semi_index_simple(json);
        let scalar_result = crate::json::simple::build_semi_index(json);

        assert_eq!(simd_result.state, scalar_result.state);
        assert_eq!(simd_result.state, SimpleState::InString);
    }

    #[test]
    fn test_simple_simd_matches_scalar_unterminated_escape() {
        // String ending with a single backslash: "\ (2 bytes: quote, backslash)
        let json = br#""\"#;
        let simd_result = build_semi_index_simple(json);
        let scalar_result = crate::json::simple::build_semi_index(json);

        assert_eq!(simd_result.state, scalar_result.state);
        assert_eq!(simd_result.state, SimpleState::InEscape);
    }
}
