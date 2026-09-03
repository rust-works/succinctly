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

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use super::document::{
    key_display_string, DistinctKeyCursors, DocumentFields, IndentSpec, JsonConvention,
};
use super::error::EvalError;
use super::escape::{write_json_body_jq, write_json_body_yq};
use super::value::{
    assert_value_tree_depth, format_number_jq_compat, infinite_float_preview_text,
    jq_bare_float_display, NumberRepr, OwnedValue,
};
use crate::yaml::{format_float_with_fraction, format_float_yq_yaml, format_float_yq_yaml_nested};

/// A value that can be streamed directly to output without intermediate allocation.
///
/// This trait abstracts over both cursor-based navigation (YamlCursor, JsonCursor)
/// and owned values (OwnedValue), enabling unified streaming output regardless of
/// how the value was obtained.
pub trait StreamableValue {
    /// Stream this value as JSON to the output.
    ///
    /// The output should be valid JSON without trailing newlines or separators.
    /// `indent` selects width/unit (`IndentSpec::COMPACT` for compact output);
    /// `sort_keys` sorts object keys before writing (`-S`/`--sort-keys`);
    /// `numbers` selects the finite-number-literal/escaping convention
    /// (#1576) — see [`JsonConvention`]'s own doc comment.
    fn stream_json<W: core::fmt::Write>(
        &self,
        out: &mut W,
        indent: IndentSpec,
        sort_keys: bool,
        numbers: JsonConvention,
    ) -> core::fmt::Result;

    /// Stream this value as YAML to the output.
    ///
    /// The output should be valid YAML. `indent` selects width/unit
    /// (`IndentSpec::COMPACT` for flow style); `sort_keys` sorts object keys
    /// before writing (`-S`/`--sort-keys`).
    fn stream_yaml<W: core::fmt::Write>(
        &self,
        out: &mut W,
        indent: IndentSpec,
        sort_keys: bool,
    ) -> core::fmt::Result;

    /// Check if this value is falsy (null or false).
    ///
    /// Used for `--exit-status` flag handling without requiring full
    /// materialization. `numbers` is the same convention `stream_json`
    /// renders under (#966 follow-up, review of #1576): a structurally
    /// invalid number span (`1.2.3`) sanitizes to `null` in `JsonCursor`'s
    /// own `JqCompat` output, so it must also answer falsy *here* under
    /// that convention to keep `-e`'s exit code consistent with what
    /// actually got printed -- `Preserve` echoes the same span unsanitized
    /// (still nominally a number), so it stays truthy there. Every other
    /// implementor ignores the parameter; it exists only for `JsonCursor`.
    fn is_falsy(&self, numbers: JsonConvention) -> bool;
}

/// Statistics returned from streaming operations.
///
/// Used to track output for `--exit-status` handling.
#[derive(Debug, Clone, Default)]
pub struct StreamStats {
    /// Number of values streamed.
    pub count: usize,
    /// Whether the last value was falsy (null or false).
    ///
    /// jq's `--exit-status` semantics: only the last output value counts.
    pub last_was_falsy: bool,
    /// Whether any streamed value was truthy (neither null nor false).
    ///
    /// yq's `--exit-status` semantics: exit 1 unless some result is truthy.
    pub any_truthy: bool,
    /// Diagnostic for an uncaught evaluation error, if the result was one.
    ///
    /// Streaming deliberately writes nothing to `out` for an error: the
    /// diagnostic belongs on stderr and must set the process exit code, which
    /// only the caller can do. Handing it back here keeps it off stdout (#355).
    pub error: Option<StreamError>,

    /// A `halt`/`halt_error(n)` exit code, if the result was one.
    ///
    /// Mutually exclusive with `error`: unlike an ordinary uncaught error, a
    /// halt is not a diagnostic to render — it is a request to exit the whole
    /// process with this code, immediately, with no further evaluation. A
    /// caller must check this field *before* `error` and call the CLI's own
    /// halt-request path with it, rather than reporting it through the
    /// ordinary error-diagnostic path — otherwise the real exit code is lost
    /// and a halt is misreported as a generic failure (#791).
    pub halt: Option<i32>,

    /// Whether the failure in `error` left `out` cut off mid-value (#1615).
    ///
    /// Distinct from `error.is_some()`, and the distinction is load-bearing:
    /// an ordinary uncaught evaluation error writes *nothing* to `out` (see
    /// `error`'s own note), so a caller streaming a multi-document input
    /// should report it and carry on to the next document (#355). A decode
    /// failure raised from inside a *cursor* stream is different -- the
    /// container around it is already partly written, with no newline closing
    /// it, so continuing would weld the next document's `---` onto the
    /// truncated line and produce output that reads back as valid YAML with a
    /// fabricated value. Only the streaming writers set this; a caller that
    /// ignores it keeps the old, continue-past behaviour for everything else.
    pub truncated: bool,
}

/// An uncaught evaluation error surfaced by a streaming operation.
///
/// Carries just enough to reproduce jq's diagnostic without borrowing from the
/// streamed result: the rendered message, and whether the raised payload was a
/// non-string (jq's `(not a string)` marker).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamError {
    /// Rendered error message.
    pub message: String,
    /// Whether the raised payload was something other than a string.
    pub not_a_string: bool,
}

/// A failure raised while *streaming* a document to output.
///
/// The streaming writers (`YamlCursor::stream_json`/`stream_yaml`,
/// `stream_json_as_yaml`) are built on [`core::fmt::Write`], whose only error
/// type is [`core::fmt::Error`] — it carries no message, so before #1615 a
/// scalar that would not decode could only be silently absorbed into a
/// substituted `null`/`""` (the design doc's deferred "Stage 6",
/// `docs/plan/decode-failure-routing.md`). This is that missing channel: a
/// write failure stays a write failure, and a decode failure carries the same
/// diagnosable [`EvalError`] every *materializing* route already raises, so
/// both spellings of one document give one answer.
///
/// Every existing `?` inside those writers keeps working unchanged via the
/// [`From<core::fmt::Error>`] impl below — that is the reason this is an
/// error *type* rather than an out-of-band slot threaded through each of the
/// family's recursive call sites.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamFailure {
    /// The underlying writer failed (a real I/O error, or a full buffer).
    Fmt,
    /// A scalar could not be decoded (an invalid escape, invalid UTF-8).
    ///
    /// Uncatchable by `?`/`try`/`catch`, like every other decode failure
    /// (#1620) — see [`EvalError::decode_failure`].
    Decode(EvalError),
}

impl From<core::fmt::Error> for StreamFailure {
    fn from(_: core::fmt::Error) -> Self {
        Self::Fmt
    }
}

// Deliberately no `From<StreamFailure> for core::fmt::Error`. That impl would
// let a plain `?` silently collapse a decode failure back into the
// message-less error this type exists to escape — reintroducing the exact
// swallow #1615 closes, implicitly, at any call site that happens to sit in a
// `core::fmt::Result` function. Without it, every such site is a compile
// error until someone decides what the diagnostic should do, which is the
// point.

/// The result of a streaming write that can distinguish a decode failure from
/// a writer failure. See [`StreamFailure`].
pub type StreamResult = Result<(), StreamFailure>;

impl StreamableValue for OwnedValue {
    fn stream_json<W: core::fmt::Write>(
        &self,
        out: &mut W,
        indent: IndentSpec,
        sort_keys: bool,
        numbers: JsonConvention,
    ) -> core::fmt::Result {
        match numbers {
            JsonConvention::Preserve => {
                stream_owned_value_json(self, out, 0, indent.width, indent.unit, sort_keys)
            }
            JsonConvention::JqCompat => stream_owned_value_json_jq_output(
                self,
                out,
                0,
                indent.width,
                indent.unit,
                sort_keys,
            ),
        }
    }

    fn stream_yaml<W: core::fmt::Write>(
        &self,
        out: &mut W,
        indent: IndentSpec,
        sort_keys: bool,
    ) -> core::fmt::Result {
        // A bare top-level/computed scalar result drops all of its own
        // styling (quotes) - issue #852, mirroring
        // `YamlCursor::stream_yaml_as_document`'s identical root-only
        // special case for the cursor path (`src/yaml/light.rs`).
        // `stream_owned_value_yaml` below is also the *recursive*
        // per-node renderer this same function calls for every nested
        // value, so this has to intercept only here, at the actual
        // top-level entry point - bypassing it inside
        // `stream_owned_value_yaml` itself would drop quoting from every
        // nested string field too, not just the root.
        if let Self::String(s) = self {
            return out.write_str(s);
        }
        stream_owned_value_yaml(self, out, "", indent.width, indent.unit, sort_keys)
    }

    fn is_falsy(&self, _numbers: JsonConvention) -> bool {
        matches!(self, Self::Null | Self::Bool(false))
    }
}

