//! Owned JSON values for jq evaluation.
//!
//! When jq expressions construct new values (arrays, objects) or perform
//! computations, we need to materialize them into owned values rather than
//! references into the original JSON bytes.

#[cfg(not(test))]
use alloc::borrow::Cow;
#[cfg(not(test))]
use alloc::boxed::Box;
#[cfg(not(test))]
use alloc::format;
#[cfg(not(test))]
use alloc::string::{String, ToString};
#[cfg(not(test))]
use alloc::vec::Vec;
#[cfg(test)]
use std::borrow::Cow;

use indexmap::IndexMap;

use super::escape::{escape_json_body, write_json_body_jq};
use super::expr::Literal;

/// The parsed value backing a [`OwnedValue::NumberLiteral`], kept separate
/// from the source text so arithmetic/comparison can read it without
/// touching the `Box<str>`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NumberRepr {
    Int(i64),
    Float(f64),
}

/// Format a raw JSON number string the way jq itself would print it.
///
/// This is jq's number formatting, not a verbatim echo of `raw`: jq always
/// renders a number through its own canonical algorithm, so `jq -c .` on
/// `1e100` prints `1E+100`, not the `1e100` the document actually contains.
/// `OwnedValue::NumberLiteral` keeps the raw text so this can be derived
/// on demand (Rust's own `f64`/`i64` `Display` can't reproduce it -- that
/// mismatch is issue #387).
///
/// - Integers: output as-is
/// - Floats with trailing zeros: preserve them (`0.10` -> `0.10`)
/// - Scientific notation: normalize mantissa (`12e2` -> `1.2E+3`), uppercase E, explicit +
/// - `e0`/`e-0`: eliminate the exponent entirely (`5.5e0` -> `5.5`)
/// - Negative exponents >= -5: convert to decimal (`1e-3` -> `0.001`)
/// - Negative exponents < -5: keep scientific (`1e-10` -> `1E-10`)
/// - Negative zero: preserved as `-0`
///
/// Shared by the CLI's `--jq-compat` output formatter
/// (`src/bin/succinctly/jq_runner.rs`) and every library-level path that
/// renders a `NumberLiteral` (`to_json`, `tostring`, `@json`, string
/// interpolation, error-message previews) -- a single definition so the two
/// cannot drift, per the #106 lesson in `CLAUDE.md`.
pub fn format_number_jq_compat(raw: &[u8]) -> String {
    let s = match core::str::from_utf8(raw) {
        Ok(s) => s,
        Err(_) => return String::from_utf8_lossy(raw).into_owned(),
    };

    // Check if it contains exponent notation
    let has_exp = s.contains('e') || s.contains('E');
    let has_dot = s.contains('.');

    if !has_exp && !has_dot {
        // Plain integer - output as-is
        return s.to_string();
    }

    if !has_exp {
        // Plain decimal without exponent - preserve as-is (keeps trailing zeros)
        return s.to_string();
    }

    // Has exponent - need to reformat according to jq rules
    // Parse the full number to get the actual value
    let value: f64 = match s.parse() {
        Ok(v) => v,
        Err(_) => return s.to_string(),
    };

    // Parse exponent to check for e0/e-0. `has_exp` above guarantees `e`/`E`
    // is present, so the position is always found.
    let exp_pos = s.find(['e', 'E']).expect("has_exp guarantees e/E present");
    let exp: i32 = s[exp_pos + 1..].parse().unwrap_or(0);

    // A literal whose magnitude overflows f64 (e.g. `1e400`) parses to
    // +/-infinity, not a parse error - `value` above is non-finite in that
    // case. The normalized-scientific-notation branch below renormalizes via
    // `log10`/`pow` on `value`, which on an infinite input produces garbage
    // (`log10(inf).floor() as i32` saturates to `i32::MAX`, and dividing by
    // `10^i32::MAX` yields `NaN`) rather than erroring - so it must be
    // special-cased here, before that branch ever runs. Every existing
    // caller already guards non-finite `NumberLiteral`s before reaching this
    // function (see #930), so this exists purely to make the function
    // correct for a caller that stops doing that.
    //
    // `exp_pos` (not the already-parsed `exp: i32` above) is passed through:
    // an overflowed literal's own exponent digit string can itself exceed
    // `i32::MAX` (`exp`'s `unwrap_or(0)` would silently zero it), so the
    // overflow path re-parses it independently, at wider precision.
    if !value.is_finite() {
        return format_overflow_literal_mantissa(s, exp_pos, value.is_sign_negative());
    }

    // For e0 or e-0, jq eliminates the exponent
    if exp == 0 {
        // Check if result is integer
        if value.fract() == 0.0 && value.abs() < 1e15 {
            return format!("{}", value as i64);
        }
        // Format as plain decimal
        return format!("{value}");
    }

    // For negative exponents >= -5, jq converts to decimal
    if (-5..0).contains(&exp) {
        // Convert to decimal: 1e-3 → 0.001
        // Use smart rounding to avoid floating point noise
        return format_decimal_jq(value);
    }

    // For other cases, use normalized scientific notation
    // jq normalizes mantissa to have one digit before decimal point
    if value == 0.0 {
        return "0".to_string();
    }

    let abs_value = value.abs();
    let log10 = libm::log10(abs_value).floor() as i32;
    let normalized_mantissa = abs_value / libm::pow(10.0, log10 as f64);
    let new_exp = log10;

    // Format mantissa with appropriate precision
    // Round to avoid floating point noise (e.g., 9.199999999999999 → 9.2)
    let mantissa_str = format_mantissa_jq(normalized_mantissa);

    let sign = if value < 0.0 { "-" } else { "" };
    assemble_scientific(sign, &mantissa_str, i64::from(new_exp))
}

/// Join a sign, an already-normalized mantissa, and an exponent into jq's
/// scientific-notation text (`{sign}{mantissa}E{+/-}{exp}`) -- the shared
/// final step of both the finite path above and the overflow path below, so
/// the two can't independently drift on the `E+`/`E-` convention.
fn assemble_scientific(sign: &str, mantissa_str: &str, exp: i64) -> String {
    let exp_sign = if exp >= 0 { "+" } else { "" };
    format!("{sign}{mantissa_str}E{exp_sign}{exp}")
}

/// Renormalize an overflowed literal's own mantissa digits into jq's
/// one-digit-before-the-point scientific form, entirely via string
/// manipulation on `s` rather than the finite path's `log10`/`pow` on the
/// (here, non-finite) parsed value -- see the call site above for why. `s`
/// is the full literal text and `exp_pos` the byte index of its `e`/`E`;
/// `negative` is `value.is_sign_negative()` from the caller, which (unlike
/// re-deriving a sign from `s` itself) is correct for `-0.0`-style edge
/// cases too and matches this module's usual `if value < 0.0` idiom.
///
/// Oracle-verified against real jq (issue #930): `1e400` -> `1E+400`,
/// `123e400` -> `1.23E+402`, `12.34e400` -> `1.234E+401`, `0.5e400` ->
/// `5E+399`, `100e400` -> `1.00E+402` (trailing zeros preserved, matching
/// this module's general trailing-zero rule), and beyond jq's own
/// literal-preservation ceiling, DBL_MAX text (`1e1000000000` ->
/// `1.7976931348623157e+308`, matching a computed infinity).
fn format_overflow_literal_mantissa(s: &str, exp_pos: usize, negative: bool) -> String {
    // `dump_truncated`'s whole design keeps preview cost independent of the
    // value's own size (see its doc comment) - a document-controlled
    // mantissa of unbounded length must not turn into unbounded work here
    // just because only a handful of its leading digits will ever survive
    // that later truncation anyway. Bounding the *copy* is enough: `shift`
    // below only ever needs `int_part.len()` (an O(1) property read, not a
    // scan), so truncating what gets copied into `rest` doesn't touch the
    // exponent math's correctness, only how many digits past the first one
    // actually get rendered.
    const MAX_RENDERED_MANTISSA_DIGITS: usize = 32;

    let sign = if negative { "-" } else { "" };
    let mantissa = s[..exp_pos].strip_prefix('-').unwrap_or(&s[..exp_pos]);
    let (int_part, frac_part) = mantissa.split_once('.').unwrap_or((mantissa, ""));

    // A leading-dot literal (`.5e400`) has an empty `int_part`, not `"0"` --
    // treat both the same way (mantissa magnitude < 1).
    let (shift, leading, rest): (i64, &str, String) = if int_part.is_empty() || int_part == "0" {
        // Shift right to the first nonzero fractional digit.
        match frac_part.find(|c: char| c != '0') {
            Some(k) => {
                let after = &frac_part[k + 1..];
                (
                    -(k as i64 + 1),
                    &frac_part[k..=k],
                    after[..after.len().min(MAX_RENDERED_MANTISSA_DIGITS)].to_string(),
                )
            }
            // An all-zero mantissa (`0.0e400`) parses to a finite `0.0` and
            // never reaches this function.
            None => unreachable!("overflow literal mantissa is provably nonzero"),
        }
    } else {
        // Mantissa >= 1: shift left past every extra integer-part digit.
        // `int_part[1..]` is a slice (no copy); only what actually gets
        // concatenated into `rest` is capped.
        let after_leading = &int_part[1..];
        let rest = if after_leading.len() >= MAX_RENDERED_MANTISSA_DIGITS {
            after_leading[..MAX_RENDERED_MANTISSA_DIGITS].to_string()
        } else {
            let budget = MAX_RENDERED_MANTISSA_DIGITS - after_leading.len();
            format!(
                "{after_leading}{}",
                &frac_part[..frac_part.len().min(budget)]
            )
        };
        (int_part.len() as i64 - 1, &int_part[..1], rest)
    };

    // The exponent digit string can itself be longer than `i32`/`i64` can
    // hold (e.g. `1e99999999999999999999`) -- any such value is already
    // certain to be past jq's literal-preservation ceiling below, so an
    // out-of-range parse can saturate to `i64::MAX`/`MIN` rather than being
    // treated as an error.
    let exp_text = &s[exp_pos + 1..];
    let parsed_exp: i64 = exp_text.parse().unwrap_or_else(|_| {
        if exp_text.trim_start_matches('+').starts_with('-') {
            i64::MIN
        } else {
            i64::MAX
        }
    });
    let new_exp = parsed_exp.saturating_add(shift);

    // jq's own literal-preserving text (as opposed to a computed value's
    // DBL_MAX text) only goes up to this exponent magnitude (decNumber's
    // limit) -- oracle-verified: `1e999999999` keeps `1E+999999999`,
    // `1e1000000000` switches to DBL_MAX text instead.
    if new_exp.unsigned_abs() >= 1_000_000_000 {
        return infinite_float_preview_text(negative).to_string();
    }

    let mantissa_str = if rest.is_empty() {
        leading.to_string()
    } else {
        format!("{leading}.{rest}")
    };
    assemble_scientific(sign, &mantissa_str, new_exp)
}

