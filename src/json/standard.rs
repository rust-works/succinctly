//! Standard Cursor JSON semi-indexing.
//!
//! The Standard Cursor uses a 4-state machine to generate Interest Bits (IB) and
//! Balanced Parentheses (BP) vectors from JSON text. Unlike the Simple Cursor,
//! it tracks when we're inside a value (numbers, booleans, null).
//!
//! ## States
//! - `InJson`: Normal JSON parsing mode
//! - `InString`: Inside a quoted string
//! - `InEscape`: Just encountered a backslash inside a string
//! - `InValue`: Inside an unquoted value (number, boolean, null)
//!
//! ## BP Encoding
//! - `{` or `[`: writes `1` (open)
//! - `}` or `]`: writes `0` (close)
//! - Start of string or value: writes `10` (open then close - leaf node)
//!
//! ## IB Encoding
//! - `{`, `[`: IB = 1
//! - `}`, `]`: IB = 0
//! - Start of string or value (first char): IB = 1

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
const PERIOD: u8 = b'.';
const MINUS: u8 = b'-';
const PLUS: u8 = b'+';

/// State machine states for Standard Cursor
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum State {
    /// Normal JSON parsing mode
    InJson,
    /// Inside a quoted string
    InString,
    /// Just encountered a backslash inside a string (next char is escaped)
    InEscape,
    /// Inside an unquoted value (number, boolean, null)
    InValue,
}

/// Result of standard cursor semi-indexing.
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

/// Check if byte is alphabetic (a-z, A-Z).
#[inline]
fn is_alphabetic(c: u8) -> bool {
    c.is_ascii_alphabetic()
}

/// Check if byte is a digit (0-9).
#[inline]
fn is_digit(c: u8) -> bool {
    c.is_ascii_digit()
}

/// Check if byte can be part of a JSON value (number, boolean, null).
/// Includes: alphanumeric, period, minus, plus
#[inline]
fn is_value_char(c: u8) -> bool {
    is_alphabetic(c) || is_digit(c) || c == PERIOD || c == MINUS || c == PLUS
}

/// Output type from state machine (3-bit value).
/// - Bit 0 (0b001): BP close
/// - Bit 1 (0b010): BP open
/// - Bit 2 (0b100): Interest bit
#[derive(Debug, Clone, Copy)]
struct Phi(u8);

impl Phi {
    const NONE: Phi = Phi(0b000);
    const CLOSE: Phi = Phi(0b001);
    const OPEN: Phi = Phi(0b110);
    const LEAF: Phi = Phi(0b111); // open + close + interest

    #[inline]
    fn ib(self) -> bool {
        (self.0 & 0b100) != 0
    }

    #[inline]
    fn bp_open(self) -> bool {
        (self.0 & 0b010) != 0
    }

    #[inline]
    fn bp_close(self) -> bool {
        (self.0 & 0b001) != 0
    }
}

/// State machine transition: given input byte and current state,
/// returns (new_state, output).
#[inline]
fn state_machine(c: u8, state: State) -> (State, Phi) {
    match state {
        State::InJson => {
            if is_open(c) {
                (State::InJson, Phi::OPEN)
            } else if is_close(c) {
                (State::InJson, Phi::CLOSE)
            } else if is_delim(c) {
                (State::InJson, Phi::NONE)
            } else if is_value_char(c) {
                (State::InValue, Phi::LEAF)
            } else if c == DOUBLE_QUOTE {
                (State::InString, Phi::LEAF)
            } else {
                // Whitespace or other
                (State::InJson, Phi::NONE)
            }
        }
        State::InString => {
            if c == DOUBLE_QUOTE {
                (State::InJson, Phi::NONE)
            } else if c == BACKSLASH {
                (State::InEscape, Phi::NONE)
            } else {
                (State::InString, Phi::NONE)
            }
        }
        State::InEscape => {
            // Any escaped character returns to InString
            (State::InString, Phi::NONE)
        }
        State::InValue => {
            if is_open(c) {
                // Value ends, open bracket/brace follows
                (State::InJson, Phi::OPEN)
            } else if is_close(c) {
                // Value ends, close bracket/brace follows
                (State::InJson, Phi::CLOSE)
            } else if is_delim(c) {
                // Value ends with delimiter
                (State::InJson, Phi::NONE)
            } else if is_value_char(c) {
                // Continue value
                (State::InValue, Phi::NONE)
            } else {
                // Whitespace ends value
                (State::InJson, Phi::NONE)
            }
        }
    }
}

