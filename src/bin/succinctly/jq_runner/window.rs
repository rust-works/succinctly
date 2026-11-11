//! A lone regular file read a window at a time for the plain `succinctly jq` route (#6920).
//!
//! The plain route (`jq -c .`, `jq .name`, `jq 'select(...)'`: no `-n`, nothing that reads `input`)
//! used to read its file whole, split every document into a span (16 bytes a record) and run the
//! program over borrowed slices of that one buffer, so its peak memory grew with the file where jq's
//! stays at a few MiB. [`WindowedFile`] hands the same loop the same documents, a window of them at a
//! time, and drops the text it has run before reading on.
//!
//! What makes a window safe to drop and the loop unchanged:
//!
//! - **A window ends at a line break** (or at the end of the file). Every document a window gives
//!   ends before that break, so the read that completes it is in the window: jq's line for a document
//!   is the count through the end of that read ([`super::LineCounter::read_end_line`]), and a number
//!   or literal is always followed by the byte that completes it.
//! - **What is kept starts at a line start**, so the next window's counter starts at line start 0 and
//!   only has to be told how many line breaks were dropped ([`Window::lines_before`]).
//! - **A document that does not scan is not yet a fault.** Short of the end of the file the text
//!   after it may be what it lacks, so it is kept, the file read on (twice as much each time, so
//!   a document of any size is found in a linear amount of reading) and the scan tried again. A fault
//!   is reported only when it is one with every remaining byte available, which is the answer the
//!   whole-file route gives; a faulty file is read whole, as before.
//! - **Invalid UTF-8 is substituted a window at a time.** A window is a whole number of lines and a
//!   JSON string holds no line break, so the substitution of one window is the substitution of the
//!   file's lines in it.
//!
//! The route's fault wording, which jq's parser makes over the whole stream, is not made here: the
//! caller reads the file again for it ([`super::reread_stream_fault`]), on the error path only.

use std::fs::File;
use std::io::{self, Read};

use super::count_newlines;

/// How much of the file one window reads, and so about how much memory the route holds besides a
/// document that is bigger than that.
const WINDOW_BYTES: usize = 1 << 20;

/// The smallest file the route reads a window at a time: below this it is not worth the second
/// route, and the file is read whole as before.
const MIN_WINDOWED_FILE_BYTES: u64 = 4 * WINDOW_BYTES as u64;

pub(super) fn window_bytes() -> usize {
    WINDOW_BYTES
}

pub(super) fn min_windowed_file_bytes() -> u64 {
    MIN_WINDOWED_FILE_BYTES
}

/// A document span within a window's text: `(start, end)`, `end` exclusive.
pub(super) type Span = (usize, usize);

/// What splits a text into documents: [`super::split_json_values`], which also gives the offset of
/// the first byte no document starts at. A parameter so the reader is tested against it.
pub(super) type Splitter = fn(&[u8]) -> (Vec<Span>, Option<usize>);

/// One window: the text, and the documents of it the caller has not run yet.
pub(super) struct Window<'a> {
    /// The text from the start of the line the first unrun document is on, to a line break (or to
    /// the end of the file for the last window). Valid UTF-8: invalid bytes are already substituted.
    pub raw: &'a [u8],
    /// The documents to run, as offsets into `raw`, each complete.
    pub values: Vec<Span>,
    /// Where the splitter stopped at a fault, an offset into `raw`: only the last window has one,
    /// and after `values` it is what jq's parser reports.
    pub split_error: Option<usize>,
    /// How many line breaks came before `raw` in the file.
    pub lines_before: usize,
    /// How many bytes came before `raw` in the file (a BOM that was skipped included).
    #[cfg(test)]
    pub offset: usize,
    /// No window follows this one.
    #[cfg(test)]
    pub last: bool,
    /// An invalid byte was substituted in this window or an earlier one.
    pub substituted: bool,
}

/// A regular file read in windows; see the module's doc.
pub(super) struct WindowedFile {
    file: File,
    split: Splitter,
    /// How much a read adds.
    block: usize,
    /// The text: `buf[..validated]` is valid UTF-8 ending at a line break (or the end of the file),
    /// and what follows it is bytes read and not yet checked, with no line break in them.
    buf: Vec<u8>,
    validated: usize,
    /// Where the first document not yet run can start in `buf`.
    skip: usize,
    /// The end of the last document the last window handed out (or `skip` when it gave none):
    /// what the next call may drop up to the start of its line.
    consumed: usize,
    /// Line breaks and bytes dropped from the front of `buf` so far.
    lines_before: usize,
    offset: usize,
    eof: bool,
    started: bool,
    /// An invalid byte was substituted in some window.
    substituted: bool,
    done: bool,
}

