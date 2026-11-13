//! Regular files read a window at a time for the plain `succinctly jq` route (#6920, #7024).
//!
//! The plain route (`jq -c .`, `jq .name`, `jq 'select(...)'`: no `-n`, nothing that reads `input`)
//! used to read its file whole, split every document into a span (16 bytes a record) and run the
//! program over borrowed slices of that one buffer, so its peak memory grew with the file where jq's
//! stays at a few MiB. [`WindowedFile`] hands the same loop the same documents, a window of them at a
//! time, and drops the text it has run before reading on; [`WindowedFiles`] reads several files as the one
//! stream jq reads them as.
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
//!   JSON jq reads, and its substitution can differ at a window's edge.) A token open where a file ends
//!   is not substituted until it is joined to the next file's bytes, which is the open tail handed on
//!   (below).
//! - **A file with few line breaks is not read twice.** Only the bytes a read adds are searched for a
//!   break, and a read that finds none asks for twice as much, so a one-line file is read in a linear
//!   amount of work, as whole, and gains nothing.
//!
//! The route's fault wording, which jq's parser makes over the whole stream, is not made from the
//! window: [`Reread`] reads the file again through a duplicate of the open handle, a chunk at a time
//! and by position, on the error path only. A file renamed, replaced or appended to meanwhile
//! is worded from the bytes the run read.
//!
//! **Several files** (#7024) are the stream jq reads them as, one after another: a token still open where
//! a file ends (a truncated container or string, a number or keyword the next byte delimits) continues at
//! the front of the next, and the whole-file route moves it there by [`SeamStitcher`] and records what it
//! moved in a [`Seam`] that its line counting reads (#4305, #4316). The windowed route cuts that open tail
//! off the end of a file's last window the same way and starts the next file's text with it, as the bytes
//! that were read (their substitution is of the two files' bytes together), and gives each window the
//! seam the whole-file route would have had for the file. What the carry on the front asks of the
//! window: nothing is dropped that is only part of it, because the seam describes the front of the
//! text, and jq's reads of the file's first line are counted from where the carry ends.

use std::borrow::Cow;
use std::fs::File;
use std::io::{self, Read};
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Condvar, Mutex, PoisonError};

use succinctly::jq::utf8_document::{original_offset, SubstitutionEdit};

use super::{
    advance_position, count_newlines, open_tail_after_split, split_json_values, Seam, SeamStitcher,
};

/// How much of the file one window reads, and so about how much memory the route holds besides a
/// document that is bigger than that.
pub(super) const WINDOW_BYTES: usize = 1 << 20;

/// The smallest file the route reads a window at a time: below this it is not worth the second
/// route, and the file is read whole as before.
pub(super) const MIN_WINDOWED_FILE_BYTES: u64 = 4 * WINDOW_BYTES as u64;

/// How many windows a document that does not fit in one is read on to before the rest of the file is.
const LOOKAHEAD_WINDOWS: usize = 8;

/// The record separator of a `--seq` stream.
const RS: u8 = 0x1e;

/// Reads each RS byte of `bytes` as a space (#7082).
fn blank_rs_bytes(bytes: &mut [u8]) {
    for b in bytes {
        if *b == RS {
            *b = b' ';
        }
    }
}

/// Whether a document starting with `byte` is a container or string, which jq's parser has read once
/// its last byte is (a number or literal waits on the byte after it).
fn opens_container_or_string(byte: u8) -> bool {
    matches!(byte, b'{' | b'[' | b'"')
}

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
    /// What the files stitched into one stream (#4305) did to this window's text: the bytes moved onto
    /// the front of the file while they are still in `raw`, and the ones moved off its end for the
    /// last window ([`Seam`], #7024). [`Seam::NONE`] for a file that is the stream's only one.
    pub seam: Seam,
    /// Where `raw` starts in jq's input, which reads every file as one stream: the 1-based line and
    /// the bytes before the start on that line (the position a depth refusal names).
    pub origin: (usize, usize),
}

/// Where a file stands among the ones the run reads as one stream, and what it takes from the files
/// before it (#7024). [`Place::alone`] is a file that is the whole stream.
pub(super) struct Place {
    /// The file's position among them: the source tag of its values.
    pub index: usize,
    /// The open tail of the previous file, which is put on the front of this one (raw, as read), and
    /// the seam that records it.
    pub carry: Vec<u8>,
    pub seam: Seam,
    /// Another file follows, so the open tail of this one is cut off and handed on.
    pub hold_tail: bool,
    /// The file may start with a BOM jq skips: only the front of the whole stream does.
    pub strip_bom: bool,
    /// Where the file's bytes start in the stream: the position of the end of the files before.
    pub origin: (usize, usize),
    /// An invalid byte was substituted in an earlier file: the position jq's parser names is counted
    /// over the bytes as read, so a message that names one is worded from the files read again.
    pub substituted: bool,
}

impl Place {
    #[cfg_attr(not(unix), allow(dead_code))]
    pub(super) fn alone() -> Self {
        Self {
            index: 0,
            carry: Vec::new(),
            seam: Seam::NONE,
            hold_tail: false,
            strip_bom: true,
            origin: (1, 0),
            substituted: false,
        }
    }
}

/// Where the invalid-UTF-8 substitution changed the text of a file that hands its open tail on, so
/// that tail can be given to the next file as the bytes that were read (#7024): the substitution of
/// the two files' bytes together is what the whole-file route made, and the text is not invertible.
#[derive(Default, Clone)]
struct RawMap {
    /// The regions the substitution changed, in order, that are still in the buffer.
    regions: Vec<SubstitutedRegion>,
    /// `raw - text` after the regions already dropped from the front.
    delta_dropped: i64,
}

#[derive(Clone)]
struct SubstitutedRegion {
    /// Its text and the bytes it was read as, absolute in the file's stream.
    text: Range<usize>,
    raw_start: usize,
    /// Relative to the region.
    edits: Vec<SubstitutionEdit>,
    /// `raw - text` after the region.
    delta_after: i64,
    /// The bytes each of `edits` replaced, kept for a stream that cannot be read again ([`Certifier`]);
    /// empty otherwise.
    raw_edits: Vec<Vec<u8>>,
}

impl RawMap {
    fn delta_before_end(&self) -> i64 {
        self.regions
            .last()
            .map_or(self.delta_dropped, |r| r.delta_after)
    }

    fn push(
        &mut self,
        text_start: usize,
        raw_len: usize,
        text_len: usize,
        edits: Vec<SubstitutionEdit>,
        raw_edits: Vec<Vec<u8>>,
    ) {
        let delta = self.delta_before_end();
        let raw_start = usize::try_from(text_start as i64 + delta).unwrap_or(0);
        self.regions.push(SubstitutedRegion {
            text: text_start..text_start + text_len,
            raw_start,
            edits,
            delta_after: delta + raw_len as i64 - text_len as i64,
            raw_edits,
        });
    }

    /// Forgets the regions that end before `offset`.
    fn drop_before(&mut self, offset: usize) {
        let keep = self.regions.partition_point(|r| r.text.end <= offset);
        if keep > 0 {
            self.delta_dropped = self.regions[keep - 1].delta_after;
            self.regions.drain(..keep);
        }
    }

    /// Whether the text from `at` on differs from the bytes it was read as.
    fn changes_from(&self, at: usize) -> bool {
        self.regions.last().is_some_and(|r| r.text.end > at)
    }

    /// The offset in the bytes as read of the text at `at`; inside a replacement, where its bytes start.
    fn raw_of(&self, at: usize) -> usize {
        let mut delta = self.delta_dropped;
        for r in &self.regions {
            if at < r.text.start {
                break;
            }
            if at < r.text.end {
                return r.raw_start + original_offset(&r.edits, at - r.text.start);
            }
            delta = r.delta_after;
        }
        usize::try_from(at as i64 + delta).unwrap_or(0)
    }
}

/// A regular file read in windows; see the module's doc.
pub(super) struct WindowedFile {
    /// Shared with the second read of the stream, which reads by position and so leaves this one's place
    /// alone: a descriptor each, not two.
    file: Rc<File>,
    /// How much of the file is left to read: its length when it was opened, less what was read. A
    /// file that grows meanwhile is read as it was, as `std::fs::read` read it, so a run whose
    /// output is appended to its input ends.
    remaining: u64,
    /// The length is not known (`stat` said 0, as it does of an empty file and of a file of the `/proc`
    /// kind that is not): the file is read to its end, as `std::fs::read` reads it.
    unknown_len: bool,
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
    /// Each RS byte (`0x1e`) of the text is read as a space (a `--seq` stream, #7082): nothing moves,
    /// so every offset and line is where it was.
    blank_rs: bool,
    /// A read failed: the bytes read before it are run, and the error is returned after them.
    read_error: Option<io::Error>,
    /// The file's place in the stream it is one of (#7024).
    index: usize,
    /// Another file follows, so the open tail at this file's end is cut off and handed on.
    hold_tail: bool,
    /// The seam of the carry on the front of `buf`, in the text's terms: [`Seam::NONE`] once the carry
    /// has been dropped from the front (a window never drops part of it).
    seam: Seam,
    /// The same seam in the terms of the bytes as read, which is what the stitching of the next file
    /// is computed in.
    seam_raw: Seam,
    /// The carry as it was read, and its length once that is dropped, to give the open tail on.
    carry_raw: Vec<u8>,
    carry_raw_len: usize,
    /// How much has been read from the file.
    file_read: u64,
    /// Where the substitution changed the text, kept for a file that hands a tail on.
    raw_map: RawMap,
    track_raw: bool,
    /// Where `buf` starts in jq's input.
    origin: (usize, usize),
    /// Where the end of the text handed out is, once the last window is.
    end_origin: (usize, usize),
    /// A stream that cannot be read again (a pipe on standard input): each window is certified before
    /// it is handed out, and its first fault kept ([`Certifier`], #7024).
    certifier: Option<Certifier>,
    #[cfg(test)]
    break_search_bytes: usize,
    #[cfg(test)]
    bytes_read: u64,
}

impl WindowedFile {
    /// `file` is read from its start, up to `len` bytes.
    #[cfg(test)]
    pub(super) fn new(file: File, len: u64, block: usize) -> Self {
        Self::in_stream(Rc::new(file), len, block, Place::alone())
    }

    /// `file`, one of several read as one stream (#7024).
    pub(super) fn in_stream(file: Rc<File>, len: u64, block: usize, place: Place) -> Self {
        let carry_len = place.carry.len();
        let track_raw = place.hold_tail || carry_len > 0;
        Self {
            file,
            remaining: if len == 0 { u64::MAX } else { len },
            unknown_len: len == 0,
            block: block.max(1),
            // The carry is on the front, unread as text: its lines are not searched for a break,
            // so the first region the text is judged in covers all of it.
            carry_raw: Vec::new(),
            buf: place.carry,
            validated: 0,
            no_break_from: carry_len,
            skip: 0,
            consumed: 0,
            lines_before: 0,
            offset: 0,
            eof: false,
            started: !place.strip_bom,
            substituted: place.substituted,
            done: false,
            blank_rs: false,
            read_error: None,
            index: place.index,
            hold_tail: place.hold_tail,
            seam_raw: place.seam.clone(),
            seam: place.seam,
            carry_raw_len: carry_len,
            file_read: 0,
            raw_map: RawMap::default(),
            track_raw,
            origin: place.origin,
            end_origin: place.origin,
            certifier: None,
            #[cfg(test)]
            break_search_bytes: 0,
            #[cfg(test)]
            bytes_read: 0,
        }
    }

