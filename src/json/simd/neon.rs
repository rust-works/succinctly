//! NEON-accelerated JSON semi-indexing for ARM64.
//!
//! Processes 16 bytes at a time using ARM NEON SIMD instructions.
//!
//! Uses nibble-based lookup tables (simdjson technique) for efficient
//! character classification with just 2 table lookups + 1 AND per chunk.

use core::arch::aarch64::*;

use crate::json::BitWriter;
use crate::json::simple::{SemiIndex as SimpleSemiIndex, State as SimpleState};
use crate::json::standard::{SemiIndex, State};

// Nibble lookup tables for character classification.
// Each byte in the result has bit flags indicating character class:
//   Bit 0: opens ({ [)
//   Bit 1: closes (} ])
//   Bit 2: delims (, :)
//   Bit 3: quote (")
//   Bit 4: backslash (\)
//
// The tables are designed so that lo_table[lo_nibble] & hi_table[hi_nibble]
// produces the correct flags for each ASCII character.

/// Low nibble lookup table (indexed by byte & 0x0F)
const LO_NIBBLE_TABLE: [u8; 16] = [
    0x00, // 0
    0x00, // 1
    0x08, // 2: quote (") has lo=2
    0x00, // 3
    0x00, // 4
    0x00, // 5
    0x00, // 6
    0x00, // 7
    0x00, // 8
    0x00, // 9
    0x04, // A: colon (:) has lo=A
    0x01, // B: opens ({ [) have lo=B
    0x14, // C: comma (,) has lo=C (bit 2), backslash (\) has lo=C (bit 4)
    0x02, // D: closes (} ]) have lo=D
    0x00, // E
    0x00, // F
];

/// High nibble lookup table (indexed by byte >> 4)
const HI_NIBBLE_TABLE: [u8; 16] = [
    0x00, // 0
    0x00, // 1
    0x0C, // 2: comma (,) and quote (") have hi=2 (bits 2,3)
    0x04, // 3: colon (:) has hi=3 (bit 2)
    0x00, // 4
    0x13, // 5: [ ] \ have hi=5 (bits 0,1,4)
    0x00, // 6
    0x03, // 7: { } have hi=7 (bits 0,1)
    0x00, // 8
    0x00, // 9
    0x00, // A
    0x00, // B
    0x00, // C
    0x00, // D
    0x00, // E
    0x00, // F
];

// Classification result bit flags
const FLAG_OPEN: u8 = 0x01;
const FLAG_CLOSE: u8 = 0x02;
const FLAG_DELIM: u8 = 0x04;
const FLAG_QUOTE: u8 = 0x08;
const FLAG_BACKSLASH: u8 = 0x10;

// Value character detection lookup tables.
// These use a different encoding than structural chars because value chars
// include ranges (0-9, A-Z, a-z) that don't map cleanly to nibble AND.
//
// Strategy: Use bit flags that indicate character classes, then AND the
// lo and hi results. A character is a value char if the AND result has
// any bit set in the VALUE_CHAR_MASK.
//
// Encoding:
//   Bit 0: Could be digit (hi=3 with lo=0-9)
//   Bit 1: Could be uppercase (hi=4 with lo=1-F, or hi=5 with lo=0-A)
//   Bit 2: Could be lowercase (hi=6 with lo=1-F, or hi=7 with lo=0-A)
//   Bit 3: Could be punct +/- (hi=2 with lo=B,D)
//   Bit 4: Could be period (hi=2 with lo=E)

/// Low nibble lookup for value chars
const VALUE_LO_TABLE: [u8; 16] = [
    0x07, // 0: digit, upper (0x50), lower (0x70)
    0x07, // 1: digit, upper (0x41, 0x51), lower (0x61, 0x71)
    0x07, // 2: digit, upper (0x42, 0x52), lower (0x62, 0x72)
    0x07, // 3: digit, upper (0x43, 0x53), lower (0x63, 0x73)
    0x07, // 4: digit, upper (0x44, 0x54), lower (0x64, 0x74)
    0x07, // 5: digit, upper (0x45, 0x55), lower (0x65, 0x75)
    0x07, // 6: digit, upper (0x46, 0x56), lower (0x66, 0x76)
    0x07, // 7: digit, upper (0x47, 0x57), lower (0x67, 0x77)
    0x07, // 8: digit, upper (0x48, 0x58), lower (0x68, 0x78)
    0x07, // 9: digit, upper (0x49, 0x59), lower (0x69, 0x79)
    0x06, // A: upper (0x4A, 0x5A), lower (0x6A, 0x7A) - NOT digit
    0x0E, // B: upper (0x4B), lower (0x6B), punct '+' (0x2B)
    0x06, // C: upper (0x4C), lower (0x6C)
    0x0E, // D: upper (0x4D), lower (0x6D), punct '-' (0x2D)
    0x16, // E: upper (0x4E), lower (0x6E), period '.' (0x2E)
    0x06, // F: upper (0x4F), lower (0x6F)
];

