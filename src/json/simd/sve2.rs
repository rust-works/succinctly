//! SVE2-accelerated JSON semi-indexing for ARM64.
//!
//! Uses SVE2 instructions to classify JSON structural characters in parallel.
//!
//! ## SVE2 vs NEON nibble tables
//!
//! - NEON: 2 table lookups + 1 AND + 5 bit extractions per classification
//! - SVE2: Comparisons with predication, scalable vector length
//!
//! ## Performance on Neoverse-V2 (128-bit SVE2)
//!
//! **Result: SVE2 is ~36% SLOWER than NEON** on Graviton 4.
//!
//! Reasons:
//! 1. Same vector width (128-bit) - no throughput advantage
//! 2. Expensive predicate-to-bitmask conversion (6 stores + scalar loops)
//! 3. NEON's nibble tables are highly optimized (2 lookups + 1 AND)
//! 4. SVE2 requires more instructions per classification
//!
//! This implementation is kept for:
//! - Testing on CPUs with wider SVE2 vectors (256-bit, 512-bit)
//! - Future optimization when better predicate extraction is available
//!
//! ## Hardware Support
//!
//! Requires SVE2:
//! - AWS Graviton 3 (Neoverse V1): SVE only, NOT SVE2
//! - AWS Graviton 4 (Neoverse V2): SVE2 with 128-bit vectors
//! - Azure Cobalt 100 (Neoverse N2): SVE2 with 128-bit vectors

use core::arch::asm;

use crate::json::simple::{SemiIndex as SimpleSemiIndex, State as SimpleState};
use crate::json::standard::{SemiIndex, State};
use crate::json::BitWriter;

/// Character classification results for a chunk (up to 16 bytes for 128-bit SVE2).
#[derive(Debug, Clone, Copy, Default)]
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
    /// Combined mask of quotes OR backslashes (for quick InString scanning)
    string_special: u16,
    /// Number of valid bytes in this chunk
    len: usize,
}

/// Get the SVE vector length in bytes.
#[inline]
#[target_feature(enable = "sve2")]
unsafe fn get_vl() -> usize {
    let vl: usize;
    unsafe {
        asm!(
            "cntb {vl}",
            vl = out(reg) vl,
            options(pure, nomem, nostack)
        );
    }
    vl
}