    /// The next window, or `None` when the file is done. A read that failed is returned here, after
    /// the last window, so the documents read before it are run first.
    #[cfg(test)]
    pub(super) fn next_window(&mut self) -> io::Result<Option<Window<'_>>> {
        self.next_window_with(&mut SeamStitcher::default())
    }

    /// [`next_window`](Self::next_window) for a file that hands its open tail to the next one, which
    /// `stitcher` holds (#7024).
    pub(super) fn next_window_with(
        &mut self,
        stitcher: &mut SeamStitcher,
    ) -> io::Result<Option<Window<'_>>> {
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
                    self.rest()
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
            let mut split_error = split_error.map(|at| at + from);
            // The end of a file that another follows: a token still open there continues in the next
            // file (jq reads them as one stream, #4305), so it is not run, nor a fault, but handed on.
            let mut tail_start = None;
            if self.eof && self.hold_tail && self.read_error.is_none() {
                // What `open_tail_start` finds of a file's text, from the split this scan made.
                let text = &self.buf[..self.validated];
                if let Some(start) =
                    open_tail_after_split(text, split_error, values.last().copied())
                {
                    values.retain(|&(_, end)| end <= start);
                    split_error = None;
                    tail_start = Some(start);
                }
            }
            // A fault before the end of the file is only one when no byte that follows can change
            // it: where no container or string starts, in a text that ends at a line break (a bare
            // token is bounded by it). Otherwise the document may only lack its text.
            let definitive =
                split_error.is_some_and(|at| !self.eof && !opens_container_or_string(self.buf[at]));
            let pending = split_error.is_some() && !self.eof && !definitive;
            // The last document before a fault that may still move is a number or literal whose
            // completion the fault's own byte could be, so it waits to be seen with what follows.
            if pending {
                if let Some(&(start, _)) = values.last() {
                    if !opens_container_or_string(self.buf[start]) {
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
                let text_end = tail_start.unwrap_or(self.validated);
                let mut seam = self.seam.clone();
                if let Some(start) = tail_start {
                    let tail = self.raw_tail(start);
                    // Placed in the bytes as read, which is where the carry's seam was computed.
                    let start_raw = if self.seam_raw.base > 0 {
                        self.raw_map.raw_of(self.offset + start)
                    } else {
                        0
                    };
                    let mut moved = self.seam_raw.clone();
                    stitcher.move_tail(self.index, &mut moved, start_raw, tail, &[], false);
                    seam.moved_len = moved.moved_len;
                    seam.moved_newline = moved.moved_newline;
                }
                // Where the next file starts in the stream: nothing else reads it.
                if last && self.hold_tail {
                    self.end_origin = advance_position(self.origin, &self.buf[..text_end]);
                }
                // The stream's first fault, if this window holds it, found before any of it runs.
                if self.certifier.is_some() {
                    let fault_at_end = last && split_error.is_some() && self.read_error.is_none();
                    self.certify(&values, fault_at_end);
                }
                return Ok(Some(Window {
                    raw: &self.buf[..text_end],
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
                    seam,
                    origin: self.origin,
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
                    self.rest()
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
    ///
    /// The carry on the front is the open tail of the previous file, which the first document of this
    /// one is made of. Nothing that is only part of it is dropped: its seam describes the front of the
    /// text, and jq's reads of the file's first line (up to 4095 bytes each) are counted from where the
    /// carry ends (#4316), so a seam kept over a text that no longer starts with the carry would
    /// count them from the wrong byte. Once the drop reaches past it the seam goes with it, and the
    /// lines of the carry are not the file's.
    fn drop_consumed(&mut self) {
        let consumed = self.consumed.min(self.buf.len());
        let mut line_start = self.buf[..consumed]
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |at| at + 1);
        let base = self.seam.base;
        if line_start < base {
            line_start = 0;
        }
        if line_start > 0 {
            let dropped = count_newlines(&self.buf[..line_start]);
            self.lines_before += dropped - count_newlines(&self.buf[..base]);
            self.origin = (self.origin.0 + dropped, 0);
            self.offset += line_start;
            self.buf.drain(..line_start);
            self.validated -= line_start;
            self.no_break_from = self.no_break_from.saturating_sub(line_start);
            if base > 0 {
                self.seam = Seam::NONE;
                self.seam_raw = Seam::NONE;
                self.carry_raw = Vec::new();
            }
            self.raw_map.drop_before(self.offset);
        }
        self.skip = consumed - line_start;
        self.consumed = self.skip;
    }

    /// The bytes of the open tail of the file from `start` in the text, as they were read, which is
    /// what the next file's text is judged with (the substitution is of the two together).
    fn raw_tail(&mut self, start: usize) -> Vec<u8> {
        let at = self.offset + start;
        if !self.raw_map.changes_from(at) {
            return self.buf[start..self.validated].to_vec();
        }
        let raw_at = self.raw_map.raw_of(at);
        let mut tail = Vec::new();
        if raw_at < self.carry_raw_len {
            // The carry as read: kept when its front was substituted, and otherwise it is the text.
            let carry = if self.carry_raw.is_empty() {
                &self.buf[..self.carry_raw_len.min(self.buf.len())]
            } else {
                self.carry_raw.as_slice()
            };
            tail.extend_from_slice(carry.get(raw_at..).unwrap_or(&[]));
        }
        let from = raw_at.saturating_sub(self.carry_raw_len) as u64;
        match read_range(&self.file, from, self.file_read) {
            Ok(bytes) => tail.extend(bytes),
            // The tail is cut short: the run ends at this failure, which is reported after the window.
            // patchcov: coverage tolerate reason="untestable: a read that fails after the file opened"
            Err(error) => {
                self.read_error.get_or_insert(error);
            } // patchcov: coverage end
        }
        tail
    }

    /// All that is left of the file, for a read that wants it in one go (the file's length is not known:
    /// a few windows).
    fn rest(&self) -> usize {
        if self.unknown_len {
            4 * WINDOW_BYTES
        } else {
            usize::try_from(self.remaining).unwrap_or(usize::MAX)
        }
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
            match (&*self.file).take(n as u64).read_to_end(&mut self.buf) {
                Ok(read) => {
                    self.remaining -= read as u64;
                    self.file_read += read as u64;
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
        self.skip_front_bom();
    }

    /// jq's parser skips a BOM at the start of its input (#5291), as the whole-file route does; with
    /// fewer than three bytes read it cannot be told from the start of another character.
    fn skip_front_bom(&mut self) {
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
        let from = self.validated;
        if succinctly::text::utf8::validate_utf8(&self.buf[from..end]).is_ok() {
            self.validated = end;
        } else {
            self.substitute_region(end);
        }
        if self.blank_rs {
            blank_rs_bytes(&mut self.buf[from..self.validated]);
        }
    }

    /// Substitutes the invalid UTF-8 of the region from `validated` to `end`, as the whole-file route
    /// does, and moves `validated` past it.
    fn substitute_region(&mut self, end: usize) {
        let region = &self.buf[self.validated..end];
        let raw_len = region.len();
        let (text, edits) = if self.track_raw {
            let (text, edits) =
                succinctly::jq::utf8_document::substitute_invalid_utf8_jq_document_mapped(region);
            (text.into_bytes(), edits)
        } else {
            (
                succinctly::jq::utf8_document::substitute_invalid_utf8_jq_document(region)
                    .into_bytes(),
                Vec::new(),
            )
        };
        let text_start = self.offset + self.validated;
        if self.track_raw {
            // The first region is the front of the file, where the carry is: its seam is moved by
            // the substitution as the whole-file route's is (`utf8_lossy_document`).
            if self.validated == 0 && self.seam.base > 0 {
                self.seam = std::mem::replace(&mut self.seam, Seam::NONE)
                    .after_substitution(raw_len, &edits);
                // The carry as read, for a tail that starts inside it and goes on to the next file.
                if self.hold_tail {
                    self.carry_raw = region[..self.carry_raw_len.min(region.len())].to_vec();
                }
            }
            // Only the bytes each edit replaced: the rest of the region reads as its text.
            let raw_edits = if self.certifier.is_some() {
                edits
                    .iter()
                    .map(|edit| region[edit.raw.clone()].to_vec())
                    .collect()
            } else {
                Vec::new()
            };
            self.raw_map
                .push(text_start, raw_len, text.len(), edits, raw_edits);
        }
        let new_end = self.validated + text.len();
        self.buf.splice(self.validated..end, text);
        self.validated = new_end;
        self.substituted = true;
    }
}

/// How many windows may be waiting for the certifying thread (or in its hands) at once: the filter
/// runs over window N while N and N - 1 are certified, so a thread that falls behind holds the filter
/// back at the next window and the copies of the text stay at two (#7106).
const CERTIFIER_WINDOWS_IN_FLIGHT: usize = 2;

/// A window of more than this is certified on the filter's own thread, over the buffer itself: it holds a
/// document bigger than a few windows, and a copy of it would add its size to the peak memory of a run
/// whose point is to hold less (a 175 MB one-line array read 395 MB with the copy and 222 MB without,
/// #7106). The probe's first window of 4 MiB or more is under it and is copied: certifying it here
/// instead saves 4 MiB of peak memory and costs the first window of a pipe of 10 MiB 4 to 10% of its
/// run (measured on terminus and the mini).
const CERTIFIER_COPY_LIMIT: usize = 8 * WINDOW_BYTES;

/// A buffer the certifying thread hands back is kept for the next window only up to this size, about two
/// windows, so the probe's copy and one that grew with a document are not held for the run.
const CERTIFIER_KEEP_BYTES: usize = 2 * WINDOW_BYTES;

/// The first fault of a stream that cannot be read again, where it has been found: shared by the thread
/// that finds it ([`Certifier`]) and the [`Reread`] that words it.
///
/// A window is certified while the filter runs over it (#7106), so a fault is read only once every
/// window handed out has been: [`SeenFault::get`] waits for them, and what it returns is what the
/// windows certified before the filter ran would have kept.
#[derive(Clone, Default)]
pub(super) struct SeenFault(Arc<SeenShared>);

#[derive(Default)]
struct SeenShared {
    state: Mutex<SeenState>,
    changed: Condvar,
}

#[derive(Default)]
struct SeenState {
    /// Windows handed to the certifying thread that it has not finished.
    in_flight: usize,
    fault: Option<String>,
    /// The certifying thread panicked, so what it found is not known.
    panicked: bool,
    /// Where jq's parser stands at a boundary in the stream (absolute, as `offset` counts): the end of
    /// the last document certified, or the start of a number or literal, which the parser waits on the
    /// next byte to read. Past it only white space has been dropped.
    resume: usize,
}

impl SeenFault {
    fn lock(&self) -> std::sync::MutexGuard<'_, SeenState> {
        self.0.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The stream's first fault, once every window handed to the certifier has been certified.
    pub(super) fn get(&self) -> Option<String> {
        let state = self
            .0
            .changed
            .wait_while(self.lock(), |state| state.in_flight > 0)
            .unwrap_or_else(PoisonError::into_inner);
        assert!(
            !state.panicked,
            "the thread that certifies the windows panicked"
        ); // patchcov: coverage tolerate-line reason="defensive: the certifier's own code does not panic; kept so a bug there is not read as a stream without a fault"
        state.fault.clone()
    }

    /// Whether a fault has been found so far, without waiting for the windows still being certified.
    fn found(&self) -> bool {
        self.lock().fault.is_some()
    }

    /// Takes a place for one more window, waiting while [`CERTIFIER_WINDOWS_IN_FLIGHT`] are taken.
    fn begin(&self) {
        let mut state = self
            .0
            .changed
            .wait_while(self.lock(), |state| {
                state.in_flight >= CERTIFIER_WINDOWS_IN_FLIGHT
            })
            .unwrap_or_else(PoisonError::into_inner);
        state.in_flight += 1;
    }

    /// A window is certified: `fault` is the first of the stream if none was found before it, and
    /// `resume` is where jq's parser stands after it (`None` where a window was given up, `panicked`).
    fn finish(&self, fault: Option<String>, resume: Option<usize>, panicked: bool) {
        let mut state = self.lock();
        state.in_flight -= 1;
        state.panicked |= panicked;
        Self::store(&mut state, fault, resume);
        drop(state);
        self.0.changed.notify_all();
    }

    fn store(state: &mut SeenState, fault: Option<String>, resume: Option<usize>) {
        if state.fault.is_none() {
            state.fault = fault;
        }
        if let Some(resume) = resume {
            state.resume = resume;
        }
    }

    /// Where jq's parser stands after the windows certified so far.
    fn resume(&self) -> usize {
        self.lock().resume
    }

    /// Certifies a window on the calling thread, over the buffer itself (#7106): `certify` gets where
    /// jq's parser stands, once every window in flight is certified, and the answer is kept as the
    /// thread would have kept it.
    fn certify_here(&self, certify: impl FnOnce(&mut usize) -> Option<String>) {
        let state = self
            .0
            .changed
            .wait_while(self.lock(), |state| state.in_flight > 0)
            .unwrap_or_else(PoisonError::into_inner);
        let mut resume = state.resume;
        // Not held while certifying: the filter's thread is the only one that hands the certifier a
        // window, so none arrives meanwhile.
        drop(state);
        let fault = certify(&mut resume);
        Self::store(&mut self.lock(), fault, Some(resume));
    }
}

/// The deepest nesting the strict validator certifies a document at. jq's parser refuses a bracket
/// when its stack holds [`succinctly::jq::JQ_MAX_PARSING_DEPTH`] entries, and an object takes two (its
/// frame and the key waiting for a value), so 127 levels of anything stay under it. A deeper document
/// is not certified; it goes to jq's parser, which is the judge.
const CERTIFIED_DEPTH: usize = 127;

/// The first fault of a stream that cannot be read again (a pipe on standard input, #7024), found as
/// the windows are handed out, before the text they drop is gone.
///
/// jq's parser words the stream's first fault, which can be in a document the run read past (a filter
/// validates only what it reads, #2103) and the window has since dropped. Each document of a window is
/// therefore certified before the window is dropped: one the strict validator accepts is one jq's
/// parser reads without a fault (`what_the_strict_validator_accepts_jq_reads_7024`), and what jq's
/// parser holds after it is nothing but its position, or for a number or literal the token's own bytes.
/// A document the validator turns down (a `nan`, a `01`, a deep one, a malformed one) is read by jq's
/// parser itself, from the last boundary, over the bytes as read. The fault it meets, the first of the
/// stream since everything before it is certified, is kept, and so is a fault after the last document
/// at the end of the stream; every later window goes unchecked.
///
/// The verdict decides the wording of a fault and nothing the run prints, so the windows are certified
/// on a thread of their own, over a copy of each, while the filter runs over the window (#7106): the
/// thread starts with the first window, in a run that reads a pipe of 4 MiB or more, and a reader of
/// the fault waits for it ([`SeenFault::get`]).
struct Certifier {
    seen: SeenFault,
    /// The thread, started with the first window.
    worker: Option<Worker>,
    /// No thread could be started: every window is certified on the filter's own thread, as the one
    /// over [`CERTIFIER_COPY_LIMIT`] is.
    threadless: bool,
    /// How many reads by jq's parser certifying the windows took.
    #[cfg(test)]
    exact_reads: Arc<AtomicUsize>,
}

/// The certifying thread's inbox, and the buffers it has finished with, to be filled again.
struct Worker {
    jobs: SyncSender<Job<'static>>,
    recycled: Receiver<Vec<u8>>,
}

/// One window to certify: a copy of the text and what the certifier needs of the reader's state to word
/// a fault from it.
struct Job<'a> {
    /// `buf[..validated]` of the window: a copy for the thread, the buffer itself for the filter's own.
    text: Cow<'a, [u8]>,
    /// The documents of the window, in `text`.
    values: Vec<Span>,
    /// The window is the last and ends in a fault.
    fault_at_end: bool,
    /// The bytes dropped from the front of the stream before `text`, and where `text` starts in jq's input.
    offset: usize,
    origin: (usize, usize),
    /// The stream ends with the window.
    eof: bool,
    raw_map: Cow<'a, RawMap>,
    #[cfg(test)]
    exact_reads: Arc<AtomicUsize>,
}

/// The certifying thread: each window of `jobs` in order, the stream's first fault kept in `seen`.
fn certify_windows(
    jobs: &Receiver<Job<'static>>,
    recycled: &SyncSender<Vec<u8>>,
    seen: &SeenFault,
) {
    for mut job in jobs {
        let mut resume = seen.resume();
        let outcome = if seen.found() {
            Ok(None)
        } else {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job.certify(&mut resume)))
        };
        let text = match std::mem::take(&mut job.text) {
            Cow::Owned(mut text) if text.capacity() <= CERTIFIER_KEEP_BYTES => {
                text.clear();
                text
            }
            _ => Vec::new(),
        };
        // The reader may have gone: a buffer nobody wants is dropped.
        let _ = recycled.send(text);
        match outcome {
            Ok(fault) => seen.finish(fault, Some(resume), false),
            Err(_) => seen.finish(None, None, true), // patchcov: coverage tolerate-line reason="defensive: certify does not panic; kept so a bug in it is reported by SeenFault::get"
        }
    }
}

/// The thread that certifies the windows, and the channels to and from it.
fn start_worker(seen: &SeenFault) -> io::Result<Worker> {
    let (jobs, inbox) = sync_channel(CERTIFIER_WINDOWS_IN_FLIGHT);
    let (outbox, recycled) = sync_channel(CERTIFIER_WINDOWS_IN_FLIGHT + 1);
    let seen = seen.clone();
    std::thread::Builder::new()
        .name("certify-windows".into())
        .spawn(move || certify_windows(&inbox, &outbox, &seen))?;
    Ok(Worker { jobs, recycled })
}

impl WindowedFile {
    /// A pipe on standard input, from what a first read of it took (`front`, which ended in `error` if
    /// that read failed): its length is not known, it cannot be read again, and its windows are
    /// certified as they are handed out, their first fault kept in `seen` ([`Certifier`], #7024).
    #[cfg_attr(not(unix), allow(dead_code))]
    pub(super) fn from_pipe(
        file: Rc<File>,
        front: Vec<u8>,
        error: Option<io::Error>,
        block: usize,
        seen: SeenFault,
    ) -> Self {
        let mut this = Self::in_stream(file, 0, block, Place::alone());
        this.file_read = front.len() as u64;
        this.buf = front;
        // Where the substitution changed the text, so the bytes as read can be given to jq's parser.
        this.track_raw = true;
        this.certifier = Some(Certifier {
            seen,
            worker: None,
            threadless: false,
            #[cfg(test)]
            exact_reads: Arc::default(),
        });
        if let Some(error) = error {
            this.read_error = Some(error);
            this.eof = true;
            // No read follows to skip it.
            this.skip_front_bom();
        }
        this
    }

    /// Certifies `values`, the documents of the window about to be handed out (and the bytes after them,
    /// when `fault_at_end`: the window is the last and ends in a fault), and keeps the stream's first
    /// fault. The window is handed to the thread that certifies them, over a copy, and waits only while
    /// two others are in flight; one of more than [`CERTIFIER_COPY_LIMIT`], or any when no thread could
    /// be started, is certified here, over the buffer itself.
    fn certify(&mut self, values: &[Span], fault_at_end: bool) {
        let Some(certifier) = self.certifier.as_mut() else {
            return;
        };
        if certifier.seen.found() {
            return;
        }
        let text = &self.buf[..self.validated];
        if certifier.worker.is_none() && !certifier.threadless && text.len() <= CERTIFIER_COPY_LIMIT
        {
            match start_worker(&certifier.seen) {
                Ok(worker) => certifier.worker = Some(worker),
                Err(_) => certifier.threadless = true,
            }
        }
        let Some(worker) = certifier
            .worker
            .as_ref()
            .filter(|_| text.len() <= CERTIFIER_COPY_LIMIT)
        else {
            let job = Job {
                text: Cow::Borrowed(text),
                values: values.to_vec(),
                fault_at_end,
                offset: self.offset,
                origin: self.origin,
                eof: self.eof,
                raw_map: Cow::Borrowed(&self.raw_map),
                #[cfg(test)]
                exact_reads: Arc::clone(&certifier.exact_reads),
            };
            certifier.seen.certify_here(|resume| job.certify(resume));
            return;
        };
        // A place first, so the copies of the text are never more than two.
        certifier.seen.begin();
        // A buffer the thread has finished with, or a new one while fewer than three are in use.
        let mut copy: Vec<u8> = worker.recycled.try_recv().unwrap_or_default();
        copy.extend_from_slice(text);
        let job = Job {
            text: Cow::Owned(copy),
            values: values.to_vec(),
            fault_at_end,
            offset: self.offset,
            origin: self.origin,
            eof: self.eof,
            raw_map: Cow::Owned(self.raw_map.clone()),
            #[cfg(test)]
            exact_reads: Arc::clone(&certifier.exact_reads),
        };
        if worker.jobs.send(job).is_err() {
            // The thread is gone, which only a panic outside its catch can do: give the place back and
            // have the next reader of the fault say so.
            certifier.seen.finish(None, None, true); // patchcov: coverage tolerate-line reason="defensive: the thread ends only when its sender is dropped, and a panic inside it is caught"
        }
    }

    /// The reads by jq's parser the windows certified so far took.
    #[cfg(test)]
    fn exact_reads(&self) -> usize {
        let certifier = self.certifier.as_ref().expect("a pipe");
        certifier.seen.get();
        certifier.exact_reads.load(Ordering::Relaxed)
    }
}

impl Job<'_> {
    /// Certifies the documents of the window, from where jq's parser stood at the end of the last:
    /// the fault it meets, if it is the stream's first.
    fn certify(&self, resume: &mut usize) -> Option<String> {
        // Only white space has been dropped past the boundary: the parser stands at the text's start.
        let mut at = (*resume).max(self.offset) - self.offset;
        let mut fault = None;
        for &(start, end) in &self.values {
            let certified = succinctly::json::validate::Validator::new(&self.text[start..end])
                .with_max_depth(CERTIFIED_DEPTH)
                .validate()
                .is_ok();
            if certified {
                at = self.boundary_after(start, end);
                continue;
            }
            // One read by jq's parser, from the last boundary to the end of the window, covers this
            // document and every one after it, so a window costs one such read at most whatever it
            // holds (a stream of `NaN`s is read at the parser's speed, not once a document).
            match self.read_from(at, self.text.len()) {
                Err(message) => fault = Some(message),
                Ok(_) => {
                    if let Some(&(start, end)) = self.values.last() {
                        at = self.boundary_after(start, end);
                    }
                }
            }
            break;
        }
        if fault.is_none() && self.fault_at_end {
            fault = self.end_fault(at);
        }
        *resume = self.offset + at;
        fault
    }

    /// Where jq's parser stands at a boundary once the document at `start..end` is read: after it for a
    /// container or string, and at its start for a number or literal, whose bytes the parser holds
    /// until the byte that ends it.
    fn boundary_after(&self, start: usize, end: usize) -> usize {
        if opens_container_or_string(self.text[start]) {
            end
        } else {
            start
        }
    }

    /// jq's parser over the text from `from` to `to`, as read, from the position it has at `from`: the
    /// fault it meets, or whether it ends at a boundary between top-level values.
    fn read_from(&self, from: usize, to: usize) -> Result<bool, String> {
        #[cfg(test)]
        self.exact_reads.fetch_add(1, Ordering::Relaxed);
        let (line, column) = self.position(from);
        succinctly::jq::jq_stream_prefix_fault([self.as_read(from, to)], line, column)
    }

    /// The fault the last window ends in, read from `from` to the end of the text: at the end of the
    /// stream, as the whole-stream reader words it (`... at EOF` included).
    fn end_fault(&self, from: usize) -> Option<String> {
        let (line, column) = self.position(from);
        let bytes = self.as_read(from, self.text.len());
        if self.eof {
            succinctly::jq::jq_stream_fault([bytes], line, column)
        } else {
            succinctly::jq::jq_stream_prefix_fault([bytes], line, column).err()
        }
    }

    /// jq's position at `at` in the text: its line, and the bytes before `at` on that line as read
    /// (where an invalid byte was substituted, the bytes it replaced).
    fn position(&self, at: usize) -> (usize, usize) {
        let (line, column) = advance_position(self.origin, &self.text[..at]);
        let line_start = if line == self.origin.0 {
            0
        } else {
            at - column
        };
        let raw = |at: usize| self.raw_map.raw_of(self.offset + at);
        (
            line,
            column - (at - line_start) + (raw(at) - raw(line_start)),
        )
    }

    /// The text from `from` to `to` as it was read: the substituted regions in it given back as the
    /// bytes they replaced.
    fn as_read(&self, from: usize, to: usize) -> Cow<'_, [u8]> {
        if !self.raw_map.changes_from(self.offset + from) {
            return Cow::Borrowed(&self.text[from..to]);
        }
        let mut bytes = Vec::with_capacity(to - from);
        let mut at = from;
        'regions: for region in &self.raw_map.regions {
            for (edit, raw) in region.edits.iter().zip(&region.raw_edits) {
                // Absolute, since the front of a region can be dropped already; a replacement holds no
                // line break, so one that ends past a dropped line start begins after it.
                let end = region.text.start + edit.text.end;
                if end <= self.offset + at {
                    continue;
                }
                let start = region.text.start + edit.text.start - self.offset;
                let end = end - self.offset;
                // An offset inside a replacement is where the bytes it replaced start, so a range that
                // starts inside one holds them and one that ends inside one does not.
                if end > to {
                    break 'regions;
                }
                if start > at {
                    bytes.extend_from_slice(&self.text[at..start]);
                }
                bytes.extend_from_slice(raw);
                at = end;
            }
        }
        if at < to {
            bytes.extend_from_slice(&self.text[at..to]);
        }
        Cow::Owned(bytes)
    }
}