/// Format a mantissa value for jq-compatible output.
/// Handles floating point precision issues by rounding appropriately.
fn format_mantissa_jq(value: f64) -> String {
    // Check if it's essentially an integer
    if (value.round() - value).abs() < 1e-10 {
        return format!("{}", value.round() as i64);
    }

    // Try different precisions and pick the shortest that rounds back correctly
    for precision in 1..=15 {
        let formatted = format!("{value:.precision$}");
        if let Ok(parsed) = formatted.parse::<f64>() {
            if (parsed - value).abs() < 1e-14 {
                // Trim trailing zeros
                let trimmed = formatted.trim_end_matches('0');
                if trimmed.ends_with('.') {
                    return format!("{trimmed}0");
                }
                return trimmed.to_string();
            }
        }
    }

    // Fallback: full precision
    let formatted = format!("{value:.15}");
    let trimmed = formatted.trim_end_matches('0');
    if trimmed.ends_with('.') {
        format!("{trimmed}0")
    } else {
        trimmed.to_string()
    }
}

/// Format a decimal value for jq-compatible output.
/// Uses smart rounding to avoid floating point noise.
fn format_decimal_jq(value: f64) -> String {
    let sign = if value < 0.0 { "-" } else { "" };
    let abs_value = value.abs();

    // Check if it's essentially an integer
    if (abs_value.round() - abs_value).abs() < 1e-10 {
        return format!("{}", value.round() as i64);
    }

    // Try different precisions and pick the shortest that rounds back correctly
    for precision in 1..=15 {
        let formatted = format!("{abs_value:.precision$}");
        if let Ok(parsed) = formatted.parse::<f64>() {
            if (parsed - abs_value).abs() < 1e-14 {
                // Trim trailing zeros
                let trimmed = formatted.trim_end_matches('0');
                if trimmed.ends_with('.') {
                    return format!("{sign}{trimmed}0");
                }
                return format!("{sign}{trimmed}");
            }
        }
    }

    // Fallback: full precision
    let formatted = format!("{abs_value:.15}");
    let trimmed = formatted.trim_end_matches('0');
    if trimmed.ends_with('.') {
        format!("{sign}{trimmed}0")
    } else {
        format!("{sign}{trimmed}")
    }
}

/// An owned JSON value.
///
/// This is used for values that are constructed during evaluation
/// (array/object construction, arithmetic results, etc.) rather than
/// references into the original JSON document.
/// Equality is *jq value equality*, not structural equality -- see the
/// hand-written [`PartialEq`] impl below. In particular `Int(1) == Float(1.0)`.
#[derive(Debug, Clone)]
pub enum OwnedValue {
    /// JSON null
    Null,
    /// JSON boolean
    Bool(bool),
    /// JSON integer (stored as i64 for precision)
    Int(i64),
    /// JSON floating-point number
    Float(f64),
    /// A number materialized straight from a document token, carrying jq's
    /// exact source spelling (e.g. `1e100`, `1.0`, `-0.0`) alongside its
    /// parsed value.
    ///
    /// Produced only by `to_owned`-style conversions out of a document
    /// cursor; every other constructor keeps using [`Int`](Self::Int)/
    /// [`Float`](Self::Float) directly. Arithmetic, comparison, and math
    /// builtins treat this exactly like `Int`/`Float` for computation --
    /// only formatting (`to_json`, `tostring`, `@json`, string
    /// interpolation, error-message previews) prefers the literal. The
    /// moment a value passes through an operation that produces a *new*
    /// number, the result collapses to plain `Int`/`Float` (see
    /// `into_plain_number`) -- matching jq, where only values that reach
    /// output untouched keep their original spelling.
    NumberLiteral(NumberRepr, Box<str>),
    /// JSON string
    String(String),
    /// JSON array
    Array(Vec<Self>),
    /// JSON object (IndexMap preserves insertion order like jq)
    Object(IndexMap<String, Self>),
}

impl OwnedValue {
    /// Create a null value.
    pub fn null() -> Self {
        Self::Null
    }

    /// Create a boolean value.
    pub fn bool(b: bool) -> Self {
        Self::Bool(b)
    }

    /// Create an integer value.
    pub fn int(n: i64) -> Self {
        Self::Int(n)
    }

    /// Create a float value.
    pub fn float(f: f64) -> Self {
        Self::Float(f)
    }

    /// Create a number that carries its document source text.
    ///
    /// Parses `literal` the same way every document-materializing conversion
    /// decides between the two representations: try `i64` first, fall back
    /// to `f64`. Falls back to [`Null`](Self::Null) if `literal` parses as
    /// neither (should not happen for a valid document number token, but
    /// matches how every other decode-failure path in this codebase
    /// represents "not actually a number" — #966).
    pub(crate) fn from_number_literal(literal: &str) -> Self {
        Self::from_number_literal_boxed(literal.into())
    }

    /// Like [`from_number_literal`](Self::from_number_literal), but takes an
    /// already-owned `Box<str>` (e.g. from `JqValue::NumberLiteral`) instead
    /// of allocating a fresh one.
    pub(crate) fn from_number_literal_boxed(literal: Box<str>) -> Self {
        let repr = if let Ok(i) = literal.parse::<i64>() {
            NumberRepr::Int(i)
        } else if let Ok(f) = literal.parse::<f64>() {
            NumberRepr::Float(f)
        } else {
            return Self::Null;
        };
        Self::NumberLiteral(repr, literal)
    }

    /// Collapse a [`NumberLiteral`](Self::NumberLiteral) into a plain
    /// `Int`/`Float`, dropping the source text. A no-op for every other
    /// variant.
    ///
    /// Every operation that *computes* with a number rather than passing it
    /// through untouched should normalize its operands through this first --
    /// arithmetic calls it at the top of each operator function so a
    /// literal-carrying operand degrades before the value/value match runs.
    pub(crate) fn into_plain_number(self) -> Self {
        match self {
            Self::NumberLiteral(NumberRepr::Int(n), _) => Self::Int(n),
            Self::NumberLiteral(NumberRepr::Float(f), _) => Self::Float(f),
            other => other,
        }
    }