/// High nibble lookup for value chars
const VALUE_HI_TABLE: [u8; 16] = [
    0x00, // 0: nothing
    0x00, // 1: nothing
    0x18, // 2: punct (+,-) and period (.)
    0x01, // 3: digits
    0x02, // 4: uppercase A-O (0x41-0x4F)
    0x02, // 5: uppercase P-Z (0x50-0x5A)
    0x04, // 6: lowercase a-o (0x61-0x6F)
    0x04, // 7: lowercase p-z (0x70-0x7A)
    0x00, // 8: nothing
    0x00, // 9: nothing
    0x00, // A: nothing
    0x00, // B: nothing
    0x00, // C: nothing
    0x00, // D: nothing
    0x00, // E: nothing
    0x00, // F: nothing
];

/// Extract a bitmask from the high bit of each byte in a NEON vector.
/// Returns a u16 where bit i is set if byte i has its high bit set.
///
/// Uses optimized fixed-shift approach that avoids slow variable shifts.
/// The key insight: variable shifts (vshlq_u8 with vector amounts) are slow on M1,
/// while fixed shifts and pairwise adds are fast.
#[inline]
#[target_feature(enable = "neon")]
unsafe fn neon_movemask(v: uint8x16_t) -> u16 {
    // Input: bytes with 0x00 or 0xFF (from comparison results)
    // We need to extract the high bit of each byte into a u16 bitmask.
    //
    // Strategy: Extract to scalar and use multiplication trick.
    // This is fast on M1 due to efficient NEON<->scalar transfer.
    //
    // Step 1: Shift right by 7 to get 0 or 1 in each byte
    let high_bits = vshrq_n_u8::<7>(v);

    // Step 2: Extract the 16 bytes as two u64 values
    let low_u64 = vgetq_lane_u64::<0>(vreinterpretq_u64_u8(high_bits));
    let high_u64 = vgetq_lane_u64::<1>(vreinterpretq_u64_u8(high_bits));

    // Step 3: Pack 8 bytes into 8 bits using multiplication trick
    // Multiply by magic number that positions each byte at the right bit
    // 0x0102040810204080 positions bytes 0-7 at bits 0-7 in the high byte
    const MAGIC: u64 = 0x0102040810204080;

    // Each byte in high_bits is 0 or 1
    // Multiply spreads them to different bit positions
    // The high byte contains all 8 bits packed
    let low_packed = (low_u64.wrapping_mul(MAGIC) >> 56) as u8;
    let high_packed = (high_u64.wrapping_mul(MAGIC) >> 56) as u8;

    (low_packed as u16) | ((high_packed as u16) << 8)
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
    /// Combined mask of quotes OR backslashes (for quick InString scanning)
    string_special: u16,
}

/// Character classification results for a 32-byte double chunk.
/// Combines two 16-byte CharClass results into u32 masks.
#[derive(Debug, Clone, Copy)]
struct CharClass32 {
    /// Mask of bytes that are '"'
    quotes: u32,
    /// Mask of bytes that are '\'
    backslashes: u32,
    /// Mask of bytes that are '{' or '['
    opens: u32,
    /// Mask of bytes that are '}' or ']'
    closes: u32,
    /// Mask of bytes that are ',' or ':'
    delims: u32,
    /// Mask of bytes that could start/continue a value (alphanumeric, ., -, +)
    value_chars: u32,
    /// Combined mask of quotes OR backslashes (for quick InString scanning)
    string_special: u32,
}

impl CharClass32 {
    /// Combine two 16-byte CharClass results into a 32-byte classification.
    #[inline]
    fn from_pair(lo: CharClass, hi: CharClass) -> Self {
        Self {
            quotes: (lo.quotes as u32) | ((hi.quotes as u32) << 16),
            backslashes: (lo.backslashes as u32) | ((hi.backslashes as u32) << 16),
            opens: (lo.opens as u32) | ((hi.opens as u32) << 16),
            closes: (lo.closes as u32) | ((hi.closes as u32) << 16),
            delims: (lo.delims as u32) | ((hi.delims as u32) << 16),
            value_chars: (lo.value_chars as u32) | ((hi.value_chars as u32) << 16),
            string_special: (lo.string_special as u32) | ((hi.string_special as u32) << 16),
        }
    }
}

