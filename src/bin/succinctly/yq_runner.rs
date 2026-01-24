//! yq-compatible command runner for succinctly.
//!
//! This module implements a yq-compatible CLI interface using the succinctly
//! YAML semi-indexing and jq expression evaluator.

use anyhow::{Context, Result};
use indexmap::IndexMap;
use std::io::{BufWriter, Read, Write};
use std::path::Path;

use succinctly::jq::eval_generic::{eval_with_cursor, to_owned, GenericResult};
use succinctly::jq::{self, Builtin, Expr, OwnedValue, QueryResult};
use succinctly::json::light::StandardJson;
use succinctly::json::JsonIndex;
use succinctly::yaml::{YamlCursor, YamlIndex, YamlValue};

use super::{InputFormat, OutputFormat, YqCommand};

/// Adapter to use `std::io::Write` with `core::fmt::Write` methods.
/// This enables streaming JSON output without intermediate String allocation.
struct FmtWriter<W>(W);

impl<W: Write> core::fmt::Write for FmtWriter<W> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.0.write_all(s.as_bytes()).map_err(|_| core::fmt::Error)
    }
}

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
#[derive(Clone)]
struct OutputConfig {
    output_format: OutputFormat,
    compact: bool,
    raw_output: bool,
    join_output: bool,
    nul_output: bool,
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

        // Compact output when indent is 0 (yq-compatible)
        let compact = args.indent == 0;

        let indent_str = if compact {
            String::new()
        } else if args.tab {
            "\t".to_string()
        } else {
            " ".repeat(args.indent as usize)
        };

