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
    #[allow(dead_code)]
    pub const USAGE_ERROR: i32 = 2; // Usage problem or system error
    #[allow(dead_code)]
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
    raw_output0: bool,
    ascii_output: bool,
    color_output: bool,
    sort_keys: bool,
    indent_string: String,
    unbuffered: bool,
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

        // Determine color output: -C forces on, -M forces off, default is auto (tty check)
        let color_output = if args.monochrome_output {
            false
        } else if args.color_output {
            true
        } else {
            // Default: color if stdout is a terminal
            atty::is(atty::Stream::Stdout)
        };

        OutputConfig {
            compact: args.compact_output,
            raw_output: args.raw_output || args.join_output || args.raw_output0,
            join_output: args.join_output,
            raw_output0: args.raw_output0,
            ascii_output: args.ascii_output,
            color_output,
            sort_keys: args.sort_keys,
            indent_string,
            unbuffered: args.unbuffered,
        }
    }
}

/// Run the jq command with the given arguments.
/// Returns the exit code (0 for success, non-zero for various errors).
pub fn run_jq(args: JqCommand) -> Result<i32> {
    // Handle --version flag
    if args.version {
        println!(
            "succinctly jq - JSON processor [version {}]",
            env!("CARGO_PKG_VERSION")
        );
        return Ok(exit_codes::SUCCESS);
    }

    // Build evaluation context from arguments
    let context = build_context(&args)?;

    // Get the filter expression
    let filter_str = get_filter(&args)?;

    // Parse the filter
    let expr = jq::parse(&filter_str).map_err(|e| {
        eprintln!("jq: compile error: {}", e);
        anyhow::anyhow!("compile error")
    })?;

    // Build the $ARGS special variable
    let args_value = build_args_var(&context);

    // Substitute variables from context into the expression
    // First substitute regular named variables, then add $ARGS
    let mut all_vars: Vec<(&str, &OwnedValue)> =
        context.named.iter().map(|(k, v)| (k.as_str(), v)).collect();
    all_vars.push(("ARGS", &args_value));

    let expr = jq::substitute_vars(&expr, all_vars);

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

    // Process --args: values become string positional args
    for arg in &args.args {
        context.positional.push(OwnedValue::String(arg.clone()));
    }

    // Process --jsonargs: values become JSON positional args
    for arg in &args.jsonargs {
        let json_value = parse_json_value(arg)
            .with_context(|| format!("Invalid JSON for --jsonargs: {}", arg))?;
        context.positional.push(json_value);
    }

    Ok(context)
}

/// Build the $ARGS special variable containing named and positional args.
fn build_args_var(context: &EvalContext) -> OwnedValue {
    let mut args_obj = BTreeMap::new();

    // Build named object from context.named
    let named_obj: BTreeMap<String, OwnedValue> = context.named.clone();
    args_obj.insert("named".to_string(), OwnedValue::Object(named_obj));

    // Build positional array from context.positional
    args_obj.insert(
        "positional".to_string(),
        OwnedValue::Array(context.positional.clone()),
    );

    OwnedValue::Object(args_obj)
}

/// Get the filter expression from arguments.
fn get_filter(args: &JqCommand) -> Result<String> {
    if let Some(ref path) = args.from_file {
        // Filter comes from file
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read filter file: {}", path.display()))?;
        Ok(contents.trim().to_string())
    } else if let Some(ref filter) = args.filter {
        Ok(filter.clone())
    } else {
        Ok(".".to_string()) // Default: identity filter
    }
}

/// Get input files from arguments.
fn get_input_files(args: &JqCommand) -> Vec<std::path::PathBuf> {
    // With --args or --jsonargs, files are not used (they would have been consumed)
    if !args.args.is_empty() || !args.jsonargs.is_empty() {
        return vec![];
    }

    // When -f is used, the 'filter' field becomes the first input file
    // because the filter comes from a file instead of command line
    let mut files: Vec<std::path::PathBuf> = Vec::new();

    if args.from_file.is_some() {
        // When -f is used, the first positional arg (if any) is an input file
        if let Some(ref first_file) = args.filter {
            files.push(std::path::PathBuf::from(first_file));
        }
    }

    // Add remaining files
    files.extend(args.files.iter().map(std::path::PathBuf::from));

    files
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
            write_terminator(out, config)?;
            return Ok(());
        }
    }

    out.write_all(output.as_bytes())?;
    write_terminator(out, config)?;

    Ok(())
}

/// Write the appropriate line terminator based on config.
fn write_terminator<W: Write>(out: &mut W, config: &OutputConfig) -> Result<()> {
    if config.raw_output0 {
        out.write_all(&[0])?; // NUL byte
    } else if !config.join_output {
        out.write_all(b"\n")?;
    }
    if config.unbuffered {
        out.flush()?;
    }
    Ok(())
}

