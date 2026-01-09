//! Lazy JSON values for jq evaluation.
//!
//! `JqValue` is the core value type for jq evaluation. It can represent either:
//! - A lazy reference to a value in the original JSON bytes (via `JsonCursor`)
//! - A materialized value that was computed during evaluation
//!
//! This design enables:
//! - Zero-copy navigation for pass-through queries (`.foo`, `.[]`, etc.)
//! - Preserved number formatting (e.g., `4e4` stays as `4e4` in output)
//! - Minimal memory usage - only materialize when computation requires it
//! - Mixed lazy/materialized arrays and objects

#[cfg(not(test))]
use alloc::borrow::Cow;
#[cfg(not(test))]
use alloc::string::{String, ToString};
#[cfg(not(test))]
use alloc::vec::Vec;

use indexmap::IndexMap;
#[cfg(test)]
use std::borrow::Cow;

use crate::json::light::{JsonCursor, StandardJson};

use super::expr::Literal;
use super::value::OwnedValue;

/// A JSON value for jq evaluation - lazy by default, materialized when needed.
///
/// For pass-through operations (field access, iteration, slicing), values stay
/// as `Cursor` references to the original JSON bytes. This preserves the exact
/// text representation (including number formatting like `4e4`) and avoids
/// allocation.
///
/// When computation is required (arithmetic, string operations, object/array
/// construction with computed values), values are materialized into the
/// appropriate owned variant.
#[derive(Debug, Clone)]
pub enum JqValue<'a, W = Vec<u64>> {
    /// Lazy reference to a value in the original JSON bytes.
    ///
    /// The cursor provides navigation methods and access to the raw bytes.
    /// Use `text_range()` to get the byte range for direct output.
    Cursor(JsonCursor<'a, W>),

    /// JSON null (materialized).
    Null,

    /// JSON boolean (materialized).
    Bool(bool),

    /// JSON integer (materialized, stored as i64 for precision).
    Int(i64),

    /// JSON floating-point number (materialized).
    Float(f64),

    /// Raw number bytes from original JSON (preserves formatting like `4e4`).
    ///
    /// This variant is used when a number is extracted from the original JSON
    /// but hasn't been parsed yet. When output, the original bytes are written
    /// directly, preserving formatting like `4e4` instead of `40000`.
    RawNumber(&'a [u8]),

    /// JSON string (materialized).
    String(String),

    /// JSON array with potentially mixed lazy/materialized children.
    ///
    /// Created when constructing arrays with `[.a, .b + 1]` or collecting
    /// iteration results. Children can be `Cursor` (lazy) or materialized.
    Array(Vec<JqValue<'a, W>>),

    /// JSON object with potentially mixed lazy/materialized values.
    ///
    /// Created when constructing objects with `{a: .x, b: .y + 1}`.
    /// Keys are always strings, values can be lazy or materialized.
    Object(IndexMap<String, JqValue<'a, W>>),
}

impl<'a, W: Clone + AsRef<[u64]>> JqValue<'a, W> {
    // =========================================================================
    // Constructors
    // =========================================================================

    /// Create a null value.
    #[inline]
    pub fn null() -> Self {
        JqValue::Null
    }

    /// Create a boolean value.
    #[inline]
    pub fn bool(b: bool) -> Self {
        JqValue::Bool(b)
    }

    /// Create an integer value.
    #[inline]
    pub fn int(n: i64) -> Self {
        JqValue::Int(n)
    }

    /// Create a float value.
    #[inline]
    pub fn float(f: f64) -> Self {
        JqValue::Float(f)
    }

    /// Create a string value.
    #[inline]
    pub fn string(s: impl Into<String>) -> Self {
        JqValue::String(s.into())
    }

    /// Create an empty array.
    #[inline]
    pub fn empty_array() -> Self {
        JqValue::Array(Vec::new())
    }

    /// Create an array from values.
    #[inline]
    pub fn array(values: Vec<JqValue<'a, W>>) -> Self {
        JqValue::Array(values)
    }

    /// Create an empty object.
    #[inline]
    pub fn empty_object() -> Self {
        JqValue::Object(IndexMap::new())
    }

    /// Create an object from key-value pairs.
    #[inline]
    pub fn object(pairs: impl IntoIterator<Item = (String, JqValue<'a, W>)>) -> Self {
        JqValue::Object(pairs.into_iter().collect())
    }

    /// Create from a cursor (lazy reference).
    #[inline]
    pub fn from_cursor(cursor: JsonCursor<'a, W>) -> Self {
        JqValue::Cursor(cursor)
    }

    /// Create from a literal.
    pub fn from_literal(lit: &Literal) -> Self {
        match lit {
            Literal::Null => JqValue::Null,
            Literal::Bool(b) => JqValue::Bool(*b),
            Literal::Int(n) => JqValue::Int(*n),
            Literal::Float(f) => JqValue::Float(*f),
            Literal::String(s) => JqValue::String(s.clone()),
        }
    }

    /// Create from an OwnedValue.
    pub fn from_owned(owned: OwnedValue) -> Self {
        match owned {
            OwnedValue::Null => JqValue::Null,
            OwnedValue::Bool(b) => JqValue::Bool(b),
            OwnedValue::Int(n) => JqValue::Int(n),
            OwnedValue::Float(f) => JqValue::Float(f),
            OwnedValue::String(s) => JqValue::String(s),
            OwnedValue::Array(arr) => {
                JqValue::Array(arr.into_iter().map(JqValue::from_owned).collect())
            }
            OwnedValue::Object(obj) => JqValue::Object(
                obj.into_iter()
                    .map(|(k, v)| (k, JqValue::from_owned(v)))
                    .collect(),
            ),
        }
    }

    // =========================================================================
    // Type checking
    // =========================================================================

    /// Check if this is a lazy cursor reference.
    #[inline]
    pub fn is_cursor(&self) -> bool {
        matches!(self, JqValue::Cursor(_))
    }

    /// Check if this value is null.
    pub fn is_null(&self) -> bool {
        match self {
            JqValue::Null => true,
            JqValue::Cursor(c) => matches!(c.value(), StandardJson::Null),
            _ => false,
        }
    }

    /// Check if this value is "truthy" (not null and not false).
    ///
    /// In jq, only `null` and `false` are falsy. Everything else
    /// (including 0, "", [], {}) is truthy.
    pub fn is_truthy(&self) -> bool {
        match self {
            JqValue::Null => false,
            JqValue::Bool(false) => false,
            JqValue::Cursor(c) => {
                !matches!(c.value(), StandardJson::Null | StandardJson::Bool(false))
            }
            _ => true,
        }
    }

    /// Get the type name of this value (for error messages).
    pub fn type_name(&self) -> &'static str {
        match self {
            JqValue::Cursor(c) => match c.value() {
                StandardJson::Null => "null",
                StandardJson::Bool(_) => "boolean",
                StandardJson::Number(_) => "number",
                StandardJson::String(_) => "string",
                StandardJson::Array(_) => "array",
                StandardJson::Object(_) => "object",
                StandardJson::Error(_) => "error",
            },
            JqValue::Null => "null",
            JqValue::Bool(_) => "boolean",
            JqValue::Int(_) | JqValue::Float(_) | JqValue::RawNumber(_) => "number",
            JqValue::String(_) => "string",
            JqValue::Array(_) => "array",
            JqValue::Object(_) => "object",
        }
    }

    // =========================================================================
    // Value accessors (force materialization when needed)
    // =========================================================================

    /// Get as boolean, if this is a boolean value.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            JqValue::Bool(b) => Some(*b),
            JqValue::Cursor(c) => match c.value() {
                StandardJson::Bool(b) => Some(b),
                _ => None,
            },
            _ => None,
        }
    }

    /// Get as i64, if this is an integer or integer-valued float.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            JqValue::Int(n) => Some(*n),
            JqValue::Float(f) if (*f - (*f as i64 as f64)).abs() < f64::EPSILON => Some(*f as i64),
            JqValue::RawNumber(bytes) => core::str::from_utf8(bytes)
                .ok()
                .and_then(|s| s.parse().ok()),
            JqValue::Cursor(c) => match c.value() {
                StandardJson::Number(n) => n.as_i64().ok(),
                _ => None,
            },
            _ => None,
        }
    }

    /// Get as f64, if this is a number.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            JqValue::Int(n) => Some(*n as f64),
            JqValue::Float(f) => Some(*f),
            JqValue::RawNumber(bytes) => core::str::from_utf8(bytes)
                .ok()
                .and_then(|s| s.parse().ok()),
            JqValue::Cursor(c) => match c.value() {
                StandardJson::Number(n) => n.as_f64().ok(),
                _ => None,
            },
            _ => None,
        }
    }

    /// Get as string reference.
    ///
    /// Returns `Cow` because:
    /// - For `JqValue::String`, returns a borrowed reference
    /// - For `JqValue::Cursor`, may need to unescape into owned string
    pub fn as_str(&self) -> Option<Cow<'_, str>> {
        match self {
            JqValue::String(s) => Some(Cow::Borrowed(s.as_str())),
            JqValue::Cursor(c) => match c.value() {
                StandardJson::String(s) => s.as_str().ok(),
                _ => None,
            },
            _ => None,
        }
    }

    /// Get the length of this value.
    ///
    /// - null: 0 (jq compat)
    /// - string: UTF-8 codepoint count
    /// - array: element count
    /// - object: key count
    /// - other: None (error)
    pub fn length(&self) -> Option<usize> {
        match self {
            JqValue::Null => Some(0),
            JqValue::String(s) => Some(s.chars().count()),
            JqValue::Array(arr) => Some(arr.len()),
            JqValue::Object(obj) => Some(obj.len()),
            JqValue::Cursor(c) => match c.value() {
                StandardJson::Null => Some(0),
                StandardJson::String(s) => s.as_str().ok().map(|s| s.chars().count()),
                StandardJson::Array(elements) => Some(elements.count()),
                StandardJson::Object(fields) => Some(fields.count()),
                _ => None,
            },
            _ => None,
        }
    }

    // =========================================================================
    // Navigation (for cursor values)
    // =========================================================================

    /// Get the cursor if this is a lazy value.
    #[inline]
    pub fn as_cursor(&self) -> Option<&JsonCursor<'a, W>> {
        match self {
            JqValue::Cursor(c) => Some(c),
            _ => None,
        }
    }

    /// Get the StandardJson value for a cursor, or None if materialized.
    pub fn as_standard_json(&self) -> Option<StandardJson<'a, W>> {
        match self {
            JqValue::Cursor(c) => Some(c.value()),
            _ => None,
        }
    }

    // =========================================================================
    // Materialization
    // =========================================================================

    /// Force full materialization into an OwnedValue.
    ///
    /// This recursively materializes all nested values. Use sparingly -
    /// prefer keeping values as cursors when possible.
    pub fn materialize(&self) -> OwnedValue {
        match self {
            JqValue::Cursor(c) => cursor_to_owned(c),
            JqValue::Null => OwnedValue::Null,
            JqValue::Bool(b) => OwnedValue::Bool(*b),
            JqValue::Int(n) => OwnedValue::Int(*n),
            JqValue::Float(f) => OwnedValue::Float(*f),
            JqValue::RawNumber(bytes) => {
                // Parse the raw number bytes
                if let Ok(s) = core::str::from_utf8(bytes) {
                    if let Ok(i) = s.parse::<i64>() {
                        return OwnedValue::Int(i);
                    }
                    if let Ok(f) = s.parse::<f64>() {
                        return OwnedValue::Float(f);
                    }
                }
                OwnedValue::Float(0.0)
            }
            JqValue::String(s) => OwnedValue::String(s.clone()),
            JqValue::Array(arr) => OwnedValue::Array(arr.iter().map(|v| v.materialize()).collect()),
            JqValue::Object(obj) => OwnedValue::Object(
                obj.iter()
                    .map(|(k, v)| (k.clone(), v.materialize()))
                    .collect(),
            ),
        }
    }

    /// Convert to OwnedValue, consuming self.
    ///
    /// More efficient than `materialize()` when you don't need to keep the original.
    pub fn into_owned(self) -> OwnedValue {
        match self {
            JqValue::Cursor(c) => cursor_to_owned(&c),
            JqValue::Null => OwnedValue::Null,
            JqValue::Bool(b) => OwnedValue::Bool(b),
            JqValue::Int(n) => OwnedValue::Int(n),
            JqValue::Float(f) => OwnedValue::Float(f),
            JqValue::RawNumber(bytes) => {
                // Parse the raw number bytes
                if let Ok(s) = core::str::from_utf8(bytes) {
                    if let Ok(i) = s.parse::<i64>() {
                        return OwnedValue::Int(i);
                    }
                    if let Ok(f) = s.parse::<f64>() {
                        return OwnedValue::Float(f);
                    }
                }
                OwnedValue::Float(0.0)
            }
            JqValue::String(s) => OwnedValue::String(s),
            JqValue::Array(arr) => {
                OwnedValue::Array(arr.into_iter().map(|v| v.into_owned()).collect())
            }
            JqValue::Object(obj) => {
                OwnedValue::Object(obj.into_iter().map(|(k, v)| (k, v.into_owned())).collect())
            }
        }
    }

    // =========================================================================
    // Output (preserves original formatting when possible)
    // =========================================================================

    /// Get the raw bytes for this value if it's a lazy reference.
    ///
    /// Returns `Some(&[u8])` for cursor values and raw numbers, `None` for
    /// materialized values. This allows zero-copy output for pass-through queries.
    pub fn raw_bytes(&self) -> Option<&'a [u8]> {
        match self {
            JqValue::Cursor(c) => c.raw_bytes(),
            JqValue::RawNumber(bytes) => Some(bytes),
            _ => None,
        }
    }

    /// Write this value as JSON to a writer.
    ///
    /// For cursor values, writes the original bytes (preserving formatting).
    /// For materialized values, serializes to JSON.
    ///
    /// This is the preferred way to output JqValue because it preserves
    /// number formatting like `4e4` for cursor values.
    pub fn write_json<Out: core::fmt::Write>(&self, out: &mut Out) -> core::fmt::Result {
        match self {
            JqValue::Cursor(c) => {
                if let Some(bytes) = c.raw_bytes() {
                    // Write raw bytes (preserves original formatting)
                    let s = core::str::from_utf8(bytes).map_err(|_| core::fmt::Error)?;
                    out.write_str(s)
                } else {
                    // Fallback: materialize and serialize
                    let owned = cursor_to_owned(c);
                    out.write_str(&owned.to_json())
                }
            }
            JqValue::Null => out.write_str("null"),
            JqValue::Bool(true) => out.write_str("true"),
            JqValue::Bool(false) => out.write_str("false"),
            JqValue::Int(n) => write!(out, "{}", n),
            JqValue::Float(f) => {
                if f.is_nan() || f.is_infinite() {
                    out.write_str("null")
                } else {
                    write!(out, "{}", f)
                }
            }
            JqValue::RawNumber(bytes) => {
                // Write raw bytes directly (preserves formatting like "4e4")
                let s = core::str::from_utf8(bytes).map_err(|_| core::fmt::Error)?;
                out.write_str(s)
            }
            JqValue::String(s) => {
                out.write_char('"')?;
                for c in s.chars() {
                    match c {
                        '"' => out.write_str("\\\"")?,
                        '\\' => out.write_str("\\\\")?,
                        '\n' => out.write_str("\\n")?,
                        '\r' => out.write_str("\\r")?,
                        '\t' => out.write_str("\\t")?,
                        c if c.is_control() => write!(out, "\\u{:04x}", c as u32)?,
                        c => out.write_char(c)?,
                    }
                }
                out.write_char('"')
            }
            JqValue::Array(arr) => {
                out.write_char('[')?;
                for (i, v) in arr.iter().enumerate() {
                    if i > 0 {
                        out.write_char(',')?;
                    }
                    v.write_json(out)?;
                }
                out.write_char(']')
            }
            JqValue::Object(obj) => {
                out.write_char('{')?;
                for (i, (k, v)) in obj.iter().enumerate() {
                    if i > 0 {
                        out.write_char(',')?;
                    }
                    // Write key
                    out.write_char('"')?;
                    for c in k.chars() {
                        match c {
                            '"' => out.write_str("\\\"")?,
                            '\\' => out.write_str("\\\\")?,
                            c => out.write_char(c)?,
                        }
                    }
                    out.write_str("\":")?;
                    v.write_json(out)?;
                }
                out.write_char('}')
            }
        }
    }

    /// Convert this value to a JSON string.
    ///
    /// For cursor values, returns the original bytes as a string.
    /// For materialized values, serializes to JSON.
    pub fn to_json_string(&self) -> String {
        let mut out = String::new();
        // Ignore error - writing to String can't fail
        let _ = self.write_json(&mut out);
        out
    }
}