impl WindowedFile {
    pub(super) fn new(file: File, block: usize, split: Splitter) -> Self {
        Self {
            file,
            split,
            block: block.max(1),
            buf: Vec::new(),
            validated: 0,
            skip: 0,
            consumed: 0,
            lines_before: 0,
            offset: 0,
            eof: false,
            started: false,
            substituted: false,
            done: false,
        }
    }

    /// Whether an invalid byte of any window so far was substituted: jq's parser counted the bytes
    /// as read, so a position that names a column is then worded from the file read again.
    #[cfg(test)]
    pub(super) fn substituted(&self) -> bool {
        self.substituted
    }

    /// The next window, or `None` when the file is done.
    pub(super) fn next_window(&mut self) -> io::Result<Option<Window<'_>>> {
        if self.done {
            return Ok(None);
        }
        self.drop_consumed();
        let mut grow = self.block;
        loop {
            if !self.eof {
                self.read_more(grow)?;
            }
            self.validate_lines();
            let from = self.skip;
            let (mut values, mut split_error) = (self.split)(&self.buf[from..self.validated]);
            for span in &mut values {
                span.0 += from;
                span.1 += from;
            }
            split_error = split_error.map(|at| at + from);
            // The end of the file settles it: what the splitter made of the text is the answer.
            // Before it, a document that does not scan may only lack its text, and the last
            // document may be a number or literal that a fault's own byte would complete, so it
            // waits to be seen with what follows it.
            if !self.eof && split_error.is_some() {
                values.pop();
            }
            if self.eof || !values.is_empty() {
                let last = self.eof;
                if last {
                    self.done = true;
                }
                self.consumed = values.last().map_or(self.skip, |&(_, end)| end);
                return Ok(Some(Window {
                    raw: &self.buf[..self.validated],
                    values,
                    split_error: split_error.filter(|_| last),
                    lines_before: self.lines_before,
                    #[cfg(test)]
                    offset: self.offset,
                    #[cfg(test)]
                    last,
                    substituted: self.substituted,
                }));
            }
            // Nothing to run, and the file goes on: white space is dropped, and a document that did
            // not scan or a line that is not over is read on, each time twice what is held.
            if split_error.is_none() {
                self.consumed = self.validated;
                self.drop_consumed();
            } else {
                grow = grow.saturating_mul(2).max(self.buf.len());
            }
        }
    }

    /// Drops what the last window's documents ran over, up to the start of the line that holds
    /// their end, and counts the line breaks it held.
    fn drop_consumed(&mut self) {
        let consumed = self.consumed.min(self.buf.len());
        let line_start = self.buf[..consumed]
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |at| at + 1);
        if line_start > 0 {
            self.lines_before += count_newlines(&self.buf[..line_start]);
            self.offset += line_start;
            self.buf.drain(..line_start);
            self.validated -= line_start;
        }
        self.skip = consumed - line_start;
        self.consumed = self.skip;
    }

    /// Reads up to `n` more bytes; a short read is the end of the file.
    fn read_more(&mut self, n: usize) -> io::Result<()> {
        self.buf.reserve(n);
        let read = self
            .file
            .by_ref()
            .take(n as u64)
            .read_to_end(&mut self.buf)?;
        if read < n {
            self.eof = true;
        }
        // jq's parser skips a BOM at the start of its input (#5291), as the whole-file route does;
        // with fewer than three bytes read it cannot be told from the start of another character.
        if !self.started && (self.buf.len() >= 3 || self.eof) {
            self.started = true;
            let bom = crate::front_matter::leading_bom_len(&self.buf);
            if bom > 0 {
                self.buf.drain(..bom);
                self.offset += bom;
            }
        }
        Ok(())
    }

    /// Moves `validated` to the last line break read, or to the end of the file once it is read,
    /// substituting any invalid UTF-8 on the way as the whole-file route does.
    fn validate_lines(&mut self) {
        let end = if self.eof {
            self.buf.len()
        } else {
            match self.buf[self.validated..].iter().rposition(|&b| b == b'\n') {
                Some(at) => self.validated + at + 1,
                None => return,
            }
        };
        if end == self.validated {
            return;
        }
        let region = &self.buf[self.validated..end];
        if succinctly::text::utf8::validate_utf8(region).is_ok() {
            self.validated = end;
            return;
        }
        let (text, _edits) =
            succinctly::jq::utf8_document::substitute_invalid_utf8_jq_document_mapped(region);
        let text = text.into_bytes();
        let new_end = self.validated + text.len();
        self.buf.splice(self.validated..end, text);
        self.validated = new_end;
        self.substituted = true;
    }
}

