//! Generic traits for document navigation.
//!
//! These traits abstract over JSON and YAML cursor-based navigation,
//! allowing the jq evaluator to work with either format without
//! intermediate conversion.

#[cfg(not(test))]
use alloc::{borrow::Cow, string::String, vec::Vec};

use indexmap::{IndexMap, IndexSet};
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

    /// Get the name of the anchor this node is an *alias* to, if it is one
    /// (`*name` yields `Some("name")`, without the `*`).
    ///
    /// The dual of [`anchor`](Self::anchor): a node is at most one of the
    /// two, and both return `None` for formats without an alias concept
    /// (JSON). Only YAML cursors override this. Needed by the write path
    /// (issue #763) — unlike the value accessors, which resolve *through*
    /// an alias to its target, re-emitting `*name` requires knowing the
    /// node is an alias before that resolution happens.
    fn alias(&self) -> Option<&str> {
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
    /// is repeated -- both formats keep the *last* duplicate key here
    /// (YAML: #174; JSON: #1251, matching real jq/RFC 8259), so callers can
    /// switch between the two without a behavior change. This is the
    /// opposite of YAML's own genuine-duplicates *preservation* elsewhere
    /// (`to_entries`'s cursor-native walk, #443) -- `find`/`find_cursor`
    /// answer "what does this key resolve to", `to_entries` answers "what
    /// are all the entries", and only YAML keeps every entry distinct for
    /// the latter question.
    fn find_cursor(&self, name: &str) -> Option<Self::Cursor>;

    /// Check if there are no fields.
    fn is_empty(&self) -> bool;

    /// Walk every field, keeping every occurrence of a repeated key in
    /// document order.
    ///
    /// Callers that must honour the evaluation mode's duplicate-key rule
    /// should use the free [`effective_fields`] instead; this is the raw
    /// walk it builds on.
    fn all_fields(&self) -> Vec<DocumentField<Self::Value, Self::Cursor>> {
        let mut out = Vec::new();
        let mut fields = self.clone();
        while let Some((field, rest)) = fields.uncons() {
            out.push(field);
            fields = rest;
        }
        out
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

/// A 64-bit fingerprint of a key, for grouping candidates cheaply.
///
/// FNV-1a: no dependency, no `HashMap` (this module is `no_std` + `alloc`),
/// and deterministic across runs. Fingerprint quality only affects *speed*
/// -- every routine below resolves a shared fingerprint by comparing the
/// keys themselves, so a collision costs work, never a wrong answer.
fn key_fingerprint(key: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in key.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The sorted, deduplicated fingerprints that occur more than once in
/// `sorted`, plus how many distinct fingerprint values it holds in total.
///
/// `sorted` must already be sorted ascending; the returned list is too, so
/// callers can `binary_search` it.
fn shared_fingerprints(sorted: &[u64]) -> (Vec<u64>, usize) {
    let mut shared = Vec::new();
    let mut distinct = 0usize;
    let mut i = 0usize;
    while i < sorted.len() {
        let mut j = i + 1;
        while j < sorted.len() && sorted[j] == sorted[i] {
            j += 1;
        }
        distinct += 1;
        if j - i > 1 {
            shared.push(sorted[i]);
        }
        i = j;
    }
    (shared, distinct)
}

/// Number of distinct values in an already-sorted slice, and whether any
/// value occurs more than once.
fn distinct_sorted(sorted: &[String]) -> (usize, bool) {
    let mut distinct = 0usize;
    let mut repeated = false;
    let mut i = 0usize;
    while i < sorted.len() {
        let mut j = i + 1;
        while j < sorted.len() && sorted[j] == sorted[i] {
            j += 1;
        }
        distinct += 1;
        repeated |= j - i > 1;
        i = j;
    }
    (distinct, repeated)
}

/// What an object's keys look like under the collapse rule, measured
/// without materializing the field list.
///
/// The census walks the cons-list keeping **8 bytes per field** -- one
/// fingerprint -- where materializing keeps a four-cursor `DocumentField`
/// (152 bytes for JSON). On a 1M-field object that is the difference
/// between ~8 MB and ~200 MB, which matters because `length` and the
/// `LazyKeys` probe both only ever wanted the *answer*, never the fields.
struct KeyCensus {
    /// Distinct stringifiable keys.
    distinct: usize,
    /// Fields whose key does not stringify at all (a YAML alias or complex
    /// key). Each is its own field: with no name to collapse *on*, it can
    /// neither absorb another field nor be absorbed.
    unkeyed: usize,
    /// Whether any key occurred more than once.
    repeated: bool,
}

impl KeyCensus {
    /// The field count the object presents once repeated keys collapse.
    fn effective_len(&self) -> usize {
        self.distinct + self.unkeyed
    }
}

/// Take the census of `fields` (see [`KeyCensus`]).
fn census<F: DocumentFields>(fields: &F) -> KeyCensus {
    let mut prints: Vec<u64> = Vec::new();
    let mut unkeyed = 0usize;
    let mut walk = fields.clone();
    while let Some((field, rest)) = walk.uncons() {
        match field.key_str() {
            Some(key) => prints.push(key_fingerprint(&key)),
            None => unkeyed += 1,
        }
        walk = rest;
    }
    let keyed = prints.len();
    prints.sort_unstable();
    let (shared, distinct_prints) = shared_fingerprints(&prints);
    if shared.is_empty() {
        return KeyCensus {
            distinct: keyed,
            unkeyed,
            repeated: false,
        };
    }

    // A shared fingerprint is nearly always a genuine repeat, but two
    // different keys can collide, and counting those as one would be
    // wrong. Re-walk owning *only* the colliding keys -- on an ordinary
    // duplicate that is a handful of strings, not one per field.
    let mut colliding: Vec<String> = Vec::new();
    let mut walk = fields.clone();
    while let Some((field, rest)) = walk.uncons() {
        if let Some(key) = field.key_str() {
            if shared.binary_search(&key_fingerprint(&key)).is_ok() {
                colliding.push(key.into_owned());
            }
        }
        walk = rest;
    }
    colliding.sort_unstable();
    let (distinct_colliding, repeated) = distinct_sorted(&colliding);
    KeyCensus {
        distinct: distinct_prints - shared.len() + distinct_colliding,
        unkeyed,
        repeated,
    }
}

/// Whether any key occurs more than once among already-walked fields.
///
/// The slice counterpart of [`census`], for callers that have had to
/// materialize the fields anyway. Same fingerprint-then-verify shape, so it
/// is linear in the field count and keeps 8 bytes per field rather than a
/// borrowed-key `Cow` (32).
///
/// A field whose key does not stringify (`key_str` answers `None`) never
/// compares equal to anything, including another such field.
fn keys_repeat<V: DocumentValue, C: DocumentCursor>(fields: &[DocumentField<V, C>]) -> bool {
    if fields.len() < 2 {
        return false;
    }
    let mut prints: Vec<(u64, usize)> = Vec::with_capacity(fields.len());
    for (i, field) in fields.iter().enumerate() {
        if let Some(key) = field.key_str() {
            prints.push((key_fingerprint(&key), i));
        }
    }
    prints.sort_unstable();
    prints.windows(2).any(|w| {
        w[0].0 == w[1].0 && {
            // Same fingerprint: decide it on the keys themselves.
            let (a, b) = (fields[w[0].1].key_str(), fields[w[1].1].key_str());
            match (a, b) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            }
        }
    })
}

/// Apply the evaluation mode's duplicate-key rule to an object's fields.
///
/// When `collapse` is false (yq, `--preserve-input`'s *output*) every
/// occurrence is kept, in document order. When true (jq) a repeated key
/// collapses to its *first* position holding its *last* value -- see
/// `EvalSemantics::COLLAPSE_DUPLICATE_KEYS` for the reference behaviour,
/// and #1385 for why the rule sits on the mode axis rather than on this
/// per-format trait, where it lived until ADR-0018 rule 2.
///
/// The rebuild runs only when a key actually repeats. That matters: the
/// unconditional `IndexMap<String, _>` this replaced cost ~896 ns/field
/// against a ~54 ns/field bare walk, because it allocated a `String` per
/// key and stored a four-cursor `DocumentField` per entry whether or not
/// the object had any duplicate at all.
pub fn effective_fields<F: DocumentFields>(
    fields: &F,
    collapse: bool,
) -> Vec<DocumentField<F::Value, F::Cursor>> {
    if !collapse {
        return fields.all_fields();
    }
    let all = fields.all_fields();
    if keys_repeat(&all) {
        collapse_repeated(all)
    } else {
        all
    }
}

/// The collapsed field list, or `None` when no key repeats.
///
/// Lets a caller keep whatever zero-allocation fast path it already has for
/// the clean case -- the `LazyKeys` cons-list arms do exactly that -- and
/// pay for a `Vec` only on an object that actually carries a duplicate.
/// The probe itself goes through `census`, so answering `None` costs 8
/// bytes per field rather than a materialized field list.
pub fn collapsed_fields<F: DocumentFields>(
    fields: &F,
) -> Option<Vec<DocumentField<F::Value, F::Cursor>>> {
    if !census(fields).repeated {
        return None;
    }
    Some(collapse_repeated(fields.all_fields()))
}

/// Collapse fields known to contain at least one repeated key.
///
/// Linear in the field count: the surviving slot for each key is looked up
/// through an `IndexMap`, never by scanning the keys accepted so far. A
/// scan would be quadratic, and a single duplicate in a wide object is a
/// perfectly ordinary input -- 100K keys took 8.5 s that way against
/// 0.02 s for not collapsing at all.
fn collapse_repeated<V: DocumentValue, C: DocumentCursor>(
    all: Vec<DocumentField<V, C>>,
) -> Vec<DocumentField<V, C>> {
    // "First position, last value": the slot is claimed by the first
    // occurrence and overwritten by every later one.
    let mut slot_of: IndexMap<String, usize> = IndexMap::new();
    let mut out: Vec<DocumentField<V, C>> = Vec::with_capacity(all.len());
    for field in all {
        let key = field.key_str().map(Cow::into_owned);
        match key {
            Some(key) => match slot_of.get(&key) {
                Some(&slot) => out[slot] = field,
                None => {
                    slot_of.insert(key, out.len());
                    out.push(field);
                }
            },
            // A key that does not stringify has no name to collapse on, so
            // it is kept where it stands rather than dropped. Dropping it
            // silently deleted the field from output (#1385 review).
            None => out.push(field),
        }
    }
    out
}

/// The field count an object presents under the mode's duplicate-key rule.
///
/// `collapse` false is the plain field count; true counts distinct keys --
/// via `census`, so it never materializes the field list.
pub fn effective_len<F: DocumentFields>(fields: &F, collapse: bool) -> usize {
    if !collapse {
        return fields.len();
    }
    census(fields).effective_len()
}

/// An object's field names under the mode's duplicate-key rule, in
/// document order (first position of each key).
///
/// `IndexSet::insert` keeps the first occurrence's position and discards
/// later equal ones -- which is the whole rule, since a key array carries
/// no values for "last value wins" to choose between.
pub fn effective_keys<F: DocumentFields>(fields: &F, collapse: bool) -> Vec<String> {
    let keys = fields.keys();
    if !collapse {
        return keys;
    }
    let mut seen: IndexSet<String> = IndexSet::with_capacity(keys.len());
    for key in keys {
        seen.insert(key);
    }
    seen.into_iter().collect()
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