/// Stream an OwnedValue as JSON, escaping strings the way `yq` does.
///
/// This is what [`StreamableValue::stream_json`] runs, and so what `syq -o json`
/// emits. For the `jq` convention — which #385 established is a genuinely
/// different one, not a stricter one — see [`stream_owned_value_json_jq`].
/// `pub(crate)` (#1055): also the yq-mode convention for a value preview
/// embedded in an error message (`src/jq/error.rs`), called there in
/// always-compact form (`current_indent`/`indent_spaces` 0, `sort_keys`
/// false) the same way `stream_owned_value_json_jq` already is for jq mode
/// — this function needs no jq-mode-error-message-specific sibling of its
/// own, since it's already the general yq real-output convention #1008
/// built, and that convention is exactly what a yq-mode error preview
/// should also use.
///
/// Whole floats keep their decimal point here (`format_float_with_fraction`):
/// this is the M2 fast path for non-identity navigation queries (`.field`,
/// `.[0]`, `.[]`) in compact mode, and it must agree with the identity
/// streaming path and the OwnedValue/DOM pretty-printer, both of which
/// preserve `1.0` rather than collapsing it to `1` (issue #169).
pub(crate) fn stream_owned_value_json<W: core::fmt::Write>(
    value: &OwnedValue,
    out: &mut W,
    current_indent: usize,
    indent_spaces: usize,
    unit: char,
    sort_keys: bool,
) -> core::fmt::Result {
    stream_owned_value_json_with(
        value,
        out,
        current_indent,
        indent_spaces,
        unit,
        sort_keys,
        write_json_body_yq,
        format_float_with_fraction,
        real_output_infinite_float,
        real_output_infinite_literal,
        real_output_finite_literal,
    )
}

/// The `yq`/real-output convention for a finite `NumberLiteral` (#1008):
/// echo the document's source spelling verbatim rather than reformatting
/// it per jq's own rules -- real yq preserves a literal's exact text
/// (`1e100` stays `1e100`, `1E5` stays `1E5`) regardless of magnitude or
/// query shape, confirmed empirically against the pinned oracle. `raw` is
/// always valid UTF-8 (sourced from `OwnedValue::NumberLiteral`'s own
/// `Box<str>`), so the lossy path here is unreachable in practice.
pub(crate) fn real_output_finite_literal(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw).into_owned()
}

/// The `yq`/real-output convention for an infinite `Float` (NaN is handled
/// unconditionally, ahead of this, in `stream_owned_value_json_with` itself
/// — both conventions agree it's always `null`): RFC 8259 has no literal for
/// Infinity either, so this always renders `null` too, regardless of sign.
/// Pinned by `test_stream_json_number_literal_nan_and_infinite` below —
/// never change this without updating that test's expectation too.
fn real_output_infinite_float(_negative: bool) -> String {
    "null".to_string()
}

/// The `yq`/real-output convention for an infinite `NumberLiteral` — same
/// rule as [`real_output_infinite_float`], just for the sibling variant that
/// also carries the document's raw source text (irrelevant here, since real
/// output never echoes it for a non-finite value).
fn real_output_infinite_literal(_raw: &[u8]) -> String {
    "null".to_string()
}

/// Stream an OwnedValue as JSON, escaping strings the way `jq` does.
///
/// Used for the values `jq` embeds in its error messages, which have to read
/// back exactly as jq renders them (`src/jq/error.rs`). The difference from the
/// `yq` convention that [`StreamableValue::stream_json`] emits is three code
/// points wide — `\b`, `\f` and DEL — and is documented in
/// [`write_json_body_jq`].
///
/// Floats here stay plain `Display` output (no whole-float repair): unlike
/// `yq`, jq only shows a decimal point when the value is an unmodified
/// literal echoed from the source (handled upstream, before this function
/// ever sees an `OwnedValue`) — a *computed* whole float like `1.0 + 2.0`
/// prints as `3`, not `3.0`, in real jq.
pub fn stream_owned_value_json_jq<W: core::fmt::Write>(
    value: &OwnedValue,
    out: &mut W,
) -> core::fmt::Result {
    // Always compact: this is the jq-error-message convention, which never
    // pretty-prints.
    stream_owned_value_json_with(
        value,
        out,
        0,
        0,
        ' ',
        false,
        write_json_body_jq,
        jq_bare_float_display,
        |negative| infinite_float_preview_text(negative).to_string(),
        preview_infinite_literal,
        format_number_jq_compat,
    )
}

/// Stream an OwnedValue as JSON, escaping strings and canonicalizing number
/// literals the way real jq's own *output* does (#1576) — as opposed to
/// [`stream_owned_value_json_jq`] above, which is jq's error-message-preview
/// convention (always compact, and infinite values render jq's own preview
/// text rather than RFC 8259's `null`).
///
/// Shares [`stream_owned_value_json_jq`]'s escape table (`write_json_body_jq`)
/// and finite-number canonicalization (`format_number_jq_compat`) — both
/// already proven correct against real jq via the jq CLI's existing
/// non-streaming `print_json`/`escape_json_string` path
/// (`src/bin/succinctly/output.rs`) — but pairs them with real output's
/// infinite-value rule (`null`, not a preview) and threads `current_indent`/
/// `indent_spaces`/`sort_keys` through instead of hardcoding compact/false,
/// so this can pretty-print and honor `-S` the way genuine output must.
pub(crate) fn stream_owned_value_json_jq_output<W: core::fmt::Write>(
    value: &OwnedValue,
    out: &mut W,
    current_indent: usize,
    indent_spaces: usize,
    unit: char,
    sort_keys: bool,
) -> core::fmt::Result {
    stream_owned_value_json_with(
        value,
        out,
        current_indent,
        indent_spaces,
        unit,
        sort_keys,
        write_json_body_jq,
        jq_bare_float_display,
        real_output_infinite_float,
        real_output_infinite_literal,
        format_number_jq_compat,
    )
}

/// The jq-error-message-preview convention for an infinite `Float`: unlike
/// real output, jq's own value previews aren't constrained by RFC 8259 (they
/// aren't JSON, just message text), so an infinite value with no source
/// literal of its own to echo — the `infinite`/`-infinite` builtins, or any
/// arithmetic overflow — renders as jq's actual `DBL_MAX` text instead of
/// `null` (issue #930).
///
/// (NaN still renders as `null` here too, same as real output — jq's own
/// value previews do the same, since NaN has no literal spelling to fall
/// back to either; that case is handled unconditionally in
/// `stream_owned_value_json_with` itself, ahead of ever reaching this.)
fn preview_infinite_literal(raw: &[u8]) -> String {
    // The document literal this overflowed from is right here, so unlike
    // the plain-`Float` case this can do better than `DBL_MAX` text: reuse
    // the same jq-canonical-formatting path finite `NumberLiteral`s already
    // go through — `format_number_jq_compat` now handles a non-finite input
    // via `format_overflow_literal_mantissa` instead of the finite path's
    // `log10`/`pow` (see that function's doc comment for why the split is
    // necessary), so this can call it unconditionally.
    format_number_jq_compat(raw)
}

/// Stream an OwnedValue as JSON without intermediate string allocation, using
/// `escape` for every string body and `float_fmt` for every float — the two
/// places the `yq`/jq-error conventions differ.
///
/// - `current_indent`: Current indentation level (number of `unit` characters)
/// - `indent_spaces`: `unit` characters per indentation level (0 for compact)
/// - `unit`: the character repeated `indent_spaces` times per level (`' '`
///   normally, `'\t'` for `--tab`)
/// - `sort_keys`: sort object keys before writing (`-S`/`--sort-keys`)
#[allow(clippy::too_many_arguments)] // STYLE-0004: every param is threaded through this function's own recursion (and its `write_object_entries` helper below); a struct would hide the 1:1 relationship each has to a specific streaming convention (escape/float/infinity rendering).
fn stream_owned_value_json_with<W: core::fmt::Write>(
    value: &OwnedValue,
    out: &mut W,
    current_indent: usize,
    indent_spaces: usize,
    unit: char,
    sort_keys: bool,
    escape: fn(&mut W, &str) -> core::fmt::Result,
    float_fmt: fn(f64) -> String,
    infinite_float: fn(bool) -> String,
    infinite_literal: fn(&[u8]) -> String,
    finite_literal: fn(&[u8]) -> String,
) -> core::fmt::Result {
    stream_owned_value_json_with_at_depth(
        value,
        out,
        current_indent,
        indent_spaces,
        unit,
        sort_keys,
        escape,
        float_fmt,
        infinite_float,
        infinite_literal,
        finite_literal,
        0,
    )
}

