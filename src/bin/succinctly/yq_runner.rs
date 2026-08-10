//! yq-compatible command runner for succinctly.
//!
//! This module implements a yq-compatible CLI interface using the succinctly
//! YAML semi-indexing and jq expression evaluator.

use anyhow::{Context, Result};
use core::fmt::Write as FmtWrite;
use indexmap::IndexMap;
use std::borrow::Cow;
use std::io::{BufWriter, IsTerminal, Read, Write};
use std::path::Path;

use succinctly::jq::document::{DocumentCursor, DocumentFields};
use succinctly::jq::eval_generic::{eval_with_cursor_using, to_owned, GenericResult};
use succinctly::jq::{
    self, sync_aliased_paths, Builtin, Expr, OwnedValue, QueryResult, YqSemantics,
};
use succinctly::json::light::StandardJson;
use succinctly::json::JsonIndex;
use succinctly::yaml::{
    format_float_with_fraction, resolve_plain, resolve_tagged, stream_yaml_sequence,
    ResolvedScalar, YamlCursor, YamlIndex, YamlValue,
};

use super::{InputFormat, OutputFormat, YqCommand};
use crate::output::{
    self, exit_codes, ColorScheme, ControlEscape, DiagStyle, ErrorSink, FloatStyle, InputLocation,
    JsonFormatOpts,
};

/// yq's diagnostics carry no `(at <file>:<line>)` marker, so the yq paths have
/// no location to report — unlike jq, whose marker names the input value (#355).
fn no_location() -> InputLocation {
    InputLocation::unknown()
}

/// Adapter to use `std::io::Write` with `core::fmt::Write` methods.
/// This enables streaming JSON output without intermediate String allocation.
struct FmtWriter<W>(W);