    /// The exact text this number should render as: the source literal when
    /// present, otherwise the parsed value's own `Display` formatting.
    /// `None` for non-numbers.
    ///
    /// Deliberately does not special-case NaN/Infinity -- callers that need
    /// JSON's "null" substitution (`to_json`, the error-preview streamer)
    /// check that themselves before falling back to this, e.g. via
    /// `self.as_f64().is_some_and(f64::is_nan)`.
    pub fn number_str(&self) -> Option<Cow<'_, str>> {
        match self {
            Self::Int(n) => Some(Cow::Owned(n.to_string())),
            Self::Float(f) => Some(Cow::Owned(f.to_string())),
            Self::NumberLiteral(_, literal) => {
                Some(Cow::Owned(format_number_jq_compat(literal.as_bytes())))
            }
            _ => None,
        }
    }

    /// Create a string value.
    pub fn string(s: impl Into<String>) -> Self {
        Self::String(s.into())
    }

    /// Create an empty array.
    pub fn array() -> Self {
        Self::Array(Vec::new())
    }

    /// Create an array from a vector of values.
    pub fn array_from(values: Vec<Self>) -> Self {
        Self::Array(values)
    }

    /// Create an empty object.
    pub fn object() -> Self {
        Self::Object(IndexMap::new())
    }

    /// Create an object from key-value pairs.
    pub fn object_from(pairs: impl IntoIterator<Item = (String, Self)>) -> Self {
        Self::Object(pairs.into_iter().collect())
    }

    /// Check if this value is null.
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Check if this value is "truthy" (not null and not false).
    pub fn is_truthy(&self) -> bool {
        !matches!(self, Self::Null | Self::Bool(false))
    }

    /// Get the type name of this value.
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "boolean",
            Self::Int(_) | Self::Float(_) | Self::NumberLiteral(..) => "number",
            Self::String(_) => "string",
            Self::Array(_) => "array",
            Self::Object(_) => "object",
        }
    }

    /// Convert to a boolean, if possible.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Convert to an i64, if possible.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(n) => Some(*n),
            Self::Float(f) if (*f - (*f as i64 as f64)).abs() < f64::EPSILON => Some(*f as i64),
            Self::NumberLiteral(NumberRepr::Int(n), _) => Some(*n),
            Self::NumberLiteral(NumberRepr::Float(f), _)
                if (*f - (*f as i64 as f64)).abs() < f64::EPSILON =>
            {
                Some(*f as i64)
            }
            _ => None,
        }
    }

    /// Convert to an f64, if possible.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Int(n) => Some(*n as f64),
            Self::Float(f) => Some(*f),
            Self::NumberLiteral(NumberRepr::Int(n), _) => Some(*n as f64),
            Self::NumberLiteral(NumberRepr::Float(f), _) => Some(*f),
            _ => None,
        }
    }

    /// The parsed [`NumberRepr`] behind `Int`, `Float`, or `NumberLiteral`;
    /// `None` for every other variant.
    ///
    /// Lets a caller (e.g. the evaluators' `compare_values`) dispatch on
    /// exactly the same `(Int, Int)` / `(Float, Float)` / mixed pairing that
    /// [`numeric_repr_eq`] uses for `==`, regardless of which of the three
    /// variants either operand happens to be -- so ordering can't disagree
    /// with equality about the same pair of numbers.
    pub(crate) fn number_repr(&self) -> Option<NumberRepr> {
        match self {
            Self::Int(n) => Some(NumberRepr::Int(*n)),
            Self::Float(f) => Some(NumberRepr::Float(*f)),
            Self::NumberLiteral(repr, _) => Some(*repr),
            _ => None,
        }
    }

    /// Convert to a string reference, if possible.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }

    /// Convert to an array reference, if possible.
    pub fn as_array(&self) -> Option<&Vec<Self>> {
        match self {
            Self::Array(arr) => Some(arr),
            _ => None,
        }
    }

    /// Convert to a mutable array reference, if possible.
    pub fn as_array_mut(&mut self) -> Option<&mut Vec<Self>> {
        match self {
            Self::Array(arr) => Some(arr),
            _ => None,
        }
    }

    /// Convert to an object reference, if possible.
    pub fn as_object(&self) -> Option<&IndexMap<String, Self>> {
        match self {
            Self::Object(obj) => Some(obj),
            _ => None,
        }
    }

    /// Convert to a mutable object reference, if possible.
    pub fn as_object_mut(&mut self) -> Option<&mut IndexMap<String, Self>> {
        match self {
            Self::Object(obj) => Some(obj),
            _ => None,
        }
    }

    /// Get the length of this value.
    /// - null: 0
    /// - string: UTF-8 codepoint count
    /// - array: element count
    /// - object: key count
    /// - other: error (returns None)
    pub fn length(&self) -> Option<usize> {
        match self {
            Self::Null => Some(0),
            Self::String(s) => Some(s.chars().count()),
            Self::Array(arr) => Some(arr.len()),
            Self::Object(obj) => Some(obj.len()),
            _ => None,
        }
    }

    /// Format this value as JSON string.
    pub fn to_json(&self) -> String {
        match self {
            Self::Null => "null".into(),
            Self::Bool(true) => "true".into(),
            Self::Bool(false) => "false".into(),
            Self::Int(n) => format!("{n}"),
            Self::Float(f) => {
                if f.is_nan() || f.is_infinite() {
                    "null".into() // JSON doesn't support NaN or Infinity
                } else {
                    format!("{f}")
                }
            }
            Self::NumberLiteral(NumberRepr::Float(f), _) if f.is_nan() || f.is_infinite() => {
                "null".into() // JSON doesn't support NaN or Infinity
            }
            Self::NumberLiteral(_, literal) => format_number_jq_compat(literal.as_bytes()),
            Self::String(s) => format!("\"{}\"", escape_json_body(write_json_body_jq, s)),
            Self::Array(arr) => {
                let elements: Vec<String> = arr.iter().map(Self::to_json).collect();
                format!("[{}]", elements.join(","))
            }
            Self::Object(obj) => {
                let entries: Vec<String> = obj
                    .iter()
                    .map(|(k, v)| {
                        format!(
                            "\"{}\":{}",
                            escape_json_body(write_json_body_jq, k),
                            v.to_json()
                        )
                    })
                    .collect();
                format!("{{{}}}", entries.join(","))
            }
        }
    }

    /// Serialize this value as JSON for `eval_generic`'s cursor-reindexing
    /// bridge, preserving ±Infinity via a self-overflowing literal
    /// (`1e999`/`-1e999`) and NaN via [`NAN_SENTINEL`] instead of
    /// [`to_json`](Self::to_json)'s `"null"` substitution.
    ///
    /// `to_json()`'s "null" is correct for actual JSON *output* (RFC 8259
    /// forbids Infinity/NaN), but wrong for this purely-internal round-trip:
    /// it silently destroys the NaN/Infinity information
    /// `numeric_display_string()` (in `src/jq/eval.rs`) needs downstream once
    /// the bridge re-parses this text and hands the cursor to the full
    /// evaluator (#561, #472).
    pub(crate) fn to_json_for_reindex(&self) -> String {
        match self {
            Self::Float(f) if f.is_nan() => NAN_SENTINEL.to_string(),
            Self::NumberLiteral(NumberRepr::Float(f), _) if f.is_nan() => NAN_SENTINEL.to_string(),
            Self::Float(f) if f.is_infinite() => overflow_literal(*f).to_string(),
            // A document-sourced overflow literal (`123e400`) is, by
            // construction, already valid JSON number syntax that reparses
            // to this exact `f64` - reusing it here (instead of the generic
            // sentinel) lets `describe()`'s preview show the real source
            // text (#930) instead of a disconnected "1e999"/"-1e999"
            // placeholder (#939). This arm only ever sees a JSON-sourced
            // literal: JSON's `number_literal()` override (`json/light.rs`)
            // is unconditional, so its overflow literals reach here
            // directly, but YAML's own override (`yaml/light.rs`, #918)
            // deliberately excludes non-finite values (`.inf`/`-.inf`/
            // `.nan` never pass `is_preservable_float_literal`'s
            // JSON-syntax check, since none of those spellings start with a
            // digit or `-digit`) - a YAML `.inf`/`.nan` still becomes a
            // plain `Float`, handled by the arm above, never this one - so
            // no further shape-checking is needed for correctness.
            //
            // The length cap *is* needed regardless of shape: the literal
            // is otherwise-unbounded document text, and this function's
            // callers (`reduce`/`foreach`/etc.'s per-iteration reindex
            // bridge) can run it over the same unchanged value thousands of
            // times - a bound this loose still comfortably covers any
            // realistic overflow literal (reaching `f64::MAX` needs on the
            // order of ~300 exponent digits at most) while keeping the
            // *reused* case itself O(1)-ish rather than O(iterations x
            // literal length) for a pathological one.
            Self::NumberLiteral(NumberRepr::Float(f), literal) if f.is_infinite() => {
                const MAX_REUSED_LITERAL_LEN: usize = 256;
                if literal.len() <= MAX_REUSED_LITERAL_LEN {
                    literal.to_string()
                } else {
                    overflow_literal(*f).to_string()
                }
            }
            Self::Array(arr) => {
                let elements: Vec<String> = arr.iter().map(Self::to_json_for_reindex).collect();
                format!("[{}]", elements.join(","))
            }
            Self::Object(obj) => {
                let entries: Vec<String> = obj
                    .iter()
                    .map(|(k, v)| {
                        format!(
                            "\"{}\":{}",
                            escape_json_body(write_json_body_jq, k),
                            v.to_json_for_reindex()
                        )
                    })
                    .collect();
                format!("{{{}}}", entries.join(","))
            }
            other => other.to_json(),
        }
    }
}

