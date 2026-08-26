//! Output helpers shared by the jq and yq CLI runners.
//!
//! Exit codes, JSON string escaping, JSON pretty-printing, ANSI colorization
//! (including `JQ_COLORS` support), and build-configuration diagnostics.

// Aliased: this module already has an `escape_json_body` of its own, which
// picks *which* convention to use; the library's runs a chosen writer.
use succinctly::jq::escape::{
    escape_json_body as run_escaper, write_json_body_jq, write_json_body_jq_ascii,
    write_json_body_yq, write_json_body_yq_ascii,
};
use succinctly::jq::eval_generic::to_owned as generic_to_owned;
use succinctly::jq::{
    assert_value_tree_depth, format_number_jq_compat, nonfinite_display_string, EvalError,
    JqSemantics, NumberRepr, OwnedValue, StreamError,
};
use succinctly::json::JsonIndex;
use succinctly::yaml::format_float_with_fraction;
pub use succinctly::yaml::format_float_yq;

/// Materialize a complete JSON document into an `OwnedValue`, preserving
/// each number's exact source spelling (`1.500` stays `1.500`, `1e100`
/// stays exponent notation) the way a document read from the program's
/// primary input already does -- unlike routing through
/// `serde_json::Value`, whose `Number` variant round-trips only through
/// Rust's own `f64`/`i64` `Display` and loses that spelling (#1058).
///
/// `bytes` must already be known-valid JSON with no trailing content --
/// this crate's own semi-indexer is deliberately lenient (`42 garbage`
/// parses as `42` with the rest silently ignored, #284) and performs no
/// validation of its own. Callers that accept raw external text (a CLI
/// argument, a file) need their own strict pre-validation pass first; see
/// `jq_runner::parse_json_value`'s doc comment for why that pass can't
/// simply reuse the `serde_json::Value` it produces.
///
/// Shared by `jq_runner::parse_json_value` (`--argjson`/`--jsonargs`) and
/// `yq_runner::parse_input`'s `InputFormat::Json` arm (the primary input
/// path) -- previously two independent copies of the same three-line
/// `JsonIndex::build`/`.root()`/`generic_to_owned` sequence (#1095 review).
/// `yq_runner::parse_input` immediately discards the preserved spelling
/// this function keeps (`canonicalize_json_numbers`, #978/#999) -- kept
/// local to `yq_runner.rs` rather than a second variant here, since it's a
/// yq-CLI-specific oracle quirk this shared helper has no business knowing
/// about.
///
/// Fallible since #1247: a string that fails to decode raises instead of
/// materializing as `null`. Every caller here re-serializes an *already
/// decoded* `OwnedValue`, so in practice this cannot fail on those paths --
/// but the signature says so rather than a comment promising it, and the
/// `--argjson`/`--jsonargs` paths take genuinely external text.
pub fn json_bytes_to_owned_value(bytes: &[u8]) -> Result<OwnedValue, EvalError> {
    let index = JsonIndex::build(bytes);
    let cursor = index.root(bytes);
    generic_to_owned(&cursor.value())
}

/// Exit codes matching jq behavior
pub mod exit_codes {
    pub const SUCCESS: i32 = 0;
    // With -e: jq exits 1 when the LAST output was false/null; yq exits 1
    // when NO result was truthy (its "no matches found" failure).
    pub const FALSE_OR_NULL: i32 = 1;
    /// yq's uniform failure code. Numerically the same as [`FALSE_OR_NULL`] but
    /// for an unrelated reason: mikefarah/yq exits 1 for *any* failure, where
    /// jq reserves distinct codes per failure kind (#355).
    pub const YQ_FAILURE: i32 = 1;
    #[allow(dead_code)] // STYLE-0005: complete jq exit-code set; not all emitted yet
    pub const USAGE_ERROR: i32 = 2; // Usage problem or system error
    pub const COMPILE_ERROR: i32 = 3; // jq program compile error
    pub const NO_OUTPUT: i32 = 4; // With -e, no valid result produced (jq-only; yq folds into 1)
    /// Uncaught runtime error (and bare `halt_error`). jq exits 5 so that a
    /// failed filter is distinguishable from a successful one in a pipeline.
    pub const RUNTIME_ERROR: i32 = 5;
}

/// Which tool's diagnostic conventions to follow.
///
/// The two upstreams disagree, and both are drop-in targets for us:
///
/// | | jq 1.7.1 | mikefarah/yq v4 |
/// |---|---|---|
/// | text | `jq: error (at <stdin>:1): boom` | `Error: boom` |
/// | position marker | yes | no |
/// | `(not a string)` marker | yes | no |
/// | exit code | 5 | 1 |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagStyle {
    Jq,
    Yq,
}

impl DiagStyle {
    /// Process exit code for an uncaught evaluation error in this style.
    pub fn error_exit_code(self) -> i32 {
        match self {
            Self::Jq => exit_codes::RUNTIME_ERROR,
            Self::Yq => exit_codes::YQ_FAILURE,
        }
    }
}

