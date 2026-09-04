//! Lazy JSON values for jq evaluation.
//!
//! `JqValue` is the core value type for jq evaluation. It can represent either:
//! - A lazy reference to a value in the original JSON bytes (via `JsonCursor`)
//! - A materialized value that was computed during evaluation
//!
//! This design enables:
//! - Zero-copy navigation for pass-through queries (`.foo`, `.[]`, etc.)
//! - Preserved number formatting (e.g., `4e4` stays as `4e4` in output)
//! - Minimal memory usage - only materialize when computation requires it
//! - Mixed lazy/materialized arrays and objects

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

use indexmap::IndexMap;
#[cfg(test)]
use std::borrow::Cow;

use crate::json::light::{JsonCursor, StandardJson};

use super::document::{
    container_tail_gap_ok, effective_len, effective_len_checked, key_display_string,
    DisplayKeyGuard, DistinctKeyCursors, DocumentCursor, DocumentFields,
};
use super::error::EvalError;
use super::escape::write_json_body_jq;
use super::expr::Literal;
use super::value::{assert_value_tree_depth, infinite_float_preview_text, OwnedValue};

/// A JSON value for jq evaluation - lazy by default, materialized when needed.
///
/// For pass-through operations (field access, iteration, slicing), values stay
/// as `Cursor` references to the original JSON bytes. This preserves the exact
/// text representation (including number formatting like `4e4`) and avoids
/// allocation.
///
/// When computation is required (arithmetic, string operations, object/array
/// construction with computed values), values are materialized into the
/// appropriate owned variant.
#[derive(Debug, Clone)]
pub enum JqValue<'a, W = Vec<u64>> {
    /// Lazy reference to a value in the original JSON bytes.
    ///
    /// The cursor provides navigation methods and access to the raw bytes.
    /// Use `text_range()` to get the byte range for direct output.
    Cursor(JsonCursor<'a, W>),

    /// JSON null (materialized).
    Null,

    /// JSON boolean (materialized).
    Bool(bool),

    /// JSON integer (materialized, stored as i64 for precision).
    Int(i64),

    /// JSON floating-point number (materialized).
    Float(f64),

    /// Raw number bytes from original JSON (preserves formatting like `4e4`).
    ///
    /// This variant is used when a number is extracted from the original JSON
    /// but hasn't been parsed yet. When output, the original bytes are written
    /// directly, preserving formatting like `4e4` instead of `40000`.
    RawNumber(&'a [u8]),

    /// An owned, materialized counterpart to `RawNumber`: a number that has
    /// been through `OwnedValue` (array/object construction, `as` binding,
    /// `sort`, ...) but reached here untouched by arithmetic, so it still
    /// carries its document source text. See `OwnedValue::NumberLiteral`.
    NumberLiteral(Box<str>),

    /// JSON string (materialized).
    String(String),

    /// JSON array with potentially mixed lazy/materialized children.
    ///
    /// Created when constructing arrays with `[.a, .b + 1]` or collecting
    /// iteration results. Children can be `Cursor` (lazy) or materialized.
    Array(Vec<Self>),

    /// JSON object with potentially mixed lazy/materialized values.
    ///
    /// Created when constructing objects with `{a: .x, b: .y + 1}`.
    /// Keys are always strings, values can be lazy or materialized.
    Object(IndexMap<String, Self>),

    /// Lazy array of `keys_unsorted` results, backed by an object field
    /// iterator — not yet decoded into `String`s (#140). `write_json` and
    /// `print_json` stream each key's raw bytes straight from its cursor,
    /// so a bare `keys_unsorted` output never materializes a `Vec<String>`.
    ///
    /// `collapse` is the evaluation mode's duplicate-key rule, carried here
    /// rather than settled before construction (#1514). #1385 built this
    /// variant only for objects a `collapsed_fields` probe had already
    /// declared clean, which meant a whole extra cons-list walk -- with a
    /// `key_str()` decode per field -- ahead of the walk that writes the
    /// output. Every consumer below now applies the rule through
    /// `DistinctKeyCursors` during the walk it was making anyway.
    LazyKeysArray {
        /// The object whose keys this array presents.
        fields: crate::json::light::JsonFields<'a, W>,
        /// Whether a repeated key collapses onto its first occurrence.
        collapse: bool,
    },

    /// Lazy array-index range: `keys`/`keys_unsorted` on an array (#684).
    /// `[0, 1, ..., len-1]` is fully determined by `len` alone, so
    /// `write_json`/`print_json` write the digits directly — no
    /// `Vec<OwnedValue::Int>`/`Vec<Self>` ever built.
    LazyIndexRange(usize),
}

impl<'a, W: Clone + AsRef<[u64]>> JqValue<'a, W> {
    // =========================================================================
    // Constructors
    // =========================================================================

    /// Create a null value.
    #[inline]
    pub fn null() -> Self {
        JqValue::Null
    }

    /// Create a boolean value.
    #[inline]
    pub fn bool(b: bool) -> Self {
        JqValue::Bool(b)
    }

    /// Create an integer value.
    #[inline]
    pub fn int(n: i64) -> Self {
        JqValue::Int(n)
    }

    /// Create a float value.
    #[inline]
    pub fn float(f: f64) -> Self {
        JqValue::Float(f)
    }

    /// Create a string value.
    #[inline]
    pub fn string(s: impl Into<String>) -> Self {
        JqValue::String(s.into())
    }

    /// Create an empty array.
    #[inline]
    pub fn empty_array() -> Self {
        JqValue::Array(Vec::new())
    }

    /// Create an array from values.
    #[inline]
    pub fn array(values: Vec<Self>) -> Self {
        JqValue::Array(values)
    }

    /// Create an empty object.
    #[inline]
    pub fn empty_object() -> Self {
        JqValue::Object(IndexMap::new())
    }

    /// Create an object from key-value pairs.
    #[inline]
    pub fn object(pairs: impl IntoIterator<Item = (String, Self)>) -> Self {
        JqValue::Object(pairs.into_iter().collect())
    }

    /// Create from a cursor (lazy reference).
    #[inline]
    pub fn from_cursor(cursor: JsonCursor<'a, W>) -> Self {
        JqValue::Cursor(cursor)
    }

    /// Create from a literal.
    ///
    /// Only `NumberLiteral` needs its own arm (#1062): `JqValue` defers
    /// parsing a number literal's text until it's actually read, unlike
    /// `OwnedValue::from_number_literal_boxed` (via `super::value::OwnedValue:
    /// From<Literal>`), which parses eagerly -- the two target types
    /// genuinely disagree on when that work happens, so this one arm can't
    /// delegate. Every other variant carries no such difference, so those
    /// route through the same canonical conversion `literal_to_owned` uses,
    /// rather than re-listing five arms a third time.
    pub fn from_literal(lit: &Literal) -> Self {
        match lit {
            // `repr` (#1062) is ignored here -- `JqValue`'s own laziness
            // (this arm's whole reason for existing, see the doc comment
            // above) means the parsed value is deliberately not read until
            // needed, regardless of whether a `NumberRepr` happens to
            // already be sitting on the node.
            Literal::NumberLiteral(_repr, text) => JqValue::NumberLiteral(text.as_str().into()),
            _ => JqValue::from_owned(OwnedValue::from(lit.clone())),
        }
    }

    /// Create from an OwnedValue.
    pub fn from_owned(owned: OwnedValue) -> Self {
        Self::from_owned_at_depth(owned, 0)
    }

    /// Panics past [`MAX_VALUE_TREE_DEPTH`](super::value::MAX_VALUE_TREE_DEPTH)
    /// levels of nesting (#1025) -- `OwnedValue::Array`/`Object` are `pub`
    /// and re-exported from `succinctly::jq`, so any library consumer can
    /// build a deeply-nested value with a plain loop (no recursion needed
    /// to construct it) and hand it to this constructor.
    fn from_owned_at_depth(owned: OwnedValue, depth: usize) -> Self {
        assert_value_tree_depth(depth);
        match owned {
            OwnedValue::Null => JqValue::Null,
            OwnedValue::Bool(b) => JqValue::Bool(b),
            OwnedValue::Int(n) => JqValue::Int(n),
            OwnedValue::Float(f) => JqValue::Float(f),
            OwnedValue::NumberLiteral(_, literal) => JqValue::NumberLiteral(literal),
            OwnedValue::String(s) => JqValue::String(s),
            OwnedValue::Array(arr) => JqValue::Array(
                arr.into_iter()
                    .map(|v| Self::from_owned_at_depth(v, depth + 1))
                    .collect(),
            ),
            OwnedValue::Object(obj) => JqValue::Object(
                obj.into_iter()
                    .map(|(k, v)| (k, Self::from_owned_at_depth(v, depth + 1)))
                    .collect(),
            ),
        }
    }

    /// [`from_owned`](Self::from_owned)'s checked twin: reports a value
    /// nested past [`MAX_VALUE_TREE_DEPTH`](super::value::MAX_VALUE_TREE_DEPTH)
    /// as an ordinary [`EvalError`] instead of panicking (#1371).
    ///
    /// The panicking form stays for library callers that own their input and
    /// treat over-deep nesting as a bug. The CLI is the opposite case: since
    /// a `def` recurses by evaluation rather than by pre-substituted body
    /// (#1371), an ordinary recursive filter can now build a value deeper
    /// than the ceiling -- `def deep(n): if n == 0 then . else [[deep(n-1)]]
    /// end;` at a few hundred levels -- and taking the whole process down
    /// with a panic for a filter a user simply typed is the failure mode
    /// #1098 established this codebase does not ship.
    ///
    /// Costs nothing extra: the depth is already being carried down the
    /// conversion that has to happen anyway, so this is the same walk with
    /// its assertion turned into a returned error.
    pub fn try_from_owned(owned: OwnedValue) -> Result<Self, EvalError> {
        Self::try_from_owned_at_depth(owned, 0)
    }

    fn try_from_owned_at_depth(owned: OwnedValue, depth: usize) -> Result<Self, EvalError> {
        if depth >= super::value::MAX_VALUE_TREE_DEPTH {
            return Err(EvalError::new(
                super::value::nesting_depth_exceeded_message(super::value::MAX_VALUE_TREE_DEPTH),
            ));
        }
        Ok(match owned {
            OwnedValue::Array(arr) => JqValue::Array(
                arr.into_iter()
                    .map(|v| Self::try_from_owned_at_depth(v, depth + 1))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
            OwnedValue::Object(obj) => JqValue::Object(
                obj.into_iter()
                    .map(|(k, v)| Self::try_from_owned_at_depth(v, depth + 1).map(|v| (k, v)))
                    .collect::<Result<IndexMap<_, _>, _>>()?,
            ),
            // Every scalar arm is exactly `from_owned_at_depth`'s, reached
            // only after the depth check above; kept as one delegation
            // rather than six duplicated arms so the two constructors cannot
            // drift on how a scalar is represented.
            scalar => Self::from_owned_at_depth(scalar, depth),
        })
    }

    // =========================================================================
    // Type checking
    // =========================================================================

    /// Check if this is a lazy cursor reference.
    #[inline]
    pub fn is_cursor(&self) -> bool {
        matches!(self, JqValue::Cursor(_))
    }

    /// Check if this value is null.
    pub fn is_null(&self) -> bool {
        match self {
            JqValue::Null => true,
            JqValue::Cursor(c) => matches!(c.value(), StandardJson::Null),
            _ => false,
        }
    }

    /// Check if this value is "truthy" (not null and not false).
    ///
    /// In jq, only `null` and `false` are falsy. Everything else
    /// (including 0, "", [], {}) is truthy.
    pub fn is_truthy(&self) -> bool {
        match self {
            JqValue::Null => false,
            JqValue::Bool(false) => false,
            JqValue::Cursor(c) => {
                !matches!(c.value(), StandardJson::Null | StandardJson::Bool(false))
            }
            _ => true,
        }
    }

    /// Get the type name of this value (for error messages).
    pub fn type_name(&self) -> &'static str {
        match self {
            JqValue::Cursor(c) => match c.value() {
                StandardJson::Null => "null",
                StandardJson::Bool(_) => "boolean",
                StandardJson::Number(_) => "number",
                StandardJson::String(_) => "string",
                StandardJson::Array(_) => "array",
                StandardJson::Object(_) => "object",
                StandardJson::Error(_) => "error",
            },
            JqValue::Null => "null",
            JqValue::Bool(_) => "boolean",
            JqValue::Int(_)
            | JqValue::Float(_)
            | JqValue::RawNumber(_)
            | JqValue::NumberLiteral(_) => "number",
            JqValue::String(_) => "string",
            JqValue::Array(_) => "array",
            JqValue::Object(_) => "object",
            JqValue::LazyKeysArray { .. } => "array",
            JqValue::LazyIndexRange(_) => "array",
        }
    }

    // =========================================================================
    // Value accessors (force materialization when needed)
    // =========================================================================

    /// Get as boolean, if this is a boolean value.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            JqValue::Bool(b) => Some(*b),
            JqValue::Cursor(c) => match c.value() {
                StandardJson::Bool(b) => Some(b),
                _ => None,
            },
            _ => None,
        }
    }

    /// Get as i64, if this is an integer or integer-valued float.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            JqValue::Int(n) => Some(*n),
            JqValue::Float(f) if (*f - (*f as i64 as f64)).abs() < f64::EPSILON => Some(*f as i64),
            JqValue::NumberLiteral(literal) => OwnedValue::from_number_literal(literal).as_i64(),
            JqValue::RawNumber(bytes) => core::str::from_utf8(bytes)
                .ok()
                .and_then(|s| s.parse().ok()),
            JqValue::Cursor(c) => match c.value() {
                StandardJson::Number(n) => n.as_i64().ok(),
                _ => None,
            },
            _ => None,
        }
    }

    /// Get as f64, if this is a number.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            JqValue::Int(n) => Some(*n as f64),
            JqValue::Float(f) => Some(*f),
            JqValue::NumberLiteral(literal) => OwnedValue::from_number_literal(literal).as_f64(),
            JqValue::RawNumber(bytes) => core::str::from_utf8(bytes)
                .ok()
                .and_then(|s| s.parse().ok()),
            JqValue::Cursor(c) => match c.value() {
                StandardJson::Number(n) => n.as_f64().ok(),
                _ => None,
            },
            _ => None,
        }
    }

    /// Get as string reference.
    ///
    /// Returns `Cow` because:
    /// - For `JqValue::String`, returns a borrowed reference
    /// - For `JqValue::Cursor`, may need to unescape into owned string
    pub fn as_str(&self) -> Option<Cow<'_, str>> {
        match self {
            JqValue::String(s) => Some(Cow::Borrowed(s.as_str())),
            JqValue::Cursor(c) => match c.value() {
                StandardJson::String(s) => s.as_str().ok(),
                _ => None,
            },
            _ => None,
        }
    }

    /// Get the length of this value.
    ///
    /// - null: 0 (jq compat)
    /// - string: UTF-8 codepoint count
    /// - array: element count
    /// - object: key count
    /// - other: None (error)
    pub fn length(&self) -> Option<usize> {
        match self {
            JqValue::Null => Some(0),
            JqValue::String(s) => Some(s.chars().count()),
            JqValue::Array(arr) => Some(arr.len()),
            JqValue::Object(obj) => Some(obj.len()),
            // No wildcard covers this case: without this arm,
            // `keys_unsorted | length` reaching `JqValue` directly would
            // silently answer `None` instead of the field count.
            JqValue::LazyKeysArray { fields, collapse } => Some(effective_len(fields, *collapse)),
            // No wildcard covers this case either, for the same reason as
            // `LazyKeysArray` above: without it, `keys_unsorted | length` on
            // an array reaching `JqValue` directly would silently answer
            // `None` instead of `len` (#684).
            JqValue::LazyIndexRange(len) => Some(*len),
            JqValue::Cursor(c) => match c.value() {
                StandardJson::Null => Some(0),
                StandardJson::String(s) => s.as_str().ok().map(|s| s.chars().count()),
                StandardJson::Array(elements) => Some(elements.count()),
                StandardJson::Object(fields) => Some(fields.count()),
                _ => None,
            },
            _ => None,
        }
    }

    /// [`length`](Self::length), refusing a `LazyKeysArray` whose walk hits
    /// a #1194/#1677 malformed member instead of silently returning a count
    /// that omits it (#1974).
    ///
    /// `length()` itself stays infallible -- it has no internal caller in
    /// this crate (every jq-side `length` evaluation goes through the
    /// generic evaluator's own `effective_len_checked`-backed path
    /// already), it's public API surface for external `succinctly::jq`
    /// consumers constructing a `JqValue` directly, and changing its
    /// existing signature would be a breaking change for no internal
    /// benefit. This is an additive sibling instead, mirroring the
    /// `effective_len`/`effective_len_checked` naming convention
    /// `LazyKeysArray`'s own arm already delegates to.
    pub fn length_checked(&self) -> Result<Option<usize>, EvalError> {
        match self {
            JqValue::LazyKeysArray { fields, collapse } => {
                Ok(Some(effective_len_checked(fields, *collapse)?))
            }
            _ => Ok(self.length()),
        }
    }

    // =========================================================================
    // Navigation (for cursor values)
    // =========================================================================

    /// Get the cursor if this is a lazy value.
    #[inline]
    pub fn as_cursor(&self) -> Option<&JsonCursor<'a, W>> {
        match self {
            JqValue::Cursor(c) => Some(c),
            _ => None,
        }
    }

    /// Get the StandardJson value for a cursor, or None if materialized.
    pub fn as_standard_json(&self) -> Option<StandardJson<'a, W>> {
        match self {
            JqValue::Cursor(c) => Some(c.value()),
            _ => None,
        }
    }

    // =========================================================================
    // Materialization
    // =========================================================================

    /// Force full materialization into an OwnedValue.
    ///
    /// This recursively materializes all nested values. Use sparingly -
    /// prefer keeping values as cursors when possible.
    /// `Err` when a string scalar in the tree cannot be decoded (#1247) --
    /// it used to materialize as an empty string, silently replacing the
    /// value with something that compares, sorts and prints as real data.
    pub fn materialize(&self) -> Result<OwnedValue, EvalError> {
        self.materialize_at_depth(0)
    }

    /// Panics past [`MAX_VALUE_TREE_DEPTH`](super::value::MAX_VALUE_TREE_DEPTH)
    /// levels of nesting (#1021, following #1005's precedent).
    fn materialize_at_depth(&self, depth: usize) -> Result<OwnedValue, EvalError> {
        assert_value_tree_depth(depth);
        Ok(match self {
            JqValue::Cursor(c) => cursor_to_owned(c)?,
            JqValue::Null => OwnedValue::Null,
            JqValue::Bool(b) => OwnedValue::Bool(*b),
            JqValue::Int(n) => OwnedValue::Int(*n),
            JqValue::Float(f) => OwnedValue::Float(*f),
            JqValue::RawNumber(bytes) => OwnedValue::from_number_bytes(bytes),
            JqValue::NumberLiteral(literal) => OwnedValue::from_number_literal(literal),
            JqValue::String(s) => OwnedValue::String(s.clone()),
            JqValue::Array(arr) => OwnedValue::Array(
                arr.iter()
                    .map(|v| v.materialize_at_depth(depth + 1))
                    .collect::<Result<_, _>>()?,
            ),
            JqValue::Object(obj) => OwnedValue::Object(
                obj.iter()
                    .map(|(k, v)| Ok((k.clone(), v.materialize_at_depth(depth + 1)?)))
                    .collect::<Result<_, EvalError>>()?,
            ),
            JqValue::LazyKeysArray { fields, collapse } => {
                lazy_keys_array_to_owned(fields, *collapse)?
            }
            JqValue::LazyIndexRange(len) => lazy_index_range_to_owned(*len),
        })
    }

    /// Convert to OwnedValue, consuming self.
    ///
    /// More efficient than `materialize()` when you don't need to keep the original.
    /// `Err` for the same reason [`materialize`](Self::materialize) does.
    pub fn into_owned(self) -> Result<OwnedValue, EvalError> {
        self.into_owned_at_depth(0)
    }

    /// Panics past [`MAX_VALUE_TREE_DEPTH`](super::value::MAX_VALUE_TREE_DEPTH)
    /// levels of nesting (#1021, following #1005's precedent).
    fn into_owned_at_depth(self, depth: usize) -> Result<OwnedValue, EvalError> {
        assert_value_tree_depth(depth);
        Ok(match self {
            JqValue::Cursor(c) => cursor_to_owned(&c)?,
            JqValue::Null => OwnedValue::Null,
            JqValue::Bool(b) => OwnedValue::Bool(b),
            JqValue::Int(n) => OwnedValue::Int(n),
            JqValue::Float(f) => OwnedValue::Float(f),
            JqValue::RawNumber(bytes) => OwnedValue::from_number_bytes(bytes),
            JqValue::NumberLiteral(literal) => OwnedValue::from_number_literal_boxed(literal),
            JqValue::String(s) => OwnedValue::String(s),
            JqValue::Array(arr) => OwnedValue::Array(
                arr.into_iter()
                    .map(|v| v.into_owned_at_depth(depth + 1))
                    .collect::<Result<_, _>>()?,
            ),
            JqValue::Object(obj) => OwnedValue::Object(
                obj.into_iter()
                    .map(|(k, v)| Ok((k, v.into_owned_at_depth(depth + 1)?)))
                    .collect::<Result<_, EvalError>>()?,
            ),
            JqValue::LazyKeysArray { fields, collapse } => {
                lazy_keys_array_to_owned(&fields, collapse)?
            }
            JqValue::LazyIndexRange(len) => lazy_index_range_to_owned(len),
        })
    }

    // =========================================================================
    // Output (preserves original formatting when possible)
    // =========================================================================

    /// Get the raw bytes for this value if it's a lazy reference.
    ///
    /// Returns `Some(&[u8])` for cursor values and raw numbers, `None` for
    /// materialized values. This allows zero-copy output for pass-through queries.
    pub fn raw_bytes(&self) -> Option<&'a [u8]> {
        match self {
            JqValue::Cursor(c) => c.raw_bytes(),
            JqValue::RawNumber(bytes) => Some(bytes),
            _ => None,
        }
    }

    /// Write this value as JSON to a writer.
    ///
    /// For cursor values, writes the original bytes (preserving formatting).
    /// For materialized values, serializes to JSON.
    ///
    /// This is the preferred way to output JqValue because it preserves
    /// number formatting like `4e4` for cursor values.
    pub fn write_json<Out: core::fmt::Write>(&self, out: &mut Out) -> core::fmt::Result {
        self.write_json_at_depth(out, 0)
    }

    /// Panics past [`MAX_VALUE_TREE_DEPTH`](super::value::MAX_VALUE_TREE_DEPTH)
    /// levels of nesting (#1025) -- `OwnedValue::Array`/`Object` are `pub`
    /// and re-exported from `succinctly::jq`, so any library consumer can
    /// build a deeply-nested `JqValue` (via [`Self::from_owned`], itself
    /// guarded) and hand it to this serializer.
    fn write_json_at_depth<Out: core::fmt::Write>(
        &self,
        out: &mut Out,
        depth: usize,
    ) -> core::fmt::Result {
        assert_value_tree_depth(depth);
        match self {
            JqValue::Cursor(c) => {
                if let Some(bytes) = c.raw_bytes() {
                    // Write raw bytes (preserves original formatting)
                    let s = core::str::from_utf8(bytes).map_err(|_| core::fmt::Error)?;
                    out.write_str(s)
                } else {
                    // Fallback: materialize and serialize. `write_json` has
                    // only `core::fmt::Error` to report with, which carries
                    // no message -- but this branch is unreachable for a
                    // decode failure anyway: it needs a cursor with no raw
                    // byte span, and every JSON string token has one. The
                    // `?` is here so a future shape that *can* fail cannot
                    // silently print a wrong value (#1247).
                    let owned = cursor_to_owned(c).map_err(|_| core::fmt::Error)?;
                    out.write_str(&owned.to_json())
                }
            }
            JqValue::Null => out.write_str("null"),
            JqValue::Bool(true) => out.write_str("true"),
            JqValue::Bool(false) => out.write_str("false"),
            JqValue::Int(n) => write!(out, "{n}"),
            JqValue::Float(f) => {
                if f.is_nan() {
                    out.write_str("null") // JSON doesn't support NaN
                } else if f.is_infinite() {
                    // A computed Infinity has no source literal to echo, so
                    // it renders jq's own DBL_MAX text instead of "null"
                    // (#1087) -- `JqValue` is jq-mode-only (no
                    // `yq_runner.rs` caller exists), so there's no yq
                    // convention to preserve here, unlike `OwnedValue::to_json`'s
                    // mode-generic sibling.
                    out.write_str(infinite_float_preview_text(f.is_sign_negative()))
                } else {
                    write!(out, "{f}")
                }
            }
            JqValue::RawNumber(bytes) => {
                // Write raw bytes directly (preserves formatting like "4e4")
                let s = core::str::from_utf8(bytes).map_err(|_| core::fmt::Error)?;
                out.write_str(s)
            }
            JqValue::NumberLiteral(literal) => out.write_str(literal),
            JqValue::String(s) => {
                out.write_char('"')?;
                write_json_body_jq(out, s)?;
                out.write_char('"')
            }
            JqValue::Array(arr) => {
                out.write_char('[')?;
                for (i, v) in arr.iter().enumerate() {
                    if i > 0 {
                        out.write_char(',')?;
                    }
                    v.write_json_at_depth(out, depth + 1)?;
                }
                out.write_char(']')
            }
            JqValue::Object(obj) => {
                out.write_char('{')?;
                for (i, (k, v)) in obj.iter().enumerate() {
                    if i > 0 {
                        out.write_char(',')?;
                    }
                    // Write key. This used to escape only `"` and `\`, which
                    // let a control character in a key through raw and so
                    // emitted invalid JSON; keys take the same convention as
                    // values.
                    out.write_char('"')?;
                    write_json_body_jq(out, k)?;
                    out.write_str("\":")?;
                    v.write_json_at_depth(out, depth + 1)?;
                }
                out.write_char('}')
            }
            // Genuinely lazy: each key's raw bytes come straight from its
            // cursor — already a valid, already-escaped JSON string token —
            // so this never allocates a `String` per key, unlike
            // `JqValue::Object`/`Array` above.
            JqValue::LazyKeysArray { fields, collapse } => {
                out.write_char('[')?;
                let mut first = true;
                let mut cursors = DistinctKeyCursors::new(fields, *collapse);
                for (key, key_cursor) in cursors.by_ref() {
                    // #1679: a key the format's grammar never allowed at all
                    // (a #1194-shaped key, e.g. a bare numeric JSON key) is
                    // not an already-quoted JSON string token -- writing its
                    // raw bytes verbatim would produce invalid JSON
                    // (`{123:1}` becoming `[123]` instead of raising).
                    // `write_json` has only `core::fmt::Error` to report
                    // with (no message), matching `JqValue::Cursor`'s own
                    // `cursor_to_owned(c).map_err(|_| core::fmt::Error)?`
                    // above.
                    if !matches!(key, StandardJson::String(_)) {
                        return Err(core::fmt::Error);
                    }
                    if !first {
                        out.write_char(',')?;
                    }
                    first = false;
                    match key_cursor.raw_bytes() {
                        Some(bytes) => {
                            let s = core::str::from_utf8(bytes).map_err(|_| core::fmt::Error)?;
                            out.write_str(s)?;
                        }
                        // Defensive fallback; JSON string-token keys are
                        // always given a text range by the semi-index, so
                        // this is not expected to be reached. The key is
                        // already confirmed to be `StandardJson::String`
                        // above, so this always writes a real key rather
                        // than the `null` placeholder it used to.
                        None => {
                            out.write_char('"')?;
                            if let StandardJson::String(k) = key {
                                if let Ok(s) = k.as_str() {
                                    write_json_body_jq(out, &s)?;
                                }
                            }
                            out.write_char('"')?;
                        }
                    }
                }
                // #1956: matches `eval_generic.rs`'s own
                // `distinct_key_cursors_checked`/`keys_are_well_formed`,
                // which both check this via `is_malformed()`.
                if cursors.is_malformed() {
                    return Err(core::fmt::Error);
                }
                out.write_char(']')
            }
            // Genuinely lazy, same convention as `LazyKeysArray` above: no
            // `Vec<OwnedValue::Int>` ever built, just digits written
            // straight to `out` (#684).
            JqValue::LazyIndexRange(len) => {
                out.write_char('[')?;
                for i in 0..*len {
                    if i > 0 {
                        out.write_char(',')?;
                    }
                    write!(out, "{i}")?;
                }
                out.write_char(']')
            }
        }
    }

    /// Convert this value to a JSON string.
    ///
    /// For cursor values, returns the original bytes as a string.
    /// For materialized values, serializes to JSON.
    pub fn to_json_string(&self) -> String {
        let mut out = String::new();
        // Writing to a `String` itself can't fail, but `write_json` can --
        // a `LazyKeysArray` malformed per #1194/#1677 returns `Err` after
        // `out` already holds a truncated fragment (#1956; see
        // `write_json_at_depth`'s own doc comment). No caller of this
        // function today needs to distinguish "well-formed" from
        // "truncated on error", so the `Err` is intentionally discarded --
        // just not for the reason the old comment gave.
        let _ = self.write_json(&mut out);
        out
    }
}

