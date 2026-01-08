//! jq-compatible command runner for succinctly.
//!
//! This module implements a jq-compatible CLI interface using the succinctly
//! JSON semi-indexing and jq expression evaluator.

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::io::{BufWriter, Read, Write};
use std::path::Path;

use succinctly::jq::{self, OwnedValue, QueryResult};
use succinctly::json::light::StandardJson;
use succinctly::json::JsonIndex;

use super::JqCommand;

/// Exit codes matching jq behavior
pub mod exit_codes {
    pub const SUCCESS: i32 = 0;
    pub const FALSE_OR_NULL: i32 = 1; // With -e, last output was false or null
    pub const USAGE_ERROR: i32 = 2; // Usage problem or system error
    pub const COMPILE_ERROR: i32 = 3; // jq program compile error
    pub const NO_OUTPUT: i32 = 4; // With -e, no valid result produced
    #[allow(dead_code)]
    pub const HALT_ERROR: i32 = 5; // halt_error without explicit code
}

/// Evaluation context for passing variables to the jq evaluator.
#[derive(Debug, Default)]
pub struct EvalContext {
    /// Named arguments from --arg, --argjson, --slurpfile, --rawfile
    pub named: BTreeMap<String, OwnedValue>,
    /// Positional arguments from --args or --jsonargs
    pub positional: Vec<OwnedValue>,
}

/// Output formatting configuration
struct OutputConfig {
    compact: bool,
    raw_output: bool,
    join_output: bool,
    sort_keys: bool,
    indent_string: String,
}

impl OutputConfig {
    fn from_args(args: &JqCommand) -> Self {
        let indent_string = if args.tab {
            "\t".to_string()
        } else if let Some(n) = args.indent {
            " ".repeat(n as usize)
        } else if args.compact_output {
            String::new()
        } else {
            "  ".to_string() // Default: 2 spaces
        };

        OutputConfig {
            compact: args.compact_output,
            raw_output: args.raw_output || args.join_output,
            join_output: args.join_output,
            sort_keys: args.sort_keys,
            indent_string,
        }
    }
}

/// Run the jq command with the given arguments.
/// Returns the exit code (0 for success, non-zero for various errors).
pub fn run_jq(args: JqCommand) -> Result<i32> {
    // Build evaluation context from arguments
    let context = build_context(&args)?;

    // Get the filter expression
    let filter_str = get_filter(&args)?;

    // Parse the filter
    let expr = jq::parse(&filter_str).map_err(|e| {
        eprintln!("jq: compile error: {}", e);
        anyhow::anyhow!("compile error")
    })?;

    // Get input values
    let inputs = get_inputs(&args)?;

    // Configure output
    let output_config = OutputConfig::from_args(&args);

    // Set up output writer
    let stdout = std::io::stdout();
    let mut out = BufWriter::new(stdout.lock());

    // Track last output for exit status
    let mut last_output: Option<OwnedValue> = None;
    let mut had_output = false;

    // Process each input
    for input in inputs {
        let results = evaluate_input(&input, &expr, &context)?;

        for result in results {
            had_output = true;
            last_output = Some(result.clone());
            write_output(&mut out, &result, &output_config)?;
        }
    }

    out.flush()?;

    // Determine exit code
    if args.exit_status {
        if !had_output {
            return Ok(exit_codes::NO_OUTPUT);
        }
        if let Some(last) = last_output {
            if matches!(last, OwnedValue::Null | OwnedValue::Bool(false)) {
                return Ok(exit_codes::FALSE_OR_NULL);
            }
        }
    }

    Ok(exit_codes::SUCCESS)
}

