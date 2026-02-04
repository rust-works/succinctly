//! Strict JSON validator according to RFC 8259.
//!
//! This module provides a fast, scalar JSON validator that performs strict
//! validation according to RFC 8259. Unlike the semi-indexing parser, this
//! validator checks all aspects of JSON validity including:
//!
//! - Structural syntax (braces, brackets, colons, commas)
//! - String escape sequences (including surrogate pairs)
//! - Number format (no leading zeros, no leading plus, etc.)
//! - UTF-8 validity
//! - No trailing content after root value
//!
//! # Example
//!
//! ```
//! use succinctly::json::validate::{Validator, ValidationError};
//!
//! let input = br#"{"name": "Alice", "age": 30}"#;
//! let mut validator = Validator::new(input);
//! assert!(validator.validate().is_ok());
//!
//! let invalid = br#"{"name": "Alice",}"#; // trailing comma
//! let mut validator = Validator::new(invalid);
//! assert!(validator.validate().is_err());
//! ```

#[cfg(not(test))]
use alloc::string::String;

use core::fmt;

/// Position information for error reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    /// Byte offset (0-indexed).
    pub offset: usize,
    /// Line number (1-indexed).
    pub line: usize,
    /// Column number (1-indexed, in bytes not characters).
    pub column: usize,
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}, column {} (offset {})", self.line, self.column, self.offset)
    }
}

/// Kinds of validation errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationErrorKind {
    // Structural errors
    /// Expected a specific character but found something else.
    UnexpectedCharacter {
        expected: &'static str,
        found: char,
    },
    /// Unexpected end of input.
    UnexpectedEof {
        expected: &'static str,
    },
    /// Extra content after the root JSON value.
    TrailingContent,

    // String errors
    /// String was not closed before end of input.
    UnclosedString,
    /// Invalid escape sequence in string.
    InvalidEscape {
        sequence: char,
    },
    /// Invalid unicode escape sequence.
    InvalidUnicodeEscape {
        reason: &'static str,
    },
    /// Unpaired surrogate in unicode escape.
    UnpairedSurrogate {
        codepoint: u16,
    },
    /// Unescaped control character in string.
    ControlCharacter {
        byte: u8,
    },

    // Number errors
    /// Number has leading zero (e.g., 01, 007).
    LeadingZero,
    /// Number has leading plus sign.
    LeadingPlus,
    /// Invalid number format.
    InvalidNumber {
        reason: &'static str,
    },

    // Other errors
    /// Invalid keyword (not null, true, or false).
    InvalidKeyword {
        found: String,
    },
    /// Invalid UTF-8 sequence.
    InvalidUtf8,
}

impl fmt::Display for ValidationErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedCharacter { expected, found } => {
                write!(f, "expected {}, found {:?}", expected, found)
            }
            Self::UnexpectedEof { expected } => {
                write!(f, "unexpected end of input, expected {}", expected)
            }
            Self::TrailingContent => write!(f, "trailing content after JSON value"),
            Self::UnclosedString => write!(f, "unclosed string"),
            Self::InvalidEscape { sequence } => {
                write!(f, "invalid escape sequence '\\{}'", sequence)
            }
            Self::InvalidUnicodeEscape { reason } => {
                write!(f, "invalid unicode escape: {}", reason)
            }
            Self::UnpairedSurrogate { codepoint } => {
                write!(f, "unpaired surrogate \\u{:04X}", codepoint)
            }
            Self::ControlCharacter { byte } => {
                write!(f, "unescaped control character 0x{:02X}", byte)
            }
            Self::LeadingZero => write!(f, "leading zeros not allowed in numbers"),
            Self::LeadingPlus => write!(f, "leading plus sign not allowed in numbers"),
            Self::InvalidNumber { reason } => write!(f, "invalid number: {}", reason),
            Self::InvalidKeyword { found } => {
                write!(f, "invalid keyword '{}' (expected null, true, or false)", found)
            }
            Self::InvalidUtf8 => write!(f, "invalid UTF-8 sequence"),
        }
    }
}

