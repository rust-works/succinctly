//! yq-compatible command runner for succinctly.
//!
//! This module implements a yq-compatible CLI interface using the succinctly
//! YAML semi-indexing and jq expression evaluator.

use anyhow::{Context, Result};
use indexmap::IndexMap;
use std::io::{BufWriter, Read, Write};
use std::path::Path;

use succinctly::jq::{self, Expr, OwnedValue, QueryResult};
use succinctly::json::light::StandardJson;
use succinctly::json::JsonIndex;
use succinctly::yaml::{YamlIndex, YamlValue};

use super::{OutputFormat, YqCommand};

/// Exit codes matching jq/yq behavior
pub mod exit_codes {
    pub const SUCCESS: i32 = 0;
    pub const FALSE_OR_NULL: i32 = 1; // With -e, last output was false or null
    #[allow(dead_code)]
    pub const USAGE_ERROR: i32 = 2; // Usage problem or system error
    #[allow(dead_code)]
    pub const COMPILE_ERROR: i32 = 3; // jq program compile error
    pub const NO_OUTPUT: i32 = 4; // With -e, no valid result produced
}

/// Print build configuration information
fn print_build_configuration() {
    println!("succinctly yq build configuration:");
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
    println!();
    println!("Platform:");
    println!("  OS: {}", std::env::consts::OS);
    println!("  Arch: {}", std::env::consts::ARCH);
    println!("  Family: {}", std::env::consts::FAMILY);
}

/// Evaluation context for passing variables to the jq evaluator.
#[derive(Debug, Default)]
pub struct EvalContext {
    /// Named arguments from --arg, --argjson
    pub named: IndexMap<String, OwnedValue>,
}

/// Output configuration
struct OutputConfig {
    output_format: OutputFormat,
    compact: bool,
    raw_output: bool,
    join_output: bool,
    ascii_output: bool,
    sort_keys: bool,
    no_doc: bool,
    indent_str: String,
    use_color: bool,
}

impl OutputConfig {
    fn from_args(args: &YqCommand) -> Self {
        let use_color = if args.monochrome_output {
            false
        } else if args.color_output {
            true
        } else {
            // Default: color if stdout is a terminal
            atty::is(atty::Stream::Stdout)
        };

        let indent_str = if args.compact_output {
            String::new()
        } else if args.tab {
            "\t".to_string()
        } else {
            " ".repeat(args.indent as usize)
        };

        OutputConfig {
            output_format: args.output_format,
            compact: args.compact_output,
            raw_output: args.raw_output || args.join_output,
            join_output: args.join_output,
            ascii_output: args.ascii_output,
            sort_keys: args.sort_keys,
            no_doc: args.no_doc,
            indent_str,
            use_color,
        }
    }
}

/// Convert a YAML value to an OwnedValue for jq evaluation.
fn yaml_to_owned_value<W: AsRef<[u64]>>(value: YamlValue<'_, W>) -> Result<OwnedValue> {
    match value {
        YamlValue::String(s) => {
            let str_value = s
                .as_str()
                .map_err(|e| anyhow::anyhow!("invalid YAML string: {}", e))?;

            // Try to parse as special YAML values
            match str_value.as_ref() {
                "null" | "~" | "" => return Ok(OwnedValue::Null),
                "true" | "True" | "TRUE" => return Ok(OwnedValue::Bool(true)),
                "false" | "False" | "FALSE" => return Ok(OwnedValue::Bool(false)),
                _ => {}
            }

            // Try to parse as number
            if let Ok(n) = str_value.parse::<i64>() {
                return Ok(OwnedValue::Int(n));
            }
            if let Ok(f) = str_value.parse::<f64>() {
                // Check for special float values
                if !f.is_nan() {
                    return Ok(OwnedValue::Float(f));
                }
            }

            // Check for special float literals
            match str_value.as_ref() {
                ".inf" | ".Inf" | ".INF" => return Ok(OwnedValue::Float(f64::INFINITY)),
                "-.inf" | "-.Inf" | "-.INF" => return Ok(OwnedValue::Float(f64::NEG_INFINITY)),
                ".nan" | ".NaN" | ".NAN" => return Ok(OwnedValue::Float(f64::NAN)),
                _ => {}
            }

            // It's a string
            Ok(OwnedValue::String(str_value.into_owned()))
        }
        YamlValue::Mapping(fields) => {
            let mut map = IndexMap::new();
            for field in fields {
                let key = match field.key() {
                    YamlValue::String(s) => s
                        .as_str()
                        .map_err(|e| anyhow::anyhow!("invalid YAML key: {}", e))?
                        .into_owned(),
                    other => {
                        // Non-string keys - convert to string representation
                        let value = yaml_to_owned_value(other)?;
                        value_to_string(&value)
                    }
                };
                let value = yaml_to_owned_value(field.value())?;
                map.insert(key, value);
            }
            Ok(OwnedValue::Object(map))
        }
        YamlValue::Sequence(elements) => {
            let mut arr = Vec::new();
            for elem in elements {
                arr.push(yaml_to_owned_value(elem)?);
            }
            Ok(OwnedValue::Array(arr))
        }
        YamlValue::Alias { target, .. } => {
            // Resolve alias by following the target cursor
            if let Some(target_cursor) = target {
                yaml_to_owned_value(target_cursor.value())
            } else {
                // Unresolved alias - treat as null
                Ok(OwnedValue::Null)
            }
        }
        YamlValue::Error(msg) => Err(anyhow::anyhow!("YAML error: {}", msg)),
        YamlValue::Null => Ok(OwnedValue::Null),
    }
}

