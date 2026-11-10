//! Block scalars that hold a Unicode line break (#6369).
//!
//! go-yaml ends a line at U+0085 (NEL), U+2028 (LS) and U+2029 (PS) as it does at `\n`. Inside a
//! block scalar (`|` or `>`) that decides everything the index and the decoders read a line for:
//! the spaces after the break are indentation (up to the scalar's indent), a text line follows, a
//! dedent ends the scalar, a tab below the indent is refused, and a document marker or a key
//! after it is read as structure. The index keeps the three as scalar content (#4913), so the
//! kernels and the decoders, which look for `\n` and `\r` alone, read such a scalar as one line.
//!
//! The kernels ([`super::simd::find_block_scalar_end`]) run once per block scalar and stay as they
//! are. A scalar is read here instead only when its text holds one of the three breaks, which
//! almost none does: the parser notes one on the line walk it makes anyway (looking at a byte only
//! where a `0xC2` or `0xE2` lead byte shows up) and lists the scalar in the index, so the decoders
//! never scan for one. What this module holds is the slow path for that scalar, and it follows go-yaml's
//! `yaml_parser_scan_block_scalar`, which is where its answers (the end, the value, the refusals)
//! come from:
//!
//! - [`scan_block_end`]: where the scalar ends, or the tab that go-yaml refuses.
//! - [`decode_block_unicode`]: the value, with go-yaml's own `leading_break` and `trailing_breaks`.
//!
//! A break's spelling is kept the way go-yaml's `read_line` keeps it: `\n`, `\r`, `\r\n` and NEL
//! all become `\n` in the value, and LS and PS stay as they were. LS and PS never fold, and the
//! last break of a scalar under `clip` or `keep` is whichever one the text has.

#[cfg(not(test))]
use alloc::{string::String, vec::Vec};

use super::light::{ChompingIndicator, YamlStringError};

/// Width of the NEL (two bytes), LS or PS (three) at `pos`, or 0.
#[inline]
pub(super) fn unicode_break_len(bytes: &[u8], pos: usize) -> usize {
    match bytes.get(pos..) {
        Some([0xC2, 0x85, ..]) => 2,
        Some([0xE2, 0x80, 0xA8 | 0xA9, ..]) => 3,
        _ => 0,
    }
}

/// Width of the line break at `pos` in go-yaml's sense (`\n`, `\r`, `\r\n`, NEL, LS or PS), or 0.
#[inline]
pub(super) fn any_break_len(bytes: &[u8], pos: usize) -> usize {
    match bytes.get(pos) {
        Some(b'\r') if bytes.get(pos + 1) == Some(&b'\n') => 2,
        Some(b'\r' | b'\n') => 1,
        Some(0xC2 | 0xE2) => unicode_break_len(bytes, pos),
        _ => 0,
    }
}

/// Where a block scalar of `indent` spaces that begins at `start` (the first byte of the first
/// line below its header) ends: the start of the first line whose text sits left of `indent`, or
/// `bytes.len()`. A tab right after the spaces of a line that is not yet at `indent` is go-yaml's
/// `found a tab character where an indentation space is expected`, and comes back as `Err` with
/// the tab's offset (#6372, #6287).
///
/// Lines end at every break [`any_break_len`] knows, so the scalar can end in the middle of what
/// is one physical line to a `\n`-only reader (`x<LS>b: 1`: the key starts after the LS).
pub(super) fn scan_block_end(bytes: &[u8], start: usize, indent: usize) -> Result<usize, usize> {
    let mut pos = start;
    loop {
        if pos >= bytes.len() {
            return Ok(bytes.len());
        }
        let line_start = pos;
        // go-yaml eats spaces only up to the indent: spaces past it are the line's text.
        let mut col = 0;
        while col < indent && bytes.get(pos) == Some(&b' ') {
            pos += 1;
            col += 1;
        }
        let Some(&first) = bytes.get(pos) else {
            return Ok(bytes.len());
        };
        let break_len = any_break_len(bytes, pos);
        if break_len > 0 {
            // An empty line, however many spaces it holds up to the indent.
            pos += break_len;
            continue;
        }
        if col < indent {
            if first == b'\t' {
                return Err(pos);
            }
            return Ok(line_start);
        }
        // A text line: to its break.
        while pos < bytes.len() && any_break_len(bytes, pos) == 0 {
            pos += 1;
        }
        pos += any_break_len(bytes, pos);
    }
}