/// A JSON validation error with position information.
#[derive(Debug, Clone)]
pub struct ValidationError {
    /// The kind of error.
    pub kind: ValidationErrorKind,
    /// Position where the error occurred.
    pub position: Position,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at {}", self.kind, self.position)
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ValidationError {}

/// A strict JSON validator with position tracking.
///
/// Uses recursive descent parsing to validate JSON according to RFC 8259.
pub struct Validator<'a> {
    input: &'a [u8],
    offset: usize,
    line: usize,
    column: usize,
}

impl<'a> Validator<'a> {
    /// Create a new validator for the given input.
    pub fn new(input: &'a [u8]) -> Self {
        Self {
            input,
            offset: 0,
            line: 1,
            column: 1,
        }
    }

    /// Validate the entire input as JSON.
    ///
    /// Returns `Ok(())` if the input is valid JSON, or an error with position
    /// information if validation fails.
    pub fn validate(&mut self) -> Result<(), ValidationError> {
        self.skip_whitespace();

        if self.is_eof() {
            return Err(self.error(ValidationErrorKind::UnexpectedEof {
                expected: "JSON value",
            }));
        }

        self.validate_value()?;
        self.skip_whitespace();

        if !self.is_eof() {
            return Err(self.error(ValidationErrorKind::TrailingContent));
        }

        Ok(())
    }

    /// Validate a JSON value (object, array, string, number, or keyword).
    fn validate_value(&mut self) -> Result<(), ValidationError> {
        match self.peek() {
            Some(b'{') => self.validate_object(),
            Some(b'[') => self.validate_array(),
            Some(b'"') => self.validate_string(),
            Some(b'-') | Some(b'0'..=b'9') => self.validate_number(),
            Some(b't') | Some(b'f') | Some(b'n') => self.validate_keyword(),
            Some(b'+') => Err(self.error(ValidationErrorKind::LeadingPlus)),
            Some(c) => Err(self.error(ValidationErrorKind::UnexpectedCharacter {
                expected: "JSON value",
                found: c as char,
            })),
            None => Err(self.error(ValidationErrorKind::UnexpectedEof {
                expected: "JSON value",
            })),
        }
    }

    /// Validate a JSON object.
    fn validate_object(&mut self) -> Result<(), ValidationError> {
        self.advance(); // consume '{'
        self.skip_whitespace();

        // Empty object
        if self.peek() == Some(b'}') {
            self.advance();
            return Ok(());
        }

        loop {
            // Expect string key
            if self.peek() != Some(b'"') {
                return Err(self.error(ValidationErrorKind::UnexpectedCharacter {
                    expected: "string key",
                    found: self.peek().map(|b| b as char).unwrap_or('\0'),
                }));
            }
            self.validate_string()?;
            self.skip_whitespace();

            // Expect colon
            if self.peek() != Some(b':') {
                return Err(self.error(ValidationErrorKind::UnexpectedCharacter {
                    expected: "':'",
                    found: self.peek().map(|b| b as char).unwrap_or('\0'),
                }));
            }
            self.advance();
            self.skip_whitespace();

            // Expect value
            self.validate_value()?;
            self.skip_whitespace();

            // Expect comma or closing brace
            match self.peek() {
                Some(b',') => {
                    self.advance();
                    self.skip_whitespace();
                    // Check for trailing comma
                    if self.peek() == Some(b'}') {
                        return Err(self.error(ValidationErrorKind::UnexpectedCharacter {
                            expected: "string key",
                            found: '}',
                        }));
                    }
                }
                Some(b'}') => {
                    self.advance();
                    return Ok(());
                }
                Some(c) => {
                    return Err(self.error(ValidationErrorKind::UnexpectedCharacter {
                        expected: "',' or '}'",
                        found: c as char,
                    }));
                }
                None => {
                    return Err(self.error(ValidationErrorKind::UnexpectedEof {
                        expected: "',' or '}'",
                    }));
                }
            }
        }
    }