/// Panics past [`MAX_VALUE_TREE_DEPTH`](super::value::MAX_VALUE_TREE_DEPTH)
/// levels of nesting (#1021, following #1005's precedent).
#[allow(clippy::too_many_arguments)] // STYLE-0004: mirrors stream_owned_value_json_with's own suppression above.
fn stream_owned_value_json_with_at_depth<W: core::fmt::Write>(
    value: &OwnedValue,
    out: &mut W,
    current_indent: usize,
    indent_spaces: usize,
    unit: char,
    sort_keys: bool,
    escape: fn(&mut W, &str) -> core::fmt::Result,
    float_fmt: fn(f64) -> String,
    infinite_float: fn(bool) -> String,
    infinite_literal: fn(&[u8]) -> String,
    finite_literal: fn(&[u8]) -> String,
    depth: usize,
) -> core::fmt::Result {
    assert_value_tree_depth(depth);
    match value {
        OwnedValue::Null => out.write_str("null"),
        OwnedValue::Bool(true) => out.write_str("true"),
        OwnedValue::Bool(false) => out.write_str("false"),
        OwnedValue::Int(n) => write!(out, "{n}"),
        OwnedValue::Float(f) => {
            if f.is_nan() {
                // Neither convention has a literal for NaN to fall back to.
                out.write_str("null")
            } else if f.is_infinite() {
                out.write_str(&infinite_float(f.is_sign_negative()))
            } else {
                out.write_str(&float_fmt(*f))
            }
        }
        OwnedValue::NumberLiteral(repr, literal) => match repr {
            NumberRepr::Float(f) if f.is_nan() => out.write_str("null"),
            NumberRepr::Float(f) if f.is_infinite() => {
                out.write_str(&infinite_literal(literal.as_bytes()))
            }
            _ => out.write_str(&finite_literal(literal.as_bytes())),
        },
        OwnedValue::String(s) => stream_json_string(out, s, escape),
        OwnedValue::Array(arr) => {
            if arr.is_empty() {
                return out.write_str("[]");
            }
            out.write_char('[')?;
            let next_indent = current_indent + indent_spaces;
            for (i, elem) in arr.iter().enumerate() {
                if i > 0 {
                    out.write_char(',')?;
                }
                if indent_spaces > 0 {
                    out.write_char('\n')?;
                    write_indent(out, next_indent, unit)?;
                }
                stream_owned_value_json_with_at_depth(
                    elem,
                    out,
                    next_indent,
                    indent_spaces,
                    unit,
                    sort_keys,
                    escape,
                    float_fmt,
                    infinite_float,
                    infinite_literal,
                    finite_literal,
                    depth + 1,
                )?;
            }
            if indent_spaces > 0 {
                out.write_char('\n')?;
                write_indent(out, current_indent, unit)?;
            }
            out.write_char(']')
        }
        OwnedValue::Object(obj) => {
            if obj.is_empty() {
                return out.write_str("{}");
            }
            out.write_char('{')?;
            let next_indent = current_indent + indent_spaces;
            // Only pay for a Vec collect+sort when an order actually needs
            // imposing -- `sort_keys` is always `false` on the
            // error-message value-preview path (`describe()`/
            // `dump_truncated()`, capped at a handful of bytes by
            // `PreviewSink`), where collecting the *whole* object first
            // defeated that path's own early-`Err`-propagation budget
            // check the `Array` arm above already benefits from (#931).
            //
            // `ObjectEntries` (below) lets both branches converge on one
            // `write_object_entries` call site instead of two independently
            // typed ones that would otherwise need to be kept in sync by
            // hand (#963 review): a rebase-merge inconsistency during this
            // fix's own development applied a signature change to only one
            // of two near-duplicate call sites, caught only by manual
            // review before push -- exactly the drift risk a single shared
            // call site removes.
            let entries = if sort_keys {
                let mut sorted: Vec<(&String, &OwnedValue)> = obj.iter().collect();
                sorted.sort_by(|a, b| a.0.cmp(b.0));
                ObjectEntries::Sorted(sorted.into_iter())
            } else {
                ObjectEntries::Unsorted(obj.iter())
            };
            write_object_entries(
                out,
                entries,
                next_indent,
                indent_spaces,
                unit,
                sort_keys,
                escape,
                float_fmt,
                infinite_float,
                infinite_literal,
                finite_literal,
                depth + 1,
            )?;
            if indent_spaces > 0 {
                out.write_char('\n')?;
                write_indent(out, current_indent, unit)?;
            }
            out.write_char('}')
        }
    }
}

/// Either a sorted (already-materialized into a `Vec`) or unsorted
/// (borrowed directly from the map) iterator over an object's `(key,
/// value)` pairs -- see the `Object` arm above for why unifying these into
/// one type, rather than two call sites, matters.
enum ObjectEntries<'a> {
    Sorted(alloc::vec::IntoIter<(&'a String, &'a OwnedValue)>),
    Unsorted(indexmap::map::Iter<'a, String, OwnedValue>),
}

impl<'a> Iterator for ObjectEntries<'a> {
    type Item = (&'a String, &'a OwnedValue);

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Sorted(it) => it.next(),
            Self::Unsorted(it) => it.next(),
        }
    }
}

/// Write an object's `(key, value)` pairs, comma-separated and optionally
/// indented -- shared by both the sorted (`Vec`-collected) and unsorted
/// (`obj.iter()` directly) paths in `stream_owned_value_json_with`'s
/// `Object` arm so the entry-writing logic isn't duplicated between them.
#[allow(clippy::too_many_arguments)] // STYLE-0004: mirrors stream_owned_value_json_with's own suppression above -- every param is forwarded verbatim to that function's recursive call for each value.
fn write_object_entries<'a, W: core::fmt::Write>(
    out: &mut W,
    entries: impl Iterator<Item = (&'a String, &'a OwnedValue)>,
    next_indent: usize,
    indent_spaces: usize,
    unit: char,
    sort_keys: bool,
    escape: fn(&mut W, &str) -> core::fmt::Result,
    float_fmt: fn(f64) -> String,
    infinite_float: fn(bool) -> String,
    infinite_literal: fn(&[u8]) -> String,
    finite_literal: fn(&[u8]) -> String,
    depth: usize,
) -> core::fmt::Result {
    for (i, (key, value)) in entries.enumerate() {
        if i > 0 {
            out.write_char(',')?;
        }
        if indent_spaces > 0 {
            out.write_char('\n')?;
            write_indent(out, next_indent, unit)?;
        }
        stream_json_string(out, key, escape)?;
        out.write_str(if indent_spaces > 0 { ": " } else { ":" })?;
        stream_owned_value_json_with_at_depth(
            value,
            out,
            next_indent,
            indent_spaces,
            unit,
            sort_keys,
            escape,
            float_fmt,
            infinite_float,
            infinite_literal,
            finite_literal,
            depth,
        )?;
    }
    Ok(())
}

/// Stream a quoted JSON string, escaping its body with `escape`.
fn stream_json_string<W: core::fmt::Write>(
    out: &mut W,
    s: &str,
    escape: fn(&mut W, &str) -> core::fmt::Result,
) -> core::fmt::Result {
    out.write_char('"')?;
    escape(out, s)?;
    out.write_char('"')
}

/// Stream a `DocumentFields`' keys (`keys_unsorted`) as a JSON array without
/// an intermediate `Vec<String>`/`OwnedValue::Array` (#685).
///
/// The lazy counterpart of `stream_owned_value_json_with`'s `Array` arm
/// above, pulling one key at a time from `DistinctKeyCursors` instead of
/// walking a materialized slice. `collapse` is threaded through to it so a
/// repeated key collapses onto its first occurrence here too, same as every
/// other `LazyKeys` consumer (#1514) — this path is reachable from
/// `yq_runner.rs`'s M2 fast path, where `collapse` is always `false` today,
/// but the rule must still hold if a jq-mode caller ever reaches it.
/// `GenericResult::LazyKeys { sorted: false, .. }` is always the entire
/// top-level result (never nested inside another container), so unlike the
/// function it mirrors this has no `current_indent` parameter — it's always
/// 0.
///
/// `error` reports a #1194 key back to the caller (a non-stringifiable key
/// token, or the field list ending on an unpaired child) without going
/// through this function's own `core::fmt::Result` -- that channel only
/// carries `core::fmt::Error`, which has no message. On that path, whatever
/// keys were already written stay written and no closing bracket is
/// skipped, matching the `Partial`/`owned_or_stream_error` idiom elsewhere
/// in this module's callers: some output already reached `out`, the failure
/// travels back out-of-band instead of as a value on `out` (#355).
pub fn stream_lazy_keys_json<W: core::fmt::Write, F: DocumentFields>(
    fields: &F,
    collapse: bool,
    out: &mut W,
    indent: IndentSpec,
    error: &mut Option<EvalError>,
) -> core::fmt::Result {
    if fields.is_empty() {
        return out.write_str("[]");
    }
    out.write_char('[')?;
    let mut cursors = DistinctKeyCursors::new(fields, collapse);
    for (i, (key, _cursor)) in cursors.by_ref().enumerate() {
        // A key that will not *decode* is preserved via its raw source
        // span rather than silently skipped (#1642), matching
        // `DocumentFields::keys()`. A key with no stringifiable spelling at
        // all (#1194) now stops the walk and reports via `error` instead of
        // silently skipping it, matching `effective_keys`.
        let Some(key) = key_display_string(&key) else {
            *error = Some(fields.malformed_member_error());
            break;
        };
        if i > 0 {
            out.write_char(',')?;
        }
        if indent.width > 0 {
            out.write_char('\n')?;
            write_indent(out, indent.width, indent.unit)?;
        }
        stream_json_string(out, &key, write_json_body_yq)?;
    }
    // #1956: `ended_unpaired()` alone missed a malformed `,`/`:` delimiter
    // -- `is_malformed()` checks both #1194 faults this walk can find.
    if error.is_none() && cursors.is_malformed() {
        *error = Some(fields.malformed_member_error());
    }
    // #2261: trailing stray comma after a real last key (`{"a":1,}`) --
    // `cursors` already retained the last key cursor this walk saw, so this
    // is one more O(1) `next_sibling()` hop, not a further walk.
    if error.is_none() && !cursors.trailing_gap_ok(b'}') {
        *error = Some(fields.malformed_member_error());
    }
    if indent.width > 0 {
        out.write_char('\n')?;
    }
    out.write_char(']')
}

// ============================================================================
// YAML Streaming
// ============================================================================

