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
//!   after it may be what it lacks, so it is kept, the file read on (to eight windows, and then to the
//!   end of the file, so the scans that fail to find a document's end cover nine windows at most,
//!   and a document that never ends is read to the end of the file once) and the scan tried again. The
//!   exception is a fault no further byte can change: one at a byte no container or string starts at
//!   (a stray `}`, a bad literal), which sits in a text that ends at a line break and so is whole.
//!   That one is reported at once; the rest are reported at the end of the file, where the splitter's
//!   answer is the whole-file route's.
//! - **Invalid UTF-8 is substituted a window at a time.** A window is a whole number of lines, and a
//!   JSON string holds no line break, so the substitution of one window is the substitution of the
//!   file's lines in it. (A text where a string does hold one, a backslash before a line break, is not
//!   JSON jq reads, and its substitution can differ at a window's edge.)
//! - **A file with few line breaks is not read twice.** Only the bytes a read adds are searched for a
//!   break, and a read that finds none asks for twice as much, so a one-line file is read in a linear
//!   amount of work, as whole, and gains nothing.
//!
//! The route's fault wording, which jq's parser makes over the whole stream, is not made from the
//! window: [`Reread`] reads the file again through a duplicate of the open handle, a chunk at a time
//! and by position, on the error path only. A file renamed, replaced or appended to meanwhile
//! is worded from the bytes the run read.

use std::fs::File;
use std::io::{self, Read};
use std::path::PathBuf;

use super::{count_newlines, split_json_values};

/// How much of the file one window reads, and so about how much memory the route holds besides a
/// document that is bigger than that.
pub(super) const WINDOW_BYTES: usize = 1 << 20;

/// The smallest file the route reads a window at a time: below this it is not worth the second
/// route, and the file is read whole as before.
pub(super) const MIN_WINDOWED_FILE_BYTES: u64 = 4 * WINDOW_BYTES as u64;

/// How many windows a document that does not fit in one is read on to before the rest of the file is.
const LOOKAHEAD_WINDOWS: usize = 8;

/// A document span within a window's text: `(start, end)`, `end` exclusive.
pub(super) type Span = (usize, usize);

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
    /// How much of the file is left to read: its length when it was opened, less what was read. A
    /// file that grows meanwhile is read as it was, as `std::fs::read` read it, so a run whose
    /// output is appended to its input ends.
    remaining: u64,
    /// How much a read adds.
    block: usize,
    /// The text: `buf[..validated]` is valid UTF-8 ending at a line break (or the end of the file),
    /// and what follows it is bytes read and not yet checked, with no line break in them.
    buf: Vec<u8>,
    validated: usize,
    /// Where the bytes after `validated` are known to hold no line break from: they are not searched
    /// again.
    no_break_from: usize,
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
    /// The last window was handed out.
    done: bool,
    /// A read failed: the bytes read before it are run, and the error is returned after them.
    read_error: Option<io::Error>,
    #[cfg(test)]
    break_search_bytes: usize,
    #[cfg(test)]
    bytes_read: u64,
}

impl WindowedFile {
    /// `file` is read from its start, up to `len` bytes.
    pub(super) fn new(file: File, len: u64, block: usize) -> Self {
        Self {
            file,
            remaining: len,
            block: block.max(1),
            buf: Vec::new(),
            validated: 0,
            no_break_from: 0,
            skip: 0,
            consumed: 0,
            lines_before: 0,
            offset: 0,
            eof: false,
            started: false,
            substituted: false,
            done: false,
            read_error: None,
            #[cfg(test)]
            break_search_bytes: 0,
            #[cfg(test)]
            bytes_read: 0,
        }
    }

