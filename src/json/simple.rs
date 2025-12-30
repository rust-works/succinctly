//! Simple Cursor JSON semi-indexing.
//!
//! The Simple Cursor uses a 3-state machine to generate Interest Bits (IB) and
//! Balanced Parentheses (BP) vectors from JSON text.
//!
//! ## States
//! - `InJson`: Normal JSON parsing mode
//! - `InString`: Inside a quoted string
//! - `InEscape`: Just encountered a backslash inside a string
//!
//! ## BP Encoding
//! - `{` or `[`: writes `11` (two open parens)
//! - `}` or `]`: writes `00` (two close parens)
//! - `,` or `:`: writes `01` (close then open)
//!
//! ## IB Encoding
//! - `{`, `}`, `[`, `]`, `,`, `:` in InJson state: IB = 1
//! - All other bytes: IB = 0

#[cfg(not(test))]
use alloc::vec::Vec;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::BitWriter;

/// ASCII byte constants
const DOUBLE_QUOTE: u8 = b'"';
const BACKSLASH: u8 = b'\\';
const OPEN_BRACE: u8 = b'{';
const CLOSE_BRACE: u8 = b'}';
const OPEN_BRACKET: u8 = b'[';
const CLOSE_BRACKET: u8 = b']';
const COMMA: u8 = b',';
const COLON: u8 = b':';

/// State machine states for Simple Cursor
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum State {
    /// Normal JSON parsing mode
    InJson,
    /// Inside a quoted string
    InString,
    /// Just encountered a backslash inside a string (next char is escaped)
    InEscape,
}

/// Result of simple cursor semi-indexing.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct SemiIndex {
    /// Final state after processing
    pub state: State,
    /// Interest bits - one bit per input byte
    pub ib: Vec<u64>,
    /// Balanced parentheses encoding
    pub bp: Vec<u64>,
}

impl SemiIndex {
    /// Get the interest bits as raw bytes for binary serialization.
    #[inline]
    pub fn ib_as_bytes(&self) -> &[u8] {
        crate::binary::words_to_bytes(&self.ib)
    }

    /// Get the balanced parentheses as raw bytes for binary serialization.
    #[inline]
    pub fn bp_as_bytes(&self) -> &[u8] {
        crate::binary::words_to_bytes(&self.bp)
    }

    /// Create a SemiIndex from raw byte slices.
    ///
    /// # Panics
    ///
    /// Panics if either byte slice length is not a multiple of 8.
    pub fn from_bytes(ib_bytes: &[u8], bp_bytes: &[u8]) -> Self {
        Self {
            state: State::InJson, // Assume complete JSON
            ib: crate::binary::bytes_to_words_vec(ib_bytes),
            bp: crate::binary::bytes_to_words_vec(bp_bytes),
        }
    }
}

/// Check if byte is an opening bracket or brace.
#[inline]
fn is_open(c: u8) -> bool {
    c == OPEN_BRACKET || c == OPEN_BRACE
}

/// Check if byte is a closing bracket or brace.
#[inline]
fn is_close(c: u8) -> bool {
    c == CLOSE_BRACKET || c == CLOSE_BRACE
}

/// Check if byte is a delimiter (comma or colon).
#[inline]
fn is_delim(c: u8) -> bool {
    c == COMMA || c == COLON
}