/// The bytes of `file` from `from` to `to`, read by position: a file that gives out early is short.
#[cfg(unix)]
fn read_range(file: &File, from: u64, to: u64) -> io::Result<Vec<u8>> {
    use std::os::unix::fs::FileExt;
    let mut bytes = vec![0u8; usize::try_from(to.saturating_sub(from)).unwrap_or(0)];
    let mut filled = 0;
    while filled < bytes.len() {
        match file.read_at(&mut bytes[filled..], from + filled as u64) {
            Ok(read) if read > 0 => filled += read,
            // patchcov: coverage tolerate reason="untestable: a file that shrank while the run read it, and a read that fails"
            Ok(_) => break,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
            // patchcov: coverage end
        }
    }
    bytes.truncate(filled);
    Ok(bytes)
}

/// The several-files route is Unix only: the tail of a file is read again by position.
#[cfg(not(unix))]
fn read_range(_file: &File, _from: u64, _to: u64) -> io::Result<Vec<u8>> {
    Ok(Vec::new())
}

/// A file of a several-files stream that is opened when the run reaches it, by its path, where a short
/// list holds each file open from the start (#7024): past the descriptors a process holds cheaply, a
/// long list would grow its descriptor table, and each doubling of that table waits for an RCU grace
/// period. The file is judged before the run starts ([`super::windowed_regular_files`]) and closed
/// again; when the run reaches it, it reads what the path names then, as jq, which opens each file as
/// it reaches it, does. A path that will not open then (or names a directory) is read as an empty file
/// and its failure kept for the run to report as jq's reader does when it moves past it
/// ([`WindowedFiles::take_open_failures`]), so the tail of the file before it continues in the file
/// after it, as in jq and in the whole-file route, which drops such a file from the stream. The second
/// read that words a fault reads the files as the run read them ([`Reach`]).
#[derive(Debug)]
pub(super) struct NamedFile {
    path: PathBuf,
    reach: std::cell::Cell<Reach>,
}

/// What the run found at a [`NamedFile`]'s path when it reached it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reach {
    /// Not reached yet.
    Ahead,
    /// A file, with its device and inode and the length the run reads of it.
    Opened(FileIdentity, u64),
    /// Nothing that opened as a file: read as no bytes.
    Failed,
}

/// Which file a path named when the run opened it: its device and inode.
#[cfg(unix)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileIdentity {
    dev: u64,
    ino: u64,
}

#[cfg(not(unix))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileIdentity;

impl FileIdentity {
    #[cfg(unix)]
    fn of(meta: &std::fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt;
        Self {
            dev: meta.dev(),
            ino: meta.ino(),
        }
    }

    #[cfg(not(unix))]
    fn of(_: &std::fs::Metadata) -> Self {
        Self
    }
}

/// `EISDIR`: what a read of a directory fails with on Linux and macOS, which jq reports as `Is a
/// directory` (`std`'s `ErrorKind::IsADirectory` needs a newer Rust than the crate's).
const EISDIR: i32 = 21;

impl NamedFile {
    pub(super) fn new(path: PathBuf) -> Self {
        Self {
            path,
            reach: std::cell::Cell::new(Reach::Ahead),
        }
    }

    /// The file the path names now, opened for the run, and how much of it the run reads; the error
    /// when there is none. A directory opens on Unix and fails at its first read, so it fails here.
    fn open_for_run(&self) -> io::Result<(File, u64)> {
        let opened = File::open(&self.path).and_then(|file| {
            let meta = file.metadata()?;
            if meta.is_dir() {
                return Err(io::Error::from_raw_os_error(EISDIR));
            }
            Ok((file, meta))
        });
        match opened {
            Ok((file, meta)) => {
                self.reach
                    .set(Reach::Opened(FileIdentity::of(&meta), meta.len()));
                Ok((file, meta.len()))
            }
            Err(error) => {
                self.reach.set(Reach::Failed);
                Err(error)
            }
        }
    }

    /// The file the run read, opened again for the second read: `None` for one it read as no bytes;
    /// an error when the path names another file now.
    fn reopen_as_read(&self) -> io::Result<Option<(File, u64)>> {
        let (identity, len) = match self.reach.get() {
            Reach::Failed => return Ok(None),
            // Not reached: what the run will read there is what the path names now.
            Reach::Ahead => return self.open_for_run().map(Some),
            Reach::Opened(identity, len) => (identity, len),
        };
        let file = File::open(&self.path)?;
        if FileIdentity::of(&file.metadata()?) != identity {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "the file is not the one the run read",
            ));
        }
        Ok(Some((file, len)))
    }
}

/// Where each file of a several-files stream comes from (#7024).
enum StreamFile {
    /// Open from the start of the run, with how much of it the run reads.
    Held(Rc<File>, u64),
    /// Opened when the run reaches it ([`NamedFile`]).
    Named(Rc<NamedFile>),
}

/// Several regular files read as the one stream jq reads them as (#7024): each a window at a time, and
/// a token still open where a file ends continued at the front of the next ([`SeamStitcher`], the code
/// the whole-file route stitches with), so the documents, the file each is read in and the lines jq
/// names for it are the whole-file route's.
pub(super) struct WindowedFiles {
    /// The files not yet reached, each with how much of it the run reads.
    pending: std::vec::IntoIter<StreamFile>,
    count: usize,
    block: usize,
    current: Option<WindowedFile>,
    stitcher: SeamStitcher,
    /// Where the files read so far end in jq's input.
    end_origin: (usize, usize),
    /// Every file so far held no bytes (as read, not as `stat` said: a `/proc` file is 0 and has some),
    /// so a BOM may still be at the front of the stream.
    only_empty: bool,
    /// The position of the next file to open.
    next_index: usize,
    /// An invalid byte was substituted in a file already done.
    substituted: bool,
    /// Each RS byte of the text is read as a space (a `--seq` stream, #7082).
    blank_rs: bool,
    /// The files that did not open when the run reached them, with their positions, not yet taken
    /// by the run ([`NamedFile`]).
    open_failures: Vec<(usize, PathBuf, io::Error)>,
}

impl WindowedFiles {
    pub(super) fn new(files: Vec<(Rc<File>, u64)>, block: usize) -> Self {
        Self::of(
            files
                .into_iter()
                .map(|(file, len)| StreamFile::Held(file, len))
                .collect(),
            block,
        )
    }

    /// Files opened a file at a time, as the run reaches each ([`NamedFile`]).
    #[cfg_attr(not(unix), allow(dead_code))]
    pub(super) fn named(files: &[Rc<NamedFile>], block: usize) -> Self {
        Self::of(
            files
                .iter()
                .map(|named| StreamFile::Named(Rc::clone(named)))
                .collect(),
            block,
        )
    }

    fn of(files: Vec<StreamFile>, block: usize) -> Self {
        Self {
            count: files.len(),
            pending: files.into_iter(),
            block,
            current: None,
            stitcher: SeamStitcher::default(),
            end_origin: (1, 0),
            only_empty: true,
            next_index: 0,
            substituted: false,
            blank_rs: false,
            open_failures: Vec::new(),
        }
    }

    /// A pipe on standard input ([`WindowedFile::from_pipe`], #7024): the stream's one source.
    #[cfg_attr(not(unix), allow(dead_code))]
    pub(super) fn pipe(
        file: Rc<File>,
        front: Vec<u8>,
        error: Option<io::Error>,
        block: usize,
        seen: SeenFault,
    ) -> Self {
        let mut this = Self::new(Vec::new(), block);
        this.count = 1;
        this.next_index = 1;
        this.current = Some(WindowedFile::from_pipe(file, front, error, block, seen));
        this
    }

