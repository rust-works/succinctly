//! Streaming output support for jq evaluation results.
//!
//! This module provides the `StreamableValue` trait which enables streaming output
//! without intermediate allocations. Both cursor-based values (YamlCursor) and
//! owned values (OwnedValue) can implement this trait.
//!
//! ## M2 Streaming Optimization
//!
//! The M2 streaming path allows navigation queries (`.field`, `.[0]`, `.[]`) to
//! stream their results directly to output without materializing OwnedValue DOMs.
//! This provides significant memory savings for queries that only navigate to
//! subtrees of the document.
//!
//! ### Execution Paths
//!
//! | Query Type | Execution Path | Memory Usage |
//! |------------|----------------|--------------|
//! | `.` (identity) | P9 streaming | ~2x input |
//! | `.field`, `.[0]` | M2 streaming | ~2x input |
//! | `.[]` (iterate) | M2 streaming | ~2.5x input |
//! | `length`, complex | OwnedValue | 5-8x input |

use super::value::OwnedValue;
use crate::yaml::simd::find_json_escape;

/// A value that can be streamed directly to output without intermediate allocation.
///
/// This trait abstracts over both cursor-based navigation (YamlCursor, JsonCursor)
/// and owned values (OwnedValue), enabling unified streaming output regardless of
/// how the value was obtained.
pub trait StreamableValue {
    /// Stream this value as compact JSON to the output.
    ///
    /// The output should be valid JSON without trailing newlines or separators.
    fn stream_json<W: core::fmt::Write>(&self, out: &mut W) -> core::fmt::Result;

    /// Stream this value as YAML to the output.
    ///
    /// The output should be valid YAML. For block style, pass indent_spaces > 0.
    /// For flow style (compact), pass indent_spaces = 0.
    fn stream_yaml<W: core::fmt::Write>(
        &self,
        out: &mut W,
        indent_spaces: usize,
    ) -> core::fmt::Result;

    /// Check if this value is falsy (null or false).
    ///
    /// Used for `--exit-status` flag handling without requiring full materialization.
    fn is_falsy(&self) -> bool;
}

/// Statistics returned from streaming operations.
///
/// Used to track output for `--exit-status` handling.
#[derive(Debug, Clone, Default)]
pub struct StreamStats {
    /// Number of values streamed.
    pub count: usize,
    /// Whether the last value was falsy (null or false).
    pub last_was_falsy: bool,
}

impl StreamableValue for OwnedValue {
    fn stream_json<W: core::fmt::Write>(&self, out: &mut W) -> core::fmt::Result {
        stream_owned_value_json(self, out)
    }

    fn stream_yaml<W: core::fmt::Write>(
        &self,
        out: &mut W,
        indent_spaces: usize,
    ) -> core::fmt::Result {
        stream_owned_value_yaml(self, out, 0, indent_spaces)
    }

    fn is_falsy(&self) -> bool {
        matches!(self, OwnedValue::Null | OwnedValue::Bool(false))
    }
}

/// Stream an OwnedValue as JSON without intermediate string allocation.
fn stream_owned_value_json<W: core::fmt::Write>(
    value: &OwnedValue,
    out: &mut W,
) -> core::fmt::Result {
    match value {
        OwnedValue::Null => out.write_str("null"),
        OwnedValue::Bool(true) => out.write_str("true"),
        OwnedValue::Bool(false) => out.write_str("false"),
        OwnedValue::Int(n) => write!(out, "{}", n),
        OwnedValue::Float(f) => {
            if f.is_nan() || f.is_infinite() {
                // JSON doesn't support NaN or Infinity
                out.write_str("null")
            } else {
                write!(out, "{}", f)
            }
        }
        OwnedValue::String(s) => stream_json_string(out, s),
        OwnedValue::Array(arr) => {
            out.write_char('[')?;
            for (i, elem) in arr.iter().enumerate() {
                if i > 0 {
                    out.write_char(',')?;
                }
                stream_owned_value_json(elem, out)?;
            }
            out.write_char(']')
        }
        OwnedValue::Object(obj) => {
            out.write_char('{')?;
            for (i, (key, value)) in obj.iter().enumerate() {
                if i > 0 {
                    out.write_char(',')?;
                }
                stream_json_string(out, key)?;
                out.write_char(':')?;
                stream_owned_value_json(value, out)?;
            }
            out.write_char('}')
        }
    }
}

