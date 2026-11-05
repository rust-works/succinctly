//! yq's `--header-preprocess` glues the line after a leading `--- <text>` marker line onto the
//! text of the marker line (#6276).
//!
//! yq's YAML decoder (`processReadStream`, `pkg/yqlib/decoder_yaml.go`, v4.53.3) reads the head
//! of its input line by line before go-yaml sees it. It sets aside every line that is blank, a
//! `#` comment, a `%YAML` directive or a bare `---` marker, up to the first line that is none of
//! those. A marker with text after it on the same line (`--- x`) is split: the marker and the
//! blanks after it go to the set-aside header and the remainder is pushed back onto the reader,
//! **without the line's own line break**. The remainder therefore continues straight into the
//! next line:
//!
//! ```text
//! --- x          is read as   x y        (the break between x and y is gone, so "xy")
//! y
//! ```
//!
//! The pushed-back text is read as a line again, so it can be a comment (`--- # c⏎x` reads the
//! comment `# cx`, and the document is the line after it), another marker (`--- --- x`, which
//! eats a second break) or the start of an ordinary line (`--- x⏎--- y` is the scalar
//! `x--- y`, and `--- a: 1⏎b: 2` is the refused `a: 1b: 2`).
//!
//! `succinctly yq` indexes the text it is given, so this port keeps the first marker and cuts
//! out what yq's stream no longer holds: the break yq loses (`--- x⏎y` is indexed as `--- xy⏎`;
//! a `\n`, or a `\r\n` as a pair), the blanks in front of the marker (`␠␠--- x⏎y` is `xy` in
//! yq, a scalar `--- xy` if left), and every later marker the pushed-back text starts with.
//! [`glue_after_leading_marker_text`] returns the bytes with those spans cut out.
//!
//! What it leaves alone, because yq's own loop does:
//!
//! - a marker after the first content line (`a: 1⏎--- x⏎y` is `x y`), since the loop has
//!   stopped by then;
//! - a bare `---` line, which keeps its break;
//! - a lone `\r` break, since yq splits its lines on `\n` only (`--- x␍y` is `x y`).

/// Go's `\s` in `regexp` (RE2, ASCII only): space, tab, line feed, form feed, carriage return.
fn is_re2_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0c | b'\r')
}

/// Whether a line that starts with `first` can be one yq's loop sets aside (white space, `#`,
/// `%YAML`, `---`, or a non-ASCII byte, which may start Unicode white space such as U+00A0).
/// Every other first byte makes the line content, and the loop ends there, so an input that
/// starts on one needs neither a scan nor a copy of its first line. The one definition of the
/// early exit in [`glue_after_leading_marker_text`] and of its test.
fn may_start_header_line(first: u8) -> bool {
    is_re2_space(first) || matches!(first, b'#' | b'%' | b'-' | 0x0b) || first >= 0x80
}

/// What yq's loop does with one line (newline already removed).
enum Line {
    /// `^\s*---\s*$` on the trimmed line: set aside whole, keeps its break.
    Separator,
    /// `^\s*---\s+` on the line: `ws` bytes of blanks, the marker, then blanks up to `end`, where
    /// the remainder starts. The remainder is pushed back without a break.
    MarkerWithText { ws: usize, end: usize },
    /// A comment, a `%YAML` directive or a blank line: set aside whole.
    Header,
    /// Anything else: the head of the input ends here.
    Content,
}

fn classify(line: &[u8]) -> Line {
    // Go trims by Unicode white space, which is `char::is_whitespace`. Once per line.
    let text = String::from_utf8_lossy(line);
    let trimmed = text.trim();
    if trimmed == "---" {
        return Line::Separator;
    }
    let mut ws = 0;
    while line.get(ws).copied().is_some_and(is_re2_space) {
        ws += 1;
    }
    if line[ws..].starts_with(b"---") && line.get(ws + 3).copied().is_some_and(is_re2_space) {
        let mut end = ws + 3;
        while line.get(end).copied().is_some_and(is_re2_space) {
            end += 1;
        }
        return Line::MarkerWithText { ws, end };
    }
    if line[ws..].starts_with(b"#") || line[ws..].starts_with(b"%YAML") || trimmed.is_empty() {
        return Line::Header;
    }
    Line::Content
}