/// Build a semi-index from JSON bytes using the Simple Cursor algorithm.
///
/// # Example
///
/// ```
/// use succinctly::json::simple::build_semi_index;
///
/// let json = br#"{"a":1}"#;
/// let semi = build_semi_index(json);
///
/// // IB marks structural characters: { " : " } at positions 0, 1, 3, 5, 6
/// // But Simple Cursor only marks {, :, } (not quotes)
/// // Position:  0 1 2 3 4 5 6
/// // Char:      { " a " : 1 }
/// // IB:        1 0 0 0 1 0 1
/// ```
pub fn build_semi_index(json: &[u8]) -> SemiIndex {
    let word_capacity = json.len().div_ceil(64);
    let mut ib = BitWriter::with_capacity(word_capacity);
    let mut bp = BitWriter::with_capacity(word_capacity * 2);
    let mut state = State::InJson;

    for &c in json {
        match state {
            State::InJson => {
                if is_open(c) {
                    // { or [: write BP=11, IB=1
                    bp.write_1();
                    bp.write_1();
                    ib.write_1();
                } else if is_close(c) {
                    // } or ]: write BP=00, IB=1
                    bp.write_0();
                    bp.write_0();
                    ib.write_1();
                } else if is_delim(c) {
                    // , or :: write BP=01, IB=1
                    bp.write_0();
                    bp.write_1();
                    ib.write_1();
                } else if c == DOUBLE_QUOTE {
                    // Start of string: IB=0, transition to InString
                    ib.write_0();
                    state = State::InString;
                } else {
                    // Whitespace or other: IB=0
                    ib.write_0();
                }
            }
            State::InString => {
                // Always IB=0 inside strings
                ib.write_0();
                if c == DOUBLE_QUOTE {
                    state = State::InJson;
                } else if c == BACKSLASH {
                    state = State::InEscape;
                }
                // Otherwise stay in InString
            }
            State::InEscape => {
                // Escaped character: IB=0, return to InString
                ib.write_0();
                state = State::InString;
            }
        }
    }

    SemiIndex {
        state,
        ib: ib.finish(),
        bp: bp.finish(),
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
    fn test_empty_object() {
        let semi = build_semi_index(b"{}");
        // IB: 1 1 (both { and } are marked)
        assert_eq!(bits_to_string(&semi.ib, 2), "11");
        // BP: 11 00 (open open, close close)
        assert_eq!(bits_to_string(&semi.bp, 4), "1100");
        assert_eq!(semi.state, State::InJson);
    }

    #[test]
    fn test_empty_array() {
        let semi = build_semi_index(b"[]");
        assert_eq!(bits_to_string(&semi.ib, 2), "11");
        assert_eq!(bits_to_string(&semi.bp, 4), "1100");
    }

    #[test]
    fn test_simple_string() {
        // {"a":"b"}
        let semi = build_semi_index(br#"{"a":"b"}"#);
        // Position: 0 1 2 3 4 5 6 7 8
        // Char:     { " a " : " b " }
        // IB:       1 0 0 0 1 0 0 0 1
        assert_eq!(bits_to_string(&semi.ib, 9), "100010001");
        // BP for { : } -> 11 01 00
        assert_eq!(bits_to_string(&semi.bp, 6), "110100");
    }

    #[test]
    fn test_array_with_values() {
        // [1,2,3]
        let semi = build_semi_index(b"[1,2,3]");
        // Position: 0 1 2 3 4 5 6
        // Char:     [ 1 , 2 , 3 ]
        // IB:       1 0 1 0 1 0 1
        assert_eq!(bits_to_string(&semi.ib, 7), "1010101");
        // BP: [ -> 11, , -> 01, , -> 01, ] -> 00
        // So: 11 01 01 00
        assert_eq!(bits_to_string(&semi.bp, 8), "11010100");
    }

    #[test]
    fn test_nested_object() {
        // {"a":{"b":1}}
        let semi = build_semi_index(br#"{"a":{"b":1}}"#);
        // Position:  0 1 2 3 4 5 6 7 8 9 10 11 12
        // Char:      { " a " : { " b " :  1  }  }
        // IB:        1 0 0 0 1 1 0 0 0 1  0  1  1
        assert_eq!(bits_to_string(&semi.ib, 13), "1000110001011");
    }

    #[test]
    fn test_escaped_quote() {
        // "a\"b" (string with escaped quote)
        let semi = build_semi_index(br#""a\"b""#);
        // Position: 0 1 2 3 4 5
        // Char:     " a \ " b "
        // IB:       0 0 0 0 0 0 (all inside string or quotes)
        assert_eq!(bits_to_string(&semi.ib, 6), "000000");
        // No BP output (no structural chars)
        assert!(semi.bp.is_empty() || semi.bp[0] == 0);
    }

    #[test]
    fn test_escaped_backslash() {
        // "a\\b" (string with escaped backslash)
        let semi = build_semi_index(br#""a\\b""#);
        // The \\ is an escape sequence, so the second \ is the escaped char
        // Position: 0 1 2 3 4 5
        // Char:     " a \ \ b "
        // State:    S S S E S S -> J at end
        // All IB = 0
        assert_eq!(bits_to_string(&semi.ib, 6), "000000");
        assert_eq!(semi.state, State::InJson);
    }

    #[test]
    fn test_whitespace_ignored() {
        // { "a" : 1 } with spaces
        let semi = build_semi_index(b"{ \"a\" : 1 }");
        // Position: 0 1 2 3 4 5 6 7 8 9 10
        // Char:     {   " a "   :   1   }
        // IB:       1 0 0 0 0 0 1 0 0 0  1
        assert_eq!(bits_to_string(&semi.ib, 11), "10000010001");
    }

    #[test]
    fn test_final_state_in_string() {
        // Unterminated string
        let semi = build_semi_index(br#"{"a"#);
        assert_eq!(semi.state, State::InString);
    }

    #[test]
    fn test_final_state_in_escape() {
        // String ending with backslash
        let semi = build_semi_index(br#""\"#);
        assert_eq!(semi.state, State::InEscape);
    }
}
