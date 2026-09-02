//! Generic traits for document navigation.
//!
//! These traits abstract over JSON and YAML cursor-based navigation,
//! allowing the jq evaluator to work with either format without
//! intermediate conversion.

#[cfg(not(test))]
use alloc::vec;
#[cfg(not(test))]
use alloc::{borrow::Cow, boxed::Box, string::String, vec::Vec};

use indexmap::IndexMap;
#[cfg(test)]
use std::borrow::Cow;

use super::error::EvalError;
use super::stream::{StreamFailure, StreamResult};

/// Indentation configuration for cursor/lazy streaming output.
///
/// `width` is the number of `unit` characters written per nesting level;
/// `width == 0` means compact/flow style (no newlines, no indentation).
/// `unit` is `' '` for ordinary space-indented output and `'\t'` for
/// `--tab`.
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

    /// The `-I`/`--tab` CLI flags' shared YAML-output rule (#1486, #1575,
    /// #1685): `--tab` means exactly one tab per level regardless of
    /// `indent`'s value; otherwise `-I0`/`-I1` both clamp to width 2 --
    /// real yq's YAML output at `-I1` is byte-identical to its own `-I2`
    /// output at every level (live-verified against v4.53.3), and `-I0`
    /// reuses that same clamp rather than modeling real yq's own irregular
    /// `-I0`-behaves-like-`-I4` quirk (out of scope, see
    /// `docs/compliance/yq/limitations.md`). `-I2` and above thread through
    /// unchanged.
    ///
    /// Takes primitive `indent`/`tab` rather than a whole CLI-args struct:
    /// this is a library-crate (`src/jq/`) type, and the parsed args live in
    /// the `succinctly-cli` binary crate (`src/bin/succinctly/yq_runner.rs`),
    /// which cannot be depended on here. Shared by that binary's own
    /// `OutputConfig::compute_indent_str` (DOM output path) and its M2
    /// streaming fast path's indent setup -- previously two independently
    /// hand-encoded copies of this exact rule (#1685), the third recurrence
    /// of the same duplication in that file's history.
    ///
    /// JSON has no such clamp (`-I1 -o=json` genuinely indents 1 space per
    /// level in real yq, and `-I0 -o=json` means compact/flow, handled
    /// separately) -- this constructor is YAML-specific, not a general
    /// `-I`-flag-to-`IndentSpec` conversion.
    pub fn for_yaml(indent: u8, tab: bool) -> Self {
        if tab {
            return Self {
                width: 1,
                unit: '\t',
            };
        }
        Self::spaces((indent as usize).max(2))
    }

    /// Whether this spec requests compact/flow-style output (no newlines).
    #[inline]
    pub fn is_compact(&self) -> bool {
        self.width == 0
    }
}

/// Which value-formatting convention JSON streaming should use (#1576).
///
/// Covers finite number literals, control-character escaping, and
/// duplicate object keys -- the three things `--preserve-input`/
/// `jq_compat` toggles together (ADR-0018 rule 5), so one enum selects all
/// three rather than three independent parameters that are never
/// independently selectable in practice.
///
/// - `Preserve`: echo the document's source number spelling verbatim
///   (`1e100` stays `1e100`), use yq's escape table (no `\b`/`\f` short
///   forms, DEL left raw), and keep every occurrence of a repeated object
///   key -- real yq's own convention (#1008), and the only one
///   `yq_runner.rs` ever selects, since yq has no `jq_compat` concept.
/// - `JqCompat`: canonicalize number literals the way real jq's own reader
///   does (`format_number_jq_compat` -- strips a redundant leading zero,
///   canonicalizes exponent notation), use jq's escape table (`\b`/`\f`
///   short forms, DEL escaped rather than left raw), and collapse a
///   repeated object key to one field (first position, last value, exactly
///   `IndexMap::insert` semantics, matching
///   `EvalSemantics::COLLAPSE_DUPLICATE_KEYS`) -- real jq's default output
///   convention, matching `stream_owned_value_json_jq`'s escaping and
///   `format_number_jq_compat`'s reformatting, both already used by the
///   jq CLI's non-streaming `print_json` path. `jq_runner.rs` selects this
///   unless `--preserve-input`/`SUCCINCTLY_PRESERVE_INPUT=1` is set, in
///   which case it selects `Preserve` instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonConvention {
    Preserve,
    JqCompat,
}

#[cfg(test)]
mod indent_spec_tests {
    use super::IndentSpec;

    /// #1685: `-I0`/`-I1` both clamp to width 2, `-I2` and above thread
    /// through unchanged -- the rule the DOM path's `compute_indent_str`
    /// and the M2 fast path's indent setup previously hand-encoded twice.
    #[test]
    fn for_yaml_clamps_zero_and_one_to_two_1685() {
        for indent in [0u8, 1, 2] {
            assert_eq!(
                IndentSpec::for_yaml(indent, false),
                IndentSpec::spaces(2),
                "indent={indent}"
            );
        }
        assert_eq!(IndentSpec::for_yaml(4, false), IndentSpec::spaces(4));
        assert_eq!(IndentSpec::for_yaml(6, false), IndentSpec::spaces(6));
    }