/// Stream a string as JSON with proper escaping.
///
/// Uses SIMD-accelerated escape detection (O3 optimization from issue #81) to
/// find characters that need escaping in 16-32 byte chunks, then copies safe
/// spans directly to output.
fn stream_json_string<W: core::fmt::Write>(out: &mut W, s: &str) -> core::fmt::Result {
    out.write_char('"')?;

    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // SIMD-accelerated scan for next escapable character (", \, or < 0x20)
        // Returns index of first match, or len if no escapes needed
        let escape_pos = find_json_escape(bytes, i);

        // Copy the safe span directly (no escaping needed)
        if i < escape_pos {
            out.write_str(&s[i..escape_pos])?;
        }

        i = escape_pos;

        // Handle escape sequence if needed
        if i < len {
            let b = bytes[i];
            match b {
                b'"' => out.write_str("\\\"")?,
                b'\\' => out.write_str("\\\\")?,
                b'\n' => out.write_str("\\n")?,
                b'\r' => out.write_str("\\r")?,
                b'\t' => out.write_str("\\t")?,
                b if b < 0x20 => {
                    // Control character - escape as \u00XX
                    out.write_str("\\u00")?;
                    const HEX: &[u8; 16] = b"0123456789abcdef";
                    out.write_char(HEX[(b >> 4) as usize] as char)?;
                    out.write_char(HEX[(b & 0xf) as usize] as char)?;
                }
                _ => out.write_char(b as char)?,
            }
            i += 1;
        }
    }

    out.write_char('"')
}

// ============================================================================
// YAML Streaming
// ============================================================================

/// Stream an OwnedValue as YAML without intermediate string allocation.
///
/// - `current_indent`: Current indentation level (number of spaces)
/// - `indent_spaces`: Spaces per indentation level (0 for flow style)
fn stream_owned_value_yaml<W: core::fmt::Write>(
    value: &OwnedValue,
    out: &mut W,
    current_indent: usize,
    indent_spaces: usize,
) -> core::fmt::Result {
    match value {
        OwnedValue::Null => out.write_str("null"),
        OwnedValue::Bool(true) => out.write_str("true"),
        OwnedValue::Bool(false) => out.write_str("false"),
        OwnedValue::Int(n) => write!(out, "{}", n),
        OwnedValue::Float(f) => {
            if f.is_nan() {
                out.write_str(".nan")
            } else if f.is_infinite() {
                if *f > 0.0 {
                    out.write_str(".inf")
                } else {
                    out.write_str("-.inf")
                }
            } else {
                write!(out, "{}", f)
            }
        }
        OwnedValue::String(s) => stream_yaml_string(out, s),
        OwnedValue::Array(arr) => {
            if arr.is_empty() {
                out.write_str("[]")
            } else if indent_spaces == 0 {
                // Flow style
                out.write_char('[')?;
                for (i, elem) in arr.iter().enumerate() {
                    if i > 0 {
                        out.write_str(", ")?;
                    }
                    stream_owned_value_yaml(elem, out, 0, 0)?;
                }
                out.write_char(']')
            } else {
                // Block style
                for (i, elem) in arr.iter().enumerate() {
                    if i > 0 {
                        out.write_char('\n')?;
                        write_indent(out, current_indent)?;
                    }
                    out.write_str("- ")?;
                    // For nested containers, put on next line with extra indent
                    if matches!(elem, OwnedValue::Array(_) | OwnedValue::Object(_))
                        && !is_empty_container(elem)
                    {
                        out.write_char('\n')?;
                        write_indent(out, current_indent + indent_spaces)?;
                        stream_owned_value_yaml(
                            elem,
                            out,
                            current_indent + indent_spaces,
                            indent_spaces,
                        )?;
                    } else {
                        stream_owned_value_yaml(
                            elem,
                            out,
                            current_indent + indent_spaces,
                            indent_spaces,
                        )?;
                    }
                }
                Ok(())
            }
        }
        OwnedValue::Object(obj) => {
            if obj.is_empty() {
                out.write_str("{}")
            } else if indent_spaces == 0 {
                // Flow style
                out.write_char('{')?;
                for (i, (key, val)) in obj.iter().enumerate() {
                    if i > 0 {
                        out.write_str(", ")?;
                    }
                    stream_yaml_string(out, key)?;
                    out.write_str(": ")?;
                    stream_owned_value_yaml(val, out, 0, 0)?;
                }
                out.write_char('}')
            } else {
                // Block style
                for (i, (key, val)) in obj.iter().enumerate() {
                    if i > 0 {
                        out.write_char('\n')?;
                        write_indent(out, current_indent)?;
                    }
                    stream_yaml_string(out, key)?;
                    out.write_str(":")?;
                    // For nested containers, put on next line with extra indent
                    if matches!(val, OwnedValue::Array(_) | OwnedValue::Object(_))
                        && !is_empty_container(val)
                    {
                        out.write_char('\n')?;
                        write_indent(out, current_indent + indent_spaces)?;
                        stream_owned_value_yaml(
                            val,
                            out,
                            current_indent + indent_spaces,
                            indent_spaces,
                        )?;
                    } else {
                        out.write_char(' ')?;
                        stream_owned_value_yaml(
                            val,
                            out,
                            current_indent + indent_spaces,
                            indent_spaces,
                        )?;
                    }
                }
                Ok(())
            }
        }
    }
}