/// Materialize a `JqValue::LazyKeysArray` into the `Vec<String>` array it
/// would have been all along — the escape hatch for consumers that need a
/// materialized value (`sort_keys`, color output, etc.).
fn lazy_keys_array_to_owned<W: Clone + AsRef<[u64]>>(
    fields: &crate::json::light::JsonFields<'_, W>,
    collapse: bool,
) -> Result<OwnedValue, EvalError> {
    let mut keys = Vec::new();
    // #1679: mirrors `effective_keys`'s pattern exactly -- a non-string key
    // (#1194) now raises instead of silently dropping the field, and the
    // walk's post-exhaustion `ended_unpaired()` catches the other #1194
    // shape (a trailing key with no paired value) that a mid-loop check
    // alone can't see.
    let mut cursors = DistinctKeyCursors::new(fields, collapse);
    for (key, _) in cursors.by_ref() {
        // A key that will not *decode* is preserved via its raw source span
        // rather than raised on (#1247/#1642), matching
        // `length`/`keys_unsorted`/`.`.
        let Some(s) = key_display_string(&key) else {
            return Err(fields.malformed_member_error());
        };
        keys.push(OwnedValue::String(s.into_owned()));
    }
    // #1956: matches `eval_generic.rs`'s own `distinct_key_cursors_checked`/
    // `keys_are_well_formed`, which both check this via `is_malformed()`.
    if cursors.is_malformed() {
        return Err(fields.malformed_member_error());
    }
    Ok(OwnedValue::Array(keys))
}

