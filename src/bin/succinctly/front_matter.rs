//! Front matter extraction for `yq --front-matter=<extract|process>`.
//!
//! Splits an input into a leading `---`-fenced YAML block and the trailing
//! body content, matching real yq's front-matter convention (e.g. Markdown
//! files with a YAML header).

use succinctly::text::line_break::line_break_len;

/// The result of splitting front matter from an input.
#[derive(Debug)]
pub struct FrontMatter<'a> {
    /// The YAML text before the closing line, or all of it when nothing closes it: the opening
    /// `---` line is part of it, since yq reads it as the header of the stream it evaluates, and
    /// writes it back as it was when the result is that document.
    pub yaml: &'a [u8],
    /// The closing line and everything after it (the closing line is the body's own first line, which
    /// `--front-matter=process` writes back as it was), or `None` when no line closes the front
    /// matter and there is no body.
    pub body: Option<&'a [u8]>,
}

/// A leading UTF-8 BOM, if present, sits before the `---` fence rather than
/// inside it -- common in Markdown files saved by Windows editors.
///
/// `pub(crate)` rather than private: `jq_runner.rs`'s own `--seq` "no RS
/// byte anywhere" diagnostic (#1525) strips the same 3 bytes before its
/// own line/column count, matching real jq's identical BOM-stripping
/// there.
pub(crate) const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

/// The length of the UTF-8 byte-order mark at the front of `bytes`, or 0 if there is none (#5356).
pub(crate) fn leading_bom_len(bytes: &[u8]) -> usize {
    if bytes.starts_with(UTF8_BOM) {
        UTF8_BOM.len()
    } else {
        0
    }
}

/// Split `input` into its YAML front matter and trailing body, as yq v4.53.3 does (#6910).
///
/// A first line that is `---` (blanks and a `\r` around it ignored, and a leading UTF-8 BOM
/// skipped) opens the front matter. The front matter ends at the next line that
/// starts with `---` at column 0 (`---`, `---  `, `--- # c`, `---x`, `----`), which is the body's first
/// line; an indented `---` and a `...` close nothing. A first line that is not a fence is the front
/// matter's own (`# c` / `---` / `a: 1` is the front matter `# c`), and an input with no closing line is
/// all front matter and has no body, so there is no input this refuses.
pub fn split_front_matter(input: &[u8]) -> FrontMatter<'_> {
    let input = input.strip_prefix(UTF8_BOM).unwrap_or(input);
    let mut lines = LineScanner::new(input);
    // The first line is the front matter's own, whatever it is: it opens the front matter when it is
    // a fence, and closes nothing when it starts with `---` and is not one.
    lines.next();
    for line in lines {
        if line.content.starts_with(b"---") {
            return FrontMatter {
                yaml: &input[..line.start],
                body: Some(&input[line.start..]),
            };
        }
    }
    FrontMatter {
        yaml: input,
        body: None,
    }
}

/// The line-ending convention `body` uses, detected from its first line
/// terminator. `--front-matter=process` reattaches `body` byte-for-byte, so
/// a fence line injected right before it (the closing `---`) must match --
/// otherwise a CRLF body ends up preceded by a mismatched fence, producing a
/// file with mixed line endings. Falls back to `\n` for a body with no line
/// break to sniff (nothing to mismatch).
pub(crate) fn body_line_ending(body: &[u8]) -> &'static [u8] {
    for (i, &b) in body.iter().enumerate() {
        match line_break_len(body, i) {
            2 => return b"\r\n",
            1 => return if b == b'\r' { b"\r" } else { b"\n" },
            _ => {}
        }
    }
    b"\n"
}

struct Line<'a> {
    /// Line content, excluding the line terminator.
    content: &'a [u8],
    /// Offset of `content`'s first byte.
    start: usize,
}

struct LineScanner<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> LineScanner<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self { input, pos: 0 }
    }
}

impl<'a> Iterator for LineScanner<'a> {
    type Item = Line<'a>;