    /// `--tab` means exactly one tab per level regardless of `-I`'s value,
    /// including the values that would otherwise clamp.
    #[test]
    fn for_yaml_tab_ignores_indent_value_1685() {
        for indent in [0u8, 1, 2, 4] {
            assert_eq!(
                IndentSpec::for_yaml(indent, true),
                IndentSpec {
                    width: 1,
                    unit: '\t'
                },
                "indent={indent}"
            );
        }
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

    /// Whether this node, already known to sit at `text_pos`, is preceded
    /// by the delimiter its position in the document requires: nothing if
    /// `expected` is `None` (a container's first child), otherwise exactly
    /// one byte matching `expected` (`,` before a later array element or
    /// object key, `:` before an object field's value) — #1677, extending
    /// #1643's CLI-only check (`jq_runner.rs`'s `print_json`) into the
    /// evaluator itself, so `.[]`/`length`/`keys`/`add`/`to_entries`/plain
    /// field access all raise on a malformed delimiter too, not just a
    /// filter that re-serializes the container whole.
    ///
    /// `text_pos` is never re-derived here -- it must be a position the
    /// caller already resolved (this cursor's own `text_position()`, or a
    /// value it already decoded, e.g. [`DocumentValue::text_start`]) --
    /// because a second `text_position()` call on a hot walk is a real
    /// cost, not a formality (see that method's own rank/select doc
    /// comment). Mirrors the `known_text_pos` reuse `#1643` already
    /// established for the CLI printer.
    ///
    /// The default answers `true` unconditionally: every format but JSON
    /// validates delimiters while parsing, so this costs them nothing.
    fn preceding_delimiter_ok(&self, text_pos: usize, expected: Option<u8>) -> bool {
        let _ = (text_pos, expected);
        true
    }

    /// [`preceding_delimiter_ok`](Self::preceding_delimiter_ok), resolving
    /// `expected` from `is_first` (`None` for a container's first child,
    /// `Some(b',')` otherwise) and skipping the check entirely when this
    /// cursor has no `text_position()` -- the #1677 array-element gap check,
    /// as one definition shared by every caller that walks elements one at
    /// a time, instead of each re-deriving `expected`/the `None`-position
    /// skip itself (#1597 code review: `DocumentElements::collect_cursors_checked`
    /// below and `eval_generic::each_lazy_array_iterate_sink` had drifted
    /// into two independent copies of this exact check before this
    /// extraction).
    fn element_gap_ok(&self, is_first: bool) -> bool {
        match self.text_position() {
            Some(pos) => {
                let expected = if is_first { None } else { Some(b',') };
                self.preceding_delimiter_ok(pos, expected)
            }
            None => true,
        }
    }

    /// The error to raise when [`preceding_delimiter_ok`](Self::preceding_delimiter_ok)
    /// answers `false`.
    ///
    /// The default is unreachable for every format shipped today, same as
    /// [`DocumentFields::malformed_member_error`] -- it exists to keep a
    /// future format honest rather than to be seen.
    fn malformed_delimiter_error(&self) -> EvalError {
        EvalError::new("Invalid document text: malformed delimiter")
    }

    /// Whether an array/object cursor whose child walk produced **zero**
    /// real children is genuinely empty (`[]`, `{}`) rather than a stray
    /// `,` with no real child at all (`[,]`, `{,}`) -- #2211.
    ///
    /// [`preceding_delimiter_ok`](Self::preceding_delimiter_ok) (#1677) only
    /// ever runs *against a real child* -- it has nothing to check when the
    /// walk yields no children at all, which is exactly the gap
    /// `crate::json::light`'s own `empty_container_gap_ok`/`trailing_gap_ok`
    /// pair closed for the CLI's cursor-native writers (#1676) and this
    /// closes for `crate::jq::eval_generic::to_owned_cursor_at_depth` (a
    /// private function, not linkable here) -- and
    /// therefore every `evaluate_bytes_lazy`-reachable filter that isn't one
    /// of that evaluator's natively-matched shapes, e.g. `if`/arithmetic/
    /// function calls) -- the one materializing conversion that still
    /// silently accepted `[,]` as `[]` because it held a per-child cursor at
    /// every point except the empty case, where there is no child cursor to
    /// hold.
    ///
    /// `close_char` is the container's own closing delimiter (`]` for an
    /// array, `}` for an object); the caller already knows which arm it is
    /// in, so unlike `empty_container_gap_ok` (whose callers don't) there is
    /// no need to derive it from the container's own raw span here.
    ///
    /// The default answers `true` unconditionally, same reasoning as
    /// [`preceding_delimiter_ok`](Self::preceding_delimiter_ok): every format
    /// but JSON validates this while parsing.
    fn container_gap_ok(&self, close_char: u8) -> bool {
        let _ = close_char;
        true
    }

    /// Whether the value immediately following this key (at `key_end`, the
    /// key's own already-known span end) is preceded by exactly one `:`
    /// (#1677) -- the forward-scan twin of
    /// [`preceding_delimiter_ok`](Self::preceding_delimiter_ok), for a
    /// caller that holds only a key and has not resolved its value's
    /// cursor at all (`census`/`checked_len`/[`DistinctKeyCursors`]'s
    /// key-only walk).
    ///
    /// Scanning forward from a position already in hand costs nothing
    /// beyond the gap itself; resolving the value's own `text_position()`
    /// purely to run the backward check instead measured a real, avoidable
    /// per-field cost (see [`crate::json::light::following_gap_ok`]'s own
    /// doc comment for the number). The default answers `true`
    /// unconditionally, same reasoning as `preceding_delimiter_ok`.
    fn following_colon_ok(&self, key_end: usize) -> bool {
        let _ = key_end;
        true
    }

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

    /// Whether this cursor's *document* declares any `*name` alias at all.
    ///
    /// A whole-document property, not a property of this node, and O(1) --
    /// `YamlIndex::has_aliases` is an emptiness check on an already-built
    /// map. `false` for formats with no alias concept (JSON), where it
    /// monomorphizes away entirely.
    ///
    /// Needed by any builtin that *reorders, selects or drops* a document's
    /// own nodes while keeping them cursor-backed (#1687: `sort`, `sort_by`,
    /// `unique`, `unique_by`, `reverse`, `min`, `max`, `min_by`, `max_by`).
    /// Re-emitting `&anchor`/`*alias` marks is only sound while every alias
    /// still follows a declaration of the same name holding an equal value;
    /// moving nodes around can break that, and `reverse` on `- &x {p: 1}` /
    /// `- *x` demonstrably does -- it yields `- *x` / `- &x {p: 1}`, a
    /// forward reference real yq rejects with `unknown anchor 'x'
    /// referenced`. `enforce_anchor_soundness` (`yq_runner.rs`) exists to
    /// prevent exactly that, but it is a DOM-path pass over a
    /// `CommentTree`, and the cursor-streaming path has none to run it over
    /// (#1350). So those builtins consult this instead and hand an
    /// alias-bearing document to the DOM path unchanged, rather than
    /// emitting YAML succinctly could not read back.
    ///
    /// Gating on aliases rather than anchors is deliberate: an unreferenced
    /// `&x` is valid YAML wherever it lands, so a document with anchors but
    /// no aliases can be reordered freely.
    fn document_has_aliases(&self) -> bool {
        false
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
    /// - `numbers`: which value-formatting convention to use (#1576) -- a
    ///   cursor type with no such distinction (YAML's own JSON-target
    ///   writer, which always behaves as yq's single convention) ignores it.
    ///
    /// Default implementation returns an error indicating streaming is not supported.
    fn stream_json<W: core::fmt::Write>(
        &self,
        _out: &mut W,
        _indent: IndentSpec,
        _sort_keys: bool,
        _numbers: JsonConvention,
    ) -> StreamResult {
        Err(StreamFailure::Fmt)
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
    ) -> StreamResult {
        Err(StreamFailure::Fmt)
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
    ) -> StreamResult {
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
    ///
    /// `numbers`: see [`stream_json`](Self::stream_json)'s own doc comment.
    fn stream_sequence_json<W: core::fmt::Write>(
        _cursors: &[Self],
        _out: &mut W,
        _indent: IndentSpec,
        _sort_keys: bool,
        _numbers: JsonConvention,
    ) -> StreamResult {
        Err(StreamFailure::Fmt)
    }

    /// The YAML counterpart of
    /// [`stream_sequence_json`](Self::stream_sequence_json) (#757), rendering
    /// `cursors` as one block- or flow-style sequence.
    fn stream_sequence_yaml<W: core::fmt::Write>(
        _cursors: &[Self],
        _out: &mut W,
        _indent: IndentSpec,
        _sort_keys: bool,
    ) -> StreamResult {
        Err(StreamFailure::Fmt)
    }

    /// Check if the value at this cursor is falsy (null or false).
    ///
    /// Used for `--exit-status` flag handling without requiring full
    /// materialization. Default implementation returns false (conservative
    /// assumption). `numbers` is the same convention `stream_json` renders
    /// under (#966 follow-up, review of #1576) -- see
    /// [`StreamableValue::is_falsy`](crate::jq::stream::StreamableValue::is_falsy)'s
    /// own doc comment for why a cursor's falsiness can depend on it.
    fn is_falsy(&self, numbers: JsonConvention) -> bool {
        let _ = numbers;
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

    /// Why a scalar that is structurally a string could not be *decoded* --
    /// invalid UTF-8, an invalid escape, an invalid `\u` codepoint.
    ///
    /// [`as_str`](Self::as_str) collapses "not a string" and "a string this
    /// document cannot hand back" into the same `None`, which is how a
    /// decode failure used to reach a materializing conversion indis-
    /// tinguishable from an unknown type and degrade silently to `null`
    /// (#1098, #1247). This separates the two: `Some(reason)` means the
    /// semi-index accepted the span as a string token but the bytes behind
    /// it are not decodable, so a caller can raise a real error instead of
    /// guessing.
    ///
    /// Returns a `&'static str` rather than a formatted message so the
    /// check stays allocation-free and `no_std`-compatible; each format's
    /// own error type owns the wording (`JsonError::message`,
    /// `YamlStringError::message`), shared with its `Display`.
    ///
    /// Defaults to `None` -- correct for any implementation whose strings
    /// cannot fail to decode.
    fn string_decode_error(&self) -> Option<&'static str> {
        None
    }

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

    /// This key's raw source-text span (quotes stripped), regardless of
    /// whether the bytes inside it actually decode.
    ///
    /// Unlike [`key_raw_unescaped`](Self::key_raw_unescaped), which bails
    /// out to `None` on any escape at all, this answers for an escaped-but-
    /// undecodable span too -- it exists solely as a display fallback for
    /// `key_display_string` (#1642), never for identity or hashing.
    /// Defaults to `None`; JSON overrides it, YAML does not (a decode
    /// failure there falls back to `""`, matching its existing
    /// [`key_string`](Self::key_string) convention for any key with no
    /// scalar form).
    fn key_raw_source_span(&self) -> Option<&[u8]> {
        None
    }

    /// The byte position of this value's own text token, when it was
    /// decoded from an already-resolved cursor position rather than
    /// computed (arithmetic, construction) -- letting a caller reuse it for
    /// [`DocumentCursor::preceding_delimiter_ok`]'s delimiter-gap check
    /// (#1677) instead of paying for a second cursor lookup to re-derive
    /// the same position.
    ///
    /// Defaults to `None`. JSON overrides it for the two token-shaped
    /// variants whose own struct already carries this position
    /// (`JsonString`/`JsonNumber`, per their own `start()` doc comments,
    /// #1643).
    fn text_start(&self) -> Option<usize> {
        None
    }

    /// The byte position immediately past this value's own text span, when
    /// resolvable without a fresh cursor lookup -- lets a caller that holds
    /// only a key check the delimiter *forward* from here via
    /// [`DocumentCursor::following_colon_ok`] instead of resolving its
    /// value's `text_position()` (#1677). Defaults to `None`; JSON
    /// overrides it for `String` keys (`JsonString::end`), the only variant
    /// this is used for -- a key is never a `Number` on a well-formed
    /// document, and #1194's own check already refuses one that is.
    fn text_end(&self) -> Option<usize> {
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
    ///
    /// `Err` when the *winning* field's own key or value is preceded by a
    /// malformed delimiter (#1677) -- a targeted lookup such as `.a` never
    /// walks every sibling the way `.[]`/`length`/`keys` do, so it is the
    /// one access pattern that needs its own check rather than inheriting
    /// one from a shared walk. Only the winning field pays for it (JSON
    /// overrides this to check after resolving which occurrence wins, not
    /// during the search); every other format's default answers `Ok` via
    /// [`DocumentCursor::preceding_delimiter_ok`]'s own no-op default.
    fn find_cursor(&self, name: &str) -> Result<Option<Self::Cursor>, EvalError>;

    /// Whether any field has this key -- existence only, no particular
    /// occurrence's value.
    ///
    /// Deliberately **not** `find(name).is_some()`: `find` must walk every
    /// field to honour last-duplicate-key-wins for the *value* it returns,
    /// where existence alone can stop at the first match (#1739). Also
    /// deliberately **not** a walk matching only a literal string-shaped
    /// key (`find`'s own JSON/YAML implementations do that, each skipping a
    /// key that fails to *decode*, #1247, and YAML's skipping an
    /// `Alias`-typed key entirely, a separate, narrower gap than this
    /// method needs to share): this method uses
    /// [`key_display_string`], the exact function [`keys`](Self::keys)'s
    /// own default is built on, so `contains` and `keys` always agree on
    /// which spelling a key resolves to -- decode-failure keys included via
    /// #1642's lossy-fallback substitution, and an alias-typed YAML key
    /// resolved exactly as far as `keys`/`.` themselves resolve it, no
    /// further (a code-review round on #1739 shipped a version calling the
    /// whole-value `as_str()` instead, which fully resolves an alias
    /// *chain* and so silently disagreed with `keys()`'s own single-hop
    /// resolution for a 2+-hop alias key -- reusing `key_display_string`
    /// directly closes that gap structurally rather than chasing each
    /// resolution depth by hand).
    ///
    /// A structurally malformed member (#1194, `key_display_string` ->
    /// `None`) never matches here -- `keys` itself doesn't silently drop
    /// such a member either, but raises rather than omitting it, so this
    /// isn't full behavioral parity with `keys`, just agreement on which
    /// *spelling* a resolvable key displays as.
    ///
    /// Walks via [`uncons_key`](Self::uncons_key), not
    /// [`uncons`](Self::uncons): the latter also materializes each field's
    /// *value*, a real, measured cost (#1514) this existence-only check has
    /// no use for.
    fn contains(&self, name: &str) -> bool {
        let mut fields = self.clone();
        while let Some((key, _key_cursor, rest)) = fields.uncons_key() {
            if key_display_string(&key).is_some_and(|k| k.as_ref() == name) {
                return true;
            }
            fields = rest;
        }
        false
    }

    /// Check if there are no fields.
    fn is_empty(&self) -> bool;

    /// Whether this field list ends on a child with no sibling to pair as
    /// its value -- structurally malformed input the format's index
    /// accepted anyway (#1194).
    ///
    /// JSON overrides this: its semi-index recovers an object's members by
    /// pairing the container's parenthesis-tree children two at a time and
    /// checks neither the key's type nor the count's parity, so
    /// `{"a":1,"b"}` indexes exactly as cleanly as `{"a":1}` does. Every
    /// other format validates while parsing and can never present one, so
    /// the default is `false` and costs them nothing.
    ///
    /// **Ask it of the list a walk has *finished* on, not the one it
    /// starts from.** On an exhausted list this is O(1) -- the answer comes
    /// from a `None` cursor without touching the tree. On the head of a
    /// list it answers only for the *first* child, which is true just for a
    /// single-member object like `{invalid}`: a trailing orphan behind any
    /// well-formed field reads as `false`, which is the mistake #1194's own
    /// first cut made in the `keys_unsorted` writer.
    fn ends_unpaired(&self) -> bool {
        false
    }

    /// The error to raise for a member this format's index accepted but the
    /// format's grammar does not -- an unpaired child, or a key that is not a
    /// string (#1194).
    ///
    /// A method rather than a fixed string at each call site so that one
    /// document reports the *same* cause however it is reached. JSON overrides
    /// it to re-run its strict validator, which names the real syntax error
    /// (`expected string key, found 'i'`); the generic evaluator cannot do
    /// that itself, because [`DocumentCursor`] exposes `text_position` but not
    /// the document bytes the validator needs.
    ///
    /// The default is unreachable for every format shipped today --
    /// [`ends_unpaired`](Self::ends_unpaired) is `false` and every key
    /// stringifies -- so it exists to keep a future format honest rather than
    /// to be seen.
    fn malformed_member_error(&self) -> EvalError {
        EvalError::new("Invalid document text: malformed object member")
    }

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
    fn keys(&self) -> Result<Vec<String>, EvalError> {
        let mut keys = Vec::new();
        let mut fields = self.clone();
        let mut is_first = true;
        while let Some((field, rest)) = fields.uncons() {
            // A key that will not *decode* (#1247/#1385) is preserved via
            // its raw source span rather than raised on (#1642) -- `keys`
            // must agree with `length`/`keys_unsorted`/`.` on whether such
            // a key is present. A key that will not stringify at *all* is a
            // different, structural fault the format's grammar never
            // allowed (#1194) and still raises.
            let Some(key) = key_display_string(&field.key) else {
                return Err(fields.malformed_member_error());
            };
            // #1677: this walk already resolved both the key and the value
            // (`uncons` does), so both checks reuse an existing decode
            // rather than deriving a new position.
            if !key_delimiter_ok::<Self>(&field.key, &field.key_cursor, is_first)
                || !value_delimiter_ok::<Self>(Some(&field.value), &field.value_cursor)
            {
                return Err(fields.malformed_member_error());
            }
            keys.push(key.into_owned());
            fields = rest;
            is_first = false;
        }
        if fields.ends_unpaired() {
            return Err(fields.malformed_member_error());
        }
        Ok(keys)
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

/// The hash a field's key compares under, or `None` when the key has no
/// reliable identity: a JSON escape that will not decode, or -- since
/// #1678 -- a YAML scalar key with the same problem.
///
/// Tries the raw span first (see [`ascii_key_hash`]) -- the #1514 fast path
/// that skips decoding entirely for the common escape-free ASCII key, which
/// [`DocumentValue::string_decode_error`] cannot answer for cheaply: its
/// only implementation (`StandardJson`) resolves through the same full
/// `as_str()` decode `key_string()` does, so checking it *before* the raw
/// span would pay that decode on every key regardless of whether the fast
/// path could have answered for free -- confirmed by CI's perf guard on
/// `wide_keys_unsorted` (+10-12%) when this was tried in that order.
///
/// Once off that fast path (an escape, non-ASCII bytes, or no raw span at
/// all -- YAML never has one), checks `string_decode_error` before falling
/// to [`key_string`](DocumentValue::key_string): YAML's `key_string()`
/// override never returns `None` at all (#222 -- a complex or undecodable
/// key stringifies to `""` rather than being dropped), so a YAML
/// decode-failure key would otherwise hash its `""` fallback like a real
/// key and silently collapse with any other undecodable key in the same
/// object under `collapse: true` (#1385's "never a duplicate" rule exists
/// precisely to prevent that) -- the same ordering
/// [`key_display_string_kind`] already uses for the display-string case.
/// A no-op for JSON off the fast path: its two checks are redundant with
/// each other (both resolve via `as_str`), just no longer free to reach.
fn key_hash_of<V: DocumentValue>(key: &V) -> Option<u64> {
    if let Some(raw) = key.key_raw_unescaped() {
        if let Some(hash) = ascii_key_hash(raw) {
            return Some(hash);
        }
    }
    if key.string_decode_error().is_some() {
        return None;
    }
    key.key_string().map(|key| key_hash(key.as_bytes()))
}

/// [`key_hash_of`] for a caller holding a whole field.
fn field_key_hash<V: DocumentValue, C: DocumentCursor>(field: &DocumentField<V, C>) -> Option<u64> {
    key_hash_of(&field.key)
}

/// Whether this key is one the format's *grammar* never allowed -- a bare
/// `123` or `invalid` where JSON demands a string (#1194).
///
/// The one definition of the #1194 key predicate, because it has to make a
/// distinction that is easy to get wrong in only one of two places: a key
/// that fails to *decode* also fails to stringify, but it is #1247's fault
/// with the opposite answer -- sometimes deliberately preserved verbatim,
/// per #1385's "a key that will not decode is never a duplicate". Testing
/// `key_string().is_none()` alone reports an invalid escape as `expected
/// string key`: the wrong cause, at the wrong severity.
///
/// `false` for every format whose parser validates, whose keys all
/// stringify (issue #222 requires an override to answer `Some("")` rather
/// than `None` for a key with no scalar form), so this costs YAML one
/// predicate on a branch it never takes.
///
/// `key_string()` first, `string_decode_error()` only on its `None` (#1677
/// perf-guard finding): the ordinary case -- a key that decodes fine --
/// answers from that one call alone, instead of paying for two independent
/// full decodes of the same bytes (`key_string()`'s own `as_str()` and a
/// second, separate `as_str()` inside `string_decode_error()`). Equivalent
/// to the original `dec_err.is_none() && key_str.is_none()`: whenever
/// `key_string()` is `Some`, that conjunction is already `false` regardless
/// of `string_decode_error()`, so short-circuiting on it first changes
/// nothing observable, only which (redundant) call gets skipped.
pub(crate) fn key_is_malformed<V: DocumentValue>(key: &V) -> bool {
    key.key_string().is_none() && key.string_decode_error().is_none()
}

/// The string to show for a key, substituting a best-effort fallback when
/// the key is malformed only because its bytes won't *decode* (#1642).
///
/// Never for a key the format's grammar rejects outright (#1194), which
/// `key_is_malformed` still catches and which still must raise. `None`
/// here means the caller should raise -- exactly the #1194 case.
/// The substituted fallback is a **display-only** value: it must never
/// feed back into an identity or hashing decision (`key_hash_of`,
/// `key_string()` itself, `collapse_repeated`/`DistinctKeyCursors`'s
/// dedup), because two different decode-failure keys can produce the same
/// fallback spelling and must still never be treated as duplicates of one
/// another (#1385). A caller that keys a *map* by this string (rather than
/// just displaying it) needs to know when that risk applies -- see
/// `key_display_string_kind` and `DisplayKeyGuard`.
///
/// `pub`, not `pub(crate)`: `succinctly-cli`'s `yq_runner.rs` (a separate
/// binary crate) needs the same key-display logic for its own
/// `--input-format json` materializer, and must not re-derive it.
pub fn key_display_string<V: DocumentValue>(key: &V) -> Option<Cow<'_, str>> {
    key_display_string_kind(key).map(|(key, _is_fallback)| key)
}

/// [`key_display_string`], plus whether the string is the decode-failure
/// **fallback** spelling (`true`) rather than a genuine decode (`false`).
///
/// Must check `string_decode_error()` *first*, not `key_string()`: YAML's
/// `key_string()` override never returns `None` at all (#222 -- a complex
/// or undecodable key stringifies to `""` rather than being dropped), so
/// checking it first would make every YAML decode-failure key silently
/// report `is_fallback = false`, indistinguishable from a genuine key
/// spelled `""`. JSON's two checks happen to be redundant with each other
/// (`string_decode_error()` and `key_string()` both resolve via the same
/// `as_str()`), but the order has to serve both formats' actual contracts,
/// not just the cheaper one.
pub(crate) fn key_display_string_kind<V: DocumentValue>(key: &V) -> Option<(Cow<'_, str>, bool)> {
    if key.string_decode_error().is_some() {
        let fallback = key
            .key_raw_source_span()
            .map_or(Cow::Borrowed(""), String::from_utf8_lossy);
        return Some((fallback, true));
    }
    key.key_string().map(|key| (key, false))
}

/// Guards a display-keyed `IndexMap` (`to_owned`/`materialize`, #1642)
/// against silently merging two keys that `key_display_string_kind`'s own
/// contract forbids treating as the same key.
///
/// A decode-failure key's fallback spelling can collide with another
/// decode-failure key's, or with an unrelated key that happens to decode to
/// the identical text. An `IndexMap<String, _>` has no way to hold two
/// entries under one string, so letting that insert happen the ordinary way
/// silently drops the earlier value -- quieter data loss than #1247's
/// original raise, which this fix relaxed but must not replace with a worse
/// failure mode.
///
/// An *ordinary* repeated key -- neither side a decode-failure fallback --
/// still overwrites without complaint here, matching jq's normal
/// last-key-wins duplicate handling; only a fallback spelling on either
/// side of the collision is refused.
///
/// `pub`, not `pub(crate)`: `succinctly-cli`'s `yq_runner.rs` (a separate
/// binary crate) constructs one per object via [`resolve_display_key`], same
/// reasoning [`key_display_string`] itself documents for why it is `pub`.
#[derive(Default)]
pub struct DisplayKeyGuard {
    fallback_keys: Vec<String>,
}

impl DisplayKeyGuard {
    /// Checks `key` (with the `is_fallback` flag from `key_display_string_kind`
    /// [private, JSON's own], or an equivalent per-format classifier -- see
    /// `YamlValue::key_string_kind` for YAML's own, #1749) against every
    /// key already present in `map` and every fallback key this guard has
    /// already approved. Returns `true` when it is safe to insert (a fresh
    /// key, or an ordinary repeat); `false` when inserting would silently
    /// collapse two keys that must stay distinct, in which case the caller
    /// should raise instead.
    ///
    /// `pub`, not `pub(crate)`: `succinctly-cli`'s `yq_runner.rs`
    /// (`yaml_to_owned_value`, a `YamlCursor`-native materializer that
    /// doesn't go through the `DocumentValue`/`resolve_display_key` path)
    /// needs to drive this guard itself with its own `is_fallback`
    /// classification, same reasoning the struct itself is `pub` for.
    pub fn check<T>(&mut self, map: &IndexMap<String, T>, key: &str, is_fallback: bool) -> bool {
        let collides = map.contains_key(key)
            && (is_fallback || self.fallback_keys.iter().any(|seen| seen.as_str() == key));
        if !collides && is_fallback {
            self.fallback_keys.push(String::from(key));
        }
        !collides
    }
}

/// The full display-key resolution sequence a `to_owned`-shaped materializer
/// needs for one field, in one call.
///
/// Gets the display spelling (`None` when the key does not stringify at all
/// -- #1194's territory, the caller's own job to handle), guards it against
/// a colliding decode-failure fallback (#1642), and raises
/// [`EvalError::colliding_display_key`] instead of allowing a silent
/// overwrite -- #1385 forbids treating two colliding keys as the same one,
/// but a display-keyed map has no way to hold both, so this is what
/// `to_owned`/`materialize` raise instead of silently dropping one of them.
///
/// Shared by `eval_generic.rs`'s three `to_owned*_at_depth` conversions,
/// its validate-only `push_generic_truthiness_cursor_error` (#1645),
/// `lazy.rs`'s `cursor_to_owned_at_depth`, and `succinctly-cli`'s
/// `yq_runner.rs` `--input-format json` bridge (`pub` for that reason,
/// same as [`DisplayKeyGuard`] and [`key_display_string`] before it) -- one
/// definition rather than a sixth hand-copied guard-and-raise sequence.
pub fn resolve_display_key<V: DocumentValue, T>(
    key: &V,
    map: &IndexMap<String, T>,
    guard: &mut DisplayKeyGuard,
) -> Result<Option<String>, EvalError> {
    let Some((key, is_fallback)) = key_display_string_kind(key) else {
        return Ok(None);
    };
    let key = key.into_owned();
    if !guard.check(map, &key, is_fallback) {
        return Err(EvalError::colliding_display_key(&key));
    }
    Ok(Some(key))
}

/// Whether an object field's key is preceded by the delimiter its position
/// requires: nothing if `is_first`, exactly one `,` otherwise (#1677).
///
/// Reuses `key`'s own decode (`text_start`) rather than a fresh cursor
/// lookup, so a caller that has already resolved the key -- every call
/// site here has -- pays nothing extra. Answers `true` when the key has no
/// resolvable text start (a format with no delimiter concept, or a key
/// this document's own grammar never allowed and #1194's separate check
/// already refuses).
///
/// Generic over `F: DocumentFields` rather than a `DocumentValue`/
/// `DocumentCursor` pair: `DocumentFields` doesn't itself bind `Self::Cursor`
/// to `Self::Value::Cursor`, so a caller generic only over `F` (`census`,
/// `checked_len`, [`DistinctKeyCursors`], `effective_fields_checked`) has no
/// way to name that equality -- naming `F` instead sidesteps needing it,
/// since `key`/`key_cursor` are each used only through their own trait.
///
/// `pub`, not `pub(crate)` (#1975): `src/bin/succinctly` is a separate crate
/// from this library, and its own `to_owned_canonicalizing_numbers_at_depth`
/// (the `--input-format json --slurp`/`--eval-all`/`--inplace` DOM bridge)
/// needed this exact check -- it had neither this nor
/// [`value_delimiter_ok`] at all, unlike its `eval_generic::to_owned_at_depth`
/// sibling it otherwise mirrors.
pub fn key_delimiter_ok<F: DocumentFields>(
    key: &F::Value,
    key_cursor: &F::Cursor,
    is_first: bool,
) -> bool {
    match key.text_start() {
        Some(pos) => {
            key_cursor.preceding_delimiter_ok(pos, if is_first { None } else { Some(b',') })
        }
        None => true,
    }
}

/// Whether an object field's value is preceded by exactly one `:` (#1677).
///
/// Reuses `value`'s own decode (`text_start`) when the caller already has
/// one -- a full [`DocumentFields::uncons`] walk resolves it regardless, so
/// this is free for every caller of this function. A key-only walk that has
/// not resolved a value at all should use `key_only_value_delimiter_ok`
/// instead (private to this crate, unlike this function, since every
/// caller of it lives here), which never touches the value cursor.
pub fn value_delimiter_ok<F: DocumentFields>(
    value: Option<&F::Value>,
    value_cursor: &F::Cursor,
) -> bool {
    let pos = value
        .and_then(DocumentValue::text_start)
        .or_else(|| value_cursor.text_position());
    match pos {
        Some(pos) => value_cursor.preceding_delimiter_ok(pos, Some(b':')),
        None => true,
    }
}

/// [`value_delimiter_ok`], for a key-only walk (`census`, `checked_len`,
/// [`DistinctKeyCursors`]) that has not resolved a value cursor at all.
///
/// Scans *forward* from the key's own already-known span end
/// (`key.text_end()`) instead of resolving the value's `text_position()`
/// backward from -- doing it the [`value_delimiter_ok`] way here instead
/// measured **+16%** on a 2 MB `wide` `keys_unsorted` query (#1677), well
/// past `scripts/perf-guard.py`'s 5% threshold. This forward-scan version
/// brings `keys`/`keys_unsorted` back to noise level, but `census`'s own
/// walk (`length`, `keys | length`) still measures **~10%** on the same
/// fixture -- accepted deliberately (see
/// [`following_gap_ok`](crate::json::light::following_gap_ok)'s own doc
/// comment for the full reasoning) because the alternative is silently
/// wrong output on this issue's own headline repro.
pub(crate) fn key_only_value_delimiter_ok<F: DocumentFields>(
    key: &F::Value,
    key_cursor: &F::Cursor,
) -> bool {
    match key.text_end() {
        Some(key_end) => key_cursor.following_colon_ok(key_end),
        None => true,
    }
}

/// An open-addressed set of key hashes: "have I seen this one?" answered
/// as a walk goes, without holding the keys.
///
/// **Only for a caller that cannot sort.** Every batch site here hashes
/// into a `Vec<u64>` and sorts instead, because a sort streams and a table
/// does not: at 7.1M keys the table is 134 MB, and on a 7950X -- 32 MB of
/// L3 per CCD -- that cost 24% on the identity path where the sort cost
/// nothing. An M4 Pro absorbed it and preferred the table, which is the
/// architecture split CLAUDE.md warns memory-bound results carry. The one
/// caller that keeps it is [`DistinctKeyCursors`], which must answer per
/// key as it streams and has nothing to sort yet.
///
/// The table holds hashes only -- 8 bytes per slot, never a key -- so it
/// can grow by rehashing what it already has, which is what a streaming
/// caller with no count up front needs.
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

    /// A table sized for `keys` insertions without a rehash: capacity is the
    /// next power of two at or above `4 * keys / 3`, matching
    /// [`insert`](Self::insert)'s own three-quarter growth threshold.
    ///
    /// Kept in step with that threshold deliberately (#1588). It used to
    /// round up from `2 * keys` for the old one-half factor, and leaving it
    /// there would have made this constructor allocate a table one power of
    /// two larger than the growth path reaches for the same key count --
    /// 2^21 rather than 2^20 slots at 629,881 keys. That matters because
    /// this is exactly the constructor #1588's "give the table a capacity
    /// hint" direction points at: wiring it up while it disagreed with
    /// `insert` would have silently restored the memory this change removes.
    pub fn with_capacity(keys: usize) -> Self {
        if keys == 0 {
            return Self::new();
        }
        let slots = keys
            .saturating_mul(4)
            .div_ceil(3)
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
        // Grow before inserting so the load factor never exceeds three
        // quarters. It was one half until #1588: linear probing's run
        // lengths do climb past that, but the table also halves, and on a
        // wide object the locality wins outright -- measured on `wide/10mb`
        // (629,881 keys), `keys_unsorted` went 52.8 -> 36.9 MiB peak RSS and
        // *also* 0.106 -> 0.099s. A sweep says 3/4 is the point: 7/8 and
        // 15/16 buy no further memory (the same power-of-two table serves
        // all three at this key count) and only pay more probing, measuring
        // 0.114s and 0.115s respectively.
        if (self.len + 1) * 4 > self.slots.len() * 3 {
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

/// The sorted, deduplicated hashes that occur more than once in `sorted`,
/// plus how many distinct hash values it holds in total.
///
/// `sorted` must already be sorted ascending; the returned list is too, so
/// callers can `binary_search` it.
fn shared_hashes(sorted: &[u64]) -> (Vec<u64>, usize) {
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

/// Whether an already-sorted hash list holds the same value twice.
fn hashes_repeat(sorted: &[u64]) -> bool {
    sorted.windows(2).any(|pair| pair[0] == pair[1])
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
    /// Whether the object holds a member the format's grammar never allowed
    /// -- a non-string key, or a trailing child with no sibling to pair as
    /// its value (#1194).
    ///
    /// Rides the census rather than taking a walk of its own: `length` is
    /// the caller, and the census already visits every key. A separate
    /// pre-check measured **+64%** on `length` over a 20 MB `wide`
    /// document, which is what "only where the caller was going to walk
    /// anyway" has to actually mean.
    malformed: bool,
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
    let mut malformed = false;
    let mut walk = fields.clone();
    let mut is_first = true;
    while let Some((key, cursor, rest)) = walk.uncons_key() {
        // #1677: both checks are free here -- comma-before-key reuses
        // `key`'s own decode, and colon-before-value scans forward from
        // it (`key_only_value_delimiter_ok`) rather than resolving the
        // value's own position.
        if !key_delimiter_ok::<F>(&key, &cursor, is_first)
            || !key_only_value_delimiter_ok::<F>(&key, &cursor)
        {
            malformed = true;
        }
        match key_hash_of(&key) {
            Some(hash) => hashes.push(hash),
            // `key_hash_of` answers `None` for exactly the keys that do not
            // stringify, so this is the only branch [`key_is_malformed`] can
            // fire on -- and it is never taken by a well-formed document.
            None => {
                unkeyed += 1;
                malformed |= key_is_malformed(&key);
            }
        }
        walk = rest;
        is_first = false;
    }
    // `walk` is the list this loop *finished* on, the only list
    // `ends_unpaired` answers for (#1194), and asking it is O(1).
    malformed |= walk.ends_unpaired();
    hashes.sort_unstable();
    let (shared, distinct_hashes) = shared_hashes(&hashes);
    if shared.is_empty() {
        return KeyCensus {
            distinct: distinct_hashes,
            unkeyed,
            repeated: false,
            malformed,
        };
    }

    // A shared hash is nearly always a genuine repeat, but two different
    // keys can collide, and counting those as one would be wrong. Re-walk
    // owning *only* the colliding keys -- on an ordinary duplicate that is
    // a handful of strings, not one per field. `uncons_key` (#1514), same
    // as the first walk above: a census never looks at values.
    let mut colliding: Vec<String> = Vec::new();
    let mut walk = fields.clone();
    while let Some((key, _cursor, rest)) = walk.uncons_key() {
        // `key_hash_of` answers `Some` only for a key that stringifies,
        // so the inner `key_string` is how the owned spelling is obtained
        // here, not a second filter that could drop a counted field.
        if let Some(hash) = key_hash_of(&key) {
            if shared.binary_search(&hash).is_ok() {
                if let Some(k) = key.key_string() {
                    colliding.push(k.into_owned());
                }
            }
        }
        walk = rest;
    }
    colliding.sort_unstable();
    let (distinct_colliding, repeated) = distinct_sorted(&colliding);
    KeyCensus {
        // `distinct_hashes` counts distinct *hashes*; every colliding
        // group contributed exactly one of them, so swap each group's
        // single hash for however many distinct keys it actually held.
        distinct: distinct_hashes - shared.len() + distinct_colliding,
        unkeyed,
        repeated,
        malformed,
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
    let mut hashes: Vec<u64> = fields.iter().filter_map(field_key_hash).collect();
    hashes.sort_unstable();
    hashes_repeat(&hashes)
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

/// [`effective_fields`], refusing an object whose members the format's
/// grammar never allowed (#1194).
///
/// The value-cursor-carrying counterpart of [`effective_len_checked`], for a
/// caller that needs the fields themselves (`.[]`'s bare object arm) rather
/// than just a count. The check and the field list come out of the same
/// walk this function was already making, for the reason
/// `effective_len_checked`'s own doc comment gives: a caller-side pre-check
/// (`malformed_object_member`) would double the cost of a walk this function
/// already runs. `to_entries` is the one caller for which that redundant
/// walk is negligible (it materializes every value regardless), which is
/// why it keeps using the pre-check instead.
#[allow(clippy::type_complexity)] // STYLE-0004: mirrors effective_fields's own Vec<DocumentField<..>>, plus a Result for the #1194 check; a named alias would add indirection for one extra wrapper.
pub fn effective_fields_checked<F: DocumentFields>(
    fields: &F,
    collapse: bool,
) -> Result<Vec<DocumentField<F::Value, F::Cursor>>, EvalError> {
    let mut out = Vec::new();
    let mut walk = fields.clone();
    let mut is_first = true;
    while let Some((field, rest)) = walk.uncons() {
        if key_is_malformed(&field.key) {
            return Err(walk.malformed_member_error());
        }
        // #1677: free here too -- `uncons` already resolved both key and
        // value, so both checks reuse an existing decode.
        if !key_delimiter_ok::<F>(&field.key, &field.key_cursor, is_first)
            || !value_delimiter_ok::<F>(Some(&field.value), &field.value_cursor)
        {
            return Err(walk.malformed_member_error());
        }
        out.push(field);
        walk = rest;
        is_first = false;
    }
    if walk.ends_unpaired() {
        return Err(walk.malformed_member_error());
    }
    if collapse && keys_repeat(&out) {
        out = collapse_repeated(out);
    }
    Ok(out)
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

/// [`collapsed_fields`], but skipped outright when the mode doesn't
/// collapse (yq) -- the single guard the two positional `LazyKeys` arms
/// (`Index`, `Last`) both need before they can answer.
pub fn collapsed_fields_if<F: DocumentFields>(
    fields: &F,
    collapse: bool,
) -> Option<Vec<DocumentField<F::Value, F::Cursor>>> {
    if collapse {
        collapsed_fields(fields)
    } else {
        None
    }
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
/// the object to `collapse_confirmed_repeat`, which decides on the keys
/// themselves:
///
/// - A genuine collapse — the remainder switches to the exact collapsed
///   list, resuming at the count already yielded, which lines up because
///   the collapsed list opens with those same first occurrences.
/// - A 64-bit hash collision, no real duplicate — the answer covers the
///   whole object, so the probe retires rather than firing again at the
///   next one.
///
/// `collapse` false (yq) carries no probe state at all and is the plain
/// cons-list walk it always was.
///
/// Key and cursor pairs for the confirmed-collapse case below -- factored
/// out into its own alias since `Option<Box<Vec<(F::Value, F::Cursor)>>>`
/// trips `clippy::type_complexity` as an inline field type.
type CollapsedKeys<F> = Vec<(<F as DocumentFields>::Value, <F as DocumentFields>::Cursor)>;

#[derive(Clone)]
pub struct DistinctKeyCursors<F: DocumentFields> {
    /// The fields still to walk.
    rest: F,
    /// The object as a whole, for the exact resolution above. A `F` is a
    /// cursor position, so this is a copy of a couple of machine words.
    all: F,
    /// Hashes of the keys yielded so far, while the rule is in force and
    /// the object is not yet proved clean. Deliberately **not** boxed
    /// (#1973 code review): unlike `collapsed` below, this is constructed
    /// eagerly in [`new`](Self::new) for every jq-mode walk (`collapse`
    /// true is the default), not lazily once a duplicate is actually
    /// found -- `KeyHashes::new()` itself is genuinely free (an empty
    /// `Vec` allocates nothing until its first insertion), but wrapping it
    /// in a `Box` would force a heap allocation for the `KeyHashes`
    /// struct itself on every single call, regardless of whether the
    /// object ever has a duplicate key. That would add a real,
    /// unconditional per-object allocation to `keys_unsorted`/`.[]`-style
    /// hot paths this codebase has otherwise gone out of its way to keep
    /// allocation-free (see `KeyHashes::slots`'s own doc comment, and the
    /// O5 optimization, #1599/#1606/#1609) in exchange for shrinking a
    /// struct that is not on those same hot paths' critical dimension.
    seen: Option<KeyHashes>,
    /// How many cursors have gone out, which is where `collapsed` resumes.
    yielded: usize,
    /// The exact collapsed key list, once a repeat is confirmed. Key and
    /// cursor only -- this iterator never reads a field's value, so
    /// `collapse_confirmed_repeat` never materializes one (#1514 review).
    /// Boxed (#1973): unlike `seen` above, this is only ever set to `Some`
    /// once a repeat is already confirmed (`collapse_confirmed_repeat` has
    /// already run its own `IndexMap`/`Vec` allocations by that point), so
    /// the one extra box here is genuinely free on every walk that never
    /// collapses -- already niche-optimized as `Option<Vec<_>>`, wrapping
    /// it in a `Box` before niching only removes the `Vec`'s own three
    /// machine words from every instance that stays `None`.
    collapsed: Option<Box<CollapsedKeys<F>>>,
    /// Whether the walk ran out on an unpaired child (#1194). Recorded as
    /// the walk discovers it, because neither branch can answer afterwards:
    /// `rest` is exhausted on one and stale mid-object on the other.
    ended_unpaired: bool,
    /// How many fields this walk has examined in raw document order --
    /// distinct from `yielded`, which undercounts once a repeat starts
    /// collapsing. Used only to know whether the *next* field is the
    /// object's first, for #1677's comma check.
    walked: usize,
    /// Whether any field this walk has examined had a malformed `,`/`:`
    /// delimiter (#1677). Same "ask only once exhausted" contract as
    /// [`ended_unpaired`](Self::ended_unpaired), and for the same reason:
    /// answering up front would cost a second walk.
    delimiter_fault: bool,
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
            ended_unpaired: false,
            walked: 0,
            delimiter_fault: false,
        }
    }

    /// Whether any field this walk has examined had a malformed `,`/`:`
    /// delimiter (#1677) -- see [`ended_unpaired`](Self::ended_unpaired)
    /// for the "only meaningful once exhausted" contract this shares.
    pub fn delimiter_fault(&self) -> bool {
        self.delimiter_fault
    }

    /// Whether the object ends on a child with no sibling to pair as its
    /// value -- structurally malformed JSON the semi-index accepted (#1194).
    ///
    /// **Only meaningful once the iterator is exhausted**, since that is
    /// when the walk finds out; it reads `false` until then. A streaming
    /// consumer therefore learns this *after* writing its keys, which is
    /// the deliberate trade: answering up front would cost a second walk
    /// over every key, and `keys_unsorted` over a 2 MB `wide` document is
    /// one of the six workloads `scripts/perf-guard.py` pins precisely
    /// because it is sensitive to that.
    ///
    /// Always `false` for a format whose parser validates (everything but
    /// JSON), via [`DocumentFields::ends_unpaired`]'s default.
    pub fn ended_unpaired(&self) -> bool {
        self.ended_unpaired
    }

    /// Either #1194 fault together: [`ended_unpaired`](Self::ended_unpaired)
    /// or [`delimiter_fault`](Self::delimiter_fault). One definition of the
    /// combined check, so a caller can't correctly check one half and
    /// forget the other -- #1956 was exactly that: two call sites checking
    /// `ended_unpaired()` alone, missing the `delimiter_fault()` sibling
    /// three others already had. Same "only meaningful once exhausted"
    /// contract as both halves individually.
    pub fn is_malformed(&self) -> bool {
        self.ended_unpaired() || self.delimiter_fault()
    }

    /// The error to raise once [`is_malformed`](Self::is_malformed) is
    /// `true` -- delegates to the whole object's own `all` copy rather than
    /// `rest` (stale mid-object on one #1194 shape, already exhausted on the
    /// other), so a caller holding only this walk and no separate `F` of its
    /// own (`LazySource::Keys`, #1956) can still build the right error.
    pub fn malformed_member_error(&self) -> EvalError {
        self.all.malformed_member_error()
    }
}

impl<F: DocumentFields> Iterator for DistinctKeyCursors<F> {
    /// The key *and* a cursor to it. Both, because every consumer wants
    /// the key itself, and re-deriving it from the cursor materializes a
    /// second time what the walk has already built (#1514).
    type Item = (F::Value, F::Cursor);

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(collapsed) = &self.collapsed {
            let (key, cursor) = collapsed.get(self.yielded)?;
            self.yielded += 1;
            return Some((key.clone(), *cursor));
        }
        let Some((key, key_cursor, tail)) = self.rest.uncons_key() else {
            // Exhaustion and "the last child never got a value" are the
            // same `None` from `uncons_key`. Ask now, while `rest` still
            // holds the list the walk stopped on -- this is the only
            // moment the two can be told apart (#1194).
            self.ended_unpaired = self.rest.ends_unpaired();
            return None;
        };
        self.rest = tail;
        // #1677: checked for every field examined, in raw document order,
        // before any collapse decision -- a comma/colon fault must surface
        // even for a field a later duplicate ends up collapsing away.
        let is_first = self.walked == 0;
        self.walked += 1;
        if !key_delimiter_ok::<F>(&key, &key_cursor, is_first)
            || !key_only_value_delimiter_ok::<F>(&key, &key_cursor)
        {
            self.delimiter_fault = true;
        }
        let repeat = self
            .seen
            .as_mut()
            .is_some_and(|seen| key_hash_of(&key).is_some_and(|hash| seen.insert(hash)));
        if repeat {
            let confirmed = collapse_confirmed_repeat(&self.all);
            // Recorded here as well as at exhaustion above: on the
            // collapsing branch below `rest` stops mid-object and never
            // reaches the `None` arm, so that arm would never see the
            // orphan. `confirmed` covers the whole object, so it can.
            self.ended_unpaired = confirmed.ends_unpaired;
            self.delimiter_fault |= confirmed.delimiter_fault;
            let ConfirmedRepeat { keys, total, .. } = confirmed;
            if keys.len() < total {
                self.collapsed = Some(Box::new(keys));
                self.seen = None;
                // Resumes at `yielded`, which the collapsed list's own
                // prefix matches: every key emitted so far was a first
                // occurrence, and collapsing keeps those in place.
                return self.next();
            }
            // A 64-bit hash collision, not a real duplicate: nothing
            // collapsed, so the answer covers the whole object and the
            // probe retires rather than firing again at the next one.
            self.seen = None;
        }
        self.yielded += 1;
        Some((key, key_cursor))
    }
}

/// The exact collapsed key list for an object [`DistinctKeyCursors`] has
/// already flagged as a probable repeat via its hash probe.
///
/// Unlike [`collapsed_fields`] (used by the `.[] | map` collapse path in
/// `eval_generic.rs`, which needs `value_cursor` too), this never
/// materializes a field's value -- `DistinctKeyCursors` only ever reads the
/// key -- and it skips `collapsed_fields`'s own [`census`] pre-check
/// entirely: the caller already knows a repeat is likely, so re-deriving
/// "is this object actually repeated?" first is redundant work whose answer
/// this caller doesn't need. A genuine hash collision (no real repeat)
/// still resolves correctly here -- the caller sees `collapsed.len() ==
/// total` in that case, exactly what `census`'s own exact check would have
/// reported, just discovered post hoc instead of as a separate pass.
///
/// This turns the up-to-three full-object walks `collapsed_fields` +
/// `census` would otherwise run (hash-collect, conditional exact-collision
/// narrowing, then a value-materializing `all_fields()` walk) into one
/// key-only walk (#1514 review).
fn collapse_confirmed_repeat<F: DocumentFields>(fields: &F) -> ConfirmedRepeat<F> {
    let mut slot_of: IndexMap<String, usize> = IndexMap::new();
    let mut out: Vec<(F::Value, F::Cursor)> = Vec::new();
    let mut total = 0usize;
    let mut delimiter_fault = false;
    let mut walk = fields.clone();
    let mut is_first = true;
    while let Some((key, key_cursor, rest)) = walk.uncons_key() {
        total += 1;
        // #1677: re-walks the whole object from the start, so this repeats
        // a check `DistinctKeyCursors::next` already ran on fields before
        // the repeat was confirmed -- negligible next to this function's
        // own re-walk, which only runs once a duplicate is confirmed.
        if !key_delimiter_ok::<F>(&key, &key_cursor, is_first)
            || !key_only_value_delimiter_ok::<F>(&key, &key_cursor)
        {
            delimiter_fault = true;
        }
        match key.key_string().map(Cow::into_owned) {
            Some(k) => match slot_of.get(&k) {
                Some(&slot) => out[slot] = (key, key_cursor),
                None => {
                    slot_of.insert(k, out.len());
                    out.push((key, key_cursor));
                }
            },
            // Same "no name to collapse on, keep it where it stands"
            // handling as `collapse_repeated` (#1385 review).
            None => out.push((key, key_cursor)),
        }
        walk = rest;
        is_first = false;
    }
    ConfirmedRepeat {
        // `walk` is the list this loop *finished* on, which is the only
        // list `ends_unpaired` answers for (#1194). Free here: the walk had
        // to run anyway to collapse the duplicate.
        ends_unpaired: walk.ends_unpaired(),
        delimiter_fault,
        keys: out,
        total,
    }
}

/// What [`collapse_confirmed_repeat`]'s single walk learned about an object.
///
/// A struct rather than the tuple this used to return: three unrelated
/// answers ride out of one walk, and naming them is what let the third be
/// added without a `clippy::type_complexity` waiver.
struct ConfirmedRepeat<F: DocumentFields> {
    /// The collapsed key list, first occurrence of each key in place.
    keys: Vec<(F::Value, F::Cursor)>,
    /// How many fields the walk actually saw, collapsed or not. Equal to
    /// `keys.len()` exactly when nothing collapsed -- the signature of a
    /// 64-bit hash collision rather than a real duplicate.
    total: usize,
    /// Whether the walk ran out on an unpaired child (#1194).
    ends_unpaired: bool,
    /// Whether any field had a malformed `,`/`:` delimiter (#1677).
    delimiter_fault: bool,
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

/// [`effective_len`], refusing an object whose members the format's grammar
/// never allowed (#1194).
///
/// The count and the check come out of **one** walk, which is the whole
/// point of putting the check here rather than in front of the call: both
/// spellings of this question -- `length` and `keys | length` -- reach
/// `effective_len`, and a caller-side pre-check both doubled the work and
/// covered only the first spelling, leaving `{invalid} | length` erroring
/// while `{invalid} | keys | length` answered `0`.
///
/// [`effective_len`] itself stays infallible for the two `eval.rs`-side
/// callers, which reach `length` through the other evaluator and answer for
/// an already-materialized value.
pub fn effective_len_checked<F: DocumentFields>(
    fields: &F,
    collapse: bool,
) -> Result<usize, EvalError> {
    if !collapse {
        return checked_len(fields);
    }
    let census = census(fields);
    if census.malformed {
        return Err(fields.malformed_member_error());
    }
    Ok(census.effective_len())
}

/// The plain field count, refusing a malformed member as it goes.
///
/// The no-collapse counterpart of the [`census`] path above, and likewise
/// one walk rather than two: this *replaces* [`DocumentFields::len`]'s own
/// walk instead of preceding it, and walks with `uncons_key` where `len`
/// walks with `uncons`, so it materializes no value cursor (#1514).
fn checked_len<F: DocumentFields>(fields: &F) -> Result<usize, EvalError> {
    let mut count = 0usize;
    let mut walk = fields.clone();
    let mut is_first = true;
    while let Some((key, cursor, rest)) = walk.uncons_key() {
        if key_is_malformed(&key) {
            return Err(walk.malformed_member_error());
        }
        // #1677: same cheap forward-scan checks as `census`'s own walk above.
        if !key_delimiter_ok::<F>(&key, &cursor, is_first)
            || !key_only_value_delimiter_ok::<F>(&key, &cursor)
        {
            return Err(walk.malformed_member_error());
        }
        count += 1;
        walk = rest;
        is_first = false;
    }
    if walk.ends_unpaired() {
        return Err(walk.malformed_member_error());
    }
    Ok(count)
}

/// An object's field names under the mode's duplicate-key rule, in
/// document order (first position of each key).
///
/// Walks [`DistinctKeyCursors`] rather than hashing `fields.keys()`'s own
/// output: collapsing on the *materialized strings* would be unsafe once a
/// decode-failure key can produce a non-`None` `key_display_string`
/// fallback (#1642) -- two different such keys can share a fallback
/// spelling and must still never collapse into one (#1385).
/// `DistinctKeyCursors` already gets this right, by deciding collapse from
/// `key_hash_of`/[`key_string`](DocumentValue::key_string) -- both `None`,
/// hence "never a duplicate", for exactly a decode-failure key -- *before*
/// any display fallback is ever applied.
pub fn effective_keys<F: DocumentFields>(
    fields: &F,
    collapse: bool,
) -> Result<Vec<String>, EvalError> {
    let mut keys = Vec::new();
    let mut cursors = DistinctKeyCursors::new(fields, collapse);
    for (key, _cursor) in cursors.by_ref() {
        let Some(key) = key_display_string(&key) else {
            return Err(fields.malformed_member_error());
        };
        keys.push(key.into_owned());
    }
    // #1677: same malformed-delimiter check `distinct_key_cursors`
    // (`eval_generic.rs`) applies to its own `DistinctKeyCursors` walk --
    // this one just never got wired to it when #1642 rewrote this function
    // onto `DistinctKeyCursors` on `main`, ahead of this check existing.
    if cursors.is_malformed() {
        return Err(fields.malformed_member_error());
    }
    Ok(keys)
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

    /// The error to raise for an element preceded by a malformed `,`
    /// (#1677) -- the array counterpart of
    /// [`DocumentFields::malformed_member_error`], which this trait had no
    /// equivalent of before #1677 (an array element can only be malformed
    /// in its own content, never by pairing, until the delimiter class).
    ///
    /// The default is unreachable for every format shipped today, same
    /// reasoning as that method's own default.
    fn malformed_element_error(&self) -> EvalError {
        EvalError::new("Invalid document text: malformed array element")
    }

    /// [`collect_cursors`](Self::collect_cursors), refusing an element
    /// preceded by a malformed `,` (#1677).
    ///
    /// A separate method rather than a change to `collect_cursors` itself:
    /// several callers (`shuffle`, `pivot`) reach each element through
    /// `to_owned_cursor` regardless, which already validates *that*
    /// element's own contents, so paying for a fresh `text_position()` per
    /// element here would tax them for a check their own walk makes moot.
    /// Only a caller that needs the gap between siblings checked --
    /// `.[]`/`to_entries` on arrays -- uses this instead.
    fn collect_cursors_checked(&self) -> Result<Vec<Self::Cursor>, EvalError> {
        let mut cursors = Vec::new();
        let mut elems = *self;
        let mut is_first = true;
        while let Some((cursor, rest)) = elems.uncons_cursor() {
            if !cursor.element_gap_ok(is_first) {
                return Err(self.malformed_element_error());
            }
            cursors.push(cursor);
            elems = rest;
            is_first = false;
        }
        Ok(cursors)
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

#[cfg(test)]
mod checked_len_tests {
    use super::{effective_len_checked, DocumentValue};
    use crate::json::JsonIndex;

    /// The two arms of [`effective_len_checked`] must agree about whether a
    /// document is well formed (#1194).
    ///
    /// `collapse` picks between two entirely separate implementations -- the
    /// `census` walk and `checked_len` -- and only the first is reachable
    /// through a shipped binary today: `collapse` is true in jq mode, and the
    /// no-collapse arm belongs to yq mode, whose parser validates and so can
    /// never present a malformed member. That is exactly why the arms are
    /// driven directly here. An unexercised copy of a predicate is how two
    /// copies drift, and the pairing is the property worth pinning: whichever
    /// arm a future format lands on, it must give the same verdict and the
    /// same count.
    #[test]
    fn both_arms_agree_on_malformed_and_well_formed_objects_1194() {
        for (json, expected) in [
            (&br#"{"a":1,"b":2}"#[..], Some(2)),
            // Collapsing is the one thing the arms legitimately differ on:
            // jq's rule folds the repeat, yq's keeps both.
            (&br#"{"a":1,"a":2}"#[..], None),
            (b"{}", Some(0)),
            (b"{invalid}", None),           // orphan, post-walk check
            (br#"{123: 1, "b": 2}"#, None), // bad key, per-field check
            (br#"{"a":1, "b"}"#, None),     // orphan behind a good field
            // #1677: the delimiter class, same shared walk.
            (br#"{"a" 1,"b":2}"#, None), // missing `:` on the first field
            (br#"{"a":1 "b":2}"#, None), // missing `,` between fields
            (br#"{"a":1,,"b":2}"#, None), // doubled `,` between fields
        ] {
            let index = JsonIndex::build(json);
            let cursor = index.root(json);
            let fields = cursor.value().as_object().expect("an object");

            let collapsed = effective_len_checked(&fields, true);
            let plain = effective_len_checked(&fields, false);

            assert_eq!(
                collapsed.is_ok(),
                plain.is_ok(),
                "the arms disagree about {}: {collapsed:?} vs {plain:?}",
                String::from_utf8_lossy(json)
            );
            if let Some(len) = expected {
                assert_eq!(collapsed.as_ref().ok(), Some(&len));
                assert_eq!(plain.as_ref().ok(), Some(&len));
            }
            if let Err(err) = &plain {
                // Same cause, not merely the same verdict -- `checked_len`
                // asks the offending key's list while the census arm asks
                // the head, and `malformed_member_error` is a method rather
                // than a per-site literal precisely so those coincide.
                assert_eq!(err.message, collapsed.unwrap_err().message);
                assert!(err.message.contains("Invalid JSON text"));
            }
        }
    }

    /// A key that will not *decode* is #1247's fault, not #1194's, on the
    /// no-collapse arm too -- the distinction `key_is_malformed` exists to
    /// make, checked on the copy of it that no shipped binary reaches.
    #[test]
    fn the_plain_arm_leaves_decode_failures_alone_1194() {
        let json: &[u8] = br#"{"a\q":1,"b":2}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let fields = cursor.value().as_object().expect("an object");

        assert_eq!(effective_len_checked(&fields, false).ok(), Some(2));
    }
}

#[cfg(test)]
mod key_display_string_tests {
    use super::{
        effective_keys, effective_len_checked, key_display_string, key_is_malformed,
        DocumentFields, DocumentValue,
    };
    use crate::json::JsonIndex;

    /// A normal key stringifies exactly as `key_string()` already would --
    /// the fallback must never engage when there is nothing to fall back
    /// from.
    #[test]
    fn normal_key_is_unaffected_1642() {
        let json: &[u8] = br#"{"a":1}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let fields = cursor.value().as_object().expect("an object");
        let (field, _) = fields.uncons().expect("one field");
        assert_eq!(key_display_string(&field.key()).as_deref(), Some("a"));
    }

    /// A key whose bytes won't *decode* (#1247) gets its raw source span
    /// instead of raising -- the whole point of #1642.
    #[test]
    fn decode_failure_key_falls_back_to_raw_span_1642() {
        let json: &[u8] = br#"{"a\q":1}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let fields = cursor.value().as_object().expect("an object");
        let (field, _) = fields.uncons().expect("one field");
        assert_eq!(key_display_string(&field.key()).as_deref(), Some("a\\q"));
    }

    /// A key the format's grammar never allowed at all (#1194) is a
    /// different, structural fault -- still `None`, still the caller's cue
    /// to raise.
    #[test]
    fn structurally_malformed_key_still_reports_none_1642() {
        let json: &[u8] = br"{123: 1}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let fields = cursor.value().as_object().expect("an object");
        let (field, _) = fields.uncons().expect("one field");
        assert!(key_display_string(&field.key()).is_none());
        assert!(key_is_malformed(&field.key()));
    }

    /// Two different decode-failure keys with byte-identical source spans
    /// must never collapse into one under jq mode's collapse rule (#1385) --
    /// `effective_keys` has to decide collapse from `key_hash_of`/
    /// `key_string`, which stay `None`-safe for a decode failure, *before*
    /// `key_display_string`'s fallback is ever applied for display.
    #[test]
    fn effective_keys_never_collapses_colliding_decode_failures_1642() {
        let json: &[u8] = br#"{"\ud800":1,"\ud800":2}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let fields = cursor.value().as_object().expect("an object");

        let keys = effective_keys(&fields, true).expect("no genuine #1194 fault");
        assert_eq!(keys, vec!["\\ud800".to_string(), "\\ud800".to_string()]);
    }

    /// A real duplicate still collapses under jq mode exactly as before --
    /// `effective_keys`'s rewrite around `DistinctKeyCursors` must not
    /// regress the ordinary case it replaces.
    #[test]
    fn effective_keys_still_collapses_a_real_duplicate_1642() {
        let json: &[u8] = br#"{"a":1,"a":2,"b":3}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let fields = cursor.value().as_object().expect("an object");

        assert_eq!(
            effective_keys(&fields, true).expect("well formed"),
            vec!["a".to_string(), "b".to_string()]
        );
        assert_eq!(
            effective_keys(&fields, false).expect("well formed"),
            vec!["a".to_string(), "a".to_string(), "b".to_string()]
        );
    }

    /// YAML counterpart of [`effective_keys_never_collapses_colliding_decode_failures_1642`]
    /// -- #1678. Unlike JSON, `YamlValue::key_string()` never returns `None`
    /// at all (#222), so before `key_hash_of` learned to check
    /// `string_decode_error()` first, two different undecodable YAML keys
    /// hashed the same `""` fallback and silently collapsed into one,
    /// violating #1385's "never a duplicate" rule on this format too.
    #[test]
    fn effective_keys_never_collapses_colliding_yaml_decode_failures_1678() {
        use crate::yaml::YamlIndex;

        let yaml = b"\"a\\qb\": 1\n\"a\\zc\": 2\n";
        let index = YamlIndex::build(yaml).expect("valid YAML");
        let cursor = index.root(yaml);
        let mapping_cursor = cursor
            .first_child()
            .expect("YAML document should have content");
        let fields = mapping_cursor.value().as_object().expect("a mapping");

        assert_eq!(effective_len_checked(&fields, true).ok(), Some(2));
        let keys = effective_keys(&fields, true).expect("no genuine #1194 fault");
        assert_eq!(keys, vec![String::new(), String::new()]);
    }

    /// `DocumentFields::keys()`'s per-field raise for a key the format's
    /// grammar never allowed at all (#1194) -- distinct from a
    /// decode-failure key, which `key_display_string` now folds into a
    /// fallback spelling instead (#1642) and which this method no longer
    /// raises on either. This trait method itself has had no production
    /// caller since `effective_keys`'s own #1385 rewrite moved off it (see
    /// that function's doc comment above), but it stays `pub` API surface
    /// and must keep raising correctly for a caller outside this crate.
    #[test]
    fn document_fields_keys_raises_on_malformed_key_1194() {
        let json: &[u8] = br"{123: 1}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let fields = cursor.value().as_object().expect("an object");
        let err = fields.keys().expect_err("a bare numeric key is not JSON");
        assert!(err.message.contains("Invalid JSON text"), "{err:?}");
    }

    /// The post-walk sibling of the test above: `{invalid}`'s lone child
    /// never pairs into a field at all, so nothing reaches the per-field
    /// check -- only `ends_unpaired()` after the loop catches it, same
    /// two-fault split as `test_jq_malformed_object_keys_raises_on_both_faults_1194`
    /// in `tests/jq_cli_tests.rs` exercises through the CLI.
    #[test]
    fn document_fields_keys_raises_on_unpaired_trailing_field_1194() {
        let json: &[u8] = br"{invalid}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let fields = cursor.value().as_object().expect("an object");
        let err = fields.keys().expect_err("an unpaired member is not JSON");
        assert!(err.message.contains("Invalid JSON text"), "{err:?}");
    }
}
