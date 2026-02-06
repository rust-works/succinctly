//! UTF-8 validation with detailed error reporting.
//!
//! This module provides UTF-8 validation that reports:
//! - The exact byte offset of the error
//! - The line number (1-indexed)
//! - The column number (1-indexed, in bytes)
//! - The specific type of UTF-8 violation
//!
//! ## UTF-8 Encoding Rules
//!
//! UTF-8 is a variable-width encoding that uses 1-4 bytes per character:
//!
//! | Bytes | First byte    | Continuation bytes | Code point range     |
//! |-------|---------------|-------------------|----------------------|
//! | 1     | `0xxxxxxx`    | -                 | U+0000 - U+007F      |
//! | 2     | `110xxxxx`    | `10xxxxxx`        | U+0080 - U+07FF      |
//! | 3     | `1110xxxx`    | `10xxxxxx` × 2    | U+0800 - U+FFFF      |
//! | 4     | `11110xxx`    | `10xxxxxx` × 3    | U+10000 - U+10FFFF   |
//!
//! ## Validation Checks
//!
//! The validator checks for:
//! 1. **Invalid lead bytes**: Bytes 0x80-0xBF appearing where a lead byte is expected
//! 2. **Invalid continuation bytes**: Non-continuation bytes where continuation expected
//! 3. **Overlong encodings**: Using more bytes than necessary (security vulnerability)
//! 4. **Surrogate code points**: U+D800-U+DFFF (reserved for UTF-16)
//! 5. **Out of range**: Code points above U+10FFFF
//! 6. **Truncated sequences**: Multi-byte sequence cut off at end of input

use alloc::string::String;

/// Error information for UTF-8 validation failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Utf8Error {
    /// The byte offset where the error occurred (0-indexed).
    pub offset: usize,
    /// The line number where the error occurred (1-indexed).
    pub line: usize,
    /// The column (byte position within the line, 1-indexed).
    pub column: usize,
    /// The kind of UTF-8 error.
    pub kind: Utf8ErrorKind,
}

impl core::fmt::Display for Utf8Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{} at byte {}, line {}, column {}",
            self.kind, self.offset, self.line, self.column
        )
    }
}

/// The specific type of UTF-8 validation error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Utf8ErrorKind {
    /// A byte in the range 0x80-0xBF appeared where a lead byte was expected.
    /// These bytes are only valid as continuation bytes.
    InvalidLeadByte,

    /// A byte outside the range 0x80-0xBF appeared where a continuation byte was expected.
    InvalidContinuationByte,

    /// A character was encoded using more bytes than necessary.
    /// For example, encoding ASCII 'A' (U+0041) as `C0 81` instead of `41`.
    /// This is a security vulnerability as it can bypass validation filters.
    OverlongEncoding,

    /// A surrogate code point (U+D800-U+DFFF) was encoded.
    /// These are reserved for UTF-16 surrogate pairs and invalid in UTF-8.
    SurrogateCodepoint,

    /// A code point above U+10FFFF was encoded.
    /// Unicode only defines code points up to U+10FFFF.
    OutOfRangeCodepoint,

    /// A multi-byte sequence was truncated at the end of input.
    TruncatedSequence,
}

impl core::fmt::Display for Utf8ErrorKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidLeadByte => write!(f, "invalid UTF-8 lead byte"),
            Self::InvalidContinuationByte => write!(f, "invalid UTF-8 continuation byte"),
            Self::OverlongEncoding => write!(f, "overlong UTF-8 encoding"),
            Self::SurrogateCodepoint => write!(f, "surrogate code point in UTF-8"),
            Self::OutOfRangeCodepoint => write!(f, "code point above U+10FFFF"),
            Self::TruncatedSequence => write!(f, "truncated UTF-8 sequence"),
        }
    }
}

/// Validate that the input is valid UTF-8.
///
/// Returns `Ok(())` if the input is valid UTF-8, or an `Err(Utf8Error)` with
/// detailed information about the first validation error.
///
/// # Examples
///
/// ```
/// use succinctly::text::utf8::validate_utf8;
///
/// // Valid ASCII
/// assert!(validate_utf8(b"Hello, world!").is_ok());
///
/// // Valid multi-byte UTF-8
/// assert!(validate_utf8("日本語".as_bytes()).is_ok());
/// assert!(validate_utf8("émoji: 🎉".as_bytes()).is_ok());
///
/// // Invalid: bare continuation byte
/// assert!(validate_utf8(&[0x80]).is_err());
///
/// // Invalid: truncated sequence
/// assert!(validate_utf8(&[0xC2]).is_err());
/// ```
#[inline]
pub fn validate_utf8(input: &[u8]) -> Result<(), Utf8Error> {
    validate_utf8_scalar(input)
}