    /// Validate a JSON array.
    fn validate_array(&mut self) -> Result<(), ValidationError> {
        self.advance(); // consume '['
        self.skip_whitespace();

        // Empty array
        if self.peek() == Some(b']') {
            self.advance();
            return Ok(());
        }

        loop {
            self.validate_value()?;
            self.skip_whitespace();

            match self.peek() {
                Some(b',') => {
                    self.advance();
                    self.skip_whitespace();
                    // Check for trailing comma
                    if self.peek() == Some(b']') {
                        return Err(self.error(ValidationErrorKind::UnexpectedCharacter {
                            expected: "JSON value",
                            found: ']',
                        }));
                    }
                }
                Some(b']') => {
                    self.advance();
                    return Ok(());
                }
                Some(c) => {
                    return Err(self.error(ValidationErrorKind::UnexpectedCharacter {
                        expected: "',' or ']'",
                        found: c as char,
                    }));
                }
                None => {
                    return Err(self.error(ValidationErrorKind::UnexpectedEof {
                        expected: "',' or ']'",
                    }));
                }
            }
        }
    }

    /// Validate a JSON string.
    fn validate_string(&mut self) -> Result<(), ValidationError> {
        self.advance(); // consume opening quote

        loop {
            match self.peek() {
                Some(b'"') => {
                    self.advance();
                    return Ok(());
                }
                Some(b'\\') => {
                    self.validate_escape()?;
                }
                Some(b) if b < 0x20 => {
                    return Err(self.error(ValidationErrorKind::ControlCharacter { byte: b }));
                }
                Some(_) => {
                    // Validate UTF-8 sequence
                    self.validate_utf8_char()?;
                }
                None => {
                    return Err(self.error(ValidationErrorKind::UnclosedString));
                }
            }
        }
    }

    /// Validate a single UTF-8 character (may be multi-byte).
    fn validate_utf8_char(&mut self) -> Result<(), ValidationError> {
        let b = self.peek().unwrap();

        // Single byte ASCII (0x00-0x7F)
        if b < 0x80 {
            self.advance();
            return Ok(());
        }

        // Determine expected length and validate leading byte
        let (len, min_cp, max_cp) = if b & 0xE0 == 0xC0 {
            (2, 0x80u32, 0x7FFu32)
        } else if b & 0xF0 == 0xE0 {
            (3, 0x800u32, 0xFFFFu32)
        } else if b & 0xF8 == 0xF0 {
            (4, 0x10000u32, 0x10FFFFu32)
        } else {
            return Err(self.error(ValidationErrorKind::InvalidUtf8));
        };

        // Check we have enough bytes
        if self.offset + len > self.input.len() {
            return Err(self.error(ValidationErrorKind::InvalidUtf8));
        }

        // Validate continuation bytes
        for i in 1..len {
            let cont = self.input[self.offset + i];
            if cont & 0xC0 != 0x80 {
                return Err(self.error(ValidationErrorKind::InvalidUtf8));
            }
        }

        // Decode and validate codepoint
        let cp = match len {
            2 => ((b as u32 & 0x1F) << 6) | (self.input[self.offset + 1] as u32 & 0x3F),
            3 => {
                ((b as u32 & 0x0F) << 12)
                    | ((self.input[self.offset + 1] as u32 & 0x3F) << 6)
                    | (self.input[self.offset + 2] as u32 & 0x3F)
            }
            4 => {
                ((b as u32 & 0x07) << 18)
                    | ((self.input[self.offset + 1] as u32 & 0x3F) << 12)
                    | ((self.input[self.offset + 2] as u32 & 0x3F) << 6)
                    | (self.input[self.offset + 3] as u32 & 0x3F)
            }
            _ => unreachable!(),
        };

        // Check for overlong encoding
        if cp < min_cp || cp > max_cp {
            return Err(self.error(ValidationErrorKind::InvalidUtf8));
        }

        // Check for surrogate codepoints (invalid in UTF-8)
        if (0xD800..=0xDFFF).contains(&cp) {
            return Err(self.error(ValidationErrorKind::InvalidUtf8));
        }

        // Advance past all bytes
        for _ in 0..len {
            self.advance();
        }

        Ok(())
    }

