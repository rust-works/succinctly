//! Output helpers shared by the jq and yq CLI runners.
//!
//! Exit codes, JSON string escaping, JSON pretty-printing, ANSI colorization
//! (including `JQ_COLORS` support), and build-configuration diagnostics.

use succinctly::jq::OwnedValue;

/// Exit codes matching jq behavior
pub mod exit_codes {
    pub const SUCCESS: i32 = 0;
    pub const FALSE_OR_NULL: i32 = 1; // With -e, last output was false or null
    #[allow(dead_code)] // STYLE-0005: complete jq exit-code set; not all emitted yet
    pub const USAGE_ERROR: i32 = 2; // Usage problem or system error
    pub const COMPILE_ERROR: i32 = 3; // jq program compile error
    pub const NO_OUTPUT: i32 = 4; // With -e, no valid result produced
    #[allow(dead_code)] // STYLE-0005: complete jq exit-code set; not all emitted yet
    pub const HALT_ERROR: i32 = 5; // halt_error without explicit code
}

/// Print build configuration information (similar to jq --build-configuration)
pub fn print_build_configuration(tool: &str) {
    println!("succinctly {tool} build configuration:");
    println!();
    println!("Version: {}", env!("CARGO_PKG_VERSION"));
    println!(
        "Target: {}-{}-{}",
        std::env::consts::ARCH,
        std::env::consts::FAMILY,
        std::env::consts::OS
    );
    println!(
        "Profile: {}",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    println!();
    println!("Features:");
    println!("  std: {}", cfg!(feature = "std"));
    println!("  simd: {}", cfg!(feature = "simd"));
    println!("  regex: {}", cfg!(feature = "regex"));
    println!();
    println!("Platform:");
    println!("  OS: {}", std::env::consts::OS);
    println!("  Arch: {}", std::env::consts::ARCH);
    println!("  Family: {}", std::env::consts::FAMILY);
    #[cfg(target_arch = "x86_64")]
    {
        println!();
        println!("x86_64 CPU features (runtime detected):");
        println!("  SSE2: true (baseline)");
        println!("  SSE4.2: {}", is_x86_feature_detected!("sse4.2"));
        println!("  AVX2: {}", is_x86_feature_detected!("avx2"));
        println!("  POPCNT: {}", is_x86_feature_detected!("popcnt"));
        println!("  BMI1: {}", is_x86_feature_detected!("bmi1"));
        println!("  BMI2: {}", is_x86_feature_detected!("bmi2"));
    }
    #[cfg(target_arch = "aarch64")]
    {
        println!();
        println!("aarch64 CPU features:");
        println!("  NEON: true (mandatory on aarch64)");
    }
}

/// Escape special characters in a JSON string.
///
/// Returns the escaped body without surrounding quotes; callers add them.
pub fn escape_json_string(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\x08' => result.push_str("\\b"), // backspace
            '\x0C' => result.push_str("\\f"), // form feed
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

/// Escape special characters in a JSON string, also escaping non-ASCII as \uXXXX.
///
/// Returns the escaped body without surrounding quotes; callers add them.
pub fn escape_json_string_ascii(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\x08' => result.push_str("\\b"), // backspace
            '\x0C' => result.push_str("\\f"), // form feed
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            c if c.is_control() => {
                result.push_str(&format!("\\u{:04x}", c as u32));
            }
            c if !c.is_ascii() => {
                // Escape non-ASCII characters as \uXXXX
                // For characters outside BMP, use surrogate pairs
                let code = c as u32;
                if code <= 0xFFFF {
                    result.push_str(&format!("\\u{code:04x}"));
                } else {
                    // Surrogate pair for characters above U+FFFF
                    let adjusted = code - 0x10000;
                    let high = 0xD800 + (adjusted >> 10);
                    let low = 0xDC00 + (adjusted & 0x3FF);
                    result.push_str(&format!("\\u{high:04x}\\u{low:04x}"));
                }
            }
            c => result.push(c),
        }
    }
    result
}

/// How to render finite floats with no fractional part.
#[derive(Clone, Copy, Debug)]
pub enum FloatStyle {
    /// Rust's shortest representation: `1.0` prints as `1` (jq).
    Shortest,
    /// Keep a trailing `.0` on whole floats in i64 range: `1.0` prints as `1.0` (yq).
    #[allow(dead_code)]
    // STYLE-0005: constructed by the yq runner migration in the next commit
    PreserveWholeFloat,
}

