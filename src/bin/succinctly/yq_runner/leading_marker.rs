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
//! `succinctly yq` indexes the text it is given, so this port keeps the marker and removes only
//! the break yq loses: `--- x⏎y` is indexed as `--- xy⏎`. [`glue_after_leading_marker_text`]
//! returns the bytes with those breaks (`\n`, or `\r\n` as a pair) cut out.
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

/// `strings.TrimSpace(line) == ""` or a bare `---` after it: the two lines yq sets aside whole
/// by their trimmed text. Go trims by Unicode white space, which is `char::is_whitespace`.
fn trimmed(line: &[u8]) -> String {
    String::from_utf8_lossy(line).trim().to_owned()
}

/// What yq's loop does with one line (newline already removed).
enum Line {
    /// `^\s*---\s*$` on the trimmed line: set aside whole, keeps its break.
    Separator,
    /// `^\s*---\s+` on the line: the marker goes, the remainder (starting at this offset of the
    /// line) is pushed back without a break.
    MarkerWithText(usize),
    /// A comment, a `%YAML` directive or a blank line: set aside whole.
    Header,
    /// Anything else: the head of the input ends here.
    Content,
}

fn classify(line: &[u8]) -> Line {
    if trimmed(line) == "---" {
        return Line::Separator;
    }
    let mut at = 0;
    while line.get(at).copied().is_some_and(is_re2_space) {
        at += 1;
    }
    if line[at..].starts_with(b"---") && line.get(at + 3).copied().is_some_and(is_re2_space) {
        let mut end = at + 3;
        while line.get(end).copied().is_some_and(is_re2_space) {
            end += 1;
        }
        return Line::MarkerWithText(end);
    }
    if line[at..].starts_with(b"#") || line[at..].starts_with(b"%YAML") || trimmed(line).is_empty()
    {
        return Line::Header;
    }
    Line::Content
}

/// The spans of `text` (a `\n` or a `\r\n`) that yq's header loop loses, in order.
fn lost_breaks(text: &[u8]) -> Vec<std::ops::Range<usize>> {
    let mut lost = Vec::new();
    // The unread input is `pending` followed by `text[pos..]`.
    let mut pos = 0;
    let mut pending: Vec<u8> = Vec::new();
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
        let mut line = std::mem::take(&mut pending);
        line.extend_from_slice(physical);
        // The `\r` of a `\r\n` is the last byte of the physical part (the pending part never ends
        // in one: a remainder is cut from a line whose `\r\n` was already taken off).
        let mut break_start = break_at;
        if break_at.is_some() && line.last() == Some(&b'\r') {
            line.pop();
            break_start = break_at.map(|nl| nl - 1);
        }
        pos += physical.len() + usize::from(break_at.is_some());
        match classify(&line) {
            Line::Separator | Line::Header => {}
            Line::MarkerWithText(end) => {
                // A break with nothing after it glues nothing on: it stays, as the bytes read
                // the same either way.
                if let (Some(start), Some(nl)) = (break_start, break_at) {
                    if nl + 1 < text.len() {
                        lost.push(start..nl + 1);
                    }
                }
                pending = line.split_off(end);
            }
            Line::Content => break,
        }
    }
    lost
}

/// `text` with the line breaks yq's `--header-preprocess` loses after a leading `--- <text>` line
/// cut out (#6276), or `None` when it loses none, which is every input that does not begin with
/// such a marker line.
pub(super) fn glue_after_leading_marker_text(text: &[u8]) -> Option<Vec<u8>> {
    let lost = lost_breaks(text);
    if lost.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(text.len());
    let mut from = 0;
    for span in lost {
        out.extend_from_slice(&text[from..span.start]);
        from = span.end;
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
    fn a_remainder_that_is_a_marker_loses_the_next_break_too() {
        assert_eq!(glued("--- --- x\ny\nz\n"), "--- --- xyz\n");
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

    #[test]
    fn a_marker_text_without_a_following_line_loses_nothing() {
        assert!(glue_after_leading_marker_text(b"--- x").is_none());
        assert!(glue_after_leading_marker_text(b"--- x\n").is_none());
        assert!(glue_after_leading_marker_text(b"--- x\r\n").is_none());
    }
}
