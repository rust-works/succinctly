//! Generic traits for document navigation.
//!
//! These traits abstract over JSON and YAML cursor-based navigation,
//! allowing the jq evaluator to work with either format without
//! intermediate conversion.

#[cfg(not(test))]
use alloc::{borrow::Cow, string::String, vec::Vec};

use indexmap::IndexMap;
#[cfg(test)]
use std::borrow::Cow;

/// Indentation configuration for cursor/lazy streaming output.
///
/// `width` is the number of `unit` characters written per nesting level;
/// `width == 0` means compact/flow style (no newlines, no indentation).
/// `unit` is `' '` for ordinary space-indented output and `'\t'` for
/// `--tab` — mirroring `OutputConfig::indent_str` in
/// `src/bin/succinctly/yq_runner.rs`, which the `OwnedValue` DOM path
/// already builds this way (`if args.tab { "\t" } else { "
/// ".repeat(args.indent) }`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndentSpec {
    /// Number of `unit` characters per indentation level.
    pub width: usize,
    /// The character repeated `width` times per level.
    pub unit: char,
}

impl IndentSpec {
    /// Compact/flow style: no indentation, no newlines.
    pub const COMPACT: Self = Self {
        width: 0,
        unit: ' ',
    };

    /// `width` spaces per indentation level.
    pub fn spaces(width: usize) -> Self {
        Self { width, unit: ' ' }
    }

    /// Whether this spec requests compact/flow-style output (no newlines).
    #[inline]
    pub fn is_compact(&self) -> bool {
        self.width == 0
    }
}

/// A cursor for navigating an indexed document.
///
/// Provides tree navigation operations that work in O(1) time
/// using the underlying balanced parentheses structure.
pub trait DocumentCursor: Sized + Copy + Clone {
    /// The value type returned by this cursor.
    type Value: DocumentValue<Cursor = Self>;

    /// Get the value at the current cursor position.
    fn value(&self) -> Self::Value;

    /// Navigate to the first child of a container.
    fn first_child(&self) -> Option<Self>;

    /// Navigate to the next sibling.
    fn next_sibling(&self) -> Option<Self>;

    /// Navigate to the parent container.
    fn parent(&self) -> Option<Self>;

    /// Check if this cursor points to a container (array or object).
    fn is_container(&self) -> bool;

    /// Get the byte position in the source text.
    fn text_position(&self) -> Option<usize>;

    /// Get the 1-based line number of this node's position.
    ///
    /// Returns 0 if position information is not available. "Not available"
    /// specifically means this cursor's evaluator has no source position to
    /// give — either the value was computed (arithmetic, string
    /// interpolation, object/array construction) rather than navigated to,
    /// or evaluation is on the `OwnedValue`/full-evaluator path
    /// (`src/jq/eval.rs`), which never carries a cursor at all. It does
    /// *not* mean position tracking is broken: ordinary navigation
    /// (`.foo`, `.[]`, `select(...)`, chained field/index access) on the
    /// generic cursor evaluator (`src/jq/eval_generic.rs`) forwards the
    /// cursor and resolves a real line (#532).
    fn line(&self) -> usize {
        0
    }

    /// Get the 1-based column number of this node's position.
    ///
    /// See [`line`](Self::line) — same "not available" contract, same
    /// caveats about when it does and doesn't apply.
    fn column(&self) -> usize {
        0
    }

    /// Get the 0-indexed document position in a multi-document stream.
    ///
    /// Returns 0 for single-document files or the first document.
    /// Returns None if document index tracking is not available.
    fn document_index(&self) -> Option<usize> {
        None
    }

    /// Get the YAML anchor name at this node, if any.
    ///
    /// Returns `None` when the node has no anchor, and for formats without
    /// an anchor concept (JSON). Only YAML cursors override this.
    fn anchor(&self) -> Option<&str> {
        None
    }

    /// Get the explicit YAML tag at this node, if any (e.g. `"!!str"`).
    ///
    /// Returns `None` when the node has no explicit tag, and for formats
    /// without a tag concept (JSON). Only YAML cursors override this —
    /// tag lookup is keyed by byte position, which only a cursor carries;
    /// a bare [`DocumentValue`] has already lost it (#747).
    fn explicit_tag(&self) -> Option<&str> {
        None
    }