#[cfg(test)]
mod tests {
    use super::super::{split_json_values, LineCounter};
    use super::*;
    use std::io::Write;

    /// What the whole-file route holds of a text: its documents (absolute), the fault, and each
    /// document's line.
    struct Whole {
        spans: Vec<Span>,
        fault: Option<usize>,
        lines: Vec<usize>,
    }

    fn whole(text: &[u8]) -> Whole {
        let (spans, fault) = split_json_values(text);
        let mut counter = LineCounter::new(text);
        let lines = spans
            .iter()
            .map(|&(_, end)| counter.read_end_line(end))
            .collect();
        Whole {
            spans,
            fault,
            lines,
        }
    }

    /// The same, as a [`WindowedFile`] of `block` bytes a read hands it, in absolute terms.
    fn windowed(text: &[u8], block: usize) -> (Whole, usize) {
        let mut file = tempfile::tempfile().expect("a temp file");
        file.write_all(text).expect("written");
        use std::io::Seek;
        file.rewind().expect("rewound");
        let mut reader = WindowedFile::new(file, block, split_json_values);
        let mut out = Whole {
            spans: Vec::new(),
            fault: None,
            lines: Vec::new(),
        };
        let mut windows = 0;
        let mut peak = 0;
        while let Some(window) = reader.next_window().expect("reads") {
            windows += 1;
            peak = peak.max(window.raw.len());
            let mut counter = LineCounter::new(window.raw);
            counter.set_lines_before(window.lines_before);
            for &(start, end) in &window.values {
                out.spans.push((window.offset + start, window.offset + end));
                out.lines.push(counter.read_end_line(end));
            }
            if let Some(at) = window.split_error {
                assert!(window.last, "a fault is only reported by the last window");
                out.fault = Some(window.offset + at);
            }
            assert!(
                window.last || window.raw.last() == Some(&b'\n'),
                "a window that is not the last ends at a line break"
            );
        }
        assert!(windows >= 1);
        (out, peak)
    }

    fn assert_same(text: &[u8], block: usize) {
        let want = whole(text);
        let (got, _) = windowed(text, block);
        let shown = String::from_utf8_lossy(text);
        assert_eq!(
            got.spans, want.spans,
            "documents of {shown:?} at block {block}"
        );
        assert_eq!(got.fault, want.fault, "fault of {shown:?} at block {block}");
        assert_eq!(got.lines, want.lines, "lines of {shown:?} at block {block}");
    }

    const BLOCKS: &[usize] = &[1, 2, 3, 5, 8, 13, 64, 4096];

    #[test]
    fn windows_give_the_whole_files_documents_and_lines() {
        let texts: &[&[u8]] = &[
            b"{\"a\":1}\n{\"a\":2}\n{\"a\":3}\n",
            b"{\"a\":1}\n{\"a\":2}\n{\"a\":3}",
            b"1\n2\n3\n",
            b"1 2 3\n4 5\n6",
            b"[1,\n2,\n3]\n{\"a\":\n1}\n\"x\"\n",
            b"\n\n  \n{\"a\":1}\n\n\n{\"a\":2}\n  \n",
            b"{\"a\":1}{\"a\":2}\n{\"a\":3} {\"a\":4}\n",
            b"true\nfalse\nnull\n12.5e3\n-0\n",
            b"\"a\"\n\"b\"\n\"c\"",
            b"{\"deep\":[[[[1,2],[3]],[4]],5]}\n[]\n{}\n",
            b"",
            b"\n",
            b"   ",
        ];
        for text in texts {
            for &block in BLOCKS {
                assert_same(text, block);
            }
        }
    }

    #[test]
    fn a_fault_is_reported_as_the_whole_file_reports_it() {
        let texts: &[&[u8]] = &[
            b"{\"a\":1}\n{\"a\":}\n{\"a\":3}\n",
            b"1\n2\n}\n3\n",
            b"1\n2]\n3\n",
            b"1\n2 }\n\n\n",
            b"{\"a\":1}\n[1,2,\n",
            b"{\"a\":1}\n{\"a\":1 xyz}\n{\"b\":2}\n",
            b"{\"a\":1}\n\"unterminated\n{\"b\":2}\n",
            b"{\"a\":1}\n[1,\n2,\n",
            b"nul\n",
            b"12 13x\n",
            b"{\"a\":1}\n\xef\xbb\xbf{\"a\":2}\n",
            b"{\"a\":1}\n[[[[[[\n",
        ];
        for text in texts {
            for &block in BLOCKS {
                assert_same(text, block);
            }
        }
    }