    /// The same reading each RS byte as a space (#7082): the plain JSON stream a clean `--seq` stream is.
    pub(super) fn blanking_rs(mut self) -> Self {
        self.blank_rs = true;
        self
    }

    /// [`Self::next_window`], handing `note` each file that did not open on the way to it, in order,
    /// with its position among the files ([`NamedFile`]).
    pub(super) fn next_window_noting(
        &mut self,
        note: &mut dyn FnMut(usize, super::InputFileOpenError),
    ) -> io::Result<Option<(usize, Window<'_>)>> {
        let count = self.count;
        if let Err(error) = self.reach_a_file_with_windows_left() {
            self.note_open_failures(count, note);
            return Err(error);
        }
        self.note_open_failures(count, note);
        self.next_window()
    }

    fn note_open_failures(
        &mut self,
        count: usize,
        note: &mut dyn FnMut(usize, super::InputFileOpenError),
    ) {
        for (at, path, error) in self.open_failures.drain(..) {
            note(
                at,
                super::InputFileOpenError::from_io(&path, &error, at + 1 == count),
            );
        }
    }

    /// The next window and the position of its file among the files, or `None` when the last file is
    /// done. A read that failed is returned after the failing file's last window.
    pub(super) fn next_window(&mut self) -> io::Result<Option<(usize, Window<'_>)>> {
        if !self.reach_a_file_with_windows_left()? {
            return Ok(None);
        }
        let stitcher = &mut self.stitcher;
        self.current.as_mut().map_or(Ok(None), |current| {
            let index = current.index;
            current
                .next_window_with(stitcher)
                .map(|window| window.map(|window| (index, window)))
        })
    }

    /// Leaves `current` a file whose last window has not been handed out, moving on from the one that
    /// is done and opening the next; `false` when there is none.
    fn reach_a_file_with_windows_left(&mut self) -> io::Result<bool> {
        loop {
            match &mut self.current {
                Some(current) if current.done => {
                    self.end_origin = current.end_origin;
                    self.substituted |= current.substituted;
                    self.only_empty &= current.file_read == 0;
                    let error = current.read_error.take();
                    self.current = None;
                    if let Some(error) = error {
                        return Err(error);
                    }
                }
                Some(_) => return Ok(true),
                None => {
                    let index = self.next_index;
                    let (file, len) = match self.pending.next() {
                        None => return Ok(false),
                        Some(StreamFile::Held(file, len)) => (file, len),
                        Some(StreamFile::Named(named)) => match named.open_for_run() {
                            Ok((file, len)) => (Rc::new(file), len),
                            Err(error) => {
                                self.open_failures.push((index, named.path.clone(), error));
                                // Left out of the stream, as the whole-file route leaves it out, so
                                // the open tail before it continues in the next file. A last one is
                                // read as no bytes instead, which ends that tail where jq's reader
                                // names it after the file it could not open (#6480).
                                if self.pending.len() > 0 {
                                    self.next_index += 1;
                                    continue;
                                }
                                (Rc::new(File::open("/dev/null")?), 0)
                            }
                        },
                    };
                    self.next_index += 1;
                    let mut seam = Seam::NONE;
                    let carry = self.stitcher.take_carry(&mut seam);
                    let place = Place {
                        index,
                        carry,
                        seam,
                        hold_tail: index + 1 < self.count,
                        strip_bom: self.only_empty,
                        origin: self.end_origin,
                        substituted: self.substituted,
                    };
                    let mut opened = WindowedFile::in_stream(file, len, self.block, place);
                    opened.blank_rs = self.blank_rs;
                    self.current = Some(opened);
                }
            }
        }
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

/// A source read as whole lines, a window at a time (#7023): `-R`'s file, and the rest of a lone source
/// the lazy `inputs` route started on the front of. A line is no more than a line break away from its
/// end, so there is nothing to scan and nothing to carry across a window but the bytes after its last
/// line break: the caller turns each window's lines into text and drops the window, and the memory held
/// is a window and one line, not the source.
pub(super) struct LineWindows {
    reader: Box<dyn Read>,
    /// How much of the source is left to read: a regular file's length when it was opened, less what
    /// was read (a file that grows meanwhile is read as it was); `None` for one of unknown length,
    /// which ends at a short read.
    remaining: Option<u64>,
    /// How much a read adds.
    block: usize,
    /// The bytes read after the last line break of the last window: the front of the next.
    carried: Vec<u8>,
    /// Nothing more is read: the end of the source, a read that failed, or the length it had.
    eof: bool,
}

impl LineWindows {
    /// `reader` is read from where it stands, up to `len` bytes if that is known; `carried` are bytes
    /// already read from it, which come before them.
    pub(super) fn new(
        reader: Box<dyn Read>,
        len: Option<u64>,
        block: usize,
        carried: Vec<u8>,
    ) -> Self {
        Self {
            reader,
            remaining: len,
            block: block.max(1),
            carried,
            eof: false,
        }
    }

    /// Whether the last window is the last: no byte is left to read or carried.
    pub(super) fn finished(&self) -> bool {
        self.eof && self.carried.is_empty()
    }

    /// The next window of at least `want` bytes (the size of a block when `None`): bytes that end at a
    /// line break, or at the end of the source for the last, whose final line then has none. `None`
    /// after the last. A read that fails ends the source there and is handed to `on_error`, once; the
    /// line it cut is dropped, as no one knows its end.
    pub(super) fn next_window(
        &mut self,
        want: Option<usize>,
        on_error: impl FnOnce(&io::Error),
    ) -> Option<Vec<u8>> {
        let mut bytes = std::mem::take(&mut self.carried);
        let mut ask = want.unwrap_or(self.block).max(1);
        // #7024: bytes carried in that make a window already (a pipe's probe of 4 MiB) are handed out as
        // one, up to their last line break. Reading a block onto them would grow the probe's buffer to
        // twice its size, and a window to the probe and a block. The bytes carried after a window are
        // past its last line break, so they hold none and are never cut here.
        if bytes.len() >= ask {
            if let Some(at) = last_line_break(&bytes) {
                self.carried = bytes.split_off(at + 1);
                return Some(bytes);
            }
        }
        let mut on_error = Some(on_error);
        loop {
            if !self.eof {
                let n = self.remaining.map_or(ask, |left| {
                    ask.min(usize::try_from(left).unwrap_or(usize::MAX))
                });
                if n == 0 {
                    self.eof = true;
                }
            }
            if self.eof {
                return (!bytes.is_empty()).then_some(bytes);
            }
            let n = self.remaining.map_or(ask, |left| {
                ask.min(usize::try_from(left).unwrap_or(usize::MAX))
            });
            let before = bytes.len();
            // patchcov: coverage tolerate reason="untestable: an allocation that fails"
            if bytes.try_reserve(n).is_err() {
                self.fail(
                    &mut bytes,
                    &mut on_error,
                    &io::Error::from(io::ErrorKind::OutOfMemory),
                );
                continue;
            }
            // patchcov: coverage end
            match self.reader.by_ref().take(n as u64).read_to_end(&mut bytes) {
                // A short read is the end: a source of unknown length, or a file that shrank.
                Ok(read) if read < n => self.eof = true,
                Ok(read) => {
                    if let Some(left) = self.remaining.as_mut() {
                        *left -= read as u64;
                        self.eof = *left == 0;
                    }
                }
                Err(error) => {
                    self.fail(&mut bytes, &mut on_error, &error);
                    continue;
                }
            }
            if !self.eof {
                if let Some(at) = last_line_break(&bytes[before..]) {
                    self.carried = bytes.split_off(before + at + 1);
                    return Some(bytes);
                }
                // No line break in what was read: a long line, read on twice as far.
                ask = ask.saturating_mul(2);
            }
        }
    }

    /// The rest of the source, from the bytes carried on: for a text that is not UTF-8, which is
    /// substituted whole.
    pub(super) fn take_rest(&mut self, on_error: impl FnOnce(&io::Error)) -> Vec<u8> {
        let mut bytes = std::mem::take(&mut self.carried);
        if !self.eof {
            if let Err(error) = self.reader.read_to_end(&mut bytes) {
                // patchcov: coverage tolerate reason="untestable: a read that fails after the source opened"
                on_error(&error);
                // patchcov: coverage end
            }
            self.eof = true;
        }
        bytes
    }

    /// A read failed: the source ends here, with the lines before the failure.
    fn fail(
        &mut self,
        bytes: &mut Vec<u8>,
        on_error: &mut Option<impl FnOnce(&io::Error)>,
        error: &io::Error,
    ) {
        self.eof = true;
        if let Some(report) = on_error.take() {
            report(error);
        }
        let kept = bytes
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |at| at + 1);
        bytes.truncate(kept);
    }
}

/// The file read again, for the wording of a fault: through a duplicate of the handle the run reads
/// by, so what it names is what the run read, whatever happened to the path since (#6920). Several
/// files are read again as the one stream jq reads them as (#7024).
pub(super) struct Reread {
    sources: Vec<RereadSource>,
    /// For a target that cannot read by position.
    #[cfg_attr(unix, allow(dead_code))]
    path: PathBuf,
    /// Each RS byte is read as a space, as the run reads a `--seq` stream (#7082): set once the
    /// prepass has vouched for the stream, after the reread is shared with the run.
    #[cfg_attr(not(unix), allow(dead_code))]
    blank_rs: std::cell::Cell<bool>,
    /// A stream that cannot be read again: its first fault, as its reader found it ([`Certifier`],
    /// #7024).
    seen: Option<SeenFault>,
    /// The first fault of a stream read again, once read, and the file it lies in
    /// ([`Reread::fault_source`]).
    memo: std::cell::OnceCell<(Option<String>, Option<usize>)>,
}

struct RereadSource {
    file: RereadFile,
    /// Where the stream starts in the file: 0 for a file named on the command line, and for a standard
    /// input that is a regular file the offset it was at, since the bytes before it were never the run's.
    #[cfg_attr(not(unix), allow(dead_code))]
    start: u64,
    /// How much of the file the run could read from `start`: what was left of it when it was opened.
    #[cfg_attr(not(unix), allow(dead_code))]
    len: u64,
}

/// The file a [`RereadSource`] reads: a handle the run holds, or a [`NamedFile`] the second read opens
/// when it reaches it (#7024), and declines if it is no longer the file the run read.
enum RereadFile {
    Held(Rc<File>),
    #[cfg_attr(not(unix), allow(dead_code))]
    Named(Rc<NamedFile>),
}

/// How much of a file the second read reads: a length of 0 is not known to be (`WindowedFile`), so the
/// file is read to its end.
fn known_len(len: u64) -> u64 {
    if len == 0 {
        u64::MAX
    } else {
        len
    }
}

/// How much of the file a chunk of the second read holds.
const REREAD_CHUNK: u64 = 1 << 20;

/// The most of one record the `--seq` prepass holds before it turns the stream down (#7082): a record
/// that big has the whole-file route, which holds it too, and a stream that opens an RS and never closes
/// it (NDJSON with one RS at its front) is not read into memory to be told it is not records.
#[cfg(unix)]
const MAX_PENDING_RECORD: usize = 64 << 20;

/// How much of the end of the file the `--seq` prepass reads first, to check the last record: a stream
/// that fails does so most often at its end, and the check then costs this, not a pass over the file.
#[cfg(unix)]
const TAIL_PROBE: u64 = 256 << 10;

/// How much of the file a chunk of the `--seq` prepass holds (#7082): small enough that the buffer comes
/// from the heap and is not a mapping that is freed, which glibc answers by raising the size it maps
/// from, so that the window buffers after it were carved from a heap that fragments (+6 MiB at 100 MiB).
#[cfg(unix)]
const PREPASS_CHUNK: usize = 64 << 10;

impl Reread {
    pub(super) fn new(file: &File, start: u64, len: u64, path: PathBuf) -> io::Result<Self> {
        Ok(Self {
            sources: vec![RereadSource {
                file: RereadFile::Held(Rc::new(file.try_clone()?)),
                start,
                len,
            }],
            path,
            blank_rs: std::cell::Cell::new(false),
            seen: None,
            memo: std::cell::OnceCell::new(),
        })
    }

    /// A stream that cannot be read again, whose reader keeps its first fault in `seen` as it reads
    /// ([`WindowedFile::from_pipe`], #7024).
    #[cfg_attr(not(unix), allow(dead_code))]
    pub(super) fn seen(seen: SeenFault) -> Self {
        Self {
            sources: Vec::new(),
            path: PathBuf::new(),
            blank_rs: std::cell::Cell::new(false),
            seen: Some(seen),
            memo: std::cell::OnceCell::new(),
        }
    }

    /// Several files, each from its start, `len` bytes of it.
    pub(super) fn new_many(files: &[(Rc<File>, u64)]) -> Self {
        let sources = files
            .iter()
            .map(|(file, len)| RereadSource {
                file: RereadFile::Held(Rc::clone(file)),
                start: 0,
                len: known_len(*len),
            })
            .collect();
        Self::of_sources(sources)
    }

    /// Several files opened by path, each when the second read reaches it ([`NamedFile`]).
    #[cfg_attr(not(unix), allow(dead_code))]
    pub(super) fn new_named(files: &[Rc<NamedFile>]) -> Self {
        let sources = files
            .iter()
            .map(|named| RereadSource {
                file: RereadFile::Named(Rc::clone(named)),
                start: 0,
                // The length is the one the run read, known once it reached the file.
                len: u64::MAX,
            })
            .collect();
        Self::of_sources(sources)
    }

    fn of_sources(sources: Vec<RereadSource>) -> Self {
        Self {
            sources,
            path: PathBuf::new(),
            blank_rs: std::cell::Cell::new(false),
            seen: None,
            memo: std::cell::OnceCell::new(),
        }
    }

    /// From now on the stream is read with each RS byte as a space (#7082).
    pub(super) fn blank_rs(&self) {
        self.blank_rs.set(true);
    }

    /// jq's parse error for the stream the file holds, or `None` when jq's parser reads it all or the
    /// file cannot be read again. A BOM is skipped at the front, as the run skips it, and the bytes
    /// are counted as read, an invalid one included, as jq's parser counts them.
    pub(super) fn stream_fault(&self) -> Option<String> {
        if let Some(seen) = &self.seen {
            return seen.get();
        }
        self.memo.get_or_init(|| self.read_fault()).0.clone()
    }

    /// The file the stream's first fault lies in, among the files (one that did not open counts, as
    /// a file of no bytes), or their count for a fault at the end of the stream: jq's reader opens the
    /// files up to that one and no further (#7024). `None` when the stream has no fault or cannot be
    /// read again.
    pub(super) fn fault_source(&self) -> Option<usize> {
        if self.seen.is_some() {
            return None;
        }
        self.memo.get_or_init(|| self.read_fault()).1
    }