/// Convert a JsonCursor to an OwnedValue (full materialization).
fn cursor_to_owned<W: Clone + AsRef<[u64]>>(cursor: &JsonCursor<'_, W>) -> OwnedValue {
    match cursor.value() {
        StandardJson::Null => OwnedValue::Null,
        StandardJson::Bool(b) => OwnedValue::Bool(b),
        StandardJson::Number(n) => {
            if let Ok(i) = n.as_i64() {
                OwnedValue::Int(i)
            } else if let Ok(f) = n.as_f64() {
                OwnedValue::Float(f)
            } else {
                OwnedValue::Float(0.0)
            }
        }
        StandardJson::String(s) => {
            if let Ok(cow) = s.as_str() {
                OwnedValue::String(cow.into_owned())
            } else {
                OwnedValue::String(String::new())
            }
        }
        StandardJson::Array(_) => {
            // Use cursor navigation to iterate children
            let items: Vec<OwnedValue> = cursor
                .children()
                .map(|child| cursor_to_owned(&child))
                .collect();
            OwnedValue::Array(items)
        }
        StandardJson::Object(fields) => {
            let mut map = IndexMap::new();
            for field in fields {
                if let StandardJson::String(key_str) = field.key() {
                    if let Ok(cow) = key_str.as_str() {
                        let value_cursor = field.value_cursor();
                        map.insert(cow.into_owned(), cursor_to_owned(&value_cursor));
                    }
                }
            }
            OwnedValue::Object(map)
        }
        StandardJson::Error(_) => OwnedValue::Null,
    }
}