/// Classify bytes using SVE2 SIMD instructions.
///
/// This uses actual SVE2 SIMD for classification but extracts results
/// through a vector store, avoiding complex predicate-to-scalar conversion.
///
/// # Safety
///
/// Requires SVE2. Caller must verify with `is_aarch64_feature_detected!("sve2")`.
#[inline]
#[target_feature(enable = "sve2")]
unsafe fn classify_chars_sve2_simd(data: *const u8, len: usize) -> CharClass {
    let vl = unsafe { get_vl() };
    // Limit to 16 bytes for bitmask extraction
    let chunk_len = len.min(vl).min(16);

    if chunk_len == 0 {
        return CharClass::default();
    }

    // Buffers for storing comparison results
    let mut quotes_buf = [0u8; 16];
    let mut backslashes_buf = [0u8; 16];
    let mut opens_buf = [0u8; 16];
    let mut closes_buf = [0u8; 16];
    let mut delims_buf = [0u8; 16];
    let mut value_chars_buf = [0u8; 16];

    unsafe {
        asm!(
            // Create predicate for valid bytes
            "whilelt p0.b, xzr, {len}",

            // Load data with predication
            "ld1b z0.b, p0/z, [{data}]",

            // Character constants
            // '"' = 0x22, '\' = 0x5C
            // '{' = 0x7B, '}' = 0x7D, '[' = 0x5B, ']' = 0x5D
            // ',' = 0x2C, ':' = 0x3A

            // Quotes: compare with '"' (0x22)
            "mov z1.b, #0x22",
            "cmpeq p1.b, p0/z, z0.b, z1.b",
            "mov z1.b, #0",
            "mov z1.b, p1/m, #0xFF",
            "st1b z1.b, p0, [{quotes_buf}]",

            // Backslashes: compare with '\' (0x5C)
            "mov z1.b, #0x5C",
            "cmpeq p1.b, p0/z, z0.b, z1.b",
            "mov z1.b, #0",
            "mov z1.b, p1/m, #0xFF",
            "st1b z1.b, p0, [{backslashes_buf}]",

            // Opens: '{' (0x7B) or '[' (0x5B)
            "mov z1.b, #0x7B",
            "cmpeq p1.b, p0/z, z0.b, z1.b",
            "mov z1.b, #0x5B",
            "cmpeq p2.b, p0/z, z0.b, z1.b",
            "orr p1.b, p0/z, p1.b, p2.b",
            "mov z1.b, #0",
            "mov z1.b, p1/m, #0xFF",
            "st1b z1.b, p0, [{opens_buf}]",

            // Closes: '}' (0x7D) or ']' (0x5D)
            "mov z1.b, #0x7D",
            "cmpeq p1.b, p0/z, z0.b, z1.b",
            "mov z1.b, #0x5D",
            "cmpeq p2.b, p0/z, z0.b, z1.b",
            "orr p1.b, p0/z, p1.b, p2.b",
            "mov z1.b, #0",
            "mov z1.b, p1/m, #0xFF",
            "st1b z1.b, p0, [{closes_buf}]",

            // Delims: ',' (0x2C) or ':' (0x3A)
            "mov z1.b, #0x2C",
            "cmpeq p1.b, p0/z, z0.b, z1.b",
            "mov z1.b, #0x3A",
            "cmpeq p2.b, p0/z, z0.b, z1.b",
            "orr p1.b, p0/z, p1.b, p2.b",
            "mov z1.b, #0",
            "mov z1.b, p1/m, #0xFF",
            "st1b z1.b, p0, [{delims_buf}]",

            // Value chars: digits (0x30-0x39), uppercase (0x41-0x5A), lowercase (0x61-0x7A)
            // Plus '.', '-', '+'

            // Digits: z0 - '0' < 10
            "mov z1.b, #0x30",
            "sub z2.b, z0.b, z1.b",
            "mov z1.b, #10",
            "cmplo p1.b, p0/z, z2.b, z1.b",

            // Uppercase: z0 - 'A' < 26
            "mov z1.b, #0x41",
            "sub z2.b, z0.b, z1.b",
            "mov z1.b, #26",
            "cmplo p2.b, p0/z, z2.b, z1.b",
            "orr p1.b, p0/z, p1.b, p2.b",

            // Lowercase: z0 - 'a' < 26
            "mov z1.b, #0x61",
            "sub z2.b, z0.b, z1.b",
            "mov z1.b, #26",
            "cmplo p2.b, p0/z, z2.b, z1.b",
            "orr p1.b, p0/z, p1.b, p2.b",

            // '.' (0x2E)
            "mov z1.b, #0x2E",
            "cmpeq p2.b, p0/z, z0.b, z1.b",
            "orr p1.b, p0/z, p1.b, p2.b",

            // '-' (0x2D)
            "mov z1.b, #0x2D",
            "cmpeq p2.b, p0/z, z0.b, z1.b",
            "orr p1.b, p0/z, p1.b, p2.b",

            // '+' (0x2B)
            "mov z1.b, #0x2B",
            "cmpeq p2.b, p0/z, z0.b, z1.b",
            "orr p1.b, p0/z, p1.b, p2.b",

            "mov z1.b, #0",
            "mov z1.b, p1/m, #0xFF",
            "st1b z1.b, p0, [{value_chars_buf}]",

            data = in(reg) data,
            len = in(reg) chunk_len,
            quotes_buf = in(reg) quotes_buf.as_mut_ptr(),
            backslashes_buf = in(reg) backslashes_buf.as_mut_ptr(),
            opens_buf = in(reg) opens_buf.as_mut_ptr(),
            closes_buf = in(reg) closes_buf.as_mut_ptr(),
            delims_buf = in(reg) delims_buf.as_mut_ptr(),
            value_chars_buf = in(reg) value_chars_buf.as_mut_ptr(),
            out("z0") _,
            out("z1") _,
            out("z2") _,
            out("p0") _,
            out("p1") _,
            out("p2") _,
            options(nostack)
        );
    }

    // Convert buffers to bitmasks
    fn buf_to_mask(buf: &[u8; 16], len: usize) -> u16 {
        let mut mask = 0u16;
        for (i, &byte) in buf.iter().enumerate().take(len) {
            if byte != 0 {
                mask |= 1u16 << i;
            }
        }
        mask
    }

    let quotes = buf_to_mask(&quotes_buf, chunk_len);
    let backslashes = buf_to_mask(&backslashes_buf, chunk_len);

    CharClass {
        quotes,
        backslashes,
        opens: buf_to_mask(&opens_buf, chunk_len),
        closes: buf_to_mask(&closes_buf, chunk_len),
        delims: buf_to_mask(&delims_buf, chunk_len),
        value_chars: buf_to_mask(&value_chars_buf, chunk_len),
        string_special: quotes | backslashes,
        len: chunk_len,
    }
}

