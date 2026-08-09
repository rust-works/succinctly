//! xq command implementation — command-line XML processor (jq-compatible syntax).
//!
//! Milestone 1 of the `xq` XML query tool (issue #667). Deliberately a much
//! smaller surface than `jq_runner.rs`/`yq_runner.rs`: single-document XML
//! in, JSON-shaped value out, no format conversion, no streaming/lazy-value
//! fast path — just enough to prove `XmlIndex`/`XmlCursor`/`XmlValue` work
//! through the same generic jq evaluator (`succinctly::jq::eval_generic`)
//! that JSON/YAML already use.
//!
//! Wired through `eval_generic::eval_with_cursor_using` (the trait-generic
//! entry point), not `jq::eval` (JSON's hardcoded-`JsonCursor` full
//! evaluator) — mirrors `yq_runner.rs`'s YAML-cursor path, since `XmlCursor`
//! is a new type only the generic evaluator can accept.

use std::io::{IsTerminal, Read, Write};

use anyhow::{Context, Result};

use succinctly::jq::document::{DocumentCursor, DocumentFields, DocumentValue};
use succinctly::jq::eval_generic::{
    eval_with_cursor_using, to_owned as generic_to_owned, GenericResult,
};
use succinctly::jq::{self, JqSemantics, OwnedValue};
use succinctly::xml::XmlIndex;

use crate::env_config::{no_color_from_env, resolve_color, ColorChoice};
use crate::output::{
    colorize_json, exit_codes, format_json, print_build_configuration, ColorScheme, ControlEscape,
    DiagStyle, ErrorSink, FloatStyle, InputLocation, JsonFormatOpts,
};
use crate::XqCommand;

struct OutputConfig {
    indent_string: String,
    raw_output: bool,
    join_output: bool,
    raw_output0: bool,
    ascii_output: bool,
    color_output: bool,
    color_scheme: ColorScheme,
    sort_keys: bool,
}

impl OutputConfig {
    fn from_args(args: &XqCommand) -> Self {
        let indent_string = if args.tab {
            "\t".to_string()
        } else if let Some(n) = args.indent {
            " ".repeat(n as usize)
        } else if args.compact_output {
            String::new()
        } else {
            "  ".to_string()
        };

        let color_output = resolve_color(
            ColorChoice::from_flags(args.monochrome_output, args.color_output),
            no_color_from_env(),
            std::io::stdout().is_terminal(),
        );

        Self {
            indent_string,
            raw_output: args.raw_output || args.join_output || args.raw_output0,
            join_output: args.join_output,
            raw_output0: args.raw_output0,
            ascii_output: args.ascii_output,
            color_output,
            color_scheme: ColorScheme::from_env(),
            sort_keys: args.sort_keys,
        }
    }

    fn format_opts(&self) -> JsonFormatOpts<'_> {
        JsonFormatOpts {
            indent: &self.indent_string,
            sort_keys: self.sort_keys,
            ascii: self.ascii_output,
            float_style: FloatStyle::Shortest,
            control_escape: ControlEscape::Jq,
        }
    }
}

fn write_output<W: Write>(out: &mut W, value: &OwnedValue, config: &OutputConfig) -> Result<()> {
    if config.raw_output {
        if let OwnedValue::String(s) = value {
            out.write_all(s.as_bytes())?;
            return write_terminator(out, config);
        }
    }

    let formatted = format_json(value, &config.format_opts());
    let formatted = if config.color_output {
        colorize_json(&formatted, &config.color_scheme)
    } else {
        formatted
    };
    out.write_all(formatted.as_bytes())?;
    write_terminator(out, config)
}

fn write_terminator<W: Write>(out: &mut W, config: &OutputConfig) -> Result<()> {
    if config.raw_output0 {
        out.write_all(&[0])?;
    } else if !config.join_output {
        out.write_all(b"\n")?;
    }
    Ok(())
}

/// Convert a `GenericResult` to the `OwnedValue`s it produced, reporting an
/// uncaught error/break to `sink` (evaluation continues past one the way jq
/// does — the failure is remembered and turned into an exit code by the
/// caller, matching `jq_runner.rs`'s `ErrorSink` contract).
fn generic_result_values<V: DocumentValue>(
    result: GenericResult<V>,
    sink: &mut ErrorSink,
    at: &InputLocation,
) -> Vec<OwnedValue> {
    match result {
        GenericResult::One(v) => vec![generic_to_owned(&v)],
        GenericResult::OneCursor(c) => vec![generic_to_owned(&c.value())],
        GenericResult::Many(vs) => vs.iter().map(generic_to_owned).collect(),
        GenericResult::ManyCursor(cs) => cs.iter().map(|c| generic_to_owned(&c.value())).collect(),
        // This runner has no lazy-streaming fast path (unlike jq_runner.rs's
        // `LazyKeysArray`), so `keys_unsorted` is always materialized here.
        GenericResult::LazyKeysUnsorted(fields) => vec![OwnedValue::Array(
            fields.keys().into_iter().map(OwnedValue::String).collect(),
        )],
        GenericResult::None => vec![],
        GenericResult::Error(e) => {
            sink.report(DiagStyle::Xq, &e, at);
            vec![]
        }
        GenericResult::Owned(v) => vec![v],
        GenericResult::ManyOwned(vs) => vs,
        GenericResult::Break(label) => {
            sink.report_break(DiagStyle::Xq, &label, at);
            vec![]
        }
        GenericResult::Partial(vs, jq::Control::Error(e)) => {
            sink.report(DiagStyle::Xq, &e, at);
            vs
        }
        GenericResult::Partial(vs, jq::Control::Break(label)) => {
            sink.report_break(DiagStyle::Xq, &label, at);
            vs
        }
    }
}