/// Materialize a `JqValue::LazyIndexRange` into the `[0, 1, ..., len-1]`
/// array it would have been all along (#684) — the escape hatch for
/// consumers that need a materialized value.
fn lazy_index_range_to_owned(len: usize) -> OwnedValue {
    OwnedValue::Array((0..len).map(|i| OwnedValue::Int(i as i64)).collect())
}

/// Convert a JsonCursor to an OwnedValue (full materialization).
///
/// Panics past [`super::eval_generic::MAX_NESTING_DEPTH`] levels of nesting
/// (#998) rather than recursing unbounded and overflowing the call stack --
/// a second, independent materializer with the identical unguarded shape
/// `eval_generic::to_owned_cursor` had, missed by that fix's own review pass
/// and only found once it: `--exit-status`/`-e` forces `JqValue::
/// materialize()` (see `Materializable::materialize` below) on every result
/// before `jq_runner.rs`'s own guarded `print_json` output path ever runs,
/// reaching this function directly. Confirmed live: `succinctly jq -e
/// '.[0]'` on a 200,000-level-deep document raw-stack-overflowed (SIGABRT)
/// even after `to_owned_cursor`'s own guard existed.
fn cursor_to_owned<W: Clone + AsRef<[u64]>>(
    cursor: &JsonCursor<'_, W>,
) -> Result<OwnedValue, EvalError> {
    cursor_to_owned_at_depth(cursor, 0)
}