/// Where an input value came from, for jq's `(at <file>:<line>)` marker.
///
/// jq reports the line on which the input value *ends*, 1-based, and falls back
/// to `<stdin>` when reading a pipe and `<unknown>` under `-n` (there is no
/// input to point at).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InputLocation {
    /// Source file, or `None` for stdin.
    pub file: Option<String>,
    /// 1-based line, or `None` when there is no input to point at (`-n`).
    pub line: Option<usize>,
}

impl InputLocation {
    /// A location with no input to point at — jq prints `<unknown>` (e.g. `-n`).
    pub fn unknown() -> Self {
        Self {
            file: None,
            line: None,
        }
    }

    /// A location in `file` (or stdin when `None`) at 1-based `line`.
    pub fn at(file: Option<&str>, line: usize) -> Self {
        Self {
            file: file.map(str::to_string),
            line: Some(line),
        }
    }
}

impl core::fmt::Display for InputLocation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.line {
            // Matches `print_validation_error`'s `<stdin>` convention, minus the
            // column: jq's uncaught-error marker carries a line only.
            Some(line) => match &self.file {
                Some(path) => write!(f, "{path}:{line}"),
                None => write!(f, "<stdin>:{line}"),
            },
            None => write!(f, "<unknown>"),
        }
    }
}

/// Collects uncaught evaluation errors from the runners.
///
/// Evaluation *continues* after an uncaught error — jq reports the diagnostic
/// and moves to the next input — so the failure has to be remembered rather
/// than returned. `hit()` then drives the process exit code (#355).
///
/// One definition shared by both runners: the jq and yq paths previously each
/// carried their own `eprintln!`, which is how the two drifted from upstream
/// independently.
#[derive(Debug, Default)]
pub struct ErrorSink {
    hit: bool,
    report_count: usize,
    halt: Option<i32>,
}

impl ErrorSink {
    /// Report an uncaught evaluation error and mark the run as failed.
    pub fn report(&mut self, style: DiagStyle, err: &EvalError, at: &InputLocation) {
        self.emit(style, &err.message, err.payload_is_not_a_string(), at);
    }

    /// Records a `halt`/`halt_error` request with its exit code (#791).
    ///
    /// Unlike `report`/`report_break`, this is not a diagnostic: no message
    /// is printed here (`halt_error`'s stderr write already happened inside
    /// the evaluator, and bare `halt` prints nothing), and `hit`/
    /// `report_count` are left untouched — `halt` outranks every other exit
    /// code path (uncaught errors, `-e`) rather than participating in their
    /// bookkeeping. First halt seen wins; callers are expected to stop
    /// evaluating further input immediately after this is set, so a second
    /// call should never happen in practice.
    pub fn request_halt(&mut self, exit_code: i32) {
        self.halt = self.halt.or(Some(exit_code));
    }

    /// The exit code requested by `halt`/`halt_error` during this run, if
    /// any. Once set, it takes precedence over every other exit code path.
    pub fn halted(&self) -> Option<i32> {
        self.halt
    }

    /// Report an error surfaced by a streaming operation ([`StreamError`]).
    pub fn report_stream(&mut self, style: DiagStyle, err: &StreamError, at: &InputLocation) {
        self.emit(style, &err.message, err.not_a_string, at);
    }

    /// Report a `break` that escaped its label — an uncaught error like any other.
    pub fn report_break(&mut self, style: DiagStyle, label: &str, at: &InputLocation) {
        self.emit(style, &format!("break ${label} not in label"), false, at);
    }

    fn emit(&mut self, style: DiagStyle, message: &str, not_a_string: bool, at: &InputLocation) {
        match style {
            DiagStyle::Jq => {
                let marker = if not_a_string { " (not a string)" } else { "" };
                eprintln!("jq: error (at {at}){marker}: {message}");
            }
            // yq carries neither marker; `Error:` matches the prefix already
            // used for its "no matches found" failure.
            DiagStyle::Yq => eprintln!("Error: {message}"),
        }
        self.hit = true;
        self.report_count += 1;
    }

    /// Whether any uncaught error was reported during the run.
    pub fn hit(&self) -> bool {
        self.hit
    }

    /// How many uncaught errors have been reported so far. Unlike `hit()`
    /// (sticky for the whole run, once true never false again), this lets a
    /// caller detect whether *this specific call* reported anything, by
    /// comparing the count before and after -- `hit()` alone can't tell
    /// "just reported" from "reported earlier, unrelated to this call" once
    /// any prior error has already flipped it (#715 follow-up: this is what
    /// `write_split_result` needs to avoid double-reporting).
    pub fn report_count(&self) -> usize {
        self.report_count
    }
}

