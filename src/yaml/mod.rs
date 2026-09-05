//! YAML semi-indexing for succinct YAML parsing.
//!
//! This module provides semi-indexing for YAML 1.2 documents, enabling efficient
//! navigation using rank/select operations on the balanced parentheses (BP) tree.
//!
//! # Supported
//!
//! - Block mappings and sequences
//! - Flow mappings `{key: value}` and sequences `[a, b, c]`
//! - Nested flow containers (e.g., `{users: [{name: Alice}]}`)
//! - Simple scalars (unquoted, double-quoted, single-quoted)
//! - Block scalars: literal (`|`) and folded (`>`)
//! - Chomping modifiers: strip (`-`), keep (`+`), clip (default)
//! - Anchors (`&name`) and aliases (`*name`)
//! - Merge keys (`<<: *anchor`), including multiple sources (`<<: [*a, *b]`),
//!   resolved at query time in `YamlFields` (issue #171)
//! - Explicit keys (`?` / `:`)
//! - Multi-document streams (`---` / `...`), wrapped in an implicit root sequence
//! - Comments (ignored in block context)
//! - `%YAML` / `%TAG` directives — recognized and consumed; a directive's version/handle
//!   is not surfaced (issue #225)
//! - Tags (`!!str`, `!custom`, verbatim `!<...>`) — resolved, matching `yq`: the 5
//!   core-schema tags (`!!str`/`!!null`/`!!bool`/`!!int`/`!!float`) force scalar-type
//!   coercion regardless of quoting style; any other tag (a custom tag,
//!   `!!seq`/`!!map`/`!!set`/`!!omap`, a `%TAG`-shorthand tag, verbatim) does not change
//!   resolution (issue #224). JSON output drops the tag either way, since JSON has no tag
//!   syntax; YAML output re-emits it verbatim on the scalar or key it decorated (matching
//!   `yq`), though a tag on a whole mapping/sequence (`!!seq [1, 2]`) is not yet preserved
//!   on that path. The explicit source tag, if any, is available via
//!   `YamlCursor::explicit_tag` — distinct from the pre-existing `YamlCursor::tag`, which
//!   is an *inferred* type label derived from the resolved value's shape, not the
//!   source text.
//!
//! # Validation
//!
//! `YamlIndex::build` performs minimal validation during indexing (structural recognition
//! only) and accepts many malformed documents. It is a non-validating loader: of the YAML
//! Test Suite's 94 invalid documents it rejects 12. Do not rely on a parse error to
//! detect malformed input. See `docs/compliance/yaml/limitations.md`.
//!
//! # Example
//!
//! ```ignore
//! use succinctly::yaml::{YamlIndex, YamlValue};
//!
//! // Block style
//! let yaml = b"name: Alice\nage: 30";
//! let index = YamlIndex::build(yaml)?;
//! let root = index.root(yaml);
//!
//! // Flow style
//! let yaml_flow = b"person: {name: Alice, age: 30}";
//! let index_flow = YamlIndex::build(yaml_flow)?;
//!
//! // Anchor and alias
//! let yaml_anchor = b"default: &def value\nref: *def";
//! let index_anchor = YamlIndex::build(yaml_anchor)?;
//! ```
//!
//! # Architecture
//!
//! YAML parsing uses an oracle + index model:
//!
//! 1. **Oracle** (sequential): Resolves YAML's context-sensitive grammar,
//!    tracks indentation/flow context, and emits IB/BP/TY bits.
//!
//! 2. **Semi-Index** (O(1) queries): Once built, navigation uses only the
//!    BP tree structure without re-parsing.
//!
//! The oracle handles block style (indentation-based), flow style
//! (bracket-based like JSON), anchors, aliases, and block scalars uniformly.

mod advance_positions;
mod end_positions;
mod error;
mod index;
mod light;
mod line_break;
mod locate;
mod parser;
mod scalar;
pub mod simd;
pub mod validate;

/// Does `next` terminate a `-` sequence-entry indicator? That is whitespace, a line
/// break, or end of input.
///
/// The one definition of the terminator set. It lives at the module root because the
/// parser (which sees the next byte as it scans) and the reader (which indexes into the
/// text) both need it, and #332 was caused by each site spelling it out separately —
/// five copies with three different acceptance sets.
#[inline]
pub(crate) fn is_seq_indicator_next(next: Option<u8>) -> bool {
    matches!(next, Some(b' ' | b'\t' | b'\n' | b'\r') | None)
}