    /// jq's parse error for the stream, and the file it lies in ([`Self::fault_source`]).
    #[cfg(unix)]
    fn read_fault(&self) -> (Option<String>, Option<usize>) {
        use std::os::unix::fs::FileExt;
        // The file of the last chunk handed to the parser, and whether the files ran out.
        let last_source = std::cell::Cell::new(0usize);
        let exhausted = std::cell::Cell::new(false);
        let failed = std::cell::Cell::new(false);
        let mut source = 0usize;
        let mut at = 0u64;
        let mut at_front = true;
        // The named file the read is in, opened when it is reached and closed when it is left, with
        // the length the run read of it.
        let mut opened: Option<(usize, File, u64)> = None;
        let chunks = std::iter::from_fn(|| loop {
            let Some(current) = self.sources.get(source) else {
                exhausted.set(true);
                return None;
            };
            if failed.get() {
                return None;
            }
            if at >= current.len {
                source += 1;
                at = 0;
                continue;
            }
            let (file, len) = match &current.file {
                RereadFile::Held(file) => (&**file, current.len),
                RereadFile::Named(named) => {
                    if !matches!(&opened, Some((at_source, _, _)) if *at_source == source) {
                        opened = None;
                        match named.reopen_as_read() {
                            Ok(Some((file, len))) => opened = Some((source, file, known_len(len))),
                            // Read as no bytes by the run: the next file follows.
                            Ok(None) => {
                                source += 1;
                                at = 0;
                                continue;
                            }
                            Err(_) => {
                                failed.set(true);
                                return None;
                            }
                        }
                    }
                    let (_, file, len) = opened.as_ref()?;
                    (file, *len)
                }
            };
            if at >= len {
                source += 1;
                at = 0;
                continue;
            }
            let want = (len - at).min(REREAD_CHUNK) as usize;
            let mut chunk = vec![0u8; want];
            return match file.read_at(&mut chunk, current.start + at) {
                // A file that gives out early is over, as it is to the run's reader, and the next
                // one follows.
                Ok(0) => {
                    source += 1;
                    at = 0;
                    continue;
                }
                Ok(read) => {
                    chunk.truncate(read);
                    if at_front {
                        at_front = false;
                        chunk.drain(..crate::front_matter::leading_bom_len(&chunk));
                    }
                    if self.blank_rs.get() {
                        blank_rs_bytes(&mut chunk);
                    }
                    at += read as u64;
                    last_source.set(source);
                    Some(chunk)
                }
                Err(_) => {
                    failed.set(true);
                    None
                }
            };
        });
        let fault = succinctly::jq::jq_stream_fault(chunks, 1, 0);
        if failed.get() || fault.is_none() {
            return (None, None);
        }
        let source = if exhausted.get() {
            self.sources.len()
        } else {
            last_source.get()
        };
        (fault, Some(source))
    }

    /// Whether the file is a `--seq` stream that is nothing but plain records (#7082): RFC 7464's own
    /// framing, which [`crate::jq_seq_reader::plain_record_sequence`] vouches for over a text held
    /// whole and this does a chunk at a time, by the same checker. So the run may read it as the plain
    /// JSON stream it is with each RS blanked. `false` is no verdict on the stream (a read that
    /// failed, or one the walk of jq's reader has to decide), and the whole-file route takes it.
    ///
    /// The memory is a chunk and the biggest record. The first and last bytes are asked of first, so a
    /// stream that is not framed at all is turned down by two reads, not by a pass over it.
    #[cfg(unix)]
    pub(super) fn is_plain_seq_records(&self) -> bool {
        use crate::jq_seq_reader::{PlainGuards, PlainRecords};
        use std::os::unix::fs::FileExt;
        let guards = PlainGuards::ALL;
        // One source: a stream of several files is not this route's.
        let [RereadSource {
            file: RereadFile::Held(file),
            start,
            len,
        }] = self.sources.as_slice()
        else {
            return false;
        };
        let (file, start, len) = (&**file, *start, *len);
        let byte_at = |at: u64| {
            let mut one = [0u8];
            match file.read_at(&mut one, start + at) {
                Ok(1) => Some(one[0]),
                _ => None,
            }
        };
        if len == 0 || byte_at(0) != Some(RS) || byte_at(len - 1) != Some(b'\n') {
            return false;
        }
        // The last record first, when an RS is near enough the end to hold it: the verdict does not
        // depend on the order, and a record that fails is most often the last (a write cut short).
        let probe = len.min(TAIL_PROBE);
        let mut tail = vec![0u8; probe as usize];
        if file.read_exact_at(&mut tail, start + len - probe).is_err() {
            return false; // patchcov: coverage tolerate-line reason="untestable: a read that fails right after the last byte was read"
        }
        if let Some(rs) = tail.iter().rposition(|&b| b == RS) {
            if !PlainRecords::record_ok(&tail[rs + 1..], guards) {
                return false;
            }
        }
        drop(tail);
        let mut check = PlainRecords::new(guards);
        let mut chunk = vec![0u8; PREPASS_CHUNK];
        let mut at = 0u64;
        while at < len {
            let want = (len - at).min(PREPASS_CHUNK as u64) as usize;
            match file.read_at(&mut chunk[..want], start + at) {
                Ok(read) if read > 0 => {
                    at += read as u64;
                    if !check.feed(&chunk[..read]) || check.pending_len() > MAX_PENDING_RECORD {
                        return false;
                    }
                }
                _ => return false, // patchcov: coverage tolerate-line reason="untestable: a file that shrinks or a read that fails between the passes"
            }
        }
        check.finish_with(&[])
    }

    #[cfg(not(unix))]
    pub(super) fn is_plain_seq_records(&self) -> bool {
        false
    }

    #[cfg(not(unix))]
    fn read_fault(&self) -> (Option<String>, Option<usize>) {
        let _ = (&self.sources, REREAD_CHUNK);
        (
            super::reread_stream_fault(std::slice::from_ref(&self.path)),
            None,
        )
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

    /// #7082: a window of a `--seq` stream reads each RS as a space, whatever the window, and nothing
    /// else moves: the documents and lines are those of the stream with its RS bytes blanked.
    #[test]
    fn rs_bytes_read_as_spaces_and_nothing_moves_7082() {
        let mut text = Vec::new();
        for n in 0..300 {
            text.push(0x1e);
            text.extend_from_slice(format!("{{\"n\":{n}}}\n").as_bytes());
        }
        let blanked: Vec<u8> = text
            .iter()
            .map(|&b| if b == 0x1e { b' ' } else { b })
            .collect();
        let want = whole(&blanked);
        for &block in BLOCKS {
            // The reader the run uses, which opens each file's `WindowedFile` itself.
            let mut file = tempfile::tempfile().expect("a temp file");
            file.write_all(&text).expect("written");
            file.rewind().expect("rewound");
            let mut reader =
                WindowedFiles::new(vec![(Rc::new(file), text.len() as u64)], block).blanking_rs();
            let (mut spans, mut seen) = (Vec::new(), Vec::new());
            while let Some((_, window)) = reader.next_window().expect("reads") {
                assert!(!window.raw.contains(&0x1e), "block {block}");
                for &(s, e) in &window.values {
                    spans.push((window.offset + s, window.offset + e));
                    seen.push(window.raw[s..e].to_vec());
                }
            }
            assert_eq!(spans, want.spans, "block {block}");
            assert_eq!(seen.len(), 300);
            assert_eq!(seen[7], b"{\"n\":7}", "block {block}");
        }
    }

    /// #7082: the prepass holds no more than a bounded part of a record. One that is longer is turned
    /// down, though it is a record, and the whole-file route reads it; a stream that opens an RS and
    /// never closes it is not read into memory to be told so.
    #[cfg(unix)]
    #[test]
    fn the_seq_prepass_turns_down_a_record_it_would_hold_whole_7082() {
        let framed = |spaces: usize| {
            let mut text = vec![0x1e];
            text.extend(std::iter::repeat_n(b' ', spaces));
            text.extend_from_slice(b"{}\n");
            let mut file = tempfile::tempfile().expect("a temp file");
            file.write_all(&text).expect("written");
            let len = text.len() as u64;
            Reread::new(&file, 0, len, PathBuf::new()).expect("a reread")
        };
        assert!(framed(1 << 20).is_plain_seq_records());
        assert!(framed(MAX_PENDING_RECORD - 1000).is_plain_seq_records());
        assert!(!framed(MAX_PENDING_RECORD + 1000).is_plain_seq_records());
    }

    /// #7082: a fault in a `--seq` stream is worded over the stream the run read, RS bytes as spaces, and a
    /// file shorter than it was when opened is no stream of records.
    #[cfg(unix)]
    #[test]
    fn the_reread_words_a_seq_stream_with_its_rs_as_spaces_7082() {
        let text = b"\x1e{\"a\":1}\n\x1e}\n";
        let reread = |len: u64| {
            let mut file = tempfile::tempfile().expect("a temp file");
            file.write_all(text).expect("written");
            Reread::new(&file, 0, len, PathBuf::new()).expect("a reread")
        };
        let plain = reread(text.len() as u64);
        let seq = reread(text.len() as u64);
        seq.blank_rs();
        let blanked: Vec<u8> = text
            .iter()
            .map(|&b| if b == 0x1e { b' ' } else { b })
            .collect();
        let want = succinctly::jq::jq_stream_fault(std::iter::once(blanked), 1, 0);
        assert!(want.is_some(), "the stray `}}` is a fault");
        assert_eq!(seq.stream_fault(), want);
        assert_ne!(plain.stream_fault(), want, "the RS bytes are not JSON");
        // A file that is shorter than the length it had when it was opened.
        assert!(!reread(text.len() as u64 + 10).is_plain_seq_records());
    }

    /// #7082: the prepass vouches for one stream of one source: several files, which the run's gate
    /// leaves to the route that reads them whole, are never records to it.
    #[cfg(unix)]
    #[test]
    fn the_seq_prepass_vouches_for_one_source_only_7082() {
        let text = b"\x1e{\"a\":1}\n";
        let open = || {
            let mut file = tempfile::tempfile().expect("a temp file");
            file.write_all(text).expect("written");
            Rc::new(file)
        };
        let len = text.len() as u64;
        assert!(Reread::new(&open(), 0, len, PathBuf::new())
            .expect("a reread")
            .is_plain_seq_records());
        assert!(!Reread::new_many(&[(open(), len), (open(), len)]).is_plain_seq_records());
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
    /// A reader that gives its bytes, then fails.
    struct Failing(io::Cursor<Vec<u8>>);

    impl Read for Failing {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.0.read(buf)? {
                0 => Err(io::Error::other("boom")),
                n => Ok(n),
            }
        }
    }

    /// #7023: the windows of a source are its lines, whole, in order, for any block and any length.
    #[test]
    fn line_windows_hand_out_the_sources_lines_whole() {
        let texts: [&[u8]; 6] = [
            b"",
            b"a",
            b"a\n",
            b"\n\n\n",
            b"one\ntwo\n\nthree",
            b"a long line with no break at all, then\nshort\n",
        ];
        for text in texts {
            for block in [1, 2, 3, 7, 64] {
                for known in [true, false] {
                    let mut windows = LineWindows::new(
                        Box::new(io::Cursor::new(text.to_vec())),
                        known.then_some(text.len() as u64),
                        block,
                        Vec::new(),
                    );
                    let mut all = Vec::new();
                    while let Some(window) = windows.next_window(None, |_| panic!("no error")) {
                        assert_ne!(window.len(), 0);
                        let ends_at_break = window.last() == Some(&b'\n');
                        all.extend_from_slice(&window);
                        // Every window but the last ends at a line break.
                        assert!(ends_at_break || windows.finished(), "{text:?} {block}");
                    }
                    assert_eq!(all, text, "{text:?} block {block} known {known}");
                    assert!(windows.finished());
                }
            }
        }
    }

    /// #7023: bytes carried in come first, `take_rest` is everything left, and a file that is
    /// shorter than it was ends at what it held.
    #[test]
    fn line_windows_carry_and_take_the_rest() {
        let mut windows = LineWindows::new(
            Box::new(io::Cursor::new(b"cd\nef\ngh".to_vec())),
            None,
            2,
            b"ab".to_vec(),
        );
        // The first read has no line break, so the second asks for twice as much.
        assert_eq!(
            windows.next_window(None, |_| ()),
            Some(b"abcd\nef\n".to_vec())
        );
        assert!(!windows.finished());
        assert_eq!(windows.take_rest(|_| ()), b"gh".to_vec());
        assert!(windows.finished());
        let mut shrunk = LineWindows::new(
            Box::new(io::Cursor::new(b"x\ny\n".to_vec())),
            Some(99),
            3,
            Vec::new(),
        );
        assert_eq!(shrunk.next_window(None, |_| ()), Some(b"x\n".to_vec()));
        assert_eq!(shrunk.next_window(None, |_| ()), Some(b"y\n".to_vec()));
        assert_eq!(shrunk.next_window(None, |_| ()), None);
        assert!(shrunk.finished());
    }

    /// #7024: bytes carried in that make a window already (a pipe's probe) are handed out as one, up to
    /// their last line break, before anything more is read: the probe is not grown by a block. The rest
    /// of the source follows, its first window led by what the probe held past that line break.
    #[test]
    fn a_carried_front_that_makes_a_window_is_handed_out_unread_7024() {
        struct Counted(io::Cursor<Vec<u8>>, Rc<std::cell::Cell<usize>>);
        impl io::Read for Counted {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                self.1.set(self.1.get() + 1);
                self.0.read(buf)
            }
        }
        let reads = Rc::new(std::cell::Cell::new(0));
        let mut windows = LineWindows::new(
            Box::new(Counted(
                io::Cursor::new(b"ef\ngh\n".to_vec()),
                Rc::clone(&reads),
            )),
            None,
            4,
            b"a\nbc\nd".to_vec(),
        );
        assert_eq!(windows.next_window(None, |_| ()), Some(b"a\nbc\n".to_vec()));
        assert_eq!(reads.get(), 0, "the front is a window without a read");
        let mut rest = Vec::new();
        while let Some(window) = windows.next_window(None, |_| ()) {
            rest.extend(window);
        }
        assert_eq!(rest, b"def\ngh\n".to_vec());
        // A front shorter than a block, or with no line break, is read on as before.
        let mut short = LineWindows::new(
            Box::new(io::Cursor::new(b"y\n".to_vec())),
            None,
            8,
            b"x\n".to_vec(),
        );
        assert_eq!(short.next_window(None, |_| ()), Some(b"x\ny\n".to_vec()));
    }