    /// Get the YAML style indicator for this node (e.g. `"flow"`, `"double"`).
    ///
    /// Returns `""` (no explicit style) by default; only YAML cursors
    /// override this to report block/flow/quote style from the source text.
    fn style(&self) -> &'static str {
        ""
    }

    /// Get this node's trailing same-line comment text, with the leading
    /// `#`/space stripped (the `line_comment` jq builtin, issue #710).
    ///
    /// Returns `None` if this node has no trailing comment, or for formats
    /// without comments at all (e.g. JSON) — the "not available" contract
    /// mirrors [`line`](Self::line)/[`column`](Self::column): the builtin
    /// itself maps `None` to `""`, matching real `yq`.
    ///
    /// Owned rather than borrowed: a trait method's `&self` elides to a
    /// lifetime scoped to the borrow at the call site, which for a `Copy`
    /// cursor obtained from e.g. `Option<V::Cursor>::and_then` can be
    /// shorter than the cursor's own internal `'a` — too short to hand back
    /// a `&str` slice of the source text. Comments are rare enough per
    /// document that the allocation only on actual read is not a concern.
    fn line_comment(&self) -> Option<String> {
        None
    }

    /// Get this node's trailing same-line comment, `#` and all, exactly as
    /// it appears in the source (issue #710) — used by the write path
    /// ([`crate::jq::eval_generic::to_owned_with_comments`]) to re-emit it
    /// verbatim. See [`line_comment`](Self::line_comment) for the stripped
    /// getter form the jq builtin uses; the two intentionally differ (this
    /// one keeps the `#`, that one usually doesn't).
    fn line_comment_raw(&self) -> Option<String> {
        None
    }

    /// Get this node's trailing same-line comment, distinguishing "no
    /// comment" (`Ok(None)`) from "comment present but not valid UTF-8"
    /// (`Err(_)`) — unlike [`line_comment`](Self::line_comment), which
    /// silently collapses both to `None` (issue #797).
    ///
    /// Default `Ok(None)`: formats without a comment concept (JSON) never
    /// have an invalid-UTF-8 comment to report. Only YAML cursors override
    /// this.
    fn line_comment_checked(&self) -> Result<Option<String>, core::str::Utf8Error> {
        Ok(None)
    }

    /// Create a cursor at the specified byte offset (0-indexed).
    ///
    /// Returns None if:
    /// - The offset is out of bounds
    /// - The offset doesn't correspond to a valid node
    /// - Position-based navigation is not supported
    fn cursor_at_offset(&self, _offset: usize) -> Option<Self> {
        None
    }

    /// Create a cursor at the specified line and column (1-indexed).
    ///
    /// Returns None if:
    /// - The position is out of bounds
    /// - The position doesn't correspond to a valid node
    /// - Position-based navigation is not supported
    fn cursor_at_position(&self, _line: usize, _col: usize) -> Option<Self> {
        None
    }

    /// Stream this cursor's value as JSON to the output.
    ///
    /// This enables M2 streaming optimization where navigation query results
    /// can be written directly to output without materializing OwnedValue.
    /// - `indent`: indentation width/unit (`IndentSpec::COMPACT` for compact)
    /// - `sort_keys`: sort mapping/object keys before writing (`-S`/`--sort-keys`)
    ///
    /// Default implementation returns an error indicating streaming is not supported.
    fn stream_json<W: core::fmt::Write>(
        &self,
        _out: &mut W,
        _indent: IndentSpec,
        _sort_keys: bool,
    ) -> core::fmt::Result {
        Err(core::fmt::Error)
    }

    /// Stream this cursor's value as YAML to the output.
    ///
    /// This enables M2.5 streaming optimization for YAML output format.
    /// - `indent`: indentation width/unit (`IndentSpec::COMPACT` forces flow
    ///   style for the whole subtree); a node whose source used flow style
    ///   renders as flow regardless of `indent` (#707).
    /// - `sort_keys`: sort mapping/object keys before writing (`-S`/`--sort-keys`)
    ///
    /// Default implementation returns an error indicating streaming is not supported.
    fn stream_yaml<W: core::fmt::Write>(
        &self,
        _out: &mut W,
        _indent: IndentSpec,
        _sort_keys: bool,
    ) -> core::fmt::Result {
        Err(core::fmt::Error)
    }

    /// Like [`stream_yaml`](Self::stream_yaml), but also appends this
    /// cursor's own trailing comment (#710/#793) when it's a container -
    /// for callers displaying this cursor's value as a complete result in
    /// its own right (a navigated query result, or the whole document), as
    /// opposed to a value nested inside a parent that already appends its
    /// children's comments as it recurses.
    ///
    /// Default: delegates to `stream_yaml` unchanged. Correct for formats
    /// without a comment concept (JSON) and as a fallback for any
    /// `DocumentCursor` impl that doesn't override it; only YAML cursors
    /// need to override this.
    fn stream_yaml_as_document<W: core::fmt::Write>(
        &self,
        out: &mut W,
        indent: IndentSpec,
        sort_keys: bool,
    ) -> core::fmt::Result {
        self.stream_yaml(out, indent, sort_keys)
    }

    /// Check if the value at this cursor is falsy (null or false).
    ///
    /// Used for `--exit-status` flag handling without requiring full materialization.
    /// Default implementation returns false (conservative assumption).
    fn is_falsy(&self) -> bool {
        false
    }
}