// ============================================================================
// From implementations
// ============================================================================

impl<'a, W> From<bool> for JqValue<'a, W> {
    fn from(b: bool) -> Self {
        JqValue::Bool(b)
    }
}

impl<'a, W> From<i64> for JqValue<'a, W> {
    fn from(n: i64) -> Self {
        JqValue::Int(n)
    }
}

impl<'a, W> From<f64> for JqValue<'a, W> {
    fn from(f: f64) -> Self {
        JqValue::Float(f)
    }
}

impl<'a, W> From<String> for JqValue<'a, W> {
    fn from(s: String) -> Self {
        JqValue::String(s)
    }
}

impl<'a, W> From<&str> for JqValue<'a, W> {
    fn from(s: &str) -> Self {
        JqValue::String(s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constructors() {
        let null: JqValue<'_, Vec<u64>> = JqValue::null();
        assert!(null.is_null());

        let b: JqValue<'_, Vec<u64>> = JqValue::bool(true);
        assert_eq!(b.as_bool(), Some(true));

        let n: JqValue<'_, Vec<u64>> = JqValue::int(42);
        assert_eq!(n.as_i64(), Some(42));

        let f: JqValue<'_, Vec<u64>> = JqValue::float(2.5);
        assert!((f.as_f64().unwrap() - 2.5).abs() < f64::EPSILON);

        let s: JqValue<'_, Vec<u64>> = JqValue::string("hello");
        assert_eq!(
            s.as_str().map(|c| c.into_owned()),
            Some("hello".to_string())
        );
    }

    #[test]
    fn test_type_name() {
        let null: JqValue<'_, Vec<u64>> = JqValue::null();
        assert_eq!(null.type_name(), "null");

        let b: JqValue<'_, Vec<u64>> = JqValue::bool(true);
        assert_eq!(b.type_name(), "boolean");

        let n: JqValue<'_, Vec<u64>> = JqValue::int(42);
        assert_eq!(n.type_name(), "number");

        let f: JqValue<'_, Vec<u64>> = JqValue::float(2.5);
        assert_eq!(f.type_name(), "number");

        let s: JqValue<'_, Vec<u64>> = JqValue::string("hello");
        assert_eq!(s.type_name(), "string");

        let arr: JqValue<'_, Vec<u64>> = JqValue::empty_array();
        assert_eq!(arr.type_name(), "array");

        let obj: JqValue<'_, Vec<u64>> = JqValue::empty_object();
        assert_eq!(obj.type_name(), "object");
    }

    #[test]
    fn test_truthy() {
        let null: JqValue<'_, Vec<u64>> = JqValue::null();
        assert!(!null.is_truthy());

        let false_val: JqValue<'_, Vec<u64>> = JqValue::bool(false);
        assert!(!false_val.is_truthy());

        let true_val: JqValue<'_, Vec<u64>> = JqValue::bool(true);
        assert!(true_val.is_truthy());

        // In jq, 0 is truthy!
        let zero: JqValue<'_, Vec<u64>> = JqValue::int(0);
        assert!(zero.is_truthy());

        // Empty string is truthy in jq
        let empty: JqValue<'_, Vec<u64>> = JqValue::string("");
        assert!(empty.is_truthy());

        // Empty array is truthy in jq
        let arr: JqValue<'_, Vec<u64>> = JqValue::empty_array();
        assert!(arr.is_truthy());
    }

    #[test]
    fn test_length() {
        let null: JqValue<'_, Vec<u64>> = JqValue::null();
        assert_eq!(null.length(), Some(0));

        let s: JqValue<'_, Vec<u64>> = JqValue::string("hello");
        assert_eq!(s.length(), Some(5));

        let unicode: JqValue<'_, Vec<u64>> = JqValue::string("héllo");
        assert_eq!(unicode.length(), Some(5));

        let arr: JqValue<'_, Vec<u64>> = JqValue::array(vec![JqValue::int(1), JqValue::int(2)]);
        assert_eq!(arr.length(), Some(2));

        // Numbers don't have length
        let n: JqValue<'_, Vec<u64>> = JqValue::int(42);
        assert_eq!(n.length(), None);
    }

    #[test]
    fn test_materialize() {
        let arr: JqValue<'_, Vec<u64>> = JqValue::array(vec![
            JqValue::int(1),
            JqValue::string("hello"),
            JqValue::null(),
        ]);

        let owned = arr.materialize();
        match owned {
            OwnedValue::Array(items) => {
                assert_eq!(items.len(), 3);
                assert_eq!(items[0], OwnedValue::Int(1));
                assert_eq!(items[1], OwnedValue::String("hello".to_string()));
                assert_eq!(items[2], OwnedValue::Null);
            }
            _ => panic!("expected array"),
        }
    }