/// Check if a value is an empty array or object.
fn is_empty_container(value: &OwnedValue) -> bool {
    match value {
        OwnedValue::Array(arr) => arr.is_empty(),
        OwnedValue::Object(obj) => obj.is_empty(),
        _ => false,
    }
}

/// Write indentation spaces.
fn write_indent<W: core::fmt::Write>(out: &mut W, spaces: usize) -> core::fmt::Result {
    for _ in 0..spaces {
        out.write_char(' ')?;
    }
    Ok(())
}

/// Stream a string as YAML with smart quoting.
///
/// Uses double quotes if the string contains special characters,
/// otherwise outputs unquoted or single-quoted based on content.
pub fn stream_yaml_string<W: core::fmt::Write>(out: &mut W, s: &str) -> core::fmt::Result {
    if s.is_empty() {
        return out.write_str("''");
    }

    // Check if we need quoting
    if needs_yaml_quoting(s) {
        stream_yaml_double_quoted(out, s)
    } else {
        out.write_str(s)
    }
}

/// Check if a string needs quoting in YAML.
fn needs_yaml_quoting(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }

    let bytes = s.as_bytes();

    // Check first character - indicators that require quoting
    let first = bytes[0];
    if matches!(
        first,
        b'-' | b'?'
            | b':'
            | b','
            | b'['
            | b']'
            | b'{'
            | b'}'
            | b'#'
            | b'&'
            | b'*'
            | b'!'
            | b'|'
            | b'>'
            | b'\''
            | b'"'
            | b'%'
            | b'@'
            | b'`'
    ) {
        return true;
    }

    // Check for leading/trailing whitespace
    if bytes[0] == b' ' || bytes[bytes.len() - 1] == b' ' {
        return true;
    }

    // Check for special values that look like YAML keywords
    let lower = s.to_lowercase();
    if matches!(
        lower.as_str(),
        "null" | "~" | "true" | "false" | "yes" | "no" | "on" | "off" | ".inf" | "-.inf" | ".nan"
    ) {
        return true;
    }

    // Check if it looks like a number
    if looks_like_number(s) {
        return true;
    }

    // Check for characters that need escaping
    for b in bytes {
        if *b < 0x20 || *b == b':' || *b == b'#' {
            return true;
        }
    }

    false
}

/// Check if a string looks like a number.
fn looks_like_number(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }

    let bytes = s.as_bytes();
    let mut i = 0;

    // Optional sign
    if bytes[i] == b'-' || bytes[i] == b'+' {
        i += 1;
        if i >= bytes.len() {
            return false;
        }
    }

    // Must have at least one digit
    if !bytes[i].is_ascii_digit() {
        return false;
    }

    // Check remaining characters
    let mut has_dot = false;
    let mut has_exp = false;
    while i < bytes.len() {
        match bytes[i] {
            b'0'..=b'9' => {}
            b'.' if !has_dot && !has_exp => has_dot = true,
            b'e' | b'E' if !has_exp => {
                has_exp = true;
                // Optional sign after exponent
                if i + 1 < bytes.len() && (bytes[i + 1] == b'-' || bytes[i + 1] == b'+') {
                    i += 1;
                }
            }
            _ => return false,
        }
        i += 1;
    }

    true
}