/// Flush `writer`'s already-buffered output, then return `err` (#1673).
///
/// A multi-file/multi-document run can hit a real error after `writer`
/// already holds output from earlier documents/files; relying on
/// `Drop`'s own best-effort flush would silently swallow a write failure
/// instead of propagating it. If the flush itself also fails, `err` is
/// not discarded in favor of the flush error -- it's kept as the
/// returned error's primary message, with the flush failure layered on
/// as additional context.
pub fn flush_then_err<W: std::io::Write, T>(
    writer: &mut W,
    err: anyhow::Error,
) -> anyhow::Result<T> {
    if let Err(flush_err) = writer.flush() {
        return Err(err.context(format!("(also failed to flush output: {flush_err})")));
    }
    Err(err)
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

/// Escape a JSON string body using jq's convention.
///
/// Returns the escaped body without surrounding quotes; callers add them. The
/// escaping itself lives in `succinctly::jq::escape`, the one place either
/// convention is defined — see `write_json_body_jq` for the full table.
pub fn escape_json_string(s: &str) -> String {
    run_escaper(write_json_body_jq, s)
}

/// [`escape_json_string`], also escaping non-ASCII as \uXXXX — jq's `-a` mode.
///
/// Returns the escaped body without surrounding quotes; callers add them.
pub fn escape_json_string_ascii(s: &str) -> String {
    run_escaper(write_json_body_jq_ascii, s)
}

/// Escape a JSON string body using yq's control-char rules.
///
/// Matches `mikefarah/yq`: only `"`, `\`, and C0 controls (`< 0x20`) are
/// escaped — with `\t`/`\n`/`\r` short forms and `\u00xx` for the rest. Unlike
/// [`escape_json_string`] (jq style), backspace/form-feed stay as
/// `\u0008`/`\u000c` (not `\b`/`\f`), and DEL (`0x7f`) plus the C1 controls
/// (`0x80..=0x9f`) are emitted raw. Returns the body without surrounding quotes.
pub fn escape_json_string_yq(s: &str) -> String {
    run_escaper(write_json_body_yq, s)
}

/// yq-style escaping (see [`escape_json_string_yq`]) that also escapes
/// non-ASCII as `\uXXXX`, for yq's ASCII output mode.
///
/// Returns the escaped body without surrounding quotes; callers add them.
pub fn escape_json_string_ascii_yq(s: &str) -> String {
    run_escaper(write_json_body_yq_ascii, s)
}

/// Which tool's control-character escaping convention [`format_json`] uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlEscape {
    /// jq style: `\b`/`\f` short escapes and DEL escaped as `\u00xx`; the C1
    /// controls are left raw, as jq leaves them (#385). See
    /// [`escape_json_string`].
    Jq,
    /// yq style: backspace/form-feed as `\u0008`/`\u000c`, DEL and C1 controls
    /// left raw. See [`escape_json_string_yq`].
    Yq,
}

/// How to render finite floats with no fractional part.
#[derive(Clone, Copy, Debug)]
pub enum FloatStyle {
    /// Rust's shortest representation: `1.0` prints as `1` (jq).
    Shortest,
    /// Keep a trailing `.0` on whole floats in i64 range: `1.0` prints as `1.0` (yq).
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
    /// Control-character escaping convention (jq vs yq).
    pub control_escape: ControlEscape,
    /// Whether the source document was JSON (only meaningful alongside
    /// `control_escape: Yq` — see the `Float` arm's own `json_sourced`
    /// branch below). A JSON-sourced float never keeps a decimal point in
    /// output, computed or not, compact or pretty (#978, #1398) — unlike
    /// `float_style`, which only distinguishes compact/pretty for *jq*
    /// mode (yq's own compact/pretty JSON output always agree with each
    /// other on float formatting; see [`format_float_yq`]'s doc comment).
    pub json_sourced: bool,
}

/// Escape a JSON string body per the opts' control-escape style and ASCII mode.
fn escape_json_body(s: &str, opts: &JsonFormatOpts) -> String {
    match (opts.control_escape, opts.ascii) {
        (ControlEscape::Jq, false) => escape_json_string(s),
        (ControlEscape::Jq, true) => escape_json_string_ascii(s),
        (ControlEscape::Yq, false) => escape_json_string_yq(s),
        (ControlEscape::Yq, true) => escape_json_string_ascii_yq(s),
    }
}

/// A JSON-sourced float's yq-mode display: a plain, bare `f64::to_string()`
/// with no forced decimal point and no scientific-notation threshold --
/// real yq's own JSON-input convention (#978/#1398), distinct from
/// `format_float_yq`'s own (non-JSON-sourced) threshold/fraction rules.
/// Named and shared rather than a hand-copied `f.to_string()` at each call
/// site, matching `jq_bare_float_display`'s own precedent for the
/// identical class of problem (CLAUDE.md's #106 lesson) -- `format_json_impl`'s
/// `Float` arm and its `NumberLiteral` arm (#1498, the same rule reached
/// through a reconstructed literal) both need this exact formatter.
fn json_sourced_float_display(f: f64) -> String {
    f.to_string()
}

