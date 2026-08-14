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

use super::document::{DocumentFields, IndentSpec};
use super::escape::{write_json_body_jq, write_json_body_yq};
use super::value::{format_number_jq_compat, NumberRepr, OwnedValue};
use crate::yaml::format_float_with_fraction;

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
    /// `sort_keys` sorts object keys before writing (`-S`/`--sort-keys`).
    fn stream_json<W: core::fmt::Write>(
        &self,
        out: &mut W,
        indent: IndentSpec,
        sort_keys: bool,
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
    /// Used for `--exit-status` flag handling without requiring full materialization.
    fn is_falsy(&self) -> bool;
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

impl StreamableValue for OwnedValue {
    fn stream_json<W: core::fmt::Write>(
        &self,
        out: &mut W,
        indent: IndentSpec,
        sort_keys: bool,
    ) -> core::fmt::Result {
        stream_owned_value_json(self, out, 0, indent.width, indent.unit, sort_keys)
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

    fn is_falsy(&self) -> bool {
        matches!(self, Self::Null | Self::Bool(false))
    }
}

/// Stream an OwnedValue as JSON, escaping strings the way `yq` does.
///
/// This is what [`StreamableValue::stream_json`] runs, and so what `syq -o json`
/// emits. For the `jq` convention — which #385 established is a genuinely
/// different one, not a stricter one — see [`stream_owned_value_json_jq`].
///
/// Whole floats keep their decimal point here (`format_float_with_fraction`):
/// this is the M2 fast path for non-identity navigation queries (`.field`,
/// `.[0]`, `.[]`) in compact mode, and it must agree with the identity
/// streaming path and the OwnedValue/DOM pretty-printer, both of which
/// preserve `1.0` rather than collapsing it to `1` (issue #169).
fn stream_owned_value_json<W: core::fmt::Write>(
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
    )
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
    stream_owned_value_json_with(value, out, 0, 0, ' ', false, write_json_body_jq, |f| {
        f.to_string()
    })
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
#[allow(clippy::too_many_arguments)]
fn stream_owned_value_json_with<W: core::fmt::Write>(
    value: &OwnedValue,
    out: &mut W,
    current_indent: usize,
    indent_spaces: usize,
    unit: char,
    sort_keys: bool,
    escape: fn(&mut W, &str) -> core::fmt::Result,
    float_fmt: fn(f64) -> String,
) -> core::fmt::Result {
    match value {
        OwnedValue::Null => out.write_str("null"),
        OwnedValue::Bool(true) => out.write_str("true"),
        OwnedValue::Bool(false) => out.write_str("false"),
        OwnedValue::Int(n) => write!(out, "{n}"),
        OwnedValue::Float(f) => {
            if f.is_nan() || f.is_infinite() {
                // JSON doesn't support NaN or Infinity
                out.write_str("null")
            } else {
                out.write_str(&float_fmt(*f))
            }
        }
        OwnedValue::NumberLiteral(repr, literal) => {
            if matches!(repr, NumberRepr::Float(f) if f.is_nan() || f.is_infinite()) {
                out.write_str("null")
            } else {
                out.write_str(&format_number_jq_compat(literal.as_bytes()))
            }
        }
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
                stream_owned_value_json_with(
                    elem,
                    out,
                    next_indent,
                    indent_spaces,
                    unit,
                    sort_keys,
                    escape,
                    float_fmt,
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
            let mut entries: Vec<(&String, &OwnedValue)> = obj.iter().collect();
            if sort_keys {
                entries.sort_by(|a, b| a.0.cmp(b.0));
            }
            for (i, (key, value)) in entries.into_iter().enumerate() {
                if i > 0 {
                    out.write_char(',')?;
                }
                if indent_spaces > 0 {
                    out.write_char('\n')?;
                    write_indent(out, next_indent, unit)?;
                }
                stream_json_string(out, key, escape)?;
                out.write_str(if indent_spaces > 0 { ": " } else { ":" })?;
                stream_owned_value_json_with(
                    value,
                    out,
                    next_indent,
                    indent_spaces,
                    unit,
                    sort_keys,
                    escape,
                    float_fmt,
                )?;
            }
            if indent_spaces > 0 {
                out.write_char('\n')?;
                write_indent(out, current_indent, unit)?;
            }
            out.write_char('}')
        }
    }
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
/// above, pulling one key at a time from `uncons()` instead of walking a
/// materialized slice. `GenericResult::LazyKeys { sorted: false, .. }` is
/// always the entire top-level result (never nested inside another
/// container), so unlike the function it mirrors this has no
/// `current_indent` parameter — it's always 0.
pub fn stream_lazy_keys_json<W: core::fmt::Write, F: DocumentFields>(
    fields: &F,
    out: &mut W,
    indent: IndentSpec,
) -> core::fmt::Result {
    if fields.is_empty() {
        return out.write_str("[]");
    }
    out.write_char('[')?;
    let mut current = fields.clone();
    let mut i = 0usize;
    while let Some((field, rest)) = current.uncons() {
        // `key_str()` is expected to always return `Some` (see its doc
        // comment on `DocumentField`); a field with no stringifiable key is
        // silently skipped, matching `DocumentFields::keys()`'s default walk.
        if let Some(key) = field.key_str() {
            if i > 0 {
                out.write_char(',')?;
            }
            if indent.width > 0 {
                out.write_char('\n')?;
                write_indent(out, indent.width, indent.unit)?;
            }
            stream_json_string(out, &key, write_json_body_yq)?;
            i += 1;
        }
        current = rest;
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
    match value {
        OwnedValue::Null => out.write_str("null"),
        OwnedValue::Bool(true) => out.write_str("true"),
        OwnedValue::Bool(false) => out.write_str("false"),
        OwnedValue::Int(n) => write!(out, "{n}"),
        OwnedValue::Float(f) => {
            if f.is_nan() {
                out.write_str(".nan")
            } else if f.is_infinite() {
                if *f > 0.0 {
                    out.write_str(".inf")
                } else {
                    out.write_str("-.inf")
                }
            } else {
                // Not `write!(out, "{f}")`: that drops the `.0` from a whole
                // float, diverging from the identity streaming path and the
                // DOM pretty-printer (issue #169).
                out.write_str(&format_float_with_fraction(*f))
            }
        }
        OwnedValue::NumberLiteral(NumberRepr::Float(f), _) if f.is_nan() => out.write_str(".nan"),
        OwnedValue::NumberLiteral(NumberRepr::Float(f), _) if f.is_infinite() => {
            out.write_str(if *f > 0.0 { ".inf" } else { "-.inf" })
        }
        OwnedValue::NumberLiteral(_, literal) => {
            out.write_str(&format_number_jq_compat(literal.as_bytes()))
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
                    stream_owned_value_yaml(elem, out, "", 0, unit, sort_keys)?;
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
                        stream_owned_value_yaml(
                            elem,
                            out,
                            &child_indent,
                            indent_spaces,
                            unit,
                            sort_keys,
                        )?;
                    } else {
                        let child_indent = deeper_indent(indent, indent_spaces, unit);
                        stream_owned_value_yaml(
                            elem,
                            out,
                            &child_indent,
                            indent_spaces,
                            unit,
                            sort_keys,
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
                    stream_owned_value_yaml(val, out, "", 0, unit, sort_keys)?;
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
                        stream_owned_value_yaml(
                            val,
                            out,
                            &child_indent,
                            indent_spaces,
                            unit,
                            sort_keys,
                        )?;
                    } else {
                        out.write_char(' ')?;
                        let child_indent = deeper_indent(indent, indent_spaces, unit);
                        stream_owned_value_yaml(
                            val,
                            out,
                            &child_indent,
                            indent_spaces,
                            unit,
                            sort_keys,
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
/// always the entire top-level result.
pub fn stream_lazy_keys_yaml<W: core::fmt::Write, F: DocumentFields>(
    fields: &F,
    out: &mut W,
    indent: IndentSpec,
) -> core::fmt::Result {
    if fields.is_empty() {
        return out.write_str("[]");
    }
    let mut current = fields.clone();
    let mut i = 0usize;
    if indent.is_compact() {
        // Flow style
        out.write_char('[')?;
        while let Some((field, rest)) = current.uncons() {
            if let Some(key) = field.key_str() {
                if i > 0 {
                    out.write_str(", ")?;
                }
                stream_yaml_string(out, &key)?;
                i += 1;
            }
            current = rest;
        }
        out.write_char(']')
    } else {
        // Block style
        while let Some((field, rest)) = current.uncons() {
            if let Some(key) = field.key_str() {
                if i > 0 {
                    out.write_char('\n')?;
                }
                out.write_str("- ")?;
                stream_yaml_string(out, &key)?;
                i += 1;
            }
            current = rest;
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
            .stream_json(&mut buf, IndentSpec::COMPACT, false)
            .unwrap();
        assert_eq!(buf, "null");
    }

    #[test]
    fn test_stream_bool() {
        let mut buf = String::new();
        OwnedValue::Bool(true)
            .stream_json(&mut buf, IndentSpec::COMPACT, false)
            .unwrap();
        assert_eq!(buf, "true");

        buf.clear();
        OwnedValue::Bool(false)
            .stream_json(&mut buf, IndentSpec::COMPACT, false)
            .unwrap();
        assert_eq!(buf, "false");
    }

    #[test]
    fn test_stream_int() {
        let mut buf = String::new();
        OwnedValue::Int(42)
            .stream_json(&mut buf, IndentSpec::COMPACT, false)
            .unwrap();
        assert_eq!(buf, "42");

        buf.clear();
        OwnedValue::Int(-123)
            .stream_json(&mut buf, IndentSpec::COMPACT, false)
            .unwrap();
        assert_eq!(buf, "-123");
    }

    #[test]
    fn test_stream_float() {
        let mut buf = String::new();
        OwnedValue::Float(3.125)
            .stream_json(&mut buf, IndentSpec::COMPACT, false)
            .unwrap();
        assert_eq!(buf, "3.125");
    }

    /// The `yq` JSON convention (what the M2 fast path for `.field`/`.[0]`/
    /// `.[]` streams through) must keep a whole float's decimal point,
    /// matching the identity path and the DOM pretty-printer (issue #169).
    #[test]
    fn test_stream_json_whole_float_keeps_decimal_point() {
        let mut buf = String::new();
        OwnedValue::Float(1.0)
            .stream_json(&mut buf, IndentSpec::COMPACT, false)
            .unwrap();
        assert_eq!(buf, "1.0");

        buf.clear();
        OwnedValue::Float(-0.0)
            .stream_json(&mut buf, IndentSpec::COMPACT, false)
            .unwrap();
        assert_eq!(buf, "-0.0");

        buf.clear();
        OwnedValue::Array(vec![OwnedValue::Float(1.0), OwnedValue::Float(2.5)])
            .stream_json(&mut buf, IndentSpec::COMPACT, false)
            .unwrap();
        assert_eq!(buf, "[1.0,2.5]");
    }

    /// The `yq` YAML convention must agree with the JSON one.
    #[test]
    fn test_stream_yaml_whole_float_keeps_decimal_point() {
        let mut buf = String::new();
        OwnedValue::Float(1.0)
            .stream_yaml(&mut buf, IndentSpec::COMPACT, false)
            .unwrap();
        assert_eq!(buf, "1.0");
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

    #[test]
    fn test_stream_json_number_literal() {
        let mut buf = String::new();
        OwnedValue::NumberLiteral(NumberRepr::Float(1.2e3), "1.2e3".into())
            .stream_json(&mut buf, IndentSpec::COMPACT, false)
            .unwrap();
        assert_eq!(buf, "1.2E+3");
    }

    #[test]
    fn test_stream_json_number_literal_nan_and_infinite() {
        // A `NumberLiteral` whose source text overflows/underflows to a
        // non-finite float (e.g. `1e400`) must render as `null` like a plain
        // non-finite Float does, not fall through to `format_number_jq_compat`.
        let mut buf = String::new();
        OwnedValue::NumberLiteral(NumberRepr::Float(f64::NAN), "nan".into())
            .stream_json(&mut buf, IndentSpec::COMPACT, false)
            .unwrap();
        assert_eq!(buf, "null");

        buf.clear();
        OwnedValue::NumberLiteral(NumberRepr::Float(f64::INFINITY), "1e400".into())
            .stream_json(&mut buf, IndentSpec::COMPACT, false)
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

    #[test]
    fn test_stream_string() {
        let mut buf = String::new();
        OwnedValue::String("hello".to_string())
            .stream_json(&mut buf, IndentSpec::COMPACT, false)
            .unwrap();
        assert_eq!(buf, "\"hello\"");
    }

    #[test]
    fn test_stream_string_escaping() {
        let mut buf = String::new();
        OwnedValue::String("hello\nworld".to_string())
            .stream_json(&mut buf, IndentSpec::COMPACT, false)
            .unwrap();
        assert_eq!(buf, "\"hello\\nworld\"");

        buf.clear();
        OwnedValue::String("tab\there".to_string())
            .stream_json(&mut buf, IndentSpec::COMPACT, false)
            .unwrap();
        assert_eq!(buf, "\"tab\\there\"");

        buf.clear();
        OwnedValue::String("quote\"here".to_string())
            .stream_json(&mut buf, IndentSpec::COMPACT, false)
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
        .stream_json(&mut buf, IndentSpec::COMPACT, false)
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
            .stream_json(&mut buf, IndentSpec::COMPACT, false)
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
            .stream_json(&mut buf, IndentSpec::spaces(2), false)
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
        let result = OwnedValue::Object(map).stream_json(&mut out, IndentSpec::spaces(2), false);
        assert!(result.is_err());
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
            .stream_json(&mut buf, IndentSpec::COMPACT, true)
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
        assert!(OwnedValue::Null.is_falsy());
        assert!(OwnedValue::Bool(false).is_falsy());
        assert!(!OwnedValue::Bool(true).is_falsy());
        assert!(!OwnedValue::Int(0).is_falsy());
        assert!(!OwnedValue::String(String::new()).is_falsy());
    }
}