/// A value from a document (JSON value or YAML value).
///
/// Provides type inspection and conversion methods.
pub trait DocumentValue: Sized + Clone {
    /// The cursor type that navigates this document.
    type Cursor: DocumentCursor<Value = Self>;

    /// The type for iterating object fields.
    type Fields: DocumentFields<Value = Self, Cursor = Self::Cursor>;

    /// The type for iterating array elements.
    type Elements: DocumentElements<Value = Self, Cursor = Self::Cursor>;

    /// Check if this value is null.
    fn is_null(&self) -> bool;

    /// Try to get as a boolean.
    fn as_bool(&self) -> Option<bool>;

    /// Try to get as an i64.
    fn as_i64(&self) -> Option<i64>;

    /// Try to get as an f64.
    fn as_f64(&self) -> Option<f64>;

    /// The exact source text of this value if it's a number, so a
    /// materializing conversion (`to_owned`) can preserve jq's formatting
    /// (`1e100`, `1.0`, `-0.0`) instead of re-rendering the parsed value —
    /// see issue #387.
    ///
    /// Defaults to `None`. JSON overrides this for a `Number` token whose
    /// raw span is independently confirmed to be valid RFC 8259 number
    /// syntax (`crate::json::validate::is_valid_number`) -- the semi-index
    /// scanner accepts number *spans* more leniently than that grammar
    /// (leading zeros, multiple decimal points), so not every `Number`
    /// token qualifies (#966). YAML overrides it too (#918), but only for a
    /// finite float whose source text is independently confirmed safe and
    /// worthwhile to echo — see `YamlValue::number_literal`'s doc comment
    /// (`src/yaml/light.rs`); YAML's own plain-scalar grammar accepts
    /// spellings (hex/octal, leading-dot) that aren't JSON-legal, and ints
    /// never lose information through the bare-`Int` path, so both stay on
    /// this `None` default.
    fn number_literal(&self) -> Option<Cow<'_, str>> {
        None
    }

    /// Try to get as a string.
    fn as_str(&self) -> Option<Cow<'_, str>>;

    /// String form of this value when it appears as a mapping key.
    ///
    /// Unlike [`as_str`](Self::as_str), a key is always representable as a
    /// string and is never dropped: the default matches `as_str` (JSON object
    /// keys are always strings), but formats with non-string keys (YAML alias
    /// or complex keys) override this to stringify them rather than return
    /// `None`. Returning `None` here causes the entry to be discarded, so
    /// overrides should return `Some("")` instead of `None` for keys that have
    /// no scalar form (issue #222).
    fn key_string(&self) -> Option<Cow<'_, str>> {
        self.as_str()
    }

    /// Try to get as object fields.
    fn as_object(&self) -> Option<Self::Fields>;

    /// Try to get as array elements.
    fn as_array(&self) -> Option<Self::Elements>;

    /// Get the type name for error messages.
    fn type_name(&self) -> &'static str;

    /// Check if this is an error value.
    fn is_error(&self) -> bool;

    /// Get error message if this is an error.
    fn error_message(&self) -> Option<&'static str>;

    // ========== Helper type-checking methods ==========

    /// Check if this value is a boolean.
    #[inline]
    fn is_bool(&self) -> bool {
        self.as_bool().is_some()
    }

    /// Check if this value is a number (integer or float).
    #[inline]
    fn is_number(&self) -> bool {
        self.as_i64().is_some() || self.as_f64().is_some()
    }

    /// Check if this value is a string.
    #[inline]
    fn is_string(&self) -> bool {
        self.as_str().is_some()
    }

    /// Check if this value is an array.
    #[inline]
    fn is_array(&self) -> bool {
        self.as_array().is_some()
    }

    /// Check if this value is an object.
    #[inline]
    fn is_object(&self) -> bool {
        self.as_object().is_some()
    }

    /// Check if this value is iterable (array or object).
    #[inline]
    fn is_iterable(&self) -> bool {
        self.is_array() || self.is_object()
    }
}