/// The literal character width of a block-sequence item's `- ` prefix
/// (dash + one ASCII space) - always exactly 2 literal ASCII bytes,
/// independent of the configured `indent_spaces`/`unit` (`-I`/`--tab`).
/// Mirrors `crate::yaml::light`'s identically-named/valued constant (a
/// separate local copy, not shared: the two live in different crate
/// modules with no existing shared indent-primitives module, and the
/// value is definitionally fixed - `"- "` - not something that could
/// drift independently). See `stream_owned_value_yaml`'s `Array` arm
/// (#785).
const COMPACT_DASH_WIDTH: usize = 2;

/// Stream an OwnedValue as YAML without intermediate string allocation.
///
/// - `indent`: the exact indent *string* to write at the start of this
///   value's own subsequent block-style lines (not a `usize` repetition
///   count) - needed because a real-yq "compact" block-sequence item's
///   continuation offset ([`COMPACT_DASH_WIDTH`] literal ASCII spaces) and
///   an ordinary `unit`-based nesting step have to interleave in the exact
///   chronological order they were nested, which a
///   `(current_indent: usize, extra_spaces: usize)` pair collapses into a
///   fixed unit-then-extra order regardless of which happened first
///   (#785) - mirrors `crate::yaml::light::stream_yaml_value`'s identical
///   fix and its `deeper_yaml_indent`/`compact_yaml_indent` helpers (this
///   module's `deeper_indent`/`compact_indent` below).
/// - `indent_spaces`: `unit` characters per *ordinary* (non-compact)
///   indentation level (0 for flow style)
/// - `unit`: the character repeated `indent_spaces` times per level (`' '`
///   normally, `'\t'` for `--tab`)
/// - `sort_keys`: sort object keys before writing (`-S`/`--sort-keys`)
fn stream_owned_value_yaml<W: core::fmt::Write>(
    value: &OwnedValue,
    out: &mut W,
    indent: &str,
    indent_spaces: usize,
    unit: char,
    sort_keys: bool,
) -> core::fmt::Result {
    stream_owned_value_yaml_at_depth(value, out, indent, indent_spaces, unit, sort_keys, 0)
}