/// Build the evaluation context from command-line arguments.
fn build_context(args: &JqCommand) -> Result<EvalContext> {
    let mut context = EvalContext::default();

    // Process --arg name value pairs
    for chunk in args.arg.chunks(2) {
        if let [name, value] = chunk {
            context
                .named
                .insert(name.clone(), OwnedValue::String(value.clone()));
        }
    }

    // Process --argjson name value pairs
    for chunk in args.argjson.chunks(2) {
        if let [name, value] = chunk {
            let json_value = parse_json_value(value)
                .with_context(|| format!("Invalid JSON for --argjson {}", name))?;
            context.named.insert(name.clone(), json_value);
        }
    }

    // Process --slurpfile name file pairs
    for chunk in args.slurpfile.chunks(2) {
        if let [name, file] = chunk {
            let contents = std::fs::read_to_string(file)
                .with_context(|| format!("Failed to read file for --slurpfile {}", name))?;
            let values = parse_json_stream(&contents)?;
            context
                .named
                .insert(name.clone(), OwnedValue::Array(values));
        }
    }

    // Process --rawfile name file pairs
    for chunk in args.rawfile.chunks(2) {
        if let [name, file] = chunk {
            let contents = std::fs::read_to_string(file)
                .with_context(|| format!("Failed to read file for --rawfile {}", name))?;
            context
                .named
                .insert(name.clone(), OwnedValue::String(contents));
        }
    }

    // Note: --args and --jsonargs handling would require modifying how we parse
    // the command line to capture remaining arguments. For now, files are treated
    // as input files, not positional arguments.

    Ok(context)
}

/// Get the filter expression from arguments.
fn get_filter(args: &JqCommand) -> Result<String> {
    if let Some(ref path) = args.from_file {
        // Filter comes from file
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read filter file: {}", path.display()))?;
        Ok(contents.trim().to_string())
    } else if !args.positional_args.is_empty() {
        // First positional arg is the filter
        Ok(args.positional_args[0].clone())
    } else {
        Ok(".".to_string()) // Default: identity filter
    }
}

/// Get input files from arguments.
fn get_input_files(args: &JqCommand) -> Vec<std::path::PathBuf> {
    if args.from_file.is_some() {
        // When -f is used, all positional args are input files
        args.positional_args
            .iter()
            .map(std::path::PathBuf::from)
            .collect()
    } else if args.positional_args.len() > 1 {
        // First arg is filter, rest are input files
        args.positional_args[1..]
            .iter()
            .map(std::path::PathBuf::from)
            .collect()
    } else {
        vec![]
    }
}

/// Get input values based on arguments.
fn get_inputs(args: &JqCommand) -> Result<Vec<OwnedValue>> {
    // Null input mode: use null as the single input
    if args.null_input {
        return Ok(vec![OwnedValue::Null]);
    }

    // Get input files
    let files = get_input_files(args);

    // Collect raw input from files or stdin
    let raw_inputs = if files.is_empty() {
        vec![read_stdin()?]
    } else {
        files
            .iter()
            .map(|path| read_file(path))
            .collect::<Result<Vec<_>>>()?
    };

    // Process based on input mode
    let mut values = Vec::new();

    for raw in raw_inputs {
        if args.raw_input {
            // Raw input: each line becomes a string
            for line in raw.lines() {
                values.push(OwnedValue::String(line.to_string()));
            }
        } else {
            // JSON input: parse as JSON stream
            let parsed = parse_json_stream(&raw)?;
            values.extend(parsed);
        }
    }

    // Slurp mode: wrap all inputs in an array
    if args.slurp {
        Ok(vec![OwnedValue::Array(values)])
    } else {
        Ok(values)
    }
}

/// Read stdin to string.
fn read_stdin() -> Result<String> {
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .context("Failed to read from stdin")?;
    Ok(buf)
}

/// Read a file to string.
fn read_file(path: &Path) -> Result<String> {
    std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read file: {}", path.display()))
}

/// Parse a JSON value from a string.
fn parse_json_value(s: &str) -> Result<OwnedValue> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(OwnedValue::Null);
    }

    // Use serde_json for parsing, then convert to OwnedValue
    let value: serde_json::Value =
        serde_json::from_str(s).with_context(|| format!("Invalid JSON: {}", s))?;

    Ok(serde_to_owned(&value))
}

