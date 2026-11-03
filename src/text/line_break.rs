//! The line-break rule, in one place.
//!
//! A line break is spelled three ways: `\n`, a lone `\r`, and `\r\n` as the
//! two-byte spelling of a *single* break. That is YAML 1.2 §5.4's definition,
//! but it is not YAML-specific — [`crate::text::LineIndex`] applies the same
//! rule to JSON and DSV text, and every consumer needs the same two answers:
//! *is this byte a break*, and *how wide is the break here*.
//!
//! Before #341 the YAML module carried roughly twenty open-coded
//! `if b == b'\r' { … if next == b'\n' { … } } else if b == b'\n' { … }` chains,
//! grown while fixing CRLF handling (#324) and folded-scalar folding (#329).
//! They all agreed, but that is exactly the shape #106 warns about — duplicated
//! predicates diverge silently, and the next edge case would have landed in one
//! copy and not the others. #341 collapsed them onto `yaml::line_break`; #228
//! added a fourth consumer outside `yaml`, so the definition moved here and
//! `yaml::line_break` (a private module, not doc-linkable from here) re-exports
//! it. Callers that need a different *shape* adapt around these
//! functions rather than restating the rule.
//!
//! Two exceptions survive, both in YAML and both documented at their sites:
//! `yaml::parser::Parser::skip_line_break` keeps a hand-rolled dispatch for a
//! measured reason and is pinned to [`line_break_len`] by a test, and the SIMD
//! kernels under `yaml::simd` keep their own representation — a
//! `carriage_returns` mask cannot be phrased as a byte predicate — covered by
//! the per-kernel differential tests.

/// Is `b` a line break? That is `\n` or `\r`; `\r\n` is the two-byte spelling
/// of a single break.
///
/// Scans that look only for `\n` run straight past a lone `\r`, which is how a
/// classic-Mac document turned every block scalar into the empty string (#324).
#[inline]
pub(crate) fn is_line_break(b: u8) -> bool {
    matches!(b, b'\n' | b'\r')
}

/// Width in bytes of the line break at `pos`: 2 for `\r\n`, 1 for a lone `\r`
/// or `\n`, 0 if `pos` is not at a break.
///
/// Zero doubles as "not at a break", so `pos += line_break_len(text, pos)` is a
/// safe unconditional advance only when the caller has already established that
/// it is at one; otherwise test the width before stepping.
///
/// `pub`, not `pub(crate)`, so the `src/bin` binary crate can share this rule
/// too (e.g. `front_matter.rs`'s line scanning) instead of re-deriving it.
#[inline]
pub fn line_break_len(text: &[u8], pos: usize) -> usize {
    match text.get(pos) {
        Some(b'\r') if text.get(pos + 1) == Some(&b'\n') => 2,
        Some(b'\r' | b'\n') => 1,
        _ => 0,
    }
}