/// Does `text` at `pos` begin with the block sequence-entry indicator?
///
/// A `-` followed by anything outside [`is_seq_indicator_next`] (`-1`, `-`, `[-]`) starts
/// a plain scalar instead. A trailing `-` *is* an indicator (see #106).
#[inline]
pub(crate) fn starts_seq_entry(text: &[u8], pos: usize) -> bool {
    text.get(pos) == Some(&b'-') && is_seq_indicator_next(text.get(pos + 1).copied())
}

/// Is the sequence-entry indicator at `pos` followed by content on the *same* line?
///
/// Deliberately narrower than [`starts_seq_entry`]: a bare `-` — line break or end of
/// input after it — is excluded. `YamlElements::uncons_cursor` uses this so that a bare
/// `-` keeps yielding the wrapper node rather than its child, because callers that use the
/// returned cursor *positionally* depend on it: `corpus_stats` counts bare-dash items by
/// the cursor's text position, and `is_yaml_cursor_container` decides block-vs-inline YAML
/// layout from it.
///
/// Value decoding is unaffected either way — `value()` applies the wider
/// [`starts_seq_entry`] and unwraps a bare-`-` wrapper itself.
#[inline]
pub(crate) fn starts_inline_seq_entry(text: &[u8], pos: usize) -> bool {
    text.get(pos) == Some(&b'-') && matches!(text.get(pos + 1), Some(b' ' | b'\t'))
}

/// Is the line at `from` *structural* — a block sequence entry (`- ` …) or a block
/// mapping entry (a `: ` value indicator before end of line) — rather than a plain
/// scalar or a flow node?
///
/// The one definition of "a leading tab here is illegal indentation". YAML forbids a
/// tab in indentation, but a tab is only *indentation* when block structure follows:
/// before a plain scalar it is separation and legal (`DK95/00` `foo:\n \tbar`, `UV7Q`
/// `x:\n - x\n  \tx`), while before a key or a `-` it is not (`DK95/06`). The loader
/// ([`parser::Parser::parse_document_line`]) and the strict validator
/// ([`validate::Validator::scan_line`]) must classify a line identically, so they share
/// this one spelling — #106 and #332 were both a predicate copied to several sites and
/// then diverging. `tests/yaml_tab_indentation_tests.rs` pins the two call sites
/// against each other (#173).
///
/// Leading spaces and tabs are skipped first, so `from` may point at either.
///
/// A flow node is deliberately *not* structural: a root flow node's leading separation
/// may contain tabs (`Q5MG` `\t{}`, `6CA3` `\t[…]`), and a `:` inside the collection is
/// flow syntax rather than a block value indicator, so scanning on would misread
/// `\t{a: 1}` as block structure while `\t{}` reads as a node.
///
/// The `:` must be a *value indicator*, so the scan skips what cannot hold one: a
/// quoted scalar (`\t"x: y"` is a node, `\t"b": 1` is a key) and a comment
/// (`\t# c: d`). Both quoted-scalar forms use [`quoted_span_end`] to find the true
/// close, however many lines it takes — but a scalar that runs past the line break
/// is a node whatever follows it, so *this* scan stops there rather than resuming
/// after the close. That is the deliberate difference from
/// [`validate::Validator::line_kind`], which asks a related question about a whole
/// block-context line: a value on a later line does not change whether *this* line
/// opened a node, so it resumes scanning after the close instead. The two used to
/// hand-roll independent quote scans (#382); now they share `quoted_span_end` and
/// differ only in what each does with its answer.
#[inline]
pub(crate) fn line_is_structural(text: &[u8], from: usize) -> bool {
    let mut i = from;
    while matches!(text.get(i), Some(b' ' | b'\t')) {
        i += 1;
    }
    // A flow node, not block structure — see the doc comment.
    if matches!(text.get(i), Some(b'{' | b'[')) {
        return false;
    }
    if starts_seq_entry(text, i) {
        return true;
    }
    // A `"`, `'` or `#` is only itself at a node position — at the start of the
    // content or after separation. Mid-scalar they are ordinary content, and the
    // `:` after them is a real indicator (`foo#bar: baz`, `foo"bar: baz`).
    let content_start = i;
    // A block mapping entry: a `:` value indicator somewhere on this line.
    while let Some(&b) = text.get(i) {
        match b {
            b'\n' | b'\r' => return false,
            // Everything past a comment is comment text, so no indicator follows.
            b'#' if after_separation(text, content_start, i) => return false,
            b'"' | b'\'' if after_separation(text, content_start, i) => {
                match quoted_span_end(text, i) {
                    QuotedSpanEnd::ClosedSameLine(end) => i = end,
                    // Crosses the line break (or never closes): a multi-line
                    // scalar, hence a node whatever follows it.
                    QuotedSpanEnd::ClosedAcrossLines(_) | QuotedSpanEnd::Unterminated => {
                        return false
                    }
                }
            }
            b':' if matches!(text.get(i + 1), None | Some(b' ' | b'\t' | b'\n' | b'\r')) => {
                return true
            }
            _ => i += 1,
        }
    }
    false
}