/// Process a chunk and update IB/BP writers.
/// Returns the new state after processing all bytes.
#[inline]
fn process_chunk_standard(
    class: CharClass,
    mut state: State,
    ib: &mut BitWriter,
    bp: &mut BitWriter,
) -> State {
    let len = class.len;
    let mut i = 0;

    while i < len {
        let remaining_mask = !((1u16 << i) - 1); // Mask of positions >= i

        match state {
            State::InJson => {
                let bit = 1u16 << i;
                let is_open = (class.opens & bit) != 0;
                let is_close = (class.closes & bit) != 0;
                let is_quote = (class.quotes & bit) != 0;
                let is_value_char = (class.value_chars & bit) != 0;

                if is_open {
                    bp.write_1();
                    ib.write_1();
                } else if is_close {
                    bp.write_0();
                    ib.write_0();
                } else if is_quote {
                    // Value start: BP=10, IB=1
                    bp.write_1();
                    bp.write_0();
                    ib.write_1();
                    state = State::InString;
                } else if is_value_char {
                    // Value start: BP=10, IB=1
                    bp.write_1();
                    bp.write_0();
                    ib.write_1();
                    state = State::InValue;
                } else {
                    ib.write_0();
                }
                i += 1;
            }
            State::InString => {
                // Fast path: find the next quote or backslash
                let special_remaining = class.string_special & remaining_mask;

                if special_remaining == 0 {
                    // No more quotes or backslashes in this chunk - write zeros for rest
                    let zeros_to_write = len - i;
                    ib.write_zeros(zeros_to_write);
                    return State::InString;
                }

                // Find the next special character
                let next_special = special_remaining.trailing_zeros() as usize;

                // Write zeros up to (but not including) the special char
                if next_special > i {
                    let zeros = next_special - i;
                    ib.write_zeros(zeros);
                    i = next_special;
                }

                // Now process the special character at position i
                let bit = 1u16 << i;
                ib.write_0();

                if (class.quotes & bit) != 0 {
                    state = State::InJson;
                } else {
                    // It's a backslash
                    state = State::InEscape;
                }
                i += 1;
            }
            State::InEscape => {
                ib.write_0();
                state = State::InString;
                i += 1;
            }
            State::InValue => {
                let bit = 1u16 << i;
                let is_open = (class.opens & bit) != 0;
                let is_close = (class.closes & bit) != 0;
                let is_quote = (class.quotes & bit) != 0;
                let is_value_char = (class.value_chars & bit) != 0;

                if is_open {
                    bp.write_1();
                    ib.write_1();
                    state = State::InJson;
                } else if is_close {
                    bp.write_0();
                    ib.write_0();
                    state = State::InJson;
                } else if is_quote {
                    // Value start: BP=10, IB=1
                    bp.write_1();
                    bp.write_0();
                    ib.write_1();
                    state = State::InString;
                } else if is_value_char {
                    ib.write_0();
                } else {
                    ib.write_0();
                    state = State::InJson;
                }
                i += 1;
            }
        }
    }

    state
}