/// Whether a wholly blank line (empty or inline whitespace only) lies in `text[from..to)`.
///
/// Callers pass `from` at the end of a comment line's text, so the first break scanned is that
/// line's own; `to` is mid-line (a key, a value, just past a `---`, or the start of the next
/// comment), so a trailing partial line is never counted. A line with content in between is not
/// blank: an empty `-` item, a bare `-` whose value is deferred, or the line holding a node's
/// properties (`  !!str`) sits between two comments without being a gap in them, and counting
/// its line break as a blank line sent a block backwards onto `PREV`'s foot (`- 1\n# h\n-\n- 3\n`
/// is `.[2]`'s head in real yq, #1079) or put a stray blank between two joined comment lines
/// (`- # c\n  !!str\n# f\n- 2\n`, #2835). A count of breaks cannot tell those from a real blank
/// line, so the scan tracks whether the line it just finished held content. `\r\n` counts once.
pub(crate) fn blank_line_between(text: &[u8], from: usize, to: usize) -> bool {
    let mut seen_content = true;
    let mut p = from;
    let end = to.min(text.len());
    while p < end {
        let byte = text[p];
        if is_line_break(byte) {
            if !seen_content {
                return true;
            }
            seen_content = false;
            p += line_break_len(text, p);
        } else {
            if !matches!(byte, b' ' | b'\t') {
                seen_content = true;
            }
            p += 1;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_line_break_accepts_both_break_bytes() {
        assert!(is_line_break(b'\n'));
        assert!(is_line_break(b'\r'));
        for b in [b' ', b'\t', b'a', 0x0b, 0x0c] {
            assert!(!is_line_break(b), "{b:#04x} is not a line break");
        }
    }

    #[test]
    fn line_break_len_measures_each_break_form() {
        assert_eq!(
            line_break_len(b"a\r\nb", 1),
            2,
            "CRLF is one two-byte break"
        );
        assert_eq!(line_break_len(b"a\rb", 1), 1, "lone CR");
        assert_eq!(line_break_len(b"a\nb", 1), 1, "LF");
        // A CR at end of input has no LF to pair with.
        assert_eq!(line_break_len(b"a\r", 1), 1);
        // Not at a break, and past the end, both measure zero.
        assert_eq!(line_break_len(b"ab", 0), 0);
        assert_eq!(line_break_len(b"a\n", 9), 0);
        assert_eq!(line_break_len(b"", 0), 0);
    }

    /// Standing on the LF of a CRLF, the break does not *end* there — it ends
    /// one byte later. Callers distinguishing the two (`find_block_content_range`,
    /// `Parser::current_line`) rely on the width being 2, not on a separate
    /// lookahead.
    #[test]
    fn line_break_len_distinguishes_a_break_that_ends_here() {
        // b"a\r\nb\rc\nd"
        //   0 1 2 3 4 5 6 7
        let text = b"a\r\nb\rc\nd";
        assert_eq!(line_break_len(text, 1), 2, "CR of a CRLF: ends at 3, not 2");
        assert_eq!(line_break_len(text, 4), 1, "lone CR ends at 5");
        assert_eq!(line_break_len(text, 6), 1, "LF ends at 7");
    }

    #[test]
    fn blank_line_between_needs_a_blank_line_not_a_line_of_content() {
        // Adjacent comments, and one with a CRLF break between.
        assert!(!blank_line_between(b"# a\n# b\n", 3, 4));
        assert!(!blank_line_between(b"# a\r\n# b\r\n", 3, 5));
        // A blank line, blank-with-spaces, and a CRLF blank line.
        assert!(blank_line_between(b"# a\n\n# b\n", 3, 5));
        assert!(blank_line_between(b"# a\n  \n# b\n", 3, 8));
        assert!(blank_line_between(b"# a\r\n\r\n# b\r\n", 3, 7));
        // A line holding only node properties between two comments is content, not a gap
        // (#2835), and so is a bare `-`.
        assert!(!blank_line_between(b"# a\n  !!str\n# b\n", 3, 12));
        assert!(!blank_line_between(b"# a\r\n  !!str\r\n# b\r\n", 3, 14));
        assert!(!blank_line_between(b"# a\n-\n# b\n", 3, 6));
        assert!(blank_line_between(b"# a\n  !!str\n\n# b\n", 3, 13));
        // `to` falling inside a CRLF pair, and past the end of the text.
        assert!(!blank_line_between(b"# a\r\n# b", 3, 4));
        assert!(blank_line_between(b"# a\n\n", 3, 99));
    }

    /// The two helpers are one rule seen from two angles, and must not drift
    /// apart: a byte is a break iff a break has non-zero width there.
    #[test]
    fn the_two_helpers_agree_over_every_break_form() {
        for text in [
            b"a\r\nb\rc\nd".as_slice(),
            b"\r\n",
            b"\n\r",
            b"\r\r\n",
            b"\n\n",
            b"x",
            b"",
        ] {
            for pos in 0..=text.len() {
                assert_eq!(
                    line_break_len(text, pos) > 0,
                    text.get(pos).copied().is_some_and(is_line_break),
                    "{text:?} @ {pos}: width disagrees with the byte predicate"
                );
            }
        }
    }
}
