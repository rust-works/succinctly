//! yq-compatible command runner for succinctly.
//!
//! This module implements a yq-compatible CLI interface using the succinctly
//! YAML semi-indexing and jq expression evaluator.

use anyhow::{Context, Result};
use core::fmt::Write as FmtWrite;
use indexmap::IndexMap;
use std::io::{BufWriter, IsTerminal, Read, Write};
use std::path::Path;

use succinctly::jq::document::{DocumentCursor, DocumentFields, IndentSpec};
use succinctly::jq::eval_generic::{
    eval_with_cursor_using, to_owned as generic_to_owned, to_owned_with_comments, CommentTree,
    GenericResult,
};
use succinctly::jq::{
    self, sync_aliased_paths, Builtin, EvalError, Expr, OwnedValue, QueryResult, YqSemantics,
};
use succinctly::json::JsonIndex;
use succinctly::yaml::{
    resolve_plain, resolve_tagged, stream_yaml_sequence, YamlCursor, YamlIndex, YamlValue,
};

use super::{FrontMatterMode, InputFormat, OutputFormat, YqCommand};
use crate::front_matter;
use crate::output::{
    self, exit_codes, format_float_yq, ColorScheme, ControlEscape, DiagStyle, ErrorSink,
    FloatStyle, InputLocation, JsonFormatOpts,
};

/// yq's diagnostics carry no `(at <file>:<line>)` marker, so the yq paths have
/// no location to report — unlike jq, whose marker names the input value (#355).
fn no_location() -> InputLocation {
    InputLocation::unknown()
}

/// Route a streamed `GenericResult`'s terminating outcome into `sink`: a halt
/// (#791) outranks an error, since `StreamStats::halt` carries the real exit
/// code and must reach `sink.request_halt` directly, never `report_stream` —
/// that path would both misreport the exit code and print a spurious "not
/// propagated" diagnostic no real jq/yq ever emits. Streaming never writes a
/// diagnostic to stdout; an error is handed back here so it reaches stderr
/// and fails the run (#355). Shared by `stream_cursor!`'s YAML and JSON arms,
/// which otherwise duplicated this precedence check verbatim.
fn absorb_stream_stats(sink: &mut ErrorSink, stats: &succinctly::jq::stream::StreamStats) {
    if let Some(code) = stats.halt {
        sink.request_halt(code);
    } else if let Some(err) = &stats.error {
        sink.report_stream(DiagStyle::Yq, err, &no_location());
    }
}

/// Adapter to use `std::io::Write` with `core::fmt::Write` methods.
/// This enables streaming JSON output without intermediate String allocation.
struct FmtWriter<W>(W);

impl<W: Write> core::fmt::Write for FmtWriter<W> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.0.write_all(s.as_bytes()).map_err(|_| core::fmt::Error)
    }
}

/// Either a direct passthrough to the real output writer, or an in-memory
/// buffer collecting output to be colorized afterward. `colorize_yaml`/
/// `output::colorize_json` are pure text-level re-lexers over an already
/// fully-rendered string, so buffering the duplicate-key-safe cursor
/// streamer's output and running it through them unmodified reuses that
/// existing coloring code without teaching the streamers anything about
/// ANSI codes (#748).
enum ColorSink<'a, W: Write> {
    Buffered(String),
    Direct(FmtWriter<&'a mut W>),
}

impl<W: Write> core::fmt::Write for ColorSink<'_, W> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        match self {
            ColorSink::Buffered(buf) => buf.write_str(s),
            ColorSink::Direct(w) => w.write_str(s),
        }
    }
}