    #[test]
    fn test_from_literal() {
        let lit = Literal::Int(42);
        let val: JqValue<'_, Vec<u64>> = JqValue::from_literal(&lit);
        assert_eq!(val.as_i64(), Some(42));

        let lit = Literal::String("hello".to_string());
        let val: JqValue<'_, Vec<u64>> = JqValue::from_literal(&lit);
        assert_eq!(
            val.as_str().map(|c| c.into_owned()),
            Some("hello".to_string())
        );
    }

    #[test]
    fn test_cursor_raw_bytes_simple_number() {
        use crate::json::JsonIndex;

        // Just a number
        let json = br#"4e4"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);

        let val = JqValue::from_cursor(cursor);
        let bytes = val.raw_bytes();
        assert!(
            bytes.is_some(),
            "raw_bytes should return Some for simple number"
        );
        assert_eq!(bytes.unwrap(), json.as_slice());
    }

    #[test]
    fn test_write_json_preserves_cursor_format() {
        use crate::json::JsonIndex;

        // JSON with exponential notation that would be reformatted if parsed
        let json = br#"4e4"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);

        let val = JqValue::from_cursor(cursor);
        let output = val.to_json_string();

        // Should preserve original format "4e4", not "40000"
        assert_eq!(output, "4e4");
    }

    #[test]
    fn test_write_json_materialized() {
        // Materialized values serialize normally
        let arr: JqValue<'_, Vec<u64>> = JqValue::array(vec![
            JqValue::int(1),
            JqValue::string("hello"),
            JqValue::bool(true),
        ]);

        let output = arr.to_json_string();
        assert_eq!(output, r#"[1,"hello",true]"#);
    }

    #[test]
    fn test_mixed_cursor_and_materialized() {
        use crate::json::JsonIndex;

        // Create a cursor value
        let json = br#"4e4"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let cursor_val = JqValue::from_cursor(cursor);

        // Create a materialized value
        let computed_val: JqValue<'_, Vec<u64>> = JqValue::int(100);

        // Mix them in an array
        let arr = JqValue::array(vec![cursor_val, computed_val]);
        let output = arr.to_json_string();

        // cursor_val should preserve "4e4", computed_val is "100"
        assert_eq!(output, "[4e4,100]");
    }
}