/// Classify 16 bytes at once using NEON with nibble lookup tables.
///
/// Uses the simdjson technique: split each byte into high/low nibbles,
/// lookup both in precomputed tables, AND the results to get classification.
/// This replaces ~12 comparisons + ~8 ORs with just 2 lookups + 1 AND.
#[inline]
#[target_feature(enable = "neon")]
unsafe fn classify_chars(chunk: uint8x16_t) -> CharClass {
    unsafe {
        // Load nibble lookup tables into NEON registers
        let lo_table = vld1q_u8(LO_NIBBLE_TABLE.as_ptr());
        let hi_table = vld1q_u8(HI_NIBBLE_TABLE.as_ptr());

        // Split each byte into nibbles
        let lo_nibble = vandq_u8(chunk, vdupq_n_u8(0x0F));
        let hi_nibble = vshrq_n_u8::<4>(chunk);

        // Lookup both nibbles in their respective tables
        let lo_result = vqtbl1q_u8(lo_table, lo_nibble);
        let hi_result = vqtbl1q_u8(hi_table, hi_nibble);

        // AND the results to get final classification
        // Each byte now has bit flags for the character classes
        let classified = vandq_u8(lo_result, hi_result);

        // Extract individual class masks using vtstq_u8 (test bits)
        // vtstq_u8 returns 0xFF if (a & b) != 0, else 0x00
        let flag_open_vec = vdupq_n_u8(FLAG_OPEN);
        let flag_close_vec = vdupq_n_u8(FLAG_CLOSE);
        let flag_delim_vec = vdupq_n_u8(FLAG_DELIM);
        let flag_quote_vec = vdupq_n_u8(FLAG_QUOTE);
        let flag_backslash_vec = vdupq_n_u8(FLAG_BACKSLASH);

        let opens_vec = vtstq_u8(classified, flag_open_vec);
        let closes_vec = vtstq_u8(classified, flag_close_vec);
        let delims_vec = vtstq_u8(classified, flag_delim_vec);
        let quotes_vec = vtstq_u8(classified, flag_quote_vec);
        let backslashes_vec = vtstq_u8(classified, flag_backslash_vec);

        // Value chars detection using nibble lookup tables
        // This replaces 13 NEON operations with 4 (2 lookups + 1 AND + 1 compare)
        let value_lo_table = vld1q_u8(VALUE_LO_TABLE.as_ptr());
        let value_hi_table = vld1q_u8(VALUE_HI_TABLE.as_ptr());

        let value_lo_result = vqtbl1q_u8(value_lo_table, lo_nibble);
        let value_hi_result = vqtbl1q_u8(value_hi_table, hi_nibble);

        // AND the results - if any bit is set, the character is a value char
        let value_classified = vandq_u8(value_lo_result, value_hi_result);

        // Check if any bit is set (non-zero means it's a value char)
        // Compare with zero and invert: value_chars = (value_classified != 0)
        let zero = vdupq_n_u8(0);
        let value_chars = vmvnq_u8(vceqq_u8(value_classified, zero));

        // Convert vector masks to u16 bitmasks
        let quotes = neon_movemask(quotes_vec);
        let backslashes = neon_movemask(backslashes_vec);

        CharClass {
            quotes,
            backslashes,
            opens: neon_movemask(opens_vec),
            closes: neon_movemask(closes_vec),
            delims: neon_movemask(delims_vec),
            value_chars: neon_movemask(value_chars),
            string_special: quotes | backslashes,
        }
    }
}

/// Process a 16-byte chunk and update IB/BP writers.
/// Returns the new state after processing all 16 bytes.
///
/// This is the original serial implementation - kept for correctness reference.
#[inline]
#[allow(dead_code)]
fn process_chunk_standard_serial(
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

/// Process a 16-byte chunk and update IB/BP writers.
/// Returns the new state after processing all 16 bytes.
///
/// Uses optimized scanning with fast-path for string content:
/// - When inside a string with no quotes/backslashes, batch-write zeros
/// - Uses trailing_zeros to skip to next interesting character
/// - Uses write_bits for common multi-bit patterns (BP=10 for value start)
#[inline]
fn process_chunk_standard(
    class: CharClass,
    mut state: State,
    ib: &mut BitWriter,
    bp: &mut BitWriter,
    bytes: &[u8],
) -> State {
    let len = bytes.len().min(16);
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

/// Process a 32-byte double chunk and update IB/BP writers.
/// Returns the new state after processing all 32 bytes.
///
/// This processes two 16-byte chunks together, reducing loop overhead
/// and potentially improving instruction-level parallelism.
#[inline]
fn process_chunk_standard_32(
    class: CharClass32,
    mut state: State,
    ib: &mut BitWriter,
    bp: &mut BitWriter,
    len: usize,
) -> State {
    let len = len.min(32);
    let mut i = 0;

    while i < len {
        let remaining_mask = !((1u32 << i) - 1); // Mask of positions >= i

        match state {
            State::InJson => {
                let bit = 1u32 << i;
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
                let bit = 1u32 << i;
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
                let bit = 1u32 << i;
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

/// Build a semi-index from JSON bytes using SIMD-accelerated Standard Cursor algorithm.
///
/// Processes 32 bytes at a time using ARM NEON instructions for character
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

        // Process 32-byte chunks (two 16-byte NEON loads)
        while offset + 32 <= json.len() {
            let chunk_lo = vld1q_u8(json.as_ptr().add(offset));
            let chunk_hi = vld1q_u8(json.as_ptr().add(offset + 16));
            let class_lo = classify_chars(chunk_lo);
            let class_hi = classify_chars(chunk_hi);
            let class32 = CharClass32::from_pair(class_lo, class_hi);
            state = process_chunk_standard_32(class32, state, &mut ib, &mut bp, 32);
            offset += 32;
        }

        // Process remaining 16-byte chunk if present
        if offset + 16 <= json.len() {
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