    /// The next window, or `None` when the file is done. A read that failed is returned here, after
    /// the last window, so the documents read before it are run first.
    pub(super) fn next_window(&mut self) -> io::Result<Option<Window<'_>>> {
        if self.done {
            return match self.read_error.take() {
                Some(error) => Err(error),
                None => Ok(None),
            };
        }
        self.drop_consumed();
        let mut grow = self.block;
        let mut first = true;
        loop {
            if !self.eof {
                self.read_more(grow);
            }
            let before = self.validated;
            self.validate_lines();
            if !self.eof && self.validated == before && !first {
                // No line break came with that read: the next asks for twice as much, so a file with
                // few of them is read in a linear amount of work, and once that is several windows
                // (it is one long document, or a file of one line) for all of what is left, as
                // the whole-file route reads it.
                grow = if grow >= 4 * WINDOW_BYTES {
                    usize::try_from(self.remaining).unwrap_or(usize::MAX)
                } else {
                    grow.saturating_mul(2)
                };
                continue;
            }
            first = false;
            let from = self.skip;
            let (mut values, split_error) = split_json_values(&self.buf[from..self.validated]);
            for span in &mut values {
                span.0 += from;
                span.1 += from;
            }
            let split_error = split_error.map(|at| at + from);
            // A fault before the end of the file is only one when no byte that follows can change
            // it: where no container or string starts, in a text that ends at a line break (a bare
            // token is bounded by it). Otherwise the document may only lack its text.
            let definitive = split_error
                .is_some_and(|at| !self.eof && !matches!(self.buf[at], b'{' | b'[' | b'"'));
            let pending = split_error.is_some() && !self.eof && !definitive;
            // The last document before a fault that may still move is a number or literal whose
            // completion the fault's own byte could be, so it waits to be seen with what follows.
            if pending {
                if let Some(&(start, _)) = values.last() {
                    if !matches!(self.buf[start], b'{' | b'[' | b'"') {
                        values.pop();
                    }
                }
            }
            if self.eof || definitive || !values.is_empty() {
                let last = self.eof || definitive;
                if last {
                    self.done = true;
                }
                self.consumed = values.last().map_or(self.skip, |&(_, end)| end);
                return Ok(Some(Window {
                    raw: &self.buf[..self.validated],
                    values,
                    // A text cut short by a read that failed is no malformed document: the failure
                    // is what the caller reports, after the last window.
                    split_error: split_error.filter(|_| last && self.read_error.is_none()),
                    lines_before: self.lines_before,
                    #[cfg(test)]
                    offset: self.offset,
                    #[cfg(test)]
                    last,
                    substituted: self.substituted,
                }));
            }
            // Nothing to run, and the file goes on: white space is dropped, and a document that did
            // not scan is read on. The scan of an incomplete document is lost, so doubling costs
            // two scans of a document in all: a document bigger than a window is read on to eight
            // windows once, and past that, which is a file that is one document or has a very big
            // one, for the rest of the file, as the whole-file route reads it.
            if pending {
                let lookahead = self.block.saturating_mul(LOOKAHEAD_WINDOWS);
                grow = if self.buf.len() >= lookahead {
                    usize::try_from(self.remaining).unwrap_or(usize::MAX)
                } else {
                    lookahead - self.buf.len()
                };
            } else {
                self.consumed = self.validated;
                self.drop_consumed();
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
            self.no_break_from = self.no_break_from.saturating_sub(line_start);
        }
        self.skip = consumed - line_start;
        self.consumed = self.skip;
    }

    /// Reads up to `n` more bytes; a short read, or the length the file had, is the end of the file,
    /// and an error is kept to be returned after the bytes before it have been run.
    fn read_more(&mut self, n: usize) {
        let n = usize::try_from(self.remaining).map_or(n, |left| n.min(left));
        if n == 0 {
            self.eof = true;
        } else if self.buf.try_reserve(n).is_err() {
            self.read_error = Some(io::Error::from(io::ErrorKind::OutOfMemory));
            self.eof = true;
        } else {
            match self.file.by_ref().take(n as u64).read_to_end(&mut self.buf) {
                Ok(read) => {
                    self.remaining -= read as u64;
                    #[cfg(test)]
                    {
                        self.bytes_read += read as u64;
                    }
                    if read < n || self.remaining == 0 {
                        self.eof = true;
                    }
                }
                Err(error) => {
                    self.read_error = Some(error);
                    self.eof = true;
                }
            }
        }
        // jq's parser skips a BOM at the start of its input (#5291), as the whole-file route does;
        // with fewer than three bytes read it cannot be told from the start of another character.
        if !self.started && (self.buf.len() >= 3 || self.eof) {
            self.started = true;
            let bom = crate::front_matter::leading_bom_len(&self.buf);
            if bom > 0 {
                self.buf.drain(..bom);
                self.offset += bom;
                self.no_break_from = self.no_break_from.saturating_sub(bom);
            }
        }
    }