/// Is the `"`, `'` or `#` byte at `i` a real delimiter — at `content_start` or right
/// after a space/tab — rather than glued to preceding scalar content (`foo"bar`,
/// `foo#bar`)? Mid-scalar, these bytes are ordinary content and a `:` after them is a
/// real value indicator.
///
/// Shared by [`line_is_structural`] and [`validate::Validator::line_kind`] — #382
/// found `line_kind`'s quote arms missing this gating (its comment arm already had
/// its own copy of the same rule).
#[inline]
pub(crate) fn after_separation(text: &[u8], content_start: usize, i: usize) -> bool {
    i == content_start || matches!(text.get(i - 1), Some(b' ' | b'\t'))
}

/// Where the quoted scalar opening at `pos` (on its `"`/`'` byte) truly closes,
/// following it across as many raw line breaks as it takes — the flow-scalar
/// line-folding rule shared by both quote kinds.
///
/// This only locates the close; it does not validate escape sequences or
/// continuation-line indentation (see
/// [`validate::Validator::scan_double_quoted`]/[`validate::Validator::scan_single_quoted`]
/// for that, on the separate content-validation pass — this function does not
/// replace them and is not used by them).
///
/// [`line_is_structural`] and [`validate::Validator::line_kind`] used to hand-roll
/// three independent versions of this scan between them (one single-line-only, two
/// more inline and line-break-blind); this is the one definition, shared (#382).
pub(crate) fn quoted_span_end(text: &[u8], pos: usize) -> QuotedSpanEnd {
    let quote = text[pos];
    let mut i = pos + 1;
    let mut crossed_line = false;
    while let Some(&c) = text.get(i) {
        match c {
            b'\n' | b'\r' => {
                i += line_break::line_break_len(text, i);
                crossed_line = true;
            }
            // Inside `"…"`, `\` escapes the next byte — and a `\` before the line
            // break is a line continuation: it still folds the scalar onward.
            b'\\' if quote == b'"' => match text.get(i + 1) {
                None => return QuotedSpanEnd::Unterminated,
                Some(b'\n' | b'\r') => {
                    i += 1 + line_break::line_break_len(text, i + 1);
                    crossed_line = true;
                }
                Some(_) => i += 2,
            },
            // Inside `'…'`, `''` is an escaped quote rather than the close.
            _ if c == quote => {
                if quote == b'\'' && text.get(i + 1) == Some(&b'\'') {
                    i += 2;
                } else {
                    let end = i + 1;
                    return if crossed_line {
                        QuotedSpanEnd::ClosedAcrossLines(end)
                    } else {
                        QuotedSpanEnd::ClosedSameLine(end)
                    };
                }
            }
            _ => i += 1,
        }
    }
    QuotedSpanEnd::Unterminated
}

/// The result of [`quoted_span_end`]: where a quoted scalar closes, and whether it
/// crossed a raw line break to get there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QuotedSpanEnd {
    /// Closed on the same physical line it opened on; the offset just past the
    /// closing quote.
    ClosedSameLine(usize),
    /// Closed after crossing at least one raw line break; the offset just past the
    /// closing quote.
    ClosedAcrossLines(usize),
    /// No closing quote before the end of input.
    Unterminated,
}