    fn next(&mut self) -> Option<Line<'a>> {
        if self.pos >= self.input.len() {
            return None;
        }
        let start = self.pos;
        let mut i = start;
        while i < self.input.len() {
            let len = line_break_len(self.input, i);
            if len > 0 {
                self.pos = i + len;
                return Some(Line {
                    content: &self.input[start..i],
                    start,
                });
            }
            i += 1;
        }
        self.pos = self.input.len();
        Some(Line {
            content: &self.input[start..],
            start,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (input, front matter text, body) as yq v4.53.3 splits them (#6910): the front matter is the text
    /// before the closing line and keeps its opening `---`, the body starts with the closing line.
    #[test]
    fn splits_as_yq_does() {
        type Row<'a> = (&'a [u8], &'a [u8], Option<&'a [u8]>);
        let rows: &[Row] = &[
            (
                b"---\ntitle: foo\n---\n# Body\n",
                b"---\ntitle: foo\n",
                Some(b"---\n# Body\n"),
            ),
            // An empty body at the end of the input, with and without the line break.
            (
                b"---\ntitle: foo\n---\n",
                b"---\ntitle: foo\n",
                Some(b"---\n"),
            ),
            (b"---\ntitle: foo\n---", b"---\ntitle: foo\n", Some(b"---")),
            // An empty front matter block.
            (b"---\n---\nBody\n", b"---\n", Some(b"---\nBody\n")),
            // `...` closes nothing: it is part of the front matter, and the input has no body.
            (
                b"---\ntitle: foo\n...\nBody text\n",
                b"---\ntitle: foo\n...\nBody text\n",
                None,
            ),
            // No closing line: all front matter, no body.
            (
                b"---\ntitle: foo\nno closing fence\n",
                b"---\ntitle: foo\nno closing fence\n",
                None,
            ),
            (b"---", b"---", None),
            (b"---\n", b"---\n", None),
            // No opening fence: the first line is the front matter's own.
            (b"title: foo\nbody text\n", b"title: foo\nbody text\n", None),
            (b"", b"", None),
            (
                b"# c\n---\na: 1\n---\nbody\n",
                b"# c\n",
                Some(b"---\na: 1\n---\nbody\n"),
            ),
            (
                b"\n---\na: 1\n---\nbody\n",
                b"\n",
                Some(b"---\na: 1\n---\nbody\n"),
            ),
            // The closing line only has to start with `---` at column 0 and is kept as it was written.
            (
                b" ---\na: 1\n---  \nbody\n",
                b" ---\na: 1\n",
                Some(b"---  \nbody\n"),
            ),
            (
                b"---\na: 1\n--- # c\nbody\n",
                b"---\na: 1\n",
                Some(b"--- # c\nbody\n"),
            ),
            (
                b"---\na: 1\n----\nbody\n",
                b"---\na: 1\n",
                Some(b"----\nbody\n"),
            ),
            // An indented `---` closes nothing.
            (
                b"---\nkey:\n  ---\n---\nBody\n",
                b"---\nkey:\n  ---\n",
                Some(b"---\nBody\n"),
            ),
            // CRLF and `\r`-only line endings.
            (
                b"---\r\ntitle: foo\r\n---\r\nBody\r\n",
                b"---\r\ntitle: foo\r\n",
                Some(b"---\r\nBody\r\n"),
            ),
            (
                b"---\rtitle: foo\r---\rBody\r",
                b"---\rtitle: foo\r",
                Some(b"---\rBody\r"),
            ),
            // A UTF-8 BOM before the fence is dropped.
            (
                b"\xEF\xBB\xBF---\nt: 1\n---\nB\n",
                b"---\nt: 1\n",
                Some(b"---\nB\n"),
            ),
        ];
        for &(input, yaml, body) in rows {
            let fm = split_front_matter(input);
            assert_eq!(
                (fm.yaml, fm.body),
                (yaml, body),
                "{:?}",
                String::from_utf8_lossy(input)
            );
        }
    }

    #[test]
    fn body_line_ending_detects_lf() {
        assert_eq!(body_line_ending(b"line one\nline two\n"), b"\n");
    }

    #[test]
    fn body_line_ending_detects_crlf() {
        assert_eq!(body_line_ending(b"line one\r\nline two\r\n"), b"\r\n");
    }

    #[test]
    fn body_line_ending_detects_bare_cr() {
        assert_eq!(body_line_ending(b"line one\rline two\r"), b"\r");
    }

    #[test]
    fn body_line_ending_defaults_to_lf_with_no_newline() {
        assert_eq!(body_line_ending(b"no newline here"), b"\n");
        assert_eq!(body_line_ending(b""), b"\n");
    }
}