/// Parse a JSON stream (multiple JSON values) from a string.
fn parse_json_stream(s: &str) -> Result<Vec<OwnedValue>> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(vec![]);
    }

    // Try to parse as a stream of JSON values
    let mut values = Vec::new();
    let mut deserializer = serde_json::Deserializer::from_str(s).into_iter::<serde_json::Value>();

    while let Some(result) = deserializer.next() {
        let value = result.context("Invalid JSON in stream")?;
        values.push(serde_to_owned(&value));
    }

    Ok(values)
}

/// Convert serde_json::Value to OwnedValue.
fn serde_to_owned(value: &serde_json::Value) -> OwnedValue {
    match value {
        serde_json::Value::Null => OwnedValue::Null,
        serde_json::Value::Bool(b) => OwnedValue::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                OwnedValue::Int(i)
            } else if let Some(f) = n.as_f64() {
                OwnedValue::Float(f)
            } else {
                OwnedValue::Null
            }
        }
        serde_json::Value::String(s) => OwnedValue::String(s.clone()),
        serde_json::Value::Array(arr) => {
            OwnedValue::Array(arr.iter().map(serde_to_owned).collect())
        }
        serde_json::Value::Object(obj) => OwnedValue::Object(
            obj.iter()
                .map(|(k, v)| (k.clone(), serde_to_owned(v)))
                .collect(),
        ),
    }
}

/// Evaluate the expression against an input value.
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

    // TODO: Pass context to evaluator for variable resolution
    // For now, we use the basic eval which doesn't support external variables
    let result = jq::eval(expr, cursor);

    // Convert result to Vec<OwnedValue>
    match result {
        QueryResult::One(v) => Ok(vec![standard_json_to_owned(&v)]),
        QueryResult::Many(vs) => Ok(vs.iter().map(standard_json_to_owned).collect()),
        QueryResult::None => Ok(vec![]),
        QueryResult::Error(e) => {
            eprintln!("jq: error: {}", e);
            Ok(vec![])
        }
        QueryResult::Owned(v) => Ok(vec![v]),
        QueryResult::ManyOwned(vs) => Ok(vs),
    }
}

/// Convert StandardJson to OwnedValue.
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
        StandardJson::Array(elements) => OwnedValue::Array(
            elements
                .clone()
                .map(|e| standard_json_to_owned(&e))
                .collect(),
        ),
        StandardJson::Object(fields) => OwnedValue::Object(
            fields
                .clone()
                .map(|f| {
                    let key = match f.key() {
                        StandardJson::String(s) => s.as_str().unwrap_or_default().to_string(),
                        _ => String::new(),
                    };
                    (key, standard_json_to_owned(&f.value()))
                })
                .collect(),
        ),
        StandardJson::Error(_) => OwnedValue::Null,
    }
}

/// Write a single output value.
fn write_output<W: Write>(out: &mut W, value: &OwnedValue, config: &OutputConfig) -> Result<()> {
    let output = if config.sort_keys {
        format_json_sorted(value, config)
    } else {
        format_json(value, config)
    };

    // Handle raw output for strings
    if config.raw_output {
        if let OwnedValue::String(s) = value {
            out.write_all(s.as_bytes())?;
            if !config.join_output {
                out.write_all(b"\n")?;
            }
            return Ok(());
        }
    }

    out.write_all(output.as_bytes())?;
    if !config.join_output {
        out.write_all(b"\n")?;
    }

    Ok(())
}

/// Format a value as JSON.
fn format_json(value: &OwnedValue, config: &OutputConfig) -> String {
    if config.compact {
        value.to_json()
    } else {
        format_json_pretty(value, &config.indent_string, 0)
    }
}