/// Options for [`format_json`].
pub struct JsonFormatOpts<'a> {
    /// Indent unit per nesting level; empty selects compact output.
    pub indent: &'a str,
    /// Sort object keys lexicographically.
    pub sort_keys: bool,
    /// Escape non-ASCII characters as \uXXXX.
    pub ascii: bool,
    /// Rendering of whole floats.
    pub float_style: FloatStyle,
}

/// Format a value as JSON text (compact or pretty, per `opts`).
pub fn format_json(value: &OwnedValue, opts: &JsonFormatOpts) -> String {
    format_json_impl(value, opts, 0)
}

/// Recursive JSON formatter behind [`format_json`].
fn format_json_impl(value: &OwnedValue, opts: &JsonFormatOpts, level: usize) -> String {
    let indent = opts.indent;
    let compact = indent.is_empty();
    let current_indent = if compact {
        String::new()
    } else {
        indent.repeat(level)
    };
    let next_indent = if compact {
        String::new()
    } else {
        indent.repeat(level + 1)
    };
    let separator = if compact { "" } else { "\n" };
    let space_after_colon = if compact { "" } else { " " };

    match value {
        OwnedValue::Null => "null".to_string(),
        OwnedValue::Bool(b) => b.to_string(),
        OwnedValue::Int(i) => i.to_string(),
        OwnedValue::Float(f) => {
            if f.is_nan() || f.is_infinite() {
                "null".to_string() // JSON doesn't support NaN or Infinity
            } else {
                match opts.float_style {
                    FloatStyle::Shortest => f.to_string(),
                    FloatStyle::PreserveWholeFloat => {
                        if f.fract() == 0.0 && *f >= i64::MIN as f64 && *f <= i64::MAX as f64 {
                            format!("{f:.1}") // Preserve decimal point for whole numbers
                        } else {
                            f.to_string()
                        }
                    }
                }
            }
        }
        OwnedValue::String(s) => {
            if opts.ascii {
                format!("\"{}\"", escape_json_string_ascii(s))
            } else {
                format!("\"{}\"", escape_json_string(s))
            }
        }
        OwnedValue::Array(arr) => {
            if arr.is_empty() {
                "[]".to_string()
            } else if compact {
                let items: Vec<String> = arr
                    .iter()
                    .map(|v| format_json_impl(v, opts, level + 1))
                    .collect();
                format!("[{}]", items.join(","))
            } else {
                let items: Vec<String> = arr
                    .iter()
                    .map(|v| format!("{}{}", next_indent, format_json_impl(v, opts, level + 1)))
                    .collect();
                format!(
                    "[{}{}{separator}{}]",
                    separator,
                    items.join(&format!(",{separator}")),
                    current_indent
                )
            }
        }
        OwnedValue::Object(obj) => {
            if obj.is_empty() {
                return "{}".to_string();
            }
            let mut entries: Vec<(&String, &OwnedValue)> = obj.iter().collect();
            if opts.sort_keys {
                entries.sort_by(|a, b| a.0.cmp(b.0));
            }
            if compact {
                let items: Vec<String> = entries
                    .iter()
                    .map(|(k, v)| {
                        let key = if opts.ascii {
                            escape_json_string_ascii(k)
                        } else {
                            escape_json_string(k)
                        };
                        format!("\"{}\":{}", key, format_json_impl(v, opts, level + 1))
                    })
                    .collect();
                format!("{{{}}}", items.join(","))
            } else {
                let items: Vec<String> = entries
                    .iter()
                    .map(|(k, v)| {
                        let key = if opts.ascii {
                            escape_json_string_ascii(k)
                        } else {
                            escape_json_string(k)
                        };
                        format!(
                            "\"{}\":{}{}",
                            key,
                            space_after_colon,
                            format_json_impl(v, opts, level + 1)
                        )
                    })
                    .collect();
                // Add indent before each key
                let indented_items: Vec<String> = items
                    .iter()
                    .map(|item| format!("{next_indent}{item}"))
                    .collect();
                format!(
                    "{{{}{}{separator}{}}}",
                    separator,
                    indented_items.join(&format!(",{separator}")),
                    current_indent
                )
            }
        }
    }
}