/// Convert an OwnedValue to a string representation (for non-string keys).
fn value_to_string(value: &OwnedValue) -> String {
    match value {
        OwnedValue::Null => "null".to_string(),
        OwnedValue::Bool(b) => b.to_string(),
        OwnedValue::Int(n) => n.to_string(),
        OwnedValue::Float(f) => {
            if f.is_nan() {
                ".nan".to_string()
            } else if f.is_infinite() {
                if *f > 0.0 {
                    ".inf".to_string()
                } else {
                    "-.inf".to_string()
                }
            } else {
                f.to_string()
            }
        }
        OwnedValue::String(s) => s.clone(),
        OwnedValue::Array(_) | OwnedValue::Object(_) => {
            // Complex types - use JSON representation
            value.to_json()
        }
    }
}

/// Read YAML input from stdin.
fn read_stdin() -> Result<Vec<u8>> {
    let mut buffer = Vec::new();
    std::io::stdin()
        .read_to_end(&mut buffer)
        .context("failed to read from stdin")?;
    Ok(buffer)
}

/// Read YAML input from a file.
fn read_file(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).with_context(|| format!("failed to read file: {}", path.display()))
}

/// Evaluate a jq expression on an OwnedValue by converting to JSON and back.
fn evaluate_input(
    input: &OwnedValue,
    expr: &jq::Expr,
    _context: &EvalContext,
) -> Result<Vec<OwnedValue>> {
    // Convert OwnedValue to JSON bytes for indexing
    let json_str = input.to_json();
    let json_bytes = json_str.as_bytes();

    // Build index and evaluate
    let index = JsonIndex::build(json_bytes);
    let cursor = index.root(json_bytes);

    let result = jq::eval(expr, cursor);

    // Convert result to Vec<OwnedValue>
    match result {
        QueryResult::One(v) => Ok(vec![standard_json_to_owned(&v)]),
        QueryResult::OneCursor(c) => Ok(vec![standard_json_to_owned(&c.value())]),
        QueryResult::Many(vs) => Ok(vs.iter().map(standard_json_to_owned).collect()),
        QueryResult::None => Ok(vec![]),
        QueryResult::Error(e) => {
            eprintln!("yq: error: {}", e);
            Ok(vec![])
        }
        QueryResult::Owned(v) => Ok(vec![v]),
        QueryResult::ManyOwned(vs) => Ok(vs),
    }
}

/// Convert a StandardJson value to an OwnedValue.
fn standard_json_to_owned<W: Clone + AsRef<[u64]>>(value: &StandardJson<'_, W>) -> OwnedValue {
    match value {
        StandardJson::Null => OwnedValue::Null,
        StandardJson::Bool(b) => OwnedValue::Bool(*b),
        StandardJson::Number(n) => {
            if let Ok(i) = n.as_i64() {
                OwnedValue::Int(i)
            } else if let Ok(f) = n.as_f64() {
                OwnedValue::Float(f)
            } else {
                OwnedValue::Null
            }
        }
        StandardJson::String(s) => {
            OwnedValue::String(s.as_str().map(|c| c.to_string()).unwrap_or_default())
        }
        StandardJson::Array(elements) => {
            OwnedValue::Array((*elements).map(|e| standard_json_to_owned(&e)).collect())
        }
        StandardJson::Object(fields) => OwnedValue::Object(
            (*fields)
                .filter_map(|field| {
                    let key = match field.key() {
                        StandardJson::String(s) => s.as_str().ok()?.to_string(),
                        _ => return None,
                    };
                    Some((key, standard_json_to_owned(&field.value())))
                })
                .collect(),
        ),
        StandardJson::Error(_) => OwnedValue::Null,
    }
}