/// Format a value as JSON.
fn format_json(value: &OwnedValue, config: &OutputConfig) -> String {
    let json = if config.compact {
        format_json_impl(value, "", 0, config.ascii_output)
    } else {
        format_json_impl(value, &config.indent_string, 0, config.ascii_output)
    };

    if config.color_output {
        colorize_json(&json)
    } else {
        json
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

/// Format JSON implementation with optional pretty printing and ASCII mode.
fn format_json_impl(value: &OwnedValue, indent: &str, level: usize, ascii: bool) -> String {
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
            if f.is_nan() {
                "null".to_string() // JSON doesn't support NaN
            } else if f.is_infinite() {
                "null".to_string() // JSON doesn't support Infinity
            } else {
                f.to_string()
            }
        }
        OwnedValue::String(s) => {
            if ascii {
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
                    .map(|v| format_json_impl(v, indent, level + 1, ascii))
                    .collect();
                format!("[{}]", items.join(","))
            } else {
                let items: Vec<String> = arr
                    .iter()
                    .map(|v| {
                        format!(
                            "{}{}",
                            next_indent,
                            format_json_impl(v, indent, level + 1, ascii)
                        )
                    })
                    .collect();
                format!(
                    "[{}{}{separator}{}]",
                    separator,
                    items.join(&format!(",{}", separator)),
                    current_indent
                )
            }
        }
        OwnedValue::Object(obj) => {
            if obj.is_empty() {
                "{}".to_string()
            } else if compact {
                let items: Vec<String> = obj
                    .iter()
                    .map(|(k, v)| {
                        let key = if ascii {
                            escape_json_string_ascii(k)
                        } else {
                            escape_json_string(k)
                        };
                        format!(
                            "\"{}\":{}",
                            key,
                            format_json_impl(v, indent, level + 1, ascii)
                        )
                    })
                    .collect();
                format!("{{{}}}", items.join(","))
            } else {
                let items: Vec<String> = obj
                    .iter()
                    .map(|(k, v)| {
                        let key = if ascii {
                            escape_json_string_ascii(k)
                        } else {
                            escape_json_string(k)
                        };
                        format!(
                            "\"{}\":{}{}",
                            key,
                            space_after_colon,
                            format_json_impl(v, indent, level + 1, ascii)
                        )
                    })
                    .collect();
                // Add indent before each key
                let indented_items: Vec<String> = items
                    .iter()
                    .map(|item| format!("{}{}", next_indent, item))
                    .collect();
                format!(
                    "{{{}{}{separator}{}}}",
                    separator,
                    indented_items.join(&format!(",{}", separator)),
                    current_indent
                )
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

/// Escape special characters in a JSON string, also escaping non-ASCII as \uXXXX.
fn escape_json_string_ascii(s: &str) -> String {
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
            c if !c.is_ascii() => {
                // Escape non-ASCII characters as \uXXXX
                // For characters outside BMP, use surrogate pairs
                let code = c as u32;
                if code <= 0xFFFF {
                    result.push_str(&format!("\\u{:04x}", code));
                } else {
                    // Surrogate pair for characters above U+FFFF
                    let adjusted = code - 0x10000;
                    let high = 0xD800 + (adjusted >> 10);
                    let low = 0xDC00 + (adjusted & 0x3FF);
                    result.push_str(&format!("\\u{:04x}\\u{:04x}", high, low));
                }
            }
            c => result.push(c),
        }
    }
    result
}

/// ANSI color codes for JSON syntax highlighting.
mod colors {
    pub const RESET: &str = "\x1b[0m";
    pub const NULL: &str = "\x1b[1;30m"; // Bold black (gray)
    pub const BOOL: &str = "\x1b[33m"; // Yellow
    pub const NUMBER: &str = "\x1b[36m"; // Cyan
    pub const STRING: &str = "\x1b[32m"; // Green
    pub const KEY: &str = "\x1b[34m"; // Blue (for object keys)
    pub const BRACKET: &str = "\x1b[1;37m"; // Bold white
}

/// Colorize a JSON string using ANSI escape codes.
/// This is a simple parser that adds colors to JSON tokens.
fn colorize_json(json: &str) -> String {
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
                result.push_str(colors::RESET);
                in_string = false;
            } else {
                result.push(c);
            }
        } else {
            match c {
                '"' => {
                    // Use expecting_key to determine if this is a key
                    if expecting_key {
                        result.push_str(colors::KEY);
                        expecting_key = false; // After seeing key, next string is value
                    } else {
                        result.push_str(colors::STRING);
                    }
                    result.push(c);
                    in_string = true;
                }
                '{' => {
                    result.push_str(colors::BRACKET);
                    result.push(c);
                    result.push_str(colors::RESET);
                    depth_stack.push('{');
                    expecting_key = true; // First thing in object is a key
                }
                '[' => {
                    result.push_str(colors::BRACKET);
                    result.push(c);
                    result.push_str(colors::RESET);
                    depth_stack.push('[');
                    // Arrays don't have keys
                }
                '}' | ']' => {
                    result.push_str(colors::BRACKET);
                    result.push(c);
                    result.push_str(colors::RESET);
                    depth_stack.pop();
                    // Restore expecting_key state based on parent context
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
                't' | 'f' => {
                    // true or false
                    result.push_str(colors::BOOL);
                    result.push(c);
                    // Consume rest of the keyword
                    while let Some(&next) = chars.peek() {
                        if next.is_alphabetic() {
                            result.push(chars.next().unwrap());
                        } else {
                            break;
                        }
                    }
                    result.push_str(colors::RESET);
                }
                'n' => {
                    // null
                    result.push_str(colors::NULL);
                    result.push(c);
                    while let Some(&next) = chars.peek() {
                        if next.is_alphabetic() {
                            result.push(chars.next().unwrap());
                        } else {
                            break;
                        }
                    }
                    result.push_str(colors::RESET);
                }
                '0'..='9' | '-' | '.' | 'e' | 'E' | '+' => {
                    result.push_str(colors::NUMBER);
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
                    result.push_str(colors::RESET);
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
