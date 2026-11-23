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

/// The length of `bytes` when they are the start of a UTF-8 byte-order mark and no more of it
/// (`EF` or `EF BB`), or 0 for anything else (#7454).
///
/// jq's parser keeps its place in the mark between buffers, so a stream that ends inside the mark
/// has no text and no fault: it is empty input (`printf '\357' | jq .` prints nothing, exit 0). A
/// mark that breaks off before the end is `Malformed BOM`, which this is not.
pub(crate) fn truncated_bom_len(bytes: &[u8]) -> usize {
    if !bytes.is_empty() && bytes.len() < UTF8_BOM.len() && UTF8_BOM.starts_with(bytes) {
        bytes.len()
    } else {
        0
    }
}

/// How many bytes of each of `sources` (a stream's sources in order, each whole or at least its
/// first three bytes) are the UTF-8 byte-order mark jq's parser skips at the front of the stream
/// (#5291), when the mark may be cut across sources (#7566).
///
/// jq reads its files as one stream and its parser matches the mark a byte at a time
/// (`bom_strip_position`), whichever buffer each byte comes in, so `EF BB` in one file and
/// `BF [2]` in the next is a mark and `[2]`, and so is a byte of it in each of three files. A
/// stream that ends inside the mark is empty input (#7454): the bytes it has are all cut. A mark
/// that departs from itself (`EF` then `1`) cuts nothing, so the parser meets its bytes and faults.
pub(crate) fn stream_bom_cuts<'a>(sources: impl IntoIterator<Item = &'a [u8]>) -> Vec<usize> {
    let mut cuts = Vec::new();
    let mut matched = 0;
    for source in sources {
        let mut cut = 0;
        if matched < UTF8_BOM.len() {
            for &byte in source {
                if byte != UTF8_BOM[matched] {
                    // No mark, or one that breaks off: the parser reads every byte.
                    return Vec::new();
                }
                matched += 1;
                cut += 1;
                if matched == UTF8_BOM.len() {
                    break;
                }
            }
        }
        cuts.push(cut);
    }
    cuts
}

/// jq's parser matching the UTF-8 byte-order mark at the front of a stream, fed one source at a time
/// (#7851): it says in which source the mark breaks off, the source jq's parser faults in with
/// `Malformed BOM` (`jv_parser_set_buf`). jq's main loop stops at that fault, so its reader never
/// opens a file after that source, and a `(at FILE:0)` names that source.
#[derive(Debug, Default)]
pub(crate) struct StreamBomMatch {
    /// How many of the mark's bytes the stream has matched so far.
    matched: usize,
    /// Whether the stream has settled it: the whole mark, no mark, or one that broke off.
    settled: bool,
}

impl StreamBomMatch {
    /// Feed the stream's next source; `true` when the mark breaks off in it: the stream started
    /// with part of the mark and this source holds the first byte that departs from it. A source
    /// with no bytes (an empty file, a directory, a file that did not open) changes nothing.
    pub(crate) fn breaks_in(&mut self, source: &[u8]) -> bool {
        if self.settled {
            return false;
        }
        for &byte in source {
            if byte != UTF8_BOM[self.matched] {
                self.settled = true;
                return self.matched > 0;
            }
            self.matched += 1;
            if self.matched == UTF8_BOM.len() {
                self.settled = true;
                return false;
            }
        }
        false
    }
}

/// The cut [`stream_bom_cuts`] gives source `at`, 0 past the sources it covers.
pub(crate) fn bom_cut_at(cuts: &[usize], at: usize) -> usize {
    cuts.get(at).copied().unwrap_or(0)
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

    #[test]
    fn truncated_bom_len_is_the_whole_of_a_mark_that_stops_short_7454() {
        assert_eq!(truncated_bom_len(b"\xEF"), 1);
        assert_eq!(truncated_bom_len(b"\xEF\xBB"), 2);
        for other in [
            &b""[..],
            b"\xEF\xBB\xBF",
            b"\xEF\xBF",
            b"\xBB",
            b"\xEF\n",
            b"{",
        ] {
            assert_eq!(truncated_bom_len(other), 0, "{other:?}");
        }
    }

    /// #7566: the mark is cut wherever its bytes fall, across sources with no bytes too; a stream
    /// that ends inside it is all cut, and one that departs from it is not cut at all.
    #[test]
    fn stream_bom_cuts_follows_the_mark_across_sources_7566() {
        let cuts = |sources: &[&[u8]]| stream_bom_cuts(sources.iter().copied());
        assert_eq!(cuts(&[b"\xEF\xBB\xBF[2]"]), vec![3]);
        assert_eq!(cuts(&[b"\xEF\xBB", b"\xBF[2]"]), vec![2, 1]);
        assert_eq!(cuts(&[b"\xEF", b"\xBB\xBF[2]"]), vec![1, 2]);
        assert_eq!(cuts(&[b"\xEF", b"\xBB", b"\xBF[2]"]), vec![1, 1, 1]);
        assert_eq!(
            cuts(&[b"", b"\xEF", b"", b"\xBB\xBF", b"\xEF\xBB\xBF"]),
            vec![0, 1, 0, 2, 0]
        );
        // The stream ends inside the mark: empty input (#7454).
        assert_eq!(cuts(&[b"\xEF", b"\xBB"]), vec![1, 1]);
        assert_eq!(cuts(&[b"\xEF", b""]), vec![1, 0]);
        // No mark, or one that departs from itself, in the first source or a later one.
        for sources in [
            &[&b"[2]"[..]][..],
            &[b"\xEF", b"1"],
            &[b"\xEF\xBB", b"\xEF"],
            &[b"\xEF\xBB", b"\xBB\xBF"],
            &[b"\xBB\xBF"],
            &[b"", b""],
        ] {
            assert!(
                cuts(sources).iter().all(|&cut| cut == 0),
                "{sources:?}: {:?}",
                cuts(sources)
            );
        }
        assert_eq!(bom_cut_at(&[], 4), 0);
        assert_eq!(bom_cut_at(&[1, 2], 1), 2);
    }

    /// #7851: the source the mark breaks off in is the one holding the first byte that departs from
    /// it, past sources with no bytes; a whole mark, no mark and a stream that ends inside the mark
    /// break off nowhere.
    #[test]
    fn stream_bom_match_names_the_source_the_mark_breaks_off_in_7851() {
        let breaks = |sources: &[&[u8]]| {
            let mut mark = StreamBomMatch::default();
            sources
                .iter()
                .map(|source| mark.breaks_in(source))
                .collect::<Vec<_>>()
        };
        assert_eq!(breaks(&[b"\xEF", b"\xEF", b"[1]"]), [false, true, false]);
        assert_eq!(breaks(&[b"\xEF", b"", b"\xBB1"]), [false, false, true]);
        assert_eq!(breaks(&[b"\xEF\xBB", b"\xEF"]), [false, true]);
        assert_eq!(breaks(&[b"", b"\xEF1", b"\xEF"]), [false, true, false]);
        assert_eq!(breaks(&[b"\xEF\xBB\x41"]), [true]);
        for sources in [
            &[&b"[2]"[..], b"\xEF"][..],
            &[b"\xEF\xBB\xBF[1]", b"\xEF"],
            &[b"\xEF", b"\xBB", b"\xBF", b"\xEF"],
            &[b"\xEF", b"\xBB"],
            &[b"", b""],
            &[b"\xBB\xBF"],
        ] {
            assert!(breaks(sources).iter().all(|&broke| !broke), "{sources:?}");
        }
    }

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
