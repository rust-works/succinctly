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
#[cfg(not(test))]
use alloc::vec::Vec;

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
        write!(
            f,
            "line {}, column {} (offset {})",
            self.line, self.column, self.offset
        )
    }
}

/// Kinds of validation errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationErrorKind {
    // Structural errors
    /// Expected a specific character but found something else.
    UnexpectedCharacter { expected: &'static str, found: char },
    /// Unexpected end of input.
    UnexpectedEof { expected: &'static str },
    /// Extra content after the root JSON value.
    TrailingContent,

    // String errors
    /// String was not closed before end of input.
    UnclosedString,
    /// Invalid escape sequence in string.
    InvalidEscape { sequence: char },
    /// Invalid unicode escape sequence.
    InvalidUnicodeEscape { reason: &'static str },
    /// Unpaired surrogate in unicode escape.
    UnpairedSurrogate { codepoint: u16 },
    /// Unescaped control character in string.
    ControlCharacter { byte: u8 },

    // Number errors
    /// Number has leading zero (e.g., 01, 007).
    LeadingZero,
    /// Number has leading plus sign.
    LeadingPlus,
    /// Invalid number format.
    InvalidNumber { reason: &'static str },

    // Other errors
    /// Invalid keyword (not null, true, or false).
    InvalidKeyword { found: String },
    /// Invalid UTF-8 sequence.
    InvalidUtf8,
    /// Container nesting depth exceeded the limit.
    NestingTooDeep { limit: usize },
}

impl fmt::Display for ValidationErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedCharacter { expected, found } => {
                write!(f, "expected {expected}, found {found:?}")
            }
            Self::UnexpectedEof { expected } => {
                write!(f, "unexpected end of input, expected {expected}")
            }
            Self::TrailingContent => write!(f, "trailing content after JSON value"),
            Self::UnclosedString => write!(f, "unclosed string"),
            Self::InvalidEscape { sequence } => {
                write!(f, "invalid escape sequence '\\{sequence}'")
            }
            Self::InvalidUnicodeEscape { reason } => {
                write!(f, "invalid unicode escape: {reason}")
            }
            Self::UnpairedSurrogate { codepoint } => {
                write!(f, "unpaired surrogate \\u{codepoint:04X}")
            }
            Self::ControlCharacter { byte } => {
                write!(f, "unescaped control character 0x{byte:02X}")
            }
            Self::LeadingZero => write!(f, "leading zeros not allowed in numbers"),
            Self::LeadingPlus => write!(f, "leading plus sign not allowed in numbers"),
            Self::InvalidNumber { reason } => write!(f, "invalid number: {reason}"),
            Self::InvalidKeyword { found } => {
                write!(
                    f,
                    "invalid keyword '{found}' (expected null, true, or false)"
                )
            }
            Self::InvalidUtf8 => write!(f, "invalid UTF-8 sequence"),
            Self::NestingTooDeep { limit } => {
                write!(f, "nesting depth exceeds limit of {limit}")
            }
        }
    }
}

/// A JSON validation error with position information.
///
/// `PartialEq`/`Eq` make the whole error comparable, so differential tests can
/// assert byte-for-byte agreement on *which* error is reported and *where*,
/// rather than merely that validation failed. Both fields already derived them.
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// Maximum nesting depth for containers (objects and arrays).
///
/// Bounds call-stack growth in the mutually recursive `validate_value` →
/// `validate_object`/`validate_array` cycle so deeply nested input fails with
/// [`ValidationErrorKind::NestingTooDeep`] instead of overflowing the stack.
/// Matches the YAML parser's cap and keeps depth-100 documents (pinned by
/// `tests/deep_nesting_valid_tests.rs`) valid.
const MAX_NESTING_DEPTH: usize = 128;

/// A strict JSON validator with position tracking.
///
/// Uses recursive descent parsing to validate JSON according to RFC 8259.
pub struct Validator<'a> {
    input: &'a [u8],
    offset: usize,
    line: usize,
    column: usize,
    /// Current container nesting depth, capped at [`MAX_NESTING_DEPTH`].
    nesting_depth: usize,
}

impl<'a> Validator<'a> {
    /// Create a new validator for the given input.
    pub fn new(input: &'a [u8]) -> Self {
        Self {
            input,
            offset: 0,
            line: 1,
            column: 1,
            nesting_depth: 0,
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
            Some(b'-' | b'0'..=b'9') => self.validate_number(),
            Some(b't' | b'f' | b'n') => self.validate_keyword(),
            Some(b'+') => Err(self.error(ValidationErrorKind::LeadingPlus)),
            Some(_) => Err(self.error(ValidationErrorKind::UnexpectedCharacter {
                expected: "JSON value",
                found: crate::text::utf8::decode_char_at(self.input, self.offset),
            })),
            None => Err(self.error(ValidationErrorKind::UnexpectedEof {
                expected: "JSON value",
            })),
        }
    }