/// Format and output a value.
fn output_value<W: Write>(writer: &mut W, value: &OwnedValue, config: &OutputConfig) -> Result<()> {
    // Handle raw output for scalars
    if config.raw_output {
        if let OwnedValue::String(s) = value {
            write!(writer, "{}", s)?;
            if !config.join_output {
                writeln!(writer)?;
            }
            return Ok(());
        }
    }

    // For YAML output format (default)
    if config.output_format == OutputFormat::Yaml {
        // For YAML, scalars are printed without quotes by default (like -r in yq)
        let output = emit_yaml_value(value, config, 0, false);
        if config.use_color {
            write!(writer, "{}", colorize_yaml(&output))?;
        } else {
            write!(writer, "{}", output)?;
        }
        if !config.join_output {
            writeln!(writer)?;
        }
        return Ok(());
    }

    // JSON output format
    let json_str = if config.compact {
        value.to_json()
    } else {
        // Custom pretty-printing with configurable indent
        pretty_print_json_value(value, config, 0)
    };

    if config.use_color {
        write!(writer, "{}", colorize_json(&json_str))?;
    } else {
        write!(writer, "{}", json_str)?;
    }

    if !config.join_output {
        writeln!(writer)?;
    }

    Ok(())
}

/// Emit a YAML value as a string.
fn emit_yaml_value(
    value: &OwnedValue,
    config: &OutputConfig,
    depth: usize,
    in_flow: bool,
) -> String {
    match value {
        OwnedValue::Null => "null".to_string(),
        OwnedValue::Bool(b) => b.to_string(),
        OwnedValue::Int(n) => n.to_string(),
        OwnedValue::Float(f) => {
            if f.is_nan() {
                ".nan".to_string()
            } else if f.is_infinite() {
                if *f > 0.0 {
                    ".inf".to_string()
                } else {
                    "-.inf".to_string()
                }
            } else if f.fract() == 0.0 && *f >= i64::MIN as f64 && *f <= i64::MAX as f64 {
                format!("{:.1}", f)
            } else {
                f.to_string()
            }
        }
        OwnedValue::String(s) => yaml_quote_string(s),
        OwnedValue::Array(arr) => {
            if arr.is_empty() {
                "[]".to_string()
            } else if in_flow {
                // Flow style for nested in flow context
                let items: Vec<_> = arr
                    .iter()
                    .map(|v| emit_yaml_value(v, config, depth, true))
                    .collect();
                format!("[{}]", items.join(", "))
            } else {
                // Block style sequence
                let indent = config.indent_str.repeat(depth);
                let items: Vec<_> = arr
                    .iter()
                    .map(|v| {
                        let item = emit_yaml_value(v, config, depth + 1, false);
                        // Check if it's a multi-line value (mapping or sequence)
                        if matches!(v, OwnedValue::Object(_) | OwnedValue::Array(_))
                            && !item.starts_with('[')
                            && !item.starts_with('{')
                        {
                            // Multi-line value - emit nested content which handles its own indentation
                            format!("{}-\n{}", indent, item)
                        } else {
                            format!("{}- {}", indent, item)
                        }
                    })
                    .collect();
                items.join("\n")
            }
        }
        OwnedValue::Object(obj) => {
            if obj.is_empty() {
                "{}".to_string()
            } else if in_flow {
                // Flow style for nested in flow context
                let entries: Vec<_> = obj
                    .iter()
                    .map(|(k, v)| {
                        let key = yaml_quote_key(k);
                        let val = emit_yaml_value(v, config, depth, true);
                        format!("{}: {}", key, val)
                    })
                    .collect();
                format!("{{{}}}", entries.join(", "))
            } else {
                // Block style mapping
                let indent = config.indent_str.repeat(depth);
                let entries: Vec<_> = if config.sort_keys {
                    let mut sorted: Vec<_> = obj.iter().collect();
                    sorted.sort_by(|a, b| a.0.cmp(b.0));
                    sorted
                } else {
                    obj.iter().collect()
                };

                let items: Vec<_> = entries
                    .iter()
                    .map(|(k, v)| {
                        let key = yaml_quote_key(k);
                        // Check if value needs to be on next line
                        if matches!(v, OwnedValue::Object(m) if !m.is_empty())
                            || matches!(v, OwnedValue::Array(a) if !a.is_empty())
                        {
                            // For nested containers, emit at depth+1 which handles its own indentation
                            let val = emit_yaml_value(v, config, depth + 1, false);
                            format!("{}{}:\n{}", indent, key, val)
                        } else {
                            let val = emit_yaml_value(v, config, depth + 1, false);
                            format!("{}{}: {}", indent, key, val)
                        }
                    })
                    .collect();
                items.join("\n")
            }
        }
    }
}