/// Append the break at `pos` (of width `len`) the way go-yaml's `read_line` does: a newline for
/// `\n`, `\r`, `\r\n` and NEL, LS and PS as themselves.
#[inline]
fn push_break(bytes: &[u8], pos: usize, len: usize, into: &mut Vec<u8>) {
    if len == 3 {
        into.extend_from_slice(&bytes[pos..pos + 3]);
    } else {
        into.push(b'\n');
    }
}

/// go-yaml's `yaml_parser_scan_block_scalar_breaks` for a known indent: eat the spaces up to
/// `indent`, and while what follows is a break, take it into `trailing` and eat again. Returns the
/// position of the first thing that is not a break and the column reached (the spaces eaten).
fn eat_breaks(
    bytes: &[u8],
    mut pos: usize,
    indent: usize,
    trailing: &mut Vec<u8>,
) -> (usize, usize) {
    loop {
        let mut col = 0;
        while col < indent && bytes.get(pos) == Some(&b' ') {
            pos += 1;
            col += 1;
        }
        let len = any_break_len(bytes, pos);
        if len == 0 {
            return (pos, col);
        }
        push_break(bytes, pos, len, trailing);
        pos += len;
    }
}

/// The value of a block scalar of `indent` spaces whose first line starts at `start`, with
/// go-yaml's own bookkeeping: `leading_break` is the break that ended the last text line,
/// `trailing_breaks` those of the empty lines since. `folded` is `>`; the three chomping modes
/// keep none, the last text line's break, or that and every empty line after it.
///
/// For text without Unicode breaks this is the value `decode_block_literal` and
/// `decode_block_folded` give (a test pins that), so the difference is only in what a Unicode
/// break does.
pub(super) fn decode_block_unicode(
    bytes: &[u8],
    start: usize,
    indent: usize,
    chomping: ChompingIndicator,
    folded: bool,
) -> Result<String, YamlStringError> {
    let mut out: Vec<u8> = Vec::new();
    let mut leading: Vec<u8> = Vec::new();
    let mut trailing: Vec<u8> = Vec::new();
    let mut leading_blank = false;

    let (mut pos, mut col) = eat_breaks(bytes, start, indent, &mut trailing);
    while pos < bytes.len() && col == indent {
        let trailing_blank = matches!(bytes[pos], b' ' | b'\t');
        if folded && !leading_blank && !trailing_blank && leading.first() == Some(&b'\n') {
            // A single break between two ordinary lines folds to a space; with empty lines
            // between, the empty lines are the breaks.
            if trailing.is_empty() {
                out.push(b' ');
            }
        } else {
            out.extend_from_slice(&leading);
        }
        leading.clear();
        out.extend_from_slice(&trailing);
        trailing.clear();
        leading_blank = trailing_blank;

        while pos < bytes.len() && any_break_len(bytes, pos) == 0 {
            out.push(bytes[pos]);
            pos += 1;
        }
        let len = any_break_len(bytes, pos);
        if len > 0 {
            push_break(bytes, pos, len, &mut leading);
            pos += len;
        }
        (pos, col) = eat_breaks(bytes, pos, indent, &mut trailing);
    }

    if chomping != ChompingIndicator::Strip {
        out.extend_from_slice(&leading);
    }
    if chomping == ChompingIndicator::Keep {
        out.extend_from_slice(&trailing);
    }
    String::from_utf8(out).map_err(|_| YamlStringError::InvalidUtf8)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LS: &str = "\u{2028}";
    const PS: &str = "\u{2029}";
    const NEL: &str = "\u{85}";

    #[test]
    fn break_widths_are_the_three_unicode_breaks_and_the_ascii_ones() {
        for (text, width) in [
            ("\u{85}", 2),
            ("\u{2028}", 3),
            ("\u{2029}", 3),
            ("\u{2014}", 0), // an em dash shares LS's lead bytes
            ("\u{c2a0}", 0), // so does a no-break space share NEL's
            ("a", 0),
        ] {
            assert_eq!(unicode_break_len(text.as_bytes(), 0), width, "{text:?}");
        }
        assert_eq!(any_break_len(b"\r\nx", 0), 2);
        assert_eq!(any_break_len(b"\rx", 0), 1);
        assert_eq!(any_break_len(b"\nx", 0), 1);
        assert_eq!(any_break_len(b"x", 0), 0);
        assert_eq!(any_break_len(b"", 0), 0);
        // A lead byte cut off by the end of the input is no break.
        assert_eq!(unicode_break_len(&[0xE2, 0x80], 0), 0);
        assert_eq!(unicode_break_len(&[0xC2], 0), 0);
    }

    #[test]
    fn scan_block_end_ends_at_a_dedent_after_any_break() {
        let end = |text: String, indent: usize| scan_block_end(text.as_bytes(), 0, indent);
        assert_eq!(end("  x\nb: 1".to_string(), 2), Ok(4), "ascii");
        assert_eq!(end(format!("  x{LS}b: 1"), 2), Ok(6), "LS, then a dedent");
        assert_eq!(end(format!("  x{PS}b: 1"), 2), Ok(6), "PS, then a dedent");
        assert_eq!(end(format!("  x{NEL}b: 1"), 2), Ok(5), "NEL, then a dedent");
        assert_eq!(
            end(format!("  x{LS}  y\nb: 1"), 2),
            Ok(10),
            "LS, then text at the indent"
        );
        // An empty line made of a space and a break does not end it; the next dedent does.
        assert_eq!(
            end(format!("  x\n {PS}  y\nb: 1"), 2),
            Ok(12),
            "an empty line"
        );
        assert_eq!(end(format!("  x{LS}"), 2), Ok(6), "the end of the input");
        assert_eq!(end(format!("  x{LS}\n"), 2), Ok(7), "LS and a newline");
    }

    #[test]
    fn scan_block_end_refuses_a_tab_where_a_space_of_the_indent_is_expected() {
        let text = format!("  x{LS}\ty");
        assert_eq!(scan_block_end(text.as_bytes(), 0, 2), Err(6));
        let text = format!("  x{LS} \ty");
        assert_eq!(scan_block_end(text.as_bytes(), 0, 2), Err(7));
        // At the indent a tab is text.
        let text = format!("  x{LS}  \ty");
        assert!(scan_block_end(text.as_bytes(), 0, 2).is_ok());
    }

    #[test]
    fn decode_keeps_ls_and_ps_and_reads_nel_as_a_newline() {
        use ChompingIndicator::{Clip, Keep, Strip};
        for (what, text, chomping, folded, want) in [
            (
                "LS in a literal",
                format!("  x{LS}  y\n"),
                Clip,
                false,
                format!("x{LS}y\n"),
            ),
            (
                "PS in a literal",
                format!("  x{PS}  y\n"),
                Clip,
                false,
                format!("x{PS}y\n"),
            ),
            (
                "NEL in a literal is a newline",
                format!("  x{NEL}  y\n"),
                Clip,
                false,
                "x\ny\n".to_string(),
            ),
            (
                "LS never folds",
                format!("  x{LS}  y\n  z\n"),
                Clip,
                true,
                format!("x{LS}y z\n"),
            ),
            (
                "NEL folds like a newline",
                format!("  x{NEL}  y\n"),
                Clip,
                true,
                "x y\n".to_string(),
            ),
            (
                "the last break is the LS under clip",
                format!("  x{LS}"),
                Clip,
                false,
                format!("x{LS}"),
            ),
            (
                "and gone under strip",
                format!("  x{LS}"),
                Strip,
                false,
                "x".to_string(),
            ),
            (
                "an LS and a newline are two breaks",
                format!("  x{LS}\n  y\n"),
                Clip,
                false,
                format!("x{LS}\ny\n"),
            ),
            (
                "keep takes the empty lines too",
                format!("  x{LS}\n\n"),
                Keep,
                false,
                format!("x{LS}\n\n"),
            ),
        ] {
            let got = decode_block_unicode(text.as_bytes(), 0, 2, chomping, folded).unwrap();
            assert_eq!(got, want, "{what}");
        }
    }
}
