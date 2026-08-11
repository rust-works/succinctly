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
//! [`crate::yaml::line_break`] re-exports it. Callers that need a different
//! *shape* adapt around these three functions rather than restating the rule.
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

/// Width in bytes of the line break ending immediately *before* `pos`: 2 for
/// `\r\n`, 1 for a lone `\r` or `\n`, 0 if `pos` is not preceded by one.
///
/// The mirror of [`line_break_len`], for backwards scans. Stepping back a fixed
/// one byte lands in the middle of a CRLF, which leaves the `\r` attached to the
/// previous line's text (#324).
///
/// Note this answers "is there break *text* behind me", not "does a break *end*
/// here": standing on the `\n` of a CRLF it reports 1, for the `\r` behind it.
/// Callers asking the latter want `line_break_len(text, pos - 1) == 1`, which is
/// true only when the break starting one byte back also finishes there.
#[inline]
pub(crate) fn line_break_len_before(text: &[u8], pos: usize) -> usize {
    match pos.checked_sub(1).and_then(|i| text.get(i)) {
        Some(b'\n') if pos >= 2 && text[pos - 2] == b'\r' => 2,
        Some(b'\n' | b'\r') => 1,
        _ => 0,
    }
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
    fn line_break_len_before_measures_backwards() {
        // b"a\r\nb\rc\nd"
        //   0 1 2 3 4 5 6 7
        let text = b"a\r\nb\rc\nd";
        assert_eq!(line_break_len_before(text, 3), 2, "CRLF ends before `b`");
        assert_eq!(line_break_len_before(text, 2), 1, "just the CR of a CRLF");
        assert_eq!(line_break_len_before(text, 5), 1, "lone CR ends before `c`");
        assert_eq!(line_break_len_before(text, 7), 1, "LF ends before `d`");
        assert_eq!(line_break_len_before(text, 1), 0, "`a` is not a break");
        assert_eq!(
            line_break_len_before(text, 0),
            0,
            "nothing before the start"
        );
        assert_eq!(line_break_len_before(b"", 0), 0);
        // A bare LF at index 0 is a one-byte break with no CR in front of it.
        assert_eq!(line_break_len_before(b"\nx", 1), 1);
    }

    /// The three helpers are one rule seen from three angles, and must not
    /// drift apart: a byte is a break iff a break has non-zero width there, and
    /// a break of width `n` *starting* at `i` is the break of width `n` ending
    /// at `i + n`.
    ///
    /// The round trip is asserted only where `pos` really starts a break. On
    /// the LF of a CRLF the forward width is 1 (that LF) while the backward
    /// width is 2 (the whole CRLF) — the two helpers answer different
    /// questions there, by design, and `find_block_content_range` depends on
    /// exactly that difference.
    #[test]
    fn the_three_helpers_agree_over_every_break_form() {
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
                let here = line_break_len(text, pos);

                assert_eq!(
                    here > 0,
                    text.get(pos).copied().is_some_and(is_line_break),
                    "{text:?} @ {pos}: width disagrees with the byte predicate"
                );

                let inside_a_crlf = pos > 0 && line_break_len(text, pos - 1) == 2;
                if here > 0 && !inside_a_crlf {
                    let ends_at = pos + here;
                    assert_eq!(
                        line_break_len_before(text, ends_at),
                        here,
                        "{text:?} @ {pos}: a break of width {here} starting here \
                         must be the break of width {here} ending at {ends_at}"
                    );
                }
            }
        }
    }
}
