//! Owned JSON values for jq evaluation.
//!
//! When jq expressions construct new values (arrays, objects) or perform
//! computations, we need to materialize them into owned values rather than
//! references into the original JSON bytes.

#[cfg(not(test))]
use alloc::format;
#[cfg(not(test))]
use alloc::string::{String, ToString};
#[cfg(not(test))]
use alloc::vec::Vec;

use indexmap::IndexMap;

use super::expr::Literal;

/// An owned JSON value.
///
/// This is used for values that are constructed during evaluation
/// (array/object construction, arithmetic results, etc.) rather than
/// references into the original JSON document.
#[derive(Debug, Clone, PartialEq)]
pub enum OwnedValue {
    /// JSON null
    Null,
    /// JSON boolean
    Bool(bool),
    /// JSON integer (stored as i64 for precision)
    Int(i64),
    /// JSON floating-point number
    Float(f64),
    /// JSON string
    String(String),
    /// JSON array
    Array(Vec<OwnedValue>),
    /// JSON object (IndexMap preserves insertion order like jq)
    Object(IndexMap<String, OwnedValue>),
}

impl OwnedValue {
    /// Create a null value.
    pub fn null() -> Self {
        OwnedValue::Null
    }

    /// Create a boolean value.
    pub fn bool(b: bool) -> Self {
        OwnedValue::Bool(b)
    }

    /// Create an integer value.
    pub fn int(n: i64) -> Self {
        OwnedValue::Int(n)
    }

    /// Create a float value.
    pub fn float(f: f64) -> Self {
        OwnedValue::Float(f)
    }

    /// Create a string value.
    pub fn string(s: impl Into<String>) -> Self {
        OwnedValue::String(s.into())
    }

    /// Create an empty array.
    pub fn array() -> Self {
        OwnedValue::Array(Vec::new())
    }

    /// Create an array from a vector of values.
    pub fn array_from(values: Vec<OwnedValue>) -> Self {
        OwnedValue::Array(values)
    }

    /// Create an empty object.
    pub fn object() -> Self {
        OwnedValue::Object(IndexMap::new())
    }

    /// Create an object from key-value pairs.
    pub fn object_from(pairs: impl IntoIterator<Item = (String, OwnedValue)>) -> Self {
        OwnedValue::Object(pairs.into_iter().collect())
    }

    /// Check if this value is null.
    pub fn is_null(&self) -> bool {
        matches!(self, OwnedValue::Null)
    }

    /// Check if this value is "truthy" (not null and not false).
    pub fn is_truthy(&self) -> bool {
        !matches!(self, OwnedValue::Null | OwnedValue::Bool(false))
    }

    /// Get the type name of this value.
    pub fn type_name(&self) -> &'static str {
        match self {
            OwnedValue::Null => "null",
            OwnedValue::Bool(_) => "boolean",
            OwnedValue::Int(_) | OwnedValue::Float(_) => "number",
            OwnedValue::String(_) => "string",
            OwnedValue::Array(_) => "array",
            OwnedValue::Object(_) => "object",
        }
    }

    /// Convert to a boolean, if possible.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            OwnedValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Convert to an i64, if possible.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            OwnedValue::Int(n) => Some(*n),
            OwnedValue::Float(f) if (*f - (*f as i64 as f64)).abs() < f64::EPSILON => {
                Some(*f as i64)
            }
            _ => None,
        }
    }

    /// Convert to an f64, if possible.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            OwnedValue::Int(n) => Some(*n as f64),
            OwnedValue::Float(f) => Some(*f),
            _ => None,
        }
    }

    /// Convert to a string reference, if possible.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            OwnedValue::String(s) => Some(s),
            _ => None,
        }
    }

    /// Convert to an array reference, if possible.
    pub fn as_array(&self) -> Option<&Vec<OwnedValue>> {
        match self {
            OwnedValue::Array(arr) => Some(arr),
            _ => None,
        }
    }

    /// Convert to a mutable array reference, if possible.
    pub fn as_array_mut(&mut self) -> Option<&mut Vec<OwnedValue>> {
        match self {
            OwnedValue::Array(arr) => Some(arr),
            _ => None,
        }
    }

    /// Convert to an object reference, if possible.
    pub fn as_object(&self) -> Option<&IndexMap<String, OwnedValue>> {
        match self {
            OwnedValue::Object(obj) => Some(obj),
            _ => None,
        }
    }

    /// Convert to a mutable object reference, if possible.
    pub fn as_object_mut(&mut self) -> Option<&mut IndexMap<String, OwnedValue>> {
        match self {
            OwnedValue::Object(obj) => Some(obj),
            _ => None,
        }
    }

    /// Get the length of this value.
    /// - null: 0
    /// - string: UTF-8 codepoint count
    /// - array: element count
    /// - object: key count
    /// - other: error (returns None)
    pub fn length(&self) -> Option<usize> {
        match self {
            OwnedValue::Null => Some(0),
            OwnedValue::String(s) => Some(s.chars().count()),
            OwnedValue::Array(arr) => Some(arr.len()),
            OwnedValue::Object(obj) => Some(obj.len()),
            _ => None,
        }
    }

    /// Format this value as JSON string.
    pub fn to_json(&self) -> String {
        match self {
            OwnedValue::Null => "null".into(),
            OwnedValue::Bool(true) => "true".into(),
            OwnedValue::Bool(false) => "false".into(),
            OwnedValue::Int(n) => format!("{}", n),
            OwnedValue::Float(f) => {
                if f.is_nan() || f.is_infinite() {
                    "null".into() // JSON doesn't support NaN or Infinity
                } else {
                    format!("{}", f)
                }
            }
            OwnedValue::String(s) => format!("\"{}\"", escape_json_string(s)),
            OwnedValue::Array(arr) => {
                let elements: Vec<String> = arr.iter().map(|v| v.to_json()).collect();
                format!("[{}]", elements.join(","))
            }
            OwnedValue::Object(obj) => {
                let entries: Vec<String> = obj
                    .iter()
                    .map(|(k, v)| format!("\"{}\":{}", escape_json_string(k), v.to_json()))
                    .collect();
                format!("{{{}}}", entries.join(","))
            }
        }
    }
}