    /// #7023: a read that fails ends the source with the lines before it, and the line it cut is not
    /// handed out as if it were whole; the failure is reported once.
    #[test]
    fn line_windows_drop_the_line_a_failed_read_cut() {
        let mut windows = LineWindows::new(
            Box::new(Failing(io::Cursor::new(b"a\nb\ncut".to_vec()))),
            None,
            64,
            Vec::new(),
        );
        let mut reported = 0;
        let first = windows.next_window(None, |_| reported += 1);
        assert_eq!(first, Some(b"a\nb\n".to_vec()));
        assert_eq!(reported, 1);
        assert_eq!(windows.next_window(None, |_| reported += 1), None);
        assert_eq!(reported, 1);
    }

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
        let reread = Reread::new(&file, 0, len, path.clone()).expect("dup");
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
        let reread = Reread::new(&file, 0, 7, bom).expect("dup");
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

/// The several-files route against the whole-file route it replaces (#7024): the same bytes, cut into
/// files anywhere, give the same documents, the same file and line for each, the same seams and the same
/// fault.
#[cfg(test)]
mod stream_tests {
    use super::super::{
        bare_token_lost_to_fault, parse_error_line_after, stitch_json_file_seams,
        strip_leading_json_bom, utf8_lossy_document, LineCounter,
    };
    use super::*;
    use std::io::{Seek, Write};

    /// What a run over the files sees: each document with the file it is read in and its line, the
    /// seam of each file, and the first fault with its file and line.
    #[derive(Debug, PartialEq)]
    struct Run {
        docs: Vec<(usize, Vec<u8>, usize)>,
        seams: Vec<Seam>,
        fault: Option<(usize, usize)>,
    }

    /// The seam in the terms a reader of it can tell apart: without `raw_ends`, which the whole-file route
    /// sets for any substituted file and the windows for one whose front was substituted, and with the
    /// bytes it places clamped to the file's text, as `LineCounter::at_seam` clamps them (a file that
    /// hands its whole text on keeps the place of the bytes as read in the whole-file route).
    fn comparable(seam: &Seam, len: usize) -> Seam {
        Seam {
            base: seam.base.min(len),
            earlier: seam.earlier.as_ref().map(|parts| {
                parts
                    .iter()
                    .map(|part| super::super::SeamPart {
                        end: part.end.min(len),
                        ..part.clone()
                    })
                    .collect()
            }),
            raw_ends: None,
            ..seam.clone()
        }
    }

    fn whole_route(files: &[Vec<u8>]) -> Run {
        let mut raws: Vec<Vec<u8>> = files.to_vec();
        strip_leading_json_bom(raws.iter_mut());
        let mut seams = stitch_json_file_seams(raws.iter_mut(), false, false);
        let raws: Vec<Vec<u8>> = raws
            .into_iter()
            .enumerate()
            .map(|(index, raw)| utf8_lossy_document(raw, seams.get_mut(index)).0)
            .collect();
        let mut run = Run {
            docs: Vec::new(),
            seams: seams
                .iter()
                .zip(&raws)
                .map(|(seam, raw)| comparable(seam, raw.len()))
                .collect(),
            fault: None,
        };
        for (index, raw) in raws.iter().enumerate() {
            let (mut values, split_error) = split_json_values(raw);
            if let (Some(offset), Some(&last)) = (split_error, values.last()) {
                if bare_token_lost_to_fault(raw, last, offset, false) {
                    values.pop();
                }
            }
            let mut counter = LineCounter::at_seam(raw, seams[index].clone());
            for (start, end) in values {
                run.docs
                    .push((index, raw[start..end].to_vec(), counter.read_end_line(end)));
            }
            if let Some(offset) = split_error {
                let line = parse_error_line_after(raw, offset, seams[index].clone(), false, 0);
                run.fault = Some((index, line));
                break;
            }
        }
        run
    }

    fn windowed_route(files: &[Vec<u8>], block: usize) -> (Run, usize) {
        let handles: Vec<(Rc<File>, u64)> = files
            .iter()
            .map(|bytes| {
                let mut file = tempfile::tempfile().expect("a temp file");
                file.write_all(bytes).expect("written");
                file.rewind().expect("rewound");
                (Rc::new(file), bytes.len() as u64)
            })
            .collect();
        windowed_run(files, WindowedFiles::new(handles, block))
    }

    /// The files written under `dir`, to be opened by path ([`NamedFile`]): what a list longer than
    /// the handles the run holds is read as (#7024).
    fn named_files(dir: &std::path::Path, files: &[Vec<u8>]) -> Vec<Rc<NamedFile>> {
        files
            .iter()
            .enumerate()
            .map(|(n, bytes)| {
                let path = dir.join(format!("f{n}.json"));
                std::fs::write(&path, bytes).expect("written");
                Rc::new(NamedFile::new(path))
            })
            .collect()
    }

    fn windowed_named_route(files: &[Vec<u8>], block: usize) -> (Run, usize) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let named = named_files(dir.path(), files);
        windowed_run(files, WindowedFiles::named(&named, block))
    }

    fn windowed_run(files: &[Vec<u8>], mut reader: WindowedFiles) -> (Run, usize) {
        let mut run = Run {
            docs: Vec::new(),
            seams: vec![Seam::NONE; files.len()],
            fault: None,
        };
        let mut firsts = vec![true; files.len()];
        let mut ends = vec![(0usize, 0usize); files.len()];
        let mut peak = 0;
        while let Some((index, window)) = reader.next_window().expect("reads") {
            peak = peak.max(window.raw.len());
            let raw = window.raw;
            let mut values = window.values.clone();
            if let (Some(offset), Some(&last)) = (window.split_error, values.last()) {
                if bare_token_lost_to_fault(raw, last, offset, false) {
                    values.pop();
                }
            }
            let mut counter = LineCounter::at_seam(raw, window.seam.clone());
            counter.set_lines_before(window.lines_before);
            for (start, end) in values {
                run.docs
                    .push((index, raw[start..end].to_vec(), counter.read_end_line(end)));
            }
            // The seam: what the front took, from the file's first window, and what the end gave
            // up, from its last.
            if std::mem::take(&mut firsts[index]) {
                run.seams[index] = window.seam.clone();
            }
            ends[index] = (window.seam.base, raw.len());
            if window.seam.moved_len > 0 || window.seam.moved_newline.is_some() {
                run.seams[index].moved_len = window.seam.moved_len;
                run.seams[index].moved_newline = window.seam.moved_newline;
            }
            if let Some(offset) = window.split_error {
                let line = parse_error_line_after(
                    raw,
                    offset,
                    window.seam.clone(),
                    false,
                    window.lines_before,
                );
                run.fault = Some((index, line));
                break;
            }
        }
        run.seams = run
            .seams
            .iter()
            .zip(&ends)
            .map(|(seam, &(base, len))| comparable(seam, if base > 0 { len } else { usize::MAX }))
            .collect();
        (run, peak)
    }

    fn assert_same(files: &[Vec<u8>], block: usize) {
        let want = whole_route(files);
        // Held handles, and the same files opened a file at a time by path (#7024).
        assert_same_as(files, block, &want, windowed_route(files, block).0);
        assert_same_as(files, block, &want, windowed_named_route(files, block).0);
    }

    fn assert_same_as(files: &[Vec<u8>], block: usize, want: &Run, got: Run) {
        let shown: Vec<String> = files
            .iter()
            .map(|f| String::from_utf8_lossy(f).into_owned())
            .collect();
        assert_eq!(
            got.docs, want.docs,
            "documents of {shown:?} at block {block}"
        );
        assert_eq!(got.fault, want.fault, "fault of {shown:?} at block {block}");
        // The seams of the files up to the fault (the run stops there, and the windows after it
        // are never made).
        let upto = want.fault.map_or(files.len(), |(index, _)| index + 1);
        assert_eq!(
            got.seams[..upto],
            want.seams[..upto],
            "seams of {shown:?} at block {block}"
        );
    }

    const BLOCKS: &[usize] = &[1, 2, 3, 5, 8, 13, 64, 4096];

    fn cut(text: &[u8], at: &[usize]) -> Vec<Vec<u8>> {
        let mut files = Vec::new();
        let mut from = 0;
        for &to in at {
            files.push(text[from..to].to_vec());
            from = to;
        }
        files.push(text[from..].to_vec());
        files
    }

    #[test]
    fn whole_files_and_open_tails_read_as_the_whole_file_route_reads_them() {
        let streams: &[&[&[u8]]] = &[
            &[b"1\n2\n", b"3\n4\n"],
            &[b"{\"a\":1}\n", b"{\"a\":2}\n"],
            &[b"{\"a\":", b"1}\n{\"b\":2}\n"],
            &[b"1\n12", b"34\n5\n"],
            &[b"tr", b"ue\nfalse\n"],
            &[b"\"ab", b"cd\"\n1\n"],
            &[b"[1,\n2,", b"\n3]\n[4]\n"],
            &[b"1", b"", b"2\n"],
            &[b"[1,", b"", b"2]\n"],
            &[b"[1,", b"2,", b"3,", b"4]\n7\n"],
            &[b"", b"", b"1\n"],
            &[b"1\n", b"", b""],
            &[b"1\n2", b""],
            &[b"[1,2", b""],
            &[b"1\n2", b"\n3\n"],
            &[b"\xef\xbb\xbf1\n", b"2\n"],
            &[b"", b"\xef\xbb\xbf1\n"],
            &[b"1\n", b"\xef\xbb\xbf2\n"],
            &[b"1\n}\n", b"2\n"],
            &[b"1\n[1,\n", b"}\n2\n"],
            &[b"{\"a\":1}\n12", b"x\n3\n"],
            &[b"{\"s\":\"\xe2\x82", b"\xac\"}\n1\n"],
            &[b"\"\xe2", b"\x82\xac\"\n"],
            &[b"\"a\xff", b"\xffb\"\n"],
            &[b"\"a\xff", b"b\"\n9\n"],
            &[b"[\"x\xe2\x82", b"\xac\",\n\"\xc3", b"\xa9\"]\n"],
            &[b"1\n2\n3\n", b"\n\n", b"4 5\n6", b" 7\n"],
            &[b"{\"a\":\n", b"1}\n{\"a\":\n", b"2}\n"],
            &[b"{\"a\":[\n", b"1,\n", b"2]}\n"],
            // A carry with line breaks in it, the document it opens finished a few lines in, and more after.
            &[b"{\"a\":\n\"b\":", b"1}\n2\n3\n}\n"],
            &[b"[1,\n2,\n3,", b"4]\n5\n[6,\n", b"7]\n8\n"],
            &[b"1\n[\n\n\n", b"2]\n\n3\n"],
        ];
        for files in streams {
            let files: Vec<Vec<u8>> = files.iter().map(|f| f.to_vec()).collect();
            for &block in BLOCKS {
                assert_same(&files, block);
            }
        }
    }

    /// jq reads a line 4095 bytes at a time, and the line a document ends on is the count through the end
    /// of the read that completes it: a carry of more than that with a line break in it, and a document
    /// that finishes it a few bytes into the next file, put the later reads at offsets that only the carry's
    /// seam knows (#4316, #7024). Dropping the carry's first line, and the seam with it, moved them.
    #[test]
    fn a_long_carry_keeps_the_reads_of_the_first_line_aligned_7024() {
        let mut first = b"1\n[\n".to_vec();
        first.extend(std::iter::repeat_n(*b"1,", 2045).flatten());
        let mut second = b"1] 7 [\n".to_vec();
        second.extend(std::iter::repeat_n(*b"1\n", 40).flatten());
        second.extend_from_slice(b"]\n8\n");
        let files = [first, second];
        for &block in BLOCKS {
            assert_same(&files, block);
        }
        // The same with the line of the open document longer than two reads.
        let mut first = b"[\n".to_vec();
        first.extend(std::iter::repeat_n(*b"1,", 5000).flatten());
        let mut second = b"1] 7 [\n".to_vec();
        second.extend(std::iter::repeat_n(*b"1\n", 40).flatten());
        second.extend_from_slice(b"]\n8\n");
        for &block in BLOCKS {
            assert_same(&[first.clone(), second.clone()], block);
        }
    }

    /// Every way to cut a stream into two or three files, and several block sizes.
    #[test]
    fn a_stream_cut_anywhere_reads_as_the_whole_file_route_reads_it() {
        let texts: &[&[u8]] = &[
            b"{\"a\":1}\n[1,\n2]\n\"x y\"\n12 true\nnull\n",
            b"{\"k\":\"caf\xc3\xa9 \xe2\x82\xac\"}\n1\n",
            b"\"a\xffb\" [1,\n\xfe]\n7\n",
            b"\xef\xbb\xbf{\"a\":1}\n2",
            b"1\n2 }\n3\n",
            b"{\"a\":1}\n[1 2]\n3\n",
            b"nul\n1\n",
            b"[[[1,\n2],\n3],\n4]\n5\n",
        ];
        for text in texts {
            for first in 0..=text.len() {
                assert_same(&cut(text, &[first]), 3);
                assert_same(&cut(text, &[first]), 4096);
                for second in (first..=text.len()).step_by(3) {
                    assert_same(&cut(text, &[first, second]), 2);
                }
            }
        }
    }

    #[test]
    fn random_streams_cut_into_files_read_as_the_whole_file_route_reads_them() {
        let mut seed = 0x1234_5678_9abc_def0u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let pieces: &[&[u8]] = &[
            b"{\"a\":1}",
            b"[1,2,\n3]",
            b"12",
            b"true",
            b"\"str\"",
            b"\"caf\xc3\xa9\"",
            b"\"\xe2\x82\xac\"",
            b"{\"x\":{\"y\":[null,\n1.5e3]}}",
            b"\n",
            b"\n\n",
            b" ",
            b"\t",
            b"null",
            b"\"a\xffb\"",
            b"\"\xe2\x82\"",
            b"[",
            b"]",
            b"}",
            b"{\"k\":",
            b"x1",
            b"-0.5",
            b"\"esc \\\" \\\\ \\u00e9\"",
        ];
        for round in 0..3000 {
            let mut text = Vec::new();
            if round % 17 == 0 {
                text.extend_from_slice(b"\xef\xbb\xbf");
            }
            for _ in 0..(next() % 12) {
                text.extend_from_slice(pieces[(next() % pieces.len() as u64) as usize]);
                if next() % 3 != 0 {
                    text.push(if next() % 2 == 0 { b'\n' } else { b' ' });
                }
            }
            let mut cuts: Vec<usize> = (0..(next() % 4))
                .map(|_| (next() % (text.len() as u64 + 1)) as usize)
                .collect();
            cuts.sort_unstable();
            let files = cut(&text, &cuts);
            for &block in &[1usize, 3, 7, 16, 100, 1 << 20] {
                assert_same(&files, block);
            }
        }
    }

    /// What a window holds is a window and the document being run, not the files: many documents over
    /// several files, a token open at each cut, never hold more than a few windows' text.
    #[test]
    fn the_text_held_is_a_window_not_the_files_7024() {
        let record = |i: usize| format!("{{\"id\":{i},\"name\":\"user {i}\"}}\n");
        let text: Vec<u8> = (0..30_000).flat_map(|i| record(i).into_bytes()).collect();
        let at = [text.len() / 3 + 7, 2 * text.len() / 3 + 3];
        let files = cut(&text, &at);
        let (run, peak) = windowed_route(&files, 4096);
        assert_eq!(run.docs.len(), 30_000);
        assert!(peak < 4 * 4096, "held {peak} bytes of {}", text.len());
    }