/// Validate UTF-8 using a scalar (byte-by-byte) algorithm.
///
/// This is a portable implementation that works on all platforms.
/// It provides detailed error information including the exact byte offset,
/// line number, and column position of any error.
pub fn validate_utf8_scalar(input: &[u8]) -> Result<(), Utf8Error> {
    let mut pos = 0;
    let mut line = 1;
    let mut line_start = 0;
    let len = input.len();

    while pos < len {
        let byte = input[pos];

        // Track newlines for error reporting
        if pos > 0 && input[pos - 1] == b'\n' {
            line += 1;
            line_start = pos;
        }

        // Determine sequence length from lead byte
        let seq_len = match byte {
            // ASCII: 0x00-0x7F (single byte)
            0x00..=0x7F => {
                pos += 1;
                continue;
            }
            // Continuation bytes appearing as lead: invalid
            0x80..=0xBF => {
                return Err(Utf8Error {
                    offset: pos,
                    line,
                    column: pos - line_start + 1,
                    kind: Utf8ErrorKind::InvalidLeadByte,
                });
            }
            // 2-byte sequence: 0xC0-0xDF
            0xC0..=0xDF => 2,
            // 3-byte sequence: 0xE0-0xEF
            0xE0..=0xEF => 3,
            // 4-byte sequence: 0xF0-0xF7
            0xF0..=0xF7 => 4,
            // Invalid lead bytes: 0xF8-0xFF
            0xF8..=0xFF => {
                return Err(Utf8Error {
                    offset: pos,
                    line,
                    column: pos - line_start + 1,
                    kind: Utf8ErrorKind::InvalidLeadByte,
                });
            }
        };

        // Check for truncation
        if pos + seq_len > len {
            return Err(Utf8Error {
                offset: pos,
                line,
                column: pos - line_start + 1,
                kind: Utf8ErrorKind::TruncatedSequence,
            });
        }

        // Validate continuation bytes and decode code point
        match seq_len {
            2 => {
                let b1 = input[pos + 1];
                if !is_continuation_byte(b1) {
                    return Err(Utf8Error {
                        offset: pos + 1,
                        line,
                        column: pos + 1 - line_start + 1,
                        kind: Utf8ErrorKind::InvalidContinuationByte,
                    });
                }

                // Check for overlong encoding (code points < 0x80 must use 1 byte)
                // 2-byte sequences must encode U+0080 or higher
                // Lead byte 0xC0 or 0xC1 would encode < 0x80
                if byte <= 0xC1 {
                    return Err(Utf8Error {
                        offset: pos,
                        line,
                        column: pos - line_start + 1,
                        kind: Utf8ErrorKind::OverlongEncoding,
                    });
                }
            }
            3 => {
                let b1 = input[pos + 1];
                let b2 = input[pos + 2];

                if !is_continuation_byte(b1) {
                    return Err(Utf8Error {
                        offset: pos + 1,
                        line,
                        column: pos + 1 - line_start + 1,
                        kind: Utf8ErrorKind::InvalidContinuationByte,
                    });
                }
                if !is_continuation_byte(b2) {
                    return Err(Utf8Error {
                        offset: pos + 2,
                        line,
                        column: pos + 2 - line_start + 1,
                        kind: Utf8ErrorKind::InvalidContinuationByte,
                    });
                }

                // Decode code point for validation
                let cp =
                    ((byte as u32 & 0x0F) << 12) | ((b1 as u32 & 0x3F) << 6) | (b2 as u32 & 0x3F);

                // Check for overlong encoding (code points < 0x800 must use 2 bytes)
                if cp < 0x800 {
                    return Err(Utf8Error {
                        offset: pos,
                        line,
                        column: pos - line_start + 1,
                        kind: Utf8ErrorKind::OverlongEncoding,
                    });
                }

                // Check for surrogate code points (U+D800-U+DFFF)
                if (0xD800..=0xDFFF).contains(&cp) {
                    return Err(Utf8Error {
                        offset: pos,
                        line,
                        column: pos - line_start + 1,
                        kind: Utf8ErrorKind::SurrogateCodepoint,
                    });
                }
            }
            4 => {
                let b1 = input[pos + 1];
                let b2 = input[pos + 2];
                let b3 = input[pos + 3];

                if !is_continuation_byte(b1) {
                    return Err(Utf8Error {
                        offset: pos + 1,
                        line,
                        column: pos + 1 - line_start + 1,
                        kind: Utf8ErrorKind::InvalidContinuationByte,
                    });
                }
                if !is_continuation_byte(b2) {
                    return Err(Utf8Error {
                        offset: pos + 2,
                        line,
                        column: pos + 2 - line_start + 1,
                        kind: Utf8ErrorKind::InvalidContinuationByte,
                    });
                }
                if !is_continuation_byte(b3) {
                    return Err(Utf8Error {
                        offset: pos + 3,
                        line,
                        column: pos + 3 - line_start + 1,
                        kind: Utf8ErrorKind::InvalidContinuationByte,
                    });
                }

                // Decode code point for validation
                let cp = ((byte as u32 & 0x07) << 18)
                    | ((b1 as u32 & 0x3F) << 12)
                    | ((b2 as u32 & 0x3F) << 6)
                    | (b3 as u32 & 0x3F);

                // Check for overlong encoding (code points < 0x10000 must use 3 bytes)
                if cp < 0x10000 {
                    return Err(Utf8Error {
                        offset: pos,
                        line,
                        column: pos - line_start + 1,
                        kind: Utf8ErrorKind::OverlongEncoding,
                    });
                }

                // Check for out of range (> U+10FFFF)
                if cp > 0x10FFFF {
                    return Err(Utf8Error {
                        offset: pos,
                        line,
                        column: pos - line_start + 1,
                        kind: Utf8ErrorKind::OutOfRangeCodepoint,
                    });
                }
            }
            _ => unreachable!(),
        }

        pos += seq_len;
    }

    Ok(())
}

/// Check if a byte is a valid UTF-8 continuation byte (0x80-0xBF).
#[inline(always)]
fn is_continuation_byte(byte: u8) -> bool {
    (byte & 0xC0) == 0x80
}

/// Get the expected sequence length from a lead byte.
/// Returns 0 for invalid lead bytes (continuation bytes or 0xF8+).
#[inline]
pub fn sequence_length(lead_byte: u8) -> usize {
    match lead_byte {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 0, // Invalid lead byte
    }
}