/// Default ANSI color codes for JSON syntax highlighting.
/// These match jq's default colors.
mod default_colors {
    pub const RESET: &str = "\x1b[0m";
    pub const NULL: &str = "\x1b[1;30m"; // Bold black (gray) - jq default
    pub const FALSE: &str = "\x1b[0;39m"; // Default - jq default
    pub const TRUE: &str = "\x1b[0;39m"; // Default - jq default
    pub const NUMBER: &str = "\x1b[0;39m"; // Default - jq default
    pub const STRING: &str = "\x1b[0;32m"; // Green - jq default
    pub const ARRAY: &str = "\x1b[1;39m"; // Bold default - jq default
    pub const OBJECT: &str = "\x1b[1;39m"; // Bold default - jq default
    pub const KEY: &str = "\x1b[1;34m"; // Bold blue - jq default (or 1;39)
}

/// Color scheme for JSON syntax highlighting.
/// Can be customized via JQ_COLORS environment variable.
#[derive(Clone)]
pub struct ColorScheme {
    reset: String,
    null: String,
    false_: String,
    true_: String,
    number: String,
    string: String,
    array: String,
    object: String,
    key: String,
}

impl Default for ColorScheme {
    fn default() -> Self {
        Self {
            reset: default_colors::RESET.to_string(),
            null: default_colors::NULL.to_string(),
            false_: default_colors::FALSE.to_string(),
            true_: default_colors::TRUE.to_string(),
            number: default_colors::NUMBER.to_string(),
            string: default_colors::STRING.to_string(),
            array: default_colors::ARRAY.to_string(),
            object: default_colors::OBJECT.to_string(),
            key: default_colors::KEY.to_string(),
        }
    }
}

/// Number of colors `JQ_COLORS` can set. Fields past this are ignored, as in jq.
const JQ_COLORS_FIELDS: usize = 8;

/// Is `sgr` a valid `JQ_COLORS` field?
///
/// jq accepts only digits and `;`, so an SGR parameter is the only thing that can
/// reach the terminal. The empty string is valid and selects `\x1b[m`.
fn is_valid_sgr(sgr: &str) -> bool {
    sgr.bytes().all(|b| b.is_ascii_digit() || b == b';')
}

impl ColorScheme {
    /// Parse a `JQ_COLORS` spec.
    ///
    /// Format: "null:false:true:numbers:strings:arrays:objects:objectkeys".
    /// Each field is an SGR parameter like "1;30" for bold black.
    ///
    /// Returns `None` if any of the first [`JQ_COLORS_FIELDS`] fields is invalid.
    /// jq rejects a malformed spec as a whole rather than keeping the fields that
    /// did parse, so callers fall back to the complete default scheme.
    ///
    /// Absent trailing fields keep their default; an empty field selects `\x1b[m`;
    /// fields beyond the eighth are ignored without being validated.
    fn from_spec(spec: &str) -> Option<Self> {
        if !spec.split(':').take(JQ_COLORS_FIELDS).all(is_valid_sgr) {
            return None;
        }

        let mut scheme = Self::default();
        let fields: [&mut String; JQ_COLORS_FIELDS] = [
            &mut scheme.null,
            &mut scheme.false_,
            &mut scheme.true_,
            &mut scheme.number,
            &mut scheme.string,
            &mut scheme.array,
            &mut scheme.object,
            &mut scheme.key,
        ];

        // zip stops at the shorter side, so a short spec leaves the remaining
        // colors at their defaults and a long one drops the excess.
        for (field, sgr) in fields.into_iter().zip(spec.split(':')) {
            *field = format!("\x1b[{sgr}m");
        }

        Some(scheme)
    }

    /// Read the color scheme from the `JQ_COLORS` environment variable.
    pub fn from_env() -> Self {
        let Ok(spec) = std::env::var("JQ_COLORS") else {
            return Self::default();
        };

        Self::from_spec(&spec).unwrap_or_else(|| {
            // Matches jq: warn on stderr, use defaults, but still exit successfully.
            eprintln!("Failed to set $JQ_COLORS");
            Self::default()
        })
    }
}