    /// An invalid byte substituted in one file makes every later window say so: a message that names a
    /// position is worded from the files read again, as the whole-file route does for any substituted
    /// file.
    #[test]
    fn a_substitution_in_an_earlier_file_is_remembered_7024() {
        let files = [b"{\"a\":\"\xff\"}\n1".to_vec(), b"\n2\n".to_vec()];
        let handles: Vec<(Rc<File>, u64)> = files
            .iter()
            .map(|bytes| {
                let mut file = tempfile::tempfile().expect("a temp file");
                file.write_all(bytes).expect("written");
                file.rewind().expect("rewound");
                (Rc::new(file), bytes.len() as u64)
            })
            .collect();
        let mut reader = WindowedFiles::new(handles, 4096);
        let mut seen = Vec::new();
        while let Some((index, window)) = reader.next_window().expect("reads") {
            seen.push((index, window.substituted));
        }
        assert_eq!(seen, vec![(0, true), (1, true)]);
    }

    fn temp_file(bytes: &[u8]) -> File {
        let mut file = tempfile::tempfile().expect("a temp file");
        file.write_all(bytes).expect("written");
        file.rewind().expect("rewound");
        file
    }

    /// A length of 0 is not known to be (`stat` says it of a `/proc` file that has content): the file is
    /// read to its end, and a file that holds less than it was said to is over at its end.
    #[test]
    fn a_length_that_is_not_known_or_not_true_is_read_to_the_end_7024() {
        let mut reader = WindowedFiles::new(
            vec![
                (Rc::new(temp_file(b"1\n2\n")), 0),
                (Rc::new(temp_file(b"3\n")), 0),
                (Rc::new(temp_file(b"4\n5\n")), 100),
            ],
            4096,
        );
        let mut seen = Vec::new();
        while let Some((index, window)) = reader.next_window().expect("reads") {
            for &(start, end) in &window.values {
                seen.push((index, window.raw[start..end].to_vec()));
            }
        }
        let want: Vec<(usize, Vec<u8>)> = [(0, "1"), (0, "2"), (1, "3"), (2, "4"), (2, "5")]
            .iter()
            .map(|&(index, text)| (index, text.as_bytes().to_vec()))
            .collect();
        assert_eq!(seen, want);
        // A file whose `stat` length is 0 and which has bytes is not an empty one: a BOM at the front of
        // the next is a byte of the stream, not the front of it.
        let mut reader = WindowedFiles::new(
            vec![
                (Rc::new(temp_file(b"1\n")), 0),
                (Rc::new(temp_file(b"\xef\xbb\xbf2\n")), 5),
            ],
            4096,
        );
        let mut faulted = None;
        while let Some((index, window)) = reader.next_window().expect("reads") {
            if window.split_error.is_some() {
                faulted = Some(index);
            }
        }
        assert_eq!(faulted, Some(1));
        // The second read of the stream, for a fault's wording, goes on to the next file when one gives out
        // early, and reads the one of unknown length to its end.
        let (a, b, c) = (
            Rc::new(temp_file(b"1\n")),
            Rc::new(temp_file(b"2\n")),
            Rc::new(temp_file(b"}\n")),
        );
        let reread = Reread::new_many(&[(a, 100), (b, 0), (c, 2)]);
        assert_eq!(
            reread.stream_fault().as_deref(),
            Some("Unmatched '}' at line 3, column 1")
        );
    }

    /// A file with no line break to cut a window at (one long document) is read on for all of what is left
    /// in one go, once the reads that asked for twice as much are 4 MiB: for a file whose length is known the
    /// rest of it, and for one whose length is not a few windows at a time.
    #[test]
    fn one_long_line_is_read_on_whether_or_not_the_length_is_known_7024() {
        let mut text = b"[1".to_vec();
        text.extend(std::iter::repeat_n(*b",1", 4_500_000).flatten());
        text.extend_from_slice(b"]\n");
        for len in [text.len() as u64, 0] {
            let mut reader = WindowedFiles::new(vec![(Rc::new(temp_file(&text)), len)], 1 << 16);
            let mut documents = Vec::new();
            while let Some((_, window)) = reader.next_window().expect("reads") {
                documents.extend(window.values.iter().map(|&(start, end)| end - start));
            }
            assert_eq!(documents, vec![text.len() - 1], "length {len}");
        }
    }

    /// A read that failed is returned after its file's last window, so the documents before it are run, and
    /// the files after it are not reached by the caller that stops there.
    #[test]
    fn a_failed_read_is_returned_after_the_files_last_window_7024() {
        let mut reader = WindowedFiles::new(
            vec![
                (Rc::new(temp_file(b"1\n")), 2),
                (Rc::new(temp_file(b"2\n")), 2),
            ],
            4096,
        );
        assert!(reader.reach_a_file_with_windows_left().expect("opens"));
        reader.current.as_mut().expect("a file").read_error = Some(io::Error::other("injected"));
        let (index, window) = reader.next_window().expect("reads").expect("a window");
        assert_eq!((index, window.values.len()), (0, 1));
        let Err(error) = reader.next_window() else {
            panic!("the failure follows the last window");
        };
        assert_eq!(error.to_string(), "injected");
        let (index, window) = reader.next_window().expect("reads").expect("a window");
        assert_eq!((index, window.values.len()), (1, 1));
        assert!(reader.next_window().expect("done").is_none());
    }

    /// Files judged by path are read a file at a time, whatever their number, as the whole-file route
    /// reads them (#7024): two hundred files, a token open where many of them end.
    #[test]
    fn many_named_files_read_as_the_whole_file_route_reads_them_7024() {
        let text: Vec<u8> = (0..2_000)
            .flat_map(|i| format!("{{\"id\":{i},\"s\":\"x{i}\"}}\n").into_bytes())
            .collect();
        let at: Vec<usize> = (1..200).map(|n| n * text.len() / 200 + n % 7).collect();
        let files = cut(&text, &at);
        let want = whole_route(&files);
        assert_eq!(want.docs.len(), 2_000);
        for block in [64, 4096] {
            assert_same_as(&files, block, &want, windowed_named_route(&files, block).0);
        }
    }

    /// The second read of files opened by path words the stream's first fault as jq's parser does over
    /// the files joined, opening each in turn, and names the file the fault lies in, which is as far as
    /// jq's reader goes; it reads each file as the run read it, so a file the run read as no bytes is
    /// left out, and it declines once a file the run read is no longer at its path.
    #[cfg(unix)]
    #[test]
    fn named_files_are_read_again_as_the_run_read_them_7024() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let files = [
            b"{\"a\":1}\n{\"a\":".to_vec(),
            b"2}\n{\"b\":[1 2]}\n".to_vec(),
            b"{\"c\":3}\n".to_vec(),
        ];
        let named = named_files(dir.path(), &files);
        let mut reader = WindowedFiles::named(&named, 4096);
        while reader.next_window().expect("reads").is_some() {}
        let reread = Reread::new_named(&named);
        let joined: Vec<u8> = files.concat();
        let want = succinctly::jq::jq_stream_fault(std::iter::once(joined), 1, 0);
        assert!(want.is_some(), "the stream has a fault");
        assert_eq!(reread.stream_fault(), want);
        assert_eq!(
            reread.fault_source(),
            Some(1),
            "the fault is in the second file"
        );
        // Replaced after the run read it: the second read declines.
        let other = dir.path().join("other.json");
        std::fs::write(&other, &files[1]).expect("written");
        std::fs::rename(&other, dir.path().join("f1.json")).expect("replaced");
        assert_eq!(Reread::new_named(&named).stream_fault(), None);
        // A file the run could not open is no bytes to the second read, and a fault at the end of
        // the stream is past every file.
        let named = named_files(
            dir.path(),
            &[b"1\n[".to_vec(), b"2\n".to_vec(), b"3".to_vec()],
        );
        std::fs::remove_file(dir.path().join("f1.json")).expect("removed");
        let mut reader = WindowedFiles::named(&named, 4096);
        while reader.next_window().expect("reads").is_some() {}
        let reread = Reread::new_named(&named);
        let want = succinctly::jq::jq_stream_fault(std::iter::once(&b"1\n[3"[..]), 1, 0);
        assert_eq!(reread.stream_fault(), want);
        assert_eq!(reread.fault_source(), Some(3));
    }

    /// A file opened by path that will not open when the run reaches it is left out of the stream, as
    /// the whole-file route leaves out a file it cannot open, so the open tail before it continues in
    /// the file after it; its failure is handed to the run with its position. A last one is read as no
    /// bytes, which ends the tail there. A file replaced at its path is read as it is now, as jq reads
    /// what the path names when it gets there.
    #[cfg(unix)]
    #[test]
    fn a_named_file_is_read_as_its_path_stands_when_the_run_reaches_it_7024() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let files = [
            b"1\n12".to_vec(),
            b"9\n".to_vec(),
            b"34\n".to_vec(),
            b"5".to_vec(),
        ];
        let named = named_files(dir.path(), &files);
        std::fs::remove_file(dir.path().join("f1.json")).expect("removed");
        std::fs::remove_file(dir.path().join("f3.json")).expect("removed");
        let mut reader = WindowedFiles::named(&named, 4096);
        let mut docs = Vec::new();
        let mut failed = Vec::new();
        let mut note = |at: usize, failure: super::super::InputFileOpenError| {
            failed.push((at, failure.to_string()));
        };
        while let Some((index, window)) = reader.next_window_noting(&mut note).expect("reads") {
            for &(start, end) in &window.values {
                docs.push((index, window.raw[start..end].to_vec()));
            }
        }
        assert_eq!(docs, vec![(0, b"1".to_vec()), (2, b"1234".to_vec())]);
        let names: Vec<String> = ["f1.json", "f3.json"]
            .iter()
            .map(|name| dir.path().join(name).display().to_string())
            .collect();
        assert_eq!(
            failed,
            vec![
                (
                    1,
                    format!(
                        "error: Could not open file {}: No such file or directory",
                        names[0]
                    )
                ),
                (
                    3,
                    format!(
                        "error: Could not open file {}: No such file or directory",
                        names[1]
                    )
                ),
            ]
        );
        // A tail held at a last file that does not open ends there, in that file's place.
        let named = named_files(dir.path(), &[b"1\n77".to_vec(), b"8\n".to_vec()]);
        std::fs::remove_file(dir.path().join("f1.json")).expect("removed");
        let mut reader = WindowedFiles::named(&named, 4096);
        let mut docs = Vec::new();
        while let Some((index, window)) = reader.next_window_noting(&mut |_, _| {}).expect("reads")
        {
            for &(start, end) in &window.values {
                docs.push((index, window.raw[start..end].to_vec()));
            }
        }
        assert_eq!(docs, vec![(0, b"1".to_vec()), (1, b"77".to_vec())]);
        // Replaced: the new file is read, and the second read reads it too.
        let named = named_files(dir.path(), &[b"1\n".to_vec(), b"2\n".to_vec()]);
        let other = dir.path().join("other.json");
        std::fs::write(&other, b"20\n21\n").expect("written");
        std::fs::rename(&other, dir.path().join("f1.json")).expect("replaced");
        let mut reader = WindowedFiles::named(&named, 4096);
        let mut count = 0;
        while let Some((_, window)) = reader.next_window().expect("reads") {
            count += window.values.len();
        }
        assert_eq!(count, 3);
        assert_eq!(Reread::new_named(&named).stream_fault(), None);
    }
}

/// A pipe on standard input (#7024): read once, a window at a time, with its first fault found as its
/// windows are handed out, where a regular file reads itself again for it.
#[cfg(test)]
mod pipe_tests {
    use super::super::LineCounter;
    use super::*;
    use std::io::{Seek, SeekFrom, Write};

    /// A backslash, so the escapes below are spelled where they are built.
    const BS: char = '\\';

    /// What a run over `text` sees: each document (as text) and its line, the fault the splitter stops
    /// at, and the largest window held.
    #[derive(Debug, PartialEq)]
    struct Run {
        docs: Vec<(Vec<u8>, usize)>,
        fault: Option<usize>,
    }

    fn drain(reader: &mut WindowedFile) -> (Run, usize) {
        let mut run = Run {
            docs: Vec::new(),
            fault: None,
        };
        let mut peak = 0;
        while let Some(window) = reader.next_window().expect("reads") {
            peak = peak.max(window.raw.len());
            let mut counter = LineCounter::new(window.raw);
            counter.set_lines_before(window.lines_before);
            for &(start, end) in &window.values {
                run.docs
                    .push((window.raw[start..end].to_vec(), counter.read_end_line(end)));
            }
            run.fault = window.split_error.map(|at| window.offset + at);
        }
        (run, peak)
    }

    /// `text` read as a pipe: the first `front` bytes as the probe took them, the rest from the source.
    /// A file stands in for the pipe, which the reader reads only in order (a real one is below).
    fn piped(text: &[u8], front: usize, block: usize) -> (Run, usize, Option<String>) {
        let mut file = tempfile::tempfile().expect("a temp file");
        file.write_all(text).expect("written");
        let front = front.min(text.len());
        file.seek(SeekFrom::Start(front as u64)).expect("seeked");
        let seen = SeenFault::default();
        let mut reader = WindowedFile::from_pipe(
            Rc::new(file),
            text[..front].to_vec(),
            None,
            block,
            seen.clone(),
        );
        let (run, peak) = drain(&mut reader);
        let fault = Reread::seen(seen).stream_fault();
        (run, peak, fault)
    }

    /// The same text as a regular file, which words its fault by reading itself again.
    fn as_file(text: &[u8], block: usize) -> (Run, Option<String>) {
        let mut file = tempfile::tempfile().expect("a temp file");
        file.write_all(text).expect("written");
        file.rewind().expect("rewound");
        let reread = Reread::new(&file, 0, text.len() as u64, PathBuf::new()).expect("reread");
        let mut reader = WindowedFile::new(file, text.len() as u64, block);
        (drain(&mut reader).0, reread.stream_fault())
    }

    fn assert_same(text: &[u8], block: usize) {
        let (file_run, file_fault) = as_file(text, block);
        // The fault is the whole stream's first, as jq's parser words it.
        let bom = crate::front_matter::leading_bom_len(text);
        assert_eq!(
            file_fault,
            succinctly::jq::jq_stream_fault([&text[bom..]], 1, 0),
            "the file route at block {block}: {:?}",
            String::from_utf8_lossy(text)
        );
        for front in [0, 1, block, text.len() / 2, text.len()] {
            let (run, _, fault) = piped(text, front, block);
            assert_eq!(
                run,
                file_run,
                "documents at block {block}, front {front}: {:?}",
                String::from_utf8_lossy(text)
            );
            assert_eq!(
                fault,
                file_fault,
                "fault at block {block}, front {front}: {:?}",
                String::from_utf8_lossy(text)
            );
        }
    }