/// A JSON number literal guaranteed to overflow to the correctly-signed
/// infinity when parsed as `f64`, used only by
/// [`OwnedValue::to_json_for_reindex`] to smuggle ±Infinity through a
/// JSON-text round-trip.
fn overflow_literal(f: f64) -> &'static str {
    if f.is_sign_negative() {
        "-1e999"
    } else {
        "1e999"
    }
}

/// The exact text real jq's error-message value previews use for an
/// infinite `f64` reached via computation (the `infinite`/`-infinite`
/// builtins, an arithmetic overflow, or any other value with no source
/// literal text of its own to echo) -- `DBL_MAX`'s shortest round-trip
/// decimal representation, per jq's `jv` (issue #930). Unlike
/// [`overflow_literal`] above, this is for display, not round-tripping, so
/// it uses jq's real number text rather than a smuggling sentinel.
///
/// Hardcoded rather than derived: `DBL_MAX` is a fixed constant, so its
/// shortest round-trip text never changes, and computing it via the
/// `log10`/`pow`-based path the finite branch of
/// [`format_number_jq_compat`] uses would hit the exact `NaN`-mantissa
/// failure that path only avoids by never being called on a non-finite
/// input in the first place.
pub(crate) fn infinite_float_preview_text(negative: bool) -> &'static str {
    if negative {
        "-1.7976931348623157e+308"
    } else {
        "1.7976931348623157e+308"
    }
}

/// The reserved JSON number literal [`OwnedValue::to_json_for_reindex`]
/// writes in place of NaN (see [`overflow_literal`] for the analogous,
/// naturally-parsing ±Infinity trick, #561). NaN has no decimal literal that
/// IEEE-754 overflow parses to, so this reserves an explicit,
/// guaranteed-unparseable-as-a-real-number token instead (#472): digit-led,
/// so `JsonCursor::value()`'s dispatcher (`src/json/light.rs`, which only
/// recognizes `-`/an ASCII digit as the start of a `Number`) routes it to
/// `StandardJson::Number` rather than `Error`; built entirely from
/// `[0-9-+.eE]`, so `JsonNumber::find_end()`'s greedy span scan captures it
/// whole; and carrying two exponent markers, so `str::parse::<f64>()`/
/// `::<i64>()` both reject it outright rather than silently overflowing to
/// something else -- see `test_nan_sentinel_is_unparseable_as_a_real_number`,
/// the load-bearing proof this design depends on.
pub(crate) const NAN_SENTINEL: &str = "9e999e999";

/// True if `bytes` is exactly [`NAN_SENTINEL`] -- the one definition every
/// jq-layer call site that reads a `to_json_for_reindex`-bridge number token
/// must check before falling back to ordinary parsing, so the comparison
/// can't diverge between call sites the way three copies of one predicate
/// did in #106.
pub(crate) fn is_nan_sentinel(bytes: &[u8]) -> bool {
    bytes == NAN_SENTINEL.as_bytes()
}

/// jq value equality.
///
/// This is deliberately **not** `#[derive]`d. Deriving would compare the
/// representation, so `Int(1)` and `Float(1.0)` -- two spellings of the same
/// JSON number -- would be unequal, and `1 == 1.0` would evaluate to `false`
/// (jq says `true`). Since `Vec`/`IndexMap` inherit their `PartialEq` from the
/// element type, every builtin routed through equality (`==`, `!=`, array `-`,
/// `contains`, `inside`, `index`, `indices`, `rindex`) picks up this impl.
///
/// Semantics, pinned against jq-1.7.1:
///
/// - Numbers compare by value across representations: `1 == 1.0`,
///   `[1] == [1.0]`, `{"a":1} == {"a":1.0}`.
/// - `Int`/`Int` compares exactly as `i64`, so integers beyond 2^53 keep their
///   precision: `9007199254740993 == 9007199254740992` is `false`, as in jq 1.7.
/// - NaN is never equal to itself. This makes the relation partial (hence
///   `PartialEq` and no `Eq`), and matches jq: `nan == nan` is `false`. It is
///   also why this cannot be written as `compare_values(..) == Equal` -- since
///   #421, `compare_values` orders NaN as strictly `Less` than every number,
///   including another NaN, so `<` can match jq; reusing that for `==` would
///   make `nan == nan` true exactly when two NaNs compare `Less`, which is
///   what `<` needs, not what `==` needs.
/// - `-0.0 == 0` and `-0.0 == 0.0`, as in jq.
/// - Objects compare order-insensitively (`IndexMap`'s own `PartialEq`), as in jq.
///
/// Known divergence: above 2^53 a mixed `Int`/`Float` comparison widens the
/// integer to `f64`, whereas jq 1.7 retains the decimal literal. So
/// `9007199254740993 == 9007199254740992.0` is `true` here and `false` in jq.
/// Every value representable exactly as an `f64` agrees.
///
/// `NumberLiteral` compares purely on its parsed [`NumberRepr`], never on the
/// source text -- two spellings of the same number (`1.0` and `1e0`) are
/// equal, matching every other numeric comparison here.
pub(crate) fn numeric_repr_eq(a: NumberRepr, b: NumberRepr) -> bool {
    match (a, b) {
        (NumberRepr::Int(a), NumberRepr::Int(b)) => a == b,
        (NumberRepr::Float(a), NumberRepr::Float(b)) => a == b,
        (NumberRepr::Int(a), NumberRepr::Float(b)) => (a as f64) == b,
        (NumberRepr::Float(a), NumberRepr::Int(b)) => a == (b as f64),
    }
}

/// Order two `f64`s the way jq's own comparator does: NaN sorts strictly
/// below every other float, including another NaN (#421).
///
/// `f64::partial_cmp` returns `None` whenever either operand is NaN, which is
/// why every caller here used to fall back to `Ordering::Equal` -- silently
/// treating NaN as equal to any number instead of ordered against it. Real jq
/// (verified against jq-1.7.1) instead treats NaN as smaller than anything,
/// even itself:
///
/// ```text
/// nan <  1    -> true      nan >  1    -> false
/// nan <= 1    -> true      nan >= 1    -> false
/// 1   <  nan  -> false     1   >  nan  -> true
/// nan <  nan  -> true      nan <= nan  -> true
/// nan >= nan  -> false     nan == nan  -> false  (a separate, hand-written `PartialEq`)
/// ```
///
/// Deliberately **not** a strict weak ordering: `cmp_f64(NaN, NaN)` answers
/// `Less` regardless of which argument is which, so both directions between
/// the same two NaNs report `Less` -- a genuine antisymmetry violation. This
/// matches jq's own comparator exactly (its C `qsort`-based sort has the
/// identical property); see [`super::eval::compare_values`]'s doc comment for
/// how narrow the practical fallout of that is.
pub(crate) fn cmp_f64(a: f64, b: f64) -> core::cmp::Ordering {
    use core::cmp::Ordering;
    if a.is_nan() {
        Ordering::Less
    } else if b.is_nan() {
        Ordering::Greater
    } else {
        a.partial_cmp(&b).expect("neither operand is NaN")
    }
}

/// Order two parsed number representations with the exact same per-pair
/// dispatch [`numeric_repr_eq`] uses for equality -- exact `i64` comparison
/// when both are `Int` (so integers beyond 2^53 keep precision, matching the
/// evaluators' plain `(Int, Int)` ordering arm), exact `f64` comparison when
/// both are `Float`, and `f64`-widening for a mixed pair (the same
/// precision-losing widen the mixed `Int`/`Float` ordering arms use).
///
/// This exists so a `NumberLiteral` operand orders consistently with how it
/// compares equal: before this, `compare_values` tried exact `i64` first for
/// *any* `NumberLiteral` pair while `==` always widened mixed pairs to `f64`,
/// so `9007199254740993 == 9007199254740992.0` could be `true` while
/// `9007199254740993 > 9007199254740992.0` was also `true` -- sort and
/// `unique`/`group_by` disagreeing with `==` about the same two numbers.
///
/// The NaN rule (#421) is centralized in [`cmp_f64`], not repeated here.
pub(crate) fn numeric_repr_cmp(a: NumberRepr, b: NumberRepr) -> core::cmp::Ordering {
    match (a, b) {
        (NumberRepr::Int(a), NumberRepr::Int(b)) => a.cmp(&b),
        (NumberRepr::Float(a), NumberRepr::Float(b)) => cmp_f64(a, b),
        (NumberRepr::Int(a), NumberRepr::Float(b)) => cmp_f64(a as f64, b),
        (NumberRepr::Float(a), NumberRepr::Int(b)) => cmp_f64(a, b as f64),
    }
}