    /// Validate an escape sequence.
    fn validate_escape(&mut self) -> Result<(), ValidationError> {
        self.advance(); // consume backslash

        match self.peek() {
            Some(b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') => {
                self.advance();
                Ok(())
            }
            Some(b'u') => {
                self.advance();
                let high = self.validate_unicode_escape()?;

                // Check for surrogate pair
                if (0xD800..=0xDBFF).contains(&high) {
                    // High surrogate - must be followed by \uXXXX low surrogate
                    if self.peek() != Some(b'\\') {
                        return Err(self.error(ValidationErrorKind::UnpairedSurrogate {
                            codepoint: high,
                        }));
                    }
                    self.advance();
                    if self.peek() != Some(b'u') {
                        return Err(self.error(ValidationErrorKind::UnpairedSurrogate {
                            codepoint: high,
                        }));
                    }
                    self.advance();

                    let low = self.validate_unicode_escape()?;
                    if !(0xDC00..=0xDFFF).contains(&low) {
                        return Err(self.error(ValidationErrorKind::UnpairedSurrogate {
                            codepoint: high,
                        }));
                    }
                } else if (0xDC00..=0xDFFF).contains(&high) {
                    // Lone low surrogate
                    return Err(self.error(ValidationErrorKind::UnpairedSurrogate {
                        codepoint: high,
                    }));
                }

                Ok(())
            }
            Some(c) => Err(self.error(ValidationErrorKind::InvalidEscape { sequence: c as char })),
            None => Err(self.error(ValidationErrorKind::UnclosedString)),
        }
    }

    /// Validate a \uXXXX unicode escape and return the codepoint.
    fn validate_unicode_escape(&mut self) -> Result<u16, ValidationError> {
        let mut value: u16 = 0;

        for _ in 0..4 {
            match self.peek() {
                Some(b @ b'0'..=b'9') => {
                    value = value * 16 + (b - b'0') as u16;
                    self.advance();
                }
                Some(b @ b'a'..=b'f') => {
                    value = value * 16 + (b - b'a' + 10) as u16;
                    self.advance();
                }
                Some(b @ b'A'..=b'F') => {
                    value = value * 16 + (b - b'A' + 10) as u16;
                    self.advance();
                }
                Some(_) => {
                    return Err(self.error(ValidationErrorKind::InvalidUnicodeEscape {
                        reason: "expected 4 hex digits",
                    }));
                }
                None => {
                    return Err(self.error(ValidationErrorKind::InvalidUnicodeEscape {
                        reason: "unexpected end of input",
                    }));
                }
            }
        }

        Ok(value)
    }

    /// Validate a JSON number.
    fn validate_number(&mut self) -> Result<(), ValidationError> {
        // Optional minus sign
        if self.peek() == Some(b'-') {
            self.advance();
        }

        // Integer part
        match self.peek() {
            Some(b'0') => {
                self.advance();
                // Check for leading zero (e.g., 01, 007)
                if matches!(self.peek(), Some(b'0'..=b'9')) {
                    return Err(self.error(ValidationErrorKind::LeadingZero));
                }
            }
            Some(b'1'..=b'9') => {
                self.advance();
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.advance();
                }
            }
            Some(_) | None => {
                return Err(self.error(ValidationErrorKind::InvalidNumber {
                    reason: "expected digit after minus sign",
                }));
            }
        }