        OutputConfig {
            output_format: args.output_format,
            compact,
            raw_output: args.raw_output || args.join_output || args.nul_output,
            join_output: args.join_output,
            nul_output: args.nul_output,
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

            // Quoted strings should always be treated as strings (yq-compatible behavior)
            // Only unquoted scalars should undergo type detection
            if !s.is_unquoted() {
                return Ok(OwnedValue::String(str_value.into_owned()));
            }

            // Type detection for unquoted scalars only
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

/// Read input from stdin as bytes.
fn read_stdin() -> Result<Vec<u8>> {
    let mut buffer = Vec::new();
    std::io::stdin()
        .read_to_end(&mut buffer)
        .context("failed to read from stdin")?;
    Ok(buffer)
}

/// Read input from stdin as a string.
fn read_stdin_string() -> Result<String> {
    let mut buffer = String::new();
    std::io::stdin()
        .read_to_string(&mut buffer)
        .context("failed to read from stdin")?;
    Ok(buffer)
}

/// Read input from a file.
fn read_file(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).with_context(|| format!("failed to read file: {}", path.display()))
}

/// Detect input format from file extension.
fn detect_format_from_path(path: &Path) -> InputFormat {
    match path.extension().and_then(|e| e.to_str()) {
        Some("json") => InputFormat::Json,
        Some("yaml") | Some("yml") => InputFormat::Yaml,
        _ => InputFormat::Yaml, // Default to YAML
    }
}

/// Get effective input format, resolving Auto to a specific format.
fn resolve_input_format(format: InputFormat, path: Option<&Path>) -> InputFormat {
    match format {
        InputFormat::Auto => path
            .map(detect_format_from_path)
            .unwrap_or(InputFormat::Yaml),
        other => other,
    }
}

/// Parse input bytes according to the specified format.
fn parse_input(bytes: &[u8], format: InputFormat) -> Result<Vec<OwnedValue>> {
    match format {
        InputFormat::Json => {
            // Parse as JSON
            let index = JsonIndex::build(bytes);
            let cursor = index.root(bytes);
            Ok(vec![standard_json_to_owned(&cursor.value())])
        }
        InputFormat::Yaml | InputFormat::Auto => {
            // Parse as YAML (Auto defaults to YAML when no extension hint)
            let index =
                YamlIndex::build(bytes).map_err(|e| anyhow::anyhow!("YAML parse error: {}", e))?;
            let root = index.root(bytes);

            match root.value() {
                YamlValue::Sequence(docs) => {
                    let mut values = Vec::new();
                    for doc in docs {
                        values.push(yaml_to_owned_value(doc)?);
                    }
                    Ok(values)
                }
                other => Ok(vec![yaml_to_owned_value(other)?]),
            }
        }
    }
}

/// Parse and evaluate YAML bytes directly using the generic evaluator.
///
/// This keeps the index alive during evaluation and preserves position metadata.
#[allow(dead_code)] // Used in tests
fn parse_and_evaluate_yaml(bytes: &[u8], expr: &Expr) -> Result<Vec<OwnedValue>> {
    let index = YamlIndex::build(bytes).map_err(|e| anyhow::anyhow!("YAML parse error: {}", e))?;
    let root = index.root(bytes);

    // YAML documents are wrapped in a sequence at the root
    match root.value() {
        YamlValue::Sequence(mut docs) => {
            let mut all_results = Vec::new();
            while let Some((cursor, rest)) = docs.uncons_cursor() {
                let results = evaluate_yaml_cursor(cursor, expr)?;
                all_results.extend(results);
                docs = rest;
            }
            Ok(all_results)
        }
        _ => {
            // Single document - navigate to actual content
            if let Some(content_cursor) = root.first_child() {
                evaluate_yaml_cursor(content_cursor, expr)
            } else {
                // Empty document
                Ok(vec![])
            }
        }
    }
}

/// Evaluate YAML input directly using the generic evaluator with per-document processing.
///
/// This processes YAML documents directly without intermediate OwnedValue conversion,
/// preserving position metadata for `line` and `column` builtins. Returns results
/// grouped by document for proper multi-doc handling (with `---` separators).
///
/// If `doc_filter` is Some((target_doc, global_offset)), only the document at global index
/// `target_doc` will be evaluated (where global index = global_offset + local_doc_index).
/// Returns the number of documents in this file for proper global index tracking.
fn evaluate_yaml_direct_filtered(
    bytes: &[u8],
    expr: &Expr,
    doc_filter: Option<(usize, usize)>,
) -> Result<(Vec<Vec<OwnedValue>>, usize)> {
    let index = YamlIndex::build(bytes).map_err(|e| anyhow::anyhow!("YAML parse error: {}", e))?;
    let root = index.root(bytes);

    // YAML documents are wrapped in a sequence at the root
    match root.value() {
        YamlValue::Sequence(mut docs) => {
            let mut doc_results = Vec::new();
            let mut local_idx = 0;
            while let Some((cursor, rest)) = docs.uncons_cursor() {
                // Check if this document should be evaluated
                let should_eval = match doc_filter {
                    Some((target_doc, global_offset)) => global_offset + local_idx == target_doc,
                    None => true,
                };

                if should_eval {
                    let results = evaluate_yaml_cursor(cursor, expr)?;
                    // Only include documents that have results (select may filter them out)
                    if !results.is_empty() {
                        doc_results.push(results);
                    }
                }

                local_idx += 1;
                docs = rest;
            }
            Ok((doc_results, local_idx))
        }
        _ => {
            // Single document - navigate to actual content
            let should_eval = match doc_filter {
                Some((target_doc, global_offset)) => global_offset == target_doc,
                None => true,
            };

            if should_eval {
                if let Some(content_cursor) = root.first_child() {
                    let results = evaluate_yaml_cursor(content_cursor, expr)?;
                    Ok((vec![results], 1))
                } else {
                    // Empty document
                    Ok((vec![vec![]], 1))
                }
            } else {
                Ok((vec![], 1))
            }
        }
    }
}

/// Evaluate YAML input directly using the generic evaluator with per-document processing.
///
/// This processes YAML documents directly without intermediate OwnedValue conversion,
/// preserving position metadata for `line` and `column` builtins. Returns results
/// grouped by document for proper multi-doc handling (with `---` separators).
#[allow(dead_code)] // Used by tests and may be used directly in future
fn evaluate_yaml_direct(bytes: &[u8], expr: &Expr) -> Result<Vec<Vec<OwnedValue>>> {
    let (results, _) = evaluate_yaml_direct_filtered(bytes, expr, None)?;
    Ok(results)
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
        QueryResult::Break(label) => {
            eprintln!("yq: error: break ${} not in label", label);
            Ok(vec![])
        }
    }
}