/// Panics past [`MAX_VALUE_TREE_DEPTH`](super::value::MAX_VALUE_TREE_DEPTH)
/// levels of nesting (#1021, following #1005's precedent).
fn stream_owned_value_yaml_at_depth<W: core::fmt::Write>(
    value: &OwnedValue,
    out: &mut W,
    indent: &str,
    indent_spaces: usize,
    unit: char,
    sort_keys: bool,
    depth: usize,
) -> core::fmt::Result {
    assert_value_tree_depth(depth);
    match value {
        OwnedValue::Null => out.write_str("null"),
        OwnedValue::Bool(true) => out.write_str("true"),
        OwnedValue::Bool(false) => out.write_str("false"),
        OwnedValue::Int(n) => write!(out, "{n}"),
        OwnedValue::Float(f) if f.is_nan() || f.is_infinite() => {
            out.write_str(super::nonfinite_display_string::<super::YqSemantics>(*f))
        }
        // Mirrors `emit_yaml_value_at_depth`'s identical depth split
        // (yq_runner.rs, issues #949 and #1090): real yq drops a computed
        // whole float's decimal point only at document-root scalar
        // position, where it suppresses every tag; nested it keeps that
        // same shortest spelling but precedes it with an explicit
        // `!!float` tag whenever the spelling would read back as an int.
        // `can_use_m2_streaming` (yq_runner.rs) has no arithmetic arm, so
        // this function is likely unreachable with a genuinely *computed*
        // bare `Float` through any current CLI path -- kept in sync with
        // the DOM path anyway for `StreamableValue::stream_yaml`'s other,
        // non-CLI callers and to avoid a silent behavioral split if a
        // future M2-eligible construct ever does carry one through (#1064
        // documents this same "unreachable but exhaustive" shape
        // elsewhere in this codebase). An untouched literal keeps its own
        // `.0` via the `NumberLiteral` arm below, unaffected by this.
        OwnedValue::Float(f) if depth == 0 => out.write_str(&format_float_yq_yaml(*f)),
        OwnedValue::Float(f) => out.write_str(&format_float_yq_yaml_nested(*f)),
        OwnedValue::NumberLiteral(NumberRepr::Float(f), _) if f.is_nan() || f.is_infinite() => {
            out.write_str(super::nonfinite_display_string::<super::YqSemantics>(*f))
        }
        OwnedValue::NumberLiteral(_, literal) => {
            // Echo verbatim (#1008), matching this function's JSON sibling
            // (`stream_owned_value_json`'s `finite_literal` hook) -- this
            // is `StreamableValue`'s yq-only YAML streamer (no `jq_runner.rs`
            // caller exists), so there is no jq convention to protect here.
            out.write_str(literal)
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
                    stream_owned_value_yaml_at_depth(elem, out, "", 0, unit, sort_keys, depth + 1)?;
                }
                out.write_char(']')
            } else {
                // Block style
                for (i, elem) in arr.iter().enumerate() {
                    if i > 0 {
                        out.write_char('\n')?;
                        out.write_str(indent)?;
                    }
                    out.write_str("- ")?;
                    // A non-empty container element renders in real yq's
                    // "compact" form (#785): sharing `- `'s own line with
                    // its first field/element rather than deferring to a
                    // fully-indented line of its own. No leading indent is
                    // written before recursing - this loop only writes one
                    // for the 2nd+ *element* of `arr` above, mirroring the
                    // same "no separate indent for the recursion's own
                    // first line" trick `light.rs`'s cursor-based
                    // renderers use.
                    if matches!(elem, OwnedValue::Array(_) | OwnedValue::Object(_))
                        && !is_empty_container(elem)
                    {
                        let child_indent = compact_indent(indent);
                        stream_owned_value_yaml_at_depth(
                            elem,
                            out,
                            &child_indent,
                            indent_spaces,
                            unit,
                            sort_keys,
                            depth + 1,
                        )?;
                    } else {
                        let child_indent = deeper_indent(indent, indent_spaces, unit);
                        stream_owned_value_yaml_at_depth(
                            elem,
                            out,
                            &child_indent,
                            indent_spaces,
                            unit,
                            sort_keys,
                            depth + 1,
                        )?;
                    }
                }
                Ok(())
            }
        }
        OwnedValue::Object(obj) => {
            if obj.is_empty() {
                return out.write_str("{}");
            }
            let mut entries: Vec<(&String, &OwnedValue)> = obj.iter().collect();
            if sort_keys {
                entries.sort_by(|a, b| a.0.cmp(b.0));
            }
            if indent_spaces == 0 {
                // Flow style
                out.write_char('{')?;
                for (i, (key, val)) in entries.into_iter().enumerate() {
                    if i > 0 {
                        out.write_str(", ")?;
                    }
                    stream_yaml_string(out, key)?;
                    out.write_str(": ")?;
                    stream_owned_value_yaml_at_depth(val, out, "", 0, unit, sort_keys, depth + 1)?;
                }
                out.write_char('}')
            } else {
                // Block style
                for (i, (key, val)) in entries.into_iter().enumerate() {
                    if i > 0 {
                        out.write_char('\n')?;
                        out.write_str(indent)?;
                    }
                    stream_yaml_string(out, key)?;
                    out.write_str(":")?;
                    // For nested containers, put on next line with extra indent
                    if matches!(val, OwnedValue::Array(_) | OwnedValue::Object(_))
                        && !is_empty_container(val)
                    {
                        out.write_char('\n')?;
                        let child_indent = deeper_indent(indent, indent_spaces, unit);
                        out.write_str(&child_indent)?;
                        stream_owned_value_yaml_at_depth(
                            val,
                            out,
                            &child_indent,
                            indent_spaces,
                            unit,
                            sort_keys,
                            depth + 1,
                        )?;
                    } else {
                        out.write_char(' ')?;
                        let child_indent = deeper_indent(indent, indent_spaces, unit);
                        stream_owned_value_yaml_at_depth(
                            val,
                            out,
                            &child_indent,
                            indent_spaces,
                            unit,
                            sort_keys,
                            depth + 1,
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

/// Write `width` copies of `unit` as indentation (`unit` is `' '` for
/// space-indented output, `'\t'` for `--tab`). JSON-only:
/// `stream_owned_value_yaml` builds its own indent *strings* instead (see
/// `deeper_indent`) - see its doc comment for why (#785).
fn write_indent<W: core::fmt::Write>(out: &mut W, width: usize, unit: char) -> core::fmt::Result {
    for _ in 0..width {
        out.write_char(unit)?;
    }
    Ok(())
}

/// Build a new YAML block-style indent string one level deeper than
/// `indent`: `indent_spaces` more copies of `unit` appended. The ordinary
/// (non-compact) nesting step for `stream_owned_value_yaml` (#785) - see
/// `compact_indent` for the other kind of step. Mirrors
/// `crate::yaml::light::deeper_yaml_indent`.
fn deeper_indent(indent: &str, indent_spaces: usize, unit: char) -> String {
    let mut next = String::with_capacity(indent.len() + indent_spaces);
    next.push_str(indent);
    for _ in 0..indent_spaces {
        next.push(unit);
    }
    next
}

/// Build a new YAML block-style indent string for a real-yq "compact"
/// block-sequence item's continuation lines: [`COMPACT_DASH_WIDTH`] more
/// literal ASCII spaces appended to `indent`, regardless of `unit` (#785).
/// Mirrors `crate::yaml::light::compact_yaml_indent`.
fn compact_indent(indent: &str) -> String {
    let mut next = String::with_capacity(indent.len() + COMPACT_DASH_WIDTH);
    next.push_str(indent);
    for _ in 0..COMPACT_DASH_WIDTH {
        next.push(' ');
    }
    next
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

/// Stream a `DocumentFields`' keys (`keys_unsorted`) as YAML without an
/// intermediate `Vec<String>`/`OwnedValue::Array` (#685).
///
/// The YAML counterpart of `stream_lazy_keys_json` above, mirroring
/// `stream_owned_value_yaml`'s `Array` arm. Keys are always plain strings,
/// never nested containers, so this omits that arm's "nested container gets
/// its own indented line" branch, and — like `stream_lazy_keys_json` — has no
/// `current_indent` parameter, since `LazyKeys { sorted: false, .. }` is
/// always the entire top-level result. `collapse` is threaded through for
/// the same reason as `stream_lazy_keys_json` (#1514): yq's own
/// `COLLAPSE_DUPLICATE_KEYS` is always `false`, so this is a no-op today,
/// but the rule must still hold if a jq-mode caller ever reaches it.
///
/// `error` -- see `stream_lazy_keys_json`'s identical parameter.
pub fn stream_lazy_keys_yaml<W: core::fmt::Write, F: DocumentFields>(
    fields: &F,
    collapse: bool,
    out: &mut W,
    indent: IndentSpec,
    error: &mut Option<EvalError>,
) -> core::fmt::Result {
    if fields.is_empty() {
        return out.write_str("[]");
    }
    if indent.is_compact() {
        // Flow style
        out.write_char('[')?;
        let mut cursors = DistinctKeyCursors::new(fields, collapse);
        for (i, (key, _cursor)) in cursors.by_ref().enumerate() {
            // Preserved via its raw source span rather than skipped on a
            // decode failure (#1642), matching `stream_lazy_keys_json`. A
            // non-stringifiable key (#1194) stops the walk and reports via
            // `error` instead of silently skipping it.
            let Some(key) = key_display_string(&key) else {
                *error = Some(fields.malformed_member_error());
                break;
            };
            if i > 0 {
                out.write_str(", ")?;
            }
            stream_yaml_string(out, &key)?;
        }
        // #1956: `ended_unpaired()` alone missed a malformed `,`/`:` delimiter
        // -- `is_malformed()` checks both #1194 faults this walk can find.
        if error.is_none() && cursors.is_malformed() {
            *error = Some(fields.malformed_member_error());
        }
        out.write_char(']')
    } else {
        // Block style
        let mut cursors = DistinctKeyCursors::new(fields, collapse);
        for (i, (key, _cursor)) in cursors.by_ref().enumerate() {
            let Some(key) = key_display_string(&key) else {
                *error = Some(fields.malformed_member_error());
                break;
            };
            if i > 0 {
                out.write_char('\n')?;
            }
            out.write_str("- ")?;
            stream_yaml_string(out, &key)?;
        }
        // #1956: `ended_unpaired()` alone missed a malformed `,`/`:` delimiter
        // -- `is_malformed()` checks both #1194 faults this walk can find.
        if error.is_none() && cursors.is_malformed() {
            *error = Some(fields.malformed_member_error());
        }
        Ok(())
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
        OwnedValue::Null
            .stream_json(
                &mut buf,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
            )
            .unwrap();
        assert_eq!(buf, "null");
    }

    /// #852: `OwnedValue::stream_yaml` (the top-level `StreamableValue`
    /// entry point, not the recursive `stream_owned_value_yaml` it calls
    /// for nested values) drops all styling for a root `String`, printing
    /// it raw even when `stream_yaml_string`'s own heuristic would have
    /// quoted it. Exercised directly here since the CLI-level tests for
    /// this fix (`-n` construction, `.a + .b` arithmetic) happen to route
    /// through `output_value`'s identical, separately-fixed special case
    /// instead of through this trait method specifically.
    #[test]
    fn test_stream_yaml_string_root_drops_ambiguous_styling_852() {
        let mut buf = String::new();
        OwnedValue::String("true".to_string())
            .stream_yaml(&mut buf, IndentSpec::COMPACT, false)
            .unwrap();
        assert_eq!(buf, "true");
    }

    #[test]
    fn test_stream_bool() {
        let mut buf = String::new();
        OwnedValue::Bool(true)
            .stream_json(
                &mut buf,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
            )
            .unwrap();
        assert_eq!(buf, "true");

        buf.clear();
        OwnedValue::Bool(false)
            .stream_json(
                &mut buf,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
            )
            .unwrap();
        assert_eq!(buf, "false");
    }

    #[test]
    fn test_stream_int() {
        let mut buf = String::new();
        OwnedValue::Int(42)
            .stream_json(
                &mut buf,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
            )
            .unwrap();
        assert_eq!(buf, "42");

        buf.clear();
        OwnedValue::Int(-123)
            .stream_json(
                &mut buf,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
            )
            .unwrap();
        assert_eq!(buf, "-123");
    }

    #[test]
    fn test_stream_float() {
        let mut buf = String::new();
        OwnedValue::Float(3.125)
            .stream_json(
                &mut buf,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
            )
            .unwrap();
        assert_eq!(buf, "3.125");
    }

    /// #1514 review: `stream_lazy_keys_json`/`stream_lazy_keys_yaml` are the
    /// M2 fast path's key-array writers. They used to walk `fields.uncons()`
    /// directly with no dedup logic at all -- every other `LazyKeys`
    /// consumer touched by #1514 was rewired through `DistinctKeyCursors`,
    /// but these two were missed, so a `collapse: true` caller would have
    /// printed a repeated key twice. Reachable today only from
    /// `yq_runner.rs`, where `collapse` is always `false`, so this is
    /// unexercised by any CLI test; pinned directly here so the rule holds
    /// if a jq-mode caller ever reaches this path.
    #[test]
    fn test_stream_lazy_keys_honors_collapse_1514() {
        use crate::json::light::{JsonIndex, StandardJson};

        let json = br#"{"b":1,"a":2,"b":3}"#;
        let index = JsonIndex::build(json);
        let StandardJson::Object(fields) = index.root(json).value() else {
            panic!("expected object");
        };

        let mut collapsed = String::new();
        let mut err = None;
        stream_lazy_keys_json(&fields, true, &mut collapsed, IndentSpec::COMPACT, &mut err)
            .unwrap();
        assert!(err.is_none(), "{err:?}");
        assert_eq!(
            collapsed, r#"["b","a"]"#,
            "collapse: true drops the repeat of \"b\""
        );

        let mut uncollapsed = String::new();
        let mut err = None;
        stream_lazy_keys_json(
            &fields,
            false,
            &mut uncollapsed,
            IndentSpec::COMPACT,
            &mut err,
        )
        .unwrap();
        assert!(err.is_none(), "{err:?}");
        assert_eq!(
            uncollapsed, r#"["b","a","b"]"#,
            "collapse: false (yq) keeps every occurrence"
        );

        let mut collapsed_yaml = String::new();
        let mut err = None;
        stream_lazy_keys_yaml(
            &fields,
            true,
            &mut collapsed_yaml,
            IndentSpec::COMPACT,
            &mut err,
        )
        .unwrap();
        assert!(err.is_none(), "{err:?}");
        assert_eq!(
            collapsed_yaml, "[b, a]",
            "the YAML writer applies the same rule"
        );

        let mut uncollapsed_yaml = String::new();
        let mut err = None;
        stream_lazy_keys_yaml(
            &fields,
            false,
            &mut uncollapsed_yaml,
            IndentSpec::COMPACT,
            &mut err,
        )
        .unwrap();
        assert!(err.is_none(), "{err:?}");
        assert_eq!(uncollapsed_yaml, "[b, a, b]");
    }

    /// #1679: a #1194 key (the format's grammar never allowed it at all, not
    /// just a decode failure) used to be silently skipped by both writers --
    /// `keys_unsorted` would come back one entry short with exit 0, while
    /// `keys`/`length` on the same document raised/counted it. Both writers
    /// now stop at the offending key and report it via `error`, keeping
    /// whatever was already written (the same `Partial` idiom
    /// `owned_or_stream_error`'s callers use) rather than a diagnostic
    /// reaching `out`.
    #[test]
    fn test_stream_lazy_keys_raises_on_non_string_key_1679() {
        use crate::json::light::{JsonIndex, StandardJson};

        let json = br#"{"b":2,123:1}"#;
        let index = JsonIndex::build(json);
        let StandardJson::Object(fields) = index.root(json).value() else {
            panic!("expected object");
        };

        let mut json_out = String::new();
        let mut err = None;
        stream_lazy_keys_json(&fields, true, &mut json_out, IndentSpec::COMPACT, &mut err).unwrap();
        let e = err.expect("a bare numeric key is not JSON");
        assert!(e.message.contains("expected string key"), "{e:?}");
        assert_eq!(
            json_out, r#"["b"]"#,
            "the key written before the fault stays written"
        );

        let mut yaml_flow = String::new();
        let mut err = None;
        stream_lazy_keys_yaml(&fields, true, &mut yaml_flow, IndentSpec::COMPACT, &mut err)
            .unwrap();
        assert!(err.is_some());
        assert_eq!(yaml_flow, "[b]");

        let mut yaml_block = String::new();
        let mut err = None;
        stream_lazy_keys_yaml(
            &fields,
            true,
            &mut yaml_block,
            IndentSpec {
                width: 2,
                unit: ' ',
            },
            &mut err,
        )
        .unwrap();
        assert!(err.is_some());
        assert_eq!(yaml_block, "- b");
    }

    /// #1679: the unpaired-tail sibling of the test above -- an object whose
    /// last child has no value to pair with. Only
    /// [`DistinctKeyCursors::ended_unpaired`] (meaningful once the walk is
    /// exhausted) can tell this apart from a clean, shorter object, so the
    /// offending key is never even yielded to the loop -- both writers must
    /// check it *after* the loop, not just inside it.
    #[test]
    fn test_stream_lazy_keys_raises_on_unpaired_field_1679() {
        use crate::json::light::{JsonIndex, StandardJson};

        for json in [&b"{invalid}"[..], &b"{\"a\"}"[..]] {
            let index = JsonIndex::build(json);
            let StandardJson::Object(fields) = index.root(json).value() else {
                panic!("expected object");
            };

            let mut json_out = String::new();
            let mut err = None;
            stream_lazy_keys_json(&fields, true, &mut json_out, IndentSpec::COMPACT, &mut err)
                .unwrap();
            err.expect("an unpaired member is not JSON");
            assert_eq!(json_out, "[]");

            let mut yaml_flow = String::new();
            let mut err = None;
            stream_lazy_keys_yaml(&fields, true, &mut yaml_flow, IndentSpec::COMPACT, &mut err)
                .unwrap();
            err.expect("an unpaired member is not JSON");
            assert_eq!(yaml_flow, "[]");

            let mut yaml_block = String::new();
            let mut err = None;
            stream_lazy_keys_yaml(
                &fields,
                true,
                &mut yaml_block,
                IndentSpec {
                    width: 2,
                    unit: ' ',
                },
                &mut err,
            )
            .unwrap();
            err.expect("an unpaired member is not JSON");
            assert_eq!(yaml_block, "");
        }
    }

    /// #1956: sibling of the unpaired-field test above, for the *other* fault
    /// [`DistinctKeyCursors::is_malformed`] covers -- a missing `,`/`:`
    /// delimiter (#1677) with an even member count, so the walk yields both
    /// keys cleanly and only `delimiter_fault()` (not `ended_unpaired()`)
    /// catches it at exhaustion. All three writers here used to check
    /// `ended_unpaired()` alone and missed this fault entirely, matching the
    /// gap `Builtin::Last`'s own arm had (`eval_generic.rs`) before this fix.
    #[test]
    fn test_stream_lazy_keys_raises_on_missing_delimiter_1956() {
        use crate::json::light::{JsonIndex, StandardJson};

        let json = br#"{"a" 1, "b": 2}"#;
        let index = JsonIndex::build(json);
        let StandardJson::Object(fields) = index.root(json).value() else {
            panic!("expected object");
        };

        let mut json_out = String::new();
        let mut err = None;
        stream_lazy_keys_json(&fields, true, &mut json_out, IndentSpec::COMPACT, &mut err).unwrap();
        err.expect("a missing ':' delimiter is not JSON");
        assert_eq!(
            json_out, r#"["a","b"]"#,
            "both keys are written before the post-loop fault check fires"
        );

        let mut yaml_flow = String::new();
        let mut err = None;
        stream_lazy_keys_yaml(&fields, true, &mut yaml_flow, IndentSpec::COMPACT, &mut err)
            .unwrap();
        err.expect("a missing ':' delimiter is not JSON");
        assert_eq!(yaml_flow, "[a, b]");

        let mut yaml_block = String::new();
        let mut err = None;
        stream_lazy_keys_yaml(
            &fields,
            true,
            &mut yaml_block,
            IndentSpec {
                width: 2,
                unit: ' ',
            },
            &mut err,
        )
        .unwrap();
        err.expect("a missing ':' delimiter is not JSON");
        assert_eq!(yaml_block, "- a\n- b");
    }

    /// The `yq` JSON convention (what the M2 fast path for `.field`/`.[0]`/
    /// `.[]` streams through) must keep a whole float's decimal point,
    /// matching the identity path and the DOM pretty-printer (issue #169).
    #[test]
    fn test_stream_json_whole_float_keeps_decimal_point() {
        let mut buf = String::new();
        OwnedValue::Float(1.0)
            .stream_json(
                &mut buf,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
            )
            .unwrap();
        assert_eq!(buf, "1.0");

        buf.clear();
        OwnedValue::Float(-0.0)
            .stream_json(
                &mut buf,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
            )
            .unwrap();
        assert_eq!(buf, "-0.0");

        buf.clear();
        OwnedValue::Array(vec![OwnedValue::Float(1.0), OwnedValue::Float(2.5)])
            .stream_json(
                &mut buf,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
            )
            .unwrap();
        assert_eq!(buf, "[1.0,2.5]");
    }

    /// Unlike the JSON M2 convention above, a bare (computed) `Float`'s YAML
    /// rendering drops the decimal point on a whole number *at document-root
    /// scalar position*: real yq's YAML output shows `2`, not `2.0`, for a
    /// genuinely computed whole float (e.g. `. + 1` on `1.0`) — issue #949.
    /// This deliberately diverges from
    /// `test_stream_json_whole_float_keeps_decimal_point` above: YAML output
    /// and JSON output of the *same* root-level computed value are not
    /// required to agree, matching real yq. An untouched literal still keeps
    /// its own `.0` via the sibling `NumberLiteral` variant, unaffected by
    /// this. A *nested* computed float (inside the array case below) keeps
    /// the same shortest spelling but gains an explicit `!!float` tag when
    /// that spelling would read back as an int — real yq's own nested
    /// disambiguation, added in #1090 (see `format_float_yq_yaml_nested`'s
    /// doc comment for the full reasoning).
    #[test]
    fn test_stream_yaml_computed_whole_float_drops_decimal_point_949() {
        let mut buf = String::new();
        OwnedValue::Float(1.0)
            .stream_yaml(&mut buf, IndentSpec::COMPACT, false)
            .unwrap();
        assert_eq!(buf, "1");

        buf.clear();
        OwnedValue::Float(-0.0)
            .stream_yaml(&mut buf, IndentSpec::COMPACT, false)
            .unwrap();
        assert_eq!(buf, "-0");

        // Nested (depth > 0): same shortest spelling as the root case above,
        // but tagged when it would otherwise read back as an int. `2.5`
        // needs no tag — its own `.` is already unambiguous.
        buf.clear();
        OwnedValue::Array(vec![OwnedValue::Float(1.0), OwnedValue::Float(2.5)])
            .stream_yaml(&mut buf, IndentSpec::COMPACT, false)
            .unwrap();
        assert_eq!(buf, "[!!float 1, 2.5]");

        // Scientific-notation threshold agrees with the DOM path's
        // `format_float_yq_yaml` (and `format_float_yq`'s own JSON
        // threshold): shortest mantissa only past exponent `>= 6`/`<= -5`.
        // This is still root position, so the threshold applies via
        // `format_float_yq_yaml` rather than the nested fallback.
        buf.clear();
        OwnedValue::Float(1_500_000.0)
            .stream_yaml(&mut buf, IndentSpec::COMPACT, false)
            .unwrap();
        assert_eq!(buf, "1.5e+06");
    }

    /// The `jq` error convention is a distinct writer (`stream_owned_value_json_jq`,
    /// used only for embedding values in `jq`'s own error messages) and must NOT
    /// gain whole-float repair: real jq shows a decimal point only when echoing an
    /// unmodified source literal (handled upstream, before `OwnedValue` exists),
    /// not for computed values — `1.0 + 2.0` prints as `3`, not `3.0`.
    #[test]
    fn test_stream_json_jq_convention_keeps_shortest_float() {
        let mut buf = String::new();
        stream_owned_value_json_jq(&OwnedValue::Float(3.0), &mut buf).unwrap();
        assert_eq!(buf, "3");
    }

    /// `stream_json` is `StreamableValue`'s yq-convention entry point --
    /// per #1008, a finite `NumberLiteral`'s source text is echoed
    /// verbatim here (real yq preserves it byte-for-byte, regardless of
    /// magnitude), not reformatted via `format_number_jq_compat`'s
    /// jq-specific rules (that remains correct only for
    /// `stream_owned_value_json_jq`, jq's own convention).
    #[test]
    fn test_stream_json_number_literal() {
        let mut buf = String::new();
        OwnedValue::NumberLiteral(NumberRepr::Float(1.2e3), "1.2e3".into())
            .stream_json(
                &mut buf,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
            )
            .unwrap();
        assert_eq!(buf, "1.2e3");
    }

    #[test]
    fn test_stream_json_number_literal_nan_and_infinite() {
        // A `NumberLiteral` whose source text overflows/underflows to a
        // non-finite float (e.g. `1e400`) must render as `null` like a plain
        // non-finite Float does, not fall through to `format_number_jq_compat`.
        let mut buf = String::new();
        OwnedValue::NumberLiteral(NumberRepr::Float(f64::NAN), "nan".into())
            .stream_json(
                &mut buf,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
            )
            .unwrap();
        assert_eq!(buf, "null");

        buf.clear();
        OwnedValue::NumberLiteral(NumberRepr::Float(f64::INFINITY), "1e400".into())
            .stream_json(
                &mut buf,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
            )
            .unwrap();
        assert_eq!(buf, "null");
    }

    /// #930: unlike real output (above), jq's error-message value previews
    /// (`stream_owned_value_json_jq`, used by `describe()`/`dump_truncated()`
    /// in `src/jq/error.rs`) aren't constrained by RFC 8259 - jq's own
    /// previews show real text for a non-finite float, not `null`. A plain
    /// `Float` carries no source literal, so an infinite one renders as
    /// jq's `DBL_MAX` text (oracle-verified: `infinite`/`-infinite`/any
    /// arithmetic overflow all match this exactly). NaN still has no
    /// literal to fall back to either way, so it stays `null` - unchanged
    /// from real output and from jq itself.
    #[test]
    fn test_stream_json_jq_float_non_finite() {
        let mut buf = String::new();
        stream_owned_value_json_jq(&OwnedValue::Float(f64::INFINITY), &mut buf).unwrap();
        assert_eq!(buf, "1.7976931348623157e+308");

        buf.clear();
        stream_owned_value_json_jq(&OwnedValue::Float(f64::NEG_INFINITY), &mut buf).unwrap();
        assert_eq!(buf, "-1.7976931348623157e+308");

        buf.clear();
        stream_owned_value_json_jq(&OwnedValue::Float(f64::NAN), &mut buf).unwrap();
        assert_eq!(buf, "null");
    }

    /// #930: a `NumberLiteral` carries its document source text, so an
    /// overflowed one gets jq's literal-preserving text instead of `DBL_MAX`
    /// text - matching how real jq distinguishes an echoed literal from a
    /// computed value. `format_number_jq_compat` is exercised directly here
    /// rather than via a CLI-level `keys`/`.[]` probe purely to keep this a
    /// focused unit test of the formatting logic on its own - `keys`/`.[]`
    /// do reach this same text today (`OwnedValue::to_json_for_reindex`
    /// reuses a document-sourced overflow literal's own text rather than
    /// substituting its `1e999` round-trip sentinel, fixed by #939; see
    /// `tests/jq_cli_tests.rs`'s CLI-level coverage of that bridge).
    #[test]
    fn test_stream_json_jq_number_literal_overflow_preserves_literal_text() {
        let mut buf = String::new();
        stream_owned_value_json_jq(
            &OwnedValue::NumberLiteral(NumberRepr::Float(f64::INFINITY), "1e400".into()),
            &mut buf,
        )
        .unwrap();
        assert_eq!(buf, "1E+400");

        buf.clear();
        stream_owned_value_json_jq(
            &OwnedValue::NumberLiteral(NumberRepr::Float(f64::NAN), "nan".into()),
            &mut buf,
        )
        .unwrap();
        assert_eq!(buf, "null");
    }

    #[test]
    fn test_stream_yaml_number_literal_nan_and_infinite() {
        let mut buf = String::new();
        OwnedValue::NumberLiteral(NumberRepr::Float(f64::NAN), "nan".into())
            .stream_yaml(&mut buf, IndentSpec::COMPACT, false)
            .unwrap();
        assert_eq!(buf, ".nan");

        buf.clear();
        OwnedValue::NumberLiteral(NumberRepr::Float(f64::INFINITY), "1e400".into())
            .stream_yaml(&mut buf, IndentSpec::COMPACT, false)
            .unwrap();
        assert_eq!(buf, ".inf");

        buf.clear();
        OwnedValue::NumberLiteral(NumberRepr::Float(f64::NEG_INFINITY), "-1e400".into())
            .stream_yaml(&mut buf, IndentSpec::COMPACT, false)
            .unwrap();
        assert_eq!(buf, "-.inf");
    }

    /// #1064: the plain `OwnedValue::Float` arm (a *computed* non-finite,
    /// as opposed to the `NumberLiteral` test above's document-sourced
    /// one) goes through a separate match arm and was independently
    /// reimplementing the same `.nan`/`.inf`/`-.inf` decision.
    #[test]
    fn test_stream_yaml_computed_float_nan_and_infinite() {
        let mut buf = String::new();
        OwnedValue::Float(f64::NAN)
            .stream_yaml(&mut buf, IndentSpec::COMPACT, false)
            .unwrap();
        assert_eq!(buf, ".nan");

        buf.clear();
        OwnedValue::Float(f64::INFINITY)
            .stream_yaml(&mut buf, IndentSpec::COMPACT, false)
            .unwrap();
        assert_eq!(buf, ".inf");

        buf.clear();
        OwnedValue::Float(f64::NEG_INFINITY)
            .stream_yaml(&mut buf, IndentSpec::COMPACT, false)
            .unwrap();
        assert_eq!(buf, "-.inf");
    }

    #[test]
    fn test_stream_string() {
        let mut buf = String::new();
        OwnedValue::String("hello".to_string())
            .stream_json(
                &mut buf,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
            )
            .unwrap();
        assert_eq!(buf, "\"hello\"");
    }

    #[test]
    fn test_stream_string_escaping() {
        let mut buf = String::new();
        OwnedValue::String("hello\nworld".to_string())
            .stream_json(
                &mut buf,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
            )
            .unwrap();
        assert_eq!(buf, "\"hello\\nworld\"");

        buf.clear();
        OwnedValue::String("tab\there".to_string())
            .stream_json(
                &mut buf,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
            )
            .unwrap();
        assert_eq!(buf, "\"tab\\there\"");

        buf.clear();
        OwnedValue::String("quote\"here".to_string())
            .stream_json(
                &mut buf,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
            )
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
        .stream_json(
            &mut buf,
            IndentSpec::COMPACT,
            false,
            JsonConvention::Preserve,
        )
        .unwrap();
        assert_eq!(buf, "[1,2,3]");
    }

    #[test]
    fn test_stream_object() {
        let mut buf = String::new();
        let mut map = IndexMap::new();
        map.insert("name".to_string(), OwnedValue::String("Alice".to_string()));
        map.insert("age".to_string(), OwnedValue::Int(30));
        OwnedValue::Object(map)
            .stream_json(
                &mut buf,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
            )
            .unwrap();
        assert_eq!(buf, "{\"name\":\"Alice\",\"age\":30}");
    }

    #[test]
    fn test_stream_json_pretty_nested_containers() {
        // All the pretty-print (indent_spaces > 0) tests above use indent 0;
        // this covers the indented recursion path for a container nested
        // inside another container (object-in-object, array-in-object).
        let mut buf = String::new();
        let mut inner = IndexMap::new();
        inner.insert("name".to_string(), OwnedValue::String("Alice".to_string()));
        let mut outer = IndexMap::new();
        outer.insert("user".to_string(), OwnedValue::Object(inner));
        outer.insert(
            "tags".to_string(),
            OwnedValue::Array(vec![OwnedValue::Int(1), OwnedValue::Int(2)]),
        );
        OwnedValue::Object(outer)
            .stream_json(
                &mut buf,
                IndentSpec::spaces(2),
                false,
                JsonConvention::Preserve,
            )
            .unwrap();
        assert_eq!(
            buf,
            "{\n  \"user\": {\n    \"name\": \"Alice\"\n  },\n  \"tags\": [\n    1,\n    2\n  ]\n}"
        );
    }

    /// A `core::fmt::Write` that fails as soon as it sees a specific marker
    /// string, simulating a downstream write failure (e.g. a broken pipe)
    /// partway through streaming.
    struct FailOnMarker {
        marker: &'static str,
    }

    impl core::fmt::Write for FailOnMarker {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            if s.contains(self.marker) {
                Err(core::fmt::Error)
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn test_stream_json_pretty_object_propagates_write_error() {
        // A write failure while streaming an object field's value (not the
        // key or punctuation around it) must propagate out of the recursive
        // call rather than being silently swallowed.
        let mut map = IndexMap::new();
        map.insert("a".to_string(), OwnedValue::String("boom".to_string()));
        let mut out = FailOnMarker { marker: "boom" };
        let result = OwnedValue::Object(map).stream_json(
            &mut out,
            IndentSpec::spaces(2),
            false,
            JsonConvention::Preserve,
        );
        assert!(result.is_err());
    }

    /// A [`core::fmt::Write`] that fails on its `fail_after`-th call to
    /// `write_str`, counting every call it sees regardless of outcome --
    /// used to prove an early write failure actually *stops* traversal
    /// instead of just being ignored by the caller.
    struct FailAfterNCalls {
        calls: usize,
        fail_after: usize,
    }

    impl core::fmt::Write for FailAfterNCalls {
        fn write_str(&mut self, _s: &str) -> core::fmt::Result {
            self.calls += 1;
            if self.calls > self.fail_after {
                Err(core::fmt::Error)
            } else {
                Ok(())
            }
        }
    }

    /// #931: the `Object` arm used to unconditionally collect every
    /// key/value pair into a `Vec` before writing anything, even when
    /// `sort_keys` is `false` and nothing needs reordering -- so an early
    /// write failure (e.g. `PreviewSink`'s truncation budget, the actual
    /// caller `describe()`/`dump_truncated()` always uses) still paid an
    /// O(n) allocation+copy over the *whole* object first before the
    /// failure was ever observed. Fixed by only collecting into a `Vec`
    /// when `sort_keys` is `true`; the unsorted path now consumes
    /// `obj.iter()` directly via the shared `write_object_entries` helper.
    ///
    /// This doesn't directly measure the removed allocation (a unit test
    /// can't assert "no O(n) Vec was built" without instrumentation), but
    /// it does pin the refactor's correctness: an early write failure on
    /// the unsorted path still propagates via a handful of `write_str`
    /// calls, not one per entry of a 100,000-entry object -- if a future
    /// change routed the unsorted path back through an eager
    /// `entries.into_iter()` collected up front, this would still pass
    /// (the collect itself doesn't call `write_str`), so it's a guard
    /// against `write_object_entries` itself losing its early-`Err`
    /// propagation, not a full performance regression test.
    #[test]
    fn test_stream_json_unsorted_object_stops_at_first_write_failure_931() {
        let mut map = IndexMap::new();
        for i in 0..100_000 {
            map.insert(format!("k{i}"), OwnedValue::Int(i));
        }
        let mut out = FailAfterNCalls {
            calls: 0,
            fail_after: 5,
        };
        let result = OwnedValue::Object(map).stream_json(
            &mut out,
            IndentSpec::COMPACT,
            false,
            JsonConvention::Preserve,
        );
        assert!(result.is_err());
        assert!(out.calls <= 20, "calls: {}", out.calls);
    }

    /// #963 review companion to the test above: the `sort_keys=true`
    /// branch (`ObjectEntries::Sorted`) must propagate an early write
    /// failure through `write_object_entries`'s shared loop just as
    /// reliably as the unsorted branch does -- the sorted path's `Vec`
    /// collect happens unconditionally either way (sorting genuinely needs
    /// every key), so this isn't guarding the same O(n)-avoidance property
    /// the test above pins, only that the write loop itself still stops at
    /// the first failing `write_str` once writing starts, regardless of
    /// which `ObjectEntries` variant is driving it.
    #[test]
    fn test_stream_json_sorted_object_stops_at_first_write_failure_963() {
        let mut map = IndexMap::new();
        for i in 0..1_000 {
            map.insert(format!("k{i}"), OwnedValue::Int(i));
        }
        let mut out = FailAfterNCalls {
            calls: 0,
            fail_after: 5,
        };
        let result = OwnedValue::Object(map).stream_json(
            &mut out,
            IndentSpec::COMPACT,
            true,
            JsonConvention::Preserve,
        );
        assert!(result.is_err());
        assert!(out.calls <= 20, "calls: {}", out.calls);
    }

    // The `sort_keys` (`-S`) parameter was threaded through
    // `stream_owned_value_json_with`/`stream_owned_value_yaml` in #746, but
    // nothing exercised its actual sort branch: the M2 CLI fast path only
    // ever reaches these `OwnedValue` streamers via `GenericResult::Owned`/
    // `ManyOwned`/`One`/`Many`, and no existing test carried a
    // multi-key object through one of those with `sort_keys: true`.
    #[test]
    fn test_stream_json_object_sorts_keys() {
        let mut map = IndexMap::new();
        map.insert("b".to_string(), OwnedValue::Int(1));
        map.insert("a".to_string(), OwnedValue::Int(2));
        let mut buf = String::new();
        OwnedValue::Object(map)
            .stream_json(
                &mut buf,
                IndentSpec::COMPACT,
                true,
                JsonConvention::Preserve,
            )
            .unwrap();
        assert_eq!(buf, "{\"a\":2,\"b\":1}");
    }

    #[test]
    fn test_stream_yaml_empty_object() {
        let mut buf = String::new();
        OwnedValue::Object(IndexMap::new())
            .stream_yaml(&mut buf, IndentSpec::spaces(2), false)
            .unwrap();
        assert_eq!(buf, "{}");
    }

    #[test]
    fn test_stream_yaml_array_flow_style_multiple_elements() {
        let mut buf = String::new();
        OwnedValue::Array(vec![OwnedValue::Int(1), OwnedValue::Int(2)])
            .stream_yaml(&mut buf, IndentSpec::COMPACT, false)
            .unwrap();
        assert_eq!(buf, "[1, 2]");
    }

    #[test]
    fn test_stream_yaml_array_block_style_nested_container() {
        // Covers the "nested containers render in real yq's compact form"
        // branch (#785), distinct from the plain-scalar-element branch
        // `test_stream_array`-style tests already exercise.
        let mut buf = String::new();
        OwnedValue::Array(vec![
            OwnedValue::Array(vec![OwnedValue::Int(1)]),
            OwnedValue::Int(2),
        ])
        .stream_yaml(&mut buf, IndentSpec::spaces(2), false)
        .unwrap();
        assert_eq!(buf, "- - 1\n- 2");
    }

    #[test]
    fn test_stream_yaml_object_flow_style_sorts_keys() {
        let mut map = IndexMap::new();
        map.insert("b".to_string(), OwnedValue::Int(1));
        map.insert("a".to_string(), OwnedValue::Int(2));
        let mut buf = String::new();
        OwnedValue::Object(map.clone())
            .stream_yaml(&mut buf, IndentSpec::COMPACT, true)
            .unwrap();
        assert_eq!(buf, "{a: 2, b: 1}");

        // `sort_keys: false` skips the `sort_by` call above but shares the
        // rest of this function -- covers that branch too.
        buf.clear();
        OwnedValue::Object(map)
            .stream_yaml(&mut buf, IndentSpec::COMPACT, false)
            .unwrap();
        assert_eq!(buf, "{b: 1, a: 2}");
    }

    #[test]
    fn test_stream_yaml_object_block_style_sorts_keys_with_nested_and_scalar_values() {
        // Sorts "b" before "a" out of insertion order, and covers both the
        // nested-container-gets-its-own-line branch (key "a") and the
        // plain-scalar-value branch (key "b") in the same pass.
        let mut map = IndexMap::new();
        map.insert("b".to_string(), OwnedValue::Int(1));
        let mut nested = IndexMap::new();
        nested.insert("x".to_string(), OwnedValue::Int(9));
        map.insert("a".to_string(), OwnedValue::Object(nested));
        let mut buf = String::new();
        OwnedValue::Object(map)
            .stream_yaml(&mut buf, IndentSpec::spaces(2), true)
            .unwrap();
        assert_eq!(buf, "a:\n  x: 9\nb: 1");
    }

    #[test]
    fn test_is_falsy() {
        assert!(OwnedValue::Null.is_falsy(JsonConvention::JqCompat));
        assert!(OwnedValue::Bool(false).is_falsy(JsonConvention::JqCompat));
        assert!(!OwnedValue::Bool(true).is_falsy(JsonConvention::JqCompat));
        assert!(!OwnedValue::Int(0).is_falsy(JsonConvention::JqCompat));
        assert!(!OwnedValue::String(String::new()).is_falsy(JsonConvention::JqCompat));
    }

    /// `depth` levels of single-element array nesting: `[[[...[null]...]]]`.
    /// Mirrors `value.rs`/`eval.rs`'s own `linear_array_nest` helper (#1005).
    fn linear_array_nest(depth: usize) -> OwnedValue {
        let mut v = OwnedValue::Null;
        for _ in 0..depth {
            v = OwnedValue::Array(vec![v]);
        }
        v
    }

    /// #1021: `stream_owned_value_json_with` (backs `stream_json`/`syq -o
    /// json`, and `stream_owned_value_json_jq`'s error-message convention)
    /// had no depth guard at all before this issue.
    #[test]
    fn stream_owned_value_json_with_panics_past_nesting_depth_limit_1021() {
        use crate::jq::value::MAX_VALUE_TREE_DEPTH;

        let under = linear_array_nest(MAX_VALUE_TREE_DEPTH - 1);
        let mut buf = String::new();
        under
            .stream_json(
                &mut buf,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
            )
            .unwrap();
        assert!(!buf.is_empty());

        let over = linear_array_nest(MAX_VALUE_TREE_DEPTH);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut buf = String::new();
            over.stream_json(
                &mut buf,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
            )
        }));
        assert!(
            result.is_err(),
            "stream_owned_value_json_with should panic at MAX_VALUE_TREE_DEPTH"
        );
    }

    /// #1021: `stream_owned_value_yaml` (backs `stream_yaml`/`yq`'s default
    /// output) had no depth guard at all before this issue.
    #[test]
    fn stream_owned_value_yaml_panics_past_nesting_depth_limit_1021() {
        use crate::jq::value::MAX_VALUE_TREE_DEPTH;

        let under = linear_array_nest(MAX_VALUE_TREE_DEPTH - 1);
        let mut buf = String::new();
        under
            .stream_yaml(&mut buf, IndentSpec::COMPACT, false)
            .unwrap();
        assert!(!buf.is_empty());

        let over = linear_array_nest(MAX_VALUE_TREE_DEPTH);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut buf = String::new();
            over.stream_yaml(&mut buf, IndentSpec::COMPACT, false)
        }));
        assert!(
            result.is_err(),
            "stream_owned_value_yaml should panic at MAX_VALUE_TREE_DEPTH"
        );
    }
}