fn cursor_to_owned_at_depth<W: Clone + AsRef<[u64]>>(
    cursor: &JsonCursor<'_, W>,
    depth: usize,
) -> Result<OwnedValue, EvalError> {
    super::eval_generic::assert_nesting_depth(depth);
    Ok(match cursor.value() {
        StandardJson::Null => OwnedValue::Null,
        StandardJson::Bool(b) => OwnedValue::Bool(b),
        StandardJson::Number(n) => OwnedValue::from_number_bytes(n.raw_bytes()),
        StandardJson::String(s) => OwnedValue::String(
            // Was an empty string, which silently replaced the real value
            // (#1098, #1247) -- the sibling `to_owned_at_depth` in
            // eval_generic.rs swallowed the same case as `null`. Both raise
            // now, with the wording #1192 established.
            s.as_str()
                .map_err(|e| EvalError::decode_failure(format!("{e}")))?
                .into_owned(),
        ),
        StandardJson::Array(_) => {
            // Use cursor navigation to iterate children
            let mut items = Vec::new();
            let mut is_first = true;
            // #2358: the last real element's own cursor, retained past the
            // loop so the trailing-gap check below (`[1,]`) has something
            // to check from -- mirrors the `Object` arm's own `last_field`.
            let mut last_elem: Option<JsonCursor<'_, W>> = None;
            for child in cursor.children() {
                // #2211 code review: this walk (unlike its
                // `eval_generic::to_owned_cursor_at_depth` sibling it
                // otherwise mirrors) never ran this check at all -- not even
                // the missing/doubled-comma-between-two-real-elements case
                // (#1677), which the `Object` arm below already had.
                // `{"a" 1, "b": 2} | -e` used to silently succeed for the
                // array shape (`[1 2, 3]`) the same way this object shape
                // used to before #1956. #1803: via the shared
                // `element_gap_ok` rather than an inline copy of it.
                if !child.element_gap_ok(is_first) {
                    return Err(child.malformed_delimiter_error());
                }
                items.push(cursor_to_owned_at_depth(&child, depth + 1)?);
                last_elem = Some(child);
                is_first = false;
            }
            // #2211/#2243, via the shared `container_tail_gap_ok`. #2358
            // closes the STYLE-0013 exemption this walk used to carry: a
            // stray `,` with no real element at all (`[,]`) was already
            // checked, directly against `container_gap_ok`, but a stray `,`
            // *after* a real last element (`[1,]`) was not -- unlike every
            // other materializer sharing this same helper. Adopting it here
            // is the behaviour change that comment used to defer.
            container_tail_gap_ok(cursor, last_elem.as_ref(), b']')?;
            OwnedValue::Array(items)
        }
        StandardJson::Object(fields) => {
            let mut map = IndexMap::new();
            let mut guard = DisplayKeyGuard::default();
            // #1679: restructured from `for field in fields` to `uncons`
            // so the walk can tell "ran out of fields" apart from "ran out
            // on an unpaired child" (#1194) -- mirrors
            // `eval_generic::to_owned_at_depth`'s identical shape.
            let mut f = fields;
            let mut is_first = true;
            // #2358: same reasoning as the `Array` arm's own `last_elem`
            // above.
            let mut last_field: Option<JsonCursor<'_, W>> = None;
            // #1803: `DocumentFields::uncons`, not the inherent
            // `JsonFields::uncons` -- the trait impl (`json::light`) builds
            // its `DocumentField` from exactly the four accessors this loop
            // already called by hand, so nothing extra is resolved, and it
            // is what lets this walk share `checked_key` with its generic
            // siblings rather than hand-copying their checks again. #1956
            // is the reminder of why that matters: this walk (unlike the
            // `eval_generic::to_owned_at_depth` sibling it otherwise
            // mirrors) had no delimiter checks at all until then, so a
            // missing or doubled `,`/`:` with an otherwise-even member
            // count silently succeeded here.
            while let Some((field, rest)) = DocumentFields::uncons(&f) {
                let key = field.checked_key(&f, &map, &mut guard, is_first)?;
                map.insert(
                    key,
                    cursor_to_owned_at_depth(&field.value_cursor, depth + 1)?,
                );
                last_field = Some(field.value_cursor);
                f = rest;
                is_first = false;
            }
            if f.ends_unpaired() {
                return Err(f.malformed_member_error());
            }
            // #2211/#2243, via the shared `container_tail_gap_ok` -- #2358
            // closes this walk's own STYLE-0013 exemption; see the `Array`
            // arm's identical comment above.
            container_tail_gap_ok(cursor, last_field.as_ref(), b'}')?;
            OwnedValue::Object(map)
        }
        // See `eval_generic::to_owned_at_depth`'s own `is_error` arm
        // (#1194/#1247): a structurally malformed value raises rather than
        // becoming `null`. #2286: decode_failure, not new -- same class as
        // the malformed member/delimiter errors; confirmed live that real
        // jq treats this uncatchably too.
        StandardJson::Error(msg) => return Err(EvalError::decode_failure(msg)),
    })
}

// ============================================================================
// From implementations
// ============================================================================

impl<W> From<bool> for JqValue<'_, W> {
    fn from(b: bool) -> Self {
        JqValue::Bool(b)
    }
}

impl<W> From<i64> for JqValue<'_, W> {
    fn from(n: i64) -> Self {
        JqValue::Int(n)
    }
}

impl<W> From<f64> for JqValue<'_, W> {
    fn from(f: f64) -> Self {
        JqValue::Float(f)
    }
}

impl<W> From<String> for JqValue<'_, W> {
    fn from(s: String) -> Self {
        JqValue::String(s)
    }
}

