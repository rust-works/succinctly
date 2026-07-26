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
//! - Explicit keys (`?` / `:`)
//! - Multi-document streams (`---` / `...`), wrapped in an implicit root sequence
//! - Comments (ignored in block context)
//!
//! # Not supported
//!
//! - Tags (`!!str`, `!custom`, verbatim `!<...>`) — rejected in block context, absorbed
//!   as scalar text in flow context
//! - `%YAML` / `%TAG` directives — parsed as plain scalars
//! - Merge keys (`<<`) — parsed as an ordinary key
//!
//! # Validation
//!
//! `YamlIndex::build` performs minimal validation during indexing (structural recognition
//! only) and accepts many malformed documents. It is a non-validating loader: of the YAML
//! Test Suite's 94 invalid documents it rejects 11. Do not rely on a parse error to
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
/// scalar?
///
/// The one definition of "a leading tab here is illegal indentation". YAML forbids a
/// tab in indentation, but a tab is only *indentation* when block structure follows:
/// before a plain scalar it is separation and legal (`DK95/00` `foo:\n \tbar`, `UV7Q`
/// `x:\n - x\n  \tx`), while before a key or a `-` it is not (`DK95/06`).
///
/// It lives at the module root because the strict validator
/// ([`validate::Validator::scan_line`]) and the loader must classify a line
/// identically, and #106 and #332 were both a predicate copied to several sites and
/// then diverging.
///
/// Leading spaces and tabs are skipped first, so `from` may point at either.
#[inline]
pub(crate) fn line_is_structural(text: &[u8], from: usize) -> bool {
    let mut i = from;
    while matches!(text.get(i), Some(b' ' | b'\t')) {
        i += 1;
    }
    if starts_seq_entry(text, i) {
        return true;
    }
    // A block mapping entry: a `:` value indicator somewhere on this line.
    while let Some(&b) = text.get(i) {
        match b {
            b'\n' | b'\r' => return false,
            b':' if matches!(text.get(i + 1), None | Some(b' ' | b'\t' | b'\n' | b'\r')) => {
                return true
            }
            _ => i += 1,
        }
    }
    false
}

pub use error::YamlError;
pub use index::YamlIndex;
pub use light::{
    ChompingIndicator, YamlCursor, YamlElements, YamlField, YamlFields, YamlNumber, YamlString,
    YamlValue,
};
pub use locate::{locate_offset, locate_offset_detailed, LocateResult};
pub use scalar::{resolve_plain, ResolvedScalar};

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
            (&b"\t{}\n"[..], false),  // Q5MG: a flow node, and no `:` at all
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
}