impl PartialEq for OwnedValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Null, Self::Null) => true,
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::Float(a), Self::Float(b)) => a == b,
            (Self::Int(a), Self::Float(b)) => (*a as f64) == *b,
            (Self::Float(a), Self::Int(b)) => *a == (*b as f64),
            (Self::NumberLiteral(a, _), Self::NumberLiteral(b, _)) => numeric_repr_eq(*a, *b),
            (Self::NumberLiteral(a, _), Self::Int(b))
            | (Self::Int(b), Self::NumberLiteral(a, _)) => numeric_repr_eq(*a, NumberRepr::Int(*b)),
            (Self::NumberLiteral(a, _), Self::Float(b))
            | (Self::Float(b), Self::NumberLiteral(a, _)) => {
                numeric_repr_eq(*a, NumberRepr::Float(*b))
            }
            (Self::String(a), Self::String(b)) => a == b,
            (Self::Array(a), Self::Array(b)) => a == b,
            (Self::Object(a), Self::Object(b)) => a == b,
            _ => false,
        }
    }
}

impl From<Literal> for OwnedValue {
    fn from(lit: Literal) -> Self {
        match lit {
            Literal::Null => Self::Null,
            Literal::Bool(b) => Self::Bool(b),
            Literal::Int(n) => Self::Int(n),
            Literal::Float(f) => Self::Float(f),
            Literal::String(s) => Self::String(s),
        }
    }
}

impl From<bool> for OwnedValue {
    fn from(b: bool) -> Self {
        Self::Bool(b)
    }
}

impl From<i64> for OwnedValue {
    fn from(n: i64) -> Self {
        Self::Int(n)
    }
}

impl From<f64> for OwnedValue {
    fn from(f: f64) -> Self {
        Self::Float(f)
    }
}

impl From<String> for OwnedValue {
    fn from(s: String) -> Self {
        Self::String(s)
    }
}

impl From<&str> for OwnedValue {
    fn from(s: &str) -> Self {
        Self::String(s.to_string())
    }
}