/// Streams through `render` directly when `use_color` is false. When true,
/// renders into a buffer instead (still via the duplicate-key-safe cursor
/// streamer passed in as `render`), then runs the buffer through `colorize`
/// before writing the colorized result (#748).
fn stream_maybe_colored<W: Write, T>(
    writer: &mut W,
    use_color: bool,
    colorize: impl FnOnce(&str) -> String,
    render: impl FnOnce(&mut ColorSink<'_, W>) -> Result<T, core::fmt::Error>,
) -> anyhow::Result<T> {
    if use_color {
        let mut sink = ColorSink::Buffered(String::new());
        let value = render(&mut sink).map_err(|_| anyhow::anyhow!("Write error"))?;
        let ColorSink::Buffered(buf) = sink else {
            unreachable!("stream_maybe_colored always constructs ColorSink::Buffered here")
        };
        write!(writer, "{}", colorize(&buf))?;
        Ok(value)
    } else {
        let mut sink = ColorSink::Direct(FmtWriter(writer));
        render(&mut sink).map_err(|_| anyhow::anyhow!("Write error"))
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
                    return Ok(resolved.to_owned_value(str_value));
                }
            }

            // Quoted strings should always be treated as strings (yq-compatible behavior)
            // Only unquoted scalars should undergo type detection
            if !s.is_unquoted() {
                return Ok(OwnedValue::String(str_value.into_owned()));
            }

            // Resolve plain scalars per the YAML 1.2 core schema
            Ok(resolve_plain(&str_value).to_owned_value(str_value))
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
            // `uncons_resolved_cursor`, not `uncons_cursor`: the recursive
            // call's own `cursor.explicit_tag()` above doesn't resolve a
            // bare `-` sequence-item wrapper itself (see
            // `YamlCursor::anchor`'s doc comment for why), so an
            // unresolved cursor here would silently drop an explicit tag
            // on a bare-dash-deferred scalar (#835).
            while let Some((elem_cursor, next)) = rest.uncons_resolved_cursor() {
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

/// Applies `--front-matter`, if set, to raw input bytes before format
/// resolution and validation: the raw bytes (e.g. Markdown) aren't valid
/// standalone YAML, so extraction must happen first. Returns
/// `(bytes, format, body)`; `body` is `Some` only in `process` mode, where
/// the caller must reattach it verbatim after the transformed front matter.
///
/// Once a mode is set, the returned format is always `InputFormat::Yaml`
/// regardless of `resolved_format` -- front matter is YAML by definition,
/// and the `run_yq` compat guard already rejects an explicit
/// `--input-format json` paired with `--front-matter`, so this never
/// actually overrides a caller's real preference.
fn apply_front_matter(
    raw_bytes: Vec<u8>,
    resolved_format: InputFormat,
    front_matter: Option<FrontMatterMode>,
    name: &str,
) -> Result<(Vec<u8>, InputFormat, Option<Vec<u8>>)> {
    let Some(mode) = front_matter else {
        return Ok((raw_bytes, resolved_format, None));
    };
    let fm =
        front_matter::split_front_matter(&raw_bytes).map_err(|e| anyhow::anyhow!("{name}: {e}"))?;
    let body = (mode == FrontMatterMode::Process).then(|| fm.body.to_vec());
    Ok((fm.yaml.to_vec(), InputFormat::Yaml, body))
}

/// The result of [`gather_input_sources`]: either the gathered sources, or
/// an exit code from a failed `--validate` check that the caller must
/// return immediately, mirroring [`yaml_validate_guard`]'s "bail with this
/// code" contract at each of this function's now-single call site.
enum GatheredSources {
    Sources(Vec<(Vec<u8>, InputFormat, Option<Vec<u8>>)>),
    ExitCode(i32),
}

/// Reads each input source -- stdin if `input_files` is empty, else each
/// file in listed order -- applying `--front-matter` (a no-op unless the
/// flag is set, via [`apply_front_matter`]) and resolving its format, then
/// running the shared `--validate` guard. Shared by `--eval-all`,
/// `--split-exp`, `--slurp`, and the default path, which all start from
/// this identical stdin-or-per-file gather step; each `--front-matter`
/// body is `None` unless `front_matter` is `Some(Process)`, same contract
/// as `apply_front_matter`.
fn gather_input_sources(
    input_files: &[String],
    input_format: InputFormat,
    front_matter: Option<FrontMatterMode>,
    validate: bool,
) -> Result<GatheredSources> {
    let mut sources = Vec::new();
    if input_files.is_empty() {
        let raw_bytes = read_stdin()?;
        let resolved_format = resolve_input_format(input_format, None);
        let (input_bytes, format, body) =
            apply_front_matter(raw_bytes, resolved_format, front_matter, "<stdin>")?;
        if let Some(code) = yaml_validate_guard(&input_bytes, format, validate, None) {
            return Ok(GatheredSources::ExitCode(code));
        }
        sources.push((input_bytes, format, body));
    } else {
        for file_path in input_files {
            let path = Path::new(file_path);
            let raw_bytes = read_file(path)?;
            let resolved_format = resolve_input_format(input_format, Some(path));
            let (input_bytes, format, body) =
                apply_front_matter(raw_bytes, resolved_format, front_matter, file_path)?;
            if let Some(code) = yaml_validate_guard(&input_bytes, format, validate, Some(file_path))
            {
                return Ok(GatheredSources::ExitCode(code));
            }
            sources.push((input_bytes, format, body));
        }
    }
    Ok(GatheredSources::Sources(sources))
}

/// Recursively collapses every `NumberLiteral` in `value` into a bare
/// `Int`/`Float`, discarding the number's original source-text spelling.
///
/// Unlike YAML input (#918, correctly preserved), real `yq` never
/// preserves a JSON-sourced number's exact spelling — `1.0` always renders
/// as `1`, whether touched by the filter or not — most plausibly because
/// its `--input-format json` path reads through Go's `encoding/json` into
/// a plain `float64`, with no "untouched literal" concept at all (#978).
/// `generic_to_owned` (this crate's shared `OwnedValue` materializer) has
/// no such format-awareness — it's also `succinctly jq`'s own conversion,
/// which correctly *does* preserve JSON literal spelling — so this stays
/// local to `yq_runner.rs`'s own JSON-input call sites rather than
/// touching that shared function.
fn canonicalize_json_numbers(value: OwnedValue) -> OwnedValue {
    match value {
        OwnedValue::Array(items) => {
            OwnedValue::Array(items.into_iter().map(canonicalize_json_numbers).collect())
        }
        OwnedValue::Object(fields) => OwnedValue::Object(
            fields
                .into_iter()
                .map(|(k, v)| (k, canonicalize_json_numbers(v)))
                .collect(),
        ),
        other => other.into_plain_number(),
    }
}

/// Parse input bytes according to the specified format.
fn parse_input(bytes: &[u8], format: InputFormat) -> Result<Vec<OwnedValue>> {
    match format {
        InputFormat::Json => {
            // Parse as JSON
            let index = JsonIndex::build(bytes);
            let cursor = index.root(bytes);
            Ok(vec![canonicalize_json_numbers(generic_to_owned(
                &cursor.value(),
            ))])
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

/// A single jq result value paired with its parallel [`CommentTree`] (issue #710).
type ResultWithComments = (OwnedValue, CommentTree);

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
    need_comments: bool,
    strip_style: bool,
) -> Result<(Vec<Vec<ResultWithComments>>, usize)> {
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
                    let results =
                        evaluate_yaml_cursor(cursor, expr, sink, need_comments, strip_style)?;
                    // Only include documents that have results (select may filter them out)
                    if !results.is_empty() {
                        doc_results.push(results);
                    }
                }

                local_idx += 1;
                docs = rest;

                // halt/halt_error (#791) outranks evaluating any further
                // documents in this file — the caller checks `sink.halted()`
                // too, to stop further *files*.
                if sink.halted().is_some() {
                    break;
                }
            }
            Ok((doc_results, local_idx))
        }
        _ => {
            // Defensive fallback only, same as the inplace fast path's
            // identical `_ =>` arm below: `root.value()` always reports the
            // virtual document sequence (single documents included), so
            // this arm is unreachable through any real input today.
            let should_eval = match doc_filter {
                Some((target_doc, global_offset)) => global_offset == target_doc,
                None => true,
            };

            if should_eval {
                if let Some(content_cursor) = root.first_child() {
                    let results = evaluate_yaml_cursor(
                        content_cursor,
                        expr,
                        sink,
                        need_comments,
                        strip_style,
                    )?;
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
    Ok(query_result_to_owned_values(result, sink))
}

/// Convert a `QueryResult` into `Vec<OwnedValue>`, reporting an uncaught
/// error/break through `sink` (evaluation continues past one, per yq's
/// convention -- see `ErrorSink`'s own doc comment) rather than failing the
/// whole run. Factored out of [`evaluate_input`] so `--eval-all`'s
/// `eval_owned_with_file_index` call site (#715) shares the exact same
/// conversion/error-reporting policy instead of a second, divergence-prone
/// copy.
fn query_result_to_owned_values(
    result: QueryResult<'_, Vec<u64>>,
    sink: &mut ErrorSink,
) -> Vec<OwnedValue> {
    match result {
        QueryResult::One(v) => vec![generic_to_owned(&v)],
        QueryResult::OneCursor(c) => vec![generic_to_owned(&c.value())],
        QueryResult::Many(vs) => vs.iter().map(generic_to_owned).collect(),
        QueryResult::None => vec![],
        QueryResult::Error(e) => {
            sink.report(DiagStyle::Yq, &e, &no_location());
            vec![]
        }
        QueryResult::Owned(v) => vec![v],
        QueryResult::ManyOwned(vs) => vs,
        QueryResult::Break(label) => {
            sink.report_break(DiagStyle::Yq, &label, &no_location());
            vec![]
        }
        // `halt`/`halt_error` (#791): not a diagnostic, so no `sink.report*`
        // call — `request_halt` records the exit code for the caller's loop
        // to short-circuit on, without touching `hit`/`report_count`.
        QueryResult::Halt(code) => {
            sink.request_halt(code);
            vec![]
        }
        // The outputs already produced no longer vanish behind the failure
        // (#400, #494).
        QueryResult::Partial(vs, jq::Control::Error(e)) => {
            sink.report(DiagStyle::Yq, &e, &no_location());
            vs
        }
        QueryResult::Partial(vs, jq::Control::Break(label)) => {
            sink.report_break(DiagStyle::Yq, &label, &no_location());
            vs
        }
        QueryResult::Partial(vs, jq::Control::Halt(code)) => {
            sink.request_halt(code);
            vs
        }
    }
}

/// Whether `expr` is itself an assignment-family write: `.path = value`,
/// `|=`, a compound assign, `//=`, or `del(...)`. Unwraps `Paren`/`Optional`
/// so `(.a = 1)?` still counts, and recurses into `Pipe` so a chain counts
/// as soon as *any* stage writes.
///
/// Split out from [`is_alias_sensitive_assign`] so a pipe made entirely of
/// pass-through stages (`select(true) | debug`, no write at all) doesn't
/// pay for alias-sync snapshotting -- only a pipe that both preserves shape
/// *and* actually writes somewhere needs the pristine-vs-result diff.
fn contains_assign(expr: &Expr) -> bool {
    match expr {
        Expr::Assign { .. }
        | Expr::Update { .. }
        | Expr::CompoundAssign { .. }
        | Expr::AlternativeAssign { .. }
        | Expr::Builtin(Builtin::Del(_)) => true,
        Expr::Paren(inner) | Expr::Optional(inner) => contains_assign(inner),
        Expr::Pipe(stages) => stages.iter().any(contains_assign),
        _ => false,
    }
}

/// Whether `expr`'s top-level shape is "rewrite the document at specific
/// paths, leaving everything else identical" -- the class of expression for
/// which comparing a path's value before and after the write is meaningful.
/// Unwraps `Paren`/`Optional` so `(.a = 1)?` still matches, and recurses into
/// `Pipe` so a chain matches when every stage does, whether the stage is a
/// write (`.a = 1 | .b = 2`) or one of a small allow-list of pass-through
/// stages that provably either emit the input document completely unchanged
/// or emit nothing at all: `.` (`Identity`), `select(...)`, `empty`,
/// `debug`/`debug(msg)`. Mixing one into an assignment pipe (`.a = 1 |
/// select(.a > 0)`, the guard-style `yq -i` idiom from #764) still leaves
/// "the same path means the same thing" true for every document that comes
/// out the other end, since none of these four ever rewrite or reshape the
/// value they pass through -- unlike `map`, `select`'s own predicate or
/// `debug`'s own message expression never appear in the pipeline's output,
/// only their pass/fail or side-effect result does, so `contains_assign`
/// deliberately does not recurse into either.
///
/// Used to gate the alias-sync post-process (#711): outside this class (a
/// bare `map`, `.a, .b`, ...) the result document doesn't necessarily share
/// the input's shape at all, so diffing "the same path" in both would be
/// meaningless at best and could clobber it at worst. A pipe with a stage
/// outside both the write list and this pass-through allow-list is
/// conservatively excluded for the same reason -- verifying more stages
/// preserve paths is left for a future extension, not assumed here.
fn is_alias_sensitive_assign(expr: &Expr) -> bool {
    fn is_shape_preserving(expr: &Expr) -> bool {
        match expr {
            Expr::Assign { .. }
            | Expr::Update { .. }
            | Expr::CompoundAssign { .. }
            | Expr::AlternativeAssign { .. }
            | Expr::Identity
            | Expr::Builtin(
                Builtin::Del(_)
                | Builtin::Select(_)
                | Builtin::Empty
                | Builtin::Debug
                | Builtin::DebugMsg(_),
            ) => true,
            Expr::Paren(inner) | Expr::Optional(inner) => is_shape_preserving(inner),
            Expr::Pipe(stages) => stages.iter().all(is_shape_preserving),
            _ => false,
        }
    }

    is_shape_preserving(expr) && contains_assign(expr)
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
            // `uncons_resolved_cursor`, not `uncons_cursor`: a bare `-`
            // item's own anchor (if any) sits on the deferred value's line,
            // not the wrapper's, and this function's own `cursor.anchor()`
            // check above doesn't resolve through the wrapper itself (see
            // `YamlCursor::anchor`'s doc comment) — an unresolved cursor
            // here would silently miss the anchor and break alias-sync
            // bookkeeping for a bare-dash-deferred anchored value (#835).
            while let Some((elem_cursor, next_rest)) = rest.uncons_resolved_cursor() {
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

/// Reconcile a pristine (pre-write) presentation tree against a post-write
/// value (issue #739, ADR-0017's mechanism 1, applied to path-mutation
/// queries: `=`, `|=`, `+=`, `del()`, ...).
///
/// Verified against the pinned real `yq` binary: a node keeps its own
/// comment and style across a write as long as the write leaves it the
/// same *kind* (`Object`/`Array`/scalar) — regardless of whether its
/// *value* actually changed. `.b = 2` on `b: 1 # keep` still prints
/// `b: 2 # keep`; `.a = "new"` on `a: 'old'` still prints `a: 'new'`
/// (single-quote style survives even though the string content didn't).
/// Real `yq`'s in-place node-mutation model (go-yaml) explains why: `Set`
/// overwrites the existing `Node.Value` but never touches that node's own
/// `Style`/`LineComment`. Only a kind change (scalar becomes a container or
/// vice versa) discards them, since there's no such node to keep - matching
/// `.a = {"x": 1}` on `a: 'old'` dropping the quote style entirely (real
/// `yq`: `a:\n  x: 1`).
///
/// Walks `pristine_value`/`result_value` in lockstep rather than computing
/// which path(s) an expression's AST touches (`resolve_dynamic_indexes` in
/// `eval.rs`) and invalidating just those - this is exact for every
/// mutation shape uniformly (a single assign, a chained pipe of several, a
/// computed-key write, a `del()`) with no per-expression-shape logic, and
/// the YAML documents this rewrites are config-file-sized, not
/// data-file-sized.
///
/// **Known gap, filed as #870**: this function matches a child to
/// its pristine counterpart purely by key/index, so any write that
/// reshuffles an `Array`'s or `Object`'s *positions* - not just a
/// wholesale replacement like `.a = [4, 5, 6]` on `a: ['x', 'y', 'z']`,
/// but an everyday `.arr = ["new"] + .arr` prepend or a `del()` that
/// shifts later indices down - misattributes old elements' style/comments
/// to whichever new element now sits at the same key/index, instead of
/// giving every element under the write fresh (empty) metadata the way
/// real `yq`'s node-mutation model does (confirmed against the pinned
/// binary: prepending to `arr:\n  - "x"\n  - y` should drop the new
/// element's style entirely and leave `"x"`'s own quotes exactly where
/// they were; this function instead lets the new element inherit `"x"`'s
/// old quote style and leaves `"x"` unquoted). This function has no way
/// to tell "recursing into an untouched sibling subtree" apart from
/// "recursing into a reshuffled subtree that happens to share
/// positions/keys with the old one" - both look identical from a pure
/// before/after value diff; only a path-based approach (the
/// `resolve_dynamic_indexes` alternative above) could distinguish them.
/// Purely cosmetic (no data loss, no incorrect *values*, still strictly
/// better than every write losing all style/comments unconditionally,
/// which was the pre-#739 baseline).
fn reconcile_presentation(
    pristine_value: &OwnedValue,
    pristine_tree: &CommentTree,
    result_value: &OwnedValue,
) -> CommentTree {
    match (pristine_value, result_value) {
        (OwnedValue::Object(p_fields), OwnedValue::Object(r_fields)) => {
            let own_comment = pristine_tree.own().map(str::to_string);
            let own_style = pristine_tree.style();
            let mut fields = IndexMap::new();
            let mut key_comments = IndexMap::new();
            for (k, r_v) in r_fields {
                let child = match p_fields.get(k) {
                    Some(p_v) => reconcile_presentation(p_v, pristine_tree.field(k), r_v),
                    None => CommentTree::empty(),
                };
                fields.insert(k.clone(), child);
                // A key's own trailing comment (#765) belongs to the key's
                // line, not its value, so it survives regardless of
                // whether the value under it changed - only a removed key
                // (not iterated here, since this loop is over `r_fields`)
                // drops it. The "deferred value materialized as nothing"
                // flag, though, is re-derived from `r_v`, not copied from
                // pristine: a write that gives the key a real value must
                // not carry over a stale "absent" flag, or
                // `key_comment_if_value_absent`'s own consumer
                // (`emit_yaml_value`'s block-mapping arm) renders only the
                // key and comment, silently dropping the write's value
                // entirely (found in review).
                if let CommentTree::Object(_, _, _, pristine_key_comments) = pristine_tree {
                    if let Some((kc, _)) = pristine_key_comments.get(k) {
                        let value_absent = matches!(r_v, OwnedValue::Null);
                        key_comments.insert(k.clone(), (kc.clone(), value_absent));
                    }
                }
            }
            CommentTree::Object(own_comment, own_style, fields, key_comments)
        }
        (OwnedValue::Array(p_items), OwnedValue::Array(r_items)) => {
            let own_comment = pristine_tree.own().map(str::to_string);
            let own_style = pristine_tree.style();
            let items = r_items
                .iter()
                .enumerate()
                .map(|(i, r_v)| match p_items.get(i) {
                    Some(p_v) => reconcile_presentation(p_v, pristine_tree.at_index(i), r_v),
                    None => CommentTree::empty(),
                })
                .collect();
            CommentTree::Array(own_comment, own_style, items)
        }
        // A kind change (container <-> scalar, or Object <-> Array) is a
        // fresh node with no presentation memory of its own.
        (OwnedValue::Object(_) | OwnedValue::Array(_), _)
        | (_, OwnedValue::Object(_) | OwnedValue::Array(_)) => CommentTree::empty(),
        // Both scalars, any variant/value: same node, only its value
        // changed - its own comment and style survive.
        _ => CommentTree::Leaf(
            pristine_tree.own().map(str::to_string),
            pristine_tree.style(),
        ),
    }
}

/// Evaluate `split_expr` against `result` with `$index` bound to
/// `output_index`, expect exactly one string back, and write `result`
/// (serialized through `output_config`, color forced off, matching
/// `--inplace`) to that path.
///
/// Non-string, empty, or multi-value split-expression results are reported
/// through `sink` — the same "continue, exit 1 at the end" convention used
/// for other uncaught evaluation errors in this file — rather than aborting
/// the run outright.
fn write_split_result(
    result: &OwnedValue,
    comments: &CommentTree,
    split_expr: &Expr,
    output_index: i64,
    output_config: &OutputConfig,
    written_files: &mut std::collections::HashSet<String>,
    sink: &mut ErrorSink,
) -> Result<()> {
    let index_val = OwnedValue::Int(output_index);
    let per_result_expr = jq::substitute_vars(split_expr, [("index", &index_val)]);

    // Snapshotted before evaluating the split-filename expression, so the
    // check below can tell "this call's own expression halted" from "the
    // *main* expression already halted and `result` is an output-bearing
    // `Partial` prefix that still owes its file" (#791): `sink.halted()` is
    // sticky for the whole run (`request_halt`'s "first halt wins"), so
    // without this snapshot every `write_split_result` call after a
    // mid-stream halt would misread the already-set flag as its own and
    // skip writing a result that must still be split out.
    let halted_before = sink.halted().is_some();

    let reports_before = sink.report_count();
    let filename_results = evaluate_input(result, &per_result_expr, sink)?;

    // halt/halt_error (#791) inside *this* split expression: not a
    // diagnostic (no `report_count` bump), so an empty result must be
    // checked before the `[]` arms below, or it would be misreported as
    // "produced no output". But a halt that already produced a value (e.g.
    // `"out\($index).yml", halt`) must still fall through to the match below
    // so that value gets written -- only bail early when the halt left
    // nothing behind, or a legitimately-produced filename is silently lost.
    if !halted_before && sink.halted().is_some() && filename_results.is_empty() {
        return Ok(());
    }

    let filename = match filename_results.as_slice() {
        [OwnedValue::String(s)] => s.clone(),
        // `evaluate_input` already reported the underlying error (e.g. an
        // undefined variable, or an explicit `error(...)`) via `sink`.
        // `report_count()` (not `hit()`, which is sticky for the whole run)
        // is what lets this tell "this call just reported" from "some
        // earlier result already tripped the sink" -- otherwise every
        // result after the first real error in the run double-reports here
        // (#715 follow-up).
        [] if sink.report_count() > reports_before => return Ok(()),
        [] => {
            sink.report(
                DiagStyle::Yq,
                &EvalError::new(format!(
                    "--split-exp expression produced no output for result #{output_index}"
                )),
                &no_location(),
            );
            return Ok(());
        }
        [other] => {
            sink.report(
                DiagStyle::Yq,
                &EvalError::new(format!(
                    "--split-exp expression must evaluate to a string, got {} for result #{output_index}",
                    other.type_name()
                )),
                &no_location(),
            );
            return Ok(());
        }
        many => {
            sink.report(
                DiagStyle::Yq,
                &EvalError::new(format!(
                    "--split-exp expression must evaluate to exactly one string, got {} results for result #{output_index}",
                    many.len()
                )),
                &no_location(),
            );
            return Ok(());
        }
    };

    if !written_files.insert(filename.clone()) {
        eprintln!("Warning: --split-exp path '{filename}' written more than once; overwriting");
    }

    let mut buf = Vec::new();
    let mut no_color_config = output_config.clone();
    no_color_config.use_color = false;
    output_value(&mut buf, result, comments, &no_color_config)?;

    std::fs::write(&filename, &buf)
        .with_context(|| format!("failed to write --split-exp output file: {filename}"))
}

/// Evaluate a jq expression directly on a YAML cursor.
///
/// This uses the generic evaluator to preserve position metadata (line/column).
/// Each result carries its parallel [`CommentTree`] (issue #710/#739) —
/// real for `OneCursor`/`ManyCursor` (still-live cursor); for every other
/// result (an already-materialized/computed value with no cursor of its
/// own) it's [`reconcile_presentation`]'s output against the pristine
/// document when `expr` is a shape-preserving write
/// ([`is_alias_sensitive_assign`]) and the caller wants comments at all
/// (`need_comments`), or [`CommentTree::empty`] otherwise — see
/// [`CommentTree`]'s own doc comment for why a cursor-less value can't
/// generally carry metadata.
fn evaluate_yaml_cursor<W: AsRef<[u64]> + Clone>(
    cursor: YamlCursor<'_, W>,
    expr: &Expr,
    sink: &mut ErrorSink,
    need_comments: bool,
    strip_style: bool,
) -> Result<Vec<ResultWithComments>> {
    // Snapshot alias-sync context from the pristine document *before*
    // evaluation, only when it could possibly matter (#711): an
    // assignment-family expression against a document that actually has
    // aliases. Everything else (JSON, plain reads, alias-free YAML) pays
    // nothing beyond this one bool check.
    let alias_sync_ctx =
        (is_alias_sensitive_assign(expr) && cursor.index().has_aliases()).then(|| {
            (
                generic_to_owned(&cursor.value()),
                collect_alias_groups(cursor),
            )
        });

    // Snapshot the pristine presentation tree *before* evaluation too
    // (#739, ADR-0017): same shape-preserving-write gate as `alias_sync_ctx`
    // above (no aliases required here - a write-family expression against
    // any YAML document can lose style/comments, not just an aliased one),
    // and only when the caller can use the result at all (`need_comments`).
    // `no_comments` below reconciles this against each result document once
    // evaluation finishes.
    let presentation_sync_ctx = (need_comments && is_alias_sensitive_assign(expr))
        .then(|| to_owned_with_comments(&cursor.value(), Some(&cursor)));

    let result = eval_with_cursor_using::<YqSemantics, _>(expr, cursor);
    // A value with no live cursor of its own (an assignment/`del()`
    // result, a computed value, ...) has no comment/style to read directly
    // - but if it came from a shape-preserving write, `presentation_sync_ctx`
    // lets it recover whatever the write didn't touch instead of falling
    // back to genuinely empty (#739).
    let no_comments = |v: OwnedValue| {
        let comments = presentation_sync_ctx
            .as_ref()
            .map_or_else(CommentTree::empty, |(pristine, tree)| {
                reconcile_presentation(pristine, tree, &v)
            });
        (v, comments)
    };
    // `to_owned_with_comments` builds a full parallel `IndexMap`/`Vec` tree
    // alongside the `OwnedValue` one, just to carry comment text - wasted
    // work when the caller can't use it (`-o json`'s output never reads
    // `CommentTree` at all; see `output_value`'s JSON branch) (#710).
    let owned_with_comments = |c: &YamlCursor<'_, W>| {
        if need_comments {
            to_owned_with_comments(&c.value(), Some(c))
        } else {
            no_comments(generic_to_owned(&c.value()))
        }
    };

    // Convert GenericResult to Vec<ResultWithComments>
    //
    // `One`/`Many` (as opposed to `OneCursor`/`ManyCursor`) only ever arise
    // from `eval_generic.rs`'s cursor-loss cascade, which requires an
    // already-cursor-less value to begin with (e.g. via the cursor-less
    // `eval()` entry point `jq`'s DOM path uses) — this function always
    // starts `eval_with_cursor_using` from a real `cursor`, so these two
    // arms are defensive/unreachable here today, kept for exhaustiveness
    // over the shared `GenericResult` enum.
    let mut docs = match result {
        GenericResult::One(v) => Ok(vec![no_comments(generic_to_owned(&v))]),
        GenericResult::OneCursor(c) => Ok(vec![owned_with_comments(&c)]),
        GenericResult::Many(vs) => Ok(vs.iter().map(generic_to_owned).map(no_comments).collect()),
        GenericResult::ManyCursor(cs) => Ok(cs.iter().map(owned_with_comments).collect()),
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
            Ok(vec![no_comments(OwnedValue::Array(
                keys.into_iter().map(OwnedValue::String).collect(),
            ))])
        }
        // Same reasoning as `LazyKeys` above, for array `keys`/
        // `keys_unsorted` (#684).
        GenericResult::LazyIndexRange(len) => Ok(vec![no_comments(OwnedValue::Array(
            (0..len).map(|i| OwnedValue::Int(i as i64)).collect(),
        ))]),
        // Same reasoning as `LazyKeys`/`LazyIndexRange` above, for a
        // composed `map` chain (#724, #725) that never resolved into a
        // narrower shape before reaching this materializing DOM boundary.
        GenericResult::LazySeq(seq) => match seq.materialize_atomic() {
            Ok(v) => Ok(vec![no_comments(v)]),
            Err(jq::Control::Error(e)) => {
                sink.report(DiagStyle::Yq, &e, &no_location());
                Ok(vec![])
            }
            Err(jq::Control::Break(label)) => {
                sink.report_break(DiagStyle::Yq, &label, &no_location());
                Ok(vec![])
            }
            Err(jq::Control::Halt(code)) => {
                sink.request_halt(code);
                Ok(vec![])
            }
        },
        GenericResult::None => Ok(vec![]),
        GenericResult::Error(e) => {
            sink.report(DiagStyle::Yq, &e, &no_location());
            Ok(vec![])
        }
        GenericResult::Owned(v) => Ok(vec![no_comments(v)]),
        GenericResult::ManyOwned(vs) => Ok(vs.into_iter().map(no_comments).collect()),
        GenericResult::Break(label) => {
            sink.report_break(DiagStyle::Yq, &label, &no_location());
            Ok(vec![])
        }
        // `halt`/`halt_error` (#791): not a diagnostic, so no `sink.report*`
        // call — `request_halt` records the exit code for the caller's loop
        // to short-circuit on, without touching `hit`/`report_count`.
        GenericResult::Halt(code) => {
            sink.request_halt(code);
            Ok(vec![])
        }
        // The outputs already produced no longer vanish behind the failure
        // (#400, #494).
        GenericResult::Partial(vs, jq::Control::Error(e)) => {
            sink.report(DiagStyle::Yq, &e, &no_location());
            Ok(vs.into_iter().map(no_comments).collect())
        }
        GenericResult::Partial(vs, jq::Control::Break(label)) => {
            sink.report_break(DiagStyle::Yq, &label, &no_location());
            Ok(vs.into_iter().map(no_comments).collect())
        }
        GenericResult::Partial(vs, jq::Control::Halt(code)) => {
            sink.request_halt(code);
            Ok(vs.into_iter().map(no_comments).collect())
        }
    };

    if let Some((pristine, groups)) = &alias_sync_ctx {
        if let Ok(docs) = &mut docs {
            for (value, _comments) in docs.iter_mut() {
                sync_aliased_paths(value, pristine, groups);
            }
        }
    }

    // A bare top-level/navigated scalar result drops all its own styling,
    // matching real `yq` (#852) - unlike a scalar nested inside a mapping/
    // sequence, which keeps it (`a: "x"` stays quoted when output as part
    // of the whole document; a standalone `.a` result doesn't). Both
    // `to_owned_with_comments` and `reconcile_presentation` above capture/
    // preserve style uniformly for every node including the root, so this
    // needs its own always-on, top-level-only pass, independent of
    // `need_comments`/`is_alias_sensitive_assign` — real `yq` does this
    // unconditionally, not just for shape-preserving writes.
    if let Ok(docs) = &mut docs {
        for (value, comments) in docs.iter_mut() {
            if !matches!(value, OwnedValue::Object(_) | OwnedValue::Array(_)) {
                *comments = CommentTree::Leaf(comments.own().map(str::to_string), "");
            }
        }
    }

    // `-P`/`--pretty-print` (#705) forces block/plain style regardless of
    // source — a real style-clearing step, not just "there's no style data
    // to clear" like before #739's style tracking existed. Comments stay
    // (`-P` only ever claimed to affect style), so this only touches the
    // style slot of every node, not the tree shape.
    if strip_style {
        if let Ok(docs) = &mut docs {
            for (_value, comments) in docs.iter_mut() {
                *comments = strip_presentation_style(comments);
            }
        }
    }

    docs
}

/// Recursively clear every node's style (issue #705's `-P` gate) while
/// keeping its comments and tree shape untouched — see the `strip_style`
/// call in [`evaluate_yaml_cursor`].
fn strip_presentation_style(tree: &CommentTree) -> CommentTree {
    match tree {
        CommentTree::Leaf(c, _) => CommentTree::Leaf(c.clone(), ""),
        CommentTree::Array(c, _, items) => CommentTree::Array(
            c.clone(),
            "",
            items.iter().map(strip_presentation_style).collect(),
        ),
        CommentTree::Object(c, _, fields, key_comments) => CommentTree::Object(
            c.clone(),
            "",
            fields
                .iter()
                .map(|(k, v)| (k.clone(), strip_presentation_style(v)))
                .collect(),
            key_comments.clone(),
        ),
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

/// Format and output a value, threading `comments` through to `emit_yaml_value`
/// (issue #710). Callers with no cursor-derived comment data (JSON input,
/// `--null-input`, `--raw-input`, `--slurp`, `--inplace`'s slow path) pass
/// `&CommentTree::empty()`.
fn output_value<W: Write>(
    writer: &mut W,
    value: &OwnedValue,
    comments: &CommentTree,
    config: &OutputConfig,
) -> Result<()> {
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
        // A bare top-level result drops all of its own styling (#852,
        // mirroring `YamlCursor::stream_yaml_as_document`'s and
        // `OwnedValue::stream_yaml`'s identical root-only special case for
        // the cursor/streaming paths) - `output_value` is the actual top
        // level for every result that reaches it (its own recursion never
        // calls back into `output_value`, only `emit_yaml_value`), so this
        // covers every caller uniformly: the main eval path, `-n`,
        // `--raw-input`, `--split-exp`, ... Redundant with (but harmless
        // alongside) `evaluate_yaml_cursor`'s equivalent root-scalar pass
        // for the cursor-based DOM path specifically.
        let body = if let OwnedValue::String(s) = value {
            s.clone()
        } else {
            emit_yaml_value(value, comments, config, "", false)
        };
        // Every non-root node's own trailing comment is appended by its
        // *parent* during `emit_yaml_value`'s recursion (see its Array/Object
        // arms), but the root has no parent call site to do that for it —
        // append it here instead, or a comment trailing the jq result's own
        // top-level node (e.g. `[1, 2, 3] # trailing`) is silently dropped
        // (#710). Scalars are excluded: verified against the pinned real
        // `yq` binary, a bare scalar document (`42 # trailing`) drops its
        // own trailing comment from output on both identity and `select`,
        // even though `line_comment` still returns it — real `yq`'s own
        // quirk, not a succinctly gap, so replicated here rather than
        // "fixed" into a new divergence.
        //
        // A flow-styled root (`comments.style() == "flow"`, #739) glues the
        // comment onto `body`'s one and only line instead
        // (`[1, 2, 3] # trailing`, matching real `yq` exactly) - there's no
        // nested child on that same line to collide with. A block-rendered
        // root instead appends it as a standalone comment line, or it would
        // be indistinguishable from the last child's own comment on
        // `body`'s last line (#793) - `append_own_comment_line`'s own doc
        // comment already flagged this as the reason it couldn't match real
        // `yq`'s flow-preserving output, before `CommentTree` carried style
        // data to tell the two cases apart.
        let output = if matches!(value, OwnedValue::Array(_) | OwnedValue::Object(_)) {
            if is_flow_safe(value, comments) {
                format!("{body}{}", trailing_comment_suffix(comments))
            } else {
                append_own_comment_line(body, comments.own(), "")
            }
        } else {
            body
        };
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

/// Whether `value`/`comments` should render in YAML flow style (`[...]`/
/// `{...}`, issue #739) — `comments.style() == "flow"`, unless a child
/// (array element or object field) has its own trailing comment.
///
/// A `#` comment runs to end of line, so a flow collection has nowhere to
/// put one before its last item without breaking onto another line anyway
/// (real `yq` does this with a synthetic trailing comma:
/// `[1, 2, # child\n]`). Falling back to block style — already
/// comment-safe, since every comment gets its own line there — is simpler
/// and more general than replicating that exact placement, at the cost of
/// not matching real `yq`'s output byte-for-byte in this one narrow case
/// (a comment on a non-final flow element); losing the comment entirely
/// would be worse.
fn is_flow_safe(value: &OwnedValue, comments: &CommentTree) -> bool {
    if comments.style() != "flow" {
        return false;
    }
    match value {
        OwnedValue::Array(items) => !(0..items.len()).any(|i| comments.at_index(i).own().is_some()),
        OwnedValue::Object(fields) => !fields.keys().any(|k| comments.field(k).own().is_some()),
        _ => true,
    }
}

/// Format a node's own trailing comment (issue #710) as `" # text"`, or
/// `""` if it has none — the single point of change for the separator
/// convention shared by every `emit_yaml_value` call site that appends one
/// *on the same line* as the rest of the value (safe only when that value
/// renders as a single line — a scalar, or an empty/flow-style container).
fn trailing_comment_suffix(comments: &CommentTree) -> String {
    comments.own().map_or_else(String::new, |c| format!(" {c}"))
}

/// Append a container's own trailing comment (#710/#793) as a standalone
/// comment line at `indent`, rather than concatenating it onto `body`'s
/// last line the way [`trailing_comment_suffix`] does. `body` here is
/// always genuinely multi-line block content (a non-empty, block-rendered
/// `Array`/`Object`) — gluing the container's *own* comment onto that last
/// line would make it indistinguishable from the last child's own trailing
/// comment on that same line, silently merging two distinct comments into
/// one (#793). `OwnedValue` carries no source flow/block style, so this
/// doesn't attempt to match real `yq`'s exact output (which keeps flow
/// style, staying on one line); it only guarantees the two comments never
/// collide.
fn append_own_comment_line(body: String, own_comment: Option<&str>, indent: &str) -> String {
    match own_comment {
        Some(c) => format!("{body}\n{indent}{c}"),
        None => body,
    }
}

/// Emit a YAML value as a string, appending each node's trailing same-line
/// comment from the parallel `comments` tree (issue #710). Flow-style
/// (`in_flow`) contexts never append one — flow items are comma-joined on
/// one line, so there's no meaningful "trailing" position between them.
///
/// `indent` is the exact indent *string* to prepend to this value's own
/// top-level line(s) — not a `depth: usize` repetition count. A block
/// sequence item whose value is a non-empty mapping/sequence renders in
/// real yq's "compact" form (`- ` shares its line with the value's first
/// field/element, and the rest of the value's own content aligns under
/// that first line's content rather than under a full `config.indent_str`
/// step further — see the `Array` arm below), so a plain `depth *
/// config.indent_str` formula can't express every line's indent; passing
/// the literal string lets a compact caller hand down `indent` plus a
/// fixed 2-character offset instead of a whole extra `config.indent_str`
/// step (#785).
fn emit_yaml_value(
    value: &OwnedValue,
    comments: &CommentTree,
    config: &OutputConfig,
    indent: &str,
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
                format_float_yq(*f)
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
        OwnedValue::String(s) => yaml_quote_string_with_style(s, comments.style()),
        OwnedValue::Array(arr) => {
            if arr.is_empty() {
                "[]".to_string()
            } else if in_flow || is_flow_safe(value, comments) {
                // Flow style for nested in flow context
                let items: Vec<_> = arr
                    .iter()
                    .enumerate()
                    .map(|(i, v)| emit_yaml_value(v, comments.at_index(i), config, indent, true))
                    .collect();
                format!("[{}]", items.join(", "))
            } else {
                // Block style sequence
                let items: Vec<_> = arr
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        let elem_comments = comments.at_index(i);
                        let elem_is_flow = is_flow_safe(v, elem_comments);
                        if !elem_is_flow
                            && (matches!(v, OwnedValue::Object(o) if !o.is_empty())
                                || matches!(v, OwnedValue::Array(a) if !a.is_empty()))
                        {
                            // A non-empty mapping/sequence element renders
                            // in real yq's "compact" form: `- ` shares its
                            // line with the value's own first field/
                            // element, and the rest of the value's own
                            // content aligns under that first line's
                            // content (`indent` plus the 2-character width
                            // of `- `), not under `indent` plus a full
                            // `config.indent_str` step like an ordinary
                            // nested block (#785).
                            //
                            // `emit_yaml_value` derives every line's own
                            // indent purely from the `indent` string it's
                            // handed, so rendering the element at
                            // `compact_indent` and then stripping that
                            // exact prefix from just the start of the
                            // result (leaving every subsequent line's own
                            // copy of the prefix untouched) reproduces the
                            // "no separate indent for the first line"
                            // effect `stream_yaml_value`'s cursor-based
                            // sibling gets for free from its per-field/
                            // per-element loop only indenting 2nd+ items.
                            let compact_indent = format!("{indent}  ");
                            let rendered =
                                emit_yaml_value(v, elem_comments, config, &compact_indent, false);
                            // The element's own comment goes on its own
                            // line rather than glued onto its last
                            // grandchild's line (#793).
                            let rendered = append_own_comment_line(
                                rendered,
                                elem_comments.own(),
                                &compact_indent,
                            );
                            let first_line = rendered
                                .strip_prefix(compact_indent.as_str())
                                .unwrap_or(&rendered);
                            format!("{indent}- {first_line}")
                        } else {
                            let val_indent = format!("{indent}{}", config.indent_str);
                            let item =
                                emit_yaml_value(v, elem_comments, config, &val_indent, false);
                            let comment_suffix = trailing_comment_suffix(elem_comments);
                            format!("{indent}- {item}{comment_suffix}")
                        }
                    })
                    .collect();
                items.join("\n")
            }
        }
        OwnedValue::Object(obj) => {
            if obj.is_empty() {
                "{}".to_string()
            } else if in_flow || is_flow_safe(value, comments) {
                // Flow style for nested in flow context
                let entries: Vec<_> = obj
                    .iter()
                    .map(|(k, v)| {
                        let key = yaml_quote_key(k);
                        let val = emit_yaml_value(v, comments.field(k), config, indent, true);
                        format!("{key}: {val}")
                    })
                    .collect();
                format!("{{{}}}", entries.join(", "))
            } else {
                // Block style mapping
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
                        let field_comments = comments.field(k);
                        let comment_suffix = trailing_comment_suffix(field_comments);
                        let val_indent = format!("{indent}{}", config.indent_str);
                        // Check if value needs to be on next line - a
                        // flow-styled container stays on the key's own line
                        // instead, the same as a scalar (#739).
                        let field_is_flow = is_flow_safe(v, field_comments);
                        if !field_is_flow
                            && (matches!(v, OwnedValue::Object(m) if !m.is_empty())
                                || matches!(v, OwnedValue::Array(a) if !a.is_empty()))
                        {
                            // A comment trailing the key's own line, when the
                            // value is deferred to the next line, belongs to
                            // the key, not the value (#765).
                            let key_comment_suffix = comments
                                .key_comment(k)
                                .map_or_else(String::new, |c| format!(" {c}"));
                            // For nested containers, emit one `config.indent_str`
                            // step deeper, which handles its own indentation.
                            // The value's own comment goes on its own line
                            // rather than glued onto its last grandchild's
                            // line (#793).
                            let val =
                                emit_yaml_value(v, field_comments, config, &val_indent, false);
                            let val =
                                append_own_comment_line(val, field_comments.own(), &val_indent);
                            format!("{indent}{key}:{key_comment_suffix}\n{val}")
                        } else if let Some(kc) = comments.key_comment_if_value_absent(k) {
                            // The deferred value materialized as nothing at
                            // all - the key's own comment stands alone with
                            // no value token, matching real yq (#765).
                            format!("{indent}{key}: {kc}")
                        } else {
                            let val =
                                emit_yaml_value(v, field_comments, config, &val_indent, false);
                            // The value's own comment takes priority; fall
                            // back to the key's own comment when the value
                            // has none - covers an explicit key's trailing
                            // comment (`? k # key comment\n: v\n`), which
                            // otherwise has no write site once key and
                            // value collapse onto one output line (#795).
                            // A no-op for the ordinary implicit-key case
                            // (`comment_suffix` is already non-empty then).
                            let comment_suffix = if comment_suffix.is_empty() {
                                comments
                                    .key_comment(k)
                                    .map_or_else(String::new, |c| format!(" {c}"))
                            } else {
                                comment_suffix
                            };
                            format!("{indent}{key}: {val}{comment_suffix}")
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
        yaml_double_quote_escaped(s)
    } else {
        s.to_string()
    }
}

/// Double-quote `s`, escaping as needed — the actual quoting mechanics
/// [`yaml_quote_string`]'s heuristic falls back to whenever it decides
/// quoting is required, factored out so [`yaml_quote_string_with_style`]
/// can also reach it directly to *force* double-quote style regardless of
/// whether the heuristic alone would have required it (#739).
fn yaml_double_quote_escaped(s: &str) -> String {
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
}

/// Single-quote `s`, doubling embedded single quotes per YAML's escaping
/// rule. Only meaningful when [`can_single_quote`] agrees this is safe (a
/// single-quoted flow scalar has no escape sequence for control
/// characters).
fn yaml_single_quote_escaped(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 2);
    result.push('\'');
    for c in s.chars() {
        if c == '\'' {
            result.push_str("''");
        } else {
            result.push(c);
        }
    }
    result.push('\'');
    result
}

/// Whether `s` can round-trip through single-quote style at all — a
/// single-quoted YAML scalar has no escape syntax for control characters
/// (no `\n`, `\t`, ...), unlike double-quoted.
fn can_single_quote(s: &str) -> bool {
    !s.chars().any(|c| c.is_ascii_control())
}

/// Quote a YAML string the way [`yaml_quote_string`] does, except honoring
/// a known original style (`"single"`/`"double"`, from [`CommentTree`]'s
/// per-node style — see [`CommentTree::style`]) when there is one and it's
/// safe to reproduce, instead of always falling back to the plain-or-
/// double-quote heuristic. This is what makes an untouched sibling of a
/// write keep its original quote style rather than losing it entirely —
/// `yaml_quote_string`'s heuristic alone only adds quotes where structurally
/// *required*, which is not the same as matching what the source actually
/// wrote (#739's `'single'` repro needs quotes at all, not just safe ones).
///
/// Any other style (`""`, `"flow"`, `"literal"`, `"folded"` — the last two
/// are block-scalar styles this DOM writer doesn't reproduce; see
/// `CommentTree`'s own doc comment) falls back to the plain heuristic
/// unchanged.
fn yaml_quote_string_with_style(s: &str, style: &str) -> String {
    // No empty-string special case needed here (unlike `yaml_quote_string`
    // below): every arm already renders `""` correctly on its own -
    // `yaml_double_quote_escaped`/`yaml_single_quote_escaped` produce
    // `""`/`''` for an empty `s`, and the `_` fallback defers to
    // `yaml_quote_string`, which has its own empty-string case. A
    // short-circuit here that always returned `''` regardless of `style`
    // used to flip an untouched double-quoted empty string to single-quote
    // style on a sibling write (found in review).
    match style {
        "single" if can_single_quote(s) => yaml_single_quote_escaped(s),
        "double" => yaml_double_quote_escaped(s),
        _ => yaml_quote_string(s),
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

/// Whether a compact block-sequence item's remaining source (the text
/// right after `- `, as returned by `Chars::as_str()` at that point) opens
/// with a mapping key rather than a scalar value — i.e. whether an
/// unquoted `:` appears before the line ends.
///
/// `colorize_yaml`'s state machine colors a token *as it's written*, one
/// `char` at a time, with no ability to go back and re-color something it
/// already emitted — so at the position right after `- `, it has to decide
/// up front whether what follows is a key (color it) or a value (don't),
/// and the only way to tell is to look ahead. Before #785, `- ` was always
/// followed by either a scalar value or a newline (a container value's
/// mapping always started on its own line), so this ambiguity never arose.
/// A nested compact marker (`- - 1`) recurses naturally: the caller
/// re-invokes this same lookahead for the *inner* `-` too, so a mapping
/// arbitrarily many `- ` markers deep (`- - a: 1`) still finds its `:` and
/// colors `a`, while a purely scalar nested sequence (`- - 1\n  - 2`) does
/// not color the inner marker - real, but narrow and, like the rest of
/// this colorizer, not oracle-matched against real yq's own (differently
/// coded) `-C` output, so left as a known residual gap rather than chased
/// further (#785).
fn compact_item_opens_with_key(rest_of_line: &str) -> bool {
    let mut quote: Option<char> = None;
    let mut chars = rest_of_line.chars();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                if c == '\\' {
                    chars.next(); // Skip the escaped character.
                } else if c == q {
                    quote = None;
                }
            }
            None => match c {
                '\n' => return false,
                '"' | '\'' => quote = Some(c),
                ':' => return true,
                _ => {}
            },
        }
    }
    false
}

/// Colorize YAML output (basic ANSI colors).
fn colorize_yaml(yaml: &str) -> String {
    let mut result = String::with_capacity(yaml.len() * 2);
    let mut in_string = false;
    let mut escape_next = false;
    let mut at_key_start = true;
    let mut in_key = false;

    let mut chars = yaml.chars();
    while let Some(c) = chars.next() {
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
                // Only close a color span that's actually open (`in_key`)
                // - otherwise this `:` belongs to a value that was never
                // colored (e.g. a quoted-key's colon after the closing
                // quote already reset color, or a `:` inside an uncolored
                // token), and unconditionally emitting a reset here would
                // write an orphaned `\x1b[0m` with no matching open.
                if in_key {
                    result.push_str("\x1b[0m");
                }
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
                // A real-yq "compact" block-sequence item (`- key: value`,
                // #785) puts its mapping's first key directly after `- `
                // on the same line, instead of on a fresh line (which
                // already re-triggers `at_key_start` via the `\n` arm
                // above) - see `compact_item_opens_with_key`'s doc comment.
                at_key_start = compact_item_opens_with_key(chars.as_str());
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

        // `select(...)` never changes position - a truthy output is always
        // the input node unchanged - and `eval_generic.rs`'s own
        // `Builtin::Select` arm already forwards the incoming cursor as-is
        // (`OneCursor`/`ManyCursor`) rather than rebuilding a value. Routing
        // it here rather than through `evaluate_yaml_cursor`'s unconditional
        // `to_owned()` DOM path is what keeps duplicate mapping keys (and
        // their comments) intact, matching `FirstExpr`/`LastExpr` above
        // (#631) and `-S`/`--tab` (#733) - `select()` had the same latent
        // gap (#796).
        Expr::Builtin(Builtin::Select(_)) => true,

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
            Builtin::HaltErrorCode(e)
            | Builtin::Has(e)
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

    if args.front_matter.is_some() {
        if args.document.is_some() {
            anyhow::bail!("--front-matter and --doc are incompatible");
        }
        if args.null_input {
            anyhow::bail!("--front-matter and --null-input are incompatible");
        }
        if args.raw_input {
            anyhow::bail!("--front-matter and --raw-input are incompatible");
        }
        if args.input_format == InputFormat::Json {
            // Front matter is YAML by definition (the `---`-fenced header),
            // so `apply_front_matter` always forces `InputFormat::Yaml` once
            // a mode is set -- reject an explicit, contradictory
            // `--input-format json` instead of silently overriding it.
            anyhow::bail!("--front-matter and --input-format json are incompatible");
        }
        if args.front_matter == Some(FrontMatterMode::Extract) && args.inplace {
            // `extract` never captures a body to reattach (only `process`
            // does, see `apply_front_matter`), so `--inplace` would
            // overwrite the file with just the transformed front matter,
            // silently discarding everything after the closing fence.
            anyhow::bail!(
                "--front-matter=extract and --inplace are incompatible (would discard the file's body); use --front-matter=process instead"
            );
        }
        if args.front_matter == Some(FrontMatterMode::Process) {
            if args.slurp {
                anyhow::bail!("--front-matter=process and --slurp are incompatible");
            }
            // `output_value` treats anything other than `Yaml` as JSON
            // output (including `Auto`, which has no YAML/Markdown-file
            // detection of its own) -- so the guard must reject everything
            // but `Yaml`, not just the explicit `Json` variant, or `-o auto`
            // silently slips through and wraps a JSON body in `---` fences.
            if args.output_format != OutputFormat::Yaml {
                anyhow::bail!(
                    "--front-matter=process requires YAML output (got -o/--output-format {})",
                    if args.output_format == OutputFormat::Json {
                        "json"
                    } else {
                        "auto"
                    }
                );
            }
        }
    }

    if args.split_exp.is_some() {
        if args.slurp {
            anyhow::bail!("--split-exp and --slurp are incompatible");
        }
        if args.inplace {
            anyhow::bail!("--split-exp and --inplace are incompatible");
        }
        if args.raw_input {
            anyhow::bail!("--split-exp with --raw-input is not yet supported");
        }
        if args.front_matter.is_some() {
            anyhow::bail!("--split-exp and --front-matter are incompatible");
        }
    }

    if args.eval_all {
        if args.slurp {
            anyhow::bail!(
                "--eval-all and --slurp are incompatible: both combine inputs into a single evaluation"
            );
        }
        if args.inplace {
            anyhow::bail!("--eval-all and --inplace are incompatible");
        }
        if args.raw_input {
            anyhow::bail!("--eval-all and --raw-input are incompatible");
        }
        if args.split_exp.is_some() {
            anyhow::bail!("--eval-all and --split-exp are incompatible");
        }
        if args.front_matter.is_some() {
            anyhow::bail!("--eval-all and --front-matter are incompatible");
        }
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
    let args_value = build_args_var(&context);
    let mut all_vars: Vec<(&str, &OwnedValue)> =
        context.named.iter().map(|(k, v)| (k.as_str(), v)).collect();
    all_vars.push(("ARGS", &args_value));
    program.expr = jq::substitute_vars(&program.expr, all_vars.iter().copied());

    // Parse the --split-exp expression, if given, once up front, applying
    // the same --arg/--argjson/$ARGS substitution as the main filter --
    // otherwise a filename expression referencing `--arg`-provided values
    // (e.g. an output directory prefix) fails as an undefined variable even
    // though the same flag works for the main filter. `$index` is bound
    // separately, per output result (see `write_split_result`).
    let split_expr: Option<Expr> = args
        .split_exp
        .as_deref()
        .map(|s| {
            jq::parse_program_with_mode(s, jq::ParserMode::Yq)
                .map(|p| jq::substitute_vars(&p.expr, all_vars.iter().copied()))
                .map_err(|e| anyhow::anyhow!("parse error in --split-exp expression: {e}"))
        })
        .transpose()?;

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
    // #978: `args.input_format` is the *raw*, unresolved CLI flag - it's
    // `Auto` whenever the user relies on `.json`-extension detection
    // instead of typing `--input-format json` explicitly, which
    // `resolve_input_format` (used everywhere else JSON input is handled)
    // still correctly resolves to `Json`. The M2 gates below need that
    // same resolution, not the raw flag, or a bare `succinctly yq '.'
    // file.json` (arguably the more common way to hit this than the
    // explicit flag) would still leak. Conservative for a mixed-format
    // multi-file `--inplace`/`--doc` invocation (`-i '.' a.yaml b.json`):
    // this disables M2 for every file, not just the JSON one(s), since
    // `is_m2_streamable`/`can_slurp_fast_path` are single, run-wide
    // booleans with no per-file M2-vs-DOM switch to hook into instead.
    let any_input_is_json = if input_files.is_empty() {
        resolve_input_format(args.input_format, None) == InputFormat::Json
    } else {
        input_files.iter().any(|f| {
            resolve_input_format(args.input_format, Some(Path::new(f))) == InputFormat::Json
        })
    };
    // every M2 fast path below streams a scalar's *value* straight
    // from the parsed cursor rather than materializing an OwnedValue, and
    // neither streamer (JSON or YAML output) reliably discards a JSON
    // number's own decimal-point/exponent spelling the way the DOM path's
    // canonicalize_json_numbers now does - `can_json_fast_path`'s
    // stream_resolved_scalar_as_json still keeps a computed whole-number
    // float's trailing `.0` (matching #918's YAML convention, wrong for
    // JSON input: `1e2` -> `100.0`, real yq gives `100`), and
    // `can_yaml_fast_path`'s streamer echoes the raw source span outright
    // (`1.50` stays `1.50`). Real yq never preserves a JSON-sourced
    // number's literal spelling at all, so JSON input (explicit flag or
    // resolved from a `.json` extension - see `any_input_is_json` above)
    // always falls back to the (already-fixed) DOM path instead.
    //
    // Known trade-off (#996): the DOM path's `OwnedValue::Object` is
    // `IndexMap`-backed and can't represent duplicate keys, unlike M2
    // streaming, which never materializes a map at all and so happened to
    // preserve them - `{"a":1,"a":2}` correctly stays that way through M2
    // (matching real yq) but collapses to `{"a":2}` once routed through
    // here. #442 already solved this for YAML input with its own
    // `OwnedValue`-avoiding fast path; a proper fix for JSON input needs
    // an equivalent (most likely teaching the shared streaming formatters
    // in `src/yaml/light.rs` to canonicalize a JSON-sourced number
    // in-stream, restoring M2 eligibility instead of disabling it) rather
    // than the DOM-fallback trade-off made here.
    let is_m2_streamable = can_use_m2_streaming(&program.expr) && !any_input_is_json;
    // pretty_print isn't implemented by the cursor streamers, so it still
    // falls back to the DOM path, unchanged, rather than silently ignoring
    // the flag the way compact mode already does today. `sort_keys` and
    // `tab` (#733) are implemented directly by the cursor/lazy streamers —
    // see `IndentSpec` and the `sort_keys` parameter threaded through
    // `DocumentCursor::stream_json`/`stream_yaml` and `GenericResult::
    // stream_json`/`stream_yaml` — so routing them through the DOM would
    // needlessly reintroduce #442's duplicate-mapping-key collapse
    // (`OwnedValue::Object`'s `IndexMap` cannot represent duplicate keys).
    // pretty_print's DOM-path rendering is currently indistinguishable from
    // the default (style preservation doesn't exist yet — #707); routing it
    // through DOM now gives it a single seam to implement real
    // style-clearing against once #707 lands (#705).
    //
    // `can_stream_pretty_or_colored` gates every M2 fast path below,
    // stdout, `--slurp`, and `--inplace` alike: color, when requested, is
    // handled by `stream_maybe_colored` buffering the (still
    // duplicate-key-safe) cursor-streamed output and running it through the
    // existing `colorize_yaml`/`colorize_json` post-processors, rather than
    // falling back to the DOM/IndexMap path that would collapse duplicate
    // mapping keys (#442, #748, #809).
    let can_stream_pretty_or_colored = !args.pretty_print;
    let can_json_fast_path = is_m2_streamable
        && (output_config.compact || (can_stream_pretty_or_colored && !args.ascii_output))
        && output_config.output_format == OutputFormat::Json
        && !args.null_input
        && !args.raw_input
        && !args.slurp
        && !args.inplace
        && args.front_matter.is_none()
        && split_expr.is_none()
        && !args.eval_all
        && context.named.is_empty();
    let can_yaml_fast_path = is_m2_streamable
        && (output_config.compact || can_stream_pretty_or_colored)
        && output_config.output_format == OutputFormat::Yaml
        && !args.null_input
        && !args.raw_input
        && !args.slurp
        && !args.inplace
        && args.front_matter.is_none()
        && split_expr.is_none()
        && !args.eval_all
        && context.named.is_empty();
    let can_fast_path = can_json_fast_path || can_yaml_fast_path;

    // `--inplace`'s own copy of the M2 gate (#478): identical conditions to
    // `can_json_fast_path`/`can_yaml_fast_path` above, but requiring
    // `args.inplace` instead of excluding it. Kept as a separate pair rather
    // than folding into the gate above because inplace output targets a
    // per-file buffer (then `fs::write`), not the shared stdout `writer`
    // the block below uses — the two loops have different write targets and
    // per-file `---` separator resets, so they stay as distinct branches
    // that happen to share the same underlying `stream_cursor!` macro. Color
    // no longer excludes the fast path here (#809): the inplace write loop
    // below passes `false` as `stream_cursor!`'s `$use_color` argument (and
    // shadows `output_config.use_color` to `false` for its own DOM-branch
    // writes), so `-C` reaching the fast path still never writes ANSI to
    // disk, but no longer forces a fallback to the duplicate-key-collapsing
    // DOM path either.
    let can_inplace_json_fast_path = is_m2_streamable
        && (output_config.compact || (can_stream_pretty_or_colored && !args.ascii_output))
        && output_config.output_format == OutputFormat::Json
        && !args.null_input
        && !args.raw_input
        && args.inplace
        && args.front_matter.is_none()
        && context.named.is_empty();
    let can_inplace_yaml_fast_path = is_m2_streamable
        && (output_config.compact || can_stream_pretty_or_colored)
        && output_config.output_format == OutputFormat::Yaml
        && !args.null_input
        && !args.raw_input
        && args.inplace
        && args.front_matter.is_none()
        && context.named.is_empty();
    let can_inplace_fast_path = can_inplace_json_fast_path || can_inplace_yaml_fast_path;

    // `--slurp`'s fast path (#478) is narrower than the two gates above:
    // scoped to plain identity only (`is_identity`, not the broader
    // `is_m2_streamable` set), since a non-trivial filter over the slurped
    // array needs real evaluation. `-o json --slurp` still uses the slow
    // DOM path below — an explicit, documented scope limit rather than a
    // silent gap. Color no longer excludes this gate either (#809): the
    // call site now wraps `stream_yaml_sequence` in `stream_maybe_colored`,
    // same as the stdout/inplace paths.
    let can_slurp_fast_path = is_identity
        && can_stream_pretty_or_colored
        && output_config.output_format == OutputFormat::Yaml
        // #978: same JSON-literal-spelling leak as can_yaml_fast_path above
        // - this gate is YAML-output-only (see its own doc comment), so no
        // JSON-output sibling to worry about excluding correctly here.
        && !any_input_is_json
        && !args.null_input
        && !args.raw_input
        && args.front_matter.is_none()
        && context.named.is_empty();

    // Indent width/unit for the fast path's streamers. `--tab` always means
    // exactly one tab per level, ignoring `-I`'s numeric value — matching
    // `OutputConfig::indent_str`'s DOM-path behavior above. YAML's `-I0` is
    // otherwise a special case: real `yq` treats it as "use the library
    // default" (4 spaces), and succinctly's existing (pre-#442) compact-YAML
    // fast path hardcodes 2 regardless of `-I` — preserved as-is here since
    // reconciling that mismatch is a separate, out-of-scope issue. Every
    // other value threads through directly, matching what the DOM path
    // already produces (verified against `-I1` through `-I6`). JSON has no
    // such quirk: `-I0` means compact/flow for both real yq and succinctly
    // today.
    let indent_unit: char = if args.tab { '\t' } else { ' ' };
    let yaml_indent_spaces: usize = if args.tab {
        1
    } else if args.indent == 0 {
        2
    } else {
        args.indent as usize
    };
    let json_indent_spaces: usize = if args.tab { 1 } else { args.indent as usize };
    let yaml_indent = IndentSpec {
        width: yaml_indent_spaces,
        unit: indent_unit,
    };
    let json_indent = IndentSpec {
        width: json_indent_spaces,
        unit: indent_unit,
    };
    let sort_keys = args.sort_keys;

    // Helper macro to stream cursor results (avoiding closure borrow issues).
    // Defined here (rather than inside the `if can_fast_path` block below) so
    // both the stdout M2 path and `--inplace`'s fast path (#478) can reuse
    // it. `$is_yaml`/`$use_color` are threaded through explicitly rather than
    // closing over `can_yaml_fast_path`/`output_config.use_color` by name:
    // both of those differ per call site (`--inplace` always forces color
    // off, #809), and — unlike `yaml_doc_streamed`/`any_truthy`/`sink`,
    // which each call site's enclosing block declares fresh right before
    // invoking this macro — `output_config` already existed when this macro
    // was *defined*, so a `let output_config = ...` shadow introduced later,
    // at a given call site, is invisible to a bare (non-`$`) reference here:
    // `macro_rules!` resolves such free identifiers against whatever was in
    // scope at definition time, not at each expansion site. `$fragment:expr`
    // parameters don't have that problem — they always evaluate in the
    // caller's own scope — hence passing color as `$use_color:expr`.
    macro_rules! stream_cursor {
        ($cursor:expr, $writer:expr, $is_yaml:expr, $doc_streamed:expr, $use_color:expr) => {{
            if $is_yaml {
                // M2 YAML path: YAML output streaming
                if is_identity {
                    // P9 path: stream directly without evaluation.
                    // `stream_yaml_as_document` (not `stream_yaml`) since
                    // `$cursor` here is the whole document being
                    // redisplayed as itself - its own trailing comment,
                    // if any, must be kept (#710).
                    emit_yaml_doc_separator($writer, $doc_streamed, true)?;
                    stream_maybe_colored($writer, $use_color, colorize_yaml, |out| {
                        $cursor.stream_yaml_as_document(out, yaml_indent, sort_keys)
                    })?;
                    writeln!($writer)?;
                    // Streaming skips evaluation, so inspect the document
                    // value directly to keep `-e` falsy tracking (#178).
                    if args.exit_status {
                        any_truthy |= !$cursor.is_falsy();
                    }
                } else {
                    // M2 YAML path: evaluate and stream YAML results
                    let result = eval_with_cursor_using::<YqSemantics, _>(&program.expr, $cursor);
                    // `produces_output` is an exhaustive match on
                    // `GenericResult`, not a hand-maintained exclusion
                    // list — a halt/halt_error (#791) with no prior
                    // output (`GenericResult::Halt`) answers `false`
                    // there; an output-bearing halt
                    // (`GenericResult::Partial`) answers `true`.
                    emit_yaml_doc_separator($writer, $doc_streamed, result.produces_output())?;
                    let stats = stream_maybe_colored($writer, $use_color, colorize_yaml, |out| {
                        result.stream_yaml(out, yaml_indent, sort_keys, |w| w.write_str("\n"))
                    })?;
                    any_truthy |= stats.any_truthy;
                    absorb_stream_stats(&mut sink, &stats);
                }
            } else {
                // M2 path: JSON output streaming
                if is_identity {
                    // P9 path: stream directly without evaluation
                    stream_maybe_colored(
                        $writer,
                        $use_color,
                        |s| output::colorize_json(s, &ColorScheme::default()),
                        |out| $cursor.stream_json(out, json_indent, sort_keys),
                    )?;
                    writeln!($writer)?;
                    // Streaming skips evaluation, so inspect the document
                    // value directly to keep `-e` falsy tracking (#178).
                    if args.exit_status {
                        any_truthy |= !$cursor.is_falsy();
                    }
                } else {
                    // M2 path: evaluate and stream results
                    let result = eval_with_cursor_using::<YqSemantics, _>(&program.expr, $cursor);
                    let stats = stream_maybe_colored(
                        $writer,
                        $use_color,
                        |s| output::colorize_json(s, &ColorScheme::default()),
                        |out| {
                            result.stream_json(out, json_indent, sort_keys, |w| w.write_str("\n"))
                        },
                    )?;
                    any_truthy |= stats.any_truthy;
                    absorb_stream_stats(&mut sink, &stats);
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
                                &mut yaml_doc_streamed,
                                output_config.use_color
                            );
                            // See the matching check in the per-file loop below.
                            if sink.halted().is_some() {
                                break;
                            }
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
                                stream_maybe_colored(
                                    &mut writer,
                                    output_config.use_color,
                                    colorize_yaml,
                                    |out| root.stream_yaml_document(out, yaml_indent, sort_keys),
                                )?;
                            } else {
                                stream_maybe_colored(
                                    &mut writer,
                                    output_config.use_color,
                                    |s| output::colorize_json(s, &ColorScheme::default()),
                                    |out| root.stream_json_document(out, json_indent, sort_keys),
                                )?;
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
                                    &mut yaml_doc_streamed,
                                    output_config.use_color
                                );
                            }
                        }
                    }
                }
            }
        } else {
            'm2_files: for file_path in &input_files {
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
                                    &mut yaml_doc_streamed,
                                    output_config.use_color
                                );
                                // Halt outranks every remaining document and
                                // file (#791): without this, a `halt` nested
                                // inside `first(...)`/`last(...)`/a computed
                                // index — the only shapes that reach the M2
                                // path with a halt at all, see
                                // `can_use_m2_streaming` — would keep
                                // streaming further documents and files
                                // instead of stopping immediately.
                                if sink.halted().is_some() {
                                    break 'm2_files;
                                }
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
                                    stream_maybe_colored(
                                        &mut writer,
                                        output_config.use_color,
                                        colorize_yaml,
                                        |out| {
                                            root.stream_yaml_document(out, yaml_indent, sort_keys)
                                        },
                                    )?;
                                } else {
                                    stream_maybe_colored(
                                        &mut writer,
                                        output_config.use_color,
                                        |s| output::colorize_json(s, &ColorScheme::default()),
                                        |out| {
                                            root.stream_json_document(out, json_indent, sort_keys)
                                        },
                                    )?;
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
                                        &mut yaml_doc_streamed,
                                        output_config.use_color
                                    );
                                    // See the matching check in the `Sequence` arm above.
                                    if sink.halted().is_some() {
                                        break 'm2_files;
                                    }
                                }
                            }
                        }
                        global_doc_index += 1;
                    }
                }
            }
        }
    } else if args.eval_all {
        // Handle --eval-all: combine every document from every file into one
        // evaluation context, exposing `file_index`/`fileIndex`/`fi` (#715).
        // Input-gathering mirrors --slurp's DOM path (all_docs collection),
        // but tracks each document's origin file index alongside it and
        // evaluates via `eval_owned_with_file_index` instead of plain
        // `evaluate_input`, so `file_index` resolves against that side table.
        // `--front-matter` is rejected in combination above, so every
        // gathered body is `None` here.
        let input_sources = match gather_input_sources(
            &input_files,
            args.input_format,
            args.front_matter,
            args.validate,
        )? {
            GatheredSources::Sources(s) => s,
            GatheredSources::ExitCode(code) => return Ok(code),
        };

        let mut all_docs: Vec<OwnedValue> = Vec::new();
        let mut file_origin: Vec<usize> = Vec::new();
        let mut global_doc_index: usize = 0;
        for (file_idx, (bytes, format, _)) in input_sources.iter().enumerate() {
            let inputs = parse_input(bytes, *format)?;
            for input in inputs {
                let should_include = args
                    .document
                    .map_or(true, |target| target == global_doc_index);
                if should_include {
                    all_docs.push(input);
                    file_origin.push(file_idx);
                }
                global_doc_index += 1;
            }
        }

        let combined = OwnedValue::Array(all_docs);
        let query_result: QueryResult<'_, Vec<u64>> = jq::eval_owned_with_file_index::<
            Vec<u64>,
            YqSemantics,
        >(
            &program.expr, &combined, &file_origin
        );
        let results = query_result_to_owned_values(query_result, &mut sink);

        let mut split_doc_state = SplitDocState::new(has_split_doc);
        for (i, result) in results.iter().enumerate() {
            if has_split_doc {
                // The filter explicitly marks its outputs as separate
                // documents (`split_doc`); route through the same state
                // machine every other output path uses for that, or no
                // separator is ever written here at all.
                split_doc_state.write_separator(&mut writer, &output_config)?;
            } else if output_config.output_format == OutputFormat::Yaml
                && !output_config.no_doc
                && results.len() > 1
                && i > 0
            {
                // `---` BETWEEN results (not before the first) -- deliberately
                // different from --slurp's no-separator convention, since
                // eval-all is explicitly a multi-document-stream feature (#715).
                writeln!(writer, "---")?;
            }
            any_truthy |= !matches!(result, OwnedValue::Null | OwnedValue::Bool(false));
            output_value(&mut writer, result, &CommentTree::empty(), &output_config)?;
        }
    } else if let Some(split_expr) = split_expr.as_ref() {
        // Handle --split-exp: write each result to its own file (named by
        // evaluating `split_expr` against it, with `$index` bound to its
        // zero-based output index) instead of stdout. `--front-matter` is
        // rejected in combination above, so no extraction step is needed
        // here; the input-gathering below otherwise mirrors the standard
        // path at the bottom of this function.
        let mut output_index: i64 = 0;
        let mut written_split_files: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        if args.null_input {
            let results = evaluate_input(&OwnedValue::Null, &program.expr, &mut sink)?;
            // Snapshotted before the loop: if the *main* filter already
            // halted after producing several values (e.g. `1,2,3,halt`),
            // `results` is the full legitimate pre-halt prefix and every
            // element still owes its file — `sink.halted()` must not be
            // misread as "this iteration halted" until a *new* halt (from
            // this batch's own split-filename evaluation) actually occurs
            // (#791, mirrors `write_split_result`'s own `halted_before`).
            let halted_before_batch = sink.halted().is_some();
            for result in &results {
                any_truthy |= !matches!(result, OwnedValue::Null | OwnedValue::Bool(false));
                write_split_result(
                    result,
                    &CommentTree::empty(),
                    split_expr,
                    output_index,
                    &output_config,
                    &mut written_split_files,
                    &mut sink,
                )?;
                output_index += 1;
                // halt/halt_error (#791) outranks writing any further split
                // files, but only once introduced during this batch.
                if !halted_before_batch && sink.halted().is_some() {
                    break;
                }
            }
        } else {
            // `--front-matter` is rejected in combination above, so every
            // gathered body is `None` here.
            let input_sources = match gather_input_sources(
                &input_files,
                args.input_format,
                args.front_matter,
                args.validate,
            )? {
                GatheredSources::Sources(s) => s,
                GatheredSources::ExitCode(code) => return Ok(code),
            };

            let mut global_doc_index: usize = 0;
            'files: for (bytes, format, _) in &input_sources {
                match format {
                    InputFormat::Yaml | InputFormat::Auto => {
                        let doc_filter = args.document.map(|target| (target, global_doc_index));
                        // Split-exp output files carry real comments when
                        // written as YAML, same as the main output path (#710).
                        let need_comments = output_config.output_format == OutputFormat::Yaml;
                        let (doc_results, num_docs) = evaluate_yaml_direct_filtered(
                            bytes,
                            &program.expr,
                            doc_filter,
                            &mut sink,
                            need_comments,
                            args.pretty_print,
                        )?;
                        global_doc_index += num_docs;
                        for results in doc_results {
                            // See the --null-input arm above: a halt already
                            // present when this document's batch starts
                            // means every element of `results` is a
                            // legitimate pre-halt prefix that still owes its
                            // file, so only a *new* halt (from this batch's
                            // own split-filename evaluation) should stop the
                            // loop early (#791).
                            let halted_before_batch = sink.halted().is_some();
                            for (result, comments) in &results {
                                any_truthy |=
                                    !matches!(result, OwnedValue::Null | OwnedValue::Bool(false));
                                write_split_result(
                                    result,
                                    comments,
                                    split_expr,
                                    output_index,
                                    &output_config,
                                    &mut written_split_files,
                                    &mut sink,
                                )?;
                                output_index += 1;
                                // halt/halt_error (#791) outranks writing any
                                // further split files or evaluating any
                                // further documents/inputs/files.
                                if !halted_before_batch && sink.halted().is_some() {
                                    break 'files;
                                }
                            }
                        }
                        if sink.halted().is_some() {
                            break 'files;
                        }
                    }
                    InputFormat::Json => {
                        let inputs = parse_input(bytes, InputFormat::Json)?;
                        for input in inputs {
                            if let Some(target_doc) = args.document {
                                if global_doc_index != target_doc {
                                    global_doc_index += 1;
                                    continue;
                                }
                            }
                            let results = evaluate_input(&input, &program.expr, &mut sink)?;
                            // See the --null-input arm above: a pre-existing
                            // halt means every element of `results` is a
                            // legitimate pre-halt prefix still owed its file
                            // (#791).
                            let halted_before_batch = sink.halted().is_some();
                            for result in &results {
                                any_truthy |=
                                    !matches!(result, OwnedValue::Null | OwnedValue::Bool(false));
                                write_split_result(
                                    result,
                                    &CommentTree::empty(),
                                    split_expr,
                                    output_index,
                                    &output_config,
                                    &mut written_split_files,
                                    &mut sink,
                                )?;
                                output_index += 1;
                                if !halted_before_batch && sink.halted().is_some() {
                                    break 'files;
                                }
                            }
                            global_doc_index += 1;
                            if sink.halted().is_some() {
                                break 'files;
                            }
                        }
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
            output_value(&mut writer, &result, &CommentTree::empty(), &output_config)?;
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
                output_value(&mut writer, &result, &CommentTree::empty(), &output_config)?;
            }
        } else {
            // Without --slurp, process each line independently
            for line in input_content.lines() {
                let input = OwnedValue::String(line.to_string());
                let results = evaluate_input(&input, &program.expr, &mut sink)?;
                for result in results {
                    split_doc_state.write_separator(&mut writer, &output_config)?;
                    any_truthy |= !matches!(&result, OwnedValue::Null | OwnedValue::Bool(false));
                    output_value(&mut writer, &result, &CommentTree::empty(), &output_config)?;
                }
                // halt/halt_error (#791) outranks evaluating any further lines.
                if sink.halted().is_some() {
                    break;
                }
            }
        }
    } else if args.slurp {
        // Handle --slurp: collect all documents from all inputs into an array

        // Collect input sources. `--front-matter` (extract only here; process
        // mode is rejected above since a slurped array can't reattach a body
        // per input file) is applied before validation, since the raw
        // file bytes (e.g. Markdown) aren't valid standalone YAML.
        let input_sources = match gather_input_sources(
            &input_files,
            args.input_format,
            args.front_matter,
            args.validate,
        )? {
            GatheredSources::Sources(s) => s,
            GatheredSources::ExitCode(code) => return Ok(code),
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
            for (bytes, _format, _) in input_sources {
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
            // Buffer-and-colorize (#748, extended to `--slurp` by #809):
            // `stream_yaml_sequence` is generic over `core::fmt::Write`, so
            // it slots into `stream_maybe_colored` unmodified.
            stream_maybe_colored(&mut writer, output_config.use_color, colorize_yaml, |out| {
                stream_yaml_sequence(
                    cursors.iter().copied(),
                    out,
                    0,
                    yaml_indent_spaces,
                    indent_unit,
                    sort_keys,
                )
            })?;
            writeln!(writer)?;
        } else {
            let mut all_docs: Vec<OwnedValue> = Vec::new();

            // Parse all inputs and collect documents
            let mut global_doc_index: usize = 0;
            for (bytes, format, _) in &input_sources {
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
                output_value(&mut writer, &result, &CommentTree::empty(), &output_config)?;
            }
        }
    } else if args.inplace {
        // Handle --inplace: process each file and write back to it
        if input_files.is_empty() {
            anyhow::bail!("--inplace requires at least one file argument");
        }

        let mut global_doc_index: usize = 0;
        'inplace_files: for file_path in &input_files {
            let path = Path::new(file_path);
            let raw_bytes = read_file(path)?;
            let resolved_format = resolve_input_format(args.input_format, Some(path));
            let (input_bytes, format, front_matter_body) =
                apply_front_matter(raw_bytes, resolved_format, args.front_matter, file_path)?;
            if let Some(code) =
                yaml_validate_guard(&input_bytes, format, args.validate, Some(file_path))
            {
                return Ok(code);
            }

            let mut output_buffer = Vec::new();

            // `--inplace` never writes ANSI to disk (#809): shadow
            // `output_config` for the rest of this file's write so the DOM
            // branch below sees `use_color: false` regardless of `-C` (the
            // fast-path branch instead passes `false` explicitly as
            // `stream_cursor!`'s `$use_color` argument — a bare
            // `output_config.use_color` reference inside that macro would
            // resolve against the *original*, unshadowed binding, since
            // `output_config` already existed when the macro was defined).
            // Forcing color off here also closes a live bug: compact mode
            // (`-I0`) already took the fast path unconditionally (the
            // `compact ||` gate short-circuits before color is checked),
            // and nothing forced color off on that path, so
            // `-C -I0 --inplace` wrote raw ANSI bytes straight into the
            // file.
            let mut output_config = output_config.clone();
            output_config.use_color = false;

            // halt/halt_error (#791): tracks whether any *real* evaluated
            // content (not a speculatively pre-written `---` separator or
            // front-matter fence) made it into `output_buffer`, so the
            // write-back guard below can tell "this file was never really
            // considered" apart from "the buffer merely has some bytes in
            // it" — see the guard's own comment for why that distinction
            // matters.
            let any_real_output = if can_inplace_fast_path {
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
                                        &mut yaml_doc_streamed,
                                        false
                                    );
                                    // halt/halt_error (#791): matches the DOM
                                    // `--inplace` branch below — stop
                                    // streaming further documents into this
                                    // file, but still let the shared
                                    // write-back-then-break-'inplace_files
                                    // logic after this `if`/`else` run, so
                                    // the prefix already streamed is still
                                    // committed to disk.
                                    if sink.halted().is_some() {
                                        break;
                                    }
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
                                            yaml_indent,
                                            sort_keys,
                                        )
                                        .map_err(|_| anyhow::anyhow!("Write error"))?;
                                    } else {
                                        root.stream_json_document(
                                            &mut FmtWriter(&mut buf_writer),
                                            json_indent,
                                            sort_keys,
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
                                        &mut yaml_doc_streamed,
                                        false
                                    );
                                }
                            }
                            global_doc_index += 1;
                        }
                    }
                    buf_writer.flush()?;
                }
                // This branch never pre-writes speculative separator/fence
                // bytes (front matter forces the DOM branch below, and its
                // own `---` separators are only emitted after a document has
                // actually streamed real content), so the buffer's emptiness
                // already reflects whether any real output happened.
                !output_buffer.is_empty()
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

                // `--front-matter=process` opens its own leading `---` fence
                // here; the body's closing fence + body text are appended
                // after `buf_writer` is dropped, below.
                if front_matter_body.is_some() {
                    writeln!(buf_writer, "---")?;
                }

                // halt/halt_error (#791): tracks real per-document output
                // only, separately from the speculative `---`/fence bytes
                // that may already be sitting in `buf_writer` above.
                let mut any_real_output = false;
                let mut split_doc_state = SplitDocState::new(has_split_doc);
                for (local_idx, input) in inputs.iter().enumerate() {
                    let current_doc_index = global_doc_index + local_idx;
                    // Apply --doc filter if specified
                    if let Some(target_doc) = args.document {
                        if current_doc_index != target_doc {
                            continue;
                        }
                    }

                    let results = evaluate_input(input, &program.expr, &mut sink)?;
                    // `output_config` was already shadowed to `use_color:
                    // false` above — reused here rather than building a
                    // second, parallel no-color config.
                    let mut doc_had_output = false;
                    for result in results {
                        // For regular multi-doc (without split_doc), add ---
                        // before each doc's first real output — not
                        // unconditionally before the doc is even evaluated.
                        // A doc whose query yields no values gets no
                        // separator either side (#175), matching the M2 fast
                        // path's already-correct behavior above; writing it
                        // eagerly left a dangling `---` whenever a doc
                        // produced zero output, whether from an ordinary
                        // empty filter or from a halt partway through (#791).
                        if !doc_had_output
                            && !has_split_doc
                            && output_config.output_format == OutputFormat::Yaml
                            && !output_config.no_doc
                            && is_multi_doc
                            && front_matter_body.is_none()
                        {
                            writeln!(buf_writer, "---")?;
                        }
                        doc_had_output = true;
                        split_doc_state.write_separator(&mut buf_writer, &output_config)?;
                        any_truthy |=
                            !matches!(&result, OwnedValue::Null | OwnedValue::Bool(false));
                        output_value(
                            &mut buf_writer,
                            &result,
                            &CommentTree::empty(),
                            &output_config,
                        )?;
                        any_real_output = true;
                    }
                    // halt/halt_error (#791): still write this file's buffer
                    // so far (below, matching the "prefix already output
                    // survives" rule elsewhere), but evaluate no further
                    // documents in this file or any other.
                    if sink.halted().is_some() {
                        break;
                    }
                }
                buf_writer.flush()?;
                global_doc_index += inputs.len();
                any_real_output
            };

            if let Some(body) = &front_matter_body {
                output_buffer.extend_from_slice(b"---");
                output_buffer.extend_from_slice(front_matter::body_line_ending(body));
                output_buffer.extend_from_slice(body);
            }

            // Write the output back to the file — unless a `halt`/
            // `halt_error` fired before this file produced any *real* output
            // at all. A halt aborts the whole process; this file was never
            // fully considered, so leaving its original content in place is
            // safer than truncating it to reflect an evaluation that never
            // really finished (#791) — `halt` used as an early-exit guard
            // clause is a natural way to trigger this under `-i`.
            //
            // Gated on `any_real_output`, not `output_buffer.is_empty()`:
            // the DOM branch above can pre-write a `--front-matter=process`
            // fence into `output_buffer` before evaluating the document that
            // follows it, so a halt with zero real output can still leave
            // the buffer non-empty. Checking emptiness directly would let
            // that speculative prefix defeat this guard and truncate the
            // file down to just the prefix. (The multi-doc `---` separator
            // is no longer speculative — it's written only after a document
            // has actually produced its first output, same as the M2 fast
            // path above.)
            //
            // Deliberately narrower than "output is empty": real yq (v4.53.3,
            // verified live) truncates a file to reflect a filter that
            // legitimately produces no output for it, e.g. `-i 'select(false)'`
            // or `-i 'del(.)'` both empty the file — so only a halt gets this
            // protection, not ordinary empty output. This is a deliberate,
            // tested product choice for `-i` specifically (see
            // `test_inplace_halt_before_any_output_does_not_truncate_file`
            // and its neighbors): unlike every other halt-propagation fix in
            // this codebase, "should halt-caused emptiness in one document
            // of a file still truncate the file because an earlier document
            // legitimately produced nothing" is not dictated by any jq
            // semantics `halt`/`halt_error` must uphold — mikefarah/yq (the
            // real-yq oracle used elsewhere in this codebase) does not even
            // parse `halt`/`if`/`then`/`end` the same way, so there is no
            // external contract to defer to here, only this project's own
            // considered choice to err toward preserving user data whenever
            // a halt is involved.
            if any_real_output || sink.halted().is_none() {
                std::fs::write(path, &output_buffer)
                    .with_context(|| format!("failed to write to file: {}", path.display()))?;
            }

            // halt/halt_error (#791) outranks editing any further files.
            if sink.halted().is_some() {
                break 'inplace_files;
            }
        }
    } else {
        // Standard path: evaluate inputs
        // For YAML inputs, use direct evaluation to preserve position metadata
        // For JSON inputs, use the OwnedValue path

        // Collect input sources with their bytes and formats. `--front-matter`
        // is applied before validation, since the raw file bytes (e.g.
        // Markdown) aren't valid standalone YAML; `process` mode's body is
        // carried alongside each source for reattachment in the output loop
        // below.
        let input_sources = match gather_input_sources(
            &input_files,
            args.input_format,
            args.front_matter,
            args.validate,
        )? {
            GatheredSources::Sources(s) => s,
            GatheredSources::ExitCode(code) => return Ok(code),
        };

        // Process all inputs first to collect results, then determine multi-doc status
        // This avoids double-parsing YAML for document counting
        // Each entry in all_results is a Vec of document results from one file.
        // Each result carries its parallel CommentTree (issue #710); JSON
        // input has none, so it's paired with CommentTree::empty().
        let mut all_results: Vec<Vec<Vec<ResultWithComments>>> = Vec::new();
        let mut global_doc_index: usize = 0;
        'collect: for (bytes, format, _) in &input_sources {
            match format {
                InputFormat::Yaml | InputFormat::Auto => {
                    // Use direct YAML evaluation to preserve position metadata
                    // Filter at evaluation time to avoid evaluating (and printing errors for)
                    // documents that don't match the --doc filter
                    let doc_filter = args.document.map(|target| (target, global_doc_index));
                    // JSON output never reads `CommentTree` (see
                    // `output_value`'s JSON branch), so don't build one (#710).
                    let need_comments = output_config.output_format == OutputFormat::Yaml;
                    let (doc_results, num_docs) = evaluate_yaml_direct_filtered(
                        bytes,
                        &program.expr,
                        doc_filter,
                        &mut sink,
                        need_comments,
                        args.pretty_print,
                    )?;
                    global_doc_index += num_docs;
                    all_results.push(doc_results);
                    // halt/halt_error (#791) outranks evaluating any further
                    // files — whatever was collected so far still prints below.
                    if sink.halted().is_some() {
                        break 'collect;
                    }
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
                        json_results.push(
                            results
                                .into_iter()
                                .map(|v| (v, CommentTree::empty()))
                                .collect::<Vec<_>>(),
                        );
                        global_doc_index += 1;
                        // halt/halt_error (#791): stop evaluating further
                        // inputs in this file (and, below, further files) —
                        // whatever was collected so far still prints below.
                        if sink.halted().is_some() {
                            break;
                        }
                    }
                    all_results.push(json_results);
                    // halt/halt_error (#791) outranks evaluating any further
                    // files — matches the sibling YAML arm above.
                    if sink.halted().is_some() {
                        break 'collect;
                    }
                }
            }
        }

        // Count total documents from collected results (after filtering)
        let total_docs: usize = all_results.iter().map(std::vec::Vec::len).sum();
        let is_multi_doc = total_docs > 1;

        // Output all results with proper separators
        // For split_doc: add --- BETWEEN each result (not before first)
        // For regular multi-doc: add --- before each document's results
        // For --front-matter=process: each file's own leading/closing ---
        // fences wrap its transformed front matter, followed by its
        // untouched body (carried alongside `input_sources`, gathered above).
        let mut split_doc_state = SplitDocState::new(has_split_doc);
        for (file_idx, doc_results) in all_results.into_iter().enumerate() {
            let front_matter_body = input_sources
                .get(file_idx)
                .and_then(|(_, _, body)| body.as_ref());
            if front_matter_body.is_some() {
                writeln!(writer, "---")?;
            }
            for results in doc_results {
                // Add document separator in YAML mode for multi-doc (before each doc's results)
                if !has_split_doc
                    && output_config.output_format == OutputFormat::Yaml
                    && !output_config.no_doc
                    && is_multi_doc
                    && front_matter_body.is_none()
                {
                    writeln!(writer, "---")?;
                }
                for (result, comments) in results {
                    split_doc_state.write_separator(&mut writer, &output_config)?;
                    any_truthy |= !matches!(&result, OwnedValue::Null | OwnedValue::Bool(false));
                    output_value(&mut writer, &result, &comments, &output_config)?;
                }
            }
            if let Some(body) = front_matter_body {
                let line_ending = front_matter::body_line_ending(body);
                writer.write_all(b"---")?;
                writer.write_all(line_ending)?;
                writer.write_all(body)?;
                // A body with no trailing line break would otherwise run
                // straight into the next file's opening fence (or whatever
                // follows), corrupting the stream -- ensure one separates
                // them, matching this body's own line-ending convention.
                if !body.is_empty() && !body.ends_with(b"\n") {
                    writer.write_all(line_ending)?;
                }
            }
        }
    }

    writer.flush()?;

    // halt/halt_error (#791) outranks everything below — every branch above
    // stops evaluating further input as soon as it's requested, but still
    // finishes writing whatever output it already had buffered/collected.
    if let Some(code) = sink.halted() {
        return Ok(code);
    }

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
        assert_eq!(
            emit_yaml_value(&nan, &CommentTree::empty(), &config, "", false),
            ".nan"
        );

        let pos_inf = OwnedValue::NumberLiteral(NumberRepr::Float(f64::INFINITY), "1e400".into());
        assert_eq!(
            emit_yaml_value(&pos_inf, &CommentTree::empty(), &config, "", false),
            ".inf"
        );

        let neg_inf =
            OwnedValue::NumberLiteral(NumberRepr::Float(f64::NEG_INFINITY), "-1e400".into());
        assert_eq!(
            emit_yaml_value(&neg_inf, &CommentTree::empty(), &config, "", false),
            "-.inf"
        );
    }

    /// `in_flow: true` is never reached through any CLI-observable path
    /// today (`output_value`'s only call site always starts at `false`, and
    /// nothing downstream re-enters flow style) - `can_use_m2_streaming`'s
    /// doc comment notes there's no `--flow`-style output flag yet. Call the
    /// private helper directly, mirroring the NaN/Infinity test above, to
    /// pin the flow-style Array/Object arms' comment threading (#710):
    /// `comments.at_index`/`comments.field` must recurse correctly even
    /// though flow style never appends a trailing comment of its own (see
    /// `emit_yaml_value`'s own doc comment for why).
    #[test]
    fn test_emit_yaml_value_flow_style_threads_comments_without_appending_them() {
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

        let mut obj = IndexMap::new();
        obj.insert("k".to_string(), OwnedValue::Int(1));
        let value = OwnedValue::Array(vec![OwnedValue::Object(obj)]);

        let mut obj_comments = IndexMap::new();
        obj_comments.insert(
            "k".to_string(),
            CommentTree::Leaf(Some("# k trailing".to_string()), ""),
        );
        let comments = CommentTree::Array(
            None,
            "",
            vec![CommentTree::Object(
                Some("# obj trailing".to_string()),
                "",
                obj_comments,
                IndexMap::new(),
            )],
        );

        // Flow style renders compactly and drops every trailing comment,
        // whether on the nested object or its field - unlike block style.
        assert_eq!(
            emit_yaml_value(&value, &comments, &config, "", true),
            "[{k: 1}]"
        );
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
        eval_yaml_with_comments(bytes, expr)
            .into_iter()
            .map(|(v, _)| v)
            .collect()
    }

    /// Like [`eval_yaml`], but keeps each result's parallel `CommentTree`
    /// (issue #710) instead of discarding it.
    fn eval_yaml_with_comments(bytes: &[u8], expr: &Expr) -> Vec<ResultWithComments> {
        let (groups, _) = evaluate_yaml_direct_filtered(
            bytes,
            expr,
            None,
            &mut ErrorSink::default(),
            true,
            false,
        )
        .unwrap();
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