        // Optional fractional part
        if self.peek() == Some(b'.') {
            self.advance();

            // Must have at least one digit after decimal point
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.error(ValidationErrorKind::InvalidNumber {
                    reason: "expected digit after decimal point",
                }));
            }

            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.advance();
            }
        }

        // Optional exponent
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.advance();

            // Optional sign
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.advance();
            }

            // Must have at least one digit
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.error(ValidationErrorKind::InvalidNumber {
                    reason: "expected digit in exponent",
                }));
            }

            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.advance();
            }
        }

        Ok(())
    }

    /// Validate a keyword (null, true, false).
    fn validate_keyword(&mut self) -> Result<(), ValidationError> {
        let start = self.offset;

        // Collect alphabetic characters
        while matches!(self.peek(), Some(b'a'..=b'z')) {
            self.advance();
        }

        let keyword = &self.input[start..self.offset];

        match keyword {
            b"null" | b"true" | b"false" => Ok(()),
            _ => {
                let found = String::from_utf8_lossy(keyword).into_owned();
                // Reset position to start for error reporting
                let err_pos = Position {
                    offset: start,
                    line: self.line,
                    column: self.column - (self.offset - start),
                };
                Err(ValidationError {
                    kind: ValidationErrorKind::InvalidKeyword { found },
                    position: err_pos,
                })
            }
        }
    }

    /// Skip whitespace characters (space, tab, newline, carriage return).
    fn skip_whitespace(&mut self) {
        while let Some(b) = self.peek() {
            match b {
                b' ' | b'\t' => {
                    self.offset += 1;
                    self.column += 1;
                }
                b'\n' => {
                    self.offset += 1;
                    self.line += 1;
                    self.column = 1;
                }
                b'\r' => {
                    self.offset += 1;
                    // Handle CRLF
                    if self.peek() == Some(b'\n') {
                        self.offset += 1;
                    }
                    self.line += 1;
                    self.column = 1;
                }
                _ => break,
            }
        }
    }

    /// Peek at the current byte without advancing.
    #[inline]
    fn peek(&self) -> Option<u8> {
        self.input.get(self.offset).copied()
    }

    /// Advance to the next byte.
    #[inline]
    fn advance(&mut self) -> Option<u8> {
        if self.offset >= self.input.len() {
            return None;
        }
        let b = self.input[self.offset];
        self.offset += 1;
        self.column += 1;
        Some(b)
    }

    /// Check if we're at end of input.
    #[inline]
    fn is_eof(&self) -> bool {
        self.offset >= self.input.len()
    }

    /// Get current position.
    fn position(&self) -> Position {
        Position {
            offset: self.offset,
            line: self.line,
            column: self.column,
        }
    }

    /// Create an error at current position.
    fn error(&self, kind: ValidationErrorKind) -> ValidationError {
        ValidationError {
            kind,
            position: self.position(),
        }
    }
}