/// Colorize a JSON string using ANSI escape codes.
/// This is a simple parser that adds colors to JSON tokens.
pub fn colorize_json(json: &str, scheme: &ColorScheme) -> String {
    let mut result = String::with_capacity(json.len() * 2);
    let mut chars = json.chars().peekable();
    let mut in_string = false;
    let mut escape_next = false;
    let mut depth_stack: Vec<char> = Vec::new(); // Track context: '{' for object, '[' for array
    let mut expecting_key = false; // True when next string in object is a key

    while let Some(c) = chars.next() {
        if escape_next {
            result.push(c);
            escape_next = false;
            continue;
        }

        if in_string {
            if c == '\\' {
                result.push(c);
                escape_next = true;
            } else if c == '"' {
                result.push(c);
                result.push_str(&scheme.reset);
                in_string = false;
            } else {
                result.push(c);
            }
        } else {
            match c {
                '"' => {
                    // Use expecting_key to determine if this is a key
                    if expecting_key {
                        result.push_str(&scheme.key);
                        expecting_key = false; // After seeing key, next string is value
                    } else {
                        result.push_str(&scheme.string);
                    }
                    result.push(c);
                    in_string = true;
                }
                '{' => {
                    result.push_str(&scheme.object);
                    result.push(c);
                    result.push_str(&scheme.reset);
                    depth_stack.push('{');
                    expecting_key = true; // First thing in object is a key
                }
                '[' => {
                    result.push_str(&scheme.array);
                    result.push(c);
                    result.push_str(&scheme.reset);
                    depth_stack.push('[');
                    // Arrays don't have keys
                }
                '}' => {
                    result.push_str(&scheme.object);
                    result.push(c);
                    result.push_str(&scheme.reset);
                    depth_stack.pop();
                    expecting_key = false;
                }
                ']' => {
                    result.push_str(&scheme.array);
                    result.push(c);
                    result.push_str(&scheme.reset);
                    depth_stack.pop();
                    expecting_key = false;
                }
                ':' => {
                    result.push(c);
                    // After colon, we're expecting a value, not a key
                    expecting_key = false;
                }
                ',' => {
                    result.push(c);
                    // After comma in object context, next string is a key
                    if depth_stack.last() == Some(&'{') {
                        expecting_key = true;
                    }
                }
                't' => {
                    // true
                    result.push_str(&scheme.true_);
                    result.push(c);
                    // Consume rest of the keyword
                    while let Some(&next) = chars.peek() {
                        if next.is_alphabetic() {
                            result.push(chars.next().unwrap());
                        } else {
                            break;
                        }
                    }
                    result.push_str(&scheme.reset);
                }
                'f' => {
                    // false
                    result.push_str(&scheme.false_);
                    result.push(c);
                    // Consume rest of the keyword
                    while let Some(&next) = chars.peek() {
                        if next.is_alphabetic() {
                            result.push(chars.next().unwrap());
                        } else {
                            break;
                        }
                    }
                    result.push_str(&scheme.reset);
                }
                'n' => {
                    // null
                    result.push_str(&scheme.null);
                    result.push(c);
                    while let Some(&next) = chars.peek() {
                        if next.is_alphabetic() {
                            result.push(chars.next().unwrap());
                        } else {
                            break;
                        }
                    }
                    result.push_str(&scheme.reset);
                }
                '0'..='9' | '-' | '.' | 'e' | 'E' | '+' => {
                    result.push_str(&scheme.number);
                    result.push(c);
                    // Consume rest of number
                    while let Some(&next) = chars.peek() {
                        if next.is_ascii_digit()
                            || next == '.'
                            || next == 'e'
                            || next == 'E'
                            || next == '+'
                            || next == '-'
                        {
                            result.push(chars.next().unwrap());
                        } else {
                            break;
                        }
                    }
                    result.push_str(&scheme.reset);
                }
                _ => {
                    // Whitespace and other characters
                    result.push(c);
                }
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;

    /// The jq default spec, spelled out. Parsing this must be a no-op.
    const DEFAULT_SPEC: &str = "1;30:0;39:0;39:0;39:0;32:1;39:1;39:1;34";

    #[test]
    fn test_jq_colors_valid_sgr() {
        assert!(is_valid_sgr("0;31"));
        assert!(is_valid_sgr("1"));
        assert!(is_valid_sgr("0;31;4"));
        // An empty field is valid and selects the empty SGR sequence.
        assert!(is_valid_sgr(""));
        // A trailing separator is accepted, as in jq.
        assert!(is_valid_sgr("0;31;"));

        // Anything that is not a digit or ';' is rejected, so arbitrary text can
        // never be interpolated into the escape sequence.
        assert!(!is_valid_sgr("0;3a"));
        assert!(!is_valid_sgr("0;31m"));
        assert!(!is_valid_sgr("31 "));
        assert!(!is_valid_sgr("-1"));
        assert!(!is_valid_sgr("bogus"));
    }

    #[test]
    fn test_jq_colors_spec_sets_every_field_in_order() {
        let scheme = ColorScheme::from_spec("1:2:3:4:5:6:7:8").expect("spec is valid");
        assert_eq!(scheme.null, "\x1b[1m");
        assert_eq!(scheme.false_, "\x1b[2m");
        assert_eq!(scheme.true_, "\x1b[3m");
        assert_eq!(scheme.number, "\x1b[4m");
        assert_eq!(scheme.string, "\x1b[5m");
        assert_eq!(scheme.array, "\x1b[6m");
        assert_eq!(scheme.object, "\x1b[7m");
        assert_eq!(scheme.key, "\x1b[8m");
        // reset is not settable via JQ_COLORS.
        assert_eq!(scheme.reset, default_colors::RESET);
    }

    #[test]
    fn test_jq_colors_default_spec_round_trips() {
        let scheme = ColorScheme::from_spec(DEFAULT_SPEC).expect("spec is valid");
        assert_eq!(scheme.null, default_colors::NULL);
        assert_eq!(scheme.string, default_colors::STRING);
        assert_eq!(scheme.key, default_colors::KEY);
    }

    #[test]
    fn test_jq_colors_empty_field_selects_empty_sgr() {
        // jq treats an empty field as "\x1b[m", not as "keep the default".
        let scheme = ColorScheme::from_spec("0;31:::::::").expect("spec is valid");
        assert_eq!(scheme.null, "\x1b[0;31m");
        assert_eq!(scheme.false_, "\x1b[m");
        assert_eq!(scheme.key, "\x1b[m");
    }

    #[test]
    fn test_jq_colors_short_spec_keeps_remaining_defaults() {
        let scheme = ColorScheme::from_spec("0;31").expect("spec is valid");
        assert_eq!(scheme.null, "\x1b[0;31m");
        assert_eq!(scheme.false_, default_colors::FALSE);
        assert_eq!(scheme.key, default_colors::KEY);
    }

    #[test]
    fn test_jq_colors_extra_fields_are_ignored_unvalidated() {
        // jq only looks at the first eight fields, so a ninth is dropped even when
        // it would not have validated.
        let scheme =
            ColorScheme::from_spec(&format!("{DEFAULT_SPEC}:bogus")).expect("spec is valid");
        assert_eq!(scheme.null, default_colors::NULL);
        assert_eq!(scheme.key, default_colors::KEY);
    }

    #[test]
    fn test_jq_colors_invalid_field_rejects_whole_spec() {
        // One bad field discards the good ones too, rather than applying them.
        assert!(ColorScheme::from_spec("bogus:0;39:0;39:0;39:0;32:1;39:1;39:9;95").is_none());
        assert!(ColorScheme::from_spec("0;31:bogus").is_none());
        assert!(ColorScheme::from_spec("0;31;4:0;39:0;39:0;39:0;32:1;39:1;39:0;31m").is_none());
    }

    #[test]
    fn test_colorize_json_token_aware() {
        let out = colorize_json(r#"{"a":true}"#, &ColorScheme::default());
        // Object keys are colored as keys, not as string values.
        assert!(out.contains("\x1b[1;34m\"a\""));
        // Keywords are colored once as whole tokens, never letter-by-letter.
        assert!(out.contains("\x1b[0;39mtrue\x1b[0m"));
        assert!(!out.contains("\x1b[0;39mt\x1b[0m"));
    }

    #[test]
    fn test_escape_json_string() {
        assert_eq!(escape_json_string("hello"), "hello");
        assert_eq!(escape_json_string("hello\nworld"), "hello\\nworld");
        assert_eq!(escape_json_string("say \"hi\""), "say \\\"hi\\\"");
    }

    #[test]
    fn test_format_json_sorts_keys() {
        let mut obj = IndexMap::new();
        obj.insert("z".to_string(), OwnedValue::Int(1));
        obj.insert("a".to_string(), OwnedValue::Int(2));
        let value = OwnedValue::Object(obj);

        let opts = JsonFormatOpts {
            indent: "",
            sort_keys: true,
            ascii: false,
            float_style: FloatStyle::Shortest,
        };
        assert_eq!(format_json(&value, &opts), r#"{"a":2,"z":1}"#);
    }

    #[test]
    fn test_format_json_float_styles() {
        let value = OwnedValue::Float(1.0);
        let opts = |float_style| JsonFormatOpts {
            indent: "",
            sort_keys: false,
            ascii: false,
            float_style,
        };
        assert_eq!(format_json(&value, &opts(FloatStyle::Shortest)), "1");
        assert_eq!(
            format_json(&value, &opts(FloatStyle::PreserveWholeFloat)),
            "1.0"
        );
    }
}