    /// Enter a nested container, erroring past [`MAX_NESTING_DEPTH`].
    ///
    /// Callers must decrement `nesting_depth` when the container's frame
    /// exits so sibling containers do not accumulate depth.
    #[inline]
    fn enter_nested(&mut self) -> Result<(), ValidationError> {
        if self.nesting_depth >= MAX_NESTING_DEPTH {
            return Err(self.error(ValidationErrorKind::NestingTooDeep {
                limit: MAX_NESTING_DEPTH,
            }));
        }
        self.nesting_depth += 1;
        Ok(())
    }

    /// Validate a JSON object.
    fn validate_object(&mut self) -> Result<(), ValidationError> {
        self.enter_nested()?;
        let result = self.validate_object_inner();
        self.nesting_depth -= 1;
        result
    }

    fn validate_object_inner(&mut self) -> Result<(), ValidationError> {
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
                    found: crate::text::utf8::decode_char_at(self.input, self.offset),
                }));
            }
            self.validate_string()?;
            self.skip_whitespace();

            // Expect colon
            if self.peek() != Some(b':') {
                return Err(self.error(ValidationErrorKind::UnexpectedCharacter {
                    expected: "':'",
                    found: crate::text::utf8::decode_char_at(self.input, self.offset),
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
                Some(_) => {
                    return Err(self.error(ValidationErrorKind::UnexpectedCharacter {
                        expected: "',' or '}'",
                        found: crate::text::utf8::decode_char_at(self.input, self.offset),
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
        self.enter_nested()?;
        let result = self.validate_array_inner();
        self.nesting_depth -= 1;
        result
    }

    fn validate_array_inner(&mut self) -> Result<(), ValidationError> {
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
                Some(_) => {
                    return Err(self.error(ValidationErrorKind::UnexpectedCharacter {
                        expected: "',' or ']'",
                        found: crate::text::utf8::decode_char_at(self.input, self.offset),
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
                        return Err(
                            self.error(ValidationErrorKind::UnpairedSurrogate { codepoint: high })
                        );
                    }
                    self.advance();
                    if self.peek() != Some(b'u') {
                        return Err(
                            self.error(ValidationErrorKind::UnpairedSurrogate { codepoint: high })
                        );
                    }
                    self.advance();

                    let low = self.validate_unicode_escape()?;
                    if !(0xDC00..=0xDFFF).contains(&low) {
                        return Err(
                            self.error(ValidationErrorKind::UnpairedSurrogate { codepoint: high })
                        );
                    }
                } else if (0xDC00..=0xDFFF).contains(&high) {
                    // Lone low surrogate
                    return Err(
                        self.error(ValidationErrorKind::UnpairedSurrogate { codepoint: high })
                    );
                }

                Ok(())
            }
            Some(_) => Err(self.error(ValidationErrorKind::InvalidEscape {
                sequence: crate::text::utf8::decode_char_at(self.input, self.offset),
            })),
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

    /// Consume a run of ASCII digits, returning how many were consumed.
    ///
    /// Replaces `while matches!(self.peek(), Some(b'0'..=b'9')) { self.advance(); }`,
    /// which paid a bounds check and an `Option` construction per digit. Digits
    /// can contain no line terminator, so bumping `column` by the run length is
    /// exactly equivalent to that many `advance()` calls.
    #[inline]
    fn skip_digits(&mut self) -> usize {
        let rest = &self.input[self.offset..];
        let n = rest
            .iter()
            .position(|b| !b.is_ascii_digit())
            .unwrap_or(rest.len());
        self.offset += n;
        self.column += n;
        n
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
                self.skip_digits();
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
            if self.skip_digits() == 0 {
                return Err(self.error(ValidationErrorKind::InvalidNumber {
                    reason: "expected digit after decimal point",
                }));
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
            if self.skip_digits() == 0 {
                return Err(self.error(ValidationErrorKind::InvalidNumber {
                    reason: "expected digit in exponent",
                }));
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

/// True if `bytes` is *exactly* one RFC 8259 JSON number token, with
/// nothing before or after it.
///
/// Grammar: optional `-`, `0`|`[1-9][0-9]*` integer part, optional
/// `.`-fraction requiring ≥1 digit, optional exponent requiring ≥1 digit.
/// This is the same grammar `Validator::validate_number` enforces as one
/// step of a larger document walk (with position tracking and detailed
/// error variants for diagnostics); this free function instead answers a
/// narrower, allocation-free question for a caller that already has a
/// candidate token isolated and just needs a yes/no on the whole slice —
/// e.g. a semi-index number token whose byte span was scanned leniently and
/// may not actually be valid JSON number syntax (see #966), or a YAML
/// scalar already routed to numeric dispatch that needs to know whether
/// it's safe to echo as a JSON number literal verbatim (see #957, #918).
///
/// # Example
///
/// ```
/// use succinctly::json::validate::is_valid_number;
///
/// assert!(is_valid_number(b"-3.14e10"));
/// assert!(!is_valid_number(b"007"));      // leading zero
/// assert!(!is_valid_number(b"1.2.3"));    // two decimal points
/// assert!(!is_valid_number(b"1."));       // no digit after '.'
/// ```
#[must_use]
pub fn is_valid_number(bytes: &[u8]) -> bool {
    let mut i = 0;
    if bytes.first() == Some(&b'-') {
        i += 1;
    }
    match bytes.get(i) {
        Some(b'0') => i += 1,
        Some(b'1'..=b'9') => {
            i += 1;
            while matches!(bytes.get(i), Some(b'0'..=b'9')) {
                i += 1;
            }
        }
        _ => return false,
    }
    if bytes.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while matches!(bytes.get(i), Some(b'0'..=b'9')) {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(bytes.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let start = i;
        while matches!(bytes.get(i), Some(b'0'..=b'9')) {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    i == bytes.len()
}

/// Strip a redundant leading zero (`007` -> `7`, `-007` -> `-7`) from a
/// single, already-isolated number token's integer part.
///
/// Real jq's own number reader tolerates a leading zero that strict
/// RFC 8259 doesn't; this is the shared building block two independent
/// "recover a lenient-but-invalid number's own literal spelling"
/// call sites use (`OwnedValue::from_number_bytes` in `src/jq/value.rs`,
/// and [`crate::json::light`]'s `DocumentValue::number_literal`
/// implementation, #1149) -- strip the redundant zero(s), re-check
/// [`is_valid_number`] on the result, and materialize *that* stripped
/// text as the literal spelling if it passes (real jq's own display
/// drops the redundant zero but otherwise preserves the spelling
/// unchanged, e.g. `007e5` -> `7E+5`, `007.500` -> `7.500`).
///
/// Returns `None` when there's nothing to strip -- either the integer
/// part doesn't start with a redundant `0` at all (an ordinary number,
/// the overwhelmingly common case, kept allocation-free), or it's a bare
/// `0`/`0.x`-style integer part RFC 8259 already accepts unchanged. An
/// all-zero run (`00`, `-000`) strips down to a single `0`, not empty.
///
/// Deliberately scoped to one token, unlike
/// `normalize_leading_zero_numbers` (`src/bin/succinctly/jq_runner.rs`,
/// #1094) which scans a whole raw JSON *string* while tracking
/// string-literal context to find every number token inside it -- both
/// of this function's callers already receive one isolated span, so
/// neither needs that machinery.
#[must_use]
pub fn strip_redundant_leading_zeros(bytes: &[u8]) -> Option<Vec<u8>> {
    let (sign, rest) = match bytes.first() {
        Some(b'-') => (&bytes[..1], &bytes[1..]),
        _ => (&bytes[..0], bytes),
    };
    if rest.first() != Some(&b'0') || !rest.get(1).is_some_and(u8::is_ascii_digit) {
        return None;
    }
    let mut i = 0;
    while rest.get(i) == Some(&b'0') && rest.get(i + 1).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    let mut stripped = Vec::with_capacity(bytes.len() - i);
    stripped.extend_from_slice(sign);
    stripped.extend_from_slice(&rest[i..]);
    Some(stripped)
}

/// Whether `bytes` becomes valid RFC 8259 number syntax after inserting a
/// single `0` immediately after a trailing `.` that sits right before an
/// exponent marker (`1.e999` -> `1.0e999`).
///
/// Real jq's own number reader tolerates this shape, the same kind of
/// leniency [`strip_redundant_leading_zeros`] already models for a
/// redundant leading zero. Shared for the identical reason that function
/// is: two independent "recover a lenient-but-invalid number's own
/// literal spelling" call sites need it
/// ([`OwnedValue::from_number_bytes`](crate::jq::OwnedValue::from_number_bytes)
/// and [`crate::json::light`]'s `DocumentValue::number_literal`
/// implementation, #2220).
///
/// Composes with the leading-zero leniency too: pass a
/// [`strip_redundant_leading_zeros`] result here (when one exists) instead
/// of the original `bytes`, so a token with *both* issues at once
/// (`007.e999`) is still recognized -- inserting the trailing-dot fixup
/// never touches the leading digits either way, so checking on whichever
/// form already applies is sufficient; there is no need to try every
/// combination of "stripped or not."
///
/// Returns `bool`, not the fixed-up candidate `Option<Vec<u8>>` the way
/// [`strip_redundant_leading_zeros`] does: both call sites always
/// materialize the *original* `bytes` on success, never this function's
/// own inserted-`0` candidate, so there is no stripped text either caller
/// ever wants back.
#[must_use]
pub fn has_trailing_dot_before_exponent(bytes: &[u8]) -> bool {
    let Some(dot_pos) = bytes
        .windows(2)
        .position(|w| w[0] == b'.' && (w[1] == b'e' || w[1] == b'E'))
    else {
        return false;
    };
    let mut fixed = Vec::with_capacity(bytes.len() + 1);
    fixed.extend_from_slice(&bytes[..=dot_pos]);
    fixed.push(b'0');
    fixed.extend_from_slice(&bytes[dot_pos + 1..]);
    is_valid_number(&fixed)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // is_valid_number tests (#957/#966)
    // ========================================================================

    #[test]
    fn is_valid_number_accepts_full_rfc_8259_grammar() {
        for s in [
            "0", "-0", "42", "-42", "0.0", "2.0", "-2.5", "0.50", "1e10", "1E10", "1e+10", "1e-10",
            "-1.5e-3", "0e0",
        ] {
            assert!(is_valid_number(s.as_bytes()), "expected valid: {s:?}");
        }
    }

    #[test]
    fn is_valid_number_rejects_lenient_semi_index_spans() {
        // These are exactly the spans #966 found the semi-index scanner
        // over-accepting as a single number token.
        for s in ["007", "1.2.3", "1-2", "1.", ".5", "1e", "-", "1e+", ""] {
            assert!(!is_valid_number(s.as_bytes()), "expected invalid: {s:?}");
        }
    }

    #[test]
    fn is_valid_number_rejects_trailing_garbage() {
        // The whole slice must be one number -- this differs from
        // `Validator::validate_number`, which only needs a number to start
        // at the current position (a sibling `,`/`}` may legitimately
        // follow within a larger document).
        assert!(!is_valid_number(b"42x"));
        assert!(!is_valid_number(b"42 "));
    }

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
        assert!(matches!(
            err.kind,
            ValidationErrorKind::UnexpectedEof { .. }
        ));
    }

    #[test]
    fn test_invalid_whitespace_only() {
        let err = validate(b"   ").unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::UnexpectedEof { .. }
        ));
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
        assert!(matches!(
            err.kind,
            ValidationErrorKind::InvalidNumber { .. }
        ));
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
        assert!(matches!(
            err.kind,
            ValidationErrorKind::InvalidNumber { .. }
        ));

        let err = validate(b"1e+").unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::InvalidNumber { .. }
        ));
    }

    #[test]
    fn test_invalid_escape_sequence() {
        let err = validate(br#""\q""#).unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::InvalidEscape { sequence: 'q' }
        ));
    }

    /// The byte after `\` (or at any other `UnexpectedCharacter`/`InvalidEscape`
    /// offset) can be the lead byte of a multi-byte UTF-8 sequence; the
    /// reported character must be the real decoded one, not a Latin-1 cast of
    /// its lead byte (#1422).
    #[test]
    fn test_unexpected_character_decodes_multibyte_char_1422() {
        let err = validate("\"\\日\"".as_bytes()).unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::InvalidEscape { sequence: '日' }
        ));

        let err = validate("{日: 1}".as_bytes()).unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::UnexpectedCharacter {
                expected: "string key",
                found: '日'
            }
        ));

        let err = validate("{\"a\"日1}".as_bytes()).unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::UnexpectedCharacter {
                expected: "':'",
                found: '日'
            }
        ));

        let err = validate("[1,2日".as_bytes()).unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::UnexpectedCharacter {
                expected: "',' or ']'",
                found: '日'
            }
        ));

        let err = validate("{\"a\":1 日}".as_bytes()).unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::UnexpectedCharacter {
                expected: "',' or '}'",
                found: '日'
            }
        ));

        let err = validate("🎉".as_bytes()).unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::UnexpectedCharacter {
                expected: "JSON value",
                found: '🎉'
            }
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
        assert!(matches!(
            err.kind,
            ValidationErrorKind::UnpairedSurrogate { .. }
        ));
    }

    #[test]
    fn test_invalid_lone_low_surrogate() {
        let err = validate(br#""\uDE00""#).unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::UnpairedSurrogate { .. }
        ));
    }

    #[test]
    fn test_invalid_bad_surrogate_pair() {
        // High surrogate followed by non-surrogate
        let err = validate(br#""\uD83D\u0041""#).unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::UnpairedSurrogate { .. }
        ));
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
        assert!(matches!(
            err.kind,
            ValidationErrorKind::UnexpectedEof { .. }
        ));
    }

    #[test]
    fn test_invalid_unclosed_array() {
        let err = validate(b"[1, 2, 3").unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::UnexpectedEof { .. }
        ));
    }

    /// A keyword immediately followed by more `[a-z]` is one bad keyword, not a
    /// good keyword plus trailing content.
    ///
    /// This is the constraint that forces `validate_keyword`'s fast path to
    /// look one byte past the literal. Without that lookahead `truex` would
    /// match the `true` prefix and be reported as `UnexpectedCharacter` at the
    /// `x`; the greedy `[a-z]*` slow path reports `InvalidKeyword` at the `t`.
    /// The two differ in kind *and* offset, and the CLI renders both.
    #[test]
    fn test_keyword_prefix_of_longer_run_is_one_bad_keyword() {
        for (input, found, offset) in [
            (&b"truex"[..], "truex", 0),
            (&b"nulll"[..], "nulll", 0),
            (&b"falsey"[..], "falsey", 0),
            (&b"[truex]"[..], "truex", 1),
        ] {
            let err = validate(input).unwrap_err();
            assert_eq!(
                err.kind,
                ValidationErrorKind::InvalidKeyword {
                    found: found.to_string()
                },
                "{}",
                String::from_utf8_lossy(input)
            );
            assert_eq!(
                err.position.offset,
                offset,
                "{}",
                String::from_utf8_lossy(input)
            );
        }
    }

    /// The mirror case: a keyword followed by a non-`[a-z]` byte really is a
    /// complete keyword, so the error belongs to what follows it.
    #[test]
    fn test_keyword_followed_by_non_letter_is_complete() {
        let err = validate(b"true1").unwrap_err();
        assert_eq!(err.kind, ValidationErrorKind::TrailingContent);
        assert_eq!(err.position.offset, 4);

        let err = validate(b"[true1]").unwrap_err();
        assert_eq!(
            err.kind,
            ValidationErrorKind::UnexpectedCharacter {
                expected: "',' or ']'",
                found: '1'
            }
        );
        assert_eq!(err.position.offset, 5);

        // And the plain forms still validate.
        for ok in [&b"true"[..], b"false", b"null", b"[true,false,null]"] {
            assert!(validate(ok).is_ok(), "{}", String::from_utf8_lossy(ok));
        }
    }

    #[test]
    fn test_invalid_keyword() {
        let err = validate(b"nul").unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::InvalidKeyword { .. }
        ));

        let err = validate(b"tru").unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::InvalidKeyword { .. }
        ));

        let err = validate(b"fals").unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::InvalidKeyword { .. }
        ));

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

    // ========================================================================
    // RFC 8259 comprehensive coverage tests
    // ========================================================================

    /// RFC 8259 Section 7: All control characters (U+0000 through U+001F) must be escaped.
    #[test]
    fn test_all_control_characters_rejected() {
        for byte in 0x00u8..=0x1F {
            let input = format!("\"hello{}world\"", byte as char);
            let err = validate(input.as_bytes()).unwrap_err();
            assert!(
                matches!(err.kind, ValidationErrorKind::ControlCharacter { byte: b } if b == byte),
                "Control char 0x{byte:02X} should be rejected"
            );
        }
    }

    /// RFC 8259 Section 6: -0 is a valid number.
    #[test]
    fn test_negative_zero() {
        assert!(validate(b"-0").is_ok());
        assert!(validate(b"[-0]").is_ok());
        assert!(validate(br#"{"value": -0}"#).is_ok());
    }

    /// RFC 8259 Section 6: Zero with exponent is valid.
    #[test]
    fn test_zero_with_exponent() {
        assert!(validate(b"0e0").is_ok());
        assert!(validate(b"0E0").is_ok());
        assert!(validate(b"0e+0").is_ok());
        assert!(validate(b"0e-0").is_ok());
        assert!(validate(b"0.0e0").is_ok());
    }

    /// RFC 8259 Section 4: Empty string keys are valid.
    #[test]
    fn test_empty_string_key() {
        assert!(validate(br#"{"": 1}"#).is_ok());
        assert!(validate(br#"{"": ""}"#).is_ok());
        assert!(validate(br#"{"": null, "a": 1}"#).is_ok());
    }

    /// RFC 8259: High surrogate followed by non-\u escape should error.
    #[test]
    fn test_high_surrogate_followed_by_regular_escape() {
        // \uD83D followed by \n (not another \uXXXX)
        let err = validate(br#""\uD83D\n""#).unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::UnpairedSurrogate { .. }
        ));

        // \uD83D followed by \t
        let err = validate(br#""\uD83D\t""#).unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::UnpairedSurrogate { .. }
        ));
    }

    /// RFC 8259: High surrogate at end of string should error.
    #[test]
    fn test_high_surrogate_at_string_end() {
        let err = validate(br#""\uD83D""#).unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::UnpairedSurrogate { .. }
        ));
    }

    /// UTF-8: Standalone continuation bytes (0x80-0xBF) are invalid.
    #[test]
    fn test_invalid_utf8_standalone_continuation() {
        let err = validate(b"\"hello\x80world\"").unwrap_err();
        assert!(matches!(err.kind, ValidationErrorKind::InvalidUtf8));

        let err = validate(b"\"hello\xBFworld\"").unwrap_err();
        assert!(matches!(err.kind, ValidationErrorKind::InvalidUtf8));
    }

    /// UTF-8: Overlong encodings are invalid (C0, C1 lead bytes).
    #[test]
    fn test_invalid_utf8_overlong_2byte() {
        // C0 80 is overlong encoding of NUL
        let err = validate(b"\"hello\xC0\x80world\"").unwrap_err();
        assert!(matches!(err.kind, ValidationErrorKind::InvalidUtf8));

        // C1 BF is overlong encoding of U+007F
        let err = validate(b"\"hello\xC1\xBFworld\"").unwrap_err();
        assert!(matches!(err.kind, ValidationErrorKind::InvalidUtf8));
    }

    /// UTF-8: Invalid lead bytes F5-FF are rejected.
    #[test]
    fn test_invalid_utf8_f5_and_above() {
        for lead in 0xF5u8..=0xFF {
            let input = [b'"', b'x', lead, 0x80, 0x80, 0x80, b'"'];
            let err = validate(&input).unwrap_err();
            assert!(
                matches!(err.kind, ValidationErrorKind::InvalidUtf8),
                "Lead byte 0x{lead:02X} should be rejected"
            );
        }
    }

    /// UTF-8: Truncated multi-byte sequences are invalid.
    #[test]
    fn test_invalid_utf8_truncated() {
        // 2-byte sequence truncated (missing continuation)
        let err = validate(b"\"hello\xC2\"").unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::UnclosedString | ValidationErrorKind::InvalidUtf8
        ));

        // 3-byte sequence truncated (missing 1 continuation)
        let err = validate(b"\"hello\xE0\xA0\"").unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::UnclosedString | ValidationErrorKind::InvalidUtf8
        ));

        // 4-byte sequence truncated (missing 2 continuations)
        let err = validate(b"\"hello\xF0\x90\"").unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::UnclosedString | ValidationErrorKind::InvalidUtf8
        ));
    }

    /// UTF-8: Surrogate codepoints encoded directly in UTF-8 (ED A0 80 - ED BF BF) are invalid.
    #[test]
    fn test_invalid_utf8_surrogate_codepoints() {
        // U+D800 encoded as UTF-8: ED A0 80
        let err = validate(b"\"hello\xED\xA0\x80world\"").unwrap_err();
        assert!(matches!(err.kind, ValidationErrorKind::InvalidUtf8));

        // U+DFFF encoded as UTF-8: ED BF BF
        let err = validate(b"\"hello\xED\xBF\xBFworld\"").unwrap_err();
        assert!(matches!(err.kind, ValidationErrorKind::InvalidUtf8));
    }

    /// UTF-8: Codepoints above U+10FFFF are invalid.
    #[test]
    fn test_invalid_utf8_above_max_codepoint() {
        // F4 90 80 80 would encode U+110000 (above max)
        let err = validate(b"\"hello\xF4\x90\x80\x80world\"").unwrap_err();
        assert!(matches!(err.kind, ValidationErrorKind::InvalidUtf8));
    }

    /// RFC 8259 Section 7: All valid escape sequences.
    #[test]
    fn test_all_valid_escapes() {
        assert!(validate(br#""\"""#).is_ok()); // quotation mark
        assert!(validate(br#""\\""#).is_ok()); // reverse solidus
        assert!(validate(br#""\/""#).is_ok()); // solidus
        assert!(validate(br#""\b""#).is_ok()); // backspace
        assert!(validate(br#""\f""#).is_ok()); // form feed
        assert!(validate(br#""\n""#).is_ok()); // line feed
        assert!(validate(br#""\r""#).is_ok()); // carriage return
        assert!(validate(br#""\t""#).is_ok()); // tab
        assert!(validate(br#""\u0000""#).is_ok()); // unicode escape
    }

    /// RFC 8259: Invalid escape sequences.
    #[test]
    fn test_invalid_escapes() {
        // Common mistakes
        let err = validate(br#""\a""#).unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::InvalidEscape { sequence: 'a' }
        ));

        let err = validate(br#""\v""#).unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::InvalidEscape { sequence: 'v' }
        ));

        let err = validate(br#""\x00""#).unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::InvalidEscape { sequence: 'x' }
        ));

        let err = validate(br#""\0""#).unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::InvalidEscape { sequence: '0' }
        ));
    }

    /// RFC 8259 Section 6: Number edge cases.
    #[test]
    fn test_number_edge_cases() {
        // Valid edge cases
        assert!(validate(b"0").is_ok());
        assert!(validate(b"-0").is_ok());
        assert!(validate(b"0.0").is_ok());
        assert!(validate(b"-0.0").is_ok());
        assert!(validate(b"1e1").is_ok());
        assert!(validate(b"1E1").is_ok());
        assert!(validate(b"1e+1").is_ok());
        assert!(validate(b"1e-1").is_ok());
        assert!(validate(b"0.1e1").is_ok());
        assert!(validate(b"123456789012345678901234567890").is_ok()); // large integer

        // Invalid: multiple leading zeros
        let err = validate(b"00").unwrap_err();
        assert!(matches!(err.kind, ValidationErrorKind::LeadingZero));

        // Invalid: leading zero before digit
        let err = validate(b"01").unwrap_err();
        assert!(matches!(err.kind, ValidationErrorKind::LeadingZero));

        // Invalid: negative with leading zero
        let err = validate(b"-01").unwrap_err();
        assert!(matches!(err.kind, ValidationErrorKind::LeadingZero));
    }

    /// RFC 8259: Structural edge cases.
    #[test]
    fn test_structural_edge_cases() {
        // Deeply nested (but valid)
        let deep_array = "[".repeat(100) + &"]".repeat(100);
        assert!(validate(deep_array.as_bytes()).is_ok());

        let deep_object = r#"{"a":"#.repeat(50) + "1" + &"}".repeat(50);
        assert!(validate(deep_object.as_bytes()).is_ok());

        // Empty containers
        assert!(validate(b"{}").is_ok());
        assert!(validate(b"[]").is_ok());
        assert!(validate(b"[[]]").is_ok());
        assert!(validate(b"{{}}").is_err()); // invalid - key required
        assert!(validate(br#"{"a":{}}"#).is_ok());
    }

    /// Issue #151: deeply nested input must return an error instead of
    /// overflowing the stack. Passing at all is the proof — an overflow
    /// would abort the test process.
    #[test]
    fn test_deep_array_errors_instead_of_aborting() {
        let input = "[".repeat(20_000);
        let err = validate(input.as_bytes()).unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::NestingTooDeep { .. }
        ));
    }

    #[test]
    fn test_deep_object_errors_instead_of_aborting() {
        let input = r#"{"a":"#.repeat(20_000);
        let err = validate(input.as_bytes()).unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::NestingTooDeep { .. }
        ));
    }

    #[test]
    fn test_deep_alternating_errors_instead_of_aborting() {
        let input = r#"[{"a":"#.repeat(10_000);
        let err = validate(input.as_bytes()).unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::NestingTooDeep { .. }
        ));
    }

    #[test]
    fn test_nesting_at_cap_validates() {
        let input = "[".repeat(MAX_NESTING_DEPTH) + &"]".repeat(MAX_NESTING_DEPTH);
        assert!(validate(input.as_bytes()).is_ok());
    }

    #[test]
    fn test_nesting_one_past_cap_errors() {
        let input = "[".repeat(MAX_NESTING_DEPTH + 1) + &"]".repeat(MAX_NESTING_DEPTH + 1);
        let err = validate(input.as_bytes()).unwrap_err();
        assert_eq!(err.kind, ValidationErrorKind::NestingTooDeep { limit: 128 });
        // The guard fires at the 129th '[', before it is consumed.
        assert_eq!(err.position.offset, 128);
        assert_eq!(err.position.column, 129);
    }

    /// Sibling containers at the same depth must not accumulate: the
    /// counter is decremented when each container's frame exits, so three
    /// siblings of depth 65 (195 containers total) stay under the cap.
    #[test]
    fn test_wide_but_shallow_nesting_validates() {
        let element = "[".repeat(64) + &"]".repeat(64);
        let input = format!("[{element},{element},{element}]");
        assert!(validate(input.as_bytes()).is_ok());
    }

    /// RFC 8259 Section 2: Whitespace handling.
    #[test]
    fn test_whitespace_edge_cases() {
        // All valid whitespace characters
        assert!(validate(b" null").is_ok());
        assert!(validate(b"\tnull").is_ok());
        assert!(validate(b"\nnull").is_ok());
        assert!(validate(b"\rnull").is_ok());
        assert!(validate(b" \t\n\r null \t\n\r ").is_ok());

        // Whitespace in structures
        assert!(validate(b"{ }").is_ok());
        assert!(validate(b"[ ]").is_ok());
        assert!(validate(br#"{ "a" : 1 }"#).is_ok());
        assert!(validate(b"[ 1 , 2 , 3 ]").is_ok());
    }

    /// RFC 8259: Unicode escape edge cases.
    #[test]
    fn test_unicode_escape_edge_cases() {
        // Lowercase hex
        assert!(validate(br#""\u00ff""#).is_ok());
        // Uppercase hex
        assert!(validate(br#""\u00FF""#).is_ok());
        // Mixed case
        assert!(validate(br#""\u00Ff""#).is_ok());

        // Valid surrogate pair (U+1F600 GRINNING FACE)
        assert!(validate(br#""\uD83D\uDE00""#).is_ok());

        // Multiple surrogate pairs
        assert!(validate(br#""\uD83D\uDE00\uD83D\uDE01""#).is_ok());
    }

    /// RFC 8259: String with all printable ASCII.
    #[test]
    fn test_printable_ascii_in_string() {
        // All printable ASCII (0x20-0x7E) except quote and backslash
        let mut s = String::from("\"");
        for c in 0x20u8..=0x7E {
            if c != b'"' && c != b'\\' {
                s.push(c as char);
            }
        }
        s.push('"');
        assert!(validate(s.as_bytes()).is_ok());
    }

    #[test]
    fn test_validation_error_kind_display() {
        assert_eq!(
            ValidationErrorKind::UnexpectedCharacter {
                expected: "','",
                found: '}'
            }
            .to_string(),
            "expected ',', found '}'"
        );
        assert_eq!(
            ValidationErrorKind::UnexpectedEof { expected: "value" }.to_string(),
            "unexpected end of input, expected value"
        );
        assert_eq!(
            ValidationErrorKind::TrailingContent.to_string(),
            "trailing content after JSON value"
        );
        assert_eq!(
            ValidationErrorKind::UnclosedString.to_string(),
            "unclosed string"
        );
        assert_eq!(
            ValidationErrorKind::InvalidEscape { sequence: 'x' }.to_string(),
            "invalid escape sequence '\\x'"
        );
        assert_eq!(
            ValidationErrorKind::InvalidUnicodeEscape { reason: "bad hex" }.to_string(),
            "invalid unicode escape: bad hex"
        );
        assert_eq!(
            ValidationErrorKind::UnpairedSurrogate { codepoint: 0xD800 }.to_string(),
            "unpaired surrogate \\uD800"
        );
        assert_eq!(
            ValidationErrorKind::ControlCharacter { byte: 0x01 }.to_string(),
            "unescaped control character 0x01"
        );
        assert_eq!(
            ValidationErrorKind::LeadingZero.to_string(),
            "leading zeros not allowed in numbers"
        );
        assert_eq!(
            ValidationErrorKind::LeadingPlus.to_string(),
            "leading plus sign not allowed in numbers"
        );
        assert_eq!(
            ValidationErrorKind::InvalidNumber {
                reason: "no digits"
            }
            .to_string(),
            "invalid number: no digits"
        );
        assert_eq!(
            ValidationErrorKind::InvalidKeyword {
                found: "nul".to_string()
            }
            .to_string(),
            "invalid keyword 'nul' (expected null, true, or false)"
        );
        assert_eq!(
            ValidationErrorKind::InvalidUtf8.to_string(),
            "invalid UTF-8 sequence"
        );
        assert_eq!(
            ValidationErrorKind::NestingTooDeep { limit: 128 }.to_string(),
            "nesting depth exceeds limit of 128"
        );
    }

    #[test]
    fn test_missing_colon_in_object() {
        // Triggers the object-key colon error path.
        let err = validate(br#"{"a" 1}"#).unwrap_err();
        assert!(matches!(
            err.kind,
            ValidationErrorKind::UnexpectedCharacter { .. }
        ));
    }

    // ========================================================================
    // strip_redundant_leading_zeros tests (#1149)
    // ========================================================================

    #[test]
    fn test_strip_redundant_leading_zeros_basic() {
        assert_eq!(strip_redundant_leading_zeros(b"007"), Some(b"7".to_vec()));
        assert_eq!(strip_redundant_leading_zeros(b"-007"), Some(b"-7".to_vec()));
        assert_eq!(
            strip_redundant_leading_zeros(b"007e5"),
            Some(b"7e5".to_vec())
        );
        assert_eq!(
            strip_redundant_leading_zeros(b"007.500"),
            Some(b"7.500".to_vec())
        );
    }

    #[test]
    fn test_strip_redundant_leading_zeros_all_zero_run_collapses_to_one_zero() {
        // An all-zero digit run strips down to a single `0`, not empty.
        assert_eq!(strip_redundant_leading_zeros(b"00"), Some(b"0".to_vec()));
        assert_eq!(strip_redundant_leading_zeros(b"000"), Some(b"0".to_vec()));
        assert_eq!(strip_redundant_leading_zeros(b"-000"), Some(b"-0".to_vec()));
    }

    #[test]
    fn test_strip_redundant_leading_zeros_nothing_to_strip_returns_none() {
        // Bare `0`/`0.x` is already RFC-8259-valid; no allocation, no strip.
        assert_eq!(strip_redundant_leading_zeros(b"0"), None);
        assert_eq!(strip_redundant_leading_zeros(b"-0"), None);
        assert_eq!(strip_redundant_leading_zeros(b"0.5"), None);
        // Ordinary numbers with no leading zero at all.
        assert_eq!(strip_redundant_leading_zeros(b"42"), None);
        assert_eq!(strip_redundant_leading_zeros(b"-42"), None);
        assert_eq!(strip_redundant_leading_zeros(b"7e5"), None);
    }

    #[test]
    fn test_strip_redundant_leading_zeros_non_number_input_returns_none() {
        // No leading digit at all -- not this function's job to validate,
        // just to decline stripping when there's nothing zero-prefixed.
        assert_eq!(strip_redundant_leading_zeros(b""), None);
        assert_eq!(strip_redundant_leading_zeros(b"-"), None);
        assert_eq!(strip_redundant_leading_zeros(b"abc"), None);
        assert_eq!(strip_redundant_leading_zeros(b".5"), None);
    }

    #[test]
    fn test_strip_redundant_leading_zeros_result_may_still_be_invalid() {
        // The stripped result can itself be invalid -- this function only
        // strips, it doesn't validate; callers re-check `is_valid_number`
        // on the result before trusting it (#966's `1.2.3`-style leniency).
        assert_eq!(
            strip_redundant_leading_zeros(b"007.5.3"),
            Some(b"7.5.3".to_vec())
        );
        assert!(!is_valid_number(b"7.5.3"));
    }
}