/// Iterator-like access to object fields.
///
/// Only `Clone`, not `Copy`: YAML fields backing a merge-resolved mapping
/// (`<<: *anchor`) hold an `Rc`-shared entry list rather than a bare cursor,
/// so cloning is O(1) but not a bitwise copy (see `YamlFields` in
/// `src/yaml/light.rs`).
#[allow(clippy::type_complexity)] // STYLE-0004: uncons returns the cons-list contract (field, rest); the nested tuple is intentional
pub trait DocumentFields: Sized + Clone {
    /// The value type for keys and values.
    type Value: DocumentValue;

    /// The cursor type.
    type Cursor: DocumentCursor;

    /// Get the first field and remaining fields.
    #[allow(clippy::type_complexity)] // STYLE-0004: uncons returns the cons-list contract (field, rest); the nested tuple is intentional
    fn uncons(&self) -> Option<(DocumentField<Self::Value, Self::Cursor>, Self)>;

    /// Find a field by name.
    fn find(&self, name: &str) -> Option<Self::Value>;

    /// Find a field by name and return a cursor to its value.
    ///
    /// Must agree with [`find`](Self::find) on which field wins when a name
    /// is repeated (YAML keeps the last duplicate key; JSON keeps the
    /// first), so callers can switch between the two without a behavior
    /// change — see the per-format `find`/`find_cursor` implementations.
    fn find_cursor(&self, name: &str) -> Option<Self::Cursor>;

    /// Check if there are no fields.
    fn is_empty(&self) -> bool;

    /// Whether a repeated key collapses to one field (`true`) or every
    /// occurrence is kept, distinct (`false`).
    ///
    /// Must agree with [`find`](Self::find)/[`find_cursor`](Self::find_cursor)'s
    /// own per-format contract: JSON dedupes (`true`), YAML doesn't
    /// (`false`) -- duplicate mapping keys are semantically distinct there.
    /// Backs [`effective_fields`](Self::effective_fields) (#1170).
    fn keys_dedup(&self) -> bool;

    /// Walk every field, applying this format's own duplicate-key rule (see
    /// [`keys_dedup`](Self::keys_dedup)): a deduping format collapses a
    /// repeated key to its first position but *last*-seen value (#1170,
    /// matching real jq's own `.foo`/`to_entries` behavior on duplicate
    /// JSON keys); a non-deduping format keeps every occurrence, in
    /// document order.
    ///
    /// The default implementation is the shared, format-agnostic algorithm;
    /// individual formats only need to answer `keys_dedup`, not reimplement
    /// the walk.
    fn effective_fields(&self) -> Vec<DocumentField<Self::Value, Self::Cursor>> {
        let mut fields = self.clone();
        if !self.keys_dedup() {
            let mut out = Vec::new();
            while let Some((field, rest)) = fields.uncons() {
                out.push(field);
                fields = rest;
            }
            return out;
        }
        // `IndexMap::insert` on an existing key retains its original
        // position but replaces the stored value -- exactly "first
        // position, last value", so a plain walk-and-insert already
        // implements the whole rule.
        let mut by_key: IndexMap<String, DocumentField<Self::Value, Self::Cursor>> =
            IndexMap::new();
        while let Some((field, rest)) = fields.uncons() {
            if let Some(key) = field.key_str() {
                by_key.insert(key.into_owned(), field);
            }
            fields = rest;
        }
        by_key.into_values().collect()
    }