impl<W> From<&str> for JqValue<'_, W> {
    fn from(s: &str) -> Self {
        JqValue::String(s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constructors() {
        let null: JqValue<'_, Vec<u64>> = JqValue::null();
        assert!(null.is_null());

        let b: JqValue<'_, Vec<u64>> = JqValue::bool(true);
        assert_eq!(b.as_bool(), Some(true));

        let n: JqValue<'_, Vec<u64>> = JqValue::int(42);
        assert_eq!(n.as_i64(), Some(42));

        let f: JqValue<'_, Vec<u64>> = JqValue::float(2.5);
        assert!((f.as_f64().unwrap() - 2.5).abs() < f64::EPSILON);

        let s: JqValue<'_, Vec<u64>> = JqValue::string("hello");
        assert_eq!(
            s.as_str().map(alloc::borrow::Cow::into_owned),
            Some("hello".to_string())
        );
    }

    #[test]
    fn test_type_name() {
        let null: JqValue<'_, Vec<u64>> = JqValue::null();
        assert_eq!(null.type_name(), "null");

        let b: JqValue<'_, Vec<u64>> = JqValue::bool(true);
        assert_eq!(b.type_name(), "boolean");

        let n: JqValue<'_, Vec<u64>> = JqValue::int(42);
        assert_eq!(n.type_name(), "number");

        let f: JqValue<'_, Vec<u64>> = JqValue::float(2.5);
        assert_eq!(f.type_name(), "number");

        let s: JqValue<'_, Vec<u64>> = JqValue::string("hello");
        assert_eq!(s.type_name(), "string");

        let arr: JqValue<'_, Vec<u64>> = JqValue::empty_array();
        assert_eq!(arr.type_name(), "array");

        let obj: JqValue<'_, Vec<u64>> = JqValue::empty_object();
        assert_eq!(obj.type_name(), "object");
    }

    #[test]
    fn test_truthy() {
        let null: JqValue<'_, Vec<u64>> = JqValue::null();
        assert!(!null.is_truthy());

        let false_val: JqValue<'_, Vec<u64>> = JqValue::bool(false);
        assert!(!false_val.is_truthy());

        let true_val: JqValue<'_, Vec<u64>> = JqValue::bool(true);
        assert!(true_val.is_truthy());

        // In jq, 0 is truthy!
        let zero: JqValue<'_, Vec<u64>> = JqValue::int(0);
        assert!(zero.is_truthy());

        // Empty string is truthy in jq
        let empty: JqValue<'_, Vec<u64>> = JqValue::string("");
        assert!(empty.is_truthy());

        // Empty array is truthy in jq
        let arr: JqValue<'_, Vec<u64>> = JqValue::empty_array();
        assert!(arr.is_truthy());
    }

    #[test]
    fn test_length() {
        let null: JqValue<'_, Vec<u64>> = JqValue::null();
        assert_eq!(null.length(), Some(0));

        let s: JqValue<'_, Vec<u64>> = JqValue::string("hello");
        assert_eq!(s.length(), Some(5));

        let unicode: JqValue<'_, Vec<u64>> = JqValue::string("héllo");
        assert_eq!(unicode.length(), Some(5));

        let arr: JqValue<'_, Vec<u64>> = JqValue::array(vec![JqValue::int(1), JqValue::int(2)]);
        assert_eq!(arr.length(), Some(2));

        // Numbers don't have length
        let n: JqValue<'_, Vec<u64>> = JqValue::int(42);
        assert_eq!(n.length(), None);
    }

    #[test]
    fn test_materialize() {
        let arr: JqValue<'_, Vec<u64>> = JqValue::array(vec![
            JqValue::int(1),
            JqValue::string("hello"),
            JqValue::null(),
        ]);

        let owned = arr.materialize().unwrap();
        match owned {
            OwnedValue::Array(items) => {
                assert_eq!(items.len(), 3);
                assert_eq!(items[0], OwnedValue::Int(1));
                assert_eq!(items[1], OwnedValue::String("hello".to_string()));
                assert_eq!(items[2], OwnedValue::Null);
            }
            _ => panic!("expected array"),
        }
    }

    #[test]
    fn test_from_literal() {
        let lit = Literal::Int(42);
        let val: JqValue<'_, Vec<u64>> = JqValue::from_literal(&lit);
        assert_eq!(val.as_i64(), Some(42));

        let lit = Literal::String("hello".to_string());
        let val: JqValue<'_, Vec<u64>> = JqValue::from_literal(&lit);
        assert_eq!(
            val.as_str().map(alloc::borrow::Cow::into_owned),
            Some("hello".to_string())
        );

        // #1035: a filter-literal number keeps its own source spelling
        // through this construction path too, same as the eval.rs/
        // eval_generic.rs sibling conversions.
        let lit = Literal::number_literal("1.500".to_string());
        let val: JqValue<'_, Vec<u64>> = JqValue::from_literal(&lit);
        assert!(matches!(val, JqValue::NumberLiteral(ref s) if s.as_ref() == "1.500"));
        assert_eq!(val.as_f64(), Some(1.5));
    }

    /// #1098/#1247: sibling of `eval_generic::to_owned`'s own regression
    /// test -- `JsonIndex::build`'s semi-index scan finds string
    /// quote/escape boundaries without decoding/validating what's between
    /// them, so invalid UTF-8 inside a string span indexes fine and only
    /// surfaces once `materialize`/`into_owned` reaches
    /// `JsonString::as_str()`.
    ///
    /// This test used to assert the *degrade* -- an empty string, which is
    /// worse than the `null` its `eval_generic` sibling produced, because
    /// `""` is a perfectly ordinary value that compares, sorts and prints
    /// as real data. #1247 made both raise instead.
    #[test]
    fn test_materialize_errors_on_decode_failure_1247() {
        use crate::json::JsonIndex;

        let json: &[u8] = b"{\"a\": \"\xff\xfe\"}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let val = JqValue::from_cursor(cursor);
        let err = val
            .materialize()
            .expect_err("an undecodable string must not materialize");
        assert!(
            err.message.contains("invalid UTF-8"),
            "message: {}",
            err.message
        );
        // `into_owned` is a separate walk of the same tree; it must agree.
        let val: JqValue<'_, Vec<u64>> = JqValue::from_cursor(index.root(json));
        assert!(val.into_owned().is_err());
    }

    /// #2211 code review: `cursor_to_owned_at_depth`'s `Array` arm never
    /// validated *any* element delimiter at all -- unlike its `Object` arm
    /// neighbor, which already had the #1956/#1677 `key_delimiter_ok`/
    /// `value_delimiter_ok` check. `[1 2, 3]` is missing the comma between
    /// its first two (real) elements.
    ///
    /// No CLI-level test accompanies this: `-e`/`--exit-status` (the flag
    /// that forces `.materialize()` at all) still always prints its result
    /// through `write_output_jq_value`/`print_json` afterward regardless of
    /// what `.materialize()` found, and `print_json`'s own independent
    /// #1643/#1676 checks catch this document too -- confirmed live against
    /// the pre-fix binary (`git stash`) that `succinctly jq -e -c '...'`
    /// already rejected every one of this test's inputs before this fix,
    /// via that unrelated, redundant check rather than this function's own
    /// validation. Calling `materialize`/`into_owned` directly is the only
    /// way to exercise this function's own gap in isolation.
    #[test]
    fn materialize_raises_on_missing_delimiter_between_array_elements_2211() {
        use crate::json::JsonIndex;

        let json: &[u8] = b"[1 2, 3]";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let val = JqValue::from_cursor(cursor);
        let err = val
            .materialize()
            .expect_err("a missing comma between two real elements is not JSON");
        assert!(
            err.message.contains("Invalid JSON text"),
            "message: {}",
            err.message
        );

        let val: JqValue<'_, Vec<u64>> = JqValue::from_cursor(index.root(json));
        assert!(val.into_owned().is_err(), "into_owned must agree");
    }

    /// #2211: a stray `,` with no real child at all (`[,]`, `{,}`) left
    /// both the array loop (which now validates a *real* element's own
    /// preceding delimiter, added by this same fix) and the pre-existing
    /// object loop (which validates a real field's own key/value
    /// delimiters) with nothing to check a delimiter against, since neither
    /// loop body ever ran -- the same "no real child to check against" gap
    /// #2211's `eval_generic::to_owned_cursor_at_depth` fix closed via
    /// `DocumentCursor::container_gap_ok`, needed again here for this
    /// independent materializer.
    #[test]
    fn materialize_raises_on_stray_comma_in_empty_containers_2211() {
        use crate::json::JsonIndex;

        for json in [b"[,]".as_slice(), b"{,}".as_slice()] {
            let index = JsonIndex::build(json);
            let cursor = index.root(json);
            let val = JqValue::from_cursor(cursor);
            let err = val
                .materialize()
                .expect_err("a stray comma in an apparently-empty container is not JSON");
            assert!(
                err.message.contains("Invalid JSON text"),
                "{json:?}: message: {}",
                err.message
            );

            let val: JqValue<'_, Vec<u64>> = JqValue::from_cursor(index.root(json));
            assert!(val.into_owned().is_err(), "{json:?}: into_owned must agree");
        }
    }

    /// #2358: a stray `,` *after* a real last child (`[1,]`, `{"a":1,}`,
    /// #2243) now raises here too -- this materializer always holds a real
    /// cursor, at every depth including the true top level (unlike
    /// `eval_generic::to_owned_at_depth`'s bare-value entry point), so
    /// swapping its old `container_gap_ok`-only check for the shared
    /// `container_tail_gap_ok` closes the gap the STYLE-0013 comment this
    /// replaced used to defer to this issue, with no residual "top level
    /// stays unchecked" caveat the way `eval_generic.rs`'s fix has to carry.
    /// Confirmed live against `/usr/bin/jq` 1.7.1: both exit 5.
    #[test]
    fn materialize_raises_on_trailing_comma_after_last_child_2358() {
        use crate::json::JsonIndex;

        for json in [b"[1,]".as_slice(), br#"{"a":1,}"#.as_slice()] {
            let index = JsonIndex::build(json);
            let cursor = index.root(json);
            let val = JqValue::from_cursor(cursor);
            let err = val
                .materialize()
                .expect_err("a stray trailing comma after a real last child is not JSON");
            assert!(
                err.message.contains("Invalid JSON text"),
                "{json:?}: message: {}",
                err.message
            );

            let val: JqValue<'_, Vec<u64>> = JqValue::from_cursor(index.root(json));
            assert!(val.into_owned().is_err(), "{json:?}: into_owned must agree");
        }
    }

    /// #2211: well-formed arrays/objects (including genuinely empty ones,
    /// which must still materialize as `[]`/`{}` rather than being caught
    /// by either new check above) are unaffected.
    #[test]
    fn materialize_wellformed_containers_unaffected_2211() {
        use crate::json::JsonIndex;

        for (json, expected) in [
            (b"[]".as_slice(), OwnedValue::Array(vec![])),
            (b"{}".as_slice(), OwnedValue::Object(IndexMap::new())),
            (
                b"[1,2,3]".as_slice(),
                OwnedValue::Array(vec![
                    OwnedValue::from_number_literal("1"),
                    OwnedValue::from_number_literal("2"),
                    OwnedValue::from_number_literal("3"),
                ]),
            ),
        ] {
            let index = JsonIndex::build(json);
            let cursor = index.root(json);
            let val = JqValue::from_cursor(cursor);
            assert_eq!(
                val.materialize().unwrap(),
                expected,
                "{json:?}: materialize"
            );

            let val: JqValue<'_, Vec<u64>> = JqValue::from_cursor(index.root(json));
            assert_eq!(val.into_owned().unwrap(), expected, "{json:?}: into_owned");
        }
    }

    /// #1247 used to raise here; #1642 preserves instead, matching
    /// `length`/`keys_unsorted`/`.` -- an undecodable *key* must never drop
    /// its whole field silently (#1247's original fix), but nor should it
    /// make the object unusable when every other route already tolerates
    /// it.
    #[test]
    fn test_materialize_preserves_object_key_decode_failure_1642() {
        use crate::json::JsonIndex;

        let json: &[u8] = b"{\"\xff\xfe\": 1}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let val = JqValue::from_cursor(cursor);
        let owned = val
            .materialize()
            .expect("an undecodable key is preserved, not raised on (#1642)");
        assert_eq!(
            owned,
            OwnedValue::Object(IndexMap::from([(
                "\u{FFFD}\u{FFFD}".to_string(),
                OwnedValue::from_number_literal("1")
            )]))
        );
    }

    #[test]
    fn test_cursor_raw_bytes_simple_number() {
        use crate::json::JsonIndex;

        // Just a number
        let json = br"4e4";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);

        let val = JqValue::from_cursor(cursor);
        let bytes = val.raw_bytes();
        assert!(
            bytes.is_some(),
            "raw_bytes should return Some for simple number"
        );
        assert_eq!(bytes.unwrap(), json.as_slice());
    }

    #[test]
    fn test_write_json_preserves_cursor_format() {
        use crate::json::JsonIndex;

        // JSON with exponential notation that would be reformatted if parsed
        let json = br"4e4";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);

        let val = JqValue::from_cursor(cursor);
        let output = val.to_json_string();

        // Should preserve original format "4e4", not "40000"
        assert_eq!(output, "4e4");
    }

    #[test]
    fn test_write_json_materialized() {
        // Materialized values serialize normally
        let arr: JqValue<'_, Vec<u64>> = JqValue::array(vec![
            JqValue::int(1),
            JqValue::string("hello"),
            JqValue::bool(true),
        ]);

        let output = arr.to_json_string();
        assert_eq!(output, r#"[1,"hello",true]"#);
    }

    #[test]
    fn test_mixed_cursor_and_materialized() {
        use crate::json::JsonIndex;

        // Create a cursor value
        let json = br"4e4";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let cursor_val = JqValue::from_cursor(cursor);

        // Create a materialized value
        let computed_val: JqValue<'_, Vec<u64>> = JqValue::int(100);

        // Mix them in an array
        let arr = JqValue::array(vec![cursor_val, computed_val]);
        let output = arr.to_json_string();

        // cursor_val should preserve "4e4", computed_val is "100"
        assert_eq!(output, "[4e4,100]");
    }

    #[test]
    fn test_jqvalue_object_constructor() {
        let obj: JqValue<'_, Vec<u64>> = JqValue::object([("a".to_string(), JqValue::Int(1))]);
        assert_eq!(obj.to_json_string(), r#"{"a":1}"#);
    }

    #[test]
    fn test_jqvalue_array_into_owned() {
        let arr: JqValue<'_, Vec<u64>> = JqValue::Array(vec![JqValue::Int(1), JqValue::Int(2)]);
        assert_eq!(
            arr.into_owned().unwrap(),
            OwnedValue::Array(vec![OwnedValue::Int(1), OwnedValue::Int(2)])
        );
    }

    #[test]
    fn test_jqvalue_float_json() {
        let f: JqValue<'_, Vec<u64>> = JqValue::Float(2.5);
        assert_eq!(f.to_json_string(), "2.5");
    }

    #[test]
    fn test_jqvalue_number_literal_materialize_into_owned_and_write() {
        // `JqValue::NumberLiteral` is `OwnedValue::NumberLiteral`'s lazy-side
        // counterpart (built by `from_owned`, e.g. by the CLI's output
        // formatter).
        let owned = OwnedValue::from_number_literal("1e100");

        // `materialize`/`into_owned` round-trip through `OwnedValue`, whose
        // own `to_json` reformats through jq's canonical algorithm (see
        // `format_number_jq_compat`) -- so both give jq's spelling.
        let via_materialize: JqValue<'_, Vec<u64>> = JqValue::from_owned(owned.clone());
        assert!(matches!(via_materialize, JqValue::NumberLiteral(_)));
        assert_eq!(via_materialize.materialize().unwrap().to_json(), "1E+100");

        let via_into_owned: JqValue<'_, Vec<u64>> = JqValue::from_owned(owned.clone());
        assert_eq!(via_into_owned.into_owned().unwrap().to_json(), "1E+100");

        // `write_json`/`to_json_string`, by contrast, is this module's
        // documented format-*preserving* serializer (see the module doc and
        // `test_write_json_preserves_cursor_format`'s `4e4` case) -- it
        // writes the source literal verbatim rather than jq's reformatting,
        // consistent with how it treats `RawNumber`/`Cursor`.
        let via_write: JqValue<'_, Vec<u64>> = JqValue::from_owned(owned);
        assert_eq!(via_write.to_json_string(), "1e100");
    }

    #[test]
    fn test_jqvalue_number_literal_as_i64_and_as_f64() {
        // Builtins like skip(n; ...)/nth(n; ...)/parent(n) read a numeric
        // argument through as_i64()/as_f64() (or an equivalent match on
        // Int/Float); a NumberLiteral-typed argument (the common case once a
        // document number flows through) must resolve the same as a plain
        // Int/Float, not silently read as "not a number".
        let int_lit: JqValue<'_, Vec<u64>> =
            JqValue::from_owned(OwnedValue::from_number_literal("2"));
        assert_eq!(int_lit.as_i64(), Some(2));
        assert_eq!(int_lit.as_f64(), Some(2.0));

        let float_lit: JqValue<'_, Vec<u64>> =
            JqValue::from_owned(OwnedValue::from_number_literal("1.5"));
        assert_eq!(float_lit.as_i64(), None);
        assert_eq!(float_lit.as_f64(), Some(1.5));

        // An integral-valued Float repr (e.g. from "2.0") still converts to
        // i64, same as JqValue::Float's own integral-value branch above.
        let integral_float_lit: JqValue<'_, Vec<u64>> =
            JqValue::from_owned(OwnedValue::from_number_literal("2.0"));
        assert_eq!(integral_float_lit.as_i64(), Some(2));
    }

    #[test]
    fn test_jqvalue_raw_number_into_owned() {
        let raw: JqValue<'_, Vec<u64>> = JqValue::RawNumber(b"4e4");
        assert_eq!(raw.into_owned().unwrap().to_json(), "4E+4");
    }

    #[test]
    fn test_jqvalue_raw_number_materialize() {
        let raw: JqValue<'_, Vec<u64>> = JqValue::RawNumber(b"4e4");
        assert_eq!(raw.materialize().unwrap().to_json(), "4E+4");
    }

    #[test]
    fn test_jqvalue_lazy_index_range_type_name_and_length() {
        let empty: JqValue<'_, Vec<u64>> = JqValue::LazyIndexRange(0);
        assert_eq!(empty.type_name(), "array");
        assert_eq!(empty.length(), Some(0));

        let three: JqValue<'_, Vec<u64>> = JqValue::LazyIndexRange(3);
        assert_eq!(three.type_name(), "array");
        assert_eq!(three.length(), Some(3));
    }

    #[test]
    fn test_jqvalue_lazy_index_range_materialize_and_into_owned() {
        let empty: JqValue<'_, Vec<u64>> = JqValue::LazyIndexRange(0);
        assert_eq!(empty.materialize().unwrap(), OwnedValue::Array(vec![]));

        let three: JqValue<'_, Vec<u64>> = JqValue::LazyIndexRange(3);
        assert_eq!(
            three.materialize().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::Int(0),
                OwnedValue::Int(1),
                OwnedValue::Int(2)
            ])
        );

        let three: JqValue<'_, Vec<u64>> = JqValue::LazyIndexRange(3);
        assert_eq!(
            three.into_owned().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::Int(0),
                OwnedValue::Int(1),
                OwnedValue::Int(2)
            ])
        );
    }

    #[test]
    fn test_jqvalue_lazy_index_range_write_json() {
        let empty: JqValue<'_, Vec<u64>> = JqValue::LazyIndexRange(0);
        assert_eq!(empty.to_json_string(), "[]");

        let three: JqValue<'_, Vec<u64>> = JqValue::LazyIndexRange(3);
        assert_eq!(three.to_json_string(), "[0,1,2]");
    }

    #[test]
    fn test_jqvalue_cursor_number_materialize_and_into_owned() {
        use crate::json::JsonIndex;

        // Materializing/consuming a cursor over a number (as opposed to
        // reading its raw bytes, which takes a separate fast path in
        // `write_json`) goes through `cursor_to_owned`, which must also
        // preserve the source literal rather than round-tripping through
        // `f64`.
        let json = br"1e100";

        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let materialize_val: JqValue<'_, Vec<u64>> = JqValue::from_cursor(cursor);
        assert_eq!(materialize_val.materialize().unwrap().to_json(), "1E+100");

        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let into_owned_val: JqValue<'_, Vec<u64>> = JqValue::from_cursor(cursor);
        assert_eq!(into_owned_val.into_owned().unwrap().to_json(), "1E+100");
    }

    /// Sibling of the number test above, for `cursor_to_owned_at_depth`'s
    /// `StandardJson::String` arm -- the ordinary successfully-decoding
    /// path, alongside `test_materialize_degrades_to_empty_string_on_decode_failure_1098`'s
    /// coverage of the same arm's `Err` side.
    #[test]
    fn test_jqvalue_cursor_string_materialize_and_into_owned() {
        use crate::json::JsonIndex;

        let json = br#""hello""#;

        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let materialize_val: JqValue<'_, Vec<u64>> = JqValue::from_cursor(cursor);
        assert_eq!(
            materialize_val.materialize().unwrap(),
            OwnedValue::String("hello".to_string())
        );

        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let into_owned_val: JqValue<'_, Vec<u64>> = JqValue::from_cursor(cursor);
        assert_eq!(
            into_owned_val.into_owned().unwrap(),
            OwnedValue::String("hello".to_string())
        );
    }

    /// `depth` levels of single-element array nesting: `[[[...[null]...]]]`.
    /// Mirrors `value.rs`/`eval.rs`'s own `linear_array_nest` helper (#1005),
    /// built directly out of `JqValue` since `materialize`/`into_owned`
    /// recurse over that type, not `OwnedValue`.
    fn linear_jqvalue_nest(depth: usize) -> JqValue<'static, Vec<u64>> {
        let mut v = JqValue::Null;
        for _ in 0..depth {
            v = JqValue::Array(vec![v]);
        }
        v
    }

    /// #1021: `JqValue::materialize` had no depth guard at all before this
    /// issue -- unlike its `Cursor` arm (already guarded independently via
    /// `cursor_to_owned_at_depth`), the `Array`/`Object` arms recursed
    /// unbounded.
    #[test]
    fn materialize_panics_past_nesting_depth_limit_1021() {
        use crate::jq::value::MAX_VALUE_TREE_DEPTH;

        let under = linear_jqvalue_nest(MAX_VALUE_TREE_DEPTH - 1);
        assert!(matches!(under.materialize().unwrap(), OwnedValue::Array(_)));

        // The `Object` arm is a separate match arm from `Array`'s, with its
        // own recursive `.map()` closure -- exercise it too so both arms are
        // covered, not just the array-nesting shape `linear_jqvalue_nest`
        // builds.
        let nested_object: JqValue<'_, Vec<u64>> = JqValue::Object(IndexMap::from([(
            "a".to_string(),
            JqValue::Array(vec![JqValue::Null]),
        )]));
        assert!(matches!(
            nested_object.materialize().unwrap(),
            OwnedValue::Object(_)
        ));

        let over = linear_jqvalue_nest(MAX_VALUE_TREE_DEPTH);
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| over.materialize().unwrap()));
        assert!(
            result.is_err(),
            "materialize should panic at MAX_VALUE_TREE_DEPTH"
        );
    }

    /// #1021: `JqValue::into_owned` had no depth guard at all before this
    /// issue -- same gap as `materialize`, just on the consuming twin.
    #[test]
    fn into_owned_panics_past_nesting_depth_limit_1021() {
        use crate::jq::value::MAX_VALUE_TREE_DEPTH;

        let under = linear_jqvalue_nest(MAX_VALUE_TREE_DEPTH - 1);
        assert!(matches!(under.into_owned().unwrap(), OwnedValue::Array(_)));

        // Exercise the `Object` arm too -- see `materialize`'s sibling test
        // above for why.
        let nested_object: JqValue<'_, Vec<u64>> = JqValue::Object(IndexMap::from([(
            "a".to_string(),
            JqValue::Array(vec![JqValue::Null]),
        )]));
        assert!(matches!(
            nested_object.into_owned().unwrap(),
            OwnedValue::Object(_)
        ));

        let over = linear_jqvalue_nest(MAX_VALUE_TREE_DEPTH);
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| over.into_owned().unwrap()));
        assert!(
            result.is_err(),
            "into_owned should panic at MAX_VALUE_TREE_DEPTH"
        );
    }

    /// `depth` levels of single-element array nesting: `[[[...[Null]...]]]`.
    /// Mirrors `eval.rs`/`value.rs`'s own identically-shaped `linear_array_nest`
    /// helper -- a separate copy (rather than a shared one) because
    /// `OwnedValue` construction has no cross-module-visible builder to
    /// share, same as `stream.rs`'s own copy of this shape.
    fn linear_owned_nest(depth: usize) -> OwnedValue {
        let mut v = OwnedValue::Null;
        for _ in 0..depth {
            v = OwnedValue::Array(vec![v]);
        }
        v
    }

    /// The `Object`-arm counterpart to [`linear_owned_nest`] -- a
    /// depth-threading bug isolated to an `Object` match arm would pass a
    /// boundary test built only from array nesting (#1025 code review).
    fn linear_owned_object_nest(depth: usize) -> OwnedValue {
        let mut v = OwnedValue::Object(IndexMap::new());
        for _ in 0..depth {
            let mut obj = IndexMap::new();
            obj.insert("k".to_string(), v);
            v = OwnedValue::Object(obj);
        }
        v
    }

    /// #1025: `JqValue::from_owned` had no depth guard at all -- since
    /// `OwnedValue::Array`/`Object` are `pub` and re-exported from
    /// `succinctly::jq`, any library consumer can build a deeply-nested
    /// value with a plain loop and hand it to this constructor.
    #[test]
    fn from_owned_panics_past_nesting_depth_limit_1025() {
        use crate::jq::value::MAX_VALUE_TREE_DEPTH;

        let under = linear_owned_nest(MAX_VALUE_TREE_DEPTH - 1);
        let value: JqValue<'_, Vec<u64>> = JqValue::from_owned(under);
        assert!(matches!(value, JqValue::Array(_)));

        let over = linear_owned_nest(MAX_VALUE_TREE_DEPTH);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            JqValue::<Vec<u64>>::from_owned(over)
        }));
        assert!(
            result.is_err(),
            "from_owned should panic at MAX_VALUE_TREE_DEPTH"
        );

        let under_obj = linear_owned_object_nest(MAX_VALUE_TREE_DEPTH - 1);
        let value: JqValue<'_, Vec<u64>> = JqValue::from_owned(under_obj);
        assert!(matches!(value, JqValue::Object(_)));

        let over_obj = linear_owned_object_nest(MAX_VALUE_TREE_DEPTH);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            JqValue::<Vec<u64>>::from_owned(over_obj)
        }));
        assert!(
            result.is_err(),
            "from_owned should panic at MAX_VALUE_TREE_DEPTH (Object arm)"
        );
    }

    /// The `Object`-arm counterpart to `linear_jqvalue_nest` -- see
    /// [`linear_owned_object_nest`]'s doc comment for why this arm needs
    /// its own boundary coverage, not just `Array`'s.
    fn linear_jqvalue_object_nest(depth: usize) -> JqValue<'static, Vec<u64>> {
        let mut v = JqValue::Object(IndexMap::new());
        for _ in 0..depth {
            let mut obj = IndexMap::new();
            obj.insert("k".to_string(), v);
            v = JqValue::Object(obj);
        }
        v
    }

    /// #1025: `JqValue::write_json` (and its `to_json_string` caller) had
    /// no depth guard at all -- reachable the same way as `materialize`/
    /// `into_owned` above, just on the serialization path instead.
    #[test]
    fn write_json_panics_past_nesting_depth_limit_1025() {
        use crate::jq::value::MAX_VALUE_TREE_DEPTH;

        let under = linear_jqvalue_nest(MAX_VALUE_TREE_DEPTH - 1);
        let mut buf = String::new();
        assert!(under.write_json(&mut buf).is_ok());

        let over = linear_jqvalue_nest(MAX_VALUE_TREE_DEPTH);
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| over.to_json_string()));
        assert!(
            result.is_err(),
            "write_json/to_json_string should panic at MAX_VALUE_TREE_DEPTH"
        );

        let under_obj = linear_jqvalue_object_nest(MAX_VALUE_TREE_DEPTH - 1);
        let mut buf = String::new();
        assert!(under_obj.write_json(&mut buf).is_ok());

        let over_obj = linear_jqvalue_object_nest(MAX_VALUE_TREE_DEPTH);
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| over_obj.to_json_string()));
        assert!(
            result.is_err(),
            "write_json/to_json_string should panic at MAX_VALUE_TREE_DEPTH (Object arm)"
        );
    }

    /// #684/#1247/#1642: `lazy_keys_array_to_owned` is `LazyKeysArray`'s own
    /// materializer -- the escape hatch `materialize`/`into_owned` reach for
    /// `sort_keys`/color output/etc. Exercise it directly, both outcomes: a
    /// clean object hits the `Ok(OwnedValue::Array(keys))` return, and an
    /// object with an undecodable key hits the *same* `Ok` return -- the
    /// key preserved via its raw source span (lossily decoded, since this
    /// key is raw invalid UTF-8 rather than a bad escape) instead of either
    /// raising (#1247's answer) or the field silently vanishing (the fault
    /// #1247 fixed).
    #[test]
    fn test_lazy_keys_array_to_owned_ok_and_preserves_decode_failure_1642() {
        use crate::json::JsonIndex;

        let json: &[u8] = br#"{"b":1,"a":2}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let fields = match cursor.value() {
            StandardJson::Object(fields) => fields,
            other => panic!("expected object, got {other:?}"),
        };
        assert_eq!(
            lazy_keys_array_to_owned(&fields, true).unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::String("b".to_string()),
                OwnedValue::String("a".to_string()),
            ])
        );

        let json: &[u8] = b"{\"\xff\xfe\": 1}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let fields = match cursor.value() {
            StandardJson::Object(fields) => fields,
            other => panic!("expected object, got {other:?}"),
        };
        assert_eq!(
            lazy_keys_array_to_owned(&fields, true).expect("preserved, not raised (#1642)"),
            OwnedValue::Array(vec![OwnedValue::String("\u{FFFD}\u{FFFD}".to_string())])
        );
    }

    /// #1679: the #1194 sibling of the #1642 test above -- a key the
    /// format's grammar never allowed at all (a bare numeric key) has no
    /// name to report and must raise, not silently drop the field. Before
    /// this fix, `sjq -c 'keys_unsorted'` on `{123:1,"a":1}` returned
    /// `["a"]` (one entry short, exit 0) while `keys`/`length` on the same
    /// document disagreed.
    #[test]
    fn test_lazy_keys_array_to_owned_raises_on_non_string_key_1679() {
        use crate::json::JsonIndex;

        let json: &[u8] = b"{123: 1, \"b\": 2}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let fields = match cursor.value() {
            StandardJson::Object(fields) => fields,
            other => panic!("expected object, got {other:?}"),
        };
        let err =
            lazy_keys_array_to_owned(&fields, true).expect_err("a bare numeric key is not JSON");
        assert!(err.message.contains("expected string key"), "{err:?}");
    }

    /// #1679: the other #1194 shape -- an object whose last child has no
    /// sibling to pair as its value. Only [`DistinctKeyCursors::ended_unpaired`]
    /// (checked only once the walk is exhausted) can tell this apart from a
    /// clean object with fewer keys.
    #[test]
    fn test_lazy_keys_array_to_owned_raises_on_unpaired_field_1679() {
        use crate::json::JsonIndex;

        for json in [&b"{invalid}"[..], &b"{\"a\"}"[..]] {
            let index = JsonIndex::build(json);
            let cursor = index.root(json);
            let fields = match cursor.value() {
                StandardJson::Object(fields) => fields,
                other => panic!("expected object, got {other:?}"),
            };
            lazy_keys_array_to_owned(&fields, true).expect_err("an unpaired member is not JSON");
        }
    }

    /// #1679: `write_json_at_depth`'s `LazyKeysArray` arm (`JqValue::write_json`/
    /// `to_json_string`'s own escape hatch, independent of
    /// `lazy_keys_array_to_owned`) had the same silent-corruption shape,
    /// but worse: it wrote a non-string key's *raw, unquoted* bytes
    /// straight into the array, producing invalid JSON (`{123:1}` ->
    /// `[123]`) rather than merely dropping the field. Found by code review
    /// (this PR's own "everywhere" claim didn't hold for this fifth call
    /// site until this test/fix).
    #[test]
    fn test_write_json_lazy_keys_array_raises_on_non_string_key_1679() {
        use crate::json::JsonIndex;

        let json: &[u8] = b"{123: 1, \"b\": 2}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let fields = match cursor.value() {
            StandardJson::Object(fields) => fields,
            other => panic!("expected object, got {other:?}"),
        };
        let val: JqValue<'_, Vec<u64>> = JqValue::LazyKeysArray {
            fields,
            collapse: true,
        };
        let mut out = String::new();
        assert!(
            val.write_json(&mut out).is_err(),
            "a bare numeric key must not be written verbatim as invalid JSON: {out:?}"
        );
    }

    /// #1679: the unpaired-tail sibling of the test above.
    #[test]
    fn test_write_json_lazy_keys_array_raises_on_unpaired_field_1679() {
        use crate::json::JsonIndex;

        for json in [&b"{invalid}"[..], &b"{\"a\"}"[..]] {
            let index = JsonIndex::build(json);
            let cursor = index.root(json);
            let fields = match cursor.value() {
                StandardJson::Object(fields) => fields,
                other => panic!("expected object, got {other:?}"),
            };
            let val: JqValue<'_, Vec<u64>> = JqValue::LazyKeysArray {
                fields,
                collapse: true,
            };
            let mut out = String::new();
            assert!(
                val.write_json(&mut out).is_err(),
                "an unpaired member must not silently close the array: {out:?}"
            );
        }
    }

    /// #1679: `cursor_to_owned_at_depth`'s `Object` arm had the identical
    /// silent-drop shape as `lazy_keys_array_to_owned` above, reached via
    /// `materialize`/`into_owned`'s `JqValue::Cursor` arm (`--sort-keys`/
    /// `-C` forcing a full materialize).
    #[test]
    fn test_cursor_to_owned_object_raises_on_non_string_key_1679() {
        use crate::json::JsonIndex;

        let json: &[u8] = b"{123: 1, \"b\": 2}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let val = JqValue::from_cursor(cursor);
        let err = val
            .materialize()
            .expect_err("a bare numeric key is not JSON");
        assert!(err.message.contains("expected string key"), "{err:?}");

        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let val: JqValue<'_, Vec<u64>> = JqValue::from_cursor(cursor);
        let err = val
            .into_owned()
            .expect_err("a bare numeric key is not JSON");
        assert!(err.message.contains("expected string key"), "{err:?}");
    }

    /// #1679: the unpaired-tail sibling of the test above.
    #[test]
    fn test_cursor_to_owned_object_raises_on_unpaired_field_1679() {
        use crate::json::JsonIndex;

        for json in [&b"{invalid}"[..], &b"{\"a\"}"[..]] {
            let index = JsonIndex::build(json);
            let cursor = index.root(json);
            let val = JqValue::from_cursor(cursor);
            val.materialize()
                .expect_err("an unpaired member is not JSON");

            let index = JsonIndex::build(json);
            let cursor = index.root(json);
            let val: JqValue<'_, Vec<u64>> = JqValue::from_cursor(cursor);
            val.into_owned()
                .expect_err("an unpaired member is not JSON");
        }
    }

    /// #1247/#1642: the same `LazyKeysArray` arms as above, reached this
    /// time through `materialize`/`into_owned`'s recursive `Array`/`Object`
    /// cases at depth > 0 -- e.g. `[keys_unsorted]`/`{x: keys_unsorted}`
    /// forced to materialize by `--sort-keys`/`-C` at the CLI boundary
    /// (`write_output_jq_value`'s "complex output" branch in
    /// `jq_runner.rs`), rather than `lazy_keys_array_to_owned` being called
    /// on a bare top-level `keys_unsorted` result.
    #[test]
    fn test_lazy_keys_array_nested_materialize_and_into_owned_1642() {
        use crate::json::JsonIndex;

        let json: &[u8] = b"{\"\xff\xfe\": 1}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let fields = match cursor.value() {
            StandardJson::Object(fields) => fields,
            other => panic!("expected object, got {other:?}"),
        };
        let lazy_keys: JqValue<'_, Vec<u64>> = JqValue::LazyKeysArray {
            fields,
            collapse: true,
        };

        let nested_in_array = JqValue::Array(vec![lazy_keys.clone()]);
        assert_eq!(
            nested_in_array
                .materialize()
                .expect("a nested undecodable key is preserved, not raised on (#1642)"),
            OwnedValue::Array(vec![OwnedValue::Array(vec![OwnedValue::String(
                "\u{FFFD}\u{FFFD}".to_string()
            )])])
        );

        let nested_in_object: JqValue<'_, Vec<u64>> =
            JqValue::Object(IndexMap::from([("x".to_string(), lazy_keys)]));
        let owned = nested_in_object
            .into_owned()
            .expect("a nested undecodable key is preserved, not raised on (#1642)");
        assert_eq!(
            owned,
            OwnedValue::Object(IndexMap::from([(
                "x".to_string(),
                OwnedValue::Array(vec![OwnedValue::String("\u{FFFD}\u{FFFD}".to_string())])
            )]))
        );
    }

    /// #1956: `lazy_keys_array_to_owned` (backing `materialize`/`into_owned`)
    /// and `write_json`'s own `LazyKeysArray` arm both used to check only
    /// `cursors.ended_unpaired()`, missing the `delimiter_fault()` half of
    /// the #1194/#1677 check that `eval_generic.rs`'s
    /// `distinct_key_cursors_checked`/`keys_are_well_formed` already get
    /// right. A missing `,`/`:` delimiter (not a non-string key, not an
    /// unpaired tail) used to silently succeed here.
    #[test]
    fn test_lazy_keys_array_raises_on_missing_delimiter_1956() {
        use crate::json::JsonIndex;

        let json: &[u8] = b"{\"a\" 1, \"b\": 2}";

        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let fields = match cursor.value() {
            StandardJson::Object(fields) => fields,
            other => panic!("expected object, got {other:?}"),
        };
        let val: JqValue<'_, Vec<u64>> = JqValue::LazyKeysArray {
            fields,
            collapse: true,
        };

        let mut out = String::new();
        assert!(
            val.write_json(&mut out).is_err(),
            "a missing delimiter must not be written past silently: {out:?}"
        );
        val.materialize()
            .expect_err("a missing delimiter is not well-formed JSON");
        val.into_owned()
            .expect_err("a missing delimiter is not well-formed JSON");
    }

    /// #1974: `length()`'s `LazyKeysArray` arm calls `effective_len`, the
    /// infallible sibling of `effective_len_checked` -- it computes the
    /// same census internally but discards the malformed flag, so it
    /// silently answers a count instead of surfacing the #1194/#1677 fault
    /// `write_json`/`materialize`/`into_owned` on the identical value
    /// already catch (per #1956, pinned by
    /// [`test_lazy_keys_array_raises_on_missing_delimiter_1956`] above).
    /// `length()` itself stays infallible (see its new sibling's doc
    /// comment); `length_checked()` is the fix -- same fault, reported.
    #[test]
    fn test_lazy_keys_array_length_checked_raises_on_missing_delimiter_1974() {
        use crate::json::JsonIndex;

        let json: &[u8] = b"{\"a\" 1, \"b\": 2}";

        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let fields = match cursor.value() {
            StandardJson::Object(fields) => fields,
            other => panic!("expected object, got {other:?}"),
        };
        let val: JqValue<'_, Vec<u64>> = JqValue::LazyKeysArray {
            fields,
            collapse: true,
        };

        assert_eq!(
            val.length(),
            Some(2),
            "documents the known gap: length() can't see the malformed member"
        );
        val.length_checked()
            .expect_err("a missing delimiter is not well-formed JSON");
    }

    /// #1956: `cursor_to_owned_at_depth`'s `StandardJson::Object` arm (backing
    /// `JqValue::Cursor`'s own `materialize`/`into_owned`) walked `uncons()`
    /// and checked only `ends_unpaired()`, never `key_delimiter_ok`/
    /// `value_delimiter_ok` -- unlike its `eval_generic::to_owned_at_depth`
    /// sibling, which checks both. An even member count with a missing `:`
    /// used to materialize cleanly here.
    #[test]
    fn test_cursor_to_owned_raises_on_missing_delimiter_1956() {
        use crate::json::JsonIndex;

        let json: &[u8] = b"{\"a\" 1, \"b\": 2}";

        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let val: JqValue<'_, Vec<u64>> = JqValue::from_cursor(cursor);
        val.materialize()
            .expect_err("a missing delimiter is not well-formed JSON");

        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let val: JqValue<'_, Vec<u64>> = JqValue::from_cursor(cursor);
        val.into_owned()
            .expect_err("a missing delimiter is not well-formed JSON");
    }

    /// #1194: sibling of `jq_runner.rs`'s `standard_json_to_jq_value` and its
    /// own `test_standard_json_to_jq_value_raises_on_malformed_top_level_value_1194`
    /// -- a bareword garbage token (`StandardJson::Error`, not a decode
    /// failure) raises through `cursor_to_owned_at_depth`'s own `Error` arm
    /// too, reached whenever `materialize`/`into_owned` walks a cursor whose
    /// value is structurally malformed (e.g. `--sort-keys`/`-C` forcing a
    /// full materialize of a query result that otherwise streams straight
    /// from a cursor over raw document bytes).
    #[test]
    fn test_materialize_and_into_owned_error_on_malformed_top_level_value_1194() {
        use crate::json::JsonIndex;

        let json: &[u8] = b"xyz123";

        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let val = JqValue::from_cursor(cursor);
        let err = val
            .materialize()
            .expect_err("a bareword is not a JSON value");
        assert!(!err.message.is_empty(), "{err:?}");

        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let val: JqValue<'_, Vec<u64>> = JqValue::from_cursor(cursor);
        let err = val
            .into_owned()
            .expect_err("a bareword is not a JSON value");
        assert!(!err.message.is_empty(), "{err:?}");
    }
}