/// The spans of `text` that yq's stream no longer holds, in no particular order and possibly
/// overlapping: the lost line breaks (a `\n` or a `\r\n`), the blanks before the first marker,
/// and the later markers of a pushed-back line.
fn cuts(text: &[u8]) -> Vec<std::ops::Range<usize>> {
    let mut cuts = Vec::new();
    // The unread input is `pending` (whose bytes sit at `pending_at` in `text`, not always
    // contiguously: a cut break lies between two pieces) followed by `text[pos..]`.
    let mut pos = 0;
    let mut pending: Vec<u8> = Vec::new();
    let mut pending_at: Vec<usize> = Vec::new();
    let mut kept_marker = false;
    loop {
        let rest = &text[pos..];
        if pending.is_empty() && rest.is_empty() {
            break;
        }
        // `ReadString('\n')` on the unread input.
        let (physical, break_at) = match rest.iter().position(|&b| b == b'\n') {
            Some(nl) => (&rest[..nl], Some(pos + nl)),
            None => (rest, None),
        };
        let merged = !pending.is_empty();
        let mut line = std::mem::take(&mut pending);
        let mut at = std::mem::take(&mut pending_at);
        line.extend_from_slice(physical);
        at.extend(pos..pos + physical.len());
        // The `\r` of a `\r\n` is the last byte of the physical part (the pending part never ends
        // in one: a remainder is cut from a line whose `\r\n` was already taken off).
        let break_start = if break_at.is_some() && line.last() == Some(&b'\r') {
            line.pop();
            at.pop();
            break_at.map(|nl| nl - 1)
        } else {
            break_at
        };
        pos += physical.len() + usize::from(break_at.is_some());
        match classify(&line) {
            Line::Separator | Line::Header => {}
            Line::MarkerWithText { ws, end } => {
                // A break with nothing after it glues nothing on: it stays, as the bytes read
                // the same either way.
                if let (Some(start), Some(nl)) = (break_start, break_at) {
                    if nl + 1 < text.len() {
                        cuts.push(start..nl + 1);
                    }
                }
                if merged || kept_marker {
                    // A marker the pushed-back text starts with: yq strips it too.
                    cuts.push(at[0]..at[end - 1] + 1);
                } else {
                    // The first marker stays; the blanks before it are not in yq's stream.
                    kept_marker = true;
                    if ws > 0 {
                        cuts.push(at[0]..at[ws - 1] + 1);
                    }
                }
                pending = line.split_off(end);
                pending_at = at.split_off(end);
            }
            Line::Content => break,
        }
    }
    cuts
}