/// Decode a UTF-8 code point from a byte slice.
///
/// Returns `None` if the input is empty or contains an invalid sequence.
/// On success, returns the decoded code point and the number of bytes consumed.
///
/// # Examples
///
/// ```
/// use succinctly::text::utf8::decode_code_point;
///
/// // ASCII
/// assert_eq!(decode_code_point(b"A"), Some(('A' as u32, 1)));
///
/// // Multi-byte
/// assert_eq!(decode_code_point("日".as_bytes()), Some((0x65E5, 3)));
///
/// // Empty input
/// assert_eq!(decode_code_point(b""), None);
/// ```
pub fn decode_code_point(input: &[u8]) -> Option<(u32, usize)> {
    if input.is_empty() {
        return None;
    }

    let lead = input[0];
    let len = sequence_length(lead);

    if len == 0 || input.len() < len {
        return None;
    }

    let cp = match len {
        1 => lead as u32,
        2 => {
            let b1 = input[1];
            if !is_continuation_byte(b1) {
                return None;
            }
            ((lead as u32 & 0x1F) << 6) | (b1 as u32 & 0x3F)
        }
        3 => {
            let b1 = input[1];
            let b2 = input[2];
            if !is_continuation_byte(b1) || !is_continuation_byte(b2) {
                return None;
            }
            ((lead as u32 & 0x0F) << 12) | ((b1 as u32 & 0x3F) << 6) | (b2 as u32 & 0x3F)
        }
        4 => {
            let b1 = input[1];
            let b2 = input[2];
            let b3 = input[3];
            if !is_continuation_byte(b1) || !is_continuation_byte(b2) || !is_continuation_byte(b3) {
                return None;
            }
            ((lead as u32 & 0x07) << 18)
                | ((b1 as u32 & 0x3F) << 12)
                | ((b2 as u32 & 0x3F) << 6)
                | (b3 as u32 & 0x3F)
        }
        _ => return None,
    };

    Some((cp, len))
}

/// Encode a Unicode code point as UTF-8.
///
/// Returns `None` if the code point is invalid (surrogate or > U+10FFFF).
/// On success, returns the UTF-8 bytes and the number of bytes used.
///
/// # Examples
///
/// ```
/// use succinctly::text::utf8::encode_code_point;
///
/// // ASCII
/// let (bytes, len) = encode_code_point(0x41).unwrap();
/// assert_eq!(&bytes[..len], b"A");
///
/// // 2-byte character (é)
/// let (bytes, len) = encode_code_point(0xE9).unwrap();
/// assert_eq!(&bytes[..len], "é".as_bytes());
///
/// // 4-byte character (🎉)
/// let (bytes, len) = encode_code_point(0x1F389).unwrap();
/// assert_eq!(&bytes[..len], "🎉".as_bytes());
///
/// // Invalid: surrogate
/// assert!(encode_code_point(0xD800).is_none());
///
/// // Invalid: out of range
/// assert!(encode_code_point(0x110000).is_none());
/// ```
pub fn encode_code_point(cp: u32) -> Option<([u8; 4], usize)> {
    // Reject surrogates and out-of-range
    if (0xD800..=0xDFFF).contains(&cp) || cp > 0x10FFFF {
        return None;
    }

    let mut buf = [0u8; 4];

    let len = if cp < 0x80 {
        buf[0] = cp as u8;
        1
    } else if cp < 0x800 {
        buf[0] = 0xC0 | ((cp >> 6) as u8);
        buf[1] = 0x80 | ((cp & 0x3F) as u8);
        2
    } else if cp < 0x10000 {
        buf[0] = 0xE0 | ((cp >> 12) as u8);
        buf[1] = 0x80 | (((cp >> 6) & 0x3F) as u8);
        buf[2] = 0x80 | ((cp & 0x3F) as u8);
        3
    } else {
        buf[0] = 0xF0 | ((cp >> 18) as u8);
        buf[1] = 0x80 | (((cp >> 12) & 0x3F) as u8);
        buf[2] = 0x80 | (((cp >> 6) & 0x3F) as u8);
        buf[3] = 0x80 | ((cp & 0x3F) as u8);
        4
    };

    Some((buf, len))
}