/// Build a semi-index from JSON bytes using the Standard Cursor algorithm.
///
/// # Example
///
/// ```
/// use succinctly::json::standard::build_semi_index;
///
/// let json = br#"{"a":1}"#;
/// let semi = build_semi_index(json);
///
/// // Standard cursor marks:
/// // - { as open (IB=1, BP=1)
/// // - "a" start as leaf (IB=1, BP=10)
/// // - 1 start as leaf (IB=1, BP=10)
/// // - } as close (IB=0, BP=0)
/// ```
pub fn build_semi_index(json: &[u8]) -> SemiIndex {
    let word_capacity = json.len().div_ceil(64);
    let mut ib = BitWriter::with_capacity(word_capacity);
    let mut bp = BitWriter::with_capacity(word_capacity * 2);
    let mut state = State::InJson;

    for &c in json {
        let (new_state, phi) = state_machine(c, state);
        state = new_state;

        // Write IB
        ib.write_bit(phi.ib());

        // Write BP (open first, then close if both)
        if phi.bp_open() {
            bp.write_1();
        }
        if phi.bp_close() {
            bp.write_0();
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
        // Position: 0 1
        // Char:     { }
        // IB:       1 0 (open marked, close not)
        assert_eq!(bits_to_string(&semi.ib, 2), "10");
        // BP: 1 0 (open, close)
        assert_eq!(bits_to_string(&semi.bp, 2), "10");
        assert_eq!(semi.state, State::InJson);
    }

    #[test]
    fn test_empty_array() {
        let semi = build_semi_index(b"[]");
        assert_eq!(bits_to_string(&semi.ib, 2), "10");
        assert_eq!(bits_to_string(&semi.bp, 2), "10");
    }

    #[test]
    fn test_simple_object_with_string() {
        // {"a":"b"}
        let semi = build_semi_index(br#"{"a":"b"}"#);
        // Position: 0 1 2 3 4 5 6 7 8
        // Char:     { " a " : " b " }
        // State:    J S S J J S S J J
        // IB:       1 1 0 0 0 1 0 0 0
        // (open, string start, nothing, nothing, nothing, string start, nothing, nothing, close no IB)
        assert_eq!(bits_to_string(&semi.ib, 9), "110001000");
        // BP: { -> 1, "a" -> 10, "b" -> 10, } -> 0
        // So: 1 1 0 1 0 0
        assert_eq!(bits_to_string(&semi.bp, 6), "110100");
    }

    #[test]
    fn test_array_with_numbers() {
        // [1,2,3]
        let semi = build_semi_index(b"[1,2,3]");
        // Position: 0 1 2 3 4 5 6
        // Char:     [ 1 , 2 , 3 ]
        // IB:       1 1 0 1 0 1 0
        assert_eq!(bits_to_string(&semi.ib, 7), "1101010");
        // BP: [ -> 1, 1 -> 10, 2 -> 10, 3 -> 10, ] -> 0
        // So: 1 10 10 10 0 = 1 1 0 1 0 1 0 0
        assert_eq!(bits_to_string(&semi.bp, 8), "11010100");
    }

    #[test]
    fn test_boolean_true() {
        // [true]
        let semi = build_semi_index(b"[true]");
        // Position: 0 1 2 3 4 5
        // Char:     [ t r u e ]
        // IB:       1 1 0 0 0 0
        assert_eq!(bits_to_string(&semi.ib, 6), "110000");
        // BP: [ -> 1, true -> 10, ] -> 0
        assert_eq!(bits_to_string(&semi.bp, 4), "1100");
    }

    #[test]
    fn test_boolean_false() {
        // [false]
        let semi = build_semi_index(b"[false]");
        // Position: 0 1 2 3 4 5 6
        // Char:     [ f a l s e ]
        // IB:       1 1 0 0 0 0 0
        assert_eq!(bits_to_string(&semi.ib, 7), "1100000");
    }

    #[test]
    fn test_null() {
        // [null]
        let semi = build_semi_index(b"[null]");
        // Position: 0 1 2 3 4 5
        // Char:     [ n u l l ]
        // IB:       1 1 0 0 0 0
        assert_eq!(bits_to_string(&semi.ib, 6), "110000");
    }

    #[test]
    fn test_negative_number() {
        // [-123]
        let semi = build_semi_index(b"[-123]");
        // Position: 0 1 2 3 4 5
        // Char:     [ - 1 2 3 ]
        // IB:       1 1 0 0 0 0
        assert_eq!(bits_to_string(&semi.ib, 6), "110000");
    }

    #[test]
    fn test_decimal_number() {
        // [3.14]
        let semi = build_semi_index(b"[3.14]");
        // Position: 0 1 2 3 4 5
        // Char:     [ 3 . 1 4 ]
        // IB:       1 1 0 0 0 0
        assert_eq!(bits_to_string(&semi.ib, 6), "110000");
    }

    #[test]
    fn test_nested_object() {
        // {"a":{"b":1}}
        let semi = build_semi_index(br#"{"a":{"b":1}}"#);
        // Position:  0 1 2 3 4 5 6 7 8 9 10 11 12
        // Char:      { " a " : { " b " :  1  }  }
        // IB:        1 1 0 0 0 1 1 0 0 0  1  0  0
        assert_eq!(bits_to_string(&semi.ib, 13), "1100011000100");
    }

    #[test]
    fn test_escaped_quote() {
        // "a\"b" (string with escaped quote)
        let semi = build_semi_index(br#""a\"b""#);
        // Position: 0 1 2 3 4 5
        // Char:     " a \ " b "
        // IB:       1 0 0 0 0 0 (only opening quote marked)
        assert_eq!(bits_to_string(&semi.ib, 6), "100000");
        // BP: " -> 10 (leaf)
        assert_eq!(bits_to_string(&semi.bp, 2), "10");
    }

    #[test]
    fn test_whitespace() {
        // { "a" : 1 }
        let semi = build_semi_index(b"{ \"a\" : 1 }");
        // Position: 0 1 2 3 4 5 6 7 8 9 10
        // Char:     {   " a "   :   1      }
        // State:    J J S S J J J V V J  J
        // Phi:      O - L - - - - L - -  C
        // IB:       1 0 1 0 0 0 0 1 0 0  0
        assert_eq!(bits_to_string(&semi.ib, 11), "10100000100");
    }

    #[test]
    fn test_final_state_in_value() {
        // [123 (unterminated)
        let semi = build_semi_index(b"[123");
        assert_eq!(semi.state, State::InValue);
    }

    #[test]
    fn test_final_state_in_string() {
        // ["abc (unterminated)
        let semi = build_semi_index(br#"["abc"#);
        assert_eq!(semi.state, State::InString);
    }

    #[test]
    fn test_scientific_notation() {
        // [1e+10]
        let semi = build_semi_index(b"[1e+10]");
        // Position: 0 1 2 3 4 5 6
        // Char:     [ 1 e + 1 0 ]
        // IB:       1 1 0 0 0 0 0
        assert_eq!(bits_to_string(&semi.ib, 7), "1100000");
    }

    #[test]
    fn test_complex_json() {
        // {"items":[1,2],"flag":true}
        let json = br#"{"items":[1,2],"flag":true}"#;
        let semi = build_semi_index(json);

        // Should end in InJson state
        assert_eq!(semi.state, State::InJson);

        // BP should be balanced (equal opens and closes)
        // Just verify we have some BP output
        assert!(!semi.bp.is_empty());
    }
}