/// Format a value as JSON with sorted keys.
fn format_json_sorted(value: &OwnedValue, config: &OutputConfig) -> String {
    let sorted = sort_keys(value);
    format_json(&sorted, config)
}

/// Sort object keys recursively.
fn sort_keys(value: &OwnedValue) -> OwnedValue {
    match value {
        OwnedValue::Object(obj) => {
            let sorted: BTreeMap<String, OwnedValue> =
                obj.iter().map(|(k, v)| (k.clone(), sort_keys(v))).collect();
            OwnedValue::Object(sorted)
        }
        OwnedValue::Array(arr) => OwnedValue::Array(arr.iter().map(sort_keys).collect()),
        _ => value.clone(),
    }
}

/// Format JSON with pretty printing.
fn format_json_pretty(value: &OwnedValue, indent: &str, level: usize) -> String {
    let current_indent = indent.repeat(level);
    let next_indent = indent.repeat(level + 1);

    match value {
        OwnedValue::Null => "null".to_string(),
        OwnedValue::Bool(b) => b.to_string(),
        OwnedValue::Int(i) => i.to_string(),
        OwnedValue::Float(f) => {
            if f.is_nan() {
                "null".to_string() // JSON doesn't support NaN
            } else if f.is_infinite() {
                "null".to_string() // JSON doesn't support Infinity
            } else {
                f.to_string()
            }
        }
        OwnedValue::String(s) => format!("\"{}\"", escape_json_string(s)),
        OwnedValue::Array(arr) => {
            if arr.is_empty() {
                "[]".to_string()
            } else {
                let items: Vec<String> = arr
                    .iter()
                    .map(|v| {
                        format!(
                            "{}{}",
                            next_indent,
                            format_json_pretty(v, indent, level + 1)
                        )
                    })
                    .collect();
                format!("[\n{}\n{}]", items.join(",\n"), current_indent)
            }
        }
        OwnedValue::Object(obj) => {
            if obj.is_empty() {
                "{}".to_string()
            } else {
                let items: Vec<String> = obj
                    .iter()
                    .map(|(k, v)| {
                        format!(
                            "{}\"{}\": {}",
                            next_indent,
                            escape_json_string(k),
                            format_json_pretty(v, indent, level + 1)
                        )
                    })
                    .collect();
                format!("{{\n{}\n{}}}", items.join(",\n"), current_indent)
            }
        }
    }
}

/// Escape special characters in a JSON string.
fn escape_json_string(s: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_json_value() {
        assert!(matches!(
            parse_json_value("null").unwrap(),
            OwnedValue::Null
        ));
        assert!(matches!(
            parse_json_value("true").unwrap(),
            OwnedValue::Bool(true)
        ));
        assert!(matches!(
            parse_json_value("42").unwrap(),
            OwnedValue::Int(42)
        ));
        assert!(matches!(
            parse_json_value("\"hello\"").unwrap(),
            OwnedValue::String(_)
        ));
    }

    #[test]
    fn test_parse_json_stream() {
        let stream = r#"{"a":1} {"b":2} {"c":3}"#;
        let values = parse_json_stream(stream).unwrap();
        assert_eq!(values.len(), 3);
    }

    #[test]
    fn test_escape_json_string() {
        assert_eq!(escape_json_string("hello"), "hello");
        assert_eq!(escape_json_string("hello\nworld"), "hello\\nworld");
        assert_eq!(escape_json_string("say \"hi\""), "say \\\"hi\\\"");
    }

    #[test]
    fn test_sort_keys() {
        let mut obj = BTreeMap::new();
        obj.insert("z".to_string(), OwnedValue::Int(1));
        obj.insert("a".to_string(), OwnedValue::Int(2));
        let value = OwnedValue::Object(obj);

        let sorted = sort_keys(&value);
        if let OwnedValue::Object(obj) = sorted {
            let keys: Vec<_> = obj.keys().collect();
            assert_eq!(keys, vec!["a", "z"]);
        }
    }
}
