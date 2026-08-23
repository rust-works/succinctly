//! Generic traits for document navigation.
//!
//! These traits abstract over JSON and YAML cursor-based navigation,
//! allowing the jq evaluator to work with either format without
//! intermediate conversion.

#[cfg(not(test))]
use alloc::vec;
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

    /// Whether this cursor's document is JSON-sourced and should canonicalize
    /// number literals rather than preserve their source spelling (#978,
    /// #1398).
    ///
    /// Returns `false` by default (preserve spelling, #918's YAML behavior
    /// and JSON-cursor evaluation's own long-standing behavior alike). Only
    /// a `YamlCursor` whose index was
    /// [`mark_json_sourced`](crate::yaml::YamlIndex::mark_json_sourced)-
    /// tagged overrides this — real yq's own JSON-input convention is a
    /// JSON-sourced number never keeps its own spelling, computed or not, a
    /// convention YAML's own `!!float`/exponent literal preservation must
    /// not inherit just because JSON parses through the same cursor type.
    fn canonicalize_numbers(&self) -> bool {
        false
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

    /// Whether this cursor type implements the two `stream_sequence_*`
    /// methods below (#757).
    ///
    /// An *advance* capability probe rather than letting the callers discover
    /// it from an `Err(fmt::Error)` return: the fallback for an unsupported
    /// cursor is to materialize an `OwnedValue::Array` and stream that
    /// instead, and a caller can only switch to it safely while `out` is
    /// still untouched. `stream_json`/`stream_yaml` above get away with
    /// signalling "unsupported" through their return value because every
    /// caller of theirs gates on the output flags in advance
    /// (`can_use_m2_streaming` and friends); a `LazySeq`'s element shapes
    /// aren't knowable from the flags, so this one needs a real probe.
    fn supports_sequence_streaming() -> bool {
        false
    }

    /// Stream `cursors` as a single JSON array, one element per cursor,
    /// without materializing an `OwnedValue` for any of them (#757).
    ///
    /// Unlike [`stream_json`](Self::stream_json), the cursors need not be
    /// siblings — or even share one index — so this is what renders a `map`
    /// chain's drained output (`LazySeq::drain_atomic`), where each element is
    /// wherever in the source document its own sub-expression navigated to.
    ///
    /// Only called when [`supports_sequence_streaming`](Self::supports_sequence_streaming)
    /// answers `true`; the default is the same "not supported" signal
    /// `stream_json` uses.
    fn stream_sequence_json<W: core::fmt::Write>(
        _cursors: &[Self],
        _out: &mut W,
        _indent: IndentSpec,
        _sort_keys: bool,
    ) -> core::fmt::Result {
        Err(core::fmt::Error)
    }

    /// The YAML counterpart of
    /// [`stream_sequence_json`](Self::stream_sequence_json) (#757), rendering
    /// `cursors` as one block- or flow-style sequence.
    fn stream_sequence_yaml<W: core::fmt::Write>(
        _cursors: &[Self],
        _out: &mut W,
        _indent: IndentSpec,
        _sort_keys: bool,
    ) -> core::fmt::Result {
        Err(core::fmt::Error)
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

    /// This value's raw source bytes when it is a string key whose span
    /// needs no decoding -- byte-identical to what
    /// [`key_string`](Self::key_string) would return.
    ///
    /// Lets the duplicate-key probe hash a key without decoding it (#1514);
    /// see `ascii_key_hash` for why that substitution is sound. Defaults to
    /// `None`, which sends the caller to `key_string`. JSON overrides it for
    /// an escape-free string span; YAML does not -- yq keeps every
    /// occurrence, so no probe runs there at all.
    fn key_raw_unescaped(&self) -> Option<&[u8]> {
        None
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

    /// Walk one field, materializing only its key.
    ///
    /// [`uncons`](Self::uncons) builds a whole [`DocumentField`], which
    /// means constructing the field's *value* as well -- wasted for a
    /// caller that only ever looks at keys, and not cheap. Routing the
    /// `keys_unsorted` writer through `uncons` rather than a key-only walk
    /// measured **+89% at 10 MB and +100% at 100 MB** on a wide object
    /// with duplicate detection switched off entirely (#1514) -- it was the
    /// whole of that path's cost, dwarfing the detector it was carrying.
    ///
    /// The default delegates to `uncons`, so a format pays nothing to
    /// ignore it; JSON overrides it.
    fn uncons_key(&self) -> Option<(Self::Value, Self::Cursor, Self)> {
        let (field, rest) = self.uncons()?;
        Some((field.key, field.key_cursor, rest))
    }

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

/// A 64-bit hash of a key, and whether every byte of it was ASCII.
///
/// Mixes eight bytes per multiply rather than one, then runs a splitmix64
/// finalizer so the low bits [`KeyHashes`] indexes on are as well
/// distributed as the high ones. No dependency and no `HashMap` (this
/// module is `no_std` + `alloc`), and deterministic across runs.
///
/// Hash quality only affects *speed* -- every routine below resolves a
/// shared hash by comparing the keys themselves, so a collision costs
/// work, never a wrong answer.
///
/// The ASCII flag rides the same loop: `high` accumulates every word and
/// one test at the end reads its `0x80` bits, so there is no branch per
/// word. [`ascii_key_hash`] is what needs it.
fn key_hash_checked(key: &[u8]) -> (u64, bool) {
    const PRIME: u64 = 0x9e37_79b9_7f4a_7c15;
    const HIGH_BITS: u64 = 0x8080_8080_8080_8080;
    let mut acc = 0xcbf2_9ce4_8422_2325 ^ (key.len() as u64);
    let mut high = 0u64;
    let mut chunks = key.chunks_exact(8);
    for chunk in &mut chunks {
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(chunk);
        let word = u64::from_le_bytes(bytes);
        high |= word;
        acc = (acc ^ word).wrapping_mul(PRIME).rotate_left(31);
    }
    let tail = chunks.remainder();
    if !tail.is_empty() {
        let mut bytes = [0u8; 8];
        bytes[..tail.len()].copy_from_slice(tail);
        let word = u64::from_le_bytes(bytes);
        high |= word;
        acc = (acc ^ word).wrapping_mul(PRIME).rotate_left(31);
    }
    // splitmix64 finalizer: without it the low bits carry too little of
    // the input, and `& mask` would cluster short keys into one run.
    acc ^= acc >> 33;
    acc = acc.wrapping_mul(0xff51_afd7_ed55_8ccd);
    acc ^= acc >> 33;
    acc = acc.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    (acc ^ (acc >> 33), high & HIGH_BITS == 0)
}

/// A 64-bit hash of a key, for grouping candidates cheaply.
pub fn key_hash(key: &[u8]) -> u64 {
    key_hash_checked(key).0
}

/// The hash of a key's *raw source span*, or `None` if the span is not
/// pure ASCII.
///
/// A JSON key whose span carries no escape decodes to its own bytes, so
/// hashing the span directly skips `JsonString::as_str`'s two extra scans
/// (closing quote, then backslash) and its UTF-8 validation, plus the
/// `Cow` around the result. On `wide/10mb` that decode is most of what the
/// probe costs per key once the walk itself is gone (#1514).
///
/// The ASCII test is what makes the substitution *safe*, not just fast.
/// Two guarantees have to hold, and ASCII gives both: an ASCII span is
/// valid UTF-8, so [`DocumentValue::key_string`] would have answered
/// `Some` with these exact bytes -- keeping the keyed/unkeyed split
/// [`census`] counts on identical -- and equal decoded keys still hash
/// equal, because a non-ASCII or escaped key takes the `key_string`
/// fallback and can never decode to the same bytes as an ASCII span
/// without being that span. It is free: [`key_hash_checked`] already
/// computes it.
fn ascii_key_hash(key: &[u8]) -> Option<u64> {
    let (hash, ascii) = key_hash_checked(key);
    ascii.then_some(hash)
}

/// The hash a field's key compares under, or `None` when the key does not
/// stringify at all (a YAML alias or complex key, or a JSON escape that
/// will not decode).
///
/// Prefers the raw span (see [`ascii_key_hash`]) and falls back to the
/// decoded key, which is what every exact resolution downstream uses.
fn key_hash_of<V: DocumentValue>(key: &V) -> Option<u64> {
    if let Some(raw) = key.key_raw_unescaped() {
        if let Some(hash) = ascii_key_hash(raw) {
            return Some(hash);
        }
    }
    key.key_string().map(|key| key_hash(key.as_bytes()))
}

/// [`key_hash_of`] for a caller holding a whole field.
fn field_key_hash<V: DocumentValue, C: DocumentCursor>(field: &DocumentField<V, C>) -> Option<u64> {
    key_hash_of(&field.key)
}

/// An open-addressed set of key hashes: "has any key repeated?" in one
/// pass, no sort, no key text kept.
///
/// This replaces the sort-then-scan shape #1385 shipped (#1514). Sorting
/// is O(n log n) with a comparison per step; on a wide object it dominated
/// the walk it was guarding -- and the printer's variant sorted *byte
/// slices*, chasing a pointer into a random offset of the document on
/// every comparison. Probing costs one hash and about 1.5 slot reads per
/// key, and the slots are one contiguous array.
///
/// The table holds hashes only -- 8 bytes per slot, never a key -- so it
/// stays a fraction of what materializing the fields would cost, and it
/// can grow by rehashing what it already has.
///
/// [`insert`](Self::insert) is deliberately **conservative**: it reports a
/// repeat when two keys merely share a 64-bit hash, which for distinct
/// keys happens about `n^2 / 2^64` of the time. Every caller resolves a
/// reported repeat against the keys themselves, so a false report costs
/// one exact pass and still answers correctly.
#[derive(Clone)]
pub struct KeyHashes {
    /// Open-addressed slots. `0` marks an empty slot, so a key hashing to
    /// zero is stored as `1` -- folding two hash values together, which
    /// the exact resolution above already tolerates.
    ///
    /// Empty until the first insertion, so a caller that constructs one
    /// per object and never uses it -- yq, where `collapse` is false --
    /// allocates nothing.
    slots: Vec<u64>,
    mask: usize,
    len: usize,
}

impl KeyHashes {
    /// Smallest table built, in slots. Below this the pairwise scans the
    /// callers keep for small objects are cheaper anyway.
    const MIN_SLOTS: usize = 16;

    /// An empty table that allocates on its first insertion.
    ///
    /// For callers that cannot count their keys up front, and for the ones
    /// that may never insert at all.
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            mask: 0,
            len: 0,
        }
    }

    /// A table sized for `keys` insertions without a rehash: capacity is
    /// the next power of two at or above `2 * keys`, keeping the load
    /// factor at or below one half.
    pub fn with_capacity(keys: usize) -> Self {
        if keys == 0 {
            return Self::new();
        }
        let slots = keys
            .saturating_mul(2)
            .next_power_of_two()
            .max(Self::MIN_SLOTS);
        Self {
            slots: vec![0; slots],
            mask: slots - 1,
            len: 0,
        }
    }

    /// Record `hash`, answering whether an equal hash was already present.
    ///
    /// `true` means "these keys may be the same" -- see the type's note on
    /// conservatism.
    pub fn insert(&mut self, hash: u64) -> bool {
        // Grow before inserting so the load factor never exceeds one half;
        // past that, linear probing's run lengths climb sharply.
        if (self.len + 1) * 2 > self.slots.len() {
            self.grow();
        }
        let hash = if hash == 0 { 1 } else { hash };
        let mut at = (hash as usize) & self.mask;
        loop {
            match self.slots[at] {
                0 => {
                    self.slots[at] = hash;
                    self.len += 1;
                    return false;
                }
                seen if seen == hash => return true,
                _ => at = (at + 1) & self.mask,
            }
        }
    }

    /// Distinct hashes recorded so far.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether nothing has been recorded yet.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Double the table and re-place what it holds.
    ///
    /// Rehashing needs no keys: the slots *are* the hashes, which is the
    /// whole reason a caller that cannot count its keys up front (the
    /// streaming `keys_unsorted` writer) can still use this.
    fn grow(&mut self) {
        let slots = (self.slots.len() * 2).max(Self::MIN_SLOTS);
        let old = core::mem::replace(&mut self.slots, vec![0; slots]);
        self.mask = slots - 1;
        for hash in old {
            if hash == 0 {
                continue;
            }
            let mut at = (hash as usize) & self.mask;
            while self.slots[at] != 0 {
                at = (at + 1) & self.mask;
            }
            self.slots[at] = hash;
        }
    }
}

impl Default for KeyHashes {
    fn default() -> Self {
        Self::new()
    }
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
///
/// The walk collects hashes into a `Vec` and the table is built afterwards,
/// rather than probing as the walk goes, purely so the table can be sized
/// exactly. A [`KeyHashes`] left to grow into a wide object rehashes
/// everything it holds at every doubling -- roughly as many extra
/// random-access inserts as there are keys, plus zeroing each intermediate
/// table -- and measured *slower* than the sort it replaces: `keys_unsorted`
/// on `wide/10mb` went +110% to +146% against the pre-#1385 baseline instead
/// of improving. The `Vec` was already here before this change, so sizing
/// the table off it costs nothing new.
fn census<F: DocumentFields>(fields: &F) -> KeyCensus {
    let mut hashes: Vec<u64> = Vec::new();
    let mut unkeyed = 0usize;
    let mut walk = fields.clone();
    while let Some((field, rest)) = walk.uncons() {
        match field_key_hash(&field) {
            Some(hash) => hashes.push(hash),
            None => unkeyed += 1,
        }
        walk = rest;
    }
    let mut seen = KeyHashes::with_capacity(hashes.len());
    let mut shared: Vec<u64> = Vec::new();
    for hash in hashes {
        if seen.insert(hash) {
            shared.push(hash);
        }
    }
    if shared.is_empty() {
        return KeyCensus {
            distinct: seen.len(),
            unkeyed,
            repeated: false,
        };
    }
    shared.sort_unstable();
    shared.dedup();

    // A shared hash is nearly always a genuine repeat, but two different
    // keys can collide, and counting those as one would be wrong. Re-walk
    // owning *only* the colliding keys -- on an ordinary duplicate that is
    // a handful of strings, not one per field.
    let mut colliding: Vec<String> = Vec::new();
    let mut walk = fields.clone();
    while let Some((field, rest)) = walk.uncons() {
        // `field_key_hash` answers `Some` only for a key that stringifies,
        // so the inner `key_str` is how the owned spelling is obtained
        // here, not a second filter that could drop a counted field.
        if let Some(hash) = field_key_hash(&field) {
            if shared.binary_search(&hash).is_ok() {
                if let Some(key) = field.key_str() {
                    colliding.push(key.into_owned());
                }
            }
        }
        walk = rest;
    }
    colliding.sort_unstable();
    let (distinct_colliding, repeated) = distinct_sorted(&colliding);
    KeyCensus {
        // `seen.len()` counts distinct *hashes*; every colliding group
        // contributed exactly one of them, so swap each group's single
        // hash for however many distinct keys it actually held.
        distinct: seen.len() - shared.len() + distinct_colliding,
        unkeyed,
        repeated,
    }
}

/// Whether any key *may* occur more than once among already-walked fields.
///
/// The slice counterpart of [`census`], for callers that have had to
/// materialize the fields anyway. One [`KeyHashes`] probe per key, no sort.
///
/// Deliberately conservative, as [`KeyHashes::insert`] is: two distinct
/// keys sharing a 64-bit hash answer `true` here. The only caller,
/// [`effective_fields`], responds by running [`collapse_repeated`], which
/// decides on the keys themselves and returns the same field list when
/// nothing actually repeated -- so a collision costs one rebuild, never a
/// wrong answer.
///
/// A field whose key does not stringify (`key_str` answers `None`) never
/// compares equal to anything, including another such field.
fn keys_repeat<V: DocumentValue, C: DocumentCursor>(fields: &[DocumentField<V, C>]) -> bool {
    if fields.len() < 2 {
        return false;
    }
    let mut seen = KeyHashes::with_capacity(fields.len());
    fields
        .iter()
        .any(|field| field_key_hash(field).is_some_and(|hash| seen.insert(hash)))
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

/// An object's key cursors under the mode's duplicate-key rule, produced
/// one at a time (#1514).
///
/// "First occurrence wins" is an *online* rule -- every key already yielded
/// was a first occurrence -- so a consumer that only moves forward (the
/// `keys_unsorted` writers, `map`'s lazy pull, `.[]`) can stream straight
/// through instead of running the whole-object probe those paths used to
/// run before producing anything. On a wide object that probe was a second
/// full cons-list walk, and `uncons` is two BP sibling hops per field.
///
/// The [`KeyHashes`] probe rides along with the walk. A repeated hash hands
/// the object to [`collapsed_fields`], which decides on the keys
/// themselves:
///
/// - `Some` — a real duplicate. The remainder switches to the exact
///   collapsed list, resuming at the count already yielded, which lines up
///   because the collapsed list opens with those same first occurrences.
/// - `None` — a 64-bit hash collision, and the answer covers the whole
///   object, so the probe retires rather than firing again at the next one.
///
/// `collapse` false (yq) carries no probe state at all and is the plain
/// cons-list walk it always was.
#[derive(Clone)]
pub struct DistinctKeyCursors<F: DocumentFields> {
    /// The fields still to walk.
    rest: F,
    /// The object as a whole, for the exact resolution above. A `F` is a
    /// cursor position, so this is a copy of a couple of machine words.
    all: F,
    /// Hashes of the keys yielded so far, while the rule is in force and
    /// the object is not yet proved clean.
    seen: Option<KeyHashes>,
    /// How many cursors have gone out, which is where `collapsed` resumes.
    yielded: usize,
    /// The exact collapsed fields, once a repeat is confirmed.
    collapsed: Option<Vec<DocumentField<F::Value, F::Cursor>>>,
}

impl<F: DocumentFields> DistinctKeyCursors<F> {
    /// Walk `fields`, collapsing a repeated key onto its first occurrence
    /// when `collapse`.
    pub fn new(fields: &F, collapse: bool) -> Self {
        Self {
            rest: fields.clone(),
            all: fields.clone(),
            seen: collapse.then(KeyHashes::new),
            yielded: 0,
            collapsed: None,
        }
    }
}

impl<F: DocumentFields> Iterator for DistinctKeyCursors<F> {
    /// The key *and* a cursor to it. Both, because every consumer wants
    /// the key itself, and re-deriving it from the cursor materializes a
    /// second time what the walk has already built (#1514).
    type Item = (F::Value, F::Cursor);

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(collapsed) = &self.collapsed {
            let field = collapsed.get(self.yielded)?;
            self.yielded += 1;
            return Some((field.key.clone(), field.key_cursor));
        }
        let (key, key_cursor, tail) = self.rest.uncons_key()?;
        self.rest = tail;
        let repeat = self
            .seen
            .as_mut()
            .is_some_and(|seen| key_hash_of(&key).is_some_and(|hash| seen.insert(hash)));
        if repeat {
            match collapsed_fields(&self.all) {
                Some(fields) => {
                    self.collapsed = Some(fields);
                    self.seen = None;
                    // Resumes at `yielded`, which the collapsed list's own
                    // prefix matches: every key emitted so far was a first
                    // occurrence, and collapsing keeps those in place.
                    return self.next();
                }
                None => self.seen = None,
            }
        }
        self.yielded += 1;
        Some((key, key_cursor))
    }
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
/// Probes with [`KeyHashes`] first and returns the walked keys untouched
/// when nothing repeats, which is the overwhelmingly common case (#1514).
/// Only a document that actually carries a repeat pays for the
/// `IndexSet`, whose `insert` keeps the first occurrence's position and
/// discards later equal ones -- the whole rule, since a key array carries
/// no values for "last value wins" to choose between.
pub fn effective_keys<F: DocumentFields>(fields: &F, collapse: bool) -> Vec<String> {
    let keys = fields.keys();
    if !collapse {
        return keys;
    }
    let mut probe = KeyHashes::with_capacity(keys.len());
    if !keys
        .iter()
        .any(|key| probe.insert(key_hash(key.as_bytes())))
    {
        return keys;
    }
    let mut seen: IndexSet<String> = IndexSet::with_capacity(keys.len());
    for key in keys {
        seen.insert(key);
    }
    seen.into_iter().collect()
}

/// A single field from an object.
#[derive(Clone)]
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

#[cfg(test)]
mod key_hash_tests {
    use super::{key_hash, KeyHashes};

    /// The set answers "already seen" exactly for repeated hashes, and
    /// `len` counts distinct ones — the two properties `census` derives
    /// its whole answer from.
    #[test]
    fn key_hashes_reports_repeats_and_counts_distinct_1514() {
        let mut seen = KeyHashes::with_capacity(4);
        assert!(!seen.insert(key_hash(b"a")));
        assert!(!seen.insert(key_hash(b"b")));
        assert!(seen.insert(key_hash(b"a")), "second `a` is a repeat");
        assert!(!seen.insert(key_hash(b"c")));
        assert_eq!(seen.len(), 3);
    }

    /// A table built with no capacity hint must grow correctly, because
    /// the streaming callers cannot count their keys up front. Growing
    /// rehashes from the stored hashes, so nothing may be lost or
    /// duplicated across the resize.
    #[test]
    fn key_hashes_grows_without_losing_entries_1514() {
        const N: usize = 5_000;
        let mut seen = KeyHashes::default();
        assert!(seen.is_empty());
        for i in 0..N {
            let key = alloc::format!("k{i}");
            assert!(
                !seen.insert(key_hash(key.as_bytes())),
                "{key} is the first of its kind"
            );
        }
        assert_eq!(seen.len(), N, "every distinct key survived the growth");
        for i in 0..N {
            let key = alloc::format!("k{i}");
            assert!(seen.insert(key_hash(key.as_bytes())), "{key} repeats");
        }
        assert_eq!(seen.len(), N, "a repeat adds nothing");
    }

    /// Zero is the empty-slot marker, so a key hashing to zero has to be
    /// folded onto another value rather than read back as "empty" — which
    /// would let it repeat forever undetected.
    #[test]
    fn key_hashes_handles_a_zero_hash_1514() {
        let mut seen = KeyHashes::with_capacity(2);
        assert!(!seen.insert(0));
        assert!(seen.insert(0), "a zero hash must still register as seen");
        assert_eq!(seen.len(), 1);
    }

    /// The hash must depend on length as well as content, or `"ab"` and
    /// `"ab\0"` would collide constantly through the tail path and turn
    /// every wide object into an exact-resolution pass.
    #[test]
    fn key_hash_separates_length_from_content_1514() {
        assert_ne!(key_hash(b"ab"), key_hash(b"ab\0"));
        assert_ne!(key_hash(b""), key_hash(b"\0"));
        assert_eq!(key_hash(b"abcdefghij"), key_hash(b"abcdefghij"));
    }

    /// Low bits carry the index, so a run of keys differing only in their
    /// last byte must not pile into one probe chain. Without the
    /// splitmix64 finalizer this distribution collapses.
    #[test]
    fn key_hash_spreads_the_low_bits_1514() {
        let mut buckets = [0usize; 16];
        for i in 0..4096u32 {
            let key = alloc::format!("field_{i:06}");
            buckets[(key_hash(key.as_bytes()) & 15) as usize] += 1;
        }
        let (lo, hi) = (
            *buckets.iter().min().expect("16 buckets"),
            *buckets.iter().max().expect("16 buckets"),
        );
        // Perfectly uniform is 256 per bucket; allow a wide margin so this
        // pins "not degenerate", not the exact hash function.
        assert!(lo > 150 && hi < 400, "buckets {buckets:?}");
    }
}

#[cfg(test)]
mod raw_key_hash_tests {
    use super::{ascii_key_hash, key_hash};

    /// The raw-span shortcut must agree with the decoded path exactly, or
    /// two spellings of one key would land in different buckets and the
    /// probe would miss a real duplicate.
    #[test]
    fn ascii_key_hash_agrees_with_key_hash_1514() {
        for key in [
            &b""[..],
            b"a",
            b"key",
            b"exactly8",
            b"nine_char",
            b"a rather longer key than one word",
            b"k1234567",
        ] {
            assert_eq!(
                ascii_key_hash(key),
                Some(key_hash(key)),
                "{:?}",
                core::str::from_utf8(key)
            );
        }
    }

    /// A non-ASCII span declines the shortcut, so the caller falls back to
    /// the decoded key. Without this the keyed/unkeyed split `census`
    /// counts on could shift on input a validating parser would reject.
    #[test]
    fn ascii_key_hash_declines_non_ascii_1514() {
        assert_eq!(ascii_key_hash("café".as_bytes()), None);
        assert_eq!(ascii_key_hash("日本".as_bytes()), None);
        assert_eq!(ascii_key_hash(&[b'a', 0x80, b'b']), None);
        // The high byte in the *tail* word, past the 8-byte stride, is the
        // case a chunk-only check would miss.
        assert_eq!(ascii_key_hash("aaaaaaaaé".as_bytes()), None);
    }
}