/// `text` with what yq's `--header-preprocess` drops after a leading `--- <text>` line cut out
/// (#6276), or `None` when it drops nothing, which is every input that does not begin with
/// such a marker line.
pub(super) fn glue_after_leading_marker_text(text: &[u8]) -> Option<Vec<u8>> {
    // The overwhelmingly common input starts on its content (`a: 1`, `{`, `[`, `"`): one byte
    // settles it.
    if !text.first().copied().is_some_and(may_start_header_line) {
        return None;
    }
    let mut cuts = cuts(text);
    if cuts.is_empty() {
        return None;
    }
    cuts.sort_by_key(|span| span.start);
    let mut out = Vec::with_capacity(text.len());
    let mut from = 0;
    for span in cuts {
        // Spans can overlap (a marker straddling a cut break); `from` only moves forward.
        if span.start > from {
            out.extend_from_slice(&text[from..span.start]);
        }
        from = from.max(span.end);
    }
    out.extend_from_slice(&text[from..]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glued(text: &str) -> String {
        match glue_after_leading_marker_text(text.as_bytes()) {
            Some(bytes) => String::from_utf8(bytes).unwrap(),
            None => text.to_owned(),
        }
    }

    #[test]
    fn marker_text_joins_the_next_line() {
        assert_eq!(glued("--- x\ny\n"), "--- xy\n");
        assert_eq!(glued("--- x\n\ty\n"), "--- x\ty\n");
        assert_eq!(glued("--- x\n  y\n"), "--- x  y\n");
        assert_eq!(glued("--- x\n\ny\n"), "--- x\ny\n");
        assert_eq!(glued("--- [a, x\ny]\n"), "--- [a, xy]\n");
    }

    #[test]
    fn crlf_is_cut_as_a_pair() {
        assert_eq!(glued("--- x\r\ny\r\n"), "--- xy\r\n");
    }

    #[test]
    fn a_lone_cr_is_not_a_break_to_yq() {
        assert_eq!(glued("--- x\ry\r"), "--- x\ry\r");
    }

    #[test]
    fn a_comment_remainder_swallows_the_next_line_into_the_comment() {
        assert_eq!(glued("--- # c\nx\ny\n"), "--- # cx\ny\n");
        assert_eq!(glued("--- # c\na: 1\nb: 2\n"), "--- # ca: 1\nb: 2\n");
    }

    #[test]
    fn a_header_before_the_marker_does_not_stop_the_loop() {
        assert_eq!(glued("# c\n--- x\ny\n"), "# c\n--- xy\n");
        assert_eq!(glued("\n--- x\ny\n"), "\n--- xy\n");
        assert_eq!(glued("%YAML 1.2\n--- x\ny\n"), "%YAML 1.2\n--- xy\n");
        assert_eq!(glued("---\n--- x\ny\n"), "---\n--- xy\n");
    }

    #[test]
    fn a_remainder_that_is_a_marker_is_stripped_and_loses_the_next_break_too() {
        assert_eq!(glued("--- --- x\ny\nz\n"), "--- xyz\n");
        // The second marker's blanks come from the next line.
        assert_eq!(glued("--- ---\n x\ny\n"), "--- xy\n");
    }

    #[test]
    fn blanks_before_the_first_marker_are_not_in_yqs_stream() {
        assert_eq!(glued("  --- x\ny\n"), "--- xy\n");
        assert_eq!(glued("\t--- x\n  y\n"), "--- x  y\n");
        assert_eq!(glued("# c\n  --- x\ny\n"), "# c\n--- xy\n");
    }

    #[test]
    fn a_glued_line_that_is_not_header_ends_the_loop() {
        assert_eq!(glued("--- x\n--- y\nz\n"), "--- x--- y\nz\n");
        assert_eq!(glued("--- x\ny\n--- z\nw\n"), "--- xy\n--- z\nw\n");
    }

    #[test]
    fn inputs_without_a_leading_marker_text_are_untouched() {
        for text in [
            "",
            "x\ny\n",
            "---\nx\ny\n",
            "--- \nx\ny\n",
            "a: 1\n--- x\ny\n",
            "---x\ny\n",
            "# c\na: 1\n--- x\ny\n",
            "--- x",
            "--- x\n",
        ] {
            assert_eq!(glued(text), text, "{text:?}");
        }
    }

    /// The first-byte early exit must never skip an input the loop would cut from: after a first
    /// byte it declines, the first line is content, so the marker line behind it is never read.
    #[test]
    fn the_first_byte_early_exit_agrees_with_the_loop() {
        for first in 0u8..=255 {
            let text = [&[first, b'\n'][..], b"--- x\ny\n"].concat();
            if !may_start_header_line(first) {
                assert!(cuts(&text).is_empty(), "first byte {first:#x}");
            }
        }
        // A header line that is not a marker, then a marker line: the loop still finds it.
        assert_eq!(glued("%YAML 1.2\n--- x\ny\n"), "%YAML 1.2\n--- xy\n");
        assert_eq!(glued("\u{a0}\n--- x\ny\n"), "\u{a0}\n--- xy\n");
        assert_eq!(glued("\u{b}\n--- x\ny\n"), "\u{b}\n--- xy\n");
    }

    #[test]
    fn a_marker_text_without_a_following_line_loses_nothing() {
        assert!(glue_after_leading_marker_text(b"--- x").is_none());
        assert!(glue_after_leading_marker_text(b"--- x\n").is_none());
        assert!(glue_after_leading_marker_text(b"--- x\r\n").is_none());
    }
}