/// Evaluate `expr` against one XML document.
fn evaluate_input(
    xml_bytes: &[u8],
    expr: &jq::Expr,
    at: &InputLocation,
    sink: &mut ErrorSink,
) -> Vec<OwnedValue> {
    let index = match XmlIndex::build(xml_bytes) {
        Ok(index) => index,
        Err(e) => {
            let err = jq::EvalError {
                message: format!("invalid XML: {e}"),
                value: None,
            };
            sink.report(DiagStyle::Xq, &err, at);
            return vec![];
        }
    };
    let cursor = index.root(xml_bytes);
    let result = eval_with_cursor_using::<JqSemantics, _>(expr, cursor);
    generic_result_values(result, sink, at)
}

/// Evaluate `expr` against jq's `-n` neutral input (`null`) — no XML
/// document to index, so this reuses JSON's own `"null"` bootstrap exactly
/// as `jq_runner.rs`'s own `-n` handling does (`OwnedValue::Null.to_json()`
/// == `"null"`). Format-agnostic: `generic_result_values` only needs *some*
/// `DocumentValue`, and JSON's is already at hand.
fn evaluate_null_input(expr: &jq::Expr, sink: &mut ErrorSink) -> Vec<OwnedValue> {
    let index = succinctly::json::JsonIndex::build(b"null");
    let cursor = index.root(b"null");
    let result = eval_with_cursor_using::<JqSemantics, _>(expr, cursor);
    generic_result_values(result, sink, &InputLocation::unknown())
}

fn get_filter(args: &XqCommand) -> Result<String> {
    if let Some(ref path) = args.from_file {
        std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read filter file: {}", path.display()))
    } else {
        Ok(args.filter.clone().unwrap_or_else(|| ".".to_string()))
    }
}

fn build_named_vars(args: &XqCommand) -> Result<Vec<(String, OwnedValue)>> {
    let mut vars = Vec::new();
    for chunk in args.arg.chunks(2) {
        if let [name, value] = chunk {
            vars.push((name.clone(), OwnedValue::String(value.clone())));
        }
    }
    for chunk in args.argjson.chunks(2) {
        if let [name, value] = chunk {
            let json_value: serde_json::Value = serde_json::from_str(value)
                .with_context(|| format!("Invalid JSON for --argjson {name}"))?;
            vars.push((name.clone(), owned_value_from_json(&json_value)));
        }
    }
    Ok(vars)
}

fn owned_value_from_json(v: &serde_json::Value) -> OwnedValue {
    match v {
        serde_json::Value::Null => OwnedValue::Null,
        serde_json::Value::Bool(b) => OwnedValue::Bool(*b),
        serde_json::Value::Number(n) => n.as_i64().map_or_else(
            || OwnedValue::Float(n.as_f64().unwrap_or(0.0)),
            OwnedValue::Int,
        ),
        serde_json::Value::String(s) => OwnedValue::String(s.clone()),
        serde_json::Value::Array(a) => {
            OwnedValue::Array(a.iter().map(owned_value_from_json).collect())
        }
        serde_json::Value::Object(o) => OwnedValue::Object(
            o.iter()
                .map(|(k, v)| (k.clone(), owned_value_from_json(v)))
                .collect(),
        ),
    }
}

fn read_stdin_bytes() -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    std::io::stdin()
        .read_to_end(&mut buf)
        .context("Failed to read stdin")?;
    Ok(buf)
}

/// Run the xq command.
pub fn run_xq(args: XqCommand) -> Result<i32> {
    if args.version {
        println!(
            "succinctly-xq {} (jq-compatible syntax)",
            env!("CARGO_PKG_VERSION")
        );
        return Ok(exit_codes::SUCCESS);
    }
    if args.build_configuration {
        print_build_configuration("xq");
        return Ok(exit_codes::SUCCESS);
    }

    let filter_str = get_filter(&args)?;
    let expr = jq::parse(&filter_str).map_err(|e| {
        eprintln!("xq: compile error: {e}");
        anyhow::anyhow!("compile error")
    })?;

    let named_vars = build_named_vars(&args)?;
    let var_refs: Vec<(&str, &OwnedValue)> =
        named_vars.iter().map(|(k, v)| (k.as_str(), v)).collect();
    let expr = jq::substitute_vars(&expr, var_refs);

    let output_config = OutputConfig::from_args(&args);
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());

    let mut sink = ErrorSink::default();
    let mut last_output: Option<OwnedValue> = None;
    let mut had_output = false;

    let mut record_results = |results: Vec<OwnedValue>,
                              out: &mut std::io::BufWriter<std::io::StdoutLock>|
     -> Result<()> {
        for result in results {
            had_output = true;
            last_output = Some(result.clone());
            write_output(out, &result, &output_config)?;
        }
        Ok(())
    };

    if args.null_input {
        let results = evaluate_null_input(&expr, &mut sink);
        record_results(results, &mut out)?;
    } else if args.files.is_empty() {
        let bytes = read_stdin_bytes()?;
        let at = InputLocation::at(None, 1);
        let results = evaluate_input(&bytes, &expr, &at, &mut sink);
        record_results(results, &mut out)?;
    } else {
        for f in &args.files {
            let bytes = std::fs::read(f).with_context(|| format!("Failed to read file: {f}"))?;
            let at = InputLocation::at(Some(f), 1);
            let results = evaluate_input(&bytes, &expr, &at, &mut sink);
            record_results(results, &mut out)?;
        }
    }

    out.flush()?;

    if sink.hit() {
        return Ok(DiagStyle::Xq.error_exit_code());
    }
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