/// Escape a string for JSON output.
///
/// Uses SIMD-accelerated scanning (AVX2/SSE2 on x86_64, NEON on ARM64) to
/// quickly skip over runs of non-escapable characters.
fn escape_json_string(s: &str) -> String {
    use crate::yaml::simd::find_json_escape;

    let bytes = s.as_bytes();
    let len = bytes.len();

    // Fast path: if string is short or no escapes needed, avoid allocation overhead
    if len == 0 {
        return String::new();
    }

    // Use SIMD to check if any escape is needed
    let first_escape = find_json_escape(bytes, 0);
    if first_escape == len {
        // No escapes needed - return copy of original string
        return s.to_string();
    }

    // Need escaping - pre-allocate with some extra space for escape sequences
    let mut result = String::with_capacity(len + len / 8);
    let mut pos = 0;

    // Copy prefix before first escape
    if first_escape > 0 {
        // SAFETY: find_json_escape only finds positions at ASCII characters (", \, or < 0x20),
        // so bytes[0..first_escape] contains only valid UTF-8 continuation bytes or complete
        // multi-byte sequences
        result.push_str(unsafe { core::str::from_utf8_unchecked(&bytes[0..first_escape]) });
        pos = first_escape;
    }

    // Process remaining characters
    while pos < len {
        let b = bytes[pos];

        // Handle the escapable character
        match b {
            b'"' => {
                result.push_str("\\\"");
                pos += 1;
            }
            b'\\' => {
                result.push_str("\\\\");
                pos += 1;
            }
            b'\n' => {
                result.push_str("\\n");
                pos += 1;
            }
            b'\r' => {
                result.push_str("\\r");
                pos += 1;
            }
            b'\t' => {
                result.push_str("\\t");
                pos += 1;
            }
            b if b < 0x20 => {
                // Other control characters use \uXXXX
                result.push_str(&format!("\\u{:04x}", b));
                pos += 1;
            }
            _ => {
                // Not an escapable byte - use SIMD to find the next one
                let next_escape = find_json_escape(bytes, pos);

                // Copy the run of non-escapable bytes
                // SAFETY: find_json_escape only stops at ASCII bytes, so the slice
                // from pos to next_escape is valid UTF-8
                result
                    .push_str(unsafe { core::str::from_utf8_unchecked(&bytes[pos..next_escape]) });
                pos = next_escape;
            }
        }
    }

    result
}

impl From<Literal> for OwnedValue {
    fn from(lit: Literal) -> Self {
        match lit {
            Literal::Null => OwnedValue::Null,
            Literal::Bool(b) => OwnedValue::Bool(b),
            Literal::Int(n) => OwnedValue::Int(n),
            Literal::Float(f) => OwnedValue::Float(f),
            Literal::String(s) => OwnedValue::String(s),
        }
    }
}

impl From<bool> for OwnedValue {
    fn from(b: bool) -> Self {
        OwnedValue::Bool(b)
    }
}