/// Validate JSON input.
///
/// Convenience function that creates a validator and runs it.
///
/// # Example
///
/// ```
/// use succinctly::json::validate::validate;
///
/// assert!(validate(br#"{"key": "value"}"#).is_ok());
/// assert!(validate(br#"{"key": }"#).is_err());
/// ```
pub fn validate(input: &[u8]) -> Result<(), ValidationError> {
    Validator::new(input).validate()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // Valid JSON tests
    // ========================================================================

    #[test]
    fn test_valid_null() {
        assert!(validate(b"null").is_ok());
    }

    #[test]
    fn test_valid_true() {
        assert!(validate(b"true").is_ok());
    }

    #[test]
    fn test_valid_false() {
        assert!(validate(b"false").is_ok());
    }

    #[test]
    fn test_valid_empty_object() {
        assert!(validate(b"{}").is_ok());
    }

    #[test]
    fn test_valid_empty_array() {
        assert!(validate(b"[]").is_ok());
    }

    #[test]
    fn test_valid_simple_object() {
        assert!(validate(br#"{"key": "value"}"#).is_ok());
    }

    #[test]
    fn test_valid_simple_array() {
        assert!(validate(b"[1, 2, 3]").is_ok());
    }

    #[test]
    fn test_valid_nested() {
        assert!(validate(br#"{"arr": [1, {"nested": true}]}"#).is_ok());
    }

    #[test]
    fn test_valid_string_escapes() {
        assert!(validate(br#""hello\nworld""#).is_ok());
        assert!(validate(br#""tab\there""#).is_ok());
        assert!(validate(br#""quote\"here""#).is_ok());
        assert!(validate(br#""backslash\\here""#).is_ok());
        assert!(validate(br#""slash\/here""#).is_ok());
        assert!(validate(br#""controls\b\f\r""#).is_ok());
    }

    #[test]
    fn test_valid_unicode_escape() {
        assert!(validate(br#""\u0041""#).is_ok()); // 'A'
        assert!(validate(br#""\u00e9""#).is_ok()); // 'é'
        assert!(validate(br#""\u4e2d""#).is_ok()); // '中'
    }

    #[test]
    fn test_valid_surrogate_pair() {
        // U+1F600 (😀) encoded as surrogate pair
        assert!(validate(br#""\uD83D\uDE00""#).is_ok());
    }

    #[test]
    fn test_valid_numbers() {
        assert!(validate(b"0").is_ok());
        assert!(validate(b"123").is_ok());
        assert!(validate(b"-456").is_ok());
        assert!(validate(b"3.14159").is_ok());
        assert!(validate(b"-0.5").is_ok());
        assert!(validate(b"1e10").is_ok());
        assert!(validate(b"1E10").is_ok());
        assert!(validate(b"1e+10").is_ok());
        assert!(validate(b"1e-10").is_ok());
        assert!(validate(b"2.5e3").is_ok());
        assert!(validate(b"-1.23e-45").is_ok());
    }

    #[test]
    fn test_valid_whitespace() {
        assert!(validate(b"  null  ").is_ok());
        assert!(validate(b"\t\n\r null \t\n\r ").is_ok());
        assert!(validate(b"{ \"key\" : \"value\" }").is_ok());
        assert!(validate(b"[ 1 , 2 , 3 ]").is_ok());
    }

    #[test]
    fn test_valid_utf8() {
        assert!(validate("\"日本語\"".as_bytes()).is_ok());
        assert!(validate("\"émoji: 😀\"".as_bytes()).is_ok());
    }

    // ========================================================================
    // Invalid JSON tests
    // ========================================================================

    #[test]
    fn test_invalid_empty() {
        let err = validate(b"").unwrap_err();
        assert!(matches!(err.kind, ValidationErrorKind::UnexpectedEof { .. }));
    }

    #[test]
    fn test_invalid_whitespace_only() {
        let err = validate(b"   ").unwrap_err();
        assert!(matches!(err.kind, ValidationErrorKind::UnexpectedEof { .. }));
    }

    #[test]
    fn test_invalid_trailing_content() {
        let err = validate(b"null extra").unwrap_err();
        assert!(matches!(err.kind, ValidationErrorKind::TrailingContent));
    }

    #[test]
    fn test_invalid_trailing_comma_object() {
        let err = validate(br#"{"key": "value",}"#).unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::UnexpectedCharacter { found: '}', .. }
        ));
    }

    #[test]
    fn test_invalid_trailing_comma_array() {
        let err = validate(b"[1, 2, 3,]").unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::UnexpectedCharacter { found: ']', .. }
        ));
    }

    #[test]
    fn test_invalid_leading_zero() {
        let err = validate(b"01").unwrap_err();
        assert!(matches!(err.kind, ValidationErrorKind::LeadingZero));

        let err = validate(b"007").unwrap_err();
        assert!(matches!(err.kind, ValidationErrorKind::LeadingZero));
    }

    #[test]
    fn test_invalid_leading_plus() {
        let err = validate(b"+1").unwrap_err();
        assert!(matches!(err.kind, ValidationErrorKind::LeadingPlus));
    }

    #[test]
    fn test_invalid_number_trailing_dot() {
        let err = validate(b"1.").unwrap_err();
        assert!(matches!(err.kind, ValidationErrorKind::InvalidNumber { .. }));
    }

    #[test]
    fn test_invalid_number_leading_dot() {
        let err = validate(b".5").unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::UnexpectedCharacter { .. }
        ));
    }

    #[test]
    fn test_invalid_number_empty_exponent() {
        let err = validate(b"1e").unwrap_err();
        assert!(matches!(err.kind, ValidationErrorKind::InvalidNumber { .. }));

        let err = validate(b"1e+").unwrap_err();
        assert!(matches!(err.kind, ValidationErrorKind::InvalidNumber { .. }));
    }

    #[test]
    fn test_invalid_escape_sequence() {
        let err = validate(br#""\q""#).unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::InvalidEscape { sequence: 'q' }
        ));
    }

    #[test]
    fn test_invalid_unicode_escape_short() {
        let err = validate(br#""\u00""#).unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::UnclosedString | ValidationErrorKind::InvalidUnicodeEscape { .. }
        ));
    }

    #[test]
    fn test_invalid_unicode_escape_bad_hex() {
        let err = validate(br#""\u00GG""#).unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::InvalidUnicodeEscape { .. }
        ));
    }

    #[test]
    fn test_invalid_lone_high_surrogate() {
        let err = validate(br#""\uD83D""#).unwrap_err();
        assert!(matches!(err.kind, ValidationErrorKind::UnpairedSurrogate { .. }));
    }

    #[test]
    fn test_invalid_lone_low_surrogate() {
        let err = validate(br#""\uDE00""#).unwrap_err();
        assert!(matches!(err.kind, ValidationErrorKind::UnpairedSurrogate { .. }));
    }

    #[test]
    fn test_invalid_bad_surrogate_pair() {
        // High surrogate followed by non-surrogate
        let err = validate(br#""\uD83D\u0041""#).unwrap_err();
        assert!(matches!(err.kind, ValidationErrorKind::UnpairedSurrogate { .. }));
    }

    #[test]
    fn test_invalid_control_character() {
        let err = validate(b"\"hello\x00world\"").unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::ControlCharacter { byte: 0x00 }
        ));

        let err = validate(b"\"hello\x1Fworld\"").unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::ControlCharacter { byte: 0x1F }
        ));
    }

    #[test]
    fn test_invalid_unclosed_string() {
        let err = validate(br#""unclosed"#).unwrap_err();
        assert!(matches!(err.kind, ValidationErrorKind::UnclosedString));
    }

    #[test]
    fn test_invalid_unclosed_object() {
        let err = validate(br#"{"key": "value""#).unwrap_err();
        assert!(matches!(err.kind, ValidationErrorKind::UnexpectedEof { .. }));
    }

    #[test]
    fn test_invalid_unclosed_array() {
        let err = validate(b"[1, 2, 3").unwrap_err();
        assert!(matches!(err.kind, ValidationErrorKind::UnexpectedEof { .. }));
    }

    #[test]
    fn test_invalid_keyword() {
        let err = validate(b"nul").unwrap_err();
        assert!(matches!(err.kind, ValidationErrorKind::InvalidKeyword { .. }));

        let err = validate(b"tru").unwrap_err();
        assert!(matches!(err.kind, ValidationErrorKind::InvalidKeyword { .. }));

        let err = validate(b"fals").unwrap_err();
        assert!(matches!(err.kind, ValidationErrorKind::InvalidKeyword { .. }));

        // "undefined" starts with 'u' which is not a valid JSON value start
        let err = validate(b"undefined").unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::UnexpectedCharacter { .. }
        ));
    }

    #[test]
    fn test_invalid_utf8() {
        // Invalid UTF-8 sequence
        let err = validate(b"\"hello\xFF\xFEworld\"").unwrap_err();
        assert!(matches!(err.kind, ValidationErrorKind::InvalidUtf8));
    }

    // ========================================================================
    // Position accuracy tests
    // ========================================================================

    #[test]
    fn test_error_position_single_line() {
        let err = validate(br#"{"key": "value",}"#).unwrap_err();
        assert_eq!(err.position.line, 1);
        assert_eq!(err.position.column, 17); // position of '}'
    }

    #[test]
    fn test_error_position_multiline() {
        let input = b"{\n  \"key\": \"value\",\n}";
        let err = validate(input).unwrap_err();
        assert_eq!(err.position.line, 3);
        assert_eq!(err.position.column, 1); // position of '}' on line 3
    }

    #[test]
    fn test_error_position_crlf() {
        let input = b"{\r\n  \"key\": \"value\",\r\n}";
        let err = validate(input).unwrap_err();
        assert_eq!(err.position.line, 3);
    }
}