/// Quote a YAML string if needed.
fn yaml_quote_string(s: &str) -> String {
    // Check if string needs quoting
    if s.is_empty() {
        return "''".to_string();
    }

    // Check for special YAML values that need quoting
    let lower = s.to_lowercase();
    let needs_quoting = lower == "null"
        || lower == "true"
        || lower == "false"
        || lower == "~"
        || lower == ".nan"
        || lower == ".inf"
        || lower == "-.inf"
        || s.parse::<f64>().is_ok()
        || s.starts_with('*')
        || s.starts_with('&')
        || s.starts_with('!')
        || s.starts_with('%')
        || s.starts_with('@')
        || s.starts_with('`')
        || s.starts_with('|')
        || s.starts_with('>')
        || s.starts_with('[')
        || s.starts_with('{')
        || s.starts_with('"')
        || s.starts_with('\'')
        || s.starts_with('#')
        || s.starts_with('-') && (s.len() == 1 || s.chars().nth(1) == Some(' '))
        || s.starts_with('?') && (s.len() == 1 || s.chars().nth(1) == Some(' '))
        || s.starts_with(':') && (s.len() == 1 || s.chars().nth(1) == Some(' '))
        || s.contains(": ")
        || s.contains(" #")
        || s.contains('\n')
        || s.contains('\r')
        || s.contains('\t')
        || s.ends_with(':')
        || s.ends_with(' ');

    if needs_quoting {
        // Use double quotes with escaping
        let mut result = String::with_capacity(s.len() + 2);
        result.push('"');
        for c in s.chars() {
            match c {
                '"' => result.push_str("\\\""),
                '\\' => result.push_str("\\\\"),
                '\n' => result.push_str("\\n"),
                '\r' => result.push_str("\\r"),
                '\t' => result.push_str("\\t"),
                c if c.is_ascii_control() => {
                    result.push_str(&format!("\\x{:02x}", c as u32));
                }
                _ => result.push(c),
            }
        }
        result.push('"');
        result
    } else {
        s.to_string()
    }
}

/// Quote a YAML key if needed.
fn yaml_quote_key(s: &str) -> String {
    // Keys have similar rules but are a bit more permissive
    if s.is_empty() {
        return "''".to_string();
    }

    let needs_quoting = s.contains(':')
        || s.contains('#')
        || s.contains('\n')
        || s.contains('\r')
        || s.starts_with('-')
        || s.starts_with('?')
        || s.starts_with('[')
        || s.starts_with('{')
        || s.starts_with('"')
        || s.starts_with('\'')
        || s.starts_with('*')
        || s.starts_with('&')
        || s.starts_with('!')
        || s.ends_with(' ');

    if needs_quoting {
        let mut result = String::with_capacity(s.len() + 2);
        result.push('"');
        for c in s.chars() {
            match c {
                '"' => result.push_str("\\\""),
                '\\' => result.push_str("\\\\"),
                '\n' => result.push_str("\\n"),
                '\r' => result.push_str("\\r"),
                '\t' => result.push_str("\\t"),
                _ => result.push(c),
            }
        }
        result.push('"');
        result
    } else {
        s.to_string()
    }
}

/// Colorize YAML output (basic ANSI colors).
fn colorize_yaml(yaml: &str) -> String {
    let mut result = String::with_capacity(yaml.len() * 2);
    let mut in_string = false;
    let mut escape_next = false;
    let mut at_key_start = true;
    let mut in_key = false;

    for c in yaml.chars() {
        if escape_next {
            result.push(c);
            escape_next = false;
            continue;
        }

        if c == '\\' && (in_string || in_key) {
            result.push(c);
            escape_next = true;
            continue;
        }

        match c {
            '"' | '\'' => {
                if in_string {
                    result.push(c);
                    result.push_str("\x1b[0m");
                    in_string = false;
                } else {
                    result.push_str("\x1b[32m"); // Green for strings
                    result.push(c);
                    in_string = true;
                }
                at_key_start = false;
            }
            ':' if !in_string => {
                result.push_str("\x1b[0m");
                result.push(c);
                in_key = false;
                at_key_start = false;
            }
            '\n' => {
                result.push(c);
                at_key_start = true;
                in_key = false;
            }
            '-' if at_key_start => {
                result.push_str("\x1b[33m"); // Yellow for list markers
                result.push(c);
                result.push_str("\x1b[0m");
                at_key_start = false;
            }
            _ if at_key_start && !c.is_whitespace() && !in_string => {
                result.push_str("\x1b[36m"); // Cyan for keys
                result.push(c);
                in_key = true;
                at_key_start = false;
            }
            _ => {
                result.push(c);
                if !c.is_whitespace() {
                    at_key_start = false;
                }
            }
        }
    }
    result.push_str("\x1b[0m");
    result
}