/// Evaluate a jq expression directly on a YAML cursor.
///
/// This uses the generic evaluator to preserve position metadata (line/column).
#[allow(dead_code)] // Used by parse_and_evaluate_yaml
fn evaluate_yaml_cursor<W: AsRef<[u64]> + Clone>(
    cursor: YamlCursor<'_, W>,
    expr: &Expr,
) -> Result<Vec<OwnedValue>> {
    let result = eval_with_cursor(expr, cursor);

    // Convert GenericResult to Vec<OwnedValue>
    match result {
        GenericResult::One(v) => Ok(vec![to_owned(&v)]),
        GenericResult::OneCursor(c) => Ok(vec![to_owned(&c.value())]),
        GenericResult::Many(vs) => Ok(vs.iter().map(to_owned).collect()),
        GenericResult::None => Ok(vec![]),
        GenericResult::Error(e) => {
            eprintln!("yq: error: {}", e);
            Ok(vec![])
        }
        GenericResult::Owned(v) => Ok(vec![v]),
        GenericResult::ManyOwned(vs) => Ok(vs),
        GenericResult::Break(label) => {
            eprintln!("yq: error: break ${} not in label", label);
            Ok(vec![])
        }
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

/// State for tracking split_doc output separators.
struct SplitDocState {
    has_split_doc: bool,
    is_first_output: bool,
}

impl SplitDocState {
    fn new(has_split_doc: bool) -> Self {
        Self {
            has_split_doc,
            is_first_output: true,
        }
    }

    /// Write a separator if needed for split_doc mode. Returns Ok(()) always.
    fn write_separator<W: Write>(&mut self, writer: &mut W, config: &OutputConfig) -> Result<()> {
        if self.has_split_doc && config.output_format == OutputFormat::Yaml && !config.no_doc {
            if !self.is_first_output {
                writeln!(writer, "---")?;
            }
            self.is_first_output = false;
        }
        Ok(())
    }
}

/// Write the appropriate line terminator based on output config.
fn write_terminator<W: Write>(writer: &mut W, config: &OutputConfig) -> Result<()> {
    if config.nul_output {
        writer.write_all(&[0])?;
    } else if !config.join_output {
        writeln!(writer)?;
    }
    Ok(())
}

/// Format and output a value.
fn output_value<W: Write>(writer: &mut W, value: &OwnedValue, config: &OutputConfig) -> Result<()> {
    // Handle raw output for scalars
    if config.raw_output {
        if let OwnedValue::String(s) = value {
            write!(writer, "{}", s)?;
            write_terminator(writer, config)?;
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
        write_terminator(writer, config)?;
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

    write_terminator(writer, config)?;

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

/// Check if an expression contains the split_doc builtin.
/// This is used to determine if output should use per-result document separators.
fn contains_split_doc(expr: &Expr) -> bool {
    match expr {
        Expr::Builtin(Builtin::SplitDoc) => true,
        Expr::Pipe(exprs) | Expr::Comma(exprs) => exprs.iter().any(contains_split_doc),
        Expr::Array(inner)
        | Expr::Paren(inner)
        | Expr::Optional(inner)
        | Expr::FirstExpr(inner)
        | Expr::LastExpr(inner)
        | Expr::Repeat(inner)
        | Expr::Error(Some(inner)) => contains_split_doc(inner),
        Expr::Arithmetic { left, right, .. }
        | Expr::Compare { left, right, .. }
        | Expr::And(left, right)
        | Expr::Or(left, right)
        | Expr::Alternative(left, right)
        | Expr::Assign {
            path: left,
            value: right,
        }
        | Expr::Update {
            path: left,
            filter: right,
        }
        | Expr::NthExpr {
            n: left,
            expr: right,
        }
        | Expr::Until {
            cond: left,
            update: right,
        }
        | Expr::While {
            cond: left,
            update: right,
        } => contains_split_doc(left) || contains_split_doc(right),
        Expr::CompoundAssign { path, value, .. } | Expr::AlternativeAssign { path, value } => {
            contains_split_doc(path) || contains_split_doc(value)
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            contains_split_doc(cond)
                || contains_split_doc(then_branch)
                || contains_split_doc(else_branch)
        }
        Expr::Try { expr, catch } => {
            contains_split_doc(expr) || catch.as_ref().is_some_and(|c| contains_split_doc(c))
        }
        Expr::As { expr, body, .. } | Expr::AsPattern { expr, body, .. } => {
            contains_split_doc(expr) || contains_split_doc(body)
        }
        Expr::Label { body, .. } => contains_split_doc(body),
        Expr::Limit { n, expr } => contains_split_doc(n) || contains_split_doc(expr),
        Expr::Reduce {
            input,
            init,
            update,
            ..
        }
        | Expr::Range {
            from: input,
            to: Some(init),
            step: Some(update),
        } => contains_split_doc(input) || contains_split_doc(init) || contains_split_doc(update),
        Expr::Foreach {
            input,
            init,
            update,
            extract,
            ..
        } => {
            contains_split_doc(input)
                || contains_split_doc(init)
                || contains_split_doc(update)
                || extract.as_ref().is_some_and(|e| contains_split_doc(e))
        }
        Expr::Range { from, to, step } => {
            contains_split_doc(from)
                || to.as_ref().is_some_and(|e| contains_split_doc(e))
                || step.as_ref().is_some_and(|e| contains_split_doc(e))
        }
        Expr::Object(entries) => entries.iter().any(|entry| {
            matches!(&entry.key, succinctly::jq::ObjectKey::Expr(e) if contains_split_doc(e))
                || contains_split_doc(&entry.value)
        }),
        Expr::StringInterpolation(parts) => parts.iter().any(
            |part| matches!(part, succinctly::jq::StringPart::Expr(e) if contains_split_doc(e)),
        ),
        Expr::FuncDef { body, then, .. } => contains_split_doc(body) || contains_split_doc(then),
        Expr::FuncCall { args, .. } | Expr::NamespacedCall { args, .. } => {
            args.iter().any(contains_split_doc)
        }
        Expr::Builtin(b) => match b {
            Builtin::Has(e)
            | Builtin::In(e)
            | Builtin::Select(e)
            | Builtin::Map(e)
            | Builtin::MapValues(e)
            | Builtin::MinBy(e)
            | Builtin::MaxBy(e)
            | Builtin::Ltrimstr(e)
            | Builtin::Rtrimstr(e)
            | Builtin::Startswith(e)
            | Builtin::Endswith(e)
            | Builtin::Split(e)
            | Builtin::Join(e)
            | Builtin::Contains(e)
            | Builtin::Inside(e)
            | Builtin::Nth(e)
            | Builtin::FlattenDepth(e)
            | Builtin::GroupBy(e)
            | Builtin::UniqueBy(e)
            | Builtin::SortBy(e)
            | Builtin::WithEntries(e)
            | Builtin::Test(e)
            | Builtin::Indices(e)
            | Builtin::Index(e)
            | Builtin::Rindex(e)
            | Builtin::GetPath(e)
            | Builtin::RecurseF(e)
            | Builtin::Walk(e)
            | Builtin::IsValid(e)
            | Builtin::Path(e)
            | Builtin::ParentN(e)
            | Builtin::PathsFilter(e)
            | Builtin::DelPaths(e)
            | Builtin::DebugMsg(e)
            | Builtin::EnvVar(e)
            | Builtin::BSearch(e)
            | Builtin::ModuleMeta(e)
            | Builtin::Pick(e)
            | Builtin::Omit(e)
            | Builtin::Del(e)
            | Builtin::Strftime(e)
            | Builtin::Strptime(e)
            | Builtin::Match(e)
            | Builtin::Capture(e)
            | Builtin::Scan(e)
            | Builtin::Splits(e)
            | Builtin::Range(e)
            | Builtin::CombinationsN(e)
            | Builtin::Tz(e)
            | Builtin::Load(e) => contains_split_doc(e),
            Builtin::RecurseCond(e1, e2)
            | Builtin::SetPath(e1, e2)
            | Builtin::Pow(e1, e2)
            | Builtin::Atan2(e1, e2)
            | Builtin::Limit(e1, e2)
            | Builtin::NthStream(e1, e2)
            | Builtin::RangeFromTo(e1, e2)
            | Builtin::Skip(e1, e2)
            | Builtin::TestFlags(e1, e2)
            | Builtin::MatchFlags(e1, e2)
            | Builtin::CaptureFlags(e1, e2)
            | Builtin::Sub(e1, e2)
            | Builtin::Gsub(e1, e2)
            | Builtin::ScanFlags(e1, e2)
            | Builtin::SplitRegex(e1, e2)
            | Builtin::SplitsFlags(e1, e2) => contains_split_doc(e1) || contains_split_doc(e2),
            Builtin::FirstStream(e) | Builtin::LastStream(e) | Builtin::IsEmpty(e) => {
                contains_split_doc(e)
            }
            Builtin::RangeFromToBy(e1, e2, e3)
            | Builtin::SubFlags(e1, e2, e3)
            | Builtin::GsubFlags(e1, e2, e3) => {
                contains_split_doc(e1) || contains_split_doc(e2) || contains_split_doc(e3)
            }
            _ => false,
        },
        // Terminal expressions that cannot contain split_doc
        Expr::Identity
        | Expr::Field(_)
        | Expr::Index(_)
        | Expr::Slice { .. }
        | Expr::Iterate
        | Expr::Literal(_)
        | Expr::RecursiveDescent
        | Expr::Not
        | Expr::Format(_)
        | Expr::Var(_)
        | Expr::Loc { .. }
        | Expr::Env
        | Expr::Break(_)
        | Expr::Error(None) => false,
    }
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

    // Validate flag compatibility
    if args.document.is_some() && args.raw_input {
        anyhow::bail!("--doc and --raw-input are incompatible");
    }

    // Parse the jq program (use Yq mode for extended identifier syntax like kebab-case)
    let program = jq::parse_program_with_mode(&filter_str, jq::ParserMode::Yq)
        .map_err(|e| anyhow::anyhow!("parse error: {}", e))?;

    // Parse variables
    let context = parse_variables(&args)?;

    // Output configuration
    let output_config = OutputConfig::from_args(&args);

    // Set up output
    let stdout = std::io::stdout();
    let mut writer = BufWriter::new(stdout.lock());

    // Track last output for exit status (only need to know if it was falsy, not the full value)
    let mut last_was_falsy = false;
    let mut had_output = false;

    // Check if expression contains split_doc - if so, each result is a separate document
    let has_split_doc = contains_split_doc(&program.expr);

    // Fast path: identity filter with JSON compact output - stream directly from YAML cursor
    // This avoids building OwnedValue DOM and JSON round-trip
    let is_identity = matches!(program.expr, Expr::Identity);
    let can_fast_path = is_identity
        && output_config.compact
        && output_config.output_format == OutputFormat::Json
        && !args.null_input
        && !args.raw_input
        && !args.slurp
        && !args.inplace
        && context.named.is_empty();

    if can_fast_path {
        // Direct YAML → JSON streaming
        // Track global document index across all files for --doc filtering
        let mut global_doc_index: usize = 0;

        if args.files.is_empty() {
            let yaml_bytes = read_stdin()?;
            let index = YamlIndex::build(&yaml_bytes)
                .map_err(|e| anyhow::anyhow!("YAML parse error: {}", e))?;
            let root = index.root(&yaml_bytes);

            // Output each document directly using cursor iteration with streaming
            match root.value() {
                YamlValue::Sequence(mut docs) => {
                    while let Some((cursor, rest)) = docs.uncons_cursor() {
                        // Apply --doc filter if specified
                        if let Some(target_doc) = args.document {
                            if global_doc_index == target_doc {
                                had_output = true;
                                cursor
                                    .stream_json(&mut FmtWriter(&mut writer))
                                    .map_err(|_| anyhow::anyhow!("Write error"))?;
                                writeln!(writer)?;
                            }
                        } else {
                            had_output = true;
                            cursor
                                .stream_json(&mut FmtWriter(&mut writer))
                                .map_err(|_| anyhow::anyhow!("Write error"))?;
                            writeln!(writer)?;
                        }
                        global_doc_index += 1;
                        docs = rest;
                    }
                }
                _ => {
                    // Single document case
                    if args.document.is_none() || args.document == Some(0) {
                        had_output = true;
                        root.stream_json_document(&mut FmtWriter(&mut writer))
                            .map_err(|_| anyhow::anyhow!("Write error"))?;
                        writeln!(writer)?;
                    }
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
                            // Apply --doc filter if specified
                            if let Some(target_doc) = args.document {
                                if global_doc_index == target_doc {
                                    had_output = true;
                                    cursor
                                        .stream_json(&mut FmtWriter(&mut writer))
                                        .map_err(|_| anyhow::anyhow!("Write error"))?;
                                    writeln!(writer)?;
                                }
                            } else {
                                had_output = true;
                                cursor
                                    .stream_json(&mut FmtWriter(&mut writer))
                                    .map_err(|_| anyhow::anyhow!("Write error"))?;
                                writeln!(writer)?;
                            }
                            global_doc_index += 1;
                            docs = rest;
                        }
                    }
                    _ => {
                        // Single document case
                        if args.document.is_none() || args.document == Some(global_doc_index) {
                            had_output = true;
                            root.stream_json_document(&mut FmtWriter(&mut writer))
                                .map_err(|_| anyhow::anyhow!("Write error"))?;
                            writeln!(writer)?;
                        }
                        global_doc_index += 1;
                    }
                }
            }
        }
    } else if args.null_input {
        // Handle --null-input
        let mut split_doc_state = SplitDocState::new(has_split_doc);
        let results = evaluate_input(&OwnedValue::Null, &program.expr, &context)?;
        for result in results {
            split_doc_state.write_separator(&mut writer, &output_config)?;
            last_was_falsy = matches!(&result, OwnedValue::Null | OwnedValue::Bool(false));
            had_output = true;
            output_value(&mut writer, &result, &output_config)?;
        }
    } else if args.raw_input {
        // Handle --raw-input: read each line as a string instead of parsing as YAML
        let input_content = if args.files.is_empty() {
            read_stdin_string()?
        } else {
            let mut content = String::new();
            for file_path in &args.files {
                let file_content = std::fs::read_to_string(file_path)
                    .with_context(|| format!("failed to read file: {}", file_path))?;
                content.push_str(&file_content);
            }
            content
        };

        let mut split_doc_state = SplitDocState::new(has_split_doc);
        if args.slurp {
            // With --slurp, collect all lines into an array
            let lines: Vec<OwnedValue> = input_content
                .lines()
                .map(|line| OwnedValue::String(line.to_string()))
                .collect();
            let slurped = OwnedValue::Array(lines);
            let results = evaluate_input(&slurped, &program.expr, &context)?;
            for result in results {
                split_doc_state.write_separator(&mut writer, &output_config)?;
                last_was_falsy = matches!(&result, OwnedValue::Null | OwnedValue::Bool(false));
                had_output = true;
                output_value(&mut writer, &result, &output_config)?;
            }
        } else {
            // Without --slurp, process each line independently
            for line in input_content.lines() {
                let input = OwnedValue::String(line.to_string());
                let results = evaluate_input(&input, &program.expr, &context)?;
                for result in results {
                    split_doc_state.write_separator(&mut writer, &output_config)?;
                    last_was_falsy = matches!(&result, OwnedValue::Null | OwnedValue::Bool(false));
                    had_output = true;
                    output_value(&mut writer, &result, &output_config)?;
                }
            }
        }
    } else if args.slurp {
        // Handle --slurp: collect all documents from all inputs into an array
        let mut all_docs: Vec<OwnedValue> = Vec::new();

        // Collect input sources
        let input_sources: Vec<(Vec<u8>, InputFormat)> = if args.files.is_empty() {
            let input_bytes = read_stdin()?;
            let format = resolve_input_format(args.input_format, None);
            vec![(input_bytes, format)]
        } else {
            let mut sources = Vec::new();
            for file_path in &args.files {
                let path = Path::new(file_path);
                let input_bytes = read_file(path)?;
                let format = resolve_input_format(args.input_format, Some(path));
                sources.push((input_bytes, format));
            }
            sources
        };

        // Parse all inputs and collect documents
        let mut global_doc_index: usize = 0;
        for (bytes, format) in &input_sources {
            let inputs = parse_input(bytes, *format)?;
            for input in inputs {
                // Apply --doc filter if specified
                if let Some(target_doc) = args.document {
                    if global_doc_index == target_doc {
                        all_docs.push(input);
                    }
                } else {
                    all_docs.push(input);
                }
                global_doc_index += 1;
            }
        }

        // Create slurped array and evaluate
        let slurped = OwnedValue::Array(all_docs);
        let results = evaluate_input(&slurped, &program.expr, &context)?;
        let mut split_doc_state = SplitDocState::new(has_split_doc);
        for result in results {
            split_doc_state.write_separator(&mut writer, &output_config)?;
            last_was_falsy = matches!(&result, OwnedValue::Null | OwnedValue::Bool(false));
            had_output = true;
            output_value(&mut writer, &result, &output_config)?;
        }
    } else if args.inplace {
        // Handle --inplace: process each file and write back to it
        if args.files.is_empty() {
            anyhow::bail!("--inplace requires at least one file argument");
        }

        let mut global_doc_index: usize = 0;
        for file_path in &args.files {
            let path = Path::new(file_path);
            let input_bytes = read_file(path)?;
            let format = resolve_input_format(args.input_format, Some(path));
            let inputs = parse_input(&input_bytes, format)?;

            // Collect all output into a buffer
            let mut output_buffer = Vec::new();
            {
                let mut buf_writer = BufWriter::new(&mut output_buffer);
                // Count matching docs for multi-doc separator logic
                let matching_docs: usize = if let Some(target_doc) = args.document {
                    if (global_doc_index..global_doc_index + inputs.len()).contains(&target_doc) {
                        1
                    } else {
                        0
                    }
                } else {
                    inputs.len()
                };
                let is_multi_doc = matching_docs > 1;

                let mut split_doc_state = SplitDocState::new(has_split_doc);
                for (local_idx, input) in inputs.iter().enumerate() {
                    let current_doc_index = global_doc_index + local_idx;
                    // Apply --doc filter if specified
                    if let Some(target_doc) = args.document {
                        if current_doc_index != target_doc {
                            continue;
                        }
                    }

                    // For regular multi-doc (without split_doc), add --- before each doc
                    if !has_split_doc
                        && output_config.output_format == OutputFormat::Yaml
                        && !output_config.no_doc
                        && is_multi_doc
                    {
                        writeln!(buf_writer, "---")?;
                    }
                    let results = evaluate_input(input, &program.expr, &context)?;
                    // Write without color for inplace editing
                    let mut no_color_config = output_config.clone();
                    no_color_config.use_color = false;
                    for result in results {
                        split_doc_state.write_separator(&mut buf_writer, &no_color_config)?;
                        last_was_falsy =
                            matches!(&result, OwnedValue::Null | OwnedValue::Bool(false));
                        had_output = true;
                        output_value(&mut buf_writer, &result, &no_color_config)?;
                    }
                }
                buf_writer.flush()?;
            }
            global_doc_index += inputs.len();

            // Write the output back to the file
            std::fs::write(path, &output_buffer)
                .with_context(|| format!("failed to write to file: {}", path.display()))?;
        }
    } else {
        // Standard path: evaluate inputs
        // For YAML inputs, use direct evaluation to preserve position metadata
        // For JSON inputs, use the OwnedValue path

        // Collect input sources with their bytes and formats
        let input_sources: Vec<(Vec<u8>, InputFormat)> = if args.files.is_empty() {
            let input_bytes = read_stdin()?;
            let format = resolve_input_format(args.input_format, None);
            vec![(input_bytes, format)]
        } else {
            let mut sources = Vec::new();
            for file_path in &args.files {
                let path = Path::new(file_path);
                let input_bytes = read_file(path)?;
                let format = resolve_input_format(args.input_format, Some(path));
                sources.push((input_bytes, format));
            }
            sources
        };

        // Process all inputs first to collect results, then determine multi-doc status
        // This avoids double-parsing YAML for document counting
        // Each entry in all_results is a Vec of document results from one file
        let mut all_results: Vec<Vec<Vec<OwnedValue>>> = Vec::new();
        let mut global_doc_index: usize = 0;
        for (bytes, format) in &input_sources {
            match format {
                InputFormat::Yaml | InputFormat::Auto => {
                    // Use direct YAML evaluation to preserve position metadata
                    // Filter at evaluation time to avoid evaluating (and printing errors for)
                    // documents that don't match the --doc filter
                    let doc_filter = args.document.map(|target| (target, global_doc_index));
                    let (doc_results, num_docs) =
                        evaluate_yaml_direct_filtered(bytes, &program.expr, doc_filter)?;
                    global_doc_index += num_docs;
                    all_results.push(doc_results);
                }
                InputFormat::Json => {
                    // Use OwnedValue path for JSON
                    let inputs = parse_input(bytes, InputFormat::Json)?;
                    let mut json_results = Vec::new();
                    for input in inputs {
                        // Apply --doc filter if specified
                        if let Some(target_doc) = args.document {
                            if global_doc_index != target_doc {
                                global_doc_index += 1;
                                continue;
                            }
                        }
                        let results = evaluate_input(&input, &program.expr, &context)?;
                        json_results.push(results);
                        global_doc_index += 1;
                    }
                    all_results.push(json_results);
                }
            }
        }

        // Count total documents from collected results (after filtering)
        let total_docs: usize = all_results.iter().map(|docs| docs.len()).sum();
        let is_multi_doc = total_docs > 1;

        // Output all results with proper separators
        // For split_doc: add --- BETWEEN each result (not before first)
        // For regular multi-doc: add --- before each document's results
        let mut split_doc_state = SplitDocState::new(has_split_doc);
        for doc_results in all_results {
            for results in doc_results {
                // Add document separator in YAML mode for multi-doc (before each doc's results)
                if !has_split_doc
                    && output_config.output_format == OutputFormat::Yaml
                    && !output_config.no_doc
                    && is_multi_doc
                {
                    writeln!(writer, "---")?;
                }
                for result in results {
                    split_doc_state.write_separator(&mut writer, &output_config)?;
                    last_was_falsy = matches!(&result, OwnedValue::Null | OwnedValue::Bool(false));
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
        if last_was_falsy {
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

    // Tests for the generic evaluator integration

    #[test]
    fn test_parse_and_evaluate_yaml_identity() {
        let yaml = b"name: Alice\nage: 30";
        let expr = Expr::Identity;
        let results = parse_and_evaluate_yaml(yaml, &expr).unwrap();

        assert_eq!(results.len(), 1);
        if let OwnedValue::Object(map) = &results[0] {
            assert_eq!(
                map.get("name"),
                Some(&OwnedValue::String("Alice".to_string()))
            );
            assert_eq!(map.get("age"), Some(&OwnedValue::Int(30)));
        } else {
            panic!("expected object, got {:?}", results[0]);
        }
    }

    #[test]
    fn test_parse_and_evaluate_yaml_field() {
        let yaml = b"name: Alice\nage: 30";
        let expr = Expr::Field("name".to_string());
        let results = parse_and_evaluate_yaml(yaml, &expr).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0], OwnedValue::String("Alice".to_string()));
    }

    #[test]
    fn test_parse_and_evaluate_yaml_line_builtin() {
        use succinctly::jq::Builtin;

        let yaml = b"name: Alice\nage: 30";
        let expr = Expr::Builtin(Builtin::Line);
        let results = parse_and_evaluate_yaml(yaml, &expr).unwrap();

        assert_eq!(results.len(), 1);
        // The mapping starts at line 1
        assert_eq!(results[0], OwnedValue::Int(1));
    }

    #[test]
    fn test_parse_and_evaluate_yaml_pipe() {
        let yaml = b"users:\n  - name: Alice\n  - name: Bob";
        // .users | .[0] | .name
        let expr = Expr::Pipe(vec![
            Expr::Field("users".to_string()),
            Expr::Index(0),
            Expr::Field("name".to_string()),
        ]);
        let results = parse_and_evaluate_yaml(yaml, &expr).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0], OwnedValue::String("Alice".to_string()));
    }

    #[test]
    fn test_slurp_multi_doc_yaml() {
        // Test slurp behavior by parsing multi-doc YAML manually
        let yaml = b"---\nname: Alice\n---\nname: Bob\n---\nname: Charlie";
        let inputs = parse_input(yaml, InputFormat::Yaml).unwrap();

        // Multi-doc YAML should parse into 3 documents
        assert_eq!(inputs.len(), 3);

        // When slurped, they become an array
        let slurped = OwnedValue::Array(inputs);
        if let OwnedValue::Array(arr) = slurped {
            assert_eq!(arr.len(), 3);

            // Verify each document
            if let OwnedValue::Object(map) = &arr[0] {
                assert_eq!(
                    map.get("name"),
                    Some(&OwnedValue::String("Alice".to_string()))
                );
            } else {
                panic!("expected object");
            }
            if let OwnedValue::Object(map) = &arr[1] {
                assert_eq!(
                    map.get("name"),
                    Some(&OwnedValue::String("Bob".to_string()))
                );
            } else {
                panic!("expected object");
            }
            if let OwnedValue::Object(map) = &arr[2] {
                assert_eq!(
                    map.get("name"),
                    Some(&OwnedValue::String("Charlie".to_string()))
                );
            } else {
                panic!("expected object");
            }
        } else {
            panic!("expected array");
        }
    }

    #[test]
    fn test_slurp_with_length() {
        // Test that slurped docs can have length computed
        let yaml = b"---\nname: Alice\n---\nname: Bob\n---\nname: Charlie";
        let inputs = parse_input(yaml, InputFormat::Yaml).unwrap();
        let slurped = OwnedValue::Array(inputs);

        let expr = succinctly::jq::parse("length").unwrap();
        let results = evaluate_input(&slurped, &expr, &EvalContext::default()).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0], OwnedValue::Int(3));
    }

    #[test]
    fn test_explicit_empty_key() {
        // Test explicit key syntax with empty key
        // YAML: ?\n: value
        let yaml = b"?\n: value\n";
        let inputs = parse_input(yaml, InputFormat::Yaml).unwrap();

        assert_eq!(inputs.len(), 1);
        if let OwnedValue::Object(map) = &inputs[0] {
            println!("map len: {}", map.len());
            for (k, v) in map.iter() {
                println!("  key={:?}, value={:?}", k, v);
            }
            assert_eq!(map.len(), 1);
            // Empty key (null key in YAML becomes empty string in our representation)
            // The key should be preserved - could be "" or "null" depending on conversion
            // Let's check what we have
            assert!(map.contains_key("") || map.contains_key("null"));
        } else {
            panic!("expected object, got {:?}", inputs[0]);
        }
    }

    #[test]
    fn test_explicit_empty_key_direct_eval() {
        // Test explicit key syntax with direct YAML evaluation
        let yaml = b"?\n: value\n";
        let expr = Expr::Identity;
        let results = parse_and_evaluate_yaml(yaml, &expr).unwrap();

        assert_eq!(results.len(), 1);
        if let OwnedValue::Object(map) = &results[0] {
            println!("direct eval map len: {}", map.len());
            for (k, v) in map.iter() {
                println!("  key={:?}, value={:?}", k, v);
            }
            assert_eq!(map.len(), 1, "expected 1 key but got {} keys", map.len());
        } else {
            panic!("expected object, got {:?}", results[0]);
        }
    }
}
