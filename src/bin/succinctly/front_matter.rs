//! Front matter extraction for `yq --front-matter=<extract|process>`.
//!
//! Splits an input into a leading `---`-fenced YAML block and the trailing
//! body content, matching real yq's front-matter convention (e.g. Markdown
//! files with a YAML header).

use std::fmt;

/// The result of successfully splitting front matter from an input.
#[derive(Debug)]
pub struct FrontMatter<'a> {
    /// The YAML content between the opening and closing fence (exclusive).
    pub yaml: &'a [u8],
    /// Everything after the closing fence line; empty if the input ends
    /// immediately after the fence.
    pub body: &'a [u8],
}

/// Errors produced when splitting front matter out of an input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontMatterError {
    /// The input doesn't start with a `---` fence line.
    NoFrontMatter,
    /// A `---` fence was found but never closed with a matching `---`/`...`.
    UnterminatedFrontMatter,
}

impl fmt::Display for FrontMatterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoFrontMatter => {
                write!(
                    f,
                    "no front matter found (input must start with a `---` line)"
                )
            }
            Self::UnterminatedFrontMatter => {
                write!(
                    f,
                    "unterminated front matter (missing closing `---` or `...`)"
                )
            }
        }
    }
}

impl std::error::Error for FrontMatterError {}

/// Split `input` into its YAML front matter and trailing body.
///
/// The input must start with a line that is exactly `---` (trailing
/// whitespace/`\r` ignored). The front matter ends at the next line that is
/// exactly `---` or `...` (mirroring YAML's own end-of-document marker).
/// Everything after that line is returned as the body, unparsed.
pub fn split_front_matter(input: &[u8]) -> Result<FrontMatter<'_>, FrontMatterError> {
    let mut lines = LineScanner::new(input);

    let first = lines.next().ok_or(FrontMatterError::NoFrontMatter)?;
    if !is_fence_line(first.content) {
        return Err(FrontMatterError::NoFrontMatter);
    }

    let yaml_start = first.end;
    loop {
        let line = lines
            .next()
            .ok_or(FrontMatterError::UnterminatedFrontMatter)?;
        if is_fence_line(line.content) || is_end_marker(line.content) {
            return Ok(FrontMatter {
                yaml: &input[yaml_start..line.start],
                body: &input[line.end..],
            });
        }
    }
}

/// The line-ending convention `body` uses, detected from its first line
/// terminator. `--front-matter=process` reattaches `body` byte-for-byte, so
/// a fence line injected right before it (the closing `---`) must match --
/// otherwise a CRLF body ends up preceded by a bare-LF fence, producing a
/// file with mixed line endings. Falls back to `\n` for a body with no line
/// break to sniff (nothing to mismatch).
pub(crate) fn body_line_ending(body: &[u8]) -> &'static [u8] {
    match body.iter().position(|&b| b == b'\n') {
        Some(nl) if nl > 0 && body[nl - 1] == b'\r' => b"\r\n",
        _ => b"\n",
    }
}

fn is_fence_line(content: &[u8]) -> bool {
    trim_trailing_ws(content) == b"---"
}

fn is_end_marker(content: &[u8]) -> bool {
    trim_trailing_ws(content) == b"..."
}

fn trim_trailing_ws(content: &[u8]) -> &[u8] {
    let end = content
        .iter()
        .rposition(|b| !matches!(b, b' ' | b'\t' | b'\r'))
        .map_or(0, |i| i + 1);
    &content[..end]
}