/// Pretty print a JSON value with configurable indentation.
fn pretty_print_json_value(value: &OwnedValue, config: &OutputConfig, depth: usize) -> String {
    match value {
        OwnedValue::Null => "null".to_string(),
        OwnedValue::Bool(b) => b.to_string(),
        OwnedValue::Int(n) => n.to_string(),
        OwnedValue::Float(f) => {
            if f.is_nan() || f.is_infinite() {
                "null".to_string() // JSON doesn't support NaN or Infinity
            } else if f.fract() == 0.0 && *f >= i64::MIN as f64 && *f <= i64::MAX as f64 {
                format!("{:.1}", f) // Preserve decimal point for whole numbers
            } else {
                f.to_string()
            }
        }
        OwnedValue::String(s) => {
            if config.ascii_output {
                escape_string_ascii(s)
            } else {
                escape_json_string(s)
            }
        }
        OwnedValue::Array(arr) => {
            if arr.is_empty() {
                "[]".to_string()
            } else if config.compact || config.indent_str.is_empty() {
                let items: Vec<_> = arr
                    .iter()
                    .map(|v| pretty_print_json_value(v, config, depth + 1))
                    .collect();
                format!("[{}]", items.join(","))
            } else {
                let indent = config.indent_str.repeat(depth + 1);
                let close_indent = config.indent_str.repeat(depth);
                let items: Vec<_> = arr
                    .iter()
                    .map(|v| {
                        format!(
                            "{}{}",
                            indent,
                            pretty_print_json_value(v, config, depth + 1)
                        )
                    })
                    .collect();
                format!("[\n{}\n{}]", items.join(",\n"), close_indent)
            }
        }
        OwnedValue::Object(obj) => {
            if obj.is_empty() {
                "{}".to_string()
            } else {
                let entries: Vec<_> = if config.sort_keys {
                    let mut sorted: Vec<_> = obj.iter().collect();
                    sorted.sort_by(|a, b| a.0.cmp(b.0));
                    sorted
                } else {
                    obj.iter().collect()
                };

                if config.compact || config.indent_str.is_empty() {
                    let items: Vec<_> = entries
                        .iter()
                        .map(|(k, v)| {
                            let key = escape_json_string(k);
                            format!("{}:{}", key, pretty_print_json_value(v, config, depth + 1))
                        })
                        .collect();
                    format!("{{{}}}", items.join(","))
                } else {
                    let indent = config.indent_str.repeat(depth + 1);
                    let close_indent = config.indent_str.repeat(depth);
                    let items: Vec<_> = entries
                        .iter()
                        .map(|(k, v)| {
                            let key = escape_json_string(k);
                            format!(
                                "{}{}: {}",
                                indent,
                                key,
                                pretty_print_json_value(v, config, depth + 1)
                            )
                        })
                        .collect();
                    format!("{{\n{}\n{}}}", items.join(",\n"), close_indent)
                }
            }
        }
    }
}

/// Escape a string for JSON output.
fn escape_json_string(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 2);
    result.push('"');
    for c in s.chars() {
        match c {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            c if c.is_ascii_control() => {
                result.push_str(&format!("\\u{:04x}", c as u32));
            }
            _ => result.push(c),
        }
    }
    result.push('"');
    result
}

/// Escape a string with ASCII-only output.
fn escape_string_ascii(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 2);
    result.push('"');
    for c in s.chars() {
        if c.is_ascii() {
            match c {
                '"' => result.push_str("\\\""),
                '\\' => result.push_str("\\\\"),
                '\n' => result.push_str("\\n"),
                '\r' => result.push_str("\\r"),
                '\t' => result.push_str("\\t"),
                c if c.is_ascii_control() => {
                    result.push_str(&format!("\\u{:04x}", c as u32));
                }
                _ => result.push(c),
            }
        } else {
            // Non-ASCII: escape as \uXXXX
            for unit in c.encode_utf16(&mut [0; 2]) {
                result.push_str(&format!("\\u{:04x}", unit));
            }
        }
    }
    result.push('"');
    result
}