impl From<i64> for OwnedValue {
    fn from(n: i64) -> Self {
        OwnedValue::Int(n)
    }
}

impl From<f64> for OwnedValue {
    fn from(f: f64) -> Self {
        OwnedValue::Float(f)
    }
}

impl From<String> for OwnedValue {
    fn from(s: String) -> Self {
        OwnedValue::String(s)
    }
}

impl From<&str> for OwnedValue {
    fn from(s: &str) -> Self {
        OwnedValue::String(s.to_string())
    }
}

impl<T: Into<OwnedValue>> From<Vec<T>> for OwnedValue {
    fn from(arr: Vec<T>) -> Self {
        OwnedValue::Array(arr.into_iter().map(Into::into).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constructors() {
        assert_eq!(OwnedValue::null(), OwnedValue::Null);
        assert_eq!(OwnedValue::bool(true), OwnedValue::Bool(true));
        assert_eq!(OwnedValue::int(42), OwnedValue::Int(42));
        assert_eq!(OwnedValue::float(2.5), OwnedValue::Float(2.5));
        assert_eq!(
            OwnedValue::string("hello"),
            OwnedValue::String("hello".into())
        );
    }

    #[test]
    fn test_truthy() {
        assert!(!OwnedValue::Null.is_truthy());
        assert!(!OwnedValue::Bool(false).is_truthy());
        assert!(OwnedValue::Bool(true).is_truthy());
        assert!(OwnedValue::Int(0).is_truthy()); // 0 is truthy in jq!
        assert!(OwnedValue::String("".into()).is_truthy()); // "" is truthy in jq!
        assert!(OwnedValue::Array(vec![]).is_truthy()); // [] is truthy in jq!
    }

    #[test]
    fn test_type_name() {
        assert_eq!(OwnedValue::Null.type_name(), "null");
        assert_eq!(OwnedValue::Bool(true).type_name(), "boolean");
        assert_eq!(OwnedValue::Int(42).type_name(), "number");
        assert_eq!(OwnedValue::Float(2.5).type_name(), "number");
        assert_eq!(OwnedValue::String("".into()).type_name(), "string");
        assert_eq!(OwnedValue::Array(vec![]).type_name(), "array");
        assert_eq!(OwnedValue::Object(IndexMap::new()).type_name(), "object");
    }

    #[test]
    fn test_length() {
        assert_eq!(OwnedValue::Null.length(), Some(0));
        assert_eq!(OwnedValue::String("hello".into()).length(), Some(5));
        assert_eq!(OwnedValue::String("héllo".into()).length(), Some(5)); // Unicode
        assert_eq!(
            OwnedValue::Array(vec![OwnedValue::Int(1), OwnedValue::Int(2)]).length(),
            Some(2)
        );
        assert_eq!(OwnedValue::Bool(true).length(), None);
        assert_eq!(OwnedValue::Int(42).length(), None);
    }

    #[test]
    fn test_to_json() {
        assert_eq!(OwnedValue::Null.to_json(), "null");
        assert_eq!(OwnedValue::Bool(true).to_json(), "true");
        assert_eq!(OwnedValue::Bool(false).to_json(), "false");
        assert_eq!(OwnedValue::Int(42).to_json(), "42");
        assert_eq!(OwnedValue::Float(2.5).to_json(), "2.5");
        assert_eq!(OwnedValue::String("hello".into()).to_json(), "\"hello\"");
        assert_eq!(
            OwnedValue::String("hello\nworld".into()).to_json(),
            "\"hello\\nworld\""
        );
        assert_eq!(
            OwnedValue::Array(vec![OwnedValue::Int(1), OwnedValue::Int(2)]).to_json(),
            "[1,2]"
        );
    }

    #[test]
    fn test_from_literal() {
        assert_eq!(OwnedValue::from(Literal::Null), OwnedValue::Null);
        assert_eq!(
            OwnedValue::from(Literal::Bool(true)),
            OwnedValue::Bool(true)
        );
        assert_eq!(OwnedValue::from(Literal::Int(42)), OwnedValue::Int(42));
        assert_eq!(
            OwnedValue::from(Literal::Float(2.5)),
            OwnedValue::Float(2.5)
        );
        assert_eq!(
            OwnedValue::from(Literal::String("hello".into())),
            OwnedValue::String("hello".into())
        );
    }

    // ========================================================================
    // Tests for SIMD-accelerated escape_json_string (Issue #91)
    // ========================================================================

    #[test]
    fn test_escape_json_string_empty() {
        assert_eq!(escape_json_string(""), "");
    }

    #[test]
    fn test_escape_json_string_no_escapes() {
        // Fast path: no escapes needed
        assert_eq!(escape_json_string("hello world"), "hello world");
        assert_eq!(escape_json_string("simple text"), "simple text");
    }

    #[test]
    fn test_escape_json_string_quotes() {
        assert_eq!(escape_json_string("hello\"world"), "hello\\\"world");
        assert_eq!(escape_json_string("\"quoted\""), "\\\"quoted\\\"");
    }

    #[test]
    fn test_escape_json_string_backslash() {
        assert_eq!(escape_json_string("hello\\world"), "hello\\\\world");
        assert_eq!(escape_json_string("path\\to\\file"), "path\\\\to\\\\file");
    }

    #[test]
    fn test_escape_json_string_control_chars() {
        assert_eq!(escape_json_string("hello\nworld"), "hello\\nworld");
        assert_eq!(escape_json_string("hello\rworld"), "hello\\rworld");
        assert_eq!(escape_json_string("hello\tworld"), "hello\\tworld");
        // Other control chars use \uXXXX
        assert_eq!(escape_json_string("hello\x01world"), "hello\\u0001world");
        assert_eq!(escape_json_string("hello\x00world"), "hello\\u0000world");
    }

    #[test]
    fn test_escape_json_string_unicode() {
        // Unicode should pass through unchanged
        assert_eq!(escape_json_string("héllo wörld"), "héllo wörld");
        assert_eq!(escape_json_string("こんにちは"), "こんにちは");
        assert_eq!(escape_json_string("emoji: 🦀"), "emoji: 🦀");
    }

    #[test]
    fn test_escape_json_string_mixed() {
        assert_eq!(
            escape_json_string("hello \"world\"\nline2"),
            "hello \\\"world\\\"\\nline2"
        );
        assert_eq!(
            escape_json_string("path: C:\\Users\\name"),
            "path: C:\\\\Users\\\\name"
        );
    }

    #[test]
    fn test_escape_json_string_long() {
        // Test with > 32 bytes to exercise SIMD path
        let long_no_escape = "a".repeat(100);
        assert_eq!(escape_json_string(&long_no_escape), long_no_escape);

        // Long string with escape in the middle
        let mut long_with_escape = "a".repeat(50);
        long_with_escape.push('"');
        long_with_escape.push_str(&"b".repeat(50));
        let expected_correct = "a".repeat(50) + "\\\"" + &"b".repeat(50);
        assert_eq!(escape_json_string(&long_with_escape), expected_correct);
    }

    #[test]
    fn test_escape_json_string_at_boundaries() {
        // Escape at 16-byte boundary (SSE2 chunk size)
        let mut input = "a".repeat(16);
        input.push('"');
        let expected = format!("{}\\\"", "a".repeat(16));
        assert_eq!(escape_json_string(&input), expected);

        // Escape at 32-byte boundary (AVX2 chunk size)
        let mut input32 = "a".repeat(32);
        input32.push('"');
        let expected32 = format!("{}\\\"", "a".repeat(32));
        assert_eq!(escape_json_string(&input32), expected32);
    }

    #[test]
    fn test_escape_json_string_simd_correctness() {
        // Reference scalar implementation for comparison
        fn escape_json_string_scalar(s: &str) -> String {
            let mut result = String::with_capacity(s.len());
            for c in s.chars() {
                match c {
                    '"' => result.push_str("\\\""),
                    '\\' => result.push_str("\\\\"),
                    '\n' => result.push_str("\\n"),
                    '\r' => result.push_str("\\r"),
                    '\t' => result.push_str("\\t"),
                    c if c.is_control() => {
                        result.push_str(&format!("\\u{:04x}", c as u32));
                    }
                    c => result.push(c),
                }
            }
            result
        }

        // Test various inputs
        let test_cases = [
            "",
            "simple",
            "hello world",
            "with\"quote",
            "with\\backslash",
            "newline\nhere",
            "tab\there",
            "carriage\rreturn",
            "control\x01char",
            "multiple \"quotes\" and \\backslashes\\ here",
            "unicode: héllo 世界 🦀",
            &"a".repeat(100),
            &format!("{}\"{}\"", "a".repeat(50), "b".repeat(50)),
        ];

        for input in test_cases {
            let simd_result = escape_json_string(input);
            let scalar_result = escape_json_string_scalar(input);
            assert_eq!(
                simd_result, scalar_result,
                "Mismatch for input: {:?}",
                input
            );
        }
    }
}