    /// The documents of the many-shapes test, and ones the splitter delimits that jq's parser turns
    /// down, or that only jq's parser can certify: a fault inside a document a filter may never read is
    /// still the stream's first, wherever the window that held it went.
    fn line(pick: u64) -> Vec<u8> {
        let deep = |n: usize| format!("{}{}\n", "[".repeat(n), "]".repeat(n));
        match pick % 24 {
            0 => b"{\"a\":[1,2,{\"b\":null}]}\n".to_vec(),
            1 => b"[1,\n 2,\n 3]\n".to_vec(),
            2 => b"\"str\" 12 true\n".to_vec(),
            3 => b"\n\n".to_vec(),
            4 => b"   {} []  \n".to_vec(),
            5 => b"-1.5e-3\n".to_vec(),
            6 => b"{\"k\":\"v\"}{\"k\":\"w\"}\n".to_vec(),
            7 => b"null\n".to_vec(),
            8 => "{\"s\":\"caf\u{e9} \u{20ac} \u{1f600}\"}\n".into(),
            9 => format!("{{\"e\":\"a{BS}\"b{BS}{BS}c{BS}u00e9{BS}n\"}}\n").into(),
            10 => b"{\"k\":1}\r\n".to_vec(),
            11 => b"{\"deep\":{\"x\":[[]]}}\n".to_vec(),
            // Only jq's parser can tell these are fine.
            12 => b"nan 01 [NaN,-Infinity]\n".to_vec(),
            13 => deep(140).into(),
            14 => b"{\"s\":\"\xff\xfe\"}\n".to_vec(),
            15 => b"0012\n".to_vec(),
            // And these are faults the splitter does not see.
            16 => b"{\"m\":[1 2]}\n".to_vec(),
            17 => b"{\"m\" 1}\n".to_vec(),
            18 => b"[1,,2]\n".to_vec(),
            19 => format!("[\"{BS}x\"]\n").into(),
            20 => b"{\"t\":\"a\tb\"}\n".to_vec(),
            21 => format!("{{\"u\":\"{BS}ud800\"}}\n").into(),
            22 => deep(300).into(),
            _ => b"[1,\xff]\n".to_vec(),
        }
    }

    #[test]
    fn a_pipe_words_the_first_fault_wherever_its_window_went_7024() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for round in 0..80 {
            let mut text = Vec::new();
            if round % 9 == 5 {
                text.extend_from_slice(b"\xEF\xBB\xBF");
            }
            for _ in 0..(5 + next() % 40) {
                // Mostly documents that are fine, so the fault is often deep in the stream.
                let pick = next();
                let pick = if pick % 3 == 0 { pick } else { pick % 12 };
                text.extend(line(pick));
            }
            if round % 4 == 1 {
                text.pop();
            }
            if round % 5 == 2 {
                text.extend_from_slice(b"}\n");
            }
            if round % 7 == 3 {
                text.extend_from_slice(b"[1,\n");
            }
            if round % 11 == 4 {
                text.extend_from_slice(b"12x\n{\"after\":1}\n");
            }
            for &block in &[1usize, 3, 7, 16, 100, 1 << 20] {
                assert_same(&text, block);
            }
        }
    }

    #[test]
    fn each_shape_alone_and_after_others_words_as_the_file_does_7024() {
        for pick in 0..24 {
            for before in [&b""[..], b"{\"x\":1}\n", b"\n\n  \n", b"12 "] {
                for after in [&b""[..], b"{\"y\":2}\n", b"\n\n", b"]", b"{"] {
                    let text = [before, &line(pick), after].concat();
                    for &block in &[1usize, 5, 64, 1 << 20] {
                        assert_same(&text, block);
                    }
                }
            }
        }
    }

    #[test]
    fn a_pipe_holds_a_window_not_the_stream_7024() {
        let text = "{\"name\":\"n\",\"v\":[1,2,3]}\n".repeat(4_000);
        let (run, peak, fault) = piped(text.as_bytes(), 1_000, 1_000);
        assert_eq!(run.docs.len(), 4_000);
        assert_eq!(fault, None);
        assert!(peak < 4_000, "{peak}");
    }

    /// A real pipe, written while it is read, with a fault in an early document that is long gone by the
    /// end, and the front a probe took.
    #[cfg(unix)]
    #[test]
    fn a_real_pipe_is_read_in_windows_and_keeps_its_first_fault_7024() {
        let mut text = b"{\"a\":1}\n{\"b\":[1 2]}\n".to_vec();
        text.extend("{\"c\":\"ccccccccccccccc\"}\n".repeat(20_000).as_bytes());
        text.extend_from_slice(b"{\"d\":");
        let (reader, mut writer) = std::io::pipe().expect("a pipe");
        let written = text.clone();
        let feeder = std::thread::spawn(move || writer.write_all(&written).expect("written"));
        let mut file = File::from(std::os::fd::OwnedFd::from(reader));
        let mut front = vec![0u8; 100];
        file.read_exact(&mut front).expect("the front");
        let seen = SeenFault::default();
        let mut windows = WindowedFile::from_pipe(Rc::new(file), front, None, 4_096, seen.clone());
        let (run, peak) = drain(&mut windows);
        feeder.join().expect("the feeder");
        assert_eq!(run.docs.len(), 20_002);
        assert!(run.fault.is_some());
        assert!(peak < 16_384, "{peak}");
        assert_eq!(
            seen.get(),
            succinctly::jq::jq_stream_fault([&text[..]], 1, 0)
        );
        assert_eq!(
            seen.get().as_deref(),
            Some("Expected separator between values at line 2, column 10")
        );
    }

    /// A window costs jq's parser one read at most, however many of its documents the validator turns
    /// down: a stream of `NaN`s (jq reads them; the validator does not) is read once, not once a document.
    #[test]
    fn documents_the_validator_turns_down_cost_one_read_a_window_7024() {
        let mut text = "{\"v\":NaN}\n".repeat(5_000).into_bytes();
        text.extend_from_slice(b"{\"x\":");
        let mut file = tempfile::tempfile().expect("a temp file");
        file.write_all(&text).expect("written");
        file.rewind().expect("rewound");
        let seen = SeenFault::default();
        let mut reader =
            WindowedFile::from_pipe(Rc::new(file), Vec::new(), None, 4_096, seen.clone());
        let mut windows = 0;
        while reader.next_window().expect("reads").is_some() {
            windows += 1;
        }
        assert!(windows > 5, "{windows}");
        assert!(
            reader.exact_reads() <= windows,
            "{} reads, {windows} windows",
            reader.exact_reads()
        );
        assert_eq!(
            seen.get(),
            succinctly::jq::jq_stream_fault([&text[..]], 1, 0)
        );
    }

    /// A reader of the fault waits for the windows handed to the certifier: what it reads is what the
    /// windows certified before the filter ran would have kept (#7106).
    #[test]
    fn the_fault_is_read_after_the_windows_in_flight_are_certified_7106() {
        let seen = SeenFault::default();
        seen.begin();
        seen.begin();
        let certifier = {
            let seen = seen.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(50));
                seen.finish(Some("the first".into()), None, false);
                std::thread::sleep(std::time::Duration::from_millis(50));
                seen.finish(Some("a later one".into()), None, false);
            })
        };
        // Both windows are certified by the time it answers, and the first fault is the stream's.
        assert_eq!(seen.get().as_deref(), Some("the first"));
        assert_eq!(seen.lock().in_flight, 0);
        certifier.join().expect("the certifier");
    }

    /// A window waits for a place while two others are in flight, so the copies of the text stay at two.
    #[test]
    fn a_third_window_waits_for_one_of_two_in_flight_7106() {
        let seen = SeenFault::default();
        seen.begin();
        seen.begin();
        let certifier = {
            let seen = seen.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(50));
                seen.finish(None, None, false);
            })
        };
        seen.begin();
        // Only after one of the two was certified: two are in flight again, not three.
        assert_eq!(seen.lock().in_flight, CERTIFIER_WINDOWS_IN_FLIGHT);
        certifier.join().expect("the certifier");
    }

    /// A certifier that panicked is not read as a stream without a fault.
    #[test]
    #[should_panic(expected = "certifies the windows panicked")]
    fn a_certifier_that_panicked_is_not_a_stream_without_a_fault_7106() {
        let seen = SeenFault::default();
        seen.begin();
        seen.finish(None, None, true);
        seen.get();
    }

    /// Only a pipe starts the thread: a regular file's windows are not certified at all.
    #[test]
    fn a_regular_file_starts_no_certifier_7106() {
        let text = "{\"a\":1}\n".repeat(2_000);
        let mut file = tempfile::tempfile().expect("a temp file");
        file.write_all(text.as_bytes()).expect("written");
        file.rewind().expect("rewound");
        let mut reader = WindowedFile::new(file, text.len() as u64, 4_096);
        while reader.next_window().expect("reads").is_some() {}
        assert!(reader.certifier.is_none());
    }

    /// A pipe starts one thread with its first window, and its windows reach it through two buffers that
    /// are used again, however many there are.
    #[test]
    fn a_pipe_certifies_many_windows_on_one_thread_7106() {
        let text = "{\"a\":1,\"b\":[1,2,3]}\n".repeat(40_000);
        let mut file = tempfile::tempfile().expect("a temp file");
        file.write_all(text.as_bytes()).expect("written");
        file.rewind().expect("rewound");
        let seen = SeenFault::default();
        let mut reader =
            WindowedFile::from_pipe(Rc::new(file), Vec::new(), None, 4_096, seen.clone());
        let mut windows = 0;
        let mut buffers = std::collections::HashSet::new();
        while reader.next_window().expect("reads").is_some() {
            windows += 1;
        }
        assert!(windows > 100, "{windows}");
        assert_eq!(seen.get(), None);
        assert_eq!(reader.exact_reads(), 0);
        // The recycled buffers: only a few were ever made (each holds a window of 4 KiB or so).
        let worker = reader
            .certifier
            .as_ref()
            .and_then(|c| c.worker.as_ref())
            .expect("started");
        while let Ok(buffer) = worker.recycled.try_recv() {
            buffers.insert(buffer.as_ptr() as usize);
        }
        assert!(
            buffers.len() <= CERTIFIER_WINDOWS_IN_FLIGHT + 1,
            "{}",
            buffers.len()
        );
    }

    /// A window over the copy limit (one document bigger than a few windows) is certified on the filter's
    /// own thread, over the buffer, and a fault after it is worded as the whole stream words it.
    #[test]
    fn a_window_over_the_copy_limit_is_certified_without_a_copy_7106() {
        let mut text = b"{\"a\":1}\n".to_vec();
        text.push(b'[');
        text.extend("1,".repeat(CERTIFIER_COPY_LIMIT).as_bytes());
        text.extend_from_slice(b"2]\n{\"x\":");
        // The whole stream is what the probe read, so the first window holds it all.
        let file = tempfile::tempfile().expect("a temp file");
        let seen = SeenFault::default();
        let mut reader =
            WindowedFile::from_pipe(Rc::new(file), text.clone(), None, 4_096, seen.clone());
        while reader.next_window().expect("reads").is_some() {}
        assert!(seen.get().is_some());
        assert_eq!(
            seen.get(),
            succinctly::jq::jq_stream_fault([&text[..]], 1, 0)
        );
        // Certified here, over the buffer: no thread was started for it.
        assert!(reader.certifier.as_ref().expect("a pipe").worker.is_none());
    }

    /// Where no thread could be started every window is certified on the filter's own thread, and the
    /// answer is the one the thread gives (#7106).
    #[test]
    fn a_pipe_with_no_thread_certifies_as_the_thread_does_7106() {
        let mut text = "{\"a\":1,\"b\":[1,2,3]}\n".repeat(3_000).into_bytes();
        text.extend_from_slice(b"{\"v\":NaN}\n{\"w\":[1 2]}\n");
        text.extend("{\"c\":2}\n".repeat(3_000).as_bytes());
        let (mut faults, mut docs) = (Vec::new(), Vec::new());
        for threadless in [false, true] {
            let mut file = tempfile::tempfile().expect("a temp file");
            file.write_all(&text).expect("written");
            file.rewind().expect("rewound");
            let seen = SeenFault::default();
            let mut reader =
                WindowedFile::from_pipe(Rc::new(file), Vec::new(), None, 4_096, seen.clone());
            reader.certifier.as_mut().expect("a pipe").threadless = threadless;
            let (run, _) = drain(&mut reader);
            docs.push(run.docs.len());
            let started = reader.certifier.as_ref().expect("a pipe").worker.is_some();
            assert_eq!(started, !threadless);
            faults.push(seen.get());
        }
        assert!(faults[0].is_some() && docs[0] > 3_000);
        assert_eq!(docs[0], docs[1]);
        assert_eq!(faults[0], faults[1]);
        assert_eq!(
            faults[0],
            succinctly::jq::jq_stream_fault([&text[..]], 1, 0)
        );
    }

    /// Bytes that are not UTF-8 in a region whose front a window has dropped are given back as read, and
    /// a fault after them is placed in the bytes as read.
    #[test]
    fn a_region_whose_front_is_dropped_reads_as_it_was_read_7024() {
        let mut text = Vec::new();
        for i in 0..400 {
            text.extend_from_slice(format!("{{\"i\":{i},\"s\":\"").as_bytes());
            text.extend_from_slice(b"\xff\xfe");
            text.extend_from_slice(b"\"}\n");
        }
        text.extend_from_slice(b"[\xff,1 2]\n");
        for &block in &[64usize, 1_000, 1 << 20] {
            assert_same(&text, block);
        }
    }

    /// A probe that fails after a BOM: the BOM is skipped as a read would skip it, and the documents
    /// before the failure are run.
    #[test]
    fn a_failed_first_read_still_skips_the_bom_7024() {
        let file = tempfile::tempfile().expect("a temp file");
        let mut reader = WindowedFile::from_pipe(
            Rc::new(file),
            b"\xEF\xBB\xBF1\n2\n".to_vec(),
            Some(io::Error::new(io::ErrorKind::Other, "gone")),
            16,
            SeenFault::default(),
        );
        let window = reader.next_window().expect("the bytes before it");
        assert_eq!(window.map(|w| w.values.len()), Some(2));
        assert!(reader.next_window().is_err());
    }

    /// A probe that failed: the bytes before the failure are run, and the failure is returned after them.
    #[test]
    fn a_failed_first_read_is_returned_after_its_bytes_7024() {
        let file = tempfile::tempfile().expect("a temp file");
        let mut reader = WindowedFile::from_pipe(
            Rc::new(file),
            b"1\n2\n".to_vec(),
            Some(io::Error::new(io::ErrorKind::Other, "gone")),
            16,
            SeenFault::default(),
        );
        let window = reader.next_window().expect("the bytes before it");
        assert_eq!(window.map(|w| w.values.len()), Some(2));
        assert!(reader.next_window().is_err());
    }
}