/// Colorize JSON output (basic ANSI colors).
fn colorize_json(json: &str) -> String {
    // Simple colorization - this could be more sophisticated
    let mut result = String::with_capacity(json.len() * 2);
    let mut in_string = false;
    let mut escape_next = false;

    for c in json.chars() {
        if escape_next {
            result.push(c);
            escape_next = false;
            continue;
        }

        if c == '\\' && in_string {
            result.push(c);
            escape_next = true;
            continue;
        }

        if c == '"' {
            if in_string {
                result.push(c);
                result.push_str("\x1b[0m"); // Reset
                in_string = false;
            } else {
                result.push_str("\x1b[32m"); // Green for strings
                result.push(c);
                in_string = true;
            }
            continue;
        }

        if !in_string {
            match c {
                't' | 'r' | 'u' | 'e' | 'f' | 'a' | 'l' | 's' => {
                    // Part of true/false - blue
                    result.push_str("\x1b[34m");
                    result.push(c);
                    result.push_str("\x1b[0m");
                }
                'n' => {
                    // Part of null - magenta
                    result.push_str("\x1b[35m");
                    result.push(c);
                    result.push_str("\x1b[0m");
                }
                '0'..='9' | '-' | '.' => {
                    // Numbers - cyan
                    result.push_str("\x1b[36m");
                    result.push(c);
                    result.push_str("\x1b[0m");
                }
                _ => result.push(c),
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Parse a JSON string into an OwnedValue.
fn parse_json_value(s: &str) -> Result<OwnedValue> {
    let bytes = s.as_bytes();
    let index = JsonIndex::build(bytes);
    let cursor = index.root(bytes);
    Ok(standard_json_to_owned(&cursor.value()))
}

/// Parse variables from command line arguments.
fn parse_variables(args: &YqCommand) -> Result<EvalContext> {
    let mut context = EvalContext::default();

    // Process --arg (string values)
    for chunk in args.arg.chunks(2) {
        if chunk.len() == 2 {
            let name = chunk[0].clone();
            let value = OwnedValue::String(chunk[1].clone());
            context.named.insert(name, value);
        }
    }

    // Process --argjson (JSON values)
    for chunk in args.argjson.chunks(2) {
        if chunk.len() == 2 {
            let name = chunk[0].clone();
            let value = parse_json_value(&chunk[1])
                .with_context(|| format!("invalid JSON for --argjson {}", name))?;
            context.named.insert(name, value);
        }
    }

    Ok(context)
}

/// Main entry point for the yq command.
pub fn run_yq(args: YqCommand) -> Result<i32> {
    // Handle --version
    if args.version {
        println!("succinctly-yq {}", env!("CARGO_PKG_VERSION"));
        return Ok(exit_codes::SUCCESS);
    }

    // Handle --build-configuration
    if args.build_configuration {
        print_build_configuration();
        return Ok(exit_codes::SUCCESS);
    }

    // Get the filter expression
    let filter_str = if let Some(ref path) = args.from_file {
        std::fs::read_to_string(path)
            .with_context(|| format!("failed to read filter file: {}", path.display()))?
    } else {
        args.filter.clone().unwrap_or_else(|| ".".to_string())
    };

    // Parse the jq program
    let program =
        jq::parse_program(&filter_str).map_err(|e| anyhow::anyhow!("parse error: {}", e))?;

    // Parse variables
    let context = parse_variables(&args)?;

    // Output configuration
    let output_config = OutputConfig::from_args(&args);

    // Set up output
    let stdout = std::io::stdout();
    let mut writer = BufWriter::new(stdout.lock());

    // Track last output for exit status
    let mut last_output: Option<OwnedValue> = None;
    let mut had_output = false;

    // Fast path: identity filter with JSON compact output - stream directly from YAML cursor
    // This avoids building OwnedValue DOM and JSON round-trip
    let is_identity = matches!(program.expr, Expr::Identity);
    let can_fast_path = is_identity
        && output_config.compact
        && output_config.output_format == OutputFormat::Json
        && !args.null_input
        && !args.slurp
        && context.named.is_empty();

    if can_fast_path {
        // Direct YAML → JSON streaming
        if args.files.is_empty() {
            let yaml_bytes = read_stdin()?;
            let index = YamlIndex::build(&yaml_bytes)
                .map_err(|e| anyhow::anyhow!("YAML parse error: {}", e))?;
            let root = index.root(&yaml_bytes);

            // Output each document directly using cursor iteration
            match root.value() {
                YamlValue::Sequence(mut docs) => {
                    while let Some((cursor, rest)) = docs.uncons_cursor() {
                        had_output = true;
                        let json = cursor.to_json();
                        writeln!(writer, "{}", json)?;
                        docs = rest;
                    }
                }
                _ => {
                    had_output = true;
                    let json = root.to_json_document();
                    writeln!(writer, "{}", json)?;
                }
            }
        } else {
            for file_path in &args.files {
                let yaml_bytes = read_file(Path::new(file_path))?;
                let index = YamlIndex::build(&yaml_bytes)
                    .map_err(|e| anyhow::anyhow!("YAML parse error in {}: {}", file_path, e))?;
                let root = index.root(&yaml_bytes);

                match root.value() {
                    YamlValue::Sequence(mut docs) => {
                        while let Some((cursor, rest)) = docs.uncons_cursor() {
                            had_output = true;
                            let json = cursor.to_json();
                            writeln!(writer, "{}", json)?;
                            docs = rest;
                        }
                    }
                    _ => {
                        had_output = true;
                        let json = root.to_json_document();
                        writeln!(writer, "{}", json)?;
                    }
                }
            }
        }
    } else if args.null_input {
        // Handle --null-input
        let results = evaluate_input(&OwnedValue::Null, &program.expr, &context)?;
        for result in results {
            last_output = Some(result.clone());
            had_output = true;
            output_value(&mut writer, &result, &output_config)?;
        }
    } else {
        // Standard path: collect inputs via OwnedValue
        let inputs: Vec<OwnedValue> = if args.files.is_empty() {
            // Read from stdin
            let yaml_bytes = read_stdin()?;
            let index = YamlIndex::build(&yaml_bytes)
                .map_err(|e| anyhow::anyhow!("YAML parse error: {}", e))?;
            let root = index.root(&yaml_bytes);

            // Get the first document from the document array
            match root.value() {
                YamlValue::Sequence(docs) => {
                    let mut values = Vec::new();
                    for doc in docs {
                        values.push(yaml_to_owned_value(doc)?);
                    }
                    values
                }
                other => vec![yaml_to_owned_value(other)?],
            }
        } else {
            // Read from files
            let mut values = Vec::new();
            for file_path in &args.files {
                let yaml_bytes = read_file(Path::new(file_path))?;
                let index = YamlIndex::build(&yaml_bytes)
                    .map_err(|e| anyhow::anyhow!("YAML parse error in {}: {}", file_path, e))?;
                let root = index.root(&yaml_bytes);

                // Get the first document from the document array
                match root.value() {
                    YamlValue::Sequence(docs) => {
                        for doc in docs {
                            values.push(yaml_to_owned_value(doc)?);
                        }
                    }
                    other => values.push(yaml_to_owned_value(other)?),
                }
            }
            values
        };

        // Handle --slurp
        if args.slurp {
            let slurped = OwnedValue::Array(inputs);
            let results = evaluate_input(&slurped, &program.expr, &context)?;
            for result in results {
                last_output = Some(result.clone());
                had_output = true;
                output_value(&mut writer, &result, &output_config)?;
            }
        } else {
            // Process each input document
            let is_multi_doc = inputs.len() > 1;
            for input in inputs {
                // Add document separator in YAML mode for multi-doc
                // yq adds --- before each document (including the first one) in multi-doc output
                if output_config.output_format == OutputFormat::Yaml
                    && !output_config.no_doc
                    && is_multi_doc
                {
                    writeln!(writer, "---")?;
                }

                let results = evaluate_input(&input, &program.expr, &context)?;
                for result in results {
                    last_output = Some(result.clone());
                    had_output = true;
                    output_value(&mut writer, &result, &output_config)?;
                }
            }
        }
    }

    writer.flush()?;

    // Determine exit code
    if args.exit_status {
        if !had_output {
            return Ok(exit_codes::NO_OUTPUT);
        }
        if let Some(OwnedValue::Null | OwnedValue::Bool(false)) = last_output {
            return Ok(exit_codes::FALSE_OR_NULL);
        }
    }

    Ok(exit_codes::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_yaml_to_owned_value_string() {
        let yaml = b"name: Alice";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        // Root is a document array, get first doc
        if let YamlValue::Sequence(docs) = root.value() {
            if let Some(doc) = docs.into_iter().next() {
                let value = yaml_to_owned_value(doc).unwrap();
                if let OwnedValue::Object(map) = value {
                    assert_eq!(
                        map.get("name"),
                        Some(&OwnedValue::String("Alice".to_string()))
                    );
                } else {
                    panic!("expected object");
                }
            }
        }
    }

    #[test]
    fn test_yaml_to_owned_value_number() {
        let yaml = b"age: 30";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Sequence(docs) = root.value() {
            if let Some(doc) = docs.into_iter().next() {
                let value = yaml_to_owned_value(doc).unwrap();
                if let OwnedValue::Object(map) = value {
                    assert_eq!(map.get("age"), Some(&OwnedValue::Int(30)));
                } else {
                    panic!("expected object");
                }
            }
        }
    }

    #[test]
    fn test_yaml_to_owned_value_bool() {
        let yaml = b"active: true";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Sequence(docs) = root.value() {
            if let Some(doc) = docs.into_iter().next() {
                let value = yaml_to_owned_value(doc).unwrap();
                if let OwnedValue::Object(map) = value {
                    assert_eq!(map.get("active"), Some(&OwnedValue::Bool(true)));
                } else {
                    panic!("expected object");
                }
            }
        }
    }

    #[test]
    fn test_yaml_to_owned_value_null() {
        let yaml = b"value: null";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Sequence(docs) = root.value() {
            if let Some(doc) = docs.into_iter().next() {
                let value = yaml_to_owned_value(doc).unwrap();
                if let OwnedValue::Object(map) = value {
                    assert_eq!(map.get("value"), Some(&OwnedValue::Null));
                } else {
                    panic!("expected object");
                }
            }
        }
    }

    #[test]
    fn test_yaml_to_owned_value_flow_sequence() {
        // Flow-style sequence
        let yaml = b"items: [one, two, three]";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Sequence(docs) = root.value() {
            if let Some(doc) = docs.into_iter().next() {
                let value = yaml_to_owned_value(doc).unwrap();
                if let OwnedValue::Object(map) = value {
                    if let Some(OwnedValue::Array(arr)) = map.get("items") {
                        assert_eq!(arr.len(), 3);
                        assert_eq!(arr[0], OwnedValue::String("one".to_string()));
                        assert_eq!(arr[1], OwnedValue::String("two".to_string()));
                        assert_eq!(arr[2], OwnedValue::String("three".to_string()));
                    } else {
                        panic!("expected array for items, got {:?}", map.get("items"));
                    }
                } else {
                    panic!("expected object");
                }
            }
        }
    }

    #[test]
    fn test_yaml_to_owned_value_flow_nested() {
        // Flow-style nested mapping
        let yaml = b"person: {name: Alice, age: 30}";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Sequence(docs) = root.value() {
            if let Some(doc) = docs.into_iter().next() {
                let value = yaml_to_owned_value(doc).unwrap();
                if let OwnedValue::Object(map) = value {
                    if let Some(OwnedValue::Object(person)) = map.get("person") {
                        assert_eq!(
                            person.get("name"),
                            Some(&OwnedValue::String("Alice".to_string()))
                        );
                        assert_eq!(person.get("age"), Some(&OwnedValue::Int(30)));
                    } else {
                        panic!("expected object for person");
                    }
                } else {
                    panic!("expected object");
                }
            }
        }
    }

    #[test]
    fn test_yaml_to_owned_value_block_sequence() {
        // Block-style nested sequence (value on next line)
        let yaml = b"items:\n  - one\n  - two\n  - three";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Sequence(docs) = root.value() {
            if let Some(doc) = docs.into_iter().next() {
                let value = yaml_to_owned_value(doc).unwrap();
                if let OwnedValue::Object(map) = value {
                    if let Some(OwnedValue::Array(arr)) = map.get("items") {
                        assert_eq!(arr.len(), 3);
                        assert_eq!(arr[0], OwnedValue::String("one".to_string()));
                        assert_eq!(arr[1], OwnedValue::String("two".to_string()));
                        assert_eq!(arr[2], OwnedValue::String("three".to_string()));
                    } else {
                        panic!("expected array for items, got {:?}", map.get("items"));
                    }
                } else {
                    panic!("expected object");
                }
            }
        }
    }

    #[test]
    fn test_yaml_to_owned_value_block_nested_mapping() {
        // Block-style nested mapping (value on next line)
        let yaml = b"person:\n  name: Alice\n  age: 30";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Sequence(docs) = root.value() {
            if let Some(doc) = docs.into_iter().next() {
                let value = yaml_to_owned_value(doc).unwrap();
                if let OwnedValue::Object(map) = value {
                    if let Some(OwnedValue::Object(person)) = map.get("person") {
                        assert_eq!(
                            person.get("name"),
                            Some(&OwnedValue::String("Alice".to_string()))
                        );
                        assert_eq!(person.get("age"), Some(&OwnedValue::Int(30)));
                    } else {
                        panic!("expected object for person, got {:?}", map.get("person"));
                    }
                } else {
                    panic!("expected object");
                }
            }
        }
    }

    #[test]
    fn test_yaml_to_owned_value_deeply_nested() {
        // Deeply nested block-style structure
        let yaml = b"root:\n  level1:\n    level2:\n      value: deep";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Sequence(docs) = root.value() {
            if let Some(doc) = docs.into_iter().next() {
                let value = yaml_to_owned_value(doc).unwrap();
                if let OwnedValue::Object(map) = value {
                    if let Some(OwnedValue::Object(level1)) = map.get("root") {
                        if let Some(OwnedValue::Object(level2)) = level1.get("level1") {
                            if let Some(OwnedValue::Object(level3)) = level2.get("level2") {
                                assert_eq!(
                                    level3.get("value"),
                                    Some(&OwnedValue::String("deep".to_string()))
                                );
                            } else {
                                panic!("expected object for level2");
                            }
                        } else {
                            panic!("expected object for level1");
                        }
                    } else {
                        panic!("expected object for root");
                    }
                } else {
                    panic!("expected object");
                }
            }
        }
    }
}