    #[test]
    fn a_leading_bom_is_skipped_and_a_later_one_is_not() {
        let mut text = b"\xef\xbb\xbf".to_vec();
        text.extend_from_slice(b"{\"a\":1}\n{\"a\":2}\n");
        let want = whole(&text[3..]);
        for &block in BLOCKS {
            let (got, _) = windowed(&text, block);
            let shifted: Vec<Span> = want.spans.iter().map(|&(s, e)| (s + 3, e + 3)).collect();
            assert_eq!(got.spans, shifted, "block {block}");
            assert_eq!(got.lines, want.lines, "block {block}");
        }
    }

    #[test]
    fn invalid_utf8_is_substituted_as_the_whole_file_is() {
        // The substitution is per line, so windows agree with it whole.
        let text: &[u8] =
            b"{\"a\":\"x\xffy\"}\n{\"a\":\"\xc3\xa9\"}\n{\"a\":\"\xe2\x82\"}\n{\"b\":1}\n";
        let (want_text, _) =
            succinctly::jq::utf8_document::substitute_invalid_utf8_jq_document_mapped(text);
        let want = whole(want_text.as_bytes());
        for &block in BLOCKS {
            let mut file = tempfile::tempfile().expect("a temp file");
            file.write_all(text).expect("written");
            use std::io::Seek;
            file.rewind().expect("rewound");
            let mut reader = WindowedFile::new(file, block, split_json_values);
            let mut spans = Vec::new();
            let mut raw_all = Vec::new();
            while let Some(window) = reader.next_window().expect("reads") {
                for &(s, e) in &window.values {
                    spans.push((window.offset + s, window.offset + e));
                    raw_all.push(window.raw[s..e].to_vec());
                }
            }
            assert!(reader.substituted(), "block {block}");
            assert_eq!(spans, want.spans, "block {block}");
            let documents: Vec<&[u8]> = want
                .spans
                .iter()
                .map(|&(s, e)| &want_text.as_bytes()[s..e])
                .collect();
            assert_eq!(
                raw_all.iter().map(Vec::as_slice).collect::<Vec<_>>(),
                documents,
                "block {block}"
            );
        }
    }

    #[test]
    fn a_long_line_and_a_big_document_are_found_whole() {
        let line = format!(
            "[{}]\n",
            (0..5000)
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        let mut text = line.clone().into_bytes();
        text.extend_from_slice(b"{\"a\":1}\n");
        text.extend_from_slice(line.as_bytes());
        for &block in &[1usize, 7, 64, 1000, 8192] {
            assert_same(&text, block);
        }
        // One line holding many documents.
        let many: String = (0..2000)
            .map(|n| format!("{{\"n\":{n}}}"))
            .collect::<Vec<_>>()
            .join(" ");
        for &block in &[1usize, 7, 64, 1000] {
            assert_same(many.as_bytes(), block);
        }
    }

    #[test]
    fn a_window_holds_a_bounded_part_of_the_file() {
        let text: String = (0..20_000).map(|n| format!("{{\"n\":{n}}}\n")).collect();
        let (got, peak) = windowed(text.as_bytes(), 4096);
        assert_eq!(got.spans.len(), 20_000);
        assert!(
            peak < 4096 * 3,
            "the largest window ({peak} bytes) stays near the block, not the {} byte file",
            text.len()
        );
    }

    #[test]
    fn a_stream_of_many_shapes_agrees_at_every_block() {
        // A fixed pseudo-random mix of documents, blank lines and white space, with and without a
        // final line break, and with one fault at each position the generator is asked for.
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for round in 0..40 {
            let mut text = String::new();
            for _ in 0..(5 + next() % 40) {
                match next() % 9 {
                    0 => text.push_str("{\"a\":[1,2,{\"b\":null}]}\n"),
                    1 => text.push_str("[1,\n 2,\n 3]\n"),
                    2 => text.push_str("\"str\" 12 true\n"),
                    3 => text.push_str("\n\n"),
                    4 => text.push_str("   {} []  \n"),
                    5 => text.push_str("-1.5e-3\n"),
                    6 => text.push_str("{\"k\":\"v\"}{\"k\":\"w\"}\n"),
                    7 => text.push_str("null\n"),
                    _ => text.push_str("{\"deep\":{\"x\":[[]]}}\n"),
                }
            }
            if round % 4 == 1 {
                text.pop();
            }
            if round % 5 == 2 {
                text.push_str("}\n");
            }
            if round % 7 == 3 {
                text.push_str("[1,\n");
            }
            for &block in &[1usize, 3, 7, 16, 100, 1 << 20] {
                assert_same(text.as_bytes(), block);
            }
        }
    }
}