/// Format a value as JSON text (compact or pretty, per `opts`).
pub fn format_json(value: &OwnedValue, opts: &JsonFormatOpts) -> String {
    format_json_impl(value, opts, 0)
}

/// Recursive JSON formatter behind [`format_json`].
///
/// Panics past `succinctly::jq::MAX_VALUE_TREE_DEPTH` levels of nesting
/// (#1005) — see that constant's own doc comment for why. `value` is the
/// filter's evaluated output, constructible via `reduce`/`foreach`/etc.
/// with no adversarial document involved; `serde_json`'s own input-depth
/// limit (used elsewhere on this eager-output path) never sees it.
fn format_json_impl(value: &OwnedValue, opts: &JsonFormatOpts, level: usize) -> String {
    assert_value_tree_depth(level);
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
            if f.is_nan() {
                "null".to_string() // JSON doesn't support NaN
            } else if f.is_infinite() {
                if opts.control_escape == ControlEscape::Yq {
                    // yq mode: still "null" -- real yq's own Go
                    // encoding/json refuses to marshal Infinity at all and
                    // errors instead, so this is a deliberate, open design
                    // question left unresolved here, not a parity bug
                    // (#1087's own scope note).
                    "null".to_string()
                } else {
                    // jq mode: a computed Infinity has no source literal to
                    // echo, so it renders jq's own DBL_MAX text instead of
                    // "null" (#1087, confirmed live against jq 1.7.1: `null
                    // | infinite` is `1.7976931348623157e+308`). Reuses
                    // #1075's `nonfinite_display_string` rather than a
                    // fourth hand-rolled copy of the same split (`value.rs`,
                    // `jq_runner.rs`'s two `LiteralFormatter` impls).
                    nonfinite_display_string::<JqSemantics>(*f).to_string()
                }
            } else if opts.control_escape == ControlEscape::Yq && opts.json_sourced {
                // A JSON-sourced float never keeps a decimal point, in any
                // output mode (#978, #1398) -- see `json_sourced_float_display`.
                json_sourced_float_display(*f)
            } else if opts.control_escape == ControlEscape::Yq {
                // yq mode: scientific notation past yq's magnitude threshold
                // (#997), decimal-with-fraction otherwise, regardless of
                // compact/pretty -- real yq's Float formatting doesn't
                // distinguish the two (`float_style` only matters for jq
                // mode below).
                format_float_yq(*f)
            } else {
                match opts.float_style {
                    FloatStyle::Shortest => f.to_string(),
                    // Whole floats keep their decimal point at any magnitude;
                    // the old `<= i64::MAX` guard silently dropped it above
                    // that, disagreeing with the YAML writers (issue #169).
                    FloatStyle::PreserveWholeFloat => format_float_with_fraction(*f),
                }
            }
        }
        OwnedValue::NumberLiteral(repr, literal) => {
            if value.as_f64().is_some_and(f64::is_nan) {
                "null".to_string() // JSON doesn't support NaN
            } else if opts.control_escape == ControlEscape::Yq {
                match repr {
                    // #1498 review: `json_sourced_float_display` (plain
                    // `f64::to_string()`) renders an infinite `f` as
                    // `inf`/`-inf`, not valid JSON -- the `Float` arm above
                    // avoids this by special-casing `is_infinite()` before
                    // its own `json_sourced` branch ever runs. No live path
                    // reconstructs an *infinite* `NumberLiteral` for
                    // JSON-sourced document input today (`to_json_for_reindex`'s
                    // unparseable sentinel gets intercepted back to a plain
                    // `Float` on reparse, never boxed into a `NumberLiteral`
                    // -- see that function's own doc comment) -- but a
                    // query-*text* literal like `1e400` reaches this same
                    // arm via the parser directly, independent of the
                    // reindex bridge, and `json_sourced` is a document-level
                    // flag that doesn't distinguish the two origins. Guarded
                    // here defensively, matching the `Float` arm's own
                    // `"null"` answer, rather than relying on that
                    // cross-file invariant to keep holding.
                    NumberRepr::Float(f) if opts.json_sourced && f.is_infinite() => {
                        "null".to_string()
                    }
                    // #1498: a `NumberLiteral` can still reach here for
                    // JSON-sourced input despite `to_owned_canonicalizing_numbers`
                    // stripping it at parse time -- `--eval-all`'s
                    // `eval_owned_input` reindex round-trip (serialize the
                    // already-stripped `OwnedValue` back to JSON text, then
                    // re-parse it through the library's own literal-preserving
                    // `to_owned`, #918) reconstructs one. Same rule as the
                    // `Float` arm above either way: a JSON-sourced *finite*
                    // float never keeps a decimal point.
                    NumberRepr::Float(f) if opts.json_sourced => json_sourced_float_display(*f),
                    // Int literals have no such spelling ambiguity, and a
                    // non-`json_sourced` float has no rule to apply here at
                    // all -- both echo the source spelling verbatim (#1008),
                    // matching real yq's documented byte-for-byte literal
                    // preservation regardless of finiteness (confirmed live:
                    // a genuine document `1e999` literal echoes verbatim in
                    // real yq too).
                    _ => literal.to_string(),
                }
            } else {
                // jq mode keeps `format_number_jq_compat`'s reformatting
                // unchanged, which itself already reformats a non-finite
                // literal's mantissa correctly (#1083/#1087) rather than
                // assuming finiteness. This PR fixed its `-0.0`-sign-loss
                // bug (also #1008, since widening
                // `is_preservable_float_literal` newly exposed it via
                // YAML), but it has other pre-existing divergences from
                // real jq, unrelated and left alone here, e.g. `0.1e1` ->
                // `1E+0` here vs real jq's `1`.
                format_number_jq_compat(literal.as_bytes())
            }
        }
        OwnedValue::String(s) => {
            format!("\"{}\"", escape_json_body(s, opts))
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
                        let key = escape_json_body(k, opts);
                        format!("\"{}\":{}", key, format_json_impl(v, opts, level + 1))
                    })
                    .collect();
                format!("{{{}}}", items.join(","))
            } else {
                let items: Vec<String> = entries
                    .iter()
                    .map(|(k, v)| {
                        let key = escape_json_body(k, opts);
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

    /// The two upstreams disagree on the exit code for an uncaught error, and
    /// both runners route through this mapping. Pinning it here keeps the jq
    /// and yq call sites from drifting apart independently (#355).
    #[test]
    fn test_error_exit_code_per_style() {
        assert_eq!(DiagStyle::Jq.error_exit_code(), 5, "jq exits 5");
        assert_eq!(DiagStyle::Yq.error_exit_code(), 1, "mikefarah/yq exits 1");
        // Distinct from -e's codes, which describe a *successful* falsy result.
        assert_ne!(DiagStyle::Jq.error_exit_code(), exit_codes::NO_OUTPUT);
        assert_ne!(DiagStyle::Jq.error_exit_code(), exit_codes::FALSE_OR_NULL);
    }

    /// A bare `halt_error` (no explicit exit code) is documented to exit with
    /// the same code as an uncaught error in the same mode (#791) — but the
    /// two constants live in different crates (`JqSemantics`/`YqSemantics`'s
    /// `DEFAULT_HALT_ERROR_CODE` in the library, `DiagStyle::error_exit_code`
    /// here in the binary) linked only by a comment on each side, with
    /// nothing that would catch one drifting from the other. Pinning the
    /// equality directly, rather than each side only asserting against its
    /// own hardcoded expectation, is what actually enforces the invariant.
    #[test]
    fn test_bare_halt_error_default_matches_uncaught_error_exit_code() {
        use succinctly::jq::{EvalSemantics, JqSemantics, YqSemantics};

        assert_eq!(
            JqSemantics::DEFAULT_HALT_ERROR_CODE,
            DiagStyle::Jq.error_exit_code(),
            "bare halt_error in jq mode must exit like an uncaught error"
        );
        assert_eq!(
            YqSemantics::DEFAULT_HALT_ERROR_CODE,
            DiagStyle::Yq.error_exit_code(),
            "bare halt_error in yq mode must exit like an uncaught error"
        );
    }

    #[test]
    fn test_input_location_display() {
        assert_eq!(InputLocation::at(Some("a.json"), 2).to_string(), "a.json:2");
        // No file means stdin, matching `print_validation_error`'s convention.
        assert_eq!(InputLocation::at(None, 1).to_string(), "<stdin>:1");
        // `-n`: no input to point at.
        assert_eq!(InputLocation::unknown().to_string(), "<unknown>");
    }

    /// jq flags a raised payload that is not a string; internal errors, which
    /// carry no payload, are message-shaped and never flagged.
    #[test]
    fn test_not_a_string_marker_tracks_the_payload() {
        assert!(!EvalError::new("expected object, got number").payload_is_not_a_string());
        assert!(!EvalError::from_value(OwnedValue::String("boom".into())).payload_is_not_a_string());
        assert!(EvalError::from_value(OwnedValue::Null).payload_is_not_a_string());
        assert!(EvalError::from_value(OwnedValue::Int(42)).payload_is_not_a_string());
        // The rendered message is unchanged by retaining the payload.
        assert_eq!(
            EvalError::from_value(OwnedValue::String("boom".into())).message,
            "boom"
        );
        assert_eq!(EvalError::from_value(OwnedValue::Int(42)).message, "42");
    }

    #[test]
    fn test_error_sink_starts_clean_and_latches() {
        let mut sink = ErrorSink::default();
        assert!(!sink.hit(), "a run with no error must not fail");
        sink.report(
            DiagStyle::Jq,
            &EvalError::new("boom"),
            &InputLocation::at(None, 1),
        );
        assert!(sink.hit(), "an uncaught error must fail the run");
    }

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
            control_escape: ControlEscape::Jq,
            json_sourced: false,
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
            control_escape: ControlEscape::Jq,
            json_sourced: false,
        };
        assert_eq!(format_json(&value, &opts(FloatStyle::Shortest)), "1");
        assert_eq!(
            format_json(&value, &opts(FloatStyle::PreserveWholeFloat)),
            "1.0"
        );
        // Non-whole floats keep the shortest form under both styles.
        let frac = OwnedValue::Float(1.5);
        assert_eq!(format_json(&frac, &opts(FloatStyle::Shortest)), "1.5");
        assert_eq!(
            format_json(&frac, &opts(FloatStyle::PreserveWholeFloat)),
            "1.5"
        );
    }

    /// Oracle-verified against real yq v4.53.3 (#997): threshold boundaries
    /// on both sides of zero, sign handling, and the `e+NN`/`e-NN` spelling.
    #[test]
    fn test_format_float_yq_997() {
        assert_eq!(format_float_yq(0.0), "0.0");
        assert_eq!(format_float_yq(-0.0), "-0.0");
        // In-range: decimal, always with a fractional part.
        assert_eq!(format_float_yq(150000.0), "150000.0");
        assert_eq!(format_float_yq(0.00015), "0.00015");
        assert_eq!(format_float_yq(1.5), "1.5");
        // Past the threshold: scientific, lowercase `e`, signed, exponent
        // padded to at least 2 digits.
        assert_eq!(format_float_yq(1_500_000.0), "1.5e+06");
        assert_eq!(format_float_yq(0.000015), "1.5e-05");
        assert_eq!(format_float_yq(-1_500_000.0), "-1.5e+06");
        assert_eq!(format_float_yq(1e100), "1e+100");
        assert_eq!(format_float_yq(1e-100), "1e-100");
    }

    /// yq mode's `OwnedValue::Float` arm must use [`format_float_yq`]
    /// regardless of `float_style`/compact-vs-pretty, matching real yq's
    /// own behavior (#997) -- distinct from the jq-mode matrix in
    /// [`test_format_json_float_styles`] above, which stays on the
    /// `float_style` path untouched.
    #[test]
    fn test_format_json_yq_mode_computed_float_scientific_notation_997() {
        let opts = |float_style| JsonFormatOpts {
            indent: "",
            sort_keys: false,
            ascii: false,
            float_style,
            control_escape: ControlEscape::Yq,
            json_sourced: false,
        };
        let huge = OwnedValue::Float(1e100);
        assert_eq!(format_json(&huge, &opts(FloatStyle::Shortest)), "1e+100");
        assert_eq!(
            format_json(&huge, &opts(FloatStyle::PreserveWholeFloat)),
            "1e+100"
        );
        // In-range whole float keeps its `.0` under both styles in yq mode
        // (unlike jq mode's `Shortest`, which drops it).
        let whole = OwnedValue::Float(150000.0);
        assert_eq!(format_json(&whole, &opts(FloatStyle::Shortest)), "150000.0");
        assert_eq!(
            format_json(&whole, &opts(FloatStyle::PreserveWholeFloat)),
            "150000.0"
        );
    }

    #[test]
    fn test_escape_json_string_control_and_specials() {
        assert_eq!(escape_json_string("a\\b"), "a\\\\b");
        assert_eq!(escape_json_string("\x08\x0C"), "\\b\\f");
        assert_eq!(escape_json_string("\r\t"), "\\r\\t");
        // Other C0 controls fall back to \uXXXX, and so does DEL.
        assert_eq!(escape_json_string("\x01"), "\\u0001");
        assert_eq!(escape_json_string("\u{7f}"), "\\u007f");
        // C1 controls (U+0080..=U+009F) do NOT: jq emits them raw, and only
        // `char::is_control()` — which this used to branch on — calls them
        // controls. Pinned against jq-1.7.1 (#385):
        //
        //     $ printf '"\302\205"' | jq -r tojson | od -An -c
        //         "  302 205   "  \n
        assert_eq!(escape_json_string("\u{85}"), "\u{85}");
        assert_eq!(escape_json_string("\u{80}\u{9f}"), "\u{80}\u{9f}");
        // Non-ASCII passes through unescaped.
        assert_eq!(escape_json_string("café"), "café");
    }

    #[test]
    fn test_escape_json_string_ascii_escapes_non_ascii() {
        // Shared escape arms match the non-ASCII escaper.
        assert_eq!(escape_json_string_ascii("say \"hi\""), "say \\\"hi\\\"");
        assert_eq!(
            escape_json_string_ascii("a\\b\x08\x0C\r\t\n"),
            "a\\\\b\\b\\f\\r\\t\\n"
        );
        assert_eq!(escape_json_string_ascii("\x01"), "\\u0001");
        // BMP characters escape as a single \uXXXX unit.
        assert_eq!(escape_json_string_ascii("é"), "\\u00e9");
        // Astral characters escape as a UTF-16 surrogate pair.
        assert_eq!(escape_json_string_ascii("😀"), "\\ud83d\\ude00");
    }

    #[test]
    fn test_escape_json_string_yq_matches_mikefarah_yq() {
        // Backspace/form-feed use the long \u00xx form (NOT jq's \b/\f) — #262.
        assert_eq!(escape_json_string_yq("\x08\x0C"), "\\u0008\\u000c");
        // \t/\n/\r keep their short forms.
        assert_eq!(escape_json_string_yq("\t\n\r"), "\\t\\n\\r");
        // Quotes/backslashes escape as usual.
        assert_eq!(escape_json_string_yq("a\"\\b"), "a\\\"\\\\b");
        // Other C0 controls fall back to \u00xx.
        assert_eq!(
            escape_json_string_yq("\x00\x07\x0b\x1b"),
            "\\u0000\\u0007\\u000b\\u001b"
        );
        // DEL (0x7f) and C1 controls (0x80..=0x9f) are emitted RAW, like yq.
        assert_eq!(escape_json_string_yq("\u{7f}"), "\u{7f}");
        assert_eq!(escape_json_string_yq("\u{85}"), "\u{85}");
        assert_eq!(escape_json_string_yq("\u{80}\u{9f}"), "\u{80}\u{9f}");
        // Printable ASCII and non-ASCII pass through unescaped.
        assert_eq!(escape_json_string_yq("café"), "café");
    }

    #[test]
    fn test_escape_json_string_ascii_yq_escapes_non_ascii() {
        // Quote/backslash and the \n/\r/\t short forms escape as usual.
        assert_eq!(
            escape_json_string_ascii_yq("a\"\\b\n\r\t"),
            "a\\\"\\\\b\\n\\r\\t"
        );
        // Same control-char rules as escape_json_string_yq...
        assert_eq!(escape_json_string_ascii_yq("\x08\x0C"), "\\u0008\\u000c");
        assert_eq!(escape_json_string_ascii_yq("\u{7f}"), "\u{7f}"); // DEL stays raw (ASCII)
                                                                     // ...but non-ASCII (including C1) escapes as \uXXXX.
        assert_eq!(escape_json_string_ascii_yq("\u{85}"), "\\u0085");
        assert_eq!(escape_json_string_ascii_yq("é"), "\\u00e9");
        assert_eq!(escape_json_string_ascii_yq("😀"), "\\ud83d\\ude00");
    }

    #[test]
    fn test_format_json_yq_ascii_routes_through_ascii_yq_escaper() {
        // yq control-escape + ASCII mode escapes both keys and values via
        // escape_json_string_ascii_yq: BS -> \u0008 (long form), C1/non-ASCII
        // -> \uXXXX. Exercises the (Yq, ascii) dispatch arm in format_json.
        let mut obj = IndexMap::new();
        obj.insert(
            "ké".to_string(),
            OwnedValue::String("a\x08\u{85}é".to_string()),
        );
        let opts = JsonFormatOpts {
            indent: "",
            sort_keys: false,
            ascii: true,
            float_style: FloatStyle::Shortest,
            control_escape: ControlEscape::Yq,
            json_sourced: false,
        };
        assert_eq!(
            format_json(&OwnedValue::Object(obj), &opts),
            r#"{"k\u00e9":"a\u0008\u0085\u00e9"}"#
        );
    }

    #[test]
    fn test_format_json_non_finite_floats_1087() {
        // NaN has no fallback text and stays "null" in jq mode; a computed
        // Infinity (no source literal) renders jq's own DBL_MAX text
        // instead, matching #1087's fix to `value.rs`'s `to_json` and
        // `jq_runner.rs`'s CLI formatters -- this is the third formatter
        // that needed the identical split.
        let opts = JsonFormatOpts {
            indent: "",
            sort_keys: false,
            ascii: false,
            float_style: FloatStyle::PreserveWholeFloat,
            control_escape: ControlEscape::Jq,
            json_sourced: false,
        };
        assert_eq!(format_json(&OwnedValue::Float(f64::NAN), &opts), "null");
        assert_eq!(
            format_json(&OwnedValue::Float(f64::INFINITY), &opts),
            "1.7976931348623157e+308"
        );
        assert_eq!(
            format_json(&OwnedValue::Float(f64::NEG_INFINITY), &opts),
            "-1.7976931348623157e+308"
        );

        // yq mode is deliberately left unchanged (#1087's own open design
        // question -- real yq's Go encoding/json errors on Infinity rather
        // than substituting anything).
        let yq_opts = JsonFormatOpts {
            control_escape: ControlEscape::Yq,
            json_sourced: false,
            ..opts
        };
        assert_eq!(
            format_json(&OwnedValue::Float(f64::INFINITY), &yq_opts),
            "null"
        );
    }

    #[test]
    fn test_format_json_non_finite_number_literal_1087() {
        // A `NumberLiteral` whose source text overflows f64 to infinity
        // (`1e400`) still has its own text to fall back to -- jq mode
        // echoes the mantissa-preserving reformat (`format_number_jq_compat`
        // already handles this correctly), not `DBL_MAX` text or "null" --
        // confirmed live against jq 1.7.1, `1e400 | .` echoes `1E+400`.
        let opts = JsonFormatOpts {
            indent: "",
            sort_keys: false,
            ascii: false,
            float_style: FloatStyle::Shortest,
            control_escape: ControlEscape::Jq,
            json_sourced: false,
        };
        let overflowed = OwnedValue::NumberLiteral(
            succinctly::jq::NumberRepr::Float(f64::INFINITY),
            "1e400".into(),
        );
        assert_eq!(format_json(&overflowed, &opts), "1E+400");

        // yq mode echoes the literal verbatim regardless of finiteness
        // (#1008's byte-for-byte preservation convention) -- confirmed live
        // against real yq.
        let yq_opts = JsonFormatOpts {
            control_escape: ControlEscape::Yq,
            json_sourced: false,
            ..opts
        };
        assert_eq!(format_json(&overflowed, &yq_opts), "1e400");

        // NaN still has no fallback text in either mode.
        let nan_lit =
            OwnedValue::NumberLiteral(succinctly::jq::NumberRepr::Float(f64::NAN), "nan".into());
        assert_eq!(format_json(&nan_lit, &opts), "null");
    }

    #[test]
    fn test_format_json_empty_containers() {
        let pretty = JsonFormatOpts {
            indent: "  ",
            sort_keys: false,
            ascii: false,
            float_style: FloatStyle::Shortest,
            control_escape: ControlEscape::Jq,
            json_sourced: false,
        };
        assert_eq!(format_json(&OwnedValue::Array(vec![]), &pretty), "[]");
        assert_eq!(
            format_json(&OwnedValue::Object(IndexMap::new()), &pretty),
            "{}"
        );
    }

    #[test]
    fn test_format_json_pretty_ascii_object() {
        let mut obj = IndexMap::new();
        obj.insert(
            "é".to_string(),
            OwnedValue::Array(vec![OwnedValue::String("ü".to_string())]),
        );
        let value = OwnedValue::Object(obj);
        let opts = JsonFormatOpts {
            indent: "  ",
            sort_keys: false,
            ascii: true,
            float_style: FloatStyle::Shortest,
            control_escape: ControlEscape::Jq,
            json_sourced: false,
        };
        assert_eq!(
            format_json(&value, &opts),
            "{\n  \"\\u00e9\": [\n    \"\\u00fc\"\n  ]\n}"
        );
    }

    #[test]
    fn test_colorize_json_arrays_keywords_numbers_escapes() {
        let out = colorize_json(r#"[false,12.5e+1,"a\"b",null]"#, &ColorScheme::default());
        // Array delimiters take the array color.
        assert!(
            out.starts_with("\x1b[1;39m[\x1b[0m"),
            "open bracket: {out:?}"
        );
        assert!(
            out.ends_with("\x1b[1;39m]\x1b[0m"),
            "close bracket: {out:?}"
        );
        // false and full numbers (incl. exponent) are single colored tokens.
        assert!(
            out.contains("\x1b[0;39mfalse\x1b[0m"),
            "false token: {out:?}"
        );
        assert!(
            out.contains("\x1b[0;39m12.5e+1\x1b[0m"),
            "number token: {out:?}"
        );
        // Escaped quotes inside strings do not terminate the string span.
        assert!(
            out.contains("\x1b[0;32m\"a\\\"b\"\x1b[0m"),
            "escaped string: {out:?}"
        );
    }

    #[test]
    fn test_print_build_configuration_smoke() {
        // Diagnostic output; assert it runs without panicking for both tools.
        print_build_configuration("jq");
        print_build_configuration("yq");
    }

    /// `depth` levels of single-element array nesting: `[[[...[Null]...]]]`.
    fn linear_array_nest(depth: usize) -> OwnedValue {
        let mut v = OwnedValue::Null;
        for _ in 0..depth {
            v = OwnedValue::Array(vec![v]);
        }
        v
    }

    /// #1005: `value` is a filter's evaluated output (e.g. a `reduce`
    /// accumulator growing one level per iteration), which has no
    /// adversarial *document* behind it — `format_json_impl` must
    /// independently refuse to recurse past the same limit rather than
    /// overflow the stack.
    #[test]
    fn format_json_panics_past_nesting_depth_limit_1005() {
        use succinctly::jq::MAX_VALUE_TREE_DEPTH;

        let opts = JsonFormatOpts {
            indent: "",
            sort_keys: false,
            ascii: false,
            float_style: FloatStyle::Shortest,
            control_escape: ControlEscape::Jq,
            json_sourced: false,
        };

        let under = linear_array_nest(MAX_VALUE_TREE_DEPTH - 1);
        let _ = format_json(&under, &opts);

        let over = linear_array_nest(MAX_VALUE_TREE_DEPTH);
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| format_json(&over, &opts)));
        assert!(
            result.is_err(),
            "format_json should panic at MAX_VALUE_TREE_DEPTH"
        );
    }
}