struct Line<'a> {
    /// Line content, excluding the line terminator.
    content: &'a [u8],
    /// Offset of `content`'s first byte.
    start: usize,
    /// Offset right after the line terminator (or end of input).
    end: usize,
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
        match self.input[start..].iter().position(|&b| b == b'\n') {
            Some(rel) => {
                let content_end = start + rel;
                let end = content_end + 1;
                self.pos = end;
                Some(Line {
                    content: &self.input[start..content_end],
                    start,
                    end,
                })
            }
            None => {
                self.pos = self.input.len();
                Some(Line {
                    content: &self.input[start..],
                    start,
                    end: self.input.len(),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_leading_fence_errors() {
        let err = split_front_matter(b"title: foo\nbody text\n").unwrap_err();
        assert_eq!(err, FrontMatterError::NoFrontMatter);
    }

    #[test]
    fn empty_input_errors() {
        let err = split_front_matter(b"").unwrap_err();
        assert_eq!(err, FrontMatterError::NoFrontMatter);
    }

    #[test]
    fn basic_dash_terminator() {
        let fm = split_front_matter(b"---\ntitle: foo\n---\n# Body\n").unwrap();
        assert_eq!(fm.yaml, b"title: foo\n");
        assert_eq!(fm.body, b"# Body\n");
    }

    #[test]
    fn dots_terminator() {
        let fm = split_front_matter(b"---\ntitle: foo\n...\nBody text\n").unwrap();
        assert_eq!(fm.yaml, b"title: foo\n");
        assert_eq!(fm.body, b"Body text\n");
    }

    #[test]
    fn empty_body_at_eof() {
        let fm = split_front_matter(b"---\ntitle: foo\n---\n").unwrap();
        assert_eq!(fm.yaml, b"title: foo\n");
        assert_eq!(fm.body, b"");
    }

    #[test]
    fn empty_body_at_eof_no_trailing_newline() {
        let fm = split_front_matter(b"---\ntitle: foo\n---").unwrap();
        assert_eq!(fm.yaml, b"title: foo\n");
        assert_eq!(fm.body, b"");
    }

    #[test]
    fn empty_front_matter_block() {
        let fm = split_front_matter(b"---\n---\nBody\n").unwrap();
        assert_eq!(fm.yaml, b"");
        assert_eq!(fm.body, b"Body\n");
    }

    #[test]
    fn unterminated_errors() {
        let err = split_front_matter(b"---\ntitle: foo\nno closing fence\n").unwrap_err();
        assert_eq!(err, FrontMatterError::UnterminatedFrontMatter);
    }

    #[test]
    fn opening_fence_only_no_newline_is_unterminated() {
        let err = split_front_matter(b"---").unwrap_err();
        assert_eq!(err, FrontMatterError::UnterminatedFrontMatter);
    }

    #[test]
    fn crlf_line_endings() {
        let fm = split_front_matter(b"---\r\ntitle: foo\r\n---\r\nBody\r\n").unwrap();
        assert_eq!(fm.yaml, b"title: foo\r\n");
        assert_eq!(fm.body, b"Body\r\n");
    }

    #[test]
    fn multiple_yaml_lines_preserved_verbatim() {
        let fm = split_front_matter(b"---\ntitle: foo\ntags: [a, b]\n---\nBody\n").unwrap();
        assert_eq!(fm.yaml, b"title: foo\ntags: [a, b]\n");
        assert_eq!(fm.body, b"Body\n");
    }

    #[test]
    fn trailing_whitespace_on_fence_lines_tolerated() {
        let fm = split_front_matter(b"---   \ntitle: foo\n---\t\nBody\n").unwrap();
        assert_eq!(fm.yaml, b"title: foo\n");
        assert_eq!(fm.body, b"Body\n");
    }

    #[test]
    fn indented_dashes_are_not_fences() {
        // YAML document markers must start at column 0; an indented `---`
        // inside the front matter (e.g. a nested key) must not be mistaken
        // for the closing fence.
        let fm = split_front_matter(b"---\nkey:\n  ---\n---\nBody\n").unwrap();
        assert_eq!(fm.yaml, b"key:\n  ---\n");
        assert_eq!(fm.body, b"Body\n");
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
    fn body_line_ending_defaults_to_lf_with_no_newline() {
        assert_eq!(body_line_ending(b"no newline here"), b"\n");
        assert_eq!(body_line_ending(b""), b"\n");
    }
}