    /// Count the number of fields.
    fn len(&self) -> usize {
        let mut count = 0;
        let mut fields = self.clone();
        while let Some((_, rest)) = fields.uncons() {
            count += 1;
            fields = rest;
        }
        count
    }

    /// Collect all field names.
    fn keys(&self) -> Vec<String> {
        let mut keys = Vec::new();
        let mut fields = self.clone();
        while let Some((field, rest)) = fields.uncons() {
            if let Some(key) = field.key_str() {
                keys.push(key.into_owned());
            }
            fields = rest;
        }
        keys
    }
}

/// A single field from an object.
pub struct DocumentField<V, C> {
    /// The field key.
    pub key: V,
    /// The field value.
    pub value: V,
    /// Cursor to the key (for lazy key-array navigation, e.g. `keys_unsorted`).
    pub key_cursor: C,
    /// Cursor to the value (for efficient sub-navigation).
    pub value_cursor: C,
}

impl<V: DocumentValue, C: DocumentCursor> DocumentField<V, C> {
    /// Get the key as a string.
    ///
    /// Uses [`DocumentValue::key_string`] so non-string keys (YAML alias or
    /// complex keys) stringify rather than dropping the entry (issue #222).
    pub fn key_str(&self) -> Option<Cow<'_, str>> {
        self.key.key_string()
    }
}

/// Iterator-like access to array elements.
pub trait DocumentElements: Sized + Copy + Clone {
    /// The value type for elements.
    type Value: DocumentValue;

    /// The cursor type.
    type Cursor: DocumentCursor;

    /// Get the first element and remaining elements.
    fn uncons(&self) -> Option<(Self::Value, Self)>;

    /// Get the first element's cursor and remaining elements.
    fn uncons_cursor(&self) -> Option<(Self::Cursor, Self)>;

    /// Get element by index (0-based).
    fn get(&self, index: usize) -> Option<Self::Value>;

    /// Get element by index (0-based) and return a cursor to it.
    ///
    /// Default: walks [`uncons_cursor`](Self::uncons_cursor) `index` times —
    /// the same sibling-walk `get` itself performs for both YAML and JSON
    /// today, so this costs no more than `get`.
    fn get_cursor(&self, index: usize) -> Option<Self::Cursor> {
        let mut elems = *self;
        for _ in 0..index {
            let (_, rest) = elems.uncons_cursor()?;
            elems = rest;
        }
        elems.uncons_cursor().map(|(cursor, _)| cursor)
    }

    /// Check if there are no elements.
    fn is_empty(&self) -> bool;

    /// Count the number of elements.
    fn len(&self) -> usize {
        let mut count = 0;
        let mut elems = *self;
        while let Some((_, rest)) = elems.uncons() {
            count += 1;
            elems = rest;
        }
        count
    }

    /// Collect all elements into a Vec.
    fn collect_values(&self) -> Vec<Self::Value> {
        let mut values = Vec::new();
        let mut elems = *self;
        while let Some((value, rest)) = elems.uncons() {
            values.push(value);
            elems = rest;
        }
        values
    }

    /// Collect a cursor for every element into a Vec.
    fn collect_cursors(&self) -> Vec<Self::Cursor> {
        let mut cursors = Vec::new();
        let mut elems = *self;
        while let Some((cursor, rest)) = elems.uncons_cursor() {
            cursors.push(cursor);
            elems = rest;
        }
        cursors
    }
}