/// Format a byte as a human-readable string for error messages.
pub fn format_byte(byte: u8) -> String {
    if byte.is_ascii_graphic() || byte == b' ' {
        alloc::format!("0x{:02X} ({:?})", byte, byte as char)
    } else {
        alloc::format!("0x{:02X}", byte)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Valid UTF-8 Tests
    // =========================================================================

    mod valid_utf8 {
        use super::*;

        #[test]
        fn empty_input() {
            assert!(validate_utf8(b"").is_ok());
        }

        #[test]
        fn ascii_single_byte() {
            // All ASCII characters (0x00-0x7F)
            for byte in 0x00..=0x7F {
                assert!(
                    validate_utf8(&[byte]).is_ok(),
                    "ASCII byte 0x{:02X} should be valid",
                    byte
                );
            }
        }

        #[test]
        fn ascii_string() {
            assert!(validate_utf8(b"Hello, world!").is_ok());
            assert!(validate_utf8(b"The quick brown fox jumps over the lazy dog").is_ok());
        }

        #[test]
        fn ascii_with_control_chars() {
            assert!(validate_utf8(b"line1\nline2\ttab\rcarriage").is_ok());
            assert!(validate_utf8(b"\x00\x01\x02\x1F").is_ok()); // Control characters
        }

        #[test]
        fn two_byte_sequences() {
            // U+0080 (first 2-byte code point)
            assert!(validate_utf8(&[0xC2, 0x80]).is_ok());

            // U+00FF (Latin Small Letter Y with Diaeresis)
            assert!(validate_utf8(&[0xC3, 0xBF]).is_ok());

            // U+07FF (last 2-byte code point)
            assert!(validate_utf8(&[0xDF, 0xBF]).is_ok());

            // Common 2-byte characters
            assert!(validate_utf8("é".as_bytes()).is_ok()); // U+00E9
            assert!(validate_utf8("ñ".as_bytes()).is_ok()); // U+00F1
            assert!(validate_utf8("ü".as_bytes()).is_ok()); // U+00FC
            assert!(validate_utf8("©".as_bytes()).is_ok()); // U+00A9
            assert!(validate_utf8("®".as_bytes()).is_ok()); // U+00AE
        }

        #[test]
        fn three_byte_sequences() {
            // U+0800 (first 3-byte code point)
            assert!(validate_utf8(&[0xE0, 0xA0, 0x80]).is_ok());

            // U+FFFF (last valid 3-byte code point in BMP)
            assert!(validate_utf8(&[0xEF, 0xBF, 0xBF]).is_ok());

            // Japanese characters
            assert!(validate_utf8("日本語".as_bytes()).is_ok());
            assert!(validate_utf8("こんにちは".as_bytes()).is_ok());

            // Chinese characters
            assert!(validate_utf8("中文".as_bytes()).is_ok());
            assert!(validate_utf8("你好世界".as_bytes()).is_ok());

            // Korean characters
            assert!(validate_utf8("한국어".as_bytes()).is_ok());
            assert!(validate_utf8("안녕하세요".as_bytes()).is_ok());

            // Arabic
            assert!(validate_utf8("مرحبا".as_bytes()).is_ok());

            // Hebrew
            assert!(validate_utf8("שלום".as_bytes()).is_ok());

            // Thai
            assert!(validate_utf8("สวัสดี".as_bytes()).is_ok());

            // Currency symbols
            assert!(validate_utf8("€".as_bytes()).is_ok()); // U+20AC Euro
            assert!(validate_utf8("₹".as_bytes()).is_ok()); // U+20B9 Indian Rupee
            assert!(validate_utf8("₿".as_bytes()).is_ok()); // U+20BF Bitcoin
        }

        #[test]
        fn four_byte_sequences() {
            // U+10000 (first 4-byte code point)
            assert!(validate_utf8(&[0xF0, 0x90, 0x80, 0x80]).is_ok());

            // U+10FFFF (last valid code point)
            assert!(validate_utf8(&[0xF4, 0x8F, 0xBF, 0xBF]).is_ok());

            // Emoji
            assert!(validate_utf8("🎉".as_bytes()).is_ok()); // U+1F389 Party Popper
            assert!(validate_utf8("😀".as_bytes()).is_ok()); // U+1F600 Grinning Face
            assert!(validate_utf8("🚀".as_bytes()).is_ok()); // U+1F680 Rocket
            assert!(validate_utf8("🌍".as_bytes()).is_ok()); // U+1F30D Earth
            assert!(validate_utf8("💻".as_bytes()).is_ok()); // U+1F4BB Laptop
            assert!(validate_utf8("🔥".as_bytes()).is_ok()); // U+1F525 Fire

            // Mathematical symbols
            assert!(validate_utf8("𝕳".as_bytes()).is_ok()); // U+1D573 Mathematical Bold Fraktur H
            assert!(validate_utf8("𝔸".as_bytes()).is_ok()); // U+1D538 Mathematical Double-Struck A

            // Ancient scripts
            assert!(validate_utf8("𐀀".as_bytes()).is_ok()); // U+10000 Linear B Syllable B008 A

            // Music symbols
            assert!(validate_utf8("𝄞".as_bytes()).is_ok()); // U+1D11E Musical Symbol G Clef
        }

        #[test]
        fn mixed_sequences() {
            // Mix of all sequence lengths
            let mixed = "A é 日 🎉";
            assert!(validate_utf8(mixed.as_bytes()).is_ok());

            // Complex mixed text
            let complex = "Hello! 你好 مرحبا 🌍🚀 Ñoño café";
            assert!(validate_utf8(complex.as_bytes()).is_ok());
        }

        #[test]
        fn boundary_code_points() {
            // First code point of each length
            assert!(validate_utf8(&[0x00]).is_ok()); // U+0000
            assert!(validate_utf8(&[0xC2, 0x80]).is_ok()); // U+0080
            assert!(validate_utf8(&[0xE0, 0xA0, 0x80]).is_ok()); // U+0800
            assert!(validate_utf8(&[0xF0, 0x90, 0x80, 0x80]).is_ok()); // U+10000

            // Last code point of each length
            assert!(validate_utf8(&[0x7F]).is_ok()); // U+007F
            assert!(validate_utf8(&[0xDF, 0xBF]).is_ok()); // U+07FF
            assert!(validate_utf8(&[0xEF, 0xBF, 0xBF]).is_ok()); // U+FFFF
            assert!(validate_utf8(&[0xF4, 0x8F, 0xBF, 0xBF]).is_ok()); // U+10FFFF
        }

        #[test]
        fn non_characters() {
            // Unicode non-characters are technically valid UTF-8
            // U+FFFE and U+FFFF
            assert!(validate_utf8(&[0xEF, 0xBF, 0xBE]).is_ok()); // U+FFFE
            assert!(validate_utf8(&[0xEF, 0xBF, 0xBF]).is_ok()); // U+FFFF

            // BOM (Byte Order Mark)
            assert!(validate_utf8(&[0xEF, 0xBB, 0xBF]).is_ok()); // U+FEFF
        }

        #[test]
        fn long_valid_string() {
            // 1KB of mixed valid UTF-8
            let mut s = String::new();
            for i in 0..100 {
                s.push_str(&format!("Line {}: Hello 世界 🎉\n", i));
            }
            assert!(validate_utf8(s.as_bytes()).is_ok());
        }
    }

    // =========================================================================
    // Invalid Lead Byte Tests
    // =========================================================================

    mod invalid_lead_byte {
        use super::*;

        #[test]
        fn continuation_byte_as_lead() {
            // Continuation bytes (0x80-0xBF) cannot start a sequence
            for byte in 0x80..=0xBF {
                let result = validate_utf8(&[byte]);
                assert!(
                    result.is_err(),
                    "Byte 0x{:02X} should be invalid as lead",
                    byte
                );
                let err = result.unwrap_err();
                assert_eq!(err.kind, Utf8ErrorKind::InvalidLeadByte);
                assert_eq!(err.offset, 0);
            }
        }

        #[test]
        fn continuation_byte_after_valid() {
            // Valid ASCII followed by bare continuation byte
            let input = [b'A', 0x80];
            let result = validate_utf8(&input);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.kind, Utf8ErrorKind::InvalidLeadByte);
            assert_eq!(err.offset, 1);
        }

        #[test]
        fn f8_ff_lead_bytes() {
            // 0xF8-0xFF are always invalid lead bytes
            for byte in 0xF8..=0xFF {
                let result = validate_utf8(&[byte]);
                assert!(
                    result.is_err(),
                    "Byte 0x{:02X} should be invalid as lead",
                    byte
                );
                let err = result.unwrap_err();
                assert_eq!(err.kind, Utf8ErrorKind::InvalidLeadByte);
            }
        }

        #[test]
        fn fe_ff_bytes() {
            // 0xFE and 0xFF are never valid in UTF-8
            assert!(validate_utf8(&[0xFE]).is_err());
            assert!(validate_utf8(&[0xFF]).is_err());
            assert!(validate_utf8(&[0xFE, 0xFE, 0xFF, 0xFF]).is_err());
        }
    }

    // =========================================================================
    // Invalid Continuation Byte Tests
    // =========================================================================

    mod invalid_continuation {
        use super::*;

        #[test]
        fn missing_continuation_2byte() {
            // 2-byte lead followed by ASCII instead of continuation
            let result = validate_utf8(&[0xC2, b'A']);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.kind, Utf8ErrorKind::InvalidContinuationByte);
            assert_eq!(err.offset, 1);
        }

        #[test]
        fn missing_continuation_3byte_first() {
            // 3-byte lead followed by ASCII
            let result = validate_utf8(&[0xE0, b'A', 0x80]);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.kind, Utf8ErrorKind::InvalidContinuationByte);
            assert_eq!(err.offset, 1);
        }

        #[test]
        fn missing_continuation_3byte_second() {
            // 3-byte sequence with second continuation wrong
            let result = validate_utf8(&[0xE0, 0xA0, b'A']);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.kind, Utf8ErrorKind::InvalidContinuationByte);
            assert_eq!(err.offset, 2);
        }

        #[test]
        fn missing_continuation_4byte() {
            // 4-byte sequence with various wrong continuations
            // Wrong first continuation
            let result = validate_utf8(&[0xF0, b'A', 0x80, 0x80]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().offset, 1);

            // Wrong second continuation
            let result = validate_utf8(&[0xF0, 0x90, b'A', 0x80]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().offset, 2);

            // Wrong third continuation
            let result = validate_utf8(&[0xF0, 0x90, 0x80, b'A']);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().offset, 3);
        }

        #[test]
        fn continuation_is_another_lead() {
            // 2-byte lead followed by another lead byte
            let result = validate_utf8(&[0xC2, 0xC2]);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.kind, Utf8ErrorKind::InvalidContinuationByte);
        }

        #[test]
        fn continuation_is_high_byte() {
            // Continuation position has 0xF0+ byte
            let result = validate_utf8(&[0xC2, 0xF0]);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().kind,
                Utf8ErrorKind::InvalidContinuationByte
            );
        }
    }

    // =========================================================================
    // Overlong Encoding Tests
    // =========================================================================

    mod overlong_encoding {
        use super::*;

        #[test]
        fn overlong_2byte_null() {
            // NUL (U+0000) encoded as 2 bytes: C0 80
            let result = validate_utf8(&[0xC0, 0x80]);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.kind, Utf8ErrorKind::OverlongEncoding);
        }

        #[test]
        fn overlong_2byte_ascii() {
            // ASCII 'A' (U+0041) encoded as 2 bytes: C1 81
            let result = validate_utf8(&[0xC1, 0x81]);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.kind, Utf8ErrorKind::OverlongEncoding);
        }

        #[test]
        fn overlong_2byte_all_c0_c1() {
            // C0 and C1 lead bytes always indicate overlong encoding
            for lead in [0xC0, 0xC1] {
                for cont in 0x80..=0xBF {
                    let result = validate_utf8(&[lead, cont]);
                    assert!(
                        result.is_err(),
                        "0x{:02X} 0x{:02X} should be overlong",
                        lead,
                        cont
                    );
                    assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::OverlongEncoding);
                }
            }
        }

        #[test]
        fn overlong_3byte_null() {
            // NUL (U+0000) encoded as 3 bytes: E0 80 80
            let result = validate_utf8(&[0xE0, 0x80, 0x80]);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.kind, Utf8ErrorKind::OverlongEncoding);
        }

        #[test]
        fn overlong_3byte_2byte_char() {
            // U+007F encoded as 3 bytes: E0 81 BF
            let result = validate_utf8(&[0xE0, 0x81, 0xBF]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::OverlongEncoding);

            // U+07FF encoded as 3 bytes: E0 9F BF (should be DF BF)
            let result = validate_utf8(&[0xE0, 0x9F, 0xBF]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::OverlongEncoding);
        }

        #[test]
        fn overlong_4byte_null() {
            // NUL (U+0000) encoded as 4 bytes: F0 80 80 80
            let result = validate_utf8(&[0xF0, 0x80, 0x80, 0x80]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::OverlongEncoding);
        }

        #[test]
        fn overlong_4byte_3byte_char() {
            // U+FFFF encoded as 4 bytes: F0 8F BF BF (should be EF BF BF)
            let result = validate_utf8(&[0xF0, 0x8F, 0xBF, 0xBF]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::OverlongEncoding);
        }

        #[test]
        fn security_overlong_slash() {
            // Security test: overlong encoding of '/' (U+002F)
            // Attackers might use this to bypass path traversal filters

            // 2-byte: C0 AF
            let result = validate_utf8(&[0xC0, 0xAF]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::OverlongEncoding);

            // 3-byte: E0 80 AF
            let result = validate_utf8(&[0xE0, 0x80, 0xAF]);
            assert!(result.is_err());

            // 4-byte: F0 80 80 AF
            let result = validate_utf8(&[0xF0, 0x80, 0x80, 0xAF]);
            assert!(result.is_err());
        }
    }

    // =========================================================================
    // Surrogate Code Point Tests
    // =========================================================================

    mod surrogate_codepoints {
        use super::*;

        #[test]
        fn high_surrogate_start() {
            // U+D800 (first high surrogate): ED A0 80
            let result = validate_utf8(&[0xED, 0xA0, 0x80]);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.kind, Utf8ErrorKind::SurrogateCodepoint);
        }

        #[test]
        fn high_surrogate_end() {
            // U+DBFF (last high surrogate): ED AF BF
            let result = validate_utf8(&[0xED, 0xAF, 0xBF]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::SurrogateCodepoint);
        }

        #[test]
        fn low_surrogate_start() {
            // U+DC00 (first low surrogate): ED B0 80
            let result = validate_utf8(&[0xED, 0xB0, 0x80]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::SurrogateCodepoint);
        }

        #[test]
        fn low_surrogate_end() {
            // U+DFFF (last low surrogate): ED BF BF
            let result = validate_utf8(&[0xED, 0xBF, 0xBF]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::SurrogateCodepoint);
        }

        #[test]
        fn all_surrogates() {
            // Test a sample of surrogate code points
            let surrogates = [
                0xD800, 0xD801, 0xDB00, 0xDBFF, 0xDC00, 0xDC01, 0xDF00, 0xDFFF,
            ];
            for cp in surrogates {
                // Encode surrogate manually
                let bytes = [
                    0xE0 | ((cp >> 12) as u8),
                    0x80 | (((cp >> 6) & 0x3F) as u8),
                    0x80 | ((cp & 0x3F) as u8),
                ];
                let result = validate_utf8(&bytes);
                assert!(result.is_err(), "U+{:04X} should be invalid surrogate", cp);
                assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::SurrogateCodepoint);
            }
        }

        #[test]
        fn surrogate_in_middle_of_valid() {
            // Valid text followed by surrogate
            let mut input = Vec::from(b"Hello ");
            input.extend_from_slice(&[0xED, 0xA0, 0x80]); // U+D800
            input.extend_from_slice(b" world");

            let result = validate_utf8(&input);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.kind, Utf8ErrorKind::SurrogateCodepoint);
            assert_eq!(err.offset, 6);
        }

        #[test]
        fn non_surrogate_ed_valid() {
            // U+D7FF (just below surrogates): ED 9F BF
            assert!(validate_utf8(&[0xED, 0x9F, 0xBF]).is_ok());

            // U+E000 (just above surrogates): EE 80 80
            assert!(validate_utf8(&[0xEE, 0x80, 0x80]).is_ok());
        }
    }

    // =========================================================================
    // Out of Range Code Point Tests
    // =========================================================================

    mod out_of_range {
        use super::*;

        #[test]
        fn just_above_max() {
            // U+110000 (first invalid): F4 90 80 80
            let result = validate_utf8(&[0xF4, 0x90, 0x80, 0x80]);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.kind, Utf8ErrorKind::OutOfRangeCodepoint);
        }

        #[test]
        fn max_valid() {
            // U+10FFFF (last valid): F4 8F BF BF
            assert!(validate_utf8(&[0xF4, 0x8F, 0xBF, 0xBF]).is_ok());
        }

        #[test]
        fn very_high_codepoints() {
            // Various out-of-range code points
            // U+1FFFFF: F7 BF BF BF
            let result = validate_utf8(&[0xF7, 0xBF, 0xBF, 0xBF]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::OutOfRangeCodepoint);

            // U+13FFFF: F4 BF BF BF
            let result = validate_utf8(&[0xF4, 0xBF, 0xBF, 0xBF]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::OutOfRangeCodepoint);
        }
    }

    // =========================================================================
    // Truncated Sequence Tests
    // =========================================================================

    mod truncated_sequences {
        use super::*;

        #[test]
        fn truncated_2byte() {
            // 2-byte lead with no continuation
            let result = validate_utf8(&[0xC2]);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.kind, Utf8ErrorKind::TruncatedSequence);
            assert_eq!(err.offset, 0);
        }

        #[test]
        fn truncated_3byte_1() {
            // 3-byte lead with no continuations
            let result = validate_utf8(&[0xE0]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::TruncatedSequence);
        }

        #[test]
        fn truncated_3byte_2() {
            // 3-byte lead with only 1 continuation
            let result = validate_utf8(&[0xE0, 0xA0]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::TruncatedSequence);
        }

        #[test]
        fn truncated_4byte_1() {
            // 4-byte lead with no continuations
            let result = validate_utf8(&[0xF0]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::TruncatedSequence);
        }

        #[test]
        fn truncated_4byte_2() {
            // 4-byte lead with only 1 continuation
            let result = validate_utf8(&[0xF0, 0x90]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::TruncatedSequence);
        }

        #[test]
        fn truncated_4byte_3() {
            // 4-byte lead with only 2 continuations
            let result = validate_utf8(&[0xF0, 0x90, 0x80]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::TruncatedSequence);
        }

        #[test]
        fn truncated_after_valid() {
            // Valid text followed by truncated sequence
            let mut input = Vec::from(b"Hello ");
            input.push(0xC2); // Truncated 2-byte sequence

            let result = validate_utf8(&input);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.kind, Utf8ErrorKind::TruncatedSequence);
            assert_eq!(err.offset, 6);
        }
    }

    // =========================================================================
    // Error Position Tests
    // =========================================================================

    mod error_positions {
        use super::*;

        #[test]
        fn line_and_column_first_byte() {
            let result = validate_utf8(&[0x80]);
            let err = result.unwrap_err();
            assert_eq!(err.offset, 0);
            assert_eq!(err.line, 1);
            assert_eq!(err.column, 1);
        }

        #[test]
        fn line_and_column_after_ascii() {
            let input = b"Hello\x80";
            let result = validate_utf8(input);
            let err = result.unwrap_err();
            assert_eq!(err.offset, 5);
            assert_eq!(err.line, 1);
            assert_eq!(err.column, 6);
        }

        #[test]
        fn line_and_column_second_line() {
            let input = b"Hello\nWorld\x80";
            let result = validate_utf8(input);
            let err = result.unwrap_err();
            assert_eq!(err.offset, 11);
            assert_eq!(err.line, 2);
            assert_eq!(err.column, 6);
        }

        #[test]
        fn line_and_column_third_line() {
            let input = b"Line 1\nLine 2\nLine \x80 3";
            let result = validate_utf8(input);
            let err = result.unwrap_err();
            assert_eq!(err.offset, 19);
            assert_eq!(err.line, 3);
            assert_eq!(err.column, 6);
        }

        #[test]
        fn line_and_column_after_multibyte() {
            // "日本" followed by invalid byte
            let mut input = "日本".as_bytes().to_vec();
            input.push(0x80);
            let result = validate_utf8(&input);
            let err = result.unwrap_err();
            assert_eq!(err.offset, 6); // After 6 bytes (2 × 3-byte chars)
            assert_eq!(err.line, 1);
            assert_eq!(err.column, 7);
        }

        #[test]
        fn line_after_crlf() {
            let input = b"Line 1\r\nLine 2\x80";
            let result = validate_utf8(input);
            let err = result.unwrap_err();
            // Line should be 2 (after \n)
            assert_eq!(err.line, 2);
        }

        #[test]
        fn multiple_newlines() {
            let input = b"\n\n\n\n\x80";
            let result = validate_utf8(input);
            let err = result.unwrap_err();
            assert_eq!(err.offset, 4);
            assert_eq!(err.line, 5);
            assert_eq!(err.column, 1);
        }
    }

    // =========================================================================
    // Decode/Encode Tests
    // =========================================================================

    mod decode_encode {
        use super::*;

        #[test]
        fn decode_ascii() {
            assert_eq!(decode_code_point(b"A"), Some((0x41, 1)));
            assert_eq!(decode_code_point(b"\x00"), Some((0x00, 1)));
            assert_eq!(decode_code_point(b"\x7F"), Some((0x7F, 1)));
        }

        #[test]
        fn decode_2byte() {
            assert_eq!(decode_code_point(&[0xC2, 0x80]), Some((0x80, 2)));
            assert_eq!(decode_code_point(&[0xDF, 0xBF]), Some((0x7FF, 2)));
            assert_eq!(decode_code_point("é".as_bytes()), Some((0xE9, 2)));
        }

        #[test]
        fn decode_3byte() {
            assert_eq!(decode_code_point(&[0xE0, 0xA0, 0x80]), Some((0x800, 3)));
            assert_eq!(decode_code_point("日".as_bytes()), Some((0x65E5, 3)));
            assert_eq!(decode_code_point("€".as_bytes()), Some((0x20AC, 3)));
        }

        #[test]
        fn decode_4byte() {
            assert_eq!(
                decode_code_point(&[0xF0, 0x90, 0x80, 0x80]),
                Some((0x10000, 4))
            );
            assert_eq!(decode_code_point("🎉".as_bytes()), Some((0x1F389, 4)));
        }

        #[test]
        fn decode_invalid() {
            assert_eq!(decode_code_point(b""), None);
            assert_eq!(decode_code_point(&[0x80]), None); // Bare continuation
            assert_eq!(decode_code_point(&[0xC2]), None); // Truncated
            assert_eq!(decode_code_point(&[0xC2, 0x00]), None); // Invalid continuation
        }

        #[test]
        fn encode_ascii() {
            let (bytes, len) = encode_code_point(0x41).unwrap();
            assert_eq!(&bytes[..len], b"A");

            let (bytes, len) = encode_code_point(0x00).unwrap();
            assert_eq!(&bytes[..len], b"\x00");
        }

        #[test]
        fn encode_2byte() {
            let (bytes, len) = encode_code_point(0x80).unwrap();
            assert_eq!(&bytes[..len], &[0xC2, 0x80]);

            let (bytes, len) = encode_code_point(0xE9).unwrap(); // é
            assert_eq!(&bytes[..len], "é".as_bytes());
        }

        #[test]
        fn encode_3byte() {
            let (bytes, len) = encode_code_point(0x800).unwrap();
            assert_eq!(&bytes[..len], &[0xE0, 0xA0, 0x80]);

            let (bytes, len) = encode_code_point(0x65E5).unwrap(); // 日
            assert_eq!(&bytes[..len], "日".as_bytes());
        }

        #[test]
        fn encode_4byte() {
            let (bytes, len) = encode_code_point(0x10000).unwrap();
            assert_eq!(&bytes[..len], &[0xF0, 0x90, 0x80, 0x80]);

            let (bytes, len) = encode_code_point(0x1F389).unwrap(); // 🎉
            assert_eq!(&bytes[..len], "🎉".as_bytes());
        }

        #[test]
        fn encode_invalid() {
            // Surrogate
            assert!(encode_code_point(0xD800).is_none());
            assert!(encode_code_point(0xDFFF).is_none());

            // Out of range
            assert!(encode_code_point(0x110000).is_none());
            assert!(encode_code_point(0xFFFFFFFF).is_none());
        }

        #[test]
        fn roundtrip() {
            // Test roundtrip encoding/decoding
            let test_points = [
                0x00, 0x41, 0x7F, // ASCII
                0x80, 0xFF, 0x7FF, // 2-byte
                0x800, 0x65E5, 0xFFFF, // 3-byte
                0x10000, 0x1F389, 0x10FFFF, // 4-byte
            ];

            for cp in test_points {
                let (encoded, len) = encode_code_point(cp).unwrap();
                let (decoded, decoded_len) = decode_code_point(&encoded[..len]).unwrap();
                assert_eq!(cp, decoded);
                assert_eq!(len, decoded_len);
            }
        }
    }

    // =========================================================================
    // Sequence Length Tests
    // =========================================================================

    mod sequence_length_tests {
        use super::*;

        #[test]
        fn ascii_length() {
            for byte in 0x00..=0x7F {
                assert_eq!(sequence_length(byte), 1);
            }
        }

        #[test]
        fn continuation_length() {
            for byte in 0x80..=0xBF {
                assert_eq!(sequence_length(byte), 0); // Invalid as lead
            }
        }

        #[test]
        fn two_byte_length() {
            for byte in 0xC0..=0xDF {
                assert_eq!(sequence_length(byte), 2);
            }
        }

        #[test]
        fn three_byte_length() {
            for byte in 0xE0..=0xEF {
                assert_eq!(sequence_length(byte), 3);
            }
        }

        #[test]
        fn four_byte_length() {
            for byte in 0xF0..=0xF7 {
                assert_eq!(sequence_length(byte), 4);
            }
        }

        #[test]
        fn invalid_lead_length() {
            for byte in 0xF8..=0xFF {
                assert_eq!(sequence_length(byte), 0);
            }
        }
    }

    // =========================================================================
    // Edge Cases and Stress Tests
    // =========================================================================

    mod edge_cases {
        use super::*;

        #[test]
        fn all_same_continuation() {
            // Many continuation bytes in a row
            let input = vec![0x80; 100];
            let result = validate_utf8(&input);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::InvalidLeadByte);
        }

        #[test]
        fn alternating_valid_invalid() {
            // Valid character followed by invalid, repeated
            let mut input = vec![b'A'; 10];
            input.push(0x80); // First invalid

            let result = validate_utf8(&input);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().offset, 10);
        }

        #[test]
        fn many_emoji() {
            // Many 4-byte sequences
            let emoji = "🎉🚀🌍💻🔥";
            let repeated: String = emoji.repeat(100);
            assert!(validate_utf8(repeated.as_bytes()).is_ok());
        }

        #[test]
        fn mixed_newline_styles() {
            let input = "Line 1\nLine 2\r\nLine 3\rLine 4";
            assert!(validate_utf8(input.as_bytes()).is_ok());
        }

        #[test]
        fn null_bytes() {
            // Null bytes are valid ASCII
            assert!(validate_utf8(&[0x00, 0x00, 0x00]).is_ok());
            assert!(validate_utf8(b"Hello\x00World").is_ok());
        }

        #[test]
        fn only_newlines() {
            assert!(validate_utf8(b"\n\n\n\n\n").is_ok());
            assert!(validate_utf8(b"\r\n\r\n\r\n").is_ok());
        }

        #[test]
        fn long_lines() {
            // Very long line without newlines
            let long_line = "A".repeat(10000);
            assert!(validate_utf8(long_line.as_bytes()).is_ok());
        }

        #[test]
        fn invalid_at_various_offsets() {
            // Test invalid byte at different positions
            for offset in [0, 1, 7, 15, 31, 63, 64, 65, 100, 127, 128, 255, 256] {
                let mut input = vec![b'A'; offset + 1];
                input[offset] = 0x80;

                let result = validate_utf8(&input);
                assert!(result.is_err());
                assert_eq!(result.unwrap_err().offset, offset);
            }
        }

        #[test]
        fn boundary_64_bytes() {
            // Test around 64-byte boundary (common SIMD width)
            let mut input = vec![b'A'; 64];
            assert!(validate_utf8(&input).is_ok());

            input.push(0x80);
            let result = validate_utf8(&input);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().offset, 64);
        }

        #[test]
        fn boundary_chunk_crossing() {
            // Multi-byte sequence crossing 64-byte boundary
            let mut input = vec![b'A'; 63];
            // Add a 3-byte character that crosses boundary
            input.extend_from_slice("日".as_bytes());
            assert!(validate_utf8(&input).is_ok());
        }
    }

    // =========================================================================
    // Comparison with std::str
    // =========================================================================

    mod std_comparison {
        use super::*;

        #[test]
        fn agree_on_valid_strings() {
            let test_cases = [
                "",
                "Hello, world!",
                "日本語",
                "🎉🚀🌍",
                "Mixed: café 日本 🎉",
                "\n\t\r",
                "\x00\x01\x02",
            ];

            for s in test_cases {
                let our_result = validate_utf8(s.as_bytes());
                assert!(our_result.is_ok(), "Should agree {} is valid", s);
            }
        }

        #[test]
        fn agree_on_invalid_bytes() {
            let test_cases: &[&[u8]] = &[
                &[0x80],                   // Bare continuation
                &[0xC2],                   // Truncated 2-byte
                &[0xE0, 0x80],             // Truncated 3-byte
                &[0xC0, 0x80],             // Overlong
                &[0xED, 0xA0, 0x80],       // Surrogate
                &[0xF4, 0x90, 0x80, 0x80], // Out of range
            ];

            for bytes in test_cases {
                let our_result = validate_utf8(bytes);
                let std_result = core::str::from_utf8(bytes);

                assert!(
                    our_result.is_err() && std_result.is_err(),
                    "Should agree {:?} is invalid",
                    bytes
                );
            }
        }
    }
}