pub use error::YamlError;
pub use index::YamlIndex;
pub use light::{
    format_float_with_fraction, format_float_yq, format_float_yq_yaml, format_float_yq_yaml_nested,
    stream_json_sequence, stream_yaml_sequence, yq_float_is_scientific, ChompingIndicator,
    YamlCursor, YamlElements, YamlField, YamlFields, YamlNumber, YamlString, YamlValue,
};
pub use locate::{locate_offset, locate_offset_detailed, LocateResult};
pub use scalar::{resolve_plain, resolve_tagged, ResolvedScalar};

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the classification that decides whether a leading tab is illegal
    /// indentation. Each row names the YAML Test Suite case it stands for, so the
    /// rule cannot be widened without someone re-deciding a named case (#173).
    #[test]
    fn line_is_structural_classifies_block_structure_only() {
        for (text, want) in [
            // Block structure: the tab before it is indentation, and illegal.
            (&b"\tb: 2\n"[..], true),
            (&b"\t- a\n"[..], true),
            (&b"\tkey:\n"[..], true), // `:` at end of line is still an indicator
            (&b"  \tb: 2\n"[..], true), // leading spaces are skipped
            // Plain scalars: the tab is separation, and legal.
            (&b"\tbar\n"[..], false), // DK95/00 `foo:\n \tbar`
            (&b"\tx\n"[..], false),   // UV7Q `x:\n - x\n  \tx`
            (&b"\ta:b\n"[..], false), // `:` not followed by whitespace is content
            (&b"\t-1\n"[..], false),  // Y79Y/010: `-1` is a scalar, not an entry
            (&b"\tbar"[..], false),   // end of input, no line break
            // Flow nodes: a root flow node's separation may contain tabs, and a `:`
            // inside the collection is flow syntax, not a block value indicator.
            (&b"\t{}\n"[..], false),     // Q5MG
            (&b"\t{a: 1}\n"[..], false), // same node, now with a `:` inside
            (&b"\t[\n"[..], false),      // 6CA3
            (&b"\t[a: 1]\n"[..], false),
            // Quoted scalars: the `:` inside one is content, so the line is a node —
            // but a quoted *key* is still a mapping entry.
            (&b"\t\"x: y\"\n"[..], false),
            (&b"\t'x: y'\n"[..], false),
            (&b"\t\"b\": 1\n"[..], true),
            (&b"\t'b': 1\n"[..], true),
            (&b"\t\"a\\\": b\"\n"[..], false), // `\"` is an escaped quote, not the close
            (&b"\t'a'': b'\n"[..], false),     // `''` likewise
            (&b"\t&an \"x: y\"\n"[..], false), // a quote after separation still opens
            (&b"\t\"x: y\n"[..], false),       // unterminated: a multi-line scalar
            (&b"\t\"x\\\n"[..], false),        // `\` continues it onto the next line
            (&b"\tfoo\"bar: baz\n"[..], true), // mid-scalar `\"` is content, `: ` is real
            // Comments: everything past one is comment text.
            (&b"\t# c: d\n"[..], false),
            (&b"\tfoo # c: d\n"[..], false),
            (&b"\tfoo#bar: baz\n"[..], true), // `#` glued to a scalar is content
            // Nothing at all.
            (&b"\t\n"[..], false),
            (&b"\t"[..], false),
        ] {
            assert_eq!(
                line_is_structural(text, 0),
                want,
                "line_is_structural({text:?})"
            );
        }
    }

    /// `from` may point at either a space or a tab, and the scan starts there —
    /// the loader passes the position of the tab, the validator the position of
    /// the first non-space.
    #[test]
    fn line_is_structural_starts_scanning_at_the_given_offset() {
        let text = b"a: 1\n  \tb foo\n";
        assert!(line_is_structural(text, 0)); // line 1: `a: 1`
                                              // Line 2 is a plain scalar, so every offset within it agrees — and the scan
                                              // must stop at the line break rather than running on into line 1's `:`.
        assert!(!line_is_structural(text, 5)); // start of line 2's indentation
        assert!(!line_is_structural(text, 7)); // the tab itself
        assert!(!line_is_structural(text, 8)); // the content
    }

    /// `quoted_span_end` follows a quoted scalar across as many raw line breaks as
    /// it takes to find its true close. `line_is_structural` only wants same-line
    /// closes and treats both `ClosedAcrossLines` and `Unterminated` as "a node";
    /// `line_kind` (validate.rs) uses the full cross-line answer.
    #[test]
    fn quoted_span_end_follows_a_scalar_across_lines() {
        use QuotedSpanEnd::*;
        assert_eq!(quoted_span_end(b"\"ab\" rest", 0), ClosedSameLine(4));
        assert_eq!(quoted_span_end(b"'ab' rest", 0), ClosedSameLine(4));
        assert_eq!(quoted_span_end(b"\"a\\\"b\" rest", 0), ClosedSameLine(6)); // `\"` inside
        assert_eq!(quoted_span_end(b"'a''b' rest", 0), ClosedSameLine(6)); // `''` inside
        assert_eq!(quoted_span_end(b"\"ab\nc\"", 0), ClosedAcrossLines(6)); // crosses a break
        assert_eq!(quoted_span_end(b"'ab\nc'", 0), ClosedAcrossLines(6));
        assert_eq!(quoted_span_end(b"\"ab\r\nc\"", 0), ClosedAcrossLines(7)); // CRLF counted as width 2
        assert_eq!(quoted_span_end(b"\"ab\\\nc\"", 0), ClosedAcrossLines(7)); // escaped break still folds
        assert_eq!(quoted_span_end(b"\"ab", 0), Unterminated); // end of input
        assert_eq!(quoted_span_end(b"\"ab\\", 0), Unterminated); // trailing `\`
        assert_eq!(quoted_span_end(b"\"ab\nc", 0), Unterminated); // crosses a line, never closes
    }
}