/// Stream a double-quoted YAML string with proper escaping.
fn stream_yaml_double_quoted<W: core::fmt::Write>(out: &mut W, s: &str) -> core::fmt::Result {
    out.write_char('"')?;

    for ch in s.chars() {
        match ch {
            '"' => out.write_str("\\\"")?,
            '\\' => out.write_str("\\\\")?,
            '\n' => out.write_str("\\n")?,
            '\r' => out.write_str("\\r")?,
            '\t' => out.write_str("\\t")?,
            c if c < ' ' => {
                // Control characters as \xNN
                let b = c as u8;
                out.write_str("\\x")?;
                const HEX: &[u8; 16] = b"0123456789abcdef";
                out.write_char(HEX[(b >> 4) as usize] as char)?;
                out.write_char(HEX[(b & 0xf) as usize] as char)?;
            }
            c => out.write_char(c)?,
        }
    }

    out.write_char('"')
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;

    #[test]
    fn test_stream_null() {
        let mut buf = String::new();
        OwnedValue::Null.stream_json(&mut buf).unwrap();
        assert_eq!(buf, "null");
    }

    #[test]
    fn test_stream_bool() {
        let mut buf = String::new();
        OwnedValue::Bool(true).stream_json(&mut buf).unwrap();
        assert_eq!(buf, "true");

        buf.clear();
        OwnedValue::Bool(false).stream_json(&mut buf).unwrap();
        assert_eq!(buf, "false");
    }

    #[test]
    fn test_stream_int() {
        let mut buf = String::new();
        OwnedValue::Int(42).stream_json(&mut buf).unwrap();
        assert_eq!(buf, "42");

        buf.clear();
        OwnedValue::Int(-123).stream_json(&mut buf).unwrap();
        assert_eq!(buf, "-123");
    }

    #[test]
    fn test_stream_float() {
        let mut buf = String::new();
        OwnedValue::Float(3.125).stream_json(&mut buf).unwrap();
        assert_eq!(buf, "3.125");
    }

    #[test]
    fn test_stream_string() {
        let mut buf = String::new();
        OwnedValue::String("hello".to_string())
            .stream_json(&mut buf)
            .unwrap();
        assert_eq!(buf, "\"hello\"");
    }

    #[test]
    fn test_stream_string_escaping() {
        let mut buf = String::new();
        OwnedValue::String("hello\nworld".to_string())
            .stream_json(&mut buf)
            .unwrap();
        assert_eq!(buf, "\"hello\\nworld\"");

        buf.clear();
        OwnedValue::String("tab\there".to_string())
            .stream_json(&mut buf)
            .unwrap();
        assert_eq!(buf, "\"tab\\there\"");

        buf.clear();
        OwnedValue::String("quote\"here".to_string())
            .stream_json(&mut buf)
            .unwrap();
        assert_eq!(buf, "\"quote\\\"here\"");
    }

    #[test]
    fn test_stream_array() {
        let mut buf = String::new();
        OwnedValue::Array(vec![
            OwnedValue::Int(1),
            OwnedValue::Int(2),
            OwnedValue::Int(3),
        ])
        .stream_json(&mut buf)
        .unwrap();
        assert_eq!(buf, "[1,2,3]");
    }

    #[test]
    fn test_stream_object() {
        let mut buf = String::new();
        let mut map = IndexMap::new();
        map.insert("name".to_string(), OwnedValue::String("Alice".to_string()));
        map.insert("age".to_string(), OwnedValue::Int(30));
        OwnedValue::Object(map).stream_json(&mut buf).unwrap();
        assert_eq!(buf, "{\"name\":\"Alice\",\"age\":30}");
    }

    #[test]
    fn test_is_falsy() {
        assert!(OwnedValue::Null.is_falsy());
        assert!(OwnedValue::Bool(false).is_falsy());
        assert!(!OwnedValue::Bool(true).is_falsy());
        assert!(!OwnedValue::Int(0).is_falsy());
        assert!(!OwnedValue::String("".to_string()).is_falsy());
    }
}