    /// Moves `validated` to the last line break read, or to the end of the file once it is read,
    /// substituting any invalid UTF-8 on the way as the whole-file route does.
    fn validate_lines(&mut self) {
        let end = if self.eof {
            self.buf.len()
        } else {
            let from = self.validated.max(self.no_break_from);
            #[cfg(test)]
            {
                self.break_search_bytes += self.buf.len() - from;
            }
            match last_line_break(&self.buf[from..]) {
                Some(at) => from + at + 1,
                None => {
                    self.no_break_from = self.buf.len();
                    return;
                }
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
        let text =
            succinctly::jq::utf8_document::substitute_invalid_utf8_jq_document(region).into_bytes();
        let new_end = self.validated + text.len();
        self.buf.splice(self.validated..end, text);
        self.validated = new_end;
        self.substituted = true;
    }
}

/// The index of the last line break in `bytes`. Fixed-width chunks with an OR fold rather than
/// `rposition`, so the search over a long run with none is a vector compare per 64 bytes
/// (see [`super::count_newlines`] for the same shape): a one-line file is searched once, whole.
fn last_line_break(bytes: &[u8]) -> Option<usize> {
    const WIDTH: usize = 64;
    let tail = bytes.len() % WIDTH;
    let body = bytes.len() - tail;
    if let Some(at) = bytes[body..].iter().rposition(|&b| b == b'\n') {
        return Some(body + at);
    }
    let mut end = body;
    while end >= WIDTH {
        let chunk = &bytes[end - WIDTH..end];
        if chunk.iter().fold(0u8, |any, &b| any | u8::from(b == b'\n')) != 0 {
            return chunk
                .iter()
                .rposition(|&b| b == b'\n')
                .map(|at| end - WIDTH + at);
        }
        end -= WIDTH;
    }
    None
}

/// A regular file read as whole lines, a window at a time, for `-R` (#7023). A line is no more than a
/// line break away from its end, so there is nothing to scan and nothing to carry across a window
/// but the bytes after its last line break: the caller turns each window's lines into strings and
/// drops the window, and the memory held is a window and one line, not the file.
pub(super) struct LineWindows {
    file: File,
    /// How much of the file is left to read: its length when it was opened, less what was read.
    remaining: u64,
    /// How much a read adds.
    block: usize,
    /// The bytes read after the last line break of the last window: the front of the next.
    carried: Vec<u8>,
    /// A read failed: the lines before it were handed out, and the failure is reported once.
    failed: bool,
}

impl LineWindows {
    /// `file` is read from its start, up to `len` bytes.
    pub(super) fn new(file: File, len: u64, block: usize) -> Self {
        Self {
            file,
            remaining: len,
            block: block.max(1),
            carried: Vec::new(),
            failed: false,
        }
    }

    /// The next window: bytes that end at a line break, or at the end of the file for the last, whose
    /// final line then has none. `None` after the last. A read that fails ends the file there and is
    /// handed to `on_error`, once, with the lines read before it handed out first.
    pub(super) fn next_window(&mut self, on_error: impl FnOnce(&io::Error)) -> Option<Vec<u8>> {
        let mut bytes = std::mem::take(&mut self.carried);
        let mut ask = self.block;
        let mut on_error = Some(on_error);
        loop {
            let want = usize::try_from(self.remaining).map_or(ask, |left| ask.min(left));
            if want == 0 || self.failed {
                return (!bytes.is_empty()).then_some(bytes);
            }
            let before = bytes.len();
            if bytes.try_reserve(want).is_err() {
                self.failed = true;
                if let Some(report) = on_error.take() {
                    report(&io::Error::from(io::ErrorKind::OutOfMemory));
                }
                continue;
            }
            match self.file.by_ref().take(want as u64).read_to_end(&mut bytes) {
                // A file that shrank meanwhile ends at what it held.
                Ok(read) if read < want => self.remaining = 0,
                Ok(read) => self.remaining -= read as u64,
                Err(error) => {
                    self.failed = true;
                    if let Some(report) = on_error.take() {
                        report(&error);
                    }
                }
            }
            if self.remaining > 0 && !self.failed {
                if let Some(at) = last_line_break(&bytes[before..]) {
                    self.carried = bytes.split_off(before + at + 1);
                    return Some(bytes);
                }
                // No line break in what was read: a long line, read on twice as far.
                ask = ask.saturating_mul(2);
            }
        }
    }
}

/// The file read again, for the wording of a fault: through a duplicate of the handle the run reads
/// by, so what it names is what the run read, whatever happened to the path since (#6920).
pub(super) struct Reread {
    file: File,
    /// How much of the file the run could read: its length when it was opened.
    len: u64,
    /// For a target that cannot read by position.
    #[cfg_attr(unix, allow(dead_code))]
    path: PathBuf,
}

/// How much of the file a chunk of the second read holds.
const REREAD_CHUNK: u64 = 1 << 20;

impl Reread {
    pub(super) fn new(file: &File, len: u64, path: PathBuf) -> io::Result<Self> {
        Ok(Self {
            file: file.try_clone()?,
            len,
            path,
        })
    }

    /// jq's parse error for the stream the file holds, or `None` when jq's parser reads it all or the
    /// file cannot be read again. A BOM is skipped at the front, as the run skips it, and the bytes
    /// are counted as read, an invalid one included, as jq's parser counts them.
    #[cfg(unix)]
    pub(super) fn stream_fault(&self) -> Option<String> {
        use std::os::unix::fs::FileExt;
        let failed = std::cell::Cell::new(false);
        let mut at = 0u64;
        let chunks = std::iter::from_fn(|| {
            if at >= self.len || failed.get() {
                return None;
            }
            let want = (self.len - at).min(REREAD_CHUNK) as usize;
            let mut chunk = vec![0u8; want];
            match self.file.read_at(&mut chunk, at) {
                Ok(0) => None,
                Ok(read) => {
                    chunk.truncate(read);
                    if at == 0 {
                        chunk.drain(..crate::front_matter::leading_bom_len(&chunk));
                    }
                    at += read as u64;
                    Some(chunk)
                }
                Err(_) => {
                    failed.set(true);
                    None
                }
            }
        });
        let fault = succinctly::jq::jq_stream_fault(chunks, 1, 0);
        (!failed.get()).then_some(fault).flatten()
    }

    #[cfg(not(unix))]
    pub(super) fn stream_fault(&self) -> Option<String> {
        let _ = (&self.file, self.len, REREAD_CHUNK);
        super::reread_stream_fault(std::slice::from_ref(&self.path))
    }
}

#[cfg(test)]
mod tests {
    use super::super::LineCounter;
    use super::*;
    use std::io::{Seek, Write};

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

    fn open(text: &[u8], block: usize) -> WindowedFile {
        let mut file = tempfile::tempfile().expect("a temp file");
        file.write_all(text).expect("written");
        file.rewind().expect("rewound");
        WindowedFile::new(file, text.len() as u64, block)
    }

    /// What a [`WindowedFile`] of `block` bytes a read gives of `text`, in the same terms, with the
    /// largest window it held and the reader itself.
    fn windowed(text: &[u8], block: usize) -> (Whole, usize, WindowedFile) {
        let mut reader = open(text, block);
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
        (out, peak, reader)
    }

    fn assert_same(text: &[u8], block: usize) {
        let want = whole(text);
        let (got, _, _) = windowed(text, block);
        let shown = String::from_utf8_lossy(text);
        assert_eq!(
            got.spans, want.spans,
            "documents of {shown:?} at block {block}"
        );
        assert_eq!(got.fault, want.fault, "fault of {shown:?} at block {block}");
        assert_eq!(got.lines, want.lines, "lines of {shown:?} at block {block}");
    }

    const BLOCKS: &[usize] = &[1, 2, 3, 5, 8, 13, 64, 4096];

    /// A read that fails part-way through a document is an I/O failure, not a malformed document:
    /// the cut text goes out without a fault, and the error follows the last window.
    #[test]
    fn a_failed_read_is_no_malformed_document() {
        let mut reader = open(b"{\"a\":1}\n{\"a\":", 1000);
        reader.read_more(1000);
        reader.read_error = Some(io::Error::other("injected"));
        let window = reader.next_window().expect("window").expect("a window");
        assert_eq!(window.values, vec![(0, 7)]);
        assert_eq!(window.split_error, None);
        assert!(reader.next_window().is_err(), "then the read's error");
        assert!(reader.next_window().expect("and then done").is_none());
    }

    #[test]
    fn last_line_break_agrees_with_rposition() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for len in [0usize, 1, 63, 64, 65, 127, 128, 129, 1000] {
            for density in [0u64, 1, 5, 50, 1000] {
                for _ in 0..20 {
                    let bytes: Vec<u8> = (0..len)
                        .map(|_| {
                            if density > 0 && next() % density == 0 {
                                b'\n'
                            } else {
                                b'a' + (next() % 26) as u8
                            }
                        })
                        .collect();
                    assert_eq!(
                        last_line_break(&bytes),
                        bytes.iter().rposition(|&b| b == b'\n'),
                        "{len} bytes, break density 1 in {density}"
                    );
                }
            }
        }
    }

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
            b"{\"s\":\"\xc3\xa9 \\u00e9 \\\"q\\\" \\\\ \xf0\x9f\x98\x80\"}\n\"\xe2\x82\xac\"\n",
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
            b"{\"a\":1}\n}\n{\"b\":2}\n}\n",
            b"1\n2\n3\nxyz\n4\n",
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
            let (got, _, _) = windowed(&text, block);
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
        let want_text = succinctly::jq::utf8_document::substitute_invalid_utf8_jq_document(text);
        let want = whole(want_text.as_bytes());
        for &block in BLOCKS {
            let mut reader = open(text, block);
            let mut spans = Vec::new();
            let mut documents = Vec::new();
            while let Some(window) = reader.next_window().expect("reads") {
                for &(s, e) in &window.values {
                    spans.push((window.offset + s, window.offset + e));
                    documents.push(window.raw[s..e].to_vec());
                }
            }
            assert!(reader.substituted, "block {block}");
            assert_eq!(spans, want.spans, "block {block}");
            let expected: Vec<&[u8]> = want
                .spans
                .iter()
                .map(|&(s, e)| &want_text.as_bytes()[s..e])
                .collect();
            assert_eq!(
                documents.iter().map(Vec::as_slice).collect::<Vec<_>>(),
                expected,
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
        let (got, peak, _) = windowed(text.as_bytes(), 4096);
        assert_eq!(got.spans.len(), 20_000);
        assert!(
            peak < 4096 * 3,
            "the largest window ({peak} bytes) stays near the block, not the {} byte file",
            text.len()
        );
    }

    /// A file with no line break, or only one at the end, is searched for breaks in a linear amount
    /// of work (the first version searched everything read so far after every read, which made a
    /// 100 MB minified document 3.7 times slower than reading it whole).
    #[test]
    fn a_file_with_few_line_breaks_is_searched_once() {
        let one_line = format!(
            "{{\"users\":[{}]}}",
            (0..40_000)
                .map(|n| format!("{{\"id\":{n},\"name\":\"user {n}\"}}"))
                .collect::<Vec<_>>()
                .join(",")
        );
        for text in [one_line.clone(), format!("{one_line}\n")] {
            let (got, _, reader) = windowed(text.as_bytes(), 1024);
            assert_eq!(got.spans.len(), 1);
            assert!(
                reader.break_search_bytes <= 2 * text.len(),
                "{} bytes searched for line breaks in a {} byte file",
                reader.break_search_bytes,
                text.len()
            );
        }
    }

    /// Several documents each bigger than half a window: the reader keeps the one it is finding and
    /// not the ones behind it (the first version held the last complete one back whatever its kind).
    #[test]
    fn documents_bigger_than_half_a_block_do_not_pile_up() {
        let doc = format!("[{}]\n", vec!["12345"; 5000].join(","));
        let text = doc.repeat(12);
        let (got, peak, _) = windowed(text.as_bytes(), doc.len() / 2 + 1);
        assert_eq!(got.spans.len(), 12);
        assert!(
            peak <= doc.len() * 3,
            "the largest window ({peak} bytes) holds about {} documents of {} bytes",
            peak / doc.len(),
            doc.len()
        );
    }

    /// A fault at a byte no container or string starts at is reported where it is, without reading on
    /// to the end of the file (a stray `}` early in a big file used to read all of it).
    #[test]
    fn a_fault_no_later_byte_can_change_is_reported_at_once() {
        let mut text = String::new();
        for n in 0..50 {
            text.push_str(&format!("{{\"n\":{n}}}\n"));
        }
        text.push_str("}\n");
        text.push_str(&"{\"n\":1}\n".repeat(200_000));
        let (got, _, reader) = windowed(text.as_bytes(), 4096);
        assert_eq!(got.spans.len(), 50);
        assert!(got.fault.is_some());
        assert!(
            reader.bytes_read < 64 * 1024,
            "{} bytes read of {} to find a stray bracket on line 51",
            reader.bytes_read,
            text.len()
        );
        // A bare token the fault's own byte completes is left to the caller.
        let (got, _, _) = windowed(b"1\n2\n3}\n4\n", 3);
        let want = whole(b"1\n2\n3}\n4\n");
        assert_eq!((got.spans, got.fault), (want.spans, want.fault));
    }

    /// A file that grows while it is read is read as it was.
    #[test]
    fn a_file_that_grows_is_read_as_it_was() {
        let named = tempfile::NamedTempFile::new().expect("a temp file");
        std::fs::write(named.path(), b"1\n2\n3\n").expect("written");
        let mut reader = WindowedFile::new(File::open(named.path()).expect("opens"), 6, 4);
        let mut appender = std::fs::OpenOptions::new()
            .append(true)
            .open(named.path())
            .expect("opens to append");
        appender.write_all(b"4\n5\n").expect("grown");
        let mut documents = 0;
        while let Some(window) = reader.next_window().expect("reads") {
            documents += window.values.len();
        }
        assert_eq!(documents, 3);
    }

    /// A read that fails is returned after the window the bytes before it make.
    #[cfg(unix)]
    #[test]
    fn a_failed_read_is_reported_after_the_documents_before_it() {
        // A directory opens and fails to read: no bytes, and the error comes after the one window.
        let dir = tempfile::tempdir().expect("tempdir");
        let file = File::open(dir.path()).expect("a directory opens");
        let mut reader = WindowedFile::new(file, 4096, 1024);
        let window = reader
            .next_window()
            .expect("the held bytes first")
            .expect("a window");
        assert_eq!(window.values, Vec::<Span>::new());
        assert!(reader.next_window().is_err(), "then the read's error");
        assert!(reader.next_window().expect("and then done").is_none());
    }

    /// The fault is worded from the file the run read, whatever happened to the path since.
    #[cfg(unix)]
    #[test]
    fn the_fault_is_worded_from_the_handle_not_the_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("log.json");
        std::fs::write(&path, b"{\"a\":1}\n{\"a\":2}\n}\n{\"a\":3}\n").expect("written");
        let file = File::open(&path).expect("opens");
        let len = file.metadata().expect("metadata").len();
        let reread = Reread::new(&file, len, path.clone()).expect("dup");
        let want = "Unmatched '}' at line 3, column 1";
        assert_eq!(reread.stream_fault().as_deref(), Some(want));
        // Rotated: another file at the path, and the old one deleted.
        let other = dir.path().join("other.json");
        std::fs::write(&other, b"1\n2\n3\n").expect("written");
        std::fs::rename(&other, &path).expect("replaced");
        assert_eq!(reread.stream_fault().as_deref(), Some(want));
        // A BOM at the front is skipped, as the run skips it.
        let bom = dir.path().join("bom.json");
        std::fs::write(&bom, b"\xef\xbb\xbf1\n}\n").expect("written");
        let file = File::open(&bom).expect("opens");
        let reread = Reread::new(&file, 7, bom).expect("dup");
        assert_eq!(
            reread.stream_fault().as_deref(),
            Some("Unmatched '}' at line 2, column 1")
        );
    }

    #[test]
    fn a_stream_of_many_shapes_agrees_at_every_block() {
        // A fixed pseudo-random mix of documents, blank lines and white space, with and without a
        // final line break, and with a fault added to some, over ASCII, multi-byte text and escapes.
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for round in 0..60 {
            let mut text = String::new();
            for _ in 0..(5 + next() % 40) {
                match next() % 12 {
                    0 => text.push_str("{\"a\":[1,2,{\"b\":null}]}\n"),
                    1 => text.push_str("[1,\n 2,\n 3]\n"),
                    2 => text.push_str("\"str\" 12 true\n"),
                    3 => text.push_str("\n\n"),
                    4 => text.push_str("   {} []  \n"),
                    5 => text.push_str("-1.5e-3\n"),
                    6 => text.push_str("{\"k\":\"v\"}{\"k\":\"w\"}\n"),
                    7 => text.push_str("null\n"),
                    8 => text.push_str("{\"s\":\"caf\u{e9} \u{20ac} \u{1f600}\"}\n"),
                    9 => text.push_str("{\"e\":\"a\\\"b\\\\c\\u00e9\\n\"}\n"),
                    10 => text.push_str("\"\u{4e2d}\u{6587}\"\n"),
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
            if round % 11 == 4 {
                text.push_str("12x\n{\"after\":1}\n");
            }
            for &block in &[1usize, 3, 7, 16, 100, 1 << 20] {
                assert_same(text.as_bytes(), block);
            }
        }
    }
}