impl<W: Write> core::fmt::Write for FmtWriter<W> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.0.write_all(s.as_bytes()).map_err(|_| core::fmt::Error)
    }
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
        // Shares the jq runner's precedence, documented on `resolve_color`.
        let use_color = crate::env_config::resolve_color(
            crate::env_config::ColorChoice::from_flags(args.monochrome_output, args.color_output),
            crate::env_config::no_color_from_env(),
            std::io::stdout().is_terminal(),
        );

        // Compact output when indent is 0 (yq-compatible)
        let compact = args.indent == 0;

        let indent_str = if compact {
            String::new()
        } else if args.tab {
            "\t".to_string()
        } else {
            " ".repeat(args.indent as usize)
        };

        Self {
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

/// Convert an already-resolved scalar to `OwnedValue`. `str_value` is the
/// original source text, used only for the `Str` case.
fn resolved_scalar_to_owned(resolved: ResolvedScalar, str_value: Cow<'_, str>) -> OwnedValue {
    match resolved {
        ResolvedScalar::Null => OwnedValue::Null,
        ResolvedScalar::Bool(b) => OwnedValue::Bool(b),
        ResolvedScalar::Int(n) => OwnedValue::Int(n),
        ResolvedScalar::Float(f) => OwnedValue::Float(f),
        ResolvedScalar::Str => OwnedValue::String(str_value.into_owned()),
    }
}

/// Convert a YAML value to an OwnedValue for jq evaluation.
///
/// Takes a cursor rather than a bare `YamlValue`: an explicit tag
/// (`!!str`, `!!int`, …) lives on the cursor's `bp_pos`
/// ([`YamlCursor::explicit_tag`]), not on the extracted value, and forces
/// resolution regardless of quoting style — `!!int "5"` converts to the
/// number 5, matching real `yq` (#224). Every recursive call passes a
/// cursor too (`field.value_cursor()`, `YamlElements::uncons_cursor`), so a
/// tag on a nested element is never lost.
fn yaml_to_owned_value<W: AsRef<[u64]>>(cursor: YamlCursor<'_, W>) -> Result<OwnedValue> {
    match cursor.value() {
        YamlValue::String(s) => {
            let str_value = s
                .as_str()
                .map_err(|e| anyhow::anyhow!("invalid YAML string: {e}"))?;

            if let Some(explicit) = cursor.explicit_tag() {
                if let Some(resolved) = resolve_tagged(&str_value, explicit) {
                    return Ok(resolved_scalar_to_owned(resolved, str_value));
                }
            }

            // Quoted strings should always be treated as strings (yq-compatible behavior)
            // Only unquoted scalars should undergo type detection
            if !s.is_unquoted() {
                return Ok(OwnedValue::String(str_value.into_owned()));
            }

            // Resolve plain scalars per the YAML 1.2 core schema
            Ok(resolved_scalar_to_owned(
                resolve_plain(&str_value),
                str_value,
            ))
        }
        YamlValue::Mapping(fields) => {
            let mut map = IndexMap::new();
            for field in fields {
                // Keys follow yq semantics (#222): alias-to-scalar resolves to
                // its content, any other complex key stringifies to "", and the
                // entry is always kept — matching the streaming/DOM emit paths.
                let key = field.key().key_string().into_owned();
                let value = yaml_to_owned_value(field.value_cursor())?;
                map.insert(key, value);
            }
            Ok(OwnedValue::Object(map))
        }
        YamlValue::Sequence(elements) => {
            let mut arr = Vec::new();
            let mut rest = elements;
            while let Some((elem_cursor, next)) = rest.uncons_cursor() {
                arr.push(yaml_to_owned_value(elem_cursor)?);
                rest = next;
            }
            Ok(OwnedValue::Array(arr))
        }
        YamlValue::Alias { target, .. } => {
            // Resolve alias by following the target cursor
            if let Some(target_cursor) = target {
                yaml_to_owned_value(target_cursor)
            } else {
                // Unresolved alias - treat as null
                Ok(OwnedValue::Null)
            }
        }
        YamlValue::Error(msg) => Err(anyhow::anyhow!("YAML error: {msg}")),
        YamlValue::Null => Ok(OwnedValue::Null),
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

/// When `--validate` is set and the resolved input format is YAML, run the
/// opt-in strict validator (`succinctly::yaml::validate`) before indexing and,
/// on the first violation, print a rustc-style diagnostic and return the exit
/// code to bail with. Mirrors `sjq --validate` (`jq_runner::validate_json_input`);
/// JSON input is left to jq-side validation.
fn yaml_validate_guard(
    input: &[u8],
    format: InputFormat,
    validate: bool,
    filename: Option<&str>,
) -> Option<i32> {
    if !validate || !matches!(format, InputFormat::Yaml | InputFormat::Auto) {
        return None;
    }
    match succinctly::yaml::validate::validate(input) {
        Ok(()) => None,
        Err(err) => {
            print_yaml_validation_error(&err, input, filename);
            Some(exit_codes::COMPILE_ERROR)
        }
    }
}

/// Print a YAML validation error with a line/column location and a caret snippet.
fn print_yaml_validation_error(
    err: &succinctly::yaml::validate::YamlValidationError,
    input: &[u8],
    filename: Option<&str>,
) {
    let pos = &err.position;
    eprintln!("yq: validation error: {}", err.kind);
    let location = filename.map_or_else(
        || format!("<stdin>:{}:{}", pos.line, pos.column),
        |f| format!("{}:{}:{}", f, pos.line, pos.column),
    );
    eprintln!("  --> {location}");

    let text = String::from_utf8_lossy(input);
    if let Some(line_content) = text.lines().nth(pos.line.saturating_sub(1)) {
        let width = pos.line.to_string().len().max(3);
        let pad = " ".repeat(width + 2);
        eprintln!("{pad}|");
        eprintln!(" {:>width$} | {}", pos.line, line_content, width = width);
        eprintln!("{}| {}^", pad, " ".repeat(pos.column.saturating_sub(1)));
    }
    eprintln!();
}

/// Detect input format from file extension.
fn detect_format_from_path(path: &Path) -> InputFormat {
    match path.extension().and_then(|e| e.to_str()) {
        Some("json") => InputFormat::Json,
        Some("yaml" | "yml") => InputFormat::Yaml,
        _ => InputFormat::Yaml, // Default to YAML
    }
}

/// Get effective input format, resolving Auto to a specific format.
fn resolve_input_format(format: InputFormat, path: Option<&Path>) -> InputFormat {
    match format {
        InputFormat::Auto => path.map_or(InputFormat::Yaml, detect_format_from_path),
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
                YamlIndex::build(bytes).map_err(|e| anyhow::anyhow!("YAML parse error: {e}"))?;
            let root = index.root(bytes);

            match root.value() {
                YamlValue::Sequence(docs) => {
                    let mut values = Vec::new();
                    let mut rest = docs;
                    while let Some((doc_cursor, next)) = rest.uncons_cursor() {
                        values.push(yaml_to_owned_value(doc_cursor)?);
                        rest = next;
                    }
                    Ok(values)
                }
                // Documents are always wrapped in a virtual root sequence, so
                // this is defensive; `root` itself is this single document's
                // cursor either way.
                _ => Ok(vec![yaml_to_owned_value(root)?]),
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
    sink: &mut ErrorSink,
) -> Result<(Vec<Vec<OwnedValue>>, usize)> {
    let index = YamlIndex::build(bytes).map_err(|e| anyhow::anyhow!("YAML parse error: {e}"))?;
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
                    let results = evaluate_yaml_cursor(cursor, expr, sink)?;
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
                    let results = evaluate_yaml_cursor(content_cursor, expr, sink)?;
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

/// Evaluate a jq expression on an OwnedValue by converting to JSON and back.
///
/// Variables (`--arg`/`--argjson`, `$ARGS`) are substituted into `expr` up
/// front in `run_yq`, so this function needs no evaluation context (#284).
fn evaluate_input(
    input: &OwnedValue,
    expr: &jq::Expr,
    sink: &mut ErrorSink,
) -> Result<Vec<OwnedValue>> {
    // Convert OwnedValue to JSON bytes for indexing
    let json_str = input.to_json();
    let json_bytes = json_str.as_bytes();

    // Build index and evaluate
    let index = JsonIndex::build(json_bytes);
    let cursor = index.root(json_bytes);

    let result = jq::eval::<Vec<u64>, YqSemantics>(expr, cursor);

    // Convert result to Vec<OwnedValue>
    match result {
        QueryResult::One(v) => Ok(vec![standard_json_to_owned(&v)]),
        QueryResult::OneCursor(c) => Ok(vec![standard_json_to_owned(&c.value())]),
        QueryResult::Many(vs) => Ok(vs.iter().map(standard_json_to_owned).collect()),
        QueryResult::None => Ok(vec![]),
        QueryResult::Error(e) => {
            sink.report(DiagStyle::Yq, &e, &no_location());
            Ok(vec![])
        }
        QueryResult::Owned(v) => Ok(vec![v]),
        QueryResult::ManyOwned(vs) => Ok(vs),
        QueryResult::Break(label) => {
            sink.report_break(DiagStyle::Yq, &label, &no_location());
            Ok(vec![])
        }
        // The outputs already produced no longer vanish behind the failure
        // (#400, #494).
        QueryResult::Partial(vs, jq::Control::Error(e)) => {
            sink.report(DiagStyle::Yq, &e, &no_location());
            Ok(vs)
        }
        QueryResult::Partial(vs, jq::Control::Break(label)) => {
            sink.report_break(DiagStyle::Yq, &label, &no_location());
            Ok(vs)
        }
    }
}

/// Whether `expr`'s top-level shape is "rewrite the document at specific
/// paths, leaving everything else identical" -- the class of expression for
/// which comparing a path's value before and after the write is meaningful.
/// Unwraps `Paren`/`Optional` so `(.a = 1)?` still matches, and recurses into
/// `Pipe` so a chain of assignments like `.a = 1 | .b = 2` (the common
/// `yq -i '... | ...'` idiom) matches when every stage does.
///
/// Used to gate the alias-sync post-process (#711): outside this class (a
/// bare `map`, `select`, `.a, .b`, ...) the result document doesn't
/// necessarily share the input's shape at all, so diffing "the same path" in
/// both would be meaningless at best and could clobber it at worst. A pipe
/// with even one non-assign stage (`.a = 1 | select(...)`) is conservatively
/// excluded for the same reason, even though some such stages would in fact
/// preserve paths -- that's left for a future extension, not assumed here.
fn is_alias_sensitive_assign(expr: &Expr) -> bool {
    match expr {
        Expr::Assign { .. }
        | Expr::Update { .. }
        | Expr::CompoundAssign { .. }
        | Expr::AlternativeAssign { .. }
        | Expr::Builtin(Builtin::Del(_)) => true,
        Expr::Paren(inner) | Expr::Optional(inner) => is_alias_sensitive_assign(inner),
        Expr::Pipe(stages) => stages.iter().all(is_alias_sensitive_assign),
        _ => false,
    }
}

/// Walk `cursor`'s document collecting, for every anchor with at least one
/// alias, its definition path and the path of every alias that resolves to
/// it (#711). Paths are plain `String`/`i64` key sequences -- the same shape
/// `sync_aliased_paths` (and jq's own `getpath`/`setpath`) use -- so they
/// line up with coordinates in the `OwnedValue` tree `to_owned` builds from
/// this same document.
///
/// Mirrors `yaml_to_owned_value`'s recursion, including its treatment of
/// merge keys (`<<: *base`): `Mapping`'s `fields` iterator already resolves
/// them transparently, so this walk does too, unchanged. Fixing merge-key/
/// anchor interaction is issue #712's territory, not this one.
fn collect_alias_groups<W: AsRef<[u64]> + Clone>(
    cursor: YamlCursor<'_, W>,
) -> Vec<(Vec<OwnedValue>, Vec<Vec<OwnedValue>>)> {
    let mut defs: IndexMap<String, Vec<OwnedValue>> = IndexMap::new();
    let mut aliases: IndexMap<String, Vec<Vec<OwnedValue>>> = IndexMap::new();
    let mut path = Vec::new();
    walk_alias_groups(cursor, &mut path, &mut defs, &mut aliases);
    defs.into_iter()
        .filter_map(|(name, def_path)| Some((def_path, aliases.swap_remove(&name)?)))
        .collect()
}

fn walk_alias_groups<W: AsRef<[u64]> + Clone>(
    cursor: YamlCursor<'_, W>,
    path: &mut Vec<OwnedValue>,
    defs: &mut IndexMap<String, Vec<OwnedValue>>,
    aliases: &mut IndexMap<String, Vec<Vec<OwnedValue>>>,
) {
    // `anchor()` returns `None` for alias nodes, so an alias is never
    // mistaken for its own definition here.
    if let Some(name) = cursor.anchor() {
        defs.insert(name.to_string(), path.clone());
    }
    match cursor.value() {
        YamlValue::Mapping(fields) => {
            for field in fields {
                path.push(OwnedValue::String(field.key().key_string().into_owned()));
                walk_alias_groups(field.value_cursor(), path, defs, aliases);
                path.pop();
            }
        }
        YamlValue::Sequence(elements) => {
            let mut idx = 0i64;
            let mut rest = elements;
            while let Some((elem_cursor, next_rest)) = rest.uncons_cursor() {
                path.push(OwnedValue::Int(idx));
                walk_alias_groups(elem_cursor, path, defs, aliases);
                path.pop();
                idx += 1;
                rest = next_rest;
            }
        }
        YamlValue::Alias { anchor_name, .. } => {
            aliases
                .entry(anchor_name.to_string())
                .or_default()
                .push(path.clone());
        }
        _ => {}
    }
}

/// Evaluate a jq expression directly on a YAML cursor.
///
/// This uses the generic evaluator to preserve position metadata (line/column).
fn evaluate_yaml_cursor<W: AsRef<[u64]> + Clone>(
    cursor: YamlCursor<'_, W>,
    expr: &Expr,
    sink: &mut ErrorSink,
) -> Result<Vec<OwnedValue>> {
    // Snapshot alias-sync context from the pristine document *before*
    // evaluation, only when it could possibly matter (#711): an
    // assignment-family expression against a document that actually has
    // aliases. Everything else (JSON, plain reads, alias-free YAML) pays
    // nothing beyond this one bool check.
    let alias_sync_ctx = (is_alias_sensitive_assign(expr) && cursor.index().has_aliases())
        .then(|| (to_owned(&cursor.value()), collect_alias_groups(cursor)));

    let result = eval_with_cursor_using::<YqSemantics, _>(expr, cursor);

    // Convert GenericResult to Vec<OwnedValue>
    let mut docs = match result {
        GenericResult::One(v) => Ok(vec![to_owned(&v)]),
        GenericResult::OneCursor(c) => Ok(vec![to_owned(&c.value())]),
        GenericResult::Many(vs) => Ok(vs.iter().map(to_owned).collect()),
        GenericResult::ManyCursor(cs) => Ok(cs.iter().map(|c| to_owned(&c.value())).collect()),
        // This is the DOM/slow path (`evaluate_yaml_direct_filtered`'s
        // fallback), reached only when `can_use_m2_streaming` rejects the
        // expression or a flag (`--sort-keys`, color, `--tab`, `--slurp`,
        // `--null-input`, named vars, ...) forces DOM output for every query
        // shape, not just `keys_unsorted`. `syq 'keys_unsorted'` under
        // default flags takes the M2 fast path instead, which streams each
        // key from `fields` without materializing (#685); this arm stays a
        // plain materializing fallback since the DOM path materializes
        // everything else here too. Sort iff `sorted` (#683) -- though in
        // practice `sorted` is always `false` here: `run_yq` always parses
        // in `ParserMode::Yq`, where the `keys` keyword itself resolves to
        // `Builtin::KeysUnsorted` (matching real yq's document-order
        // semantics, see `parser.rs`'s `keys`/`keys_unsorted` handling), so
        // `Builtin::Keys` can never reach this arm through the `yq` CLI.
        // Handled anyway for exhaustiveness and because the generic
        // evaluator is shared with `jq` (#140's `Pipe` dispatch is generic
        // over `V: DocumentValue`).
        GenericResult::LazyKeys { fields, sorted } => {
            let mut keys = fields.keys();
            if sorted {
                keys.sort();
            }
            Ok(vec![OwnedValue::Array(
                keys.into_iter().map(OwnedValue::String).collect(),
            )])
        }
        // Same reasoning as `LazyKeys` above, for array `keys`/
        // `keys_unsorted` (#684).
        GenericResult::LazyIndexRange(len) => Ok(vec![OwnedValue::Array(
            (0..len).map(|i| OwnedValue::Int(i as i64)).collect(),
        )]),
        GenericResult::None => Ok(vec![]),
        GenericResult::Error(e) => {
            sink.report(DiagStyle::Yq, &e, &no_location());
            Ok(vec![])
        }
        GenericResult::Owned(v) => Ok(vec![v]),
        GenericResult::ManyOwned(vs) => Ok(vs),
        GenericResult::Break(label) => {
            sink.report_break(DiagStyle::Yq, &label, &no_location());
            Ok(vec![])
        }
        // The outputs already produced no longer vanish behind the failure
        // (#400, #494).
        GenericResult::Partial(vs, jq::Control::Error(e)) => {
            sink.report(DiagStyle::Yq, &e, &no_location());
            Ok(vs)
        }
        GenericResult::Partial(vs, jq::Control::Break(label)) => {
            sink.report_break(DiagStyle::Yq, &label, &no_location());
            Ok(vs)
        }
    };

    if let Some((pristine, groups)) = &alias_sync_ctx {
        if let Ok(docs) = &mut docs {
            for doc in docs.iter_mut() {
                sync_aliased_paths(doc, pristine, groups);
            }
        }
    }

    docs
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

/// Write the `---` document separator for fast-path YAML output (#175).
///
/// yq separates YAML documents with `---` only between documents that produce
/// output: never before the first, and never around a document whose query
/// yields no results. `will_output` says whether the current document is about
/// to emit anything; `streamed` records that an earlier document already did.
fn emit_yaml_doc_separator<W: std::io::Write>(
    writer: &mut W,
    streamed: &mut bool,
    will_output: bool,
) -> std::io::Result<()> {
    if *streamed && will_output {
        writeln!(writer, "---")?;
    }
    *streamed |= will_output;
    Ok(())
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
            write!(writer, "{s}")?;
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
            write!(writer, "{output}")?;
        }
        write_terminator(writer, config)?;
        return Ok(());
    }

    // JSON output format. Both compact and pretty route through the shared
    // formatter with yq's control-char escaping so the two agree byte-for-byte
    // on control characters — `\u0008`/`\u000c` (not jq's `\b`/`\f`) and raw
    // DEL/C1 controls — matching `mikefarah/yq` and the M2 streaming fast path
    // (#262). Compact keeps jq-shortest floats (e.g. `1`) to match the streaming
    // path; pretty preserves whole floats (e.g. `1.0`).
    let json_str = output::format_json(
        value,
        &JsonFormatOpts {
            indent: if config.compact {
                ""
            } else {
                &config.indent_str
            },
            sort_keys: config.sort_keys,
            ascii: config.ascii_output,
            float_style: if config.compact {
                FloatStyle::Shortest
            } else {
                FloatStyle::PreserveWholeFloat
            },
            control_escape: ControlEscape::Yq,
        },
    );

    if config.use_color {
        write!(
            writer,
            "{}",
            output::colorize_json(&json_str, &ColorScheme::default())
        )?;
    } else {
        write!(writer, "{json_str}")?;
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
            } else {
                format_float_with_fraction(*f)
            }
        }
        OwnedValue::NumberLiteral(..) => {
            if value.as_f64().is_some_and(f64::is_nan) {
                ".nan".to_string()
            } else if value.as_f64().is_some_and(f64::is_infinite) {
                if value.as_f64() > Some(0.0) {
                    ".inf".to_string()
                } else {
                    "-.inf".to_string()
                }
            } else {
                value.number_str().expect("numeric variant").into_owned()
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
                            format!("{indent}-\n{item}")
                        } else {
                            format!("{indent}- {item}")
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
                        format!("{key}: {val}")
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
                            format!("{indent}{key}:\n{val}")
                        } else {
                            let val = emit_yaml_value(v, config, depth + 1, false);
                            format!("{indent}{key}: {val}")
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

/// Parse a `--argjson` value into an `OwnedValue`.
///
/// Validates strictly (RFC 8259) via `serde_json`, matching jq's `--argjson`
/// (see `jq_runner::parse_json_value`). The lenient JSON semi-index would
/// otherwise silently coerce malformed input (e.g. `42 garbage` → `42`)
/// instead of surfacing an error (#284).
fn parse_json_value(s: &str) -> Result<OwnedValue> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(OwnedValue::Null);
    }
    let value: serde_json::Value =
        serde_json::from_str(s).with_context(|| format!("invalid JSON: {s}"))?;
    Ok(serde_json_to_owned(&value))
}

/// Convert a `serde_json::Value` into an `OwnedValue`
/// (mirrors `jq_runner::serde_to_owned`).
fn serde_json_to_owned(value: &serde_json::Value) -> OwnedValue {
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
            OwnedValue::Array(arr.iter().map(serde_json_to_owned).collect())
        }
        serde_json::Value::Object(obj) => OwnedValue::Object(
            obj.iter()
                .map(|(k, v)| (k.clone(), serde_json_to_owned(v)))
                .collect(),
        ),
    }
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
                .with_context(|| format!("invalid JSON for --argjson {name}"))?;
            context.named.insert(name, value);
        }
    }

    Ok(context)
}

/// Build the `$ARGS` special variable (`{named, positional}`), mirroring
/// `jq_runner::build_args_var`.
///
/// yq has no positional-argument flags (`--args`/`--jsonargs`), so `positional`
/// is always an empty array; `named` carries the `--arg`/`--argjson` values.
fn build_args_var(context: &EvalContext) -> OwnedValue {
    let mut args_obj = IndexMap::new();
    args_obj.insert(
        "named".to_string(),
        OwnedValue::Object(context.named.clone()),
    );
    args_obj.insert("positional".to_string(), OwnedValue::Array(Vec::new()));
    OwnedValue::Object(args_obj)
}

/// Check if an expression can use M2 streaming path.
///
/// M2 streaming is used for simple navigation expressions that produce
/// cursor results without requiring OwnedValue construction:
/// - Identity: `.`
/// - Field access: `.field`
/// - Index access: `.[0]`, `.[-1]`
/// - Iteration: `.[]`
/// - Chained navigation: `.field[0].name`
/// - Optional variants: `.field?`, `.[0]?`, `.[]?`
/// - `keys_unsorted` (streams lazily via `GenericResult::LazyKeys { sorted: false, .. }`, #685)
///
/// Expressions that require OwnedValue construction cannot use M2:
/// - Builtins like `length`, `keys` (sorted), `map`
/// - Array/object construction: `[...]`, `{...}`
/// - Arithmetic, comparison, and logic operators
/// - String interpolation
/// - Variables and function calls
fn can_use_m2_streaming(expr: &Expr) -> bool {
    match expr {
        // Core M2 expressions
        Expr::Identity => true,
        Expr::Field(_) => true,
        Expr::Index(_) => true,
        Expr::Iterate => true,

        // Chained navigation
        Expr::Pipe(exprs) => exprs.iter().all(can_use_m2_streaming),

        // Optional variants
        Expr::Optional(inner) => can_use_m2_streaming(inner),

        // Parentheses don't affect streamability
        Expr::Paren(inner) => can_use_m2_streaming(inner),

        // first(f)/last(f) (both AST spellings the parser produces, see
        // `Expr::FirstExpr`/`LastExpr` doc comments) and computed indexing
        // `.[(expr)]` all thread a cursor through natively in
        // `eval_generic.rs` (#607), so their `GenericResult` streams exactly
        // like plain navigation instead of needing OwnedValue construction.
        // Streaming through `eval_with_cursor_using` here (rather than
        // `evaluate_yaml_cursor`'s unconditional `to_owned()` DOM path) is
        // also what keeps duplicate mapping keys intact for these shapes,
        // matching `.[0]` on the same input (#631).
        Expr::FirstExpr(_) | Expr::LastExpr(_) => true,
        Expr::Builtin(Builtin::FirstStream(_) | Builtin::LastStream(_)) => true,
        Expr::IndexExpr { .. } => true,

        // `keys_unsorted` on a mapping produces `GenericResult::LazyKeys { sorted: false, .. }`,
        // which `GenericResult::stream_json`/`stream_yaml` now stream directly
        // from the field cursor (#685) instead of materializing a `Vec<String>`
        // first. On an array input it already returns `GenericResult::Owned`
        // cheaply, so this only changes routing for the mapping case.
        Expr::Builtin(Builtin::KeysUnsorted) => true,

        // Everything else requires OwnedValue
        _ => false,
    }
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
            | Builtin::CombinationsN(e)
            | Builtin::Tz(e)
            | Builtin::Load(e) => contains_split_doc(e),
            Builtin::RecurseCond(e1, e2)
            | Builtin::SetPath(e1, e2)
            | Builtin::Pow(e1, e2)
            | Builtin::Atan2(e1, e2)
            | Builtin::Limit(e1, e2)
            | Builtin::NthStream(e1, e2)
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
            Builtin::SubFlags(e1, e2, e3) | Builtin::GsubFlags(e1, e2, e3) => {
                contains_split_doc(e1) || contains_split_doc(e2) || contains_split_doc(e3)
            }
            _ => false,
        },
        // Terminal expressions that cannot contain split_doc
        // Both halves hold sub-expressions, so `split_doc` can hide in either.
        Expr::IndexExpr { target, key } => contains_split_doc(target) || contains_split_doc(key),
        // Same reasoning as `IndexExpr`: `split_doc` can hide in the target
        // or either bound.
        Expr::SliceExpr { target, start, end } => {
            contains_split_doc(target)
                || start.as_deref().is_some_and(contains_split_doc)
                || end.as_deref().is_some_and(contains_split_doc)
        }
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

/// Get input files from arguments.
fn get_input_files(args: &YqCommand) -> Vec<String> {
    // When --from-file is used, the 'filter' field becomes the first input file
    // because the filter comes from a file instead of command line
    let mut files = Vec::new();

    if args.from_file.is_some() {
        // When --from-file is used, the first positional arg (if any) is an input file
        if let Some(ref first_file) = args.filter {
            files.push(first_file.clone());
        }
    }

    // Add remaining files
    files.extend(args.files.iter().cloned());

    files
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
        output::print_build_configuration("yq");
        return Ok(exit_codes::SUCCESS);
    }

    // Get the filter expression
    let filter_str = if let Some(ref path) = args.from_file {
        std::fs::read_to_string(path)
            .with_context(|| format!("failed to read filter file: {}", path.display()))?
    } else {
        args.filter.clone().unwrap_or_else(|| ".".to_string())
    };

    // Get input files (with --from-file, the filter positional is an input file)
    let input_files = get_input_files(&args);

    // Validate flag compatibility
    if args.document.is_some() && args.raw_input {
        anyhow::bail!("--doc and --raw-input are incompatible");
    }

    // Parse the jq program (use Yq mode for extended identifier syntax like kebab-case)
    let mut program = jq::parse_program_with_mode(&filter_str, jq::ParserMode::Yq)
        .map_err(|e| anyhow::anyhow!("parse error: {e}"))?;

    // Parse variables
    let context = parse_variables(&args)?;

    // Substitute named variables (--arg/--argjson) and the $ARGS special
    // variable into the expression AST before evaluating, mirroring the jq
    // runner (see jq_runner.rs). Without this, filter references like `$g`
    // error as "undefined variable" even though the values were parsed (#284).
    {
        let args_value = build_args_var(&context);
        let mut all_vars: Vec<(&str, &OwnedValue)> =
            context.named.iter().map(|(k, v)| (k.as_str(), v)).collect();
        all_vars.push(("ARGS", &args_value));
        program.expr = jq::substitute_vars(&program.expr, all_vars);
    }

    // Output configuration
    let output_config = OutputConfig::from_args(&args);

    // Set up output
    let stdout = std::io::stdout();
    let mut writer = BufWriter::new(stdout.lock());

    // yq --exit-status semantics: exit 1 unless some result is truthy.
    // Unlike jq (which inspects only the last output value), yq treats empty
    // output and all-falsy output alike as "no matches found".
    let mut any_truthy = false;

    // Uncaught evaluation errors. Evaluation continues past one, so the failure
    // is remembered here and turned into yq's exit 1 below (#355).
    let mut sink = ErrorSink::default();

    // Check if expression contains split_doc - if so, each result is a separate document
    let has_split_doc = contains_split_doc(&program.expr);

    // M2/M2.5 streaming fast path: navigation queries stream results
    // directly from the document cursor, skipping the OwnedValue DOM (and
    // its IndexMap, which cannot represent duplicate mapping keys — #442).
    // Compact output always qualified; indented (pretty) output now does
    // too, since both the YAML- and JSON-target streamers are indent-aware.
    // This avoids building OwnedValue DOM for:
    // - Identity: `.`
    // - Field access: `.field`
    // - Index access: `.[0]`
    // - Iteration: `.[]`
    // - Chained navigation: `.field[0].name`
    //
    // Supports both JSON and YAML output formats.
    let is_identity = matches!(program.expr, Expr::Identity);
    let is_m2_streamable = can_use_m2_streaming(&program.expr);
    // sort_keys and color aren't implemented by the cursor streamers, and
    // tab indentation needs a string-based indent unit they don't accept
    // yet — all three fall back to the DOM path, unchanged, rather than
    // silently ignoring the flag the way compact mode already does today.
    let can_stream_pretty = !args.sort_keys && !output_config.use_color && !args.tab;
    let can_json_fast_path = is_m2_streamable
        && (output_config.compact || (can_stream_pretty && !args.ascii_output))
        && output_config.output_format == OutputFormat::Json
        && !args.null_input
        && !args.raw_input
        && !args.slurp
        && !args.inplace
        && context.named.is_empty();
    let can_yaml_fast_path = is_m2_streamable
        && (output_config.compact || can_stream_pretty)
        && output_config.output_format == OutputFormat::Yaml
        && !args.null_input
        && !args.raw_input
        && !args.slurp
        && !args.inplace
        && context.named.is_empty();
    let can_fast_path = can_json_fast_path || can_yaml_fast_path;

    // `--inplace`'s own copy of the M2 gate (#478): identical conditions to
    // `can_json_fast_path`/`can_yaml_fast_path` above, but requiring
    // `args.inplace` instead of excluding it. Kept as a separate pair rather
    // than folding into the gate above because inplace output targets a
    // per-file buffer (then `fs::write`), not the shared stdout `writer`
    // the block below uses — the two loops have different write targets and
    // per-file `---` separator resets, so they stay as distinct branches
    // that happen to share the same underlying `stream_cursor!` macro.
    let can_inplace_json_fast_path = is_m2_streamable
        && (output_config.compact || (can_stream_pretty && !args.ascii_output))
        && output_config.output_format == OutputFormat::Json
        && !args.null_input
        && !args.raw_input
        && args.inplace
        && context.named.is_empty();
    let can_inplace_yaml_fast_path = is_m2_streamable
        && (output_config.compact || can_stream_pretty)
        && output_config.output_format == OutputFormat::Yaml
        && !args.null_input
        && !args.raw_input
        && args.inplace
        && context.named.is_empty();
    let can_inplace_fast_path = can_inplace_json_fast_path || can_inplace_yaml_fast_path;

    // `--slurp`'s fast path (#478) is narrower than the two gates above:
    // scoped to plain identity only (`is_identity`, not the broader
    // `is_m2_streamable` set), since a non-trivial filter over the slurped
    // array needs real evaluation. `-o json --slurp` still uses the slow
    // DOM path below — an explicit, documented scope limit rather than a
    // silent gap, matching `sort_keys`/color/tab already being excluded
    // from the gates above.
    let can_slurp_fast_path = is_identity
        && can_stream_pretty
        && output_config.output_format == OutputFormat::Yaml
        && !args.null_input
        && !args.raw_input
        && context.named.is_empty();

    // Indent width for the fast path's streamers. YAML's `-I0` is a special
    // case: real `yq` treats it as "use the library default" (4 spaces),
    // and succinctly's existing (pre-#442) compact-YAML fast path hardcodes
    // 2 regardless of `-I` — preserved as-is here since reconciling that
    // mismatch is a separate, out-of-scope issue. Every other value threads
    // through directly, matching what the DOM path already produces
    // (verified against `-I1` through `-I6`). JSON has no such quirk: `-I0`
    // means compact/flow for both real yq and succinctly today.
    let yaml_indent_spaces: usize = if args.indent == 0 {
        2
    } else {
        args.indent as usize
    };
    let json_indent_spaces: usize = args.indent as usize;

    // Helper macro to stream cursor results (avoiding closure borrow issues).
    // Defined here (rather than inside the `if can_fast_path` block below) so
    // both the stdout M2 path and `--inplace`'s fast path (#478) can reuse
    // it. `$is_yaml` is threaded through explicitly rather than closing over
    // `can_yaml_fast_path` by name, and `yaml_doc_streamed`/`any_truthy`/
    // `sink` resolve, at each call site, to whichever same-named local is in
    // scope there — each branch below declares its own `yaml_doc_streamed`.
    macro_rules! stream_cursor {
            ($cursor:expr, $writer:expr, $is_yaml:expr, $doc_streamed:expr) => {{
                if $is_yaml {
                    // M2 YAML path: YAML output streaming
                    if is_identity {
                        // P9 path: stream directly without evaluation
                        emit_yaml_doc_separator($writer, $doc_streamed, true)?;
                        $cursor
                            .stream_yaml(&mut FmtWriter($writer), yaml_indent_spaces)
                            .map_err(|_| anyhow::anyhow!("Write error"))?;
                        writeln!($writer)?;
                        // Streaming skips evaluation, so inspect the document
                        // value directly to keep `-e` falsy tracking (#178).
                        if args.exit_status {
                            any_truthy |= !$cursor.is_falsy();
                        }
                    } else {
                        // M2 YAML path: evaluate and stream YAML results
                        let result = eval_with_cursor_using::<YqSemantics, _>(&program.expr, $cursor);
                        let will_output = !matches!(
                            &result,
                            GenericResult::None | GenericResult::Break(_)
                        ) && !matches!(&result, GenericResult::Many(vs) if vs.is_empty())
                            && !matches!(&result, GenericResult::ManyCursor(cs) if cs.is_empty())
                            && !matches!(&result, GenericResult::ManyOwned(vs) if vs.is_empty());
                        emit_yaml_doc_separator($writer, $doc_streamed, will_output)?;
                        let stats = result
                            .stream_yaml(&mut FmtWriter($writer), yaml_indent_spaces, |w| {
                                w.write_str("\n")
                            })
                            .map_err(|_| anyhow::anyhow!("Write error"))?;
                        any_truthy |= stats.any_truthy;
                        // Streaming never writes a diagnostic to stdout; it hands
                        // the error back here so it reaches stderr and fails the run (#355).
                        if let Some(err) = &stats.error {
                            sink.report_stream(DiagStyle::Yq, err, &no_location());
                        }
                    }
                } else {
                    // M2 path: JSON output streaming
                    if is_identity {
                        // P9 path: stream directly without evaluation
                        $cursor
                            .stream_json(&mut FmtWriter($writer), json_indent_spaces)
                            .map_err(|_| anyhow::anyhow!("Write error"))?;
                        writeln!($writer)?;
                        // Streaming skips evaluation, so inspect the document
                        // value directly to keep `-e` falsy tracking (#178).
                        if args.exit_status {
                            any_truthy |= !$cursor.is_falsy();
                        }
                    } else {
                        // M2 path: evaluate and stream results
                        let result = eval_with_cursor_using::<YqSemantics, _>(&program.expr, $cursor);
                        let stats = result
                            .stream_json(&mut FmtWriter($writer), json_indent_spaces, |w| {
                                w.write_str("\n")
                            })
                            .map_err(|_| anyhow::anyhow!("Write error"))?;
                        any_truthy |= stats.any_truthy;
                        // Streaming never writes a diagnostic to stdout; it hands
                        // the error back here so it reaches stderr and fails the run (#355).
                        if let Some(err) = &stats.error {
                            sink.report_stream(DiagStyle::Yq, err, &no_location());
                        }
                    }
                }
            }};
        }

    if can_fast_path {
        // M2 streaming fast path: evaluate expression and stream results directly
        // Track global document index across all files for --doc filtering
        let mut global_doc_index: usize = 0;
        // Whether any document has produced YAML output yet — drives `---`
        // separator placement between documents (#175).
        let mut yaml_doc_streamed = false;

        if input_files.is_empty() {
            let yaml_bytes = read_stdin()?;
            let fmt = resolve_input_format(args.input_format, None);
            if let Some(code) = yaml_validate_guard(&yaml_bytes, fmt, args.validate, None) {
                return Ok(code);
            }
            let index = YamlIndex::build(&yaml_bytes)
                .map_err(|e| anyhow::anyhow!("YAML parse error: {e}"))?;
            let root = index.root(&yaml_bytes);

            // Output each document using M2 streaming
            match root.value() {
                YamlValue::Sequence(mut docs) => {
                    while let Some((cursor, rest)) = docs.uncons_cursor() {
                        // Apply --doc filter if specified
                        let should_process = args
                            .document
                            .map_or(true, |target| global_doc_index == target);
                        if should_process {
                            stream_cursor!(
                                cursor,
                                &mut writer,
                                can_yaml_fast_path,
                                &mut yaml_doc_streamed
                            );
                        }
                        global_doc_index += 1;
                        docs = rest;
                    }
                }
                _ => {
                    // Single document case. Defensive fallback only: the root
                    // cursor (bp_pos 0) always reports the virtual document
                    // sequence, so documents — including #175 `---` separator
                    // handling — go through the Sequence arm above.
                    if args.document.is_none() || args.document == Some(0) {
                        if is_identity {
                            // P9 path for identity on single doc
                            if can_yaml_fast_path {
                                root.stream_yaml_document(
                                    &mut FmtWriter(&mut writer),
                                    yaml_indent_spaces,
                                )
                                .map_err(|_| anyhow::anyhow!("Write error"))?;
                            } else {
                                root.stream_json_document(
                                    &mut FmtWriter(&mut writer),
                                    json_indent_spaces,
                                )
                                .map_err(|_| anyhow::anyhow!("Write error"))?;
                            }
                            writeln!(writer)?;
                            // `root` is the virtual document sequence; falsiness
                            // lives on the actual document value (#178).
                            if args.exit_status {
                                any_truthy |= root.first_child().is_some_and(|c| !c.is_falsy());
                            }
                        } else {
                            // M2 path: need to get the actual document cursor
                            if let Some(doc_cursor) = root.first_child() {
                                stream_cursor!(
                                    doc_cursor,
                                    &mut writer,
                                    can_yaml_fast_path,
                                    &mut yaml_doc_streamed
                                );
                            }
                        }
                    }
                }
            }
        } else {
            for file_path in &input_files {
                let path = Path::new(file_path);
                let yaml_bytes = read_file(path)?;
                let fmt = resolve_input_format(args.input_format, Some(path));
                if let Some(code) =
                    yaml_validate_guard(&yaml_bytes, fmt, args.validate, Some(file_path))
                {
                    return Ok(code);
                }
                let index = YamlIndex::build(&yaml_bytes)
                    .map_err(|e| anyhow::anyhow!("YAML parse error in {file_path}: {e}"))?;
                let root = index.root(&yaml_bytes);

                match root.value() {
                    YamlValue::Sequence(mut docs) => {
                        while let Some((cursor, rest)) = docs.uncons_cursor() {
                            // Apply --doc filter if specified
                            let should_process = args
                                .document
                                .map_or(true, |target| global_doc_index == target);
                            if should_process {
                                stream_cursor!(
                                    cursor,
                                    &mut writer,
                                    can_yaml_fast_path,
                                    &mut yaml_doc_streamed
                                );
                            }
                            global_doc_index += 1;
                            docs = rest;
                        }
                    }
                    _ => {
                        // Single document case. Defensive fallback only: the
                        // root cursor (bp_pos 0) always reports the virtual
                        // document sequence, so documents — including #175
                        // `---` separator handling — go through the Sequence
                        // arm above.
                        let should_process = args
                            .document
                            .map_or(true, |target| global_doc_index == target);
                        if should_process {
                            if is_identity {
                                // P9 path for identity on single doc
                                if can_yaml_fast_path {
                                    root.stream_yaml_document(
                                        &mut FmtWriter(&mut writer),
                                        yaml_indent_spaces,
                                    )
                                    .map_err(|_| anyhow::anyhow!("Write error"))?;
                                } else {
                                    root.stream_json_document(
                                        &mut FmtWriter(&mut writer),
                                        json_indent_spaces,
                                    )
                                    .map_err(|_| anyhow::anyhow!("Write error"))?;
                                }
                                writeln!(writer)?;
                                // `root` is the virtual document sequence; falsiness
                                // lives on the actual document value (#178).
                                if args.exit_status {
                                    any_truthy |= root.first_child().is_some_and(|c| !c.is_falsy());
                                }
                            } else {
                                // M2 path: need to get the actual document cursor
                                if let Some(doc_cursor) = root.first_child() {
                                    stream_cursor!(
                                        doc_cursor,
                                        &mut writer,
                                        can_yaml_fast_path,
                                        &mut yaml_doc_streamed
                                    );
                                }
                            }
                        }
                        global_doc_index += 1;
                    }
                }
            }
        }
    } else if args.null_input {
        // Handle --null-input
        let mut split_doc_state = SplitDocState::new(has_split_doc);
        let results = evaluate_input(&OwnedValue::Null, &program.expr, &mut sink)?;
        for result in results {
            split_doc_state.write_separator(&mut writer, &output_config)?;
            any_truthy |= !matches!(&result, OwnedValue::Null | OwnedValue::Bool(false));
            output_value(&mut writer, &result, &output_config)?;
        }
    } else if args.raw_input {
        // Handle --raw-input: read each line as a string instead of parsing as YAML
        let input_content = if input_files.is_empty() {
            read_stdin_string()?
        } else {
            let mut content = String::new();
            for file_path in &input_files {
                let file_content = std::fs::read_to_string(file_path)
                    .with_context(|| format!("failed to read file: {file_path}"))?;
                content.push_str(&file_content);
            }
            content
        };

        let mut split_doc_state = SplitDocState::new(has_split_doc);
        if args.slurp {
            // yq -R -s (jq semantics): the entire input (all files
            // concatenated) becomes a single string; no line splitting and
            // no array wrap.
            let slurped = OwnedValue::String(input_content);
            let results = evaluate_input(&slurped, &program.expr, &mut sink)?;
            for result in results {
                split_doc_state.write_separator(&mut writer, &output_config)?;
                any_truthy |= !matches!(&result, OwnedValue::Null | OwnedValue::Bool(false));
                output_value(&mut writer, &result, &output_config)?;
            }
        } else {
            // Without --slurp, process each line independently
            for line in input_content.lines() {
                let input = OwnedValue::String(line.to_string());
                let results = evaluate_input(&input, &program.expr, &mut sink)?;
                for result in results {
                    split_doc_state.write_separator(&mut writer, &output_config)?;
                    any_truthy |= !matches!(&result, OwnedValue::Null | OwnedValue::Bool(false));
                    output_value(&mut writer, &result, &output_config)?;
                }
            }
        }
    } else if args.slurp {
        // Handle --slurp: collect all documents from all inputs into an array

        // Collect input sources
        let input_sources: Vec<(Vec<u8>, InputFormat)> = if input_files.is_empty() {
            let input_bytes = read_stdin()?;
            let format = resolve_input_format(args.input_format, None);
            if let Some(code) = yaml_validate_guard(&input_bytes, format, args.validate, None) {
                return Ok(code);
            }
            vec![(input_bytes, format)]
        } else {
            let mut sources = Vec::new();
            for file_path in &input_files {
                let path = Path::new(file_path);
                let input_bytes = read_file(path)?;
                let format = resolve_input_format(args.input_format, Some(path));
                if let Some(code) =
                    yaml_validate_guard(&input_bytes, format, args.validate, Some(file_path))
                {
                    return Ok(code);
                }
                sources.push((input_bytes, format));
            }
            sources
        };

        if can_slurp_fast_path {
            // M2 streaming fast path (#478): stream each source's document
            // cursor(s) directly into one combined YAML sequence, skipping
            // the OwnedValue DOM. `evaluate_input`'s JSON round-trip below
            // would otherwise re-collapse duplicate mapping keys even if the
            // initial conversion into it didn't (the array-builder step
            // has its own `IndexMap`-backed collapse).
            //
            // Two-phase: parse every source into an owned `(bytes, YamlIndex)`
            // pair first, so all sources stay alive together — `YamlCursor`
            // borrows both `text` and `index` with the same lifetime, so
            // cursors from different sources can't be collected into one
            // `Vec` unless their backing bytes/index all outlive that `Vec`.
            let mut parsed_sources: Vec<(Vec<u8>, YamlIndex<Vec<u64>>)> =
                Vec::with_capacity(input_sources.len());
            for (bytes, _format) in input_sources {
                let index = YamlIndex::build(&bytes)
                    .map_err(|e| anyhow::anyhow!("YAML parse error: {e}"))?;
                parsed_sources.push((bytes, index));
            }

            let mut cursors = Vec::new();
            let mut global_doc_index: usize = 0;
            for (bytes, index) in &parsed_sources {
                let root = index.root(bytes);
                match root.value() {
                    YamlValue::Sequence(mut docs) => {
                        while let Some((cursor, rest)) = docs.uncons_cursor() {
                            let should_process = args
                                .document
                                .map_or(true, |target| global_doc_index == target);
                            if should_process {
                                cursors.push(cursor);
                            }
                            global_doc_index += 1;
                            docs = rest;
                        }
                    }
                    _ => {
                        // Defensive fallback only, matching the stdout/inplace
                        // M2 paths: the root cursor always reports the virtual
                        // document sequence, so real documents go through the
                        // Sequence arm above.
                        let should_process = args
                            .document
                            .map_or(true, |target| global_doc_index == target);
                        if should_process {
                            if let Some(doc_cursor) = root.first_child() {
                                cursors.push(doc_cursor);
                            }
                        }
                        global_doc_index += 1;
                    }
                }
            }

            if args.exit_status {
                // `--slurp '.'` always yields exactly one (array) result, and
                // a non-empty array is truthy regardless of its elements —
                // matching jq/yq `-e` semantics for arrays.
                any_truthy = true;
            }
            stream_yaml_sequence(
                cursors.iter().copied(),
                &mut FmtWriter(&mut writer),
                0,
                yaml_indent_spaces,
            )
            .map_err(|_| anyhow::anyhow!("Write error"))?;
            writeln!(writer)?;
        } else {
            let mut all_docs: Vec<OwnedValue> = Vec::new();

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
            let results = evaluate_input(&slurped, &program.expr, &mut sink)?;
            let mut split_doc_state = SplitDocState::new(has_split_doc);
            for result in results {
                split_doc_state.write_separator(&mut writer, &output_config)?;
                any_truthy |= !matches!(&result, OwnedValue::Null | OwnedValue::Bool(false));
                output_value(&mut writer, &result, &output_config)?;
            }
        }
    } else if args.inplace {
        // Handle --inplace: process each file and write back to it
        if input_files.is_empty() {
            anyhow::bail!("--inplace requires at least one file argument");
        }

        let mut global_doc_index: usize = 0;
        for file_path in &input_files {
            let path = Path::new(file_path);
            let input_bytes = read_file(path)?;
            let format = resolve_input_format(args.input_format, Some(path));
            if let Some(code) =
                yaml_validate_guard(&input_bytes, format, args.validate, Some(file_path))
            {
                return Ok(code);
            }

            let mut output_buffer = Vec::new();

            if can_inplace_fast_path {
                // M2 streaming fast path (#478): stream cursor results
                // directly into this file's buffer, skipping the OwnedValue
                // DOM (and its IndexMap, which cannot represent duplicate
                // mapping keys) the same way the stdout M2 path above does.
                let index = YamlIndex::build(&input_bytes)
                    .map_err(|e| anyhow::anyhow!("YAML parse error in {file_path}: {e}"))?;
                let root = index.root(&input_bytes);

                {
                    let mut buf_writer = BufWriter::new(&mut output_buffer);
                    // `---` separators start fresh per file, unlike the
                    // stdout path where they persist across all input files.
                    let mut yaml_doc_streamed = false;

                    match root.value() {
                        YamlValue::Sequence(mut docs) => {
                            while let Some((cursor, rest)) = docs.uncons_cursor() {
                                let should_process = args
                                    .document
                                    .map_or(true, |target| global_doc_index == target);
                                if should_process {
                                    stream_cursor!(
                                        cursor,
                                        &mut buf_writer,
                                        can_inplace_yaml_fast_path,
                                        &mut yaml_doc_streamed
                                    );
                                }
                                global_doc_index += 1;
                                docs = rest;
                            }
                        }
                        _ => {
                            // Single document case. See the stdout M2 path's
                            // identical fallback above: the root cursor
                            // always reports the virtual document sequence,
                            // so real documents go through the Sequence arm.
                            let should_process = args
                                .document
                                .map_or(true, |target| global_doc_index == target);
                            if should_process {
                                if is_identity {
                                    if can_inplace_yaml_fast_path {
                                        root.stream_yaml_document(
                                            &mut FmtWriter(&mut buf_writer),
                                            yaml_indent_spaces,
                                        )
                                        .map_err(|_| anyhow::anyhow!("Write error"))?;
                                    } else {
                                        root.stream_json_document(
                                            &mut FmtWriter(&mut buf_writer),
                                            json_indent_spaces,
                                        )
                                        .map_err(|_| anyhow::anyhow!("Write error"))?;
                                    }
                                    writeln!(buf_writer)?;
                                    if args.exit_status {
                                        any_truthy |=
                                            root.first_child().is_some_and(|c| !c.is_falsy());
                                    }
                                } else if let Some(doc_cursor) = root.first_child() {
                                    stream_cursor!(
                                        doc_cursor,
                                        &mut buf_writer,
                                        can_inplace_yaml_fast_path,
                                        &mut yaml_doc_streamed
                                    );
                                }
                            }
                            global_doc_index += 1;
                        }
                    }
                    buf_writer.flush()?;
                }
            } else {
                let inputs = parse_input(&input_bytes, format)?;

                let mut buf_writer = BufWriter::new(&mut output_buffer);
                // Count matching docs for multi-doc separator logic
                let matching_docs: usize = if let Some(target_doc) = args.document {
                    usize::from(
                        (global_doc_index..global_doc_index + inputs.len()).contains(&target_doc),
                    )
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
                    let results = evaluate_input(input, &program.expr, &mut sink)?;
                    // Write without color for inplace editing
                    let mut no_color_config = output_config.clone();
                    no_color_config.use_color = false;
                    for result in results {
                        split_doc_state.write_separator(&mut buf_writer, &no_color_config)?;
                        any_truthy |=
                            !matches!(&result, OwnedValue::Null | OwnedValue::Bool(false));
                        output_value(&mut buf_writer, &result, &no_color_config)?;
                    }
                }
                buf_writer.flush()?;
                global_doc_index += inputs.len();
            }

            // Write the output back to the file
            std::fs::write(path, &output_buffer)
                .with_context(|| format!("failed to write to file: {}", path.display()))?;
        }
    } else {
        // Standard path: evaluate inputs
        // For YAML inputs, use direct evaluation to preserve position metadata
        // For JSON inputs, use the OwnedValue path

        // Collect input sources with their bytes and formats
        let input_sources: Vec<(Vec<u8>, InputFormat)> = if input_files.is_empty() {
            let input_bytes = read_stdin()?;
            let format = resolve_input_format(args.input_format, None);
            if let Some(code) = yaml_validate_guard(&input_bytes, format, args.validate, None) {
                return Ok(code);
            }
            vec![(input_bytes, format)]
        } else {
            let mut sources = Vec::new();
            for file_path in &input_files {
                let path = Path::new(file_path);
                let input_bytes = read_file(path)?;
                let format = resolve_input_format(args.input_format, Some(path));
                if let Some(code) =
                    yaml_validate_guard(&input_bytes, format, args.validate, Some(file_path))
                {
                    return Ok(code);
                }
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
                        evaluate_yaml_direct_filtered(bytes, &program.expr, doc_filter, &mut sink)?;
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
                        let results = evaluate_input(&input, &program.expr, &mut sink)?;
                        json_results.push(results);
                        global_doc_index += 1;
                    }
                    all_results.push(json_results);
                }
            }
        }

        // Count total documents from collected results (after filtering)
        let total_docs: usize = all_results.iter().map(std::vec::Vec::len).sum();
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
                    any_truthy |= !matches!(&result, OwnedValue::Null | OwnedValue::Bool(false));
                    output_value(&mut writer, &result, &output_config)?;
                }
            }
        }
    }

    writer.flush()?;

    // Determine exit code. An uncaught error outranks -e: the filter failed,
    // which is not the same as it succeeding with a falsy result (#355 vs #178).
    // yq collapses both to 1, but the diagnostic already went to stderr, so
    // reporting "no matches found" on top of it would be misleading.
    if sink.hit() {
        return Ok(DiagStyle::Yq.error_exit_code());
    }

    // yq compat: empty output and all-falsy output are the same failure,
    // reported on stderr with a fixed message and exit 1.
    if args.exit_status && !any_truthy {
        eprintln!("Error: no matches found");
        return Ok(exit_codes::FALSE_OR_NULL);
    }

    Ok(exit_codes::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use succinctly::jq::NumberRepr;

    #[test]
    fn test_yaml_to_owned_value_string() {
        let yaml = b"name: Alice";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        // Root is a document array, get first doc
        if let YamlValue::Sequence(docs) = root.value() {
            if let Some((doc, _)) = docs.uncons_cursor() {
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
            if let Some((doc, _)) = docs.uncons_cursor() {
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
    fn test_emit_yaml_value_number_literal_nan_and_infinite() {
        // A `NumberLiteral` that parses to a non-finite float (NaN or +/-
        // infinity -- reachable from a document number that overflows f64,
        // e.g. `1e400`) must render the same YAML sentinels as a plain
        // non-finite Float, not fall through to `number_str`.
        let config = OutputConfig {
            output_format: OutputFormat::Yaml,
            compact: true,
            raw_output: false,
            join_output: false,
            nul_output: false,
            ascii_output: false,
            sort_keys: false,
            no_doc: false,
            indent_str: String::new(),
            use_color: false,
        };

        let nan = OwnedValue::NumberLiteral(NumberRepr::Float(f64::NAN), "nan".into());
        assert_eq!(emit_yaml_value(&nan, &config, 0, false), ".nan");

        let pos_inf = OwnedValue::NumberLiteral(NumberRepr::Float(f64::INFINITY), "1e400".into());
        assert_eq!(emit_yaml_value(&pos_inf, &config, 0, false), ".inf");

        let neg_inf =
            OwnedValue::NumberLiteral(NumberRepr::Float(f64::NEG_INFINITY), "-1e400".into());
        assert_eq!(emit_yaml_value(&neg_inf, &config, 0, false), "-.inf");
    }

    #[test]
    fn test_yaml_to_owned_value_bool() {
        let yaml = b"active: true";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Sequence(docs) = root.value() {
            if let Some((doc, _)) = docs.uncons_cursor() {
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
            if let Some((doc, _)) = docs.uncons_cursor() {
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
            if let Some((doc, _)) = docs.uncons_cursor() {
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
            if let Some((doc, _)) = docs.uncons_cursor() {
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
            if let Some((doc, _)) = docs.uncons_cursor() {
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
            if let Some((doc, _)) = docs.uncons_cursor() {
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
            if let Some((doc, _)) = docs.uncons_cursor() {
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

    /// Evaluate YAML through the production path and flatten per-document groups.
    fn eval_yaml(bytes: &[u8], expr: &Expr) -> Vec<OwnedValue> {
        let (groups, _) =
            evaluate_yaml_direct_filtered(bytes, expr, None, &mut ErrorSink::default()).unwrap();
        groups.into_iter().flatten().collect()
    }

    #[test]
    fn test_evaluate_yaml_identity() {
        let yaml = b"name: Alice\nage: 30";
        let expr = Expr::Identity;
        let results = eval_yaml(yaml, &expr);

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
    fn test_evaluate_yaml_field() {
        let yaml = b"name: Alice\nage: 30";
        let expr = Expr::Field("name".to_string());
        let results = eval_yaml(yaml, &expr);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0], OwnedValue::String("Alice".to_string()));
    }

    #[test]
    fn test_evaluate_yaml_line_builtin() {
        use succinctly::jq::Builtin;

        let yaml = b"name: Alice\nage: 30";
        let expr = Expr::Builtin(Builtin::Line);
        let results = eval_yaml(yaml, &expr);

        assert_eq!(results.len(), 1);
        // The mapping starts at line 1
        assert_eq!(results[0], OwnedValue::Int(1));
    }

    #[test]
    fn test_evaluate_yaml_pipe() {
        let yaml = b"users:\n  - name: Alice\n  - name: Bob";
        // .users | .[0] | .name
        let expr = Expr::Pipe(vec![
            Expr::Field("users".to_string()),
            Expr::Index(0),
            Expr::Field("name".to_string()),
        ]);
        let results = eval_yaml(yaml, &expr);

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
        let results = evaluate_input(&slurped, &expr, &mut ErrorSink::default()).unwrap();

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
            for (k, v) in map {
                println!("  key={k:?}, value={v:?}");
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
        let results = eval_yaml(yaml, &expr);

        assert_eq!(results.len(), 1);
        if let OwnedValue::Object(map) = &results[0] {
            println!("direct eval map len: {}", map.len());
            for (k, v) in map {
                println!("  key={k:?}, value={v:?}");
            }
            assert_eq!(map.len(), 1, "expected 1 key but got {} keys", map.len());
        } else {
            panic!("expected object, got {:?}", results[0]);
        }
    }
}