/// Process a chunk for simple cursor and update IB/BP writers.
#[inline]
fn process_chunk_simple(
    class: CharClass,
    mut state: SimpleState,
    ib: &mut BitWriter,
    bp: &mut BitWriter,
) -> SimpleState {
    for i in 0..class.len {
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

/// Build a semi-index from JSON bytes using SVE2-accelerated Standard Cursor algorithm.
///
/// # Safety
///
/// Requires SVE2. Caller must verify with `is_aarch64_feature_detected!("sve2")`.
#[target_feature(enable = "sve2")]
pub unsafe fn build_semi_index_standard(json: &[u8]) -> SemiIndex {
    let word_capacity = json.len().div_ceil(64);
    let mut ib = BitWriter::with_capacity(word_capacity);
    let mut bp = BitWriter::with_capacity(word_capacity * 2);
    let mut state = State::InJson;

    let mut offset = 0;

    // Process 16-byte chunks (matches NEON for 128-bit SVE2)
    while offset < json.len() {
        let remaining = json.len() - offset;
        let class = unsafe { classify_chars_sve2_simd(json.as_ptr().add(offset), remaining) };
        state = process_chunk_standard(class, state, &mut ib, &mut bp);
        offset += class.len;
    }

    SemiIndex {
        state,
        ib: ib.finish(),
        bp: bp.finish(),
    }
}

/// Build a semi-index from JSON bytes using SVE2-accelerated Simple Cursor algorithm.
///
/// # Safety
///
/// Requires SVE2. Caller must verify with `is_aarch64_feature_detected!("sve2")`.
#[target_feature(enable = "sve2")]
pub unsafe fn build_semi_index_simple(json: &[u8]) -> SimpleSemiIndex {
    let word_capacity = json.len().div_ceil(64);
    let mut ib = BitWriter::with_capacity(word_capacity);
    let mut bp = BitWriter::with_capacity(word_capacity * 2);
    let mut state = SimpleState::InJson;

    let mut offset = 0;

    // Process 16-byte chunks (matches NEON for 128-bit SVE2)
    while offset < json.len() {
        let remaining = json.len() - offset;
        let class = unsafe { classify_chars_sve2_simd(json.as_ptr().add(offset), remaining) };
        state = process_chunk_simple(class, state, &mut ib, &mut bp);
        offset += class.len;
    }

    SimpleSemiIndex {
        state,
        ib: ib.finish(),
        bp: bp.finish(),
    }
}

/// Check if SVE2 is available at runtime.
#[cfg(feature = "std")]
#[inline]
pub fn has_sve2() -> bool {
    std::arch::is_aarch64_feature_detected!("sve2")
}

/// Check if SVE2 is available (no_std version - always false).
#[cfg(not(feature = "std"))]
#[inline]
pub const fn has_sve2() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_sve2_runtime() -> bool {
        #[cfg(feature = "std")]
        {
            std::arch::is_aarch64_feature_detected!("sve2")
        }
        #[cfg(not(feature = "std"))]
        {
            false
        }
    }

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
    fn test_sve2_matches_scalar_empty_object() {
        if !has_sve2_runtime() {
            eprintln!("Skipping SVE2 test: CPU doesn't support SVE2");
            return;
        }

        let json = b"{}";
        let sve2_result = unsafe { build_semi_index_standard(json) };
        let scalar_result = crate::json::standard::build_semi_index(json);

        assert_eq!(
            bits_to_string(&sve2_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch"
        );
        assert_eq!(sve2_result.state, scalar_result.state);
    }

    #[test]
    fn test_sve2_matches_scalar_simple_object() {
        if !has_sve2_runtime() {
            return;
        }

        let json = br#"{"a":"b"}"#;
        let sve2_result = unsafe { build_semi_index_standard(json) };
        let scalar_result = crate::json::standard::build_semi_index(json);

        assert_eq!(
            bits_to_string(&sve2_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch"
        );
        assert_eq!(sve2_result.state, scalar_result.state);
    }

    #[test]
    fn test_sve2_matches_scalar_array() {
        if !has_sve2_runtime() {
            return;
        }

        let json = b"[1,2,3]";
        let sve2_result = unsafe { build_semi_index_standard(json) };
        let scalar_result = crate::json::standard::build_semi_index(json);

        assert_eq!(
            bits_to_string(&sve2_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch"
        );
    }

    #[test]
    fn test_sve2_matches_scalar_nested() {
        if !has_sve2_runtime() {
            return;
        }

        let json = br#"{"a":{"b":1}}"#;
        let sve2_result = unsafe { build_semi_index_standard(json) };
        let scalar_result = crate::json::standard::build_semi_index(json);

        assert_eq!(
            bits_to_string(&sve2_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch"
        );
    }

    #[test]
    fn test_sve2_matches_scalar_escaped() {
        if !has_sve2_runtime() {
            return;
        }

        let json = br#"{"a":"b\"c"}"#;
        let sve2_result = unsafe { build_semi_index_standard(json) };
        let scalar_result = crate::json::standard::build_semi_index(json);

        assert_eq!(
            bits_to_string(&sve2_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch"
        );
    }

    #[test]
    fn test_sve2_matches_scalar_long_input() {
        if !has_sve2_runtime() {
            return;
        }

        let json = br#"{"name":"value","number":12345,"array":[1,2,3]}"#;
        let sve2_result = unsafe { build_semi_index_standard(json) };
        let scalar_result = crate::json::standard::build_semi_index(json);

        assert_eq!(
            bits_to_string(&sve2_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch for long input"
        );
        assert_eq!(sve2_result.state, scalar_result.state);
    }

    #[test]
    fn test_sve2_simple_matches_scalar() {
        if !has_sve2_runtime() {
            return;
        }

        let json = br#"{"a":"b"}"#;
        let sve2_result = unsafe { build_semi_index_simple(json) };
        let scalar_result = crate::json::simple::build_semi_index(json);

        assert_eq!(
            bits_to_string(&sve2_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch"
        );
        assert_eq!(sve2_result.state, scalar_result.state);
    }
}