impl<T: Into<Self>> From<Vec<T>> for OwnedValue {
    fn from(arr: Vec<T>) -> Self {
        Self::Array(arr.into_iter().map(Into::into).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constructors() {
        assert_eq!(OwnedValue::null(), OwnedValue::Null);
        assert_eq!(OwnedValue::bool(true), OwnedValue::Bool(true));
        assert_eq!(OwnedValue::int(42), OwnedValue::Int(42));
        assert_eq!(OwnedValue::float(2.5), OwnedValue::Float(2.5));
        assert_eq!(
            OwnedValue::string("hello"),
            OwnedValue::String("hello".into())
        );
    }

    #[test]
    fn test_truthy() {
        assert!(!OwnedValue::Null.is_truthy());
        assert!(!OwnedValue::Bool(false).is_truthy());
        assert!(OwnedValue::Bool(true).is_truthy());
        assert!(OwnedValue::Int(0).is_truthy()); // 0 is truthy in jq!
        assert!(OwnedValue::String(String::new()).is_truthy()); // "" is truthy in jq!
        assert!(OwnedValue::Array(vec![]).is_truthy()); // [] is truthy in jq!
    }

    #[test]
    fn test_type_name() {
        assert_eq!(OwnedValue::Null.type_name(), "null");
        assert_eq!(OwnedValue::Bool(true).type_name(), "boolean");
        assert_eq!(OwnedValue::Int(42).type_name(), "number");
        assert_eq!(OwnedValue::Float(2.5).type_name(), "number");
        assert_eq!(OwnedValue::String(String::new()).type_name(), "string");
        assert_eq!(OwnedValue::Array(vec![]).type_name(), "array");
        assert_eq!(OwnedValue::Object(IndexMap::new()).type_name(), "object");
    }

    #[test]
    fn test_length() {
        assert_eq!(OwnedValue::Null.length(), Some(0));
        assert_eq!(OwnedValue::String("hello".into()).length(), Some(5));
        assert_eq!(OwnedValue::String("héllo".into()).length(), Some(5)); // Unicode
        assert_eq!(
            OwnedValue::Array(vec![OwnedValue::Int(1), OwnedValue::Int(2)]).length(),
            Some(2)
        );
        assert_eq!(OwnedValue::Bool(true).length(), None);
        assert_eq!(OwnedValue::Int(42).length(), None);
    }

    #[test]
    fn test_to_json() {
        assert_eq!(OwnedValue::Null.to_json(), "null");
        assert_eq!(OwnedValue::Bool(true).to_json(), "true");
        assert_eq!(OwnedValue::Bool(false).to_json(), "false");
        assert_eq!(OwnedValue::Int(42).to_json(), "42");
        assert_eq!(OwnedValue::Float(2.5).to_json(), "2.5");
        assert_eq!(OwnedValue::String("hello".into()).to_json(), "\"hello\"");
        assert_eq!(
            OwnedValue::String("hello\nworld".into()).to_json(),
            "\"hello\\nworld\""
        );
        assert_eq!(
            OwnedValue::Array(vec![OwnedValue::Int(1), OwnedValue::Int(2)]).to_json(),
            "[1,2]"
        );
    }

    /// `to_json` is what `tojson`, `@json`, `tostring` and string interpolation
    /// all render through, so its escaping has to be jq's. Pinned against
    /// jq-1.7.1 (#385):
    ///
    /// ```console
    /// $ printf '"a\\u0001b\\u007fc\\u0085d\\u0008e\\u000cf"' | jq -r tojson | od -An -c
    ///     "   a   \   u   0   0   0   1   b   \   u   0   0   7   f   c
    ///   302 205   d   \   b   e   \   f   f   "  \n
    /// ```
    #[test]
    fn test_to_json_string_escaping_matches_jq() {
        let json = |s: &str| OwnedValue::String(s.into()).to_json();

        // Backspace and form feed take jq's short forms, not the long
        // \u0008/\u000c that yq uses.
        assert_eq!(json("\u{8}\u{c}"), "\"\\b\\f\"");
        // Other C0 controls, and DEL, take the long form.
        assert_eq!(json("\u{1}\u{1f}"), "\"\\u0001\\u001f\"");
        assert_eq!(json("\u{7f}"), "\"\\u007f\"");
        // C1 controls are emitted raw: `char::is_control()` covers them but
        // JSON does not require escaping them and jq does not escape them.
        assert_eq!(json("a\u{85}b"), "\"a\u{85}b\"");
        assert_eq!(json("\u{80}\u{9f}"), "\"\u{80}\u{9f}\"");
        // Non-ASCII is untouched, above and below the BMP.
        assert_eq!(json("café 😀"), "\"café 😀\"");
        // Object keys take the same treatment as values.
        let mut obj = IndexMap::new();
        obj.insert("k\u{85}\u{8}".to_string(), OwnedValue::Int(1));
        assert_eq!(OwnedValue::Object(obj).to_json(), "{\"k\u{85}\\b\":1}");
    }

    /// Whatever `to_json` emits has to parse back to the value it came from —
    /// the escaping change must not make a control character round-trip badly.
    #[test]
    fn test_to_json_round_trips_through_a_parser() {
        for cp in (0u32..=0xFF).chain([0x2028, 0x1F600]) {
            let c = char::from_u32(cp).unwrap();
            let value = OwnedValue::String(String::from(c));
            let json = value.to_json();
            let parsed: serde_json::Value =
                serde_json::from_str(&json).unwrap_or_else(|e| panic!("U+{cp:04X}: {json:?}: {e}"));
            assert_eq!(
                parsed.as_str(),
                Some(String::from(c).as_str()),
                "U+{cp:04X}"
            );
        }
    }

    #[test]
    fn test_from_literal() {
        assert_eq!(OwnedValue::from(Literal::Null), OwnedValue::Null);
        assert_eq!(
            OwnedValue::from(Literal::Bool(true)),
            OwnedValue::Bool(true)
        );
        assert_eq!(OwnedValue::from(Literal::Int(42)), OwnedValue::Int(42));
        assert_eq!(
            OwnedValue::from(Literal::Float(2.5)),
            OwnedValue::Float(2.5)
        );
        assert_eq!(
            OwnedValue::from(Literal::String("hello".into())),
            OwnedValue::String("hello".into())
        );
    }

    #[test]
    fn test_collection_constructors() {
        assert_eq!(OwnedValue::array(), OwnedValue::Array(vec![]));
        assert_eq!(
            OwnedValue::array_from(vec![OwnedValue::Int(1), OwnedValue::Int(2)]),
            OwnedValue::Array(vec![OwnedValue::Int(1), OwnedValue::Int(2)])
        );
        assert_eq!(OwnedValue::object(), OwnedValue::Object(IndexMap::new()));
        let obj = OwnedValue::object_from([("a".to_string(), OwnedValue::Int(1))]);
        assert_eq!(obj.as_object().unwrap().get("a"), Some(&OwnedValue::Int(1)));
    }

    #[test]
    fn test_is_null() {
        assert!(OwnedValue::Null.is_null());
        assert!(!OwnedValue::Bool(false).is_null());
        assert!(!OwnedValue::Int(0).is_null());
    }

    #[test]
    fn test_as_bool() {
        assert_eq!(OwnedValue::Bool(true).as_bool(), Some(true));
        assert_eq!(OwnedValue::Bool(false).as_bool(), Some(false));
        assert_eq!(OwnedValue::Int(1).as_bool(), None);
        assert_eq!(OwnedValue::Null.as_bool(), None);
    }

    #[test]
    fn test_as_i64() {
        assert_eq!(OwnedValue::Int(42).as_i64(), Some(42));
        // A float with an integral value converts to i64...
        assert_eq!(OwnedValue::Float(3.0).as_i64(), Some(3));
        // ...but a fractional float does not.
        assert_eq!(OwnedValue::Float(3.5).as_i64(), None);
        assert_eq!(OwnedValue::String("3".into()).as_i64(), None);
    }

    #[test]
    fn test_as_f64() {
        assert_eq!(OwnedValue::Int(42).as_f64(), Some(42.0));
        assert_eq!(OwnedValue::Float(2.5).as_f64(), Some(2.5));
        assert_eq!(OwnedValue::Bool(true).as_f64(), None);
    }

    #[test]
    fn test_as_str() {
        assert_eq!(OwnedValue::String("hi".into()).as_str(), Some("hi"));
        assert_eq!(OwnedValue::Int(1).as_str(), None);
    }

    #[test]
    fn test_as_array_and_mut() {
        let mut v = OwnedValue::Array(vec![OwnedValue::Int(1)]);
        assert_eq!(v.as_array(), Some(&vec![OwnedValue::Int(1)]));
        v.as_array_mut().unwrap().push(OwnedValue::Int(2));
        assert_eq!(
            v,
            OwnedValue::Array(vec![OwnedValue::Int(1), OwnedValue::Int(2)])
        );
        assert_eq!(OwnedValue::Null.as_array(), None);
        assert_eq!(OwnedValue::Null.as_array_mut(), None);
    }

    #[test]
    fn test_as_object_and_mut() {
        let mut map = IndexMap::new();
        map.insert("a".to_string(), OwnedValue::Int(1));
        let mut v = OwnedValue::Object(map);
        assert_eq!(v.as_object().unwrap().len(), 1);
        v.as_object_mut()
            .unwrap()
            .insert("b".to_string(), OwnedValue::Int(2));
        assert_eq!(v.as_object().unwrap().len(), 2);
        assert_eq!(OwnedValue::Int(1).as_object(), None);
        assert_eq!(OwnedValue::Int(1).as_object_mut(), None);
    }

    #[test]
    fn test_from_primitives() {
        assert_eq!(OwnedValue::from(true), OwnedValue::Bool(true));
        assert_eq!(OwnedValue::from(7i64), OwnedValue::Int(7));
        assert_eq!(OwnedValue::from(1.5f64), OwnedValue::Float(1.5));
        assert_eq!(
            OwnedValue::from(String::from("s")),
            OwnedValue::String("s".into())
        );
        assert_eq!(OwnedValue::from("s"), OwnedValue::String("s".into()));
    }

    #[test]
    fn test_eq_is_numeric_across_int_and_float() {
        // The whole point of the hand-written `PartialEq`: `1 == 1.0` (#156).
        assert_eq!(OwnedValue::Int(1), OwnedValue::Float(1.0));
        assert_eq!(OwnedValue::Float(1.0), OwnedValue::Int(1));
        assert_eq!(OwnedValue::Int(-7), OwnedValue::Float(-7.0));
        assert_ne!(OwnedValue::Int(1), OwnedValue::Float(1.5));
        assert_ne!(OwnedValue::Float(1.5), OwnedValue::Int(1));
        // Same-representation comparisons are unchanged.
        assert_eq!(OwnedValue::Int(2), OwnedValue::Int(2));
        assert_ne!(OwnedValue::Int(2), OwnedValue::Int(3));
        assert_eq!(OwnedValue::Float(2.5), OwnedValue::Float(2.5));
    }

    #[test]
    fn test_eq_int_int_keeps_i64_precision() {
        // Widening both sides to f64 would collapse these two; jq 1.7 keeps
        // integer literal precision here and so do we.
        assert_ne!(
            OwnedValue::Int(9_007_199_254_740_993),
            OwnedValue::Int(9_007_199_254_740_992)
        );
    }

    #[test]
    fn test_eq_nan_is_never_equal() {
        // Matches jq: `nan == nan` is false. This is why equality cannot be
        // expressed as `compare_values(..) == Equal` -- `compare_values` orders
        // NaN as strictly `Less` than every number, including another NaN
        // (#421), not `Equal`.
        let nan = OwnedValue::Float(f64::NAN);
        assert_ne!(nan, nan.clone());
        assert_ne!(nan, OwnedValue::Int(0));
        // Infinities, by contrast, do compare equal to themselves.
        assert_eq!(
            OwnedValue::Float(f64::INFINITY),
            OwnedValue::Float(f64::INFINITY)
        );
        assert_ne!(
            OwnedValue::Float(f64::INFINITY),
            OwnedValue::Float(f64::NEG_INFINITY)
        );
    }

    #[test]
    fn test_eq_signed_zero() {
        // jq: `-0.0 == 0` and `-0.0 == 0.0` are both true.
        assert_eq!(OwnedValue::Float(-0.0), OwnedValue::Int(0));
        assert_eq!(OwnedValue::Float(-0.0), OwnedValue::Float(0.0));
    }

    #[test]
    fn test_eq_recurses_through_containers() {
        // `Vec`/`IndexMap` inherit element equality, so containers are numeric-aware too.
        assert_eq!(
            OwnedValue::Array(vec![OwnedValue::Int(1)]),
            OwnedValue::Array(vec![OwnedValue::Float(1.0)])
        );
        assert_ne!(
            OwnedValue::Array(vec![OwnedValue::Int(1)]),
            OwnedValue::Array(vec![OwnedValue::Float(1.0), OwnedValue::Int(2)])
        );
        assert_eq!(
            OwnedValue::object_from([("a".to_string(), OwnedValue::Int(1))]),
            OwnedValue::object_from([("a".to_string(), OwnedValue::Float(1.0))])
        );
        // Objects compare order-insensitively, as in jq.
        assert_eq!(
            OwnedValue::object_from([
                ("a".to_string(), OwnedValue::Int(1)),
                ("b".to_string(), OwnedValue::Int(2)),
            ]),
            OwnedValue::object_from([
                ("b".to_string(), OwnedValue::Float(2.0)),
                ("a".to_string(), OwnedValue::Int(1)),
            ])
        );
    }

    #[test]
    fn test_eq_across_types_is_false() {
        // Numbers are only conflated with each other -- never with other types.
        assert_ne!(OwnedValue::Int(1), OwnedValue::String("1".into()));
        assert_ne!(OwnedValue::Int(1), OwnedValue::Bool(true));
        assert_ne!(OwnedValue::Int(0), OwnedValue::Bool(false));
        assert_ne!(OwnedValue::Int(0), OwnedValue::Null);
        assert_ne!(OwnedValue::Float(0.0), OwnedValue::Null);
        assert_ne!(
            OwnedValue::Array(vec![]),
            OwnedValue::Object(IndexMap::new())
        );
    }

    #[test]
    fn test_from_vec() {
        let v: OwnedValue = vec![1i64, 2, 3].into();
        assert_eq!(
            v,
            OwnedValue::Array(vec![
                OwnedValue::Int(1),
                OwnedValue::Int(2),
                OwnedValue::Int(3)
            ])
        );
    }

    // =========================================================================
    // NumberLiteral (#387: tostring/tojson lose a number's source literal)
    // =========================================================================

    #[test]
    fn test_from_number_literal_picks_int_or_float() {
        assert_eq!(
            OwnedValue::from_number_literal("42"),
            OwnedValue::NumberLiteral(NumberRepr::Int(42), "42".into())
        );
        assert_eq!(
            OwnedValue::from_number_literal("1.0"),
            OwnedValue::NumberLiteral(NumberRepr::Float(1.0), "1.0".into())
        );
        assert_eq!(
            OwnedValue::from_number_literal("1e100"),
            OwnedValue::NumberLiteral(NumberRepr::Float(1e100), "1e100".into())
        );
    }

    #[test]
    fn test_number_literal_eq_matches_plain_int_and_float() {
        // A NumberLiteral is numerically equal to the plain variant it parses
        // to, and to the other representation of the same number -- equality
        // never looks at the source text (#387).
        assert_eq!(OwnedValue::from_number_literal("42"), OwnedValue::Int(42));
        assert_eq!(OwnedValue::Int(42), OwnedValue::from_number_literal("42"));
        assert_eq!(
            OwnedValue::from_number_literal("1.0"),
            OwnedValue::Float(1.0)
        );
        assert_eq!(OwnedValue::from_number_literal("1.0"), OwnedValue::Int(1));
        assert_eq!(
            OwnedValue::from_number_literal("1e0"),
            OwnedValue::from_number_literal("1")
        );
        assert_ne!(OwnedValue::from_number_literal("1.5"), OwnedValue::Int(1));
    }

    #[test]
    fn test_number_repr_agrees_with_partial_eq_across_variants() {
        // `number_repr` must return the same `NumberRepr` regardless of which
        // of the three numeric variants a value happens to be in, and
        // `numeric_repr_cmp` must call two values `Equal` exactly when
        // `numeric_repr_eq` calls them equal -- otherwise ordering-based
        // consumers (`sort`, `group_by`, `unique`) can disagree with `==`
        // about the very same pair (#387: this actually happened for a
        // `NumberLiteral` operand above 2^53, before `compare_values` was
        // rewritten to share this exact dispatch with equality).
        let pairs = [
            (OwnedValue::Int(42), OwnedValue::from_number_literal("42")),
            (
                OwnedValue::Float(1.5),
                OwnedValue::from_number_literal("1.5"),
            ),
            (
                OwnedValue::from_number_literal("1.0"),
                OwnedValue::from_number_literal("1e0"),
            ),
            // The precision-boundary case #387 regressed: a `NumberLiteral`
            // `Int` above 2^53 against a `Float` that collapses to the same
            // `f64` value on widening.
            (
                OwnedValue::from_number_literal("9007199254740993"),
                OwnedValue::Float(9007199254740992.0),
            ),
        ];
        for (a, b) in pairs {
            let are_eq = a == b;
            let ra = a.number_repr().expect("numeric operand");
            let rb = b.number_repr().expect("numeric operand");
            assert_eq!(
                numeric_repr_eq(ra, rb),
                are_eq,
                "numeric_repr_eq disagrees with OwnedValue::eq for {a:?} vs {b:?}"
            );
            assert_eq!(
                numeric_repr_cmp(ra, rb) == core::cmp::Ordering::Equal,
                are_eq,
                "numeric_repr_cmp disagrees with OwnedValue::eq for {a:?} vs {b:?}"
            );
        }
    }

    #[test]
    fn test_number_repr_is_none_for_non_numeric_variants() {
        assert_eq!(OwnedValue::Null.number_repr(), None);
        assert_eq!(OwnedValue::Bool(true).number_repr(), None);
        assert_eq!(OwnedValue::String("x".into()).number_repr(), None);
        assert_eq!(OwnedValue::array().number_repr(), None);
        assert_eq!(OwnedValue::object().number_repr(), None);
    }

    #[test]
    fn test_number_literal_type_name_and_conversions() {
        let lit = OwnedValue::from_number_literal("1e100");
        assert_eq!(lit.type_name(), "number");
        assert_eq!(lit.as_f64(), Some(1e100));
        assert_eq!(lit.as_i64(), None); // not integral

        let int_lit = OwnedValue::from_number_literal("42");
        assert_eq!(int_lit.as_i64(), Some(42));
        assert_eq!(int_lit.as_f64(), Some(42.0));

        // A NumberLiteral backed by an integral Float representation also
        // converts to i64, just like plain OwnedValue::Float does.
        let float_int_lit = OwnedValue::from_number_literal("2.0");
        assert_eq!(float_int_lit.as_i64(), Some(2));
    }

    #[test]
    fn test_number_literal_to_json_preserves_source_spelling() {
        // Preserved verbatim where jq's own canonical formatting agrees with
        // the source text...
        assert_eq!(OwnedValue::from_number_literal("42").to_json(), "42");
        assert_eq!(OwnedValue::from_number_literal("1.0").to_json(), "1.0");
        assert_eq!(OwnedValue::from_number_literal("-0.0").to_json(), "-0.0");
        // ...and reformatted into jq's canonical spelling where it doesn't --
        // this is the exact repro from #387, pinned against jq-1.7.1.
        assert_eq!(OwnedValue::from_number_literal("1e100").to_json(), "1E+100");
        assert_eq!(OwnedValue::from_number_literal("1e-7").to_json(), "1E-7");
    }

    #[test]
    fn test_number_literal_to_json_overflow_to_infinity_is_null() {
        // "1e400" overflows f64 to infinity during parsing even though the
        // source text is a normal-looking (if extreme) JSON number token.
        // JSON has no Infinity, so, like plain Float, this renders as null.
        let lit = OwnedValue::from_number_literal("1e400");
        assert!(matches!(
            lit,
            OwnedValue::NumberLiteral(NumberRepr::Float(f), _) if f.is_infinite()
        ));
        assert_eq!(lit.to_json(), "null");
    }

    #[test]
    fn test_nan_sentinel_is_unparseable_as_a_real_number() {
        // Load-bearing for #472: `NAN_SENTINEL` is only safe to reserve as an
        // out-of-band NaN marker because it can never arise from a
        // legitimately formatted number -- both parses must fail today.
        assert!(NAN_SENTINEL.parse::<f64>().is_err());
        assert!(NAN_SENTINEL.parse::<i64>().is_err());
    }

    #[test]
    fn test_to_json_for_reindex_preserves_nan() {
        // Unlike ±Infinity (which self-overflows via `1e999`), NaN has no
        // natural JSON-number spelling, so `to_json_for_reindex` must emit
        // the reserved sentinel instead of falling back to `to_json`'s
        // "null" substitution (#472).
        assert_eq!(
            OwnedValue::Float(f64::NAN).to_json_for_reindex(),
            NAN_SENTINEL
        );
        let lit = OwnedValue::NumberLiteral(NumberRepr::Float(f64::NAN), "nan".into());
        assert_eq!(lit.to_json_for_reindex(), NAN_SENTINEL);
    }

    /// #939: an infinite `NumberLiteral` backed by real document text (any
    /// magnitude-overflow literal, e.g. `123e400`) is already guaranteed to
    /// reparse to this exact value - reusing it avoids the disconnected
    /// `1e999`/`-1e999` sentinel leaking into a downstream preview (#930's
    /// `format_overflow_literal_mantissa` can then reformat the *real* text
    /// instead of the sentinel's).
    #[test]
    fn test_to_json_for_reindex_reuses_overflow_literal_text() {
        let lit = OwnedValue::NumberLiteral(NumberRepr::Float(f64::INFINITY), "123e400".into());
        assert_eq!(lit.to_json_for_reindex(), "123e400");

        let lit = OwnedValue::NumberLiteral(NumberRepr::Float(f64::NEG_INFINITY), "-1e400".into());
        assert_eq!(lit.to_json_for_reindex(), "-1e400");
    }

    /// #939 review: reusing the literal's own text is O(its length), and
    /// this function's callers (`reduce`/`foreach`'s per-iteration reindex
    /// bridge) can run it over the same unchanged value many times - so an
    /// unbounded literal must fall back to the O(1) sentinel rather than
    /// turning a loop into O(iterations x literal length). No realistic
    /// document overflow literal is anywhere near this long (#930's own
    /// tests only go up to a handful of digits before the exponent), so the
    /// cap only ever trips on a pathological/adversarial one.
    #[test]
    fn test_to_json_for_reindex_bounds_an_unrealistically_long_literal() {
        let huge_literal = format!("{}e400", "9".repeat(300));
        let lit = OwnedValue::NumberLiteral(NumberRepr::Float(f64::INFINITY), huge_literal.into());
        assert_eq!(lit.to_json_for_reindex(), "1e999");
    }

    #[test]
    fn test_number_literal_number_str_matches_to_json() {
        let lit = OwnedValue::from_number_literal("1e100");
        assert_eq!(lit.number_str().as_deref(), Some("1E+100"));
        assert_eq!(OwnedValue::Null.number_str(), None);

        // Plain Int/Float variants (not just NumberLiteral) also produce a
        // number_str, via Display rather than format_number_jq_compat.
        assert_eq!(OwnedValue::Int(7).number_str().as_deref(), Some("7"));
        assert_eq!(OwnedValue::Float(2.5).number_str().as_deref(), Some("2.5"));
    }

    #[test]
    fn test_from_number_literal_boxed_falls_back_to_null_for_unparseable_text() {
        // Callers gate on `is_valid_number` before reaching this (#966), but
        // it's defensive: neither an i64 nor f64 parse succeeding falls back
        // to `Null` (matching every other decode-failure path in this
        // codebase) rather than panicking or silently producing `0`.
        assert_eq!(
            OwnedValue::from_number_literal("not-a-number"),
            OwnedValue::Null
        );
    }

    #[test]
    fn test_into_plain_number_drops_the_literal() {
        // Once a value is treated as a fresh computed number (what every
        // arithmetic operator does before matching), the literal is gone and
        // formatting falls back to plain Int/Float Display -- matching jq,
        // where only untouched values keep their original spelling.
        assert_eq!(
            OwnedValue::from_number_literal("1e100").into_plain_number(),
            OwnedValue::Float(1e100)
        );
        assert_eq!(
            OwnedValue::from_number_literal("42").into_plain_number(),
            OwnedValue::Int(42)
        );
        // No-op for every other variant.
        assert_eq!(
            OwnedValue::Bool(true).into_plain_number(),
            OwnedValue::Bool(true)
        );
        assert_eq!(
            OwnedValue::String("x".into()).into_plain_number(),
            OwnedValue::String("x".into())
        );
    }

    #[test]
    fn test_format_number_jq_compat_matches_jq_1_7_1() {
        // Pinned against jq-1.7.1 -- the exact cases from #387's repro.
        assert_eq!(format_number_jq_compat(b"1e100"), "1E+100");
        assert_eq!(format_number_jq_compat(b"1.0"), "1.0");
        assert_eq!(format_number_jq_compat(b"-0.0"), "-0.0");
        assert_eq!(format_number_jq_compat(b"1e-7"), "1E-7");
        assert_eq!(format_number_jq_compat(b"42"), "42");
        // e0/e-0 drop the exponent entirely.
        assert_eq!(format_number_jq_compat(b"5.5e0"), "5.5");
        // Negative exponents within [-5, -1] expand to decimal.
        assert_eq!(format_number_jq_compat(b"1e-3"), "0.001");
    }

    #[test]
    fn test_format_number_jq_compat_invalid_utf8_falls_back_to_lossy() {
        // Not a real document number (the parser only ever hands this valid
        // UTF-8), but the function is `pub` and takes raw bytes, so it must
        // not panic on garbage.
        let raw = &[0xFF, 0xFE][..];
        assert_eq!(
            format_number_jq_compat(raw),
            String::from_utf8_lossy(raw).into_owned()
        );
    }

    #[test]
    fn test_format_number_jq_compat_unparseable_exponent_falls_back_to_raw() {
        // "1e" contains 'e' but has no exponent digits, so it fails the f64
        // parse and falls back to echoing the input unchanged.
        assert_eq!(format_number_jq_compat(b"1e"), "1e");
    }

    #[test]
    fn test_format_number_jq_compat_e0_integer_drops_exponent() {
        // e0 eliminates the exponent; when the result is a whole number it
        // takes the integer-formatting branch rather than float Display.
        assert_eq!(format_number_jq_compat(b"5e0"), "5");
    }

    #[test]
    fn test_format_number_jq_compat_zero_with_positive_exponent() {
        assert_eq!(format_number_jq_compat(b"0e10"), "0");
    }

    #[test]
    fn test_format_number_jq_compat_non_integer_mantissa() {
        // Docstring example: `12e2` -> `1.2E+3`. Exercises the mantissa's
        // shortest-round-tripping-precision loop (the integer fast path
        // covers only whole-number mantissas like `1`).
        assert_eq!(format_number_jq_compat(b"12e2"), "1.2E+3");
    }

    #[test]
    fn test_format_number_jq_compat_negative_exponent_integer_decimal() {
        // Negative exponent that still resolves to a whole number takes
        // format_decimal_jq's integer fast path rather than its precision loop.
        assert_eq!(format_number_jq_compat(b"100e-2"), "1");
        assert_eq!(format_number_jq_compat(b"-100e-2"), "-1");
    }

    /// #930: a literal whose magnitude overflows f64 (e.g. `1e400`) used to
    /// route into the finite path's `log10`/`pow` renormalization, producing
    /// garbage (`"NaNE+2147483647"`) since that path needs a finite value to
    /// renormalize. Every case here is oracle-verified against real jq
    /// 1.7.1's `describe()`-equivalent value preview (`try (. + "s") catch
    /// .` on the given literal as JSON input, which reaches the value
    /// through `keys`/`.[]`-style operations that don't lose `NumberLiteral`
    /// status the way arithmetic does).
    #[test]
    fn test_format_number_jq_compat_overflow_literal_matches_jq_1_7_1() {
        assert_eq!(format_number_jq_compat(b"1e400"), "1E+400");
        assert_eq!(format_number_jq_compat(b"-1e400"), "-1E+400");
        assert_eq!(format_number_jq_compat(b"123e400"), "1.23E+402");
        assert_eq!(format_number_jq_compat(b"12.34e400"), "1.234E+401");
        assert_eq!(format_number_jq_compat(b"0.5e400"), "5E+399");
        assert_eq!(format_number_jq_compat(b"100e400"), "1.00E+402");
        assert_eq!(format_number_jq_compat(b"9.999e400"), "9.999E+400");
        assert_eq!(format_number_jq_compat(b"1E400"), "1E+400");
    }

    /// #930 review: a leading-dot mantissa (`.5e400`, no digit before the
    /// decimal point) has an *empty* integer part, which `int_part == "0"`
    /// alone doesn't catch - the first version of this fix took the
    /// `else` (mantissa >= 1) branch instead and panicked slicing
    /// `&int_part[..1]` on an empty string. Oracle-verified: real jq treats
    /// this identically to an explicit `0.5e400`.
    #[test]
    fn test_format_number_jq_compat_overflow_leading_dot_mantissa() {
        assert_eq!(format_number_jq_compat(b".5e400"), "5E+399");
        assert_eq!(format_number_jq_compat(b"-.5e400"), "-5E+399");
    }

    /// #930 review: jq's own literal-preservation only holds up to an
    /// exponent magnitude just under 1,000,000,000 (decNumber's limit) -
    /// beyond that, real jq falls back to `DBL_MAX` text, same as a
    /// *computed* infinity (`infinite`). The first version of this fix had
    /// no such ceiling, and separately parsed the exponent digits as `i32`
    /// with a silent `unwrap_or(0)` fallback - so an exponent that
    /// overflowed even that narrower range produced a flatly wrong `"1E+0"`
    /// instead of anything resembling the real magnitude.
    #[test]
    fn test_format_number_jq_compat_overflow_exponent_ceiling_matches_jq_1_7_1() {
        // Just at the boundary: still literal-preserving text.
        assert_eq!(format_number_jq_compat(b"1e999999999"), "1E+999999999");
        assert_eq!(format_number_jq_compat(b"9e999999999"), "9E+999999999");
        // One past the boundary: DBL_MAX text instead.
        assert_eq!(
            format_number_jq_compat(b"1e1000000000"),
            "1.7976931348623157e+308"
        );
        assert_eq!(
            format_number_jq_compat(b"-1e1000000000"),
            "-1.7976931348623157e+308"
        );
        // An exponent digit string past i64's own range: still DBL_MAX
        // text, not a garbage or silently-wrong value.
        assert_eq!(
            format_number_jq_compat(b"1e99999999999999999999"),
            "1.7976931348623157e+308"
        );
    }

    /// #930 review: the exponent-parsing fallback used to reuse
    /// `format_number_jq_compat`'s own `i32`-typed `exp`, which silently
    /// zeroed via `unwrap_or(0)` for any exponent digit string past
    /// `i32::MAX` (~10 digits) - producing `"1E+0"` for a value that's
    /// actually astronomically large, well before the real ceiling above
    /// even comes into play. `format_overflow_literal_mantissa` now
    /// re-parses the exponent text itself at `i64` precision instead.
    #[test]
    fn test_format_number_jq_compat_overflow_exponent_past_i32_matches_jq_1_7_1() {
        assert_eq!(
            format_number_jq_compat(b"1e2147483648"),
            "1.7976931348623157e+308"
        );
    }

    /// #930: a no-exponent overflow literal (e.g. a 400-digit plain integer)
    /// takes the function's pre-existing "no exponent -> output as-is" early
    /// return (before `value.parse()` is ever consulted), so it was already
    /// correct - jq itself doesn't reformat these into scientific notation
    /// either, it just echoes the digits (oracle-verified).
    #[test]
    fn test_format_number_jq_compat_overflow_no_exponent_is_unaffected() {
        let digits = "9".repeat(400);
        assert_eq!(format_number_jq_compat(digits.as_bytes()), digits);
    }

    /// #930 review: an overflowed literal's mantissa can be arbitrarily long
    /// (it's document-controlled text), but only a handful of its leading
    /// digits ever survive `dump_truncated`'s later preview truncation - so
    /// rendering the *whole* mantissa here, only to throw almost all of it
    /// away one call up, would be unbounded work for no visible benefit.
    /// Pins that the leading digits (and thus the exponent, which is
    /// unaffected either way) stay correct, and that the rendered text
    /// itself is bounded rather than millions of bytes long.
    #[test]
    fn test_format_number_jq_compat_overflow_huge_mantissa_is_bounded() {
        let mantissa = "9".repeat(2_000_000);
        let literal = format!("{mantissa}.5e400");
        let result = format_number_jq_compat(literal.as_bytes());
        let expected_prefix = format!("9.{}", "9".repeat(32));
        assert!(
            result.starts_with(&expected_prefix),
            "leading digits must still be correct: {result}"
        );
        // The exponent shift (mantissa.len() - 1) is unaffected by the
        // rendering cap - only how many digits after the leading one get
        // copied into the output text.
        assert!(
            result.ends_with("E+2000399"),
            "exponent must reflect the mantissa's true (uncapped) length: {result}"
        );
        assert!(
            result.len() < 100,
            "must not render anywhere near the full 2,000,000-digit mantissa: got {} bytes",
            result.len()
        );
    }
}
