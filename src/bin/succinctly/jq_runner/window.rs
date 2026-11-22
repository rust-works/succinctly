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
    advance_position, count_newlines, open_tail_after_split_reading, split_json_values, Seam,
    SeamStitcher,
};

/// How much of the file one window reads, and so about how much memory the route holds besides a
/// document that is bigger than that.
///
/// The route reads [`window_bytes`], which is this constant in every build that ships. The limits
/// below that are multiples of it are functions of that for the same reason.
///
/// 256 KiB since #7412, 1 MiB before: the window, and the two a pipe holds while its certifying
/// thread reads one, were most of a streaming run's private memory (2.0 of 2.0 MiB RssAnon for
/// `jq -r .name` over a 100 MB file on the 7950X, where jq 1.7.1 holds 0.8; 3.2 over a pipe), and a
/// quarter of the window costs no time on the 7950X or the M4 Pro: a window of 256 KiB is still
/// thousands of records, so what a window costs on its own (a read, a scan's setup, a hand-off to the
/// certifying thread) stays a small part of it.
pub(super) const WINDOW_BYTES: usize = 1 << 18;

/// The environment variable a **debug build only** reads a smaller window from (#7376), so a CLI
/// test can run the route over a fixture of tens of KiB where the real window needs 4 MiB. It is
/// compiled out of a release build (`cfg(debug_assertions)`): there the name is not in the binary
/// and [`window_bytes`] is the constant. Unit tests do not read it either (they pass the window in,
/// or use the `cfg(test)` overrides), so a variable left in the environment cannot move them.
#[cfg(any(test, debug_assertions))]
pub(super) const DEBUG_WINDOW_ENV: &str = "SUCCINCTLY_DEBUG_JQ_WINDOW_BYTES";

/// The smallest window [`DEBUG_WINDOW_ENV`] may name: two of the pages a `--seq` gate's first read
/// takes ([`SEQ_UNOPENED_BLOCK_BYTES`]), so that read stays smaller than a window as it is in production.
#[cfg(any(test, debug_assertions))]
const DEBUG_WINDOW_MIN_BYTES: usize = 2 * SEQ_UNOPENED_BLOCK_BYTES;

/// The window [`DEBUG_WINDOW_ENV`] asks for: `None` when it is not set, and a refusal for anything
/// but a decimal count of bytes from [`DEBUG_WINDOW_MIN_BYTES`] to [`WINDOW_BYTES`]. A value that is
/// not usable is refused, not ignored: a test whose small fixture ran on the 1 MiB window would
/// read its file whole and still pass, having tested nothing.
#[cfg(any(test, debug_assertions))]
fn debug_window_bytes(value: Option<&std::ffi::OsStr>) -> Result<Option<usize>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let refusal = || {
        format!(
            "{DEBUG_WINDOW_ENV}={} is not a count of bytes from {DEBUG_WINDOW_MIN_BYTES} to {WINDOW_BYTES}",
            value.to_string_lossy()
        )
    };
    let text = value.to_str().ok_or_else(refusal)?;
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(refusal());
    }
    match text.parse::<usize>() {
        Ok(bytes) if (DEBUG_WINDOW_MIN_BYTES..=WINDOW_BYTES).contains(&bytes) => Ok(Some(bytes)),
        _ => Err(refusal()),
    }
}

/// [`WINDOW_BYTES`], or in a debug build what [`DEBUG_WINDOW_ENV`] names (#7376). Read once.
#[cfg(all(debug_assertions, not(test)))]
pub(super) fn window_bytes() -> usize {
    static BYTES: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *BYTES.get_or_init(
        || match debug_window_bytes(std::env::var_os(DEBUG_WINDOW_ENV).as_deref()) {
            Ok(bytes) => bytes.unwrap_or(WINDOW_BYTES),
            Err(refusal) => {
                eprintln!("succinctly: {refusal} (a debug-build test knob, #7376)");
                std::process::exit(2);
            }
        },
    )
}

/// [`WINDOW_BYTES`]: a release build and the unit tests have no other window.
#[cfg(any(test, not(debug_assertions)))]
#[inline(always)]
pub(super) const fn window_bytes() -> usize {
    WINDOW_BYTES
}

/// The environment variable that has a run read every input whole (#7688): a regular file, a
/// descriptor, a pipe, standard input and a named pipe each decline the windowed routes' gates, as
/// `/dev/fd/N` once did by accident. The differential fuzz (`scripts/jq-windowed-file-fuzz.py`) sets it
/// on its oracle, so the oracle stays the whole-file route however the gates change; unlike
/// [`DEBUG_WINDOW_ENV`] it is in a release build too, since the fuzz runs against one. Any non-empty
/// value but `0` forces it.
pub(super) const WHOLE_FILE_ENV: &str = "SUCCINCTLY_DEBUG_JQ_WHOLE_FILE";

/// The environment variable naming a file each windowed route a run takes appends its name to, one
/// line each (#7688): the fuzz reads it to learn that the side under test went windowed and that its
/// oracle did not, so a case that compares a route with itself is no longer a silent pass.
pub(super) const ROUTE_LOG_ENV: &str = "SUCCINCTLY_DEBUG_JQ_ROUTE_LOG";

/// Whether `value`, the content of [`WHOLE_FILE_ENV`], forces the whole-file route.
fn forces_whole_file(value: Option<&std::ffi::OsStr>) -> bool {
    value.is_some_and(|value| !value.is_empty() && value != "0")
}

/// Whether the run reads every input whole ([`WHOLE_FILE_ENV`]). Asked by the gates every windowed
/// route goes through, which a run that never reaches one does not pay for.
pub(super) fn whole_file_forced() -> bool {
    forces_whole_file(std::env::var_os(WHOLE_FILE_ENV).as_deref())
}

/// Records that the run took the windowed route `route` ([`ROUTE_LOG_ENV`]); nothing when the
/// variable is not set. A run's routes are decided once, before its first window is read.
pub(super) fn note_route(route: &str) {
    use std::io::Write;
    let Some(path) = std::env::var_os(ROUTE_LOG_ENV) else {
        return;
    };
    if let Ok(mut log) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(log, "{route}");
    }
}

/// The smallest file the route reads a window at a time, four windows (1 MiB in every build that
/// ships, 4 MiB before #7412): below this it is not worth the second route, and the file is read whole
/// as before.
#[inline]
pub(super) fn min_windowed_file_bytes() -> u64 {
    4 * window_bytes() as u64
}

/// The smallest lone `--seq` regular file (named, or on standard input) the route reads a window at a
/// time, a sixteenth of a window (16 KiB in every build that ships, 64 KiB before #7412), where the
/// plain route waits for
/// [`min_windowed_file_bytes`] (#7260). The whole-file route walks a `--seq` stream that is not plain
/// records with jq's reader and converts every record to an owned value, which cost a 1 MiB stream with
/// one bad record 17-18% of its time against the windows that carry on past it (#7322); a stream that
/// is plain records takes as long either way, so the windows take both. Below this the run is its
/// startup, and the file keeps the route that reads it whole.
#[inline]
pub(super) fn min_windowed_seq_file_bytes() -> u64 {
    (window_bytes() / 16) as u64
}

/// The fewest bytes a pipe on standard input holds for the plain and `--seq` routes to read it a
/// window at a time, two windows (512 KiB in every build that ships, #7412; 2 MiB from #7260): its
/// length is not known, so
/// that many are read first to learn it, and they are what the run holds at its peak. Since a pipe's
/// windows share their buffers with the thread that certifies them and the probe is freed a block at a
/// time, a windowed pipe holds about two windows, which a pipe that is read whole holds once it is a
/// little over one: four windows held a probe of 4 MiB for a run whose windows hold 2.
#[inline]
pub(super) fn min_windowed_pipe_bytes() -> u64 {
    2 * window_bytes() as u64
}

/// The window of a lone `--seq` file whose first byte is not an RS (#7312): [`SeqGate`] hands the stream
/// on at that byte whatever is read, so the first read, which is all the gate looks at, is a page and
/// not a [`WINDOW_BYTES`] block that is filled, found to be all tail and freed, which the allocator
/// then keeps resident beside the tail (1 MiB at the run's peak).
const SEQ_UNOPENED_BLOCK_BYTES: usize = 4096;

const _: () = assert!(SEQ_UNOPENED_BLOCK_BYTES < WINDOW_BYTES);

/// The window size for a `--seq` file read from byte `start`: [`WINDOW_BYTES`], or
/// [`SEQ_UNOPENED_BLOCK_BYTES`] when the stream does not open with an RS, which the gate hands on at
/// its first byte (#7312). Only the size of the reads differs, never the answers: the windows of a
/// stream that is handed on at once are empty. A file that cannot be read at `start` gets the full
/// block.
#[cfg(unix)]
pub(super) fn seq_block_bytes(file: &File, start: u64) -> usize {
    use std::os::unix::fs::FileExt;
    let mut first = [0u8; 1];
    match file.read_at(&mut first, start) {
        Ok(read) if read == 0 || first[0] != RS => SEQ_UNOPENED_BLOCK_BYTES,
        _ => window_bytes(),
    }
}

/// How many windows a document that does not fit in one is read on to before the rest of the file is.
const LOOKAHEAD_WINDOWS: usize = 8;

/// A file shorter than this many windows (4 MiB in every build that ships) whose window ends inside a
/// document is read on to its end at once ([`WindowedFile::small_enough_to_read_whole`], #7412).
const MAX_READ_WHOLE_WINDOWS: usize = 16;

/// The record separator of a `--seq` stream.
const RS: u8 = 0x1e;

/// The most bytes a scan of jq's reads leaves unread at the end of a piece: less than one read
/// ([`crate::cli::jq_seq_reader::JQ_FGETS_CHUNK`]), so a piece with this much room past what is
/// left always reads on (#7260).
const JQ_READ_CARRY: usize = crate::cli::jq_seq_reader::JQ_FGETS_CHUNK;

/// Reads each RS byte of `bytes` as a space (#7082).
fn blank_rs_bytes(bytes: &mut [u8]) {
    for b in bytes {
        if *b == RS {
            *b = b' ';
        }
    }
}

/// Reads every byte that is not a line break as a space: the bytes of a `--seq` stretch that jq's
/// reader made no value of (#7322).
fn blank_between_values(bytes: &mut [u8]) {
    for b in bytes {
        if *b != b'\n' {
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
    /// The warnings of jq's `--seq` reader for the stretches of this window that were not plain
    /// records (#7322), in order, each with the offset in `raw` it was met at: it is printed before
    /// the first of `values` that starts after that offset. Empty for any other window.
    pub notes: Vec<(usize, String)>,
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
    /// The carry is text the file before validated whole, with nothing substituted and no NUL in it
    /// (#7750, #7792): it is not validated again, which for one value spread over many files was every
    /// byte before each file boundary again.
    pub carry_validated: bool,
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
            carry_validated: false,
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
    /// How much of the file is left of the length it had when it was opened, less what was read: the
    /// reads ask for no more than that while there is some, and once it is read they go on to the end
    /// the file has then, as jq's reader reads it to the read that returns nothing (#7446). So a line a
    /// writer appends while the run reads is read, and a run whose output is appended to its input
    /// through a pipe (`jq . f | tee -a f`) reads what it wrote, as jq does, a window at a time.
    remaining: u64,
    /// The length is not known (`stat` said 0, as it does of an empty file and of a file of the `/proc`
    /// kind that is not): the file is read to its end, as `std::fs::read` reads it.
    unknown_len: bool,
    /// The reads end at the length the file had ([`Self::remaining`]), not at the end it has then: a
    /// `--seq` file whose records the prepass vouched for up to that length (#7082), which has no gate
    /// to judge records read past it ([`Reread::is_plain_seq_records`]).
    stop_at_len: bool,
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
    /// The bytes of the BOM taken off the front of the file, which jq's first read of its first line
    /// holds (#7437).
    front_lead: usize,
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
    /// The last window's text, once `buf` is shared with the certifier that waits for a reader of the
    /// fault to certify it ([`SeenFault::defer_last`], #7294).
    held: Option<Arc<Vec<u8>>>,
    /// A `--seq` pipe (#7164): only its records that are RFC 7464's own framing are handed out, and the
    /// stream from the first that is not is handed on ([`SeqGate`]).
    seq_gate: Option<SeqGate>,
    /// The gate's stream is the run's only source ([`Self::from_seq_pipe`]), whose tail goes to the
    /// route that reads a stream whole, not a file of a `--seq` list (#7260, [`Self::no_rs_end`]).
    lone_seq: bool,
    /// The warnings of the stretches the gate resumed after (#7322) that no window has handed out
    /// yet, in order, each with where in `buf` jq's reader met it ([`Window::notes`]).
    notes: Vec<(usize, String)>,
    /// The gate ended the text validated so far after a stretch, short of what is read, because
    /// [`MAX_QUEUED_NOTES`] warnings were waiting (#7341): the window it makes is not the last, even
    /// once the stream is read to its end.
    gate_cut: bool,
    /// Where the bytes a pipe's probe read end in `buf` (0 once they are dropped, and for every other
    /// source): while they hold a block past `validated`, a window is made of them and nothing is read
    /// onto them (#7266). Reading a block onto the probe's 4 MiB grew its buffer to 8 MiB, and made a
    /// first window of 5 MiB that the certifier copied whole.
    front_end: usize,
    /// Two blocks past where a pipe's probe ends in the stream (0 for every other source): a document
    /// still open there is read on a block at a time up to it before the look-ahead (#7266).
    probe_reach: u64,
    /// The rest of a pipe's probe, read in blocks of their own (#7260): each read takes the next of
    /// them before the pipe is read, so a block of the probe is freed once its window is out, where
    /// one buffer of the probe was held until all of it was.
    pre_read: std::collections::VecDeque<Vec<u8>>,
    /// The windows whose kept bytes went to a buffer of their own because the certifying thread still
    /// read them ([`Self::drop_consumed`], #7260).
    #[cfg(test)]
    kept_apart: usize,
    #[cfg(test)]
    break_search_bytes: usize,
    /// The bytes the splitter has scanned, rescans of a document not yet whole included.
    #[cfg(test)]
    split_bytes: usize,
    /// jq's parser after reading the carry on the front of `buf`, one container or string the files
    /// before left open (#7750), and how far into `buf` it has read: while the value stays open, the
    /// text is not split again from its first byte at each read and at each file boundary.
    tail_reader: Option<succinctly::jq::JqOpenTail>,
    tail_read_to: usize,
    /// A NUL was found in the text validated so far (#7437), which the validation changed.
    nul_seen: bool,
    /// How much of the front was validated before the file was read: a carry the file before
    /// validated whole ([`Place::carry_validated`]), or 0.
    front_validated: usize,
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
        let carry_validated = if place.carry_validated { carry_len } else { 0 };
        Self {
            file,
            remaining: if len == 0 { u64::MAX } else { len },
            unknown_len: len == 0,
            stop_at_len: false,
            block: block.max(1),
            // The carry is on the front, unread as text: its lines are not searched for a break,
            // so the first region the text is judged in covers all of it.
            carry_raw: Vec::new(),
            buf: place.carry,
            validated: carry_validated,
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
            front_lead: 0,
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
            held: None,
            seq_gate: None,
            lone_seq: false,
            notes: Vec::new(),
            gate_cut: false,
            front_end: 0,
            probe_reach: 0,
            pre_read: std::collections::VecDeque::new(),
            #[cfg(test)]
            kept_apart: 0,
            #[cfg(test)]
            break_search_bytes: 0,
            #[cfg(test)]
            split_bytes: 0,
            tail_reader: None,
            tail_read_to: 0,
            nul_seen: false,
            front_validated: carry_validated,
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
        // #7266: a pipe's probe that still holds bytes past the text handed out gives the window,
        // ending at its last line break within a block of what is validated, with no read onto it,
        // so a stream of lines never grows the probe's buffer. A window of the probe with no
        // document that ends in it (one over several lines and longer than a block), or whose last
        // document is open over half a block, is read as the probe handed out whole was: the rest
        // of the probe and a read onto it, of the room the windows before freed in its buffer
        // ([`Self::room_or_block`]), then the look-ahead below. A document still open then that
        // starts after the stream's front, where the whole probe's first window had run the
        // documents before it, is read on a block at a time to two blocks past the probe first, as
        // far as that reader had read when a window of it alone looked ahead. None of it holds or
        // scans more than the probe handed out whole did.
        let mut in_front = true;
        // #7341: the text the gate cut short is followed by bytes read and not yet judged, which
        // the gate goes on through before anything more is read onto them.
        let mut after_cut = self.gate_cut;
        loop {
            let from_front = in_front && !self.eof && self.front_end > self.validated;
            if from_front {
                // The read that would have skipped it.
                self.skip_front_bom();
            } else if !self.eof && !after_cut {
                self.read_more(grow);
            }
            after_cut = false;
            let before = self.validated;
            self.validate_lines(
                from_front.then(|| (self.validated + self.block).min(self.front_end)),
            );
            // A `--seq` pipe's gate that handed the stream on ended it (#7277): what is validated is
            // the last window, as after a read.
            if from_front && self.validated == before && !self.eof {
                // What is left of the probe is part of a line: the next read completes it.
                in_front = false;
                first = false;
                continue;
            }
            if !self.eof && self.validated == before && !first {
                // No line break came with that read: the next asks for twice as much, so a file with
                // few of them is read in a linear amount of work, and once that is several windows
                // (it is one long document, or a file of one line) for all of what is left, as
                // the whole-file route reads it.
                grow = if grow >= 4 * window_bytes() || self.small_enough_to_read_whole() {
                    self.rest()
                } else {
                    grow.saturating_mul(2)
                };
                continue;
            }
            first = false;
            let from = self.skip;
            // #7750: a value the files before left open, still open after what this read added, is no
            // document yet, as the splitter would find of it from its first byte.
            let carried_open = self.carried_value_open(from);
            #[cfg(test)]
            if !carried_open {
                self.split_bytes += self.validated - from;
            }
            let (mut values, split_error) = if carried_open {
                (Vec::new(), Some(0))
            } else {
                split_json_values(&self.buf[from..self.validated])
            };
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
                let tail = if carried_open {
                    Some((from, self.tail_reader.take()))
                } else {
                    open_tail_after_split_reading(text, split_error, values.last().copied())
                };
                if let Some((start, reader)) = tail {
                    values.retain(|&(_, end)| end <= start);
                    split_error = None;
                    tail_start = Some(start);
                    stitcher.reader = reader;
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
            // #7266: a window of the probe whose last document is open over half a block or more
            // would hand that much out to be copied again with the next window: it is read as a
            // window with no document that ends in it is (below), the rest of the probe and a block.
            if pending && from_front {
                let open_from = values.last().map_or(self.skip, |&(_, end)| end);
                if !values.is_empty() && self.validated - open_from >= self.block / 2 {
                    in_front = false;
                    grow = self.room_or_block();
                    continue;
                }
            }
            // #7341: a text the gate cut short because its warnings were waiting is handed out to print
            // them, values or not.
            let cut_with_notes = self.gate_cut && !self.notes.is_empty();
            if self.eof || definitive || !values.is_empty() || cut_with_notes {
                // #7341: a text the gate cut short is followed by more of the stream, read or not.
                let last = (self.eof && !self.gate_cut) || definitive;
                if last {
                    self.done = true;
                }
                // A cut text with no value is white space the next window does not need.
                let empty = if cut_with_notes && !pending {
                    self.validated
                } else {
                    self.skip
                };
                self.consumed = values.last().map_or(empty, |&(_, end)| end);
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
                    // #7750, #7792: text that needs nothing done to it is the next file's front as is.
                    // Only a container or string jq's parser reads on (`stitcher.reader`): the split
                    // waits for it to close, where a bare number or literal on its own would be split
                    // as a whole value before the next file's bytes that go on with it are read.
                    let validated = self.raw_map.regions.is_empty()
                        && !self.nul_seen
                        && !self.blank_rs
                        && self.seq_gate.is_none()
                        && stitcher.reader.is_some();
                    stitcher.move_tail(self.index, &mut moved, start_raw, tail, &[], false);
                    stitcher.carry_validated = validated;
                    seam.moved_len = moved.moved_len;
                    seam.moved_newline = moved.moved_newline;
                }
                // Where the next file starts in the stream: nothing else reads it. #7341: a file of a
                // `--seq` list its own gate reads holds no tail, and a file may follow it.
                if last && (self.hold_tail || self.seq_gate.is_some()) {
                    self.end_origin = advance_position(self.origin, &self.buf[..text_end]);
                }
                // The stream's first fault, if this window holds it, found before any of it runs.
                if self.certifier.is_some() {
                    let fault_at_end = last && split_error.is_some() && self.read_error.is_none();
                    self.certify(&values, fault_at_end);
                }
                // #7322: the warnings met in the text this window ends. #7341: the last window takes every
                // one left: a warning met at the RS the stream is handed on from (a stretch's piece
                // ends on it) comes after this window's values, and before anything handed on.
                let met = if last {
                    self.notes.len()
                } else {
                    self.notes.partition_point(|&(at, _)| at < self.validated)
                };
                let notes: Vec<(usize, String)> = self.notes.drain(..met).collect();
                let text = self.held.as_deref().unwrap_or(&self.buf);
                return Ok(Some(Window {
                    raw: &text[..text_end],
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
                    notes,
                }));
            }
            // Nothing to run, and the file goes on: white space is dropped, and a document that did
            // not scan is read on. The scan of an incomplete document is lost, so doubling costs
            // two scans of a document in all: a document bigger than a window is read on to eight
            // windows once, and past that, which is a file that is one document or has a very big
            // one, for the rest of the file, as the whole-file route reads it.
            if pending && from_front {
                // #7266: the rest of the probe, and the room it leaves (or a block onto it).
                in_front = false;
                grow = self.room_or_block();
            } else if pending && self.offset > 0 && self.file_read < self.probe_reach {
                grow = self.block;
            } else if pending {
                let lookahead = self.block.saturating_mul(LOOKAHEAD_WINDOWS);
                grow = if self.buf.len() >= lookahead || self.small_enough_to_read_whole() {
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

    /// Whether the carry on the front of `buf`, a container or string the files before left open, is
    /// still open once jq's parser has read the bytes validated since it last read (#7750). The carry
    /// is where the text starts (`from` is 0), and no byte of the text was substituted, so the parser's
    /// position is the buffer's. Once the value closes or faults, or the text no longer is that, the
    /// reader is dropped and the text is split from its front as before, once. The last file of the
    /// list does not read on with it: nothing follows to hand a tail to.
    fn carried_value_open(&mut self, from: usize) -> bool {
        let Some(reader) = self.tail_reader.as_mut() else {
            return false;
        };
        // The last file hands nothing on: its text is split as it is read, which it must be anyway.
        if from != 0
            || !self.hold_tail
            || !self.raw_map.regions.is_empty()
            || self.tail_read_to > self.validated
        {
            self.tail_reader = None;
            return false;
        }
        let read = reader.read(&self.buf[..self.validated], self.tail_read_to);
        self.tail_read_to = self.validated;
        if read == succinctly::jq::TailRead::Open {
            debug_assert!(
                self.validated > 1 << 20
                    || split_json_values(&self.buf[..self.validated]) == (Vec::new(), Some(0)),
                "the reader of a carried value answers as the splitter does from its front"
            );
            return true;
        }
        self.tail_reader = None;
        false
    }

    /// #7412: a file of a known length under [`MAX_READ_WHOLE_WINDOWS`] windows whose window ends inside
    /// a document is read on to its end at once, as the route that read it whole read it while the
    /// window was 1 MiB and a file under 4 MiB did not take this route. Read on in doubling steps, its
    /// buffer was grown three times on the way to the length (and copied each time), which held a
    /// 3 MB file of one document 1.4 MB more on the M4 Pro than reading it whole.
    fn small_enough_to_read_whole(&self) -> bool {
        !self.unknown_len
            && self.file_read.saturating_add(self.remaining)
                < (MAX_READ_WHOLE_WINDOWS * self.block) as u64
    }

    /// What a read onto a pipe's probe asks for once a document in it is open past its window
    /// (#7266): the room the windows handed out of the probe have freed in its buffer, so it is not
    /// grown, or with no room a block, which grows it as the probe handed out whole grew it.
    fn room_or_block(&self) -> usize {
        match self.buf.capacity() - self.buf.len() {
            0 => self.block,
            room => room,
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
    ///
    /// #7260: the last window's text may be shared with the thread that certifies it ([`Self::held`]):
    /// it is the buffer again once the thread is done with it, and while it is not, what is kept goes
    /// to a buffer of its own, so no window is copied for the thread.
    fn drop_consumed(&mut self) {
        let held = self.held.take();
        let (consumed, line_start, dropped) = {
            let text = held.as_deref().unwrap_or(&self.buf);
            let consumed = self.consumed.min(text.len());
            let mut line_start = text[..consumed]
                .iter()
                .rposition(|&b| b == b'\n')
                .map_or(0, |at| at + 1);
            let base = self.seam.base;
            if line_start < base {
                line_start = 0;
            }
            let dropped = (line_start > 0).then(|| {
                (
                    count_newlines(&text[..line_start]),
                    count_newlines(&text[..base]),
                )
            });
            (consumed, line_start, dropped)
        };
        let mut kept_apart = false;
        if let Some(held) = held {
            self.buf = match Arc::try_unwrap(held) {
                Ok(buf) => buf,
                Err(held) => {
                    kept_apart = true;
                    #[cfg(test)]
                    {
                        self.kept_apart += 1;
                    }
                    let mut buf = self.spare_buffer(held.len() - line_start, held.capacity());
                    buf.extend_from_slice(&held[line_start..]);
                    buf
                }
            };
        }
        if line_start > 0 {
            self.offset += line_start;
            if !kept_apart {
                self.buf.drain(..line_start);
            }
            self.validated -= line_start;
        }
        self.after_front_drop(line_start, dropped);
        self.skip = consumed - line_start;
        self.consumed = self.skip;
    }

    /// What [`Self::drop_consumed`] keeps track of when `line_start` bytes are dropped from the front of
    /// the text, with `dropped` line breaks in them and the carry's own before `seam.base`.
    fn after_front_drop(&mut self, line_start: usize, dropped: Option<(usize, usize)>) {
        let base = self.seam.base;
        if let Some((dropped, carry_lines)) = dropped {
            self.lines_before += dropped - carry_lines;
            self.origin = (self.origin.0 + dropped, 0);
            self.front_end = self.front_end.saturating_sub(line_start);
            self.no_break_from = self.no_break_from.saturating_sub(line_start);
            if let Some(gate) = self.seq_gate.as_mut() {
                gate.dropped(line_start, dropped);
            }
            // A warning met in text dropped with no value to run comes before whatever is next.
            for (at, _) in &mut self.notes {
                *at = at.saturating_sub(line_start);
            }
            if base > 0 {
                self.seam = Seam::NONE;
                self.seam_raw = Seam::NONE;
                self.carry_raw = Vec::new();
            }
            // The text starts on a later line now, whose reads start at its front (#7569).
            self.seam.lead = 0;
            self.raw_map.drop_before(self.offset);
        }
    }

    /// A buffer to read the next window into with `kept` bytes of the last at its front, while the
    /// certifying thread still reads that one (#7260): one it has handed back, or a new one of a block
    /// and those bytes, but no bigger than the one it stands in for (`room`), as the windows of one
    /// buffer never grew it.
    fn spare_buffer(&self, kept: usize, room: usize) -> Vec<u8> {
        let spare = self
            .certifier
            .as_ref()
            .and_then(|certifier| certifier.worker.as_ref())
            .and_then(|worker| worker.recycled.try_recv().ok());
        let mut buf = spare.unwrap_or_default();
        buf.clear();
        // The next read takes a block of the probe, which these bytes are put in front of.
        buf.reserve(if self.pre_read.is_empty() {
            (kept + self.block).min(room.max(kept))
        } else {
            kept
        });
        buf
    }

    /// The bytes of the open tail of the file from `start` in the text, as they were read, which is
    /// what the next file's text is judged with (the substitution is of the two together).
    fn raw_tail(&mut self, start: usize) -> Vec<u8> {
        let at = self.offset + start;
        if !self.raw_map.changes_from(at) {
            // #7750, #7792: the whole text is the tail (one value goes on into the next file), which
            // is handed on as it is, not copied: copied, every file boundary copied the value so far.
            if start == 0 && self.validated == self.buf.len() && self.held.is_none() {
                return std::mem::take(&mut self.buf);
            }
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

    /// All that is left of the file, for a read that wants it in one go (the file's length is not known,
    /// or what it had is read and the file is read on past it, #7446: a few windows).
    fn rest(&self) -> usize {
        if self.unknown_len || self.remaining == 0 {
            4 * window_bytes()
        } else {
            usize::try_from(self.remaining).unwrap_or(usize::MAX)
        }
    }

    /// Reads up to `n` more bytes; a short read is the end of the file (and the length it had, for a file
    /// that stops there, [`Self::stop_at_len`]), and an error is kept to be returned after the bytes
    /// before it have been run. The read that reaches the length the file had is followed by one more
    /// ([`Self::read_on_past_len`]), as jq's reader reads on to the read that returns nothing (#7446).
    fn read_more(&mut self, n: usize) {
        if let Some(block) = self.pre_read.pop_front() {
            // #7260: the probe's next block, read before the pipe is, with what is kept in front of it;
            // a buffer that holds less than it is put on its front, so the block is not copied.
            if self.buf.len() <= block.len() {
                let mut block = block;
                block.splice(..0, self.buf.drain(..));
                self.buf = block;
            } else {
                self.buf.extend_from_slice(&block);
            }
            // As after a read of the pipe: a BOM is the stream's front's, which may be cut across blocks.
            self.skip_front_bom();
            return;
        }
        // #7446: past the length it had, the file is read on to the end it has now.
        let n = if self.remaining == 0 && !self.stop_at_len {
            n
        } else {
            usize::try_from(self.remaining).map_or(n, |left| n.min(left))
        };
        if n == 0 {
            self.eof = true;
        } else if self.buf.try_reserve(n).is_err() {
            self.read_error = Some(io::Error::from(io::ErrorKind::OutOfMemory));
            self.eof = true;
        } else {
            let had = self.remaining;
            match (&*self.file).take(n as u64).read_to_end(&mut self.buf) {
                Ok(read) => {
                    self.remaining = had.saturating_sub(read as u64);
                    self.file_read += read as u64;
                    #[cfg(test)]
                    {
                        self.bytes_read += read as u64;
                    }
                    if read < n || (self.stop_at_len && self.remaining == 0) {
                        self.eof = true;
                    } else if self.remaining == 0 && had > 0 {
                        self.read_on_past_len();
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

    /// The read after the one that reached the length the file had when it was opened (#7446): up to a
    /// block more, and the end of the file when it is short. A file nothing was appended to ends where it
    /// always did, at the cost of the one read that returns nothing (which jq's reader makes too), and
    /// one a writer appended to meanwhile is read on, a window at a time, to the end it has when the
    /// reads get there, as jq reads it.
    fn read_on_past_len(&mut self) {
        match (&*self.file)
            .take(self.block as u64)
            .read_to_end(&mut self.buf)
        {
            Ok(read) => {
                self.file_read += read as u64;
                #[cfg(test)]
                {
                    self.bytes_read += read as u64;
                }
                if read < self.block {
                    self.eof = true;
                }
            }
            // patchcov: coverage tolerate reason="untestable: a read that fails after the file opened"
            Err(error) => {
                self.read_error = Some(error);
                self.eof = true;
            } // patchcov: coverage end
        }
    }

    /// jq's parser skips a BOM at the start of its input (#5291), as the whole-file route does; with
    /// fewer than three bytes read it cannot be told from the start of another character.
    fn skip_front_bom(&mut self) {
        if !self.started && (self.buf.len() >= 3 || self.eof) {
            self.started = true;
            let bom = crate::cli::front_matter::leading_bom_len(&self.buf);
            if bom > 0 {
                self.buf.drain(..bom);
                self.front_lead = bom;
                self.offset += bom;
                // jq's first read holds the mark, so the reads of the first line start before the
                // text (#7569).
                self.seam.lead = bom;
                self.no_break_from = self.no_break_from.saturating_sub(bom);
                self.front_end = self.front_end.saturating_sub(bom);
            }
        }
    }

    /// Moves `validated` to the last line break read, or to the end of the file once it is read,
    /// substituting any invalid UTF-8 on the way as the whole-file route does. With `near`, to the last
    /// line break before it when there is one (a window made of a pipe's probe, #7266).
    fn validate_lines(&mut self, near: Option<usize>) {
        self.gate_cut = false;
        let end = if self.seq_gate.is_some() {
            loop {
                let gate = self
                    .seq_gate
                    .as_mut()
                    .expect("checked: the stream has a gate");
                match gate.advance(&self.buf, self.eof) {
                    // #7277: a window of a pipe's probe ends at the last record the gate passed
                    // within a block, as the plain pipe's ends at a line break (#7266); a record
                    // longer than that is the gate's end.
                    GateStep::Clean(end) => {
                        break near
                            .and_then(|near| self.last_record_start(near.min(end)))
                            .unwrap_or(end);
                    }
                    GateStep::Unclean(at) => break self.hand_on_seq_tail(at),
                    // #7322: the records go on being plain after a stretch that is not, which is
                    // read as jq's reader reads it and the windows carry on.
                    GateStep::Stretch { from, to, next_end } => {
                        if !self.resume_after_stretch(from, to, Some(next_end)) {
                            break self.hand_on_seq_tail(from);
                        }
                    }
                    // #7341: a long stretch, or one that runs to the stream's end, read up to an RS at
                    // a line start; the gate goes on judging from it.
                    GateStep::Cut { from, to } => {
                        if !self.resume_after_stretch(from, to, None) {
                            break self.hand_on_seq_tail(from);
                        }
                    }
                }
                // #7341: the warnings wait for the window that ends the text they were met in, and a
                // stretch can warn once a byte: once enough are waiting, the window ends where the gate
                // carried on from (after the plain record that follows a stretch, or at a cut), and the
                // gate goes on from there with the next.
                if self.notes.len() >= MAX_QUEUED_NOTES {
                    self.gate_cut = true;
                    break self
                        .seq_gate
                        .as_ref()
                        .expect("checked: the stream has a gate")
                        .checked;
                }
            }
        } else if self.eof {
            self.buf.len()
        } else {
            let from = self.validated.max(self.no_break_from);
            let near = near
                .map(|near| near.min(self.buf.len()))
                .filter(|&near| near > from);
            let in_near = near.and_then(|near| last_line_break(&self.buf[from..near]));
            #[cfg(test)]
            {
                self.break_search_bytes += match (near, in_near) {
                    (Some(near), Some(_)) => near - from,
                    _ => self.buf.len() - from,
                };
            }
            match in_near.or_else(|| last_line_break(&self.buf[from..])) {
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
        let blanks_nul_tails = !self.blank_rs && self.seq_gate.is_none();
        // #7437: the region is searched for a NUL as it is validated, while it is in the cache
        // (`scan_utf8_and_nul`); a region with none is not read again for it.
        let scan = if blanks_nul_tails {
            succinctly::jq::scan_utf8_and_nul(&self.buf[from..end])
        } else {
            succinctly::jq::TextScan {
                utf8: succinctly::text::utf8::validate_utf8(&self.buf[from..end]).is_ok(),
                nul: false,
            }
        };
        self.nul_seen |= scan.nul;
        if scan.utf8 {
            self.validated = end;
        } else {
            // The first region after a carry that was not validated again: the substitution moves the
            // carry's seam over the whole front, so it is made over that, as for any other carry.
            if self.front_validated > 0 && self.validated == self.front_validated {
                self.validated = 0;
            }
            self.substitute_region(end);
        }
        if self.blank_rs {
            blank_rs_bytes(&mut self.buf[from..self.validated]);
        } else if blanks_nul_tails && scan.nul {
            // #7437: a number token's NUL tail, as the whole-file route blanks it
            // (`blank_nul_number_tails_in`): the region is whole lines, so its reads are jq's, and
            // its first line is the file's first when no line has been dropped, behind the BOM.
            // The substitution adds no NUL (it writes U+FFFD), so a region with none has none after it.
            let lead = if from == 0 && self.lines_before == 0 {
                self.front_lead
            } else {
                0
            };
            succinctly::jq::blank_nul_number_tails(
                &mut self.buf[from..self.validated],
                Some(crate::cli::jq_seq_reader::JQ_FGETS_CHUNK),
                lead,
                &[],
            );
        }
    }

    /// Substitutes the invalid UTF-8 of the region from `validated` to `end`, as the whole-file route
    /// does, and moves `validated` past it.
    fn substitute_region(&mut self, end: usize) {
        let region = &self.buf[self.validated..end];
        let raw_len = region.len();
        // #7341: a `--seq` gate's positions and the warnings of its stretches are offsets into the text,
        // which the substitution moves (only a stretch's values can hold invalid UTF-8 there).
        let (text, edits) = if self.track_raw || self.seq_gate.is_some() {
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
        if let Some(gate) = self.seq_gate.as_mut() {
            let (from, delta) = (self.validated, text.len() as isize - raw_len as isize);
            let moved = |at: usize| {
                if at < from {
                    at // patchcov: coverage tolerate-line reason="defensive: the gate's positions are past what was validated before, and a warning before the region is one a window has not handed out yet, which the region's text cannot move"
                } else if at >= end {
                    at.checked_add_signed(delta)
                        .expect("past the region, which keeps its bytes")
                } else {
                    from + succinctly::jq::utf8_document::substituted_offset(&edits, at - from)
                }
            };
            gate.moved(moved);
            for (at, _) in &mut self.notes {
                *at = moved(*at);
            }
        }
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
        // The probe's bytes after the region move with its end (#7266). A region that ends past
        // them (a line longer than the probe's block, found by the search past `reach`) leaves none
        // of them to validate: 0 says so, as it does once they are dropped.
        if self.front_end >= end {
            self.front_end = self.front_end - end + new_end;
        } else if self.front_end > self.validated {
            self.front_end = 0;
        }
        self.buf.splice(self.validated..end, text);
        self.validated = new_end;
        self.substituted = true;
    }
}

/// How many windows may be waiting for the certifying thread (or in its hands) at once: the filter
/// runs over window N while N and N - 1 are certified, so a thread that falls behind holds the filter
/// back at the next window and the buffers it holds stay at two (#7106, #7260).
const CERTIFIER_WINDOWS_IN_FLIGHT: usize = 2;

/// A window of more than this is certified on the filter's own thread, over the buffer itself: it holds a
/// document bigger than a few windows, which a copy for the thread added the size of to the peak memory
/// of a run whose point is to hold less (a 175 MB one-line array read 395 MB with the copy and 222 MB
/// without, #7106). The thread reads a window's own buffer since #7260, so this is no longer a copy's
/// limit, and is kept so such a document is certified as it was. Certifying every window here instead
/// would cost a pipe of 10 MiB 4 to 10% of its run (measured, #7106, when the probe was one window of
/// 4 MiB or more).
#[inline]
fn certifier_copy_limit() -> usize {
    8 * window_bytes()
}

/// A buffer the certifying thread hands back is kept for the next window only up to this size, about two
/// windows, so one that grew with a document is not held for the run.
#[inline]
fn certifier_keep_bytes() -> usize {
    2 * window_bytes()
}

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
    /// The last window, not certified yet (#7294): it is never dropped, so its text is there when a
    /// reader of the fault asks, and a run that asks for none never certifies it.
    last: Option<Job<'static>>,
}

impl SeenFault {
    fn lock(&self) -> std::sync::MutexGuard<'_, SeenState> {
        self.0.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The stream's first fault, once every window handed to the certifier has been certified, and the
    /// last window too when it was left for this ([`Self::defer_last`]).
    pub(super) fn get(&self) -> Option<String> {
        let mut state = self
            .0
            .changed
            .wait_while(self.lock(), |state| state.in_flight > 0)
            .unwrap_or_else(PoisonError::into_inner);
        assert!(
            !state.panicked,
            "the thread that certifies the windows panicked"
        ); // patchcov: coverage tolerate-line reason="defensive: the certifier's own code does not panic; kept so a bug there is not read as a stream without a fault"
        if let Some(last) = state.last.take() {
            // Certified under the lock: every window before it is, and none follows it.
            if state.fault.is_none() {
                let mut resume = state.resume;
                let fault = last.certify(&mut resume);
                Self::store(&mut state, fault, Some(resume));
            }
        }
        state.fault.clone()
    }

    /// Keeps the last window to be certified when the fault is asked for ([`Self::get`], #7294). A
    /// window is certified because its text is dropped before the run may need a fault in it, and the
    /// last one is not: certifying it as it is handed out cost a run of one big document piped on
    /// standard input a strict pass over all of it, or a copy of it for the thread, which the same
    /// file read by name never pays.
    fn defer_last(&self, last: Job<'static>) {
        self.lock().last = Some(last);
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
/// on a thread of their own, over each window's own buffer, which the window handed out shares (#7260;
/// a copy of each until then), while the filter runs over the window (#7106): the
/// thread starts with the first window, in a run that reads a pipe of 4 MiB or more, and a reader of
/// the fault waits for it ([`SeenFault::get`]). The last window is never dropped, so it is certified
/// only when the fault is asked for, over its own buffer (#7294): a pipe that holds one big document is
/// that one window, and a run over it that meets no fault never certifies it.
struct Certifier {
    seen: SeenFault,
    /// The thread, started with the first window.
    worker: Option<Worker>,
    /// No thread could be started: every window is certified on the filter's own thread, as the one
    /// over [`certifier_copy_limit`] is.
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

/// The text of a window to certify ([`Job::text`]).
enum JobText<'a> {
    /// The buffer itself, certified on the filter's own thread.
    Borrowed(&'a [u8]),
    /// A window's buffer, shared with the reader that hands it out: `len` bytes of it, the last
    /// window's (#7294) and, for the certifying thread, every other one's (#7260).
    Held(Arc<Vec<u8>>, usize),
}

impl Default for JobText<'_> {
    fn default() -> Self {
        Self::Borrowed(&[])
    }
}

impl std::ops::Deref for JobText<'_> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match self {
            Self::Borrowed(text) => text,
            Self::Held(text, len) => &text[..*len],
        }
    }
}

/// One window to certify: its text and what the certifier needs of the reader's state to word a fault
/// from it.
struct Job<'a> {
    /// `buf[..validated]` of the window: the buffer the window shares, for the thread (#7260), or the
    /// buffer itself, for the filter's own.
    text: JobText<'a>,
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
        // #7260: a window's buffer the reader no longer holds is handed back to read a window into.
        let text = match std::mem::take(&mut job.text) {
            JobText::Held(text, _) => Arc::try_unwrap(text).ok(),
            JobText::Borrowed(_) => None, // patchcov: coverage tolerate-line reason="unreachable: the thread is handed held windows only"
        };
        if let Some(mut text) = text.filter(|text| text.capacity() <= certifier_keep_bytes()) {
            text.clear();
            // The reader may have gone, or hold a spare already: a buffer nobody wants is dropped.
            let _ = recycled.try_send(text);
        }
        match outcome {
            Ok(fault) => seen.finish(fault, Some(resume), false),
            Err(_) => seen.finish(None, None, true), // patchcov: coverage tolerate-line reason="defensive: certify does not panic; kept so a bug in it is reported by SeenFault::get"
        }
    }
}

/// The thread that certifies the windows, and the channels to and from it.
fn start_worker(seen: &SeenFault) -> io::Result<Worker> {
    let (jobs, inbox) = sync_channel(CERTIFIER_WINDOWS_IN_FLIGHT);
    // #7260: one spare at most, so the buffers the reader has finished with are not kept resident.
    let (outbox, recycled) = sync_channel(1);
    let seen = seen.clone();
    std::thread::Builder::new()
        .name("certify-windows".into())
        .spawn(move || certify_windows(&inbox, &outbox, &seen))?;
    Ok(Worker { jobs, recycled })
}

/// What a `--seq` pipe hands on at its first record that is not RFC 7464's own framing (#7164): the
/// windows before it were run, and the rest of the stream is the whole-stream route's.
pub(super) struct SeqTail {
    /// The stream from the RS that opens that record to its end, as read, behind the front that resumes
    /// jq's reader after `lines` line breaks ([`crate::cli::jq_seq_reader::resumed_stream`]): the tail is read
    /// into it, never copied (#7262).
    pub stream: Vec<u8>,
    /// The failure of a read of the pipe, which ended the stream there.
    pub error: Option<io::Error>,
    /// The byte the records before it left at index 1 of jq's token buffer
    /// ([`crate::cli::jq_seq_reader::token_byte_after`]).
    pub token_byte: Option<u8>,
    /// Where in `stream` the RS the tail opens with is, when jq's reader met it with a value pending:
    /// a stretch read before it warned there, which the windows have printed
    /// ([`crate::cli::jq_seq_reader::seed_faulted_rs`], #7260).
    pub faulted_rs: Option<usize>,
    /// #7260: the line breaks before the tail that `stream`'s front does not hold
    /// ([`crate::cli::jq_seq_reader::resumed_front_counted`]), which the walks of it are seeded with.
    pub line_base: usize,
    /// #7260: a regular file that does not open with an RS, read to its end and found to hold no RS jq
    /// sees, is not handed on as its bytes (`stream` is empty): this is the line and column jq's
    /// reader stands at at its end, where it reports `Unfinished abandoned text at EOF`, which is all
    /// the route that reads a stream whole would make of it.
    pub no_rs_end: Option<(usize, usize)>,
    /// Where `stream` was first allocated, and the room it had before the tail was read, so a test can
    /// tell the tail was read into it in place (a pointer alone does not: a large buffer can be grown in
    /// place by `mremap` on Linux).
    #[cfg(test)]
    pub reserved_at: usize,
    #[cfg(test)]
    pub reserved_room: usize,
}

/// Where a `--seq` pipe leaves its [`SeqTail`] for the run, which takes it once the windows are out.
pub(super) type SeqTailSlot = Rc<std::cell::RefCell<Option<SeqTail>>>;

/// Where a `--seq` list of files opened by path stopped (#7221): the first file that is not plain
/// `RS value LF` records ending in an LF, judged on the handle the run opened when it reached it. The
/// windows before it were run, and that file and the ones after it are the whole-file route's, read
/// as jq's reader reads them after the files before.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct SeqFilesTail {
    /// The file's position among the files named.
    pub index: usize,
    /// The line breaks of the files before it, every one in a record already run: 0 when none held
    /// a byte, and the stream starts at this file.
    pub lines: usize,
    /// The byte the records before it left at index 1 of jq's token buffer
    /// ([`crate::cli::jq_seq_reader::token_byte_after`]).
    pub token_byte: Option<u8>,
    /// #7341: the file was read a window at a time by a gate of its own ([`SeqGate`]) up to a stretch
    /// it could not carry on after, and this is the rest of it, from that stretch's RS, behind the front
    /// that resumes jq's reader after the file's lines before it: the file's bytes as the route that
    /// reads it whole is to read them. `None` when the stream stops before the file.
    pub front: Option<SeqFront>,
}

/// The rest of a file of a `--seq` list from where its gate stopped (#7341, [`SeqFilesTail::front`]).
#[derive(Debug)]
pub(super) struct SeqFront {
    pub stream: Vec<u8>,
    /// The failure of a read of the file, which ended it there.
    pub error: Option<io::Error>,
    /// Where in `stream` the RS it opens with is, when jq's reader met it with a value pending
    /// ([`SeqTail::faulted_rs`], #7404).
    pub faulted_rs: Option<usize>,
}

impl PartialEq for SeqFront {
    fn eq(&self, other: &Self) -> bool {
        self.stream == other.stream
            && self.error.is_some() == other.error.is_some()
            && self.faulted_rs == other.faulted_rs
    }
}

impl Eq for SeqFront {}

/// Where a `--seq` list of files opened by path leaves its [`SeqFilesTail`] for the run (#7221).
pub(super) type SeqFilesTailSlot = Rc<std::cell::RefCell<Option<SeqFilesTail>>>;

/// The judgement of each file of a `--seq` list opened by path, as the run reaches it (#7221), and
/// what the files judged so far left of jq's token buffer.
struct SeqFilesCheck {
    slot: SeqFilesTailSlot,
    token_byte: Option<u8>,
    /// For files the run holds (#7279): the position of the first that the prepass found is not plain
    /// records ending in an LF. The files before it are vouched for and are not judged again; it and the
    /// ones after it are judged as the run reaches them, as a list opened by path is.
    stop_at: Option<usize>,
    /// #7341: the file being read by a gate of its own, with the line breaks of the stream before it,
    /// and where its gate leaves what it hands on.
    gated: Option<(usize, SeqTailSlot)>,
}

/// The certification of a `--seq` pipe's records (#7164), which decides what of the stream is read a window
/// at a time. A pipe cannot be read twice, and its windows' output cannot be taken back, so unlike the
/// regular file's prepass (#7082) the verdict comes as the windows are read: each record is checked when
/// the RS after it (or the stream's end) is read, with the check [`crate::cli::jq_seq_reader::PlainRecords`]
/// makes of a whole stream, and a window gives the records up to the last checked one. A record that is
/// not `RS value LF` is read as jq's reader reads it when plain records follow it, and the windows carry
/// on after it ([`SeqGate::judge`], #7322). At the first such record they cannot carry on after, or one
/// bigger than [`MAX_PENDING_RECORD`], the windows end before its RS and the rest of the stream is read
/// and handed on ([`SeqTail`]), where jq's reader stands at a line start with nothing pending.
struct SeqGate {
    /// `buf[..checked]` is certified: records that are each `RS value LF`, and the stretches between
    /// them the windows carried on after (#7322). The record in progress opens at `checked` (an RS), or
    /// the stream is all certified when it is the end of `buf`.
    checked: usize,
    /// Where the search for the RS that closes the record at `cand` goes on from.
    search_from: usize,
    /// The stream's first byte was an RS.
    opened: bool,
    /// What the records certified so far left at index 1 of jq's token buffer.
    token_byte: Option<u8>,
    /// The RS that opens the record being judged: `checked`, or past it while the gate looks for
    /// where the records are plain again after one at `checked` that is not (#7322).
    cand: usize,
    /// `buf[..lines_at]` holds `lines` line breaks: the line a stretch starts on is counted on from
    /// the stretch before it, not from the front of the buffer each time.
    lines_at: usize,
    lines: usize,
    /// #7341: the last RS past `checked` that starts a line, in the stretch being judged: where it can
    /// be cut ([`GateStep::Cut`]).
    stretch_cut: Option<usize>,
    /// #7341: the line breaks of the stream before the source the gate reads: none for a stream of one
    /// source, and those of the files before it for a file of a `--seq` list. jq's reader counts the
    /// lines it warns at over the whole stream.
    lines_base: usize,
    slot: SeqTailSlot,
}

/// What [`SeqGate::advance`] found: how far the stream is certified, where its first record that is not
/// plain opens when the rest of the stream is the whole-stream route's, or a stretch that is not plain
/// with plain records after it.
enum GateStep {
    Clean(usize),
    Unclean(usize),
    /// #7322: the record at `from` is not plain, and `to` is the first RS after it that starts a line
    /// and opens a record that is, which ends at `next_end`.
    Stretch {
        from: usize,
        to: usize,
        next_end: usize,
    },
    /// #7341: the stretch at `from` is read on to `to`, an RS that starts a line and opens a record
    /// that is not plain (or is not read yet), and the gate goes on judging from there. The stretch is
    /// long enough to be read in pieces, or it runs to the end of the stream, which is handed on from
    /// `to` rather than from `from`.
    Cut {
        from: usize,
        to: usize,
    },
}

/// The longest stretch that is not plain records a `--seq` gate carries on after (#7322), and how far
/// past the stretch's start it reads while the record that may end it is still open. Past it the rest
/// of the stream is handed on, as all of it was before: the look-ahead is held beside the tail while
/// that is read, so it is a few windows.
#[inline]
fn max_seq_stretch() -> usize {
    4 * window_bytes()
}

/// How many warnings of the stretches a `--seq` gate carried on after may wait for their window
/// (#7341). Each is a line of text held until the values before it have run, and a window of
/// stretches that warn once a byte held about a million: a pipe of 18 MB peaked at 633 MiB where jq
/// holds 2.6. Past this the window ends at the next plain record: about 8 MiB of warnings at most,
/// and a window of at least a sixteenth of a block of a stream that warns once a byte, so the text
/// is dropped from the front of the buffer a few times a block, not once a few stretches.
const MAX_QUEUED_NOTES: usize = 1 << 16;

/// How long a stretch a `--seq` gate reads before it cuts it at the last RS that starts a line
/// (#7341, [`GateStep::Cut`]): jq's reader stands after any such RS as it stands after one between
/// plain records, so the stretch before it is read, its warnings handed out with their window, and
/// what follows is judged as from a record's start. Each piece a stretch is read in is this or a line,
/// so the look-ahead of [`max_seq_stretch`] is the longest a stretch's line may be, not the stretch.
const SEQ_STRETCH_PIECE: usize = 64 << 10;

impl SeqGate {
    #[cfg_attr(not(unix), allow(dead_code))]
    fn new(slot: SeqTailSlot) -> Self {
        Self {
            checked: 0,
            search_from: 0,
            opened: false,
            token_byte: None,
            cand: 0,
            lines_at: 0,
            lines: 0,
            stretch_cut: None,
            lines_base: 0,
            slot,
        }
    }

    /// A gate for a file of a `--seq` list (#7341): `lines_base` line breaks of the stream come before
    /// it and left `token_byte` in jq's token buffer, and the stream stands at a line start there.
    fn for_file_after(slot: SeqTailSlot, lines_base: usize, token_byte: Option<u8>) -> Self {
        Self {
            token_byte,
            lines_base,
            ..Self::new(slot)
        }
    }

    /// Checks the records of `buf` that are complete (the stream's last one too at `eof`).
    fn advance(&mut self, buf: &[u8], eof: bool) -> GateStep {
        if !self.opened {
            match buf.first() {
                None if !eof => return GateStep::Clean(0), // patchcov: coverage tolerate-line reason="unreachable: the window reads before it asks the gate, so the buffer is empty only at the end of the stream"
                Some(&RS) => {
                    self.opened = true;
                    self.search_from = 1;
                }
                // Bytes before the first RS (a BOM among them) are no record jq's reader reads.
                _ => return GateStep::Unclean(0),
            }
        }
        let from = self.checked;
        let step = self.judge(buf, eof);
        if let Some(byte) = crate::cli::jq_seq_reader::token_byte_after(&buf[from..self.checked]) {
            self.token_byte = Some(byte);
        }
        step
    }

    /// [`Self::advance`] once the stream is open: the records from `cand` on, each judged when the
    /// RS after it (or the stream's end) is read.
    ///
    /// #7322: a record at `checked` that is not plain does not end the windows there. The gate reads
    /// on, up to [`max_seq_stretch`], for the first RS that starts a line and opens a record that is
    /// plain: jq's reader stands after that RS as it stands after any RS between plain records
    /// ([`crate::cli::jq_seq_reader::walk_stretch`]), so the windows can carry on from it. A stretch that
    /// runs to the stream's end, or past the look-ahead, is handed on with the rest of the stream.
    fn judge(&mut self, buf: &[u8], eof: bool) -> GateStep {
        use crate::cli::jq_seq_reader::{PlainGuards, PlainRecords};
        while self.cand < buf.len() {
            let in_stretch = self.cand > self.checked;
            let next = buf[self.search_from..].iter().position(|&b| b == RS);
            let end = match next {
                Some(at) => self.search_from + at,
                None if eof => buf.len(),
                None => {
                    self.search_from = buf.len();
                    if in_stretch {
                        // The record being read is part of the stretch, which may be long enough to cut.
                        let here = (buf[self.cand - 1] == b'\n').then_some(self.cand);
                        return self
                            .cut_if_long(buf.len(), here)
                            .unwrap_or(GateStep::Clean(self.checked));
                    }
                    return if buf.len() - self.checked <= MAX_PENDING_RECORD {
                        GateStep::Clean(self.checked)
                    } else {
                        GateStep::Unclean(self.checked)
                    };
                }
            };
            let plain = PlainRecords::record_ok(&buf[self.cand + 1..end], PlainGuards::ALL);
            if !in_stretch {
                if plain {
                    self.checked = end;
                }
            } else {
                let starts_line = buf[self.cand - 1] == b'\n';
                if plain && starts_line {
                    return GateStep::Stretch {
                        from: self.checked,
                        to: self.cand,
                        next_end: end,
                    };
                }
                if let Some(step) = self.cut_if_long(self.cand, starts_line.then_some(self.cand)) {
                    return step;
                }
            }
            self.cand = end;
            self.search_from = end + 1;
        }
        if self.cand > self.checked {
            // The stretch runs to the end of the stream: it is read to its last RS that starts a line,
            // and the stream is handed on from there.
            match self.stretch_cut.take() {
                Some(to) => GateStep::Cut {
                    from: self.checked,
                    to,
                },
                None => GateStep::Unclean(self.checked),
            }
        } else {
            GateStep::Clean(self.checked)
        }
    }

    /// #7341: the stretch from `checked`, read to `reach`, where `here` is the RS of the record at
    /// `cand` when it starts a line. Once the stretch is longer than a piece ([`SEQ_STRETCH_PIECE`]) it
    /// is cut before that record, at the last RS that starts a line, so no piece is longer than one;
    /// or at `here` when there was none before it (the piece is then one line, and longer). With no
    /// RS that starts a line in it at all, it is handed on past the look-ahead ([`max_seq_stretch`]).
    /// `None` while it is shorter.
    fn cut_if_long(&mut self, reach: usize, here: Option<usize>) -> Option<GateStep> {
        let long = reach - self.checked;
        if long > SEQ_STRETCH_PIECE {
            if let Some(to) = self.stretch_cut.take().or(here) {
                return Some(GateStep::Cut {
                    from: self.checked,
                    to,
                });
            }
        }
        if here.is_some() {
            self.stretch_cut = here;
        }
        (long > max_seq_stretch() && self.stretch_cut.is_none())
            .then_some(GateStep::Unclean(self.checked))
    }

    /// The stretch [`GateStep::Stretch`] named was read as jq's reader reads it, and left `token_byte`
    /// in its token buffer: the stream is certified to `next_end`, the end of the plain record after it,
    /// whose first byte is `buf[to]`.
    fn resume(&mut self, buf: &[u8], to: usize, next_end: usize, token_byte: Option<u8>) {
        self.token_byte =
            crate::cli::jq_seq_reader::token_byte_after(&buf[to..next_end]).or(token_byte);
        self.checked = next_end;
        self.cand = next_end;
        self.search_from = next_end + 1;
        self.stretch_cut = None;
    }

    /// The stretch [`GateStep::Cut`] named was read to `to` as jq's reader reads it, and left
    /// `token_byte` in its token buffer: the stream is certified to `to`, an RS at a line start, and
    /// the record it opens is judged next (#7341).
    fn resume_at_cut(&mut self, to: usize, token_byte: Option<u8>) {
        self.token_byte = token_byte;
        self.checked = to;
        self.cand = to;
        self.search_from = to + 1;
        self.stretch_cut = None;
    }

    /// The text's invalid UTF-8 was substituted, which moved each position by `moved` (#7341).
    fn moved(&mut self, moved: impl Fn(usize) -> usize) {
        self.checked = moved(self.checked);
        self.cand = moved(self.cand);
        self.search_from = moved(self.search_from);
        self.lines_at = moved(self.lines_at);
        if let Some(cut) = self.stretch_cut.as_mut() {
            *cut = moved(*cut);
        }
    }

    /// How many line breaks `buf[..at]` holds, for an `at` that only moves on between drops.
    fn lines_to(&mut self, buf: &[u8], at: usize) -> usize {
        self.lines += count_newlines(&buf[self.lines_at..at]);
        self.lines_at = at;
        self.lines
    }

    /// `n` bytes were dropped from the front of the buffer, all before `checked`, with `newlines`
    /// line breaks in them.
    fn dropped(&mut self, n: usize, newlines: usize) {
        self.checked -= n;
        self.cand -= n;
        self.search_from -= n;
        if let Some(cut) = self.stretch_cut.as_mut() {
            *cut -= n;
        }
        if self.lines_at >= n {
            self.lines_at -= n;
            self.lines -= newlines;
        } else {
            self.lines_at = 0;
            self.lines = 0;
        }
    }
}

impl WindowedFile {
    /// A `--seq` pipe on standard input (#7164), from the front a first read of it took: its records
    /// that are `RS value LF` are handed out, each RS read as a space, with the values of the stretches
    /// between them that jq's reader can be followed through (#7322), and the stream from the first
    /// record it cannot be goes to `slot` ([`SeqGate`]). Nothing of the text is changed before it is certified, so the
    /// tail is the bytes as read: no BOM is skipped (one is no record's start, and the gate hands it on).
    #[cfg_attr(not(unix), allow(dead_code))]
    pub(super) fn from_seq_pipe(
        file: Rc<File>,
        front: Vec<u8>,
        block: usize,
        slot: SeqTailSlot,
    ) -> Self {
        let mut this = Self::in_stream(file, 0, block, Place::alone());
        this.file_read = front.len() as u64;
        // #7277: the probe is handed out a block of records at a time, not read onto.
        this.front_end = front.len();
        this.buf = front;
        this.started = true;
        this.blank_rs = true;
        this.seq_gate = Some(SeqGate::new(slot));
        this.lone_seq = true;
        this
    }

    /// The last record start (an RS) after `validated` and no later than `limit`, which the gate has
    /// passed (#7277): every RS it passed opens a record, and an RS past `validated` is not blanked yet.
    fn last_record_start(&self, limit: usize) -> Option<usize> {
        let from = self.validated + 1;
        let to = (limit + 1).min(self.buf.len());
        (from < to)
            .then(|| self.buf[from..to].iter().rposition(|&b| b == RS))
            .flatten()
            .map(|at| from + at)
    }

    /// Reads the stretch `buf[from..to]`, which is not plain records, as jq's `--seq` reader reads it
    /// (#7322), so the windows carry on at `to`, the RS of the plain record that ends at `next_end`;
    /// `false` when the stretch is left to the whole-stream route with the rest of the stream.
    ///
    /// The reader's model says which values the stretch yields and what it warns of
    /// ([`crate::cli::jq_seq_reader::walk_stretch`]). Every byte of the stretch that is in no value is
    /// read as a space, as an RS is (a line break stays, so every line and offset is where it was),
    /// and the warnings are handed out with the window ([`Window::notes`]). What is left is those
    /// values with white space between them, which the filter's parser reads as jq's parser reads
    /// them, as it does the documents of any other input: jq's leniencies (`nan`, `012`, nesting past
    /// RFC 8259's 128 levels up to the 256 jq's reader allows) included (#7341). A value with invalid
    /// UTF-8 in a string is substituted with the rest of the text the gate certified, once a window
    /// (`substitute_region`, which moves the gate's positions and the warnings with it). The stretch is
    /// declined only when the splitter does not find exactly those values in the text: a check, not a
    /// case that is known to occur.
    fn resume_after_stretch(&mut self, from: usize, to: usize, next_end: Option<usize>) -> bool {
        let gate = self
            .seq_gate
            .as_mut()
            .expect("only the gate of a --seq stream finds a stretch");
        let lines = gate.lines_base + self.lines_before + gate.lines_to(&self.buf, from);
        let Some(walk) =
            crate::cli::jq_seq_reader::walk_stretch(&self.buf[from..=to], lines, gate.token_byte)
        else {
            return false;
        };
        let mut text = self.buf[from..to].to_vec();
        let mut at = 0;
        for &(start, end) in &walk.values {
            blank_between_values(&mut text[at..start]);
            at = end;
        }
        blank_between_values(&mut text[at..]);
        let (values, fault) = split_json_values(&text);
        if fault.is_some() || values != walk.values {
            return false; // patchcov: coverage tolerate-line reason="defensive: the splitter reads the values jq's reader yields with white space between them as jq's parser does; kept so a disagreement leaves the stretch to the reader's model"
        }
        self.buf[from..to].copy_from_slice(&text);
        self.notes.extend(
            walk.warnings
                .into_iter()
                .map(|(at, line)| (from + at, line)),
        );
        let gate = self
            .seq_gate
            .as_mut()
            .expect("checked above: the stream has a gate");
        match next_end {
            Some(next_end) => gate.resume(&self.buf, to, next_end, walk.token_byte),
            None => gate.resume_at_cut(to, walk.token_byte),
        }
        true
    }

    /// Ends the text at `at`, where the gate found the first record that is not plain, reads the rest of
    /// the pipe and hands it on; the end of the text.
    ///
    /// #7262: the tail is read once, into the stream the route that reads it whole is given, which has room
    /// for all of it when the source is a regular file (what is left of it from where the reads stand): a
    /// fault near a big file's start hands on most of the file, which a copy would hold twice.
    ///
    /// The front counts the lines of this source alone, not the gate's `lines_base`: a file of a `--seq`
    /// list goes behind a source of its own for the files before it ([`SeqFilesTail::front`],
    /// `hand_on_seq_files_tail`), as a runtime error names the line in its own file.
    fn hand_on_seq_tail(&mut self, at: usize) -> usize {
        use std::io::Seek;
        if let Some(end) = self.no_rs_end(at) {
            self.buf.truncate(at);
            self.buf.shrink_to_fit();
            self.eof = true;
            let gate = self
                .seq_gate
                .as_ref()
                .expect("only a --seq source's gate hands a tail on");
            *gate.slot.borrow_mut() = Some(SeqTail {
                #[cfg(test)]
                reserved_at: 0,
                #[cfg(test)]
                reserved_room: 0,
                stream: Vec::new(),
                error: None,
                token_byte: gate.token_byte,
                faulted_rs: None,
                line_base: 0,
                no_rs_end: Some(end),
            });
            return at;
        }
        let lines = self.lines_before + count_newlines(&self.buf[..at]);
        let left = if self.eof {
            0
        } else {
            let mut file = &*self.file;
            match (file.metadata(), file.stream_position()) {
                (Ok(meta), Ok(at)) if meta.is_file() => {
                    usize::try_from(meta.len().saturating_sub(at)).unwrap_or(0)
                }
                _ => 0,
            }
        };
        let pre_read: usize = self.pre_read.iter().map(Vec::len).sum();
        // #7260: the front of the run's only source holds one line break and counts the rest; a file
        // of a `--seq` list goes behind a source of its own for the files before it, whose walk counts
        // every line, so its front holds them all.
        let room = self.buf.len() - at + pre_read + left;
        let (mut bytes, line_base) = if self.lone_seq {
            crate::cli::jq_seq_reader::resumed_front_counted(lines, room)
        } else {
            (crate::cli::jq_seq_reader::resumed_front(lines, room), 0)
        };
        #[cfg(test)]
        let (reserved_at, reserved_room) = (bytes.as_ptr() as usize, bytes.capacity());
        // #7260: a warning met at the RS the tail opens with is a stretch's, which ended there.
        let faulted_rs = (self.buf.get(at) == Some(&RS)
            && self.notes.iter().any(|&(met, _)| met == at))
        .then_some(bytes.len());
        bytes.extend_from_slice(&self.buf[at..]);
        // #7260: the blocks of a pipe's probe not read yet come before the rest of the pipe.
        for block in self.pre_read.drain(..) {
            bytes.extend_from_slice(&block);
        }
        // The windows' buffer keeps only the text still to be handed out, so the run does not hold its
        // room beside the tail.
        self.buf.truncate(at);
        self.buf.shrink_to_fit();
        let error = if self.eof {
            self.read_error.take()
        } else {
            (&*self.file).read_to_end(&mut bytes).err()
        };
        // #7576: a tail there is no room for is jq's `cannot allocate memory`, not a stream that ends
        // where the room did.
        if let Some(error) = &error {
            super::abort_if_out_of_memory(error);
        }
        self.eof = true;
        let gate = self
            .seq_gate
            .as_ref()
            .expect("only a --seq source's gate hands a tail on");
        *gate.slot.borrow_mut() = Some(SeqTail {
            #[cfg(test)]
            reserved_at,
            #[cfg(test)]
            reserved_room,
            stream: bytes,
            error,
            token_byte: gate.token_byte,
            faulted_rs,
            line_base,
            no_rs_end: None,
        });
        at
    }

    /// #7260: where jq's reader stands at the end of a regular file whose stream does not open with an
    /// RS (the gate hands it on at `at`, its first byte), read to its end a piece at a time, when it
    /// holds no RS jq sees ([`crate::cli::jq_seq_reader::NoRsScan`]): jq prints only `Unfinished
    /// abandoned text at EOF` there, so the route that reads a stream whole need not be handed all of
    /// it (78 MB held for a 70 MB file). `None` for any other stream, which is then read from where
    /// it was, as before: a pipe, which cannot be read again; a stream an RS or a malformed BOM turns
    /// up in, or whose read fails, which the file is put back for (it is seeked to where the scan
    /// began).
    fn no_rs_end(&self, at: usize) -> Option<(usize, usize)> {
        use std::io::{Seek, SeekFrom};
        /// How much of the file a read of the scan takes.
        const PIECE: usize = 64 << 10;
        let gate = self.seq_gate.as_ref()?;
        if !self.lone_seq || at != 0 || gate.opened || self.read_error.is_some() {
            return None;
        }
        let mut file = &*self.file;
        if !file.metadata().ok()?.is_file() {
            return None;
        }
        let start = file.stream_position().ok()?;
        let mut scan = crate::cli::jq_seq_reader::NoRsScan::default();
        let mut piece = Vec::with_capacity(PIECE.max(self.buf.len()) + JQ_READ_CARRY);
        piece.extend_from_slice(&self.buf);
        // What the gate read first is judged before anything more is read: a stream that has an RS
        // near its front (one led by a line break) is handed on with nothing read past it to put back.
        let read = scan.feed(&piece, self.eof)?;
        if self.eof {
            return Some(scan.position());
        }
        piece.drain(..read);
        let mut eof = false;
        let found = loop {
            if !eof {
                let room = piece.capacity() - piece.len();
                match file.take(room as u64).read_to_end(&mut piece) {
                    // A short read is the file's end, as a read of it to its end finds it.
                    Ok(read) => eof = read < room,
                    // patchcov: coverage tolerate-line reason="untestable: a read that fails after the file opened"
                    Err(_) => break false,
                }
            }
            let Some(read) = scan.feed(&piece, eof) else {
                break false;
            };
            if eof {
                break true;
            }
            piece.drain(..read);
        };
        if found {
            return Some(scan.position());
        }
        // patchcov: coverage tolerate-line reason="defensive: a regular file that was read can be seeked"
        file.seek(SeekFrom::Start(start)).ok()?;
        None
    }

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
        this.front_end = front.len();
        this.probe_reach = front.len() as u64 + 2 * this.block as u64;
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

    /// A pipe whose probe read `blocks` after its front (#7260): they are read, in order, before the
    /// pipe is ([`Self::pre_read`]).
    fn then_reads(mut self, blocks: std::collections::VecDeque<Vec<u8>>) -> Self {
        let read: u64 = blocks.iter().map(|block| block.len() as u64).sum();
        self.file_read += read;
        self.probe_reach += read;
        self.pre_read = blocks;
        self
    }

    /// Certifies `values`, the documents of the window about to be handed out (and the bytes after them,
    /// when `fault_at_end`: the window is the last and ends in a fault), and keeps the stream's first
    /// fault. The window is handed to the thread that certifies them, over its own buffer, which the window
    /// handed out shares ([`Self::held`], #7260), and waits only while two others are in flight; one of
    /// more than [`certifier_copy_limit`], or any when no thread could
    /// be started, is certified here, over the buffer itself. The last window is certified only if a
    /// fault is asked for, over the buffer, which it then shares ([`SeenFault::defer_last`], #7294).
    fn certify(&mut self, values: &[Span], fault_at_end: bool) {
        let Some(certifier) = self.certifier.as_mut() else {
            return;
        };
        if certifier.seen.found() {
            return;
        }
        if self.done {
            let text = Arc::new(std::mem::take(&mut self.buf));
            self.held = Some(Arc::clone(&text));
            certifier.seen.defer_last(Job {
                text: JobText::Held(text, self.validated),
                values: values.to_vec(),
                fault_at_end,
                offset: self.offset,
                origin: self.origin,
                eof: self.eof,
                // Nothing reads the map once the last window is out.
                raw_map: Cow::Owned(std::mem::take(&mut self.raw_map)),
                #[cfg(test)]
                exact_reads: Arc::clone(&certifier.exact_reads),
            });
            return;
        }
        let text = &self.buf[..self.validated];
        if certifier.worker.is_none()
            && !certifier.threadless
            && text.len() <= certifier_copy_limit()
        {
            match start_worker(&certifier.seen) {
                Ok(worker) => certifier.worker = Some(worker),
                Err(_) => certifier.threadless = true,
            }
        }
        let Some(worker) = certifier
            .worker
            .as_ref()
            .filter(|_| text.len() <= certifier_copy_limit())
        else {
            let job = Job {
                text: JobText::Borrowed(text),
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
        // A place first, so the windows the thread holds are never more than two.
        certifier.seen.begin();
        // #7260: the thread reads the window's own buffer, which it shares with the window handed out
        // ([`Self::held`]), where it read a copy: that copied every byte of the stream once more, and
        // held up to three copies of a window beside the buffer.
        let validated = self.validated;
        let text = Arc::new(std::mem::take(&mut self.buf));
        self.held = Some(Arc::clone(&text));
        let job = Job {
            text: JobText::Held(text, validated),
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

    /// The room of the buffer the windows are read into, the last window's once it is shared (#7294).
    #[cfg(test)]
    fn text_capacity(&self) -> usize {
        self.held
            .as_ref()
            .map_or_else(|| self.buf.capacity(), |text| text.capacity())
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
        // `JobText`'s `Deref` is a match and, for a held buffer, a bounds check: take it once, not per document (#7313).
        let text: &[u8] = &self.text;
        for &(start, end) in &self.values {
            let certified = succinctly::json::validate::Validator::new(&text[start..end])
                .with_max_depth(CERTIFIED_DEPTH)
                .validate()
                .is_ok();
            if certified {
                at = Self::boundary_after(text, start, end);
                continue;
            }
            // One read by jq's parser, from the last boundary to the end of the window, covers this
            // document and every one after it, so a window costs one such read at most whatever it
            // holds (a stream of `NaN`s is read at the parser's speed, not once a document).
            match self.read_from(at, text.len()) {
                Err(message) => fault = Some(message),
                Ok(_) => {
                    if let Some(&(start, end)) = self.values.last() {
                        at = Self::boundary_after(text, start, end);
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
    fn boundary_after(text: &[u8], start: usize, end: usize) -> usize {
        if opens_container_or_string(text[start]) {
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
#[cfg_attr(not(unix), allow(dead_code))]
pub(super) struct NamedFile {
    path: PathBuf,
    reach: std::cell::Cell<Reach>,
}

/// What the run found at a [`NamedFile`]'s path when it reached it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(unix), allow(dead_code))]
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
#[allow(dead_code)]
struct FileIdentity;

#[cfg_attr(not(unix), allow(dead_code))]
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

/// The file `path` names now, opened, with its metadata. A directory opens on Unix and fails at its first
/// read, so it fails here, as `EISDIR`.
fn open_not_a_directory(path: &std::path::Path) -> io::Result<(File, std::fs::Metadata)> {
    let file = File::open(path)?;
    let meta = file.metadata()?;
    if meta.is_dir() {
        return Err(io::Error::from_raw_os_error(super::EISDIR));
    }
    Ok((file, meta))
}

/// A file of a long list that `-R` (#7024) or `input`/`inputs` (#7197) reads, opened by its path when the
/// run first reads it and closed once a read reaches its end, so a list of any length holds one
/// descriptor, where a short one holds each file open from the start. Closing at the end, not when the
/// reader is dropped, keeps that true for a consumer that holds the readers it has read to the end, as
/// the eager reader a hand-off gives the rest of the stream to does ([`super::hand_off_later_sources`]).
/// The list is judged by `stat` before the run starts ([`super::judge_regular_paths`]), which opens
/// nothing; the run then reads what the path names when it gets there, with no second judgement and no
/// length fixed beforehand, as jq, which opens each file as it reaches it and reads it to its end, does
/// (a path replaced by a named pipe waits for a writer there, as it does in jq). A path that will not
/// open then (or names a directory) is read as an empty file and its failure handed to
/// [`super::note_raw_open_failure`], so the tail of the file before it continues in the file after it,
/// as in jq and in the whole-file route, which drops such a file from the stream.
pub(super) struct OpenWhenRead {
    path: PathBuf,
    /// The file's position among the files named, which its lines are tagged with.
    at: usize,
    /// Whether it is the last of them: jq's reader does not report a directory there (#6472).
    last: bool,
    file: Option<File>,
    /// The file did not open, or was read to its end and closed: it reads as no bytes.
    ended: bool,
    /// Where the stream starts in the file once it is open: past a byte-order mark jq skips.
    start: u64,
}

impl OpenWhenRead {
    pub(super) fn new(path: PathBuf, at: usize, last: bool) -> Self {
        Self {
            path,
            at,
            last,
            file: None,
            ended: false,
            start: 0,
        }
    }

    /// The same, reading from byte `start` of the file (#7197).
    pub(super) fn starting_at(mut self, start: u64) -> Self {
        self.start = start;
        self
    }
}

impl Read for OpenWhenRead {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.ended {
            return Ok(0);
        }
        let file = match self.file.as_mut() {
            Some(file) => file,
            None => match open_not_a_directory(&self.path).and_then(|(mut file, _)| {
                if self.start > 0 {
                    std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(self.start))?;
                }
                Ok(file)
            }) {
                Ok(file) => self.file.insert(file),
                Err(error) => {
                    self.ended = true;
                    super::note_raw_open_failure(&self.path, self.at, &error, self.last);
                    return Ok(0);
                }
            },
        };
        let read = file.read(buf)?;
        if read == 0 && !buf.is_empty() {
            // The end of the file: its descriptor is given back now, and the file is not opened again.
            self.ended = true;
            self.file = None;
        }
        Ok(read)
    }
}

#[cfg_attr(not(unix), allow(dead_code))]
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
        match open_not_a_directory(&self.path) {
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
    /// Open from the start of the run, with the length it had then, which the reads go on past to the
    /// end it has when they get there (#7446).
    Held(Rc<File>, u64),
    /// Opened when the run reaches it ([`NamedFile`]).
    #[cfg_attr(not(unix), allow(dead_code))]
    Named(Rc<NamedFile>),
}

/// Several regular files read as the one stream jq reads them as (#7024): each a window at a time, and
/// a token still open where a file ends continued at the front of the next ([`SeamStitcher`], the code
/// the whole-file route stitches with), so the documents, the file each is read in and the lines jq
/// names for it are the whole-file route's.
pub(super) struct WindowedFiles {
    /// The files not yet reached, each with the length it had when it was opened.
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
    /// Each file opened by path is judged when the run reaches it ([`Self::certifying_seq_files`],
    /// #7221): one that is not plain `--seq` records is read by a gate of its own when it opens with an
    /// RS (#7341), and the stream ends before it otherwise.
    seq_files: Option<SeqFilesCheck>,
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
            seq_files: None,
        }
    }

    /// A pipe on standard input ([`WindowedFile::from_pipe`], #7024), the stream's one source, from the
    /// probe that read its front in blocks (#7260): the first block is the front, and the rest are read
    /// before the pipe is ([`WindowedFile::pre_read`]).
    #[cfg_attr(not(unix), allow(dead_code))]
    pub(super) fn pipe(
        file: Rc<File>,
        mut blocks: std::collections::VecDeque<Vec<u8>>,
        block: usize,
        seen: SeenFault,
    ) -> Self {
        let front = blocks.pop_front().unwrap_or_default();
        let mut this = Self::new(Vec::new(), block);
        this.count = 1;
        this.next_index = 1;
        this.current =
            Some(WindowedFile::from_pipe(file, front, None, block, seen).then_reads(blocks));
        this
    }

    /// A `--seq` pipe on standard input ([`WindowedFile::from_seq_pipe`], #7164): the stream's one source,
    /// its clean records read with each RS as a space and the rest handed on to `slot`.
    #[cfg_attr(not(unix), allow(dead_code))]
    pub(super) fn seq_pipe(
        file: Rc<File>,
        front: Vec<u8>,
        block: usize,
        slot: SeqTailSlot,
    ) -> Self {
        Self::seq_pipe_blocks(file, std::collections::VecDeque::from([front]), block, slot)
    }

    /// [`Self::seq_pipe`] from a probe read in blocks (#7260): the first is the front, and the rest are
    /// read before the pipe is ([`WindowedFile::pre_read`]).
    #[cfg_attr(not(unix), allow(dead_code))]
    pub(super) fn seq_pipe_blocks(
        file: Rc<File>,
        mut blocks: std::collections::VecDeque<Vec<u8>>,
        block: usize,
        slot: SeqTailSlot,
    ) -> Self {
        let front = blocks.pop_front().unwrap_or_default();
        let mut this = Self::new(Vec::new(), block);
        this.count = 1;
        this.next_index = 1;
        this.current =
            Some(WindowedFile::from_seq_pipe(file, front, block, slot).then_reads(blocks));
        this
    }

    /// The same reading each RS byte as a space (#7082): the plain JSON stream a clean `--seq` stream is.
    pub(super) fn blanking_rs(mut self) -> Self {
        self.blank_rs = true;
        self
    }

    /// The same for a `--seq` list of files opened by path (#7221), which no prepass can vouch for (it
    /// would read what each path names before the run, and the run what it names when it gets there):
    /// each file is judged on the handle the run opens when it reaches it, as the prepass judges a
    /// file the run holds ([`Reread::is_plain_seq_records`]). One that is not plain records ending in an
    /// LF is read by a gate of its own when it opens with an RS (#7341); at the first that does not, or
    /// where a gate cannot carry on, the stream ends, and where it stopped goes to `slot`.
    #[cfg_attr(not(unix), allow(dead_code))]
    pub(super) fn certifying_seq_files(mut self, slot: SeqFilesTailSlot) -> Self {
        self.seq_files = Some(SeqFilesCheck {
            slot,
            token_byte: None,
            stop_at: None,
            gated: None,
        });
        self
    }

    /// The same for a `--seq` list of files the run holds (#7279), where the prepass found the first that is
    /// not plain records ending in an LF at position `stop_at` ([`Reread::plain_seq_files`]): the files
    /// before it are read a window at a time, and it and the ones after it are judged as the run reaches
    /// them, as a list opened by path judges its files (#7341).
    #[cfg_attr(not(unix), allow(dead_code))]
    pub(super) fn stopping_seq_files_at(mut self, slot: SeqFilesTailSlot, stop_at: usize) -> Self {
        self.seq_files = Some(SeqFilesCheck {
            slot,
            token_byte: None,
            stop_at: Some(stop_at),
            gated: None,
        });
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
                    let (index, gate_token) = (
                        current.index,
                        current.seq_gate.as_ref().map(|g| g.token_byte),
                    );
                    self.current = None;
                    // #7341: a file a gate of its own read either handed the rest of itself on, which
                    // ends the stream here, or was read to its end, and the records after it are read
                    // after the byte it left in jq's token buffer.
                    if let Some(check) = &mut self.seq_files {
                        if let Some((lines, slot)) = check.gated.take() {
                            if let Some(tail) = slot.borrow_mut().take() {
                                *check.slot.borrow_mut() = Some(SeqFilesTail {
                                    index,
                                    lines,
                                    token_byte: tail.token_byte,
                                    front: Some(SeqFront {
                                        stream: tail.stream,
                                        error: tail.error,
                                        faulted_rs: tail.faulted_rs,
                                    }),
                                });
                                self.pending = Vec::new().into_iter();
                                return Ok(false);
                            }
                            check.token_byte = gate_token.flatten();
                        }
                    }
                    if let Some(error) = error {
                        return Err(error);
                    }
                }
                Some(_) => return Ok(true),
                None => {
                    let index = self.next_index;
                    let (file, len) = match self.pending.next() {
                        None => return Ok(false),
                        Some(StreamFile::Held(file, len)) => {
                            if let Some(check) = &self.seq_files {
                                // #7279: vouched for by the prepass up to `stop_at`, so only the byte
                                // its records leave in jq's token buffer is read (from its end); a read
                                // that fails stops the stream here instead, before the file. #7341: the
                                // files from `stop_at` on are judged as the run reaches them.
                                // The prepass already found the file at `stop_at` is not plain.
                                let verdict = match check.stop_at {
                                    Some(stop) if index < stop => seq_token_byte_in(&file, len)
                                        .map_or(SeqFileVerdict::Unread, SeqFileVerdict::Plain),
                                    Some(stop) if index == stop => SeqFileVerdict::Other,
                                    _ => seq_file_verdict(&file, len),
                                };
                                if !self.judge_seq_file(&file, verdict, index) {
                                    return Ok(false);
                                }
                            }
                            (file, len)
                        }
                        Some(StreamFile::Named(named)) => match named.open_for_run() {
                            Ok((file, len)) => {
                                let file = Rc::new(file);
                                if self.seq_files.is_some() {
                                    let verdict = seq_file_verdict(&file, len);
                                    if !self.judge_seq_file(&file, verdict, index) {
                                        return Ok(false);
                                    }
                                }
                                (file, len)
                            }
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
                    // #7341: a file a gate of its own reads stands at a line start with nothing open
                    // before it, and hands on no open tail: the gate certifies whole records.
                    let gate = self.seq_files.as_ref().and_then(|check| {
                        check.gated.as_ref().map(|(lines, slot)| {
                            SeqGate::for_file_after(Rc::clone(slot), *lines, check.token_byte)
                        })
                    });
                    let carry_validated =
                        std::mem::take(&mut self.stitcher.carry_validated) && !carry.is_empty();
                    let place = Place {
                        index,
                        carry,
                        seam,
                        hold_tail: gate.is_none() && index + 1 < self.count,
                        strip_bom: self.only_empty,
                        origin: self.end_origin,
                        substituted: self.substituted,
                        carry_validated,
                    };
                    let block = if gate.is_some() {
                        window_bytes()
                    } else {
                        self.block
                    };
                    let carry_len = place.carry.len();
                    let mut opened = WindowedFile::in_stream(file, len, block, place);
                    // #7750: the parser that read the carry goes on with this file.
                    opened.tail_reader = self.stitcher.take_reader().filter(|_| carry_len > 0);
                    opened.tail_read_to = carry_len;
                    opened.blank_rs = self.blank_rs;
                    // #7446: a `--seq` file read with no gate of its own was vouched for up to its
                    // length, so that is where it ends; a gate certifies whatever it reads.
                    opened.stop_at_len = self.blank_rs && gate.is_none();
                    opened.seq_gate = gate;
                    self.current = Some(opened);
                }
            }
        }
    }
}

impl WindowedFiles {
    /// What a `--seq` list does with the file at `index` the run has reached, by `verdict`: `true` to
    /// read it, after a plain one's records, or by a gate of its own (#7341) when it is not plain and
    /// opens with an RS (the files before it end at a line start, so jq's reader stands there as it
    /// stands between records); `false` when the stream ends before it, which goes to the slot.
    fn judge_seq_file(&mut self, file: &File, verdict: SeqFileVerdict, index: usize) -> bool {
        let lines = self.end_origin.0 - 1;
        let check = self
            .seq_files
            .as_mut()
            .expect("only a --seq list judges its files");
        match verdict {
            SeqFileVerdict::Plain(byte) => {
                check.token_byte = byte.or(check.token_byte);
                true
            }
            SeqFileVerdict::Other if opens_with_rs(file) => {
                check.gated = Some((lines, SeqTailSlot::default()));
                true
            }
            SeqFileVerdict::Other | SeqFileVerdict::Unread => {
                // The files before it end in an LF, so the stream stands at a line's start:
                // `end_origin` counts the lines.
                *check.slot.borrow_mut() = Some(SeqFilesTail {
                    index,
                    lines,
                    token_byte: check.token_byte,
                    front: None,
                });
                self.pending = Vec::new().into_iter();
                false
            }
        }
    }
}

/// Whether `file`'s first byte is an RS (#7341).
#[cfg(unix)]
fn opens_with_rs(file: &File) -> bool {
    use std::os::unix::fs::FileExt;
    let mut first = [0u8; 1];
    matches!(file.read_at(&mut first, 0), Ok(1) if first[0] == RS)
}

#[cfg(not(unix))]
fn opens_with_rs(_file: &File) -> bool {
    false
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
    /// was read (a file that grows meanwhile is read as it was); `None` for one read to its end as it
    /// stands when the reads get there (a file a writer may append to, #7217), or of unknown length,
    /// which ends at a short read.
    remaining: Option<u64>,
    /// How much a read adds.
    block: usize,
    /// The bytes read after the last line break of the last window: the front of the next.
    carried: Vec<u8>,
    /// Nothing more is read: the end of the source, a read that failed, or the length it had.
    eof: bool,
    /// The most the next window reads, once (#7264): a source started on a few KiB reads up to where
    /// a start on the whole front would have ended, so its windows then fall where they always did.
    first_ask: Option<usize>,
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
            first_ask: None,
        }
    }

    /// The first window reads no more than `bytes`.
    pub(super) fn with_first_window(mut self, bytes: usize) -> Self {
        self.first_ask = Some(bytes);
        self
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
        self.next_window_after(want, Vec::new(), on_error).ok()
    }

    /// Whether the next window is the one [`with_first_window`](Self::with_first_window) limits.
    #[cfg(test)]
    pub(super) fn first_window_pending(&self) -> bool {
        self.first_ask.is_some()
    }

    /// [`next_window`](Self::next_window) read onto `before`, which comes first in what it returns: the
    /// bytes of a text that the window is to be appended to, so that the window is read straight into
    /// the text's own buffer instead of one of its own and then copied (#7264: the copy faulted a
    /// second set of pages for the 240 KiB after a short start). Its last line break is still the
    /// window's, and a read that fails cuts only the window's line. `Err(before)` when there is no
    /// window: nothing was added. A `before` with no buffer at all ([`next_window`](Self::next_window)'s)
    /// takes the carried bytes' own buffer instead, so they are not copied (#7323).
    pub(super) fn next_window_after(
        &mut self,
        want: Option<usize>,
        before: Vec<u8>,
        on_error: impl FnOnce(&io::Error),
    ) -> Result<Vec<u8>, Vec<u8>> {
        let floor = before.len();
        let carried = std::mem::take(&mut self.carried);
        // #7323: with no buffer to read onto, the carried bytes are the window's buffer. Copying them
        // onto a new one held a pipe's 4 MiB probe twice (an 8.0 MiB heap peak for `-R` over a pipe).
        let mut bytes = if before.capacity() == 0 {
            carried
        } else {
            let mut bytes = before;
            bytes.extend_from_slice(&carried);
            bytes
        };
        let mut ask = want.unwrap_or(self.block).max(1);
        if let Some(first) = self.first_ask.take() {
            ask = ask.min(first.max(1));
        }
        // #7024: bytes carried in that make a window already (a pipe's probe of 4 MiB) are handed out as
        // one, up to their last line break. Reading a block onto them would grow the probe's buffer to
        // twice its size, and a window to the probe and a block. The bytes carried after a window are
        // past its last line break, so they hold none and are never cut here.
        if bytes.len() - floor >= ask {
            if let Some(at) = last_line_break(&bytes[floor..]) {
                self.carried = bytes.split_off(floor + at + 1);
                return Ok(bytes);
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
                return if bytes.len() > floor {
                    Ok(bytes)
                } else {
                    Err(bytes)
                };
            }
            let n = self.remaining.map_or(ask, |left| {
                ask.min(usize::try_from(left).unwrap_or(usize::MAX))
            });
            let before = bytes.len();
            // patchcov: coverage tolerate reason="untestable: an allocation that fails"
            if bytes.try_reserve(n).is_err() {
                self.fail(
                    &mut bytes,
                    floor,
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
                    self.fail(&mut bytes, floor, &mut on_error, &error);
                    continue;
                }
            }
            if !self.eof {
                if let Some(at) = last_line_break(&bytes[before..]) {
                    self.carried = bytes.split_off(before + at + 1);
                    return Ok(bytes);
                }
                // No line break in what was read: a long line, read on twice as far.
                ask = ask.saturating_mul(2);
            }
        }
    }

    /// The rest of the source, from the bytes carried on, up to the length it was given (to its end when
    /// there is none) as [`next_window`](Self::next_window) reads it: for a text that is not UTF-8, which is substituted
    /// whole, and for the eager reader a fault hands the rest of several files to (#7024).
    pub(super) fn take_rest(&mut self, on_error: impl FnOnce(&io::Error)) -> Vec<u8> {
        let mut bytes = std::mem::take(&mut self.carried);
        if !self.eof {
            let read = match self.remaining {
                Some(left) => self.reader.by_ref().take(left).read_to_end(&mut bytes),
                None => self.reader.read_to_end(&mut bytes),
            };
            if let Err(error) = read {
                // patchcov: coverage tolerate reason="untestable: a read that fails after the source opened"
                // #7576: the rest read whole, with no room for it, is jq's `cannot allocate memory`.
                super::abort_if_out_of_memory(&error);
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
        floor: usize,
        on_error: &mut Option<impl FnOnce(&io::Error)>,
        error: &io::Error,
    ) {
        self.eof = true;
        if let Some(report) = on_error.take() {
            report(error);
        }
        let kept = bytes[floor..]
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |at| at + 1);
        bytes.truncate(floor + kept);
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
    /// What was left of the file from `start` when it was opened: how much of it the `--seq` prepass
    /// vouches for and its run reads. Every other run reads on past it (#7446), and so does the second
    /// read that words its fault ([`Reread::read_fault`]).
    #[cfg_attr(not(unix), allow(dead_code))]
    len: u64,
}

/// The file a [`RereadSource`] reads: a handle the run holds, or a [`NamedFile`] the second read opens
/// when it reaches it (#7024), and declines if it is no longer the file the run read.
#[cfg_attr(not(unix), allow(dead_code))]
enum RereadFile {
    Held(Rc<File>),
    #[cfg_attr(not(unix), allow(dead_code))]
    Named(Rc<NamedFile>),
}

/// How much of a file the second read reads: a length of 0 is not known to be (`WindowedFile`), so the
/// file is read to its end.
#[cfg_attr(not(unix), allow(dead_code))]
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
/// it (NDJSON with one RS at its front) is not read into memory to be told it is not records. A `--seq`
/// pipe hands such a record on to that route too ([`SeqGate`], #7164).
const MAX_PENDING_RECORD: usize = 64 << 20;

/// How much of the end of a file of a `--seq` list the run reads at a time for the byte its records
/// leave in jq's token buffer ([`seq_token_byte_in`]).
#[cfg(unix)]
const TAIL_PROBE: u64 = 256 << 10;

/// How much of the file a chunk of the `--seq` prepass holds (#7082): small enough that the buffer comes
/// from the heap and is not a mapping that is freed, which glibc answers by raising the size it maps
/// from, so that the window buffers after it were carved from a heap that fragments (+6 MiB at 100 MiB).
/// It is also how much of the file's end the prepass reads first, to check the last record: a stream
/// that fails does so most often at its end, and the check then costs this, not a pass over the file.
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
    #[cfg_attr(not(unix), allow(dead_code))] // its callers read files at an offset, Unix only
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
            // #7446: the run reads each file on past the length it had, to the end it has when the reads
            // get there ([`WindowedFile::remaining`]), so the file is read again to the end it has now; a
            // `--seq` stream ends at the length its records were vouched for up to, as its run does.
            let len = if self.blank_rs.get() { len } else { u64::MAX };
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
                        chunk.drain(..crate::cli::front_matter::leading_bom_len(&chunk));
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
    /// framing, which [`crate::cli::jq_seq_reader::plain_record_sequence`] vouches for over a text held
    /// whole and this does a chunk at a time, by the same checker. So the run may read it as the plain
    /// JSON stream it is with each RS blanked. `false` is no verdict on the stream (a read that
    /// failed, or one the walk of jq's reader has to decide), and the whole-file route takes it.
    ///
    /// The memory is a chunk and the biggest record. The first and last bytes are asked of first, so a
    /// stream that is not framed at all is turned down by two reads, not by a pass over it. Several files
    /// are vouched for when each one is (#7218).
    ///
    /// #7218: several files are one stream, so they are plain records when each is, on its own, and
    /// each ends in an LF: the join of two such files is `... LF RS ...`, which is framed as it is
    /// inside one. A file that is not (no RS to start, a cut-short record, no final LF, or none at
    /// all) is left to the whole-file route, which walks the join as jq's reader does. A file the
    /// run opens by its path when it reaches it ([`NamedFile`], a long list) is not read here: the
    /// prepass would read what the path names now and the run what it names then. The run asks
    /// [`Self::plain_seq_files`], of which this is the verdict for every file.
    #[cfg(unix)]
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn is_plain_seq_records(&self) -> bool {
        !self.sources.is_empty() && self.plain_seq_files() == self.sources.len()
    }

    /// How many of the files, from the first, [`Self::is_plain_seq_records`] vouches for one by one
    /// (#7279): plain records ending in an LF. All of them is the plain stream; fewer is a stream whose
    /// files before the first that is not can still be read a window at a time. 0 for a list of files
    /// the run opens by path, or none.
    #[cfg(unix)]
    pub(super) fn plain_seq_files(&self) -> usize {
        let held =
            |source: &RereadSource, check: &dyn Fn(&File, u64, u64) -> bool| match &source.file {
                RereadFile::Held(file) => check(file, source.start, source.len),
                RereadFile::Named(_) => false,
            };
        // The ends first, as above: a file that is not framed at all stops the count with two reads.
        let ends = self
            .sources
            .iter()
            .take_while(|source| held(source, &plain_seq_ends_in))
            .count();
        self.sources[..ends]
            .iter()
            .take_while(|source| held(source, &plain_seq_records_in))
            .count()
    }

    #[cfg(not(unix))]
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn is_plain_seq_records(&self) -> bool {
        false
    }

    #[cfg(not(unix))]
    pub(super) fn plain_seq_files(&self) -> usize {
        0
    }

    /// Whether the run opens every file by its path when it reaches it ([`NamedFile`], a long list):
    /// [`Self::is_plain_seq_records`] cannot vouch for those, and a `--seq` run judges each as it is
    /// reached instead ([`WindowedFiles::certifying_seq_files`], #7221).
    pub(super) fn opens_files_by_path(&self) -> bool {
        !self.sources.is_empty()
            && self
                .sources
                .iter()
                .all(|source| matches!(source.file, RereadFile::Named(_)))
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

/// Whether the `len` bytes of `file` from `start` begin on an RS and end on an LF, the first of what
/// [`Reread::is_plain_seq_records`] asks of each file.
#[cfg(unix)]
fn plain_seq_ends_in(file: &File, start: u64, len: u64) -> bool {
    use std::os::unix::fs::FileExt;
    // A length of 0 is not known to be (`known_len`), and a file of none is no record.
    if len == 0 || len == u64::MAX {
        return false;
    }
    let byte_at = |at: u64| {
        let mut one = [0u8];
        match file.read_at(&mut one, start + at) {
            Ok(1) => Some(one[0]),
            _ => None,
        }
    };
    byte_at(0) == Some(RS) && byte_at(len - 1) == Some(b'\n')
}

/// The records of a file whose ends [`plain_seq_ends_in`] has vouched for.
#[cfg(unix)]
fn plain_seq_records_in(file: &File, start: u64, len: u64) -> bool {
    use crate::cli::jq_seq_reader::{PlainGuards, PlainRecords};
    use std::os::unix::fs::FileExt;
    let guards = PlainGuards::ALL;
    let mut chunk = vec![0u8; PREPASS_CHUNK];
    // The last record first, when an RS is near enough the end to hold it: the verdict does not
    // depend on the order, and a record that fails is most often the last (a write cut short). #7260:
    // read into the chunk the pass reads with, and only for a file longer than one, which the pass
    // reads whole at once: a probe of its own was a 256 KiB buffer mapped, faulted in and freed, and a
    // file of 256 KiB was read twice before its windows read it (4-6% of its run on the 7950X).
    if len > PREPASS_CHUNK as u64 {
        let tail = &mut chunk[..PREPASS_CHUNK];
        if file
            .read_exact_at(tail, start + len - PREPASS_CHUNK as u64)
            .is_err()
        {
            return false; // patchcov: coverage tolerate-line reason="untestable: a read that fails right after the last byte was read"
        }
        if let Some(rs) = tail.iter().rposition(|&b| b == RS) {
            if !PlainRecords::record_ok(&tail[rs + 1..], guards) {
                return false;
            }
        }
    }
    let mut check = PlainRecords::new(guards);
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

/// What the run finds of a file of a `--seq` list opened by path when it reaches it (#7221).
#[derive(Debug, PartialEq, Eq)]
enum SeqFileVerdict {
    /// Plain records ending in an LF, with the byte they leave at index 1 of jq's token buffer (`None`
    /// for records that write none there).
    Plain(Option<u8>),
    /// Not plain records, or a file a read of failed in: a gate of its own reads it when it opens with
    /// an RS (#7341), and the whole-file route does otherwise.
    Other,
    /// A file the prepass vouched for whose byte could not be read: the whole-file route's.
    Unread,
}

/// [`SeqFileVerdict`] of a file judged on the handle the run opened, as [`Reread::is_plain_seq_records`]
/// judges a file the run holds.
#[cfg(unix)]
fn seq_file_verdict(file: &File, len: u64) -> SeqFileVerdict {
    if !(plain_seq_ends_in(file, 0, len) && plain_seq_records_in(file, 0, len)) {
        return SeqFileVerdict::Other;
    }
    match seq_token_byte_in(file, len) {
        Ok(byte) => SeqFileVerdict::Plain(byte),
        // patchcov: coverage tolerate reason="untestable: a read that fails right after the judgement read the same bytes"
        Err(_) => SeqFileVerdict::Other,
        // patchcov: coverage end
    }
}

#[cfg(not(unix))]
fn seq_file_verdict(_file: &File, _len: u64) -> SeqFileVerdict {
    SeqFileVerdict::Other
}

/// The byte the plain records of `file`'s first `len` bytes leave at index 1 of jq's token buffer
/// ([`crate::cli::jq_seq_reader::token_byte_after`]), read from the end a stretch at a time: the last token
/// that writes there decides, so most files are answered by their last record. A stretch starts at an
/// RS, which no record of RFC 8259 holds, so it is whole records; one with no RS in it is part of a
/// record bigger than it, and the next read goes twice as far back.
#[cfg(unix)]
fn seq_token_byte_in(file: &File, len: u64) -> io::Result<Option<u8>> {
    let mut end = len;
    let mut reach = TAIL_PROBE;
    while end > 0 {
        let from = end.saturating_sub(reach);
        let bytes = read_range(file, from, end)?;
        if bytes.len() as u64 != end - from {
            // patchcov: coverage tolerate reason="untestable: a file that shrinks between the judgement and this read"
            return Err(io::ErrorKind::UnexpectedEof.into());
            // patchcov: coverage end
        }
        // The file starts on an RS ([`plain_seq_ends_in`]).
        let Some(rs) = bytes.iter().position(|&b| b == RS) else {
            reach = reach.saturating_mul(2);
            continue;
        };
        if let Some(byte) = crate::cli::jq_seq_reader::token_byte_after(&bytes[rs..]) {
            return Ok(Some(byte));
        }
        end = from + rs as u64;
        reach = TAIL_PROBE;
    }
    Ok(None)
}

/// Without positioned reads the byte is not read: the caller takes the file as
/// [`SeqFileVerdict::Unread`], the whole-file route's, as the non-Unix [`seq_file_verdict`] takes
/// every file as `Other`.
#[cfg(not(unix))]
fn seq_token_byte_in(_file: &File, _len: u64) -> io::Result<Option<u8>> {
    Err(io::ErrorKind::Unsupported.into())
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

    /// #7688: the switch that has a run read every input whole is any non-empty value but `0`, and
    /// the route log's name is the one the fuzz script sets.
    #[test]
    fn the_whole_file_switch_is_a_non_empty_value_but_zero_7688() {
        use std::ffi::OsStr;
        assert!(!forces_whole_file(None));
        for value in ["", "0"] {
            assert!(!forces_whole_file(Some(OsStr::new(value))), "{value:?}");
        }
        for value in ["1", "true", "yes", "00"] {
            assert!(forces_whole_file(Some(OsStr::new(value))), "{value:?}");
        }
        assert_eq!(WHOLE_FILE_ENV, "SUCCINCTLY_DEBUG_JQ_WHOLE_FILE");
        assert_eq!(ROUTE_LOG_ENV, "SUCCINCTLY_DEBUG_JQ_ROUTE_LOG");
    }

    /// #7312: a `--seq` file is read a megabyte at a time only when its first byte, from where the run
    /// reads it, is an RS: any other stream is handed on at that byte, so a page is all that is read.
    /// #7376: the debug-build window knob takes a decimal count of bytes from two `--seq` pages to
    /// the real window and refuses everything else, so a test whose small fixture would be read
    /// whole on the shipped window fails instead of passing without the route. The unit tests
    /// themselves run on the constants: `window_bytes` does not read the variable under `cfg(test)`.
    #[test]
    fn the_debug_window_knob_takes_a_window_in_range_and_refuses_the_rest_7376() {
        use std::ffi::OsStr;
        assert_eq!(debug_window_bytes(None), Ok(None));
        for (value, bytes) in [
            ("8192", 8192),
            ("65536", 65536),
            ("100000", 100_000),
            ("262144", WINDOW_BYTES),
        ] {
            assert_eq!(
                debug_window_bytes(Some(OsStr::new(value))),
                Ok(Some(bytes)),
                "{value}"
            );
        }
        for value in [
            "",
            "0",
            "1",
            "8191",
            "262145",
            "64k",
            "65536 ",
            " 65536",
            "+65536",
            "-1",
            "0x10000",
            "1e5",
            "99999999999999999999999999",
        ] {
            let refusal = debug_window_bytes(Some(OsStr::new(value))).unwrap_err();
            assert_eq!(
                refusal,
                format!(
                    "SUCCINCTLY_DEBUG_JQ_WINDOW_BYTES={value} is not a count of bytes from 8192 to 262144"
                ),
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            assert!(debug_window_bytes(Some(OsStr::from_bytes(b"6553\xff"))).is_err());
        }
        assert_eq!(DEBUG_WINDOW_ENV, "SUCCINCTLY_DEBUG_JQ_WINDOW_BYTES");
        // The unit tests' window, and every limit that is a multiple of it, are the shipped ones.
        assert_eq!(window_bytes(), 256 << 10);
        assert_eq!(min_windowed_file_bytes(), 1 << 20);
        assert_eq!(min_windowed_pipe_bytes(), 512 << 10);
        assert_eq!(certifier_copy_limit(), 2 << 20);
        assert_eq!(certifier_keep_bytes(), 512 << 10);
        assert_eq!(max_seq_stretch(), 1 << 20);
    }

    #[test]
    fn seq_block_is_a_page_unless_the_stream_opens_with_an_rs_7312() {
        let file_of = |bytes: &[u8]| {
            let mut file = tempfile::tempfile().expect("a temp file");
            file.write_all(bytes).expect("written");
            file
        };
        let opens = file_of(b"\x1e1\n");
        assert_eq!(seq_block_bytes(&opens, 0), WINDOW_BYTES);
        // Text before the first RS, a BOM, white space, a file with no RS at all, and an empty one.
        for (name, bytes) in [
            ("text", &b"x\x1e1\n"[..]),
            ("bom", &b"\xef\xbb\xbf\x1e1\n"[..]),
            ("space", &b" \x1e1\n"[..]),
            ("none", &b"[1,2]\n"[..]),
            ("empty", &b""[..]),
        ] {
            assert_eq!(
                seq_block_bytes(&file_of(bytes), 0),
                SEQ_UNOPENED_BLOCK_BYTES,
                "{name}"
            );
        }
        // Where the run reads from decides, not the file's first byte: a file on standard input that
        // is read from an offset (the descriptor's position).
        let shifted = file_of(b"x\x1e1\n");
        assert_eq!(seq_block_bytes(&shifted, 1), WINDOW_BYTES);
        assert_eq!(seq_block_bytes(&shifted, 0), SEQ_UNOPENED_BLOCK_BYTES);
        assert_eq!(seq_block_bytes(&shifted, 99), SEQ_UNOPENED_BLOCK_BYTES);
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

    /// #7412: a file under [`MAX_READ_WHOLE_WINDOWS`] windows whose window ends inside its one document
    /// is read on to its end at once, into a buffer of the file's length, as the route that read it
    /// whole read it while such a file did not take this route; a longer one is read on in doubling
    /// steps, as before. Both give what the file read whole gives.
    #[test]
    fn a_small_file_of_one_long_document_is_read_on_to_its_end_at_once_7412() {
        let block = 1024;
        let document = |len: usize| {
            let mut text = String::from("[");
            while text.len() + 4 < len {
                text.push_str("1,");
            }
            text.push_str("1]\n");
            text.into_bytes()
        };
        let read_on = |text: &[u8]| {
            let mut reader = open(text, block);
            let values = reader
                .next_window()
                .expect("reads")
                .expect("a window")
                .values;
            assert_eq!(values, vec![(0, text.len() - 1)]);
            assert!(reader.next_window().expect("reads").is_none());
            reader.buf.capacity()
        };
        let small = document(10 * block);
        assert!(small.len() < MAX_READ_WHOLE_WINDOWS * block);
        assert_eq!(read_on(&small), small.len());
        assert_same(&small, block);
        let long = document(20 * block);
        assert!(long.len() > MAX_READ_WHOLE_WINDOWS * block);
        assert!(read_on(&long) > long.len(), "grown by doubling");
        assert_same(&long, block);
    }

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

    /// What [`gated`] answers.
    type Gated = (Vec<u8>, Option<usize>, Option<u8>, Vec<(usize, String)>);

    /// What a `--seq` gate makes of `stream`, worked out over the whole of it (#7322): the text its
    /// windows hand out, where in the stream it is handed on from (`None` when it is not), the byte
    /// jq's token buffer holds there, and the warnings of the stretches the windows carried on after,
    /// each with where in the text it was met. A record that is not plain ends the windows only when
    /// no RS after it both starts a line and opens a plain record. The stretch up to that RS is its
    /// values with white space between them, whatever jq's parser took them as (#7341), and a value's
    /// invalid UTF-8 is substituted as the whole-stream route substitutes it.
    fn gated(stream: &[u8]) -> Gated {
        use crate::cli::jq_seq_reader::{
            token_byte_after, walk_stretch, PlainGuards, PlainRecords,
        };
        use succinctly::jq::utf8_document::substitute_invalid_utf8_jq_document_mapped;
        let mut text = Vec::with_capacity(stream.len());
        let (mut token, mut warnings) = (None, Vec::new());
        if stream.first() != Some(&RS) {
            return (Vec::new(), Some(0), token, warnings);
        }
        let mut starts: Vec<usize> = (0..stream.len()).filter(|&i| stream[i] == RS).collect();
        starts.push(stream.len());
        let plain = |i: usize| {
            PlainRecords::record_ok(&stream[starts[i] + 1..starts[i + 1]], PlainGuards::ALL)
        };
        // The stretch from `from` read up to the RS at `to`, as the gate's windows read it.
        let read_to = |from: usize, to: usize, token: Option<u8>| {
            let lines = count_newlines(&stream[..from]);
            let walk = walk_stretch(&stream[from..=to], lines, token)?;
            let mut stretch = stream[from..to].to_vec();
            let mut at = 0;
            for &(start, end) in &walk.values {
                blank_between_values(&mut stretch[at..start]);
                at = end;
            }
            blank_between_values(&mut stretch[at..]);
            let (stretch, edits) = substitute_invalid_utf8_jq_document_mapped(&stretch);
            let warned: Vec<(usize, String)> = walk
                .warnings
                .iter()
                .map(|(at, line)| {
                    (
                        succinctly::jq::utf8_document::substituted_offset(&edits, *at),
                        line.clone(),
                    )
                })
                .collect();
            Some((stretch.into_bytes(), warned, walk.token_byte))
        };
        let mut i = 0;
        while i + 1 < starts.len() {
            if plain(i) {
                token = token_byte_after(&stream[starts[i]..starts[i + 1]]).or(token);
                text.extend_from_slice(&stream[starts[i]..starts[i + 1]]);
                i += 1;
                continue;
            }
            let from = starts[i];
            let line_starts = |j: &usize| stream[starts[*j] - 1] == b'\n';
            let resumed = (i + 1..starts.len() - 1)
                .find(|j| line_starts(j) && plain(*j))
                .and_then(|j| Some((j, read_to(from, starts[j], token)?)));
            let Some((j, (stretch, warned, token_byte))) = resumed else {
                // #7341: a stretch that runs to the stream's end is read to its last RS that starts a
                // line, and the stream is handed on from there.
                let cut = (i + 1..starts.len() - 1)
                    .rev()
                    .find(line_starts)
                    .and_then(|j| Some((j, read_to(from, starts[j], token)?)));
                let at = match cut {
                    Some((j, (stretch, warned, token_byte))) => {
                        let base = text.len();
                        warnings.extend(warned.into_iter().map(|(at, line)| (base + at, line)));
                        text.extend_from_slice(&stretch);
                        token = token_byte;
                        starts[j]
                    }
                    None => from,
                };
                blank_rs_bytes(&mut text);
                return (text, Some(at), token, warnings);
            };
            let base = text.len();
            warnings.extend(warned.into_iter().map(|(at, line)| (base + at, line)));
            text.extend_from_slice(&stretch);
            token = token_byte;
            i = j;
        }
        blank_rs_bytes(&mut text);
        (text, None, token, warnings)
    }

    /// The streams the gate's tests read: `count` plain records of every kind, then `tail`.
    fn seq_stream(count: usize, tail: &[u8]) -> Vec<u8> {
        let record = |i: usize| -> String {
            match i % 5 {
                0 => format!("\x1e{{\"id\":{i},\"s\":\"bus\"}}\n"),
                1 => format!("\x1e[{i},\n  \"\\u0041\"]\n"),
                2 => format!("\x1e{i}\n"),
                3 => "\x1enull\n".to_string(),
                _ => "\x1e\"a\"\n\n".to_string(),
            }
        };
        let records: String = (0..count).map(record).collect();
        [records.as_bytes(), tail].concat()
    }

    /// The check [`a_seq_stream_carries_on_after_a_stretch_that_is_not_plain_7322`] makes of one
    /// stream, for each read size of `blocks` and three probe splits: the gate's windows against the
    /// model of them ([`gated`]), and with what they hand on, against jq's reader walking the stream
    /// whole. Whether any stretch was carried on after, with warnings.
    ///
    /// Without `model`, against jq's reader alone: the model reads a stretch in one walk, which keeps
    /// at most a walk's worth of warnings, where the gate reads one in pieces (#7341).
    fn check_gated_windows(stream: &[u8], blocks: &[usize], model: bool) -> bool {
        use crate::cli::jq_seq_reader::{
            resumed_stream_counted, seed_line_base, seed_token_byte, walk_stream,
        };
        let walked = |stream: &[u8]| -> (Vec<String>, Vec<(String, usize)>) {
            let sources = vec![(None, stream.to_vec())];
            let mut warnings = Vec::new();
            let walk = walk_stream(&sources, false, &mut |w| warnings.push(w.to_string()));
            let values = walk
                .values
                .iter()
                .zip(&walk.locations)
                .map(|(&(start, end), &(_, line))| {
                    (
                        String::from_utf8_lossy(&stream[start..end]).into_owned(),
                        line,
                    )
                })
                .collect();
            (warnings, values)
        };
        let (text, at, token, warnings) = gated(stream);
        let want = whole(&text);
        // A pipe's probe holds the front of the stream before the first window: none of
        // it, half of it and all of it.
        // #7260: and the probe read in blocks, each read before the pipe.
        for (block, probe, split) in blocks.iter().flat_map(|&block| {
            [0, stream.len() / 2, stream.len()]
                .into_iter()
                .flat_map(move |p| [(block, p, false), (block, p, true)])
        }) {
            let mut file = tempfile::tempfile().expect("a temp file");
            file.write_all(&stream[probe..]).expect("written");
            file.rewind().expect("rewound");
            let slot = SeqTailSlot::default();
            let mut read: std::collections::VecDeque<Vec<u8>> = if split {
                stream[..probe]
                    .chunks(block.max(1))
                    .map(<[u8]>::to_vec)
                    .collect()
            } else {
                std::collections::VecDeque::from([stream[..probe].to_vec()])
            };
            let front = read.pop_front().unwrap_or_default();
            let mut reader =
                WindowedFile::from_seq_pipe(Rc::new(file), front, block, Rc::clone(&slot))
                    .then_reads(read);
            let (mut spans, mut lines, mut notes) = (Vec::new(), Vec::new(), Vec::new());
            // What a run shows, in its order: each warning before the first value that
            // starts after it.
            let mut shown = Vec::new();
            while let Some(window) = reader.next_window().expect("reads") {
                let mut counter = LineCounter::new(window.raw);
                counter.set_lines_before(window.lines_before);
                let mut met = window.notes.iter().peekable();
                for &(start, end) in &window.values {
                    while let Some((_, line)) = met.next_if(|&&(at, _)| at < start) {
                        shown.push(line.clone());
                    }
                    let text = String::from_utf8_lossy(&window.raw[start..end]);
                    shown.push(format!("{text} at {}", counter.read_end_line(end)));
                    spans.push((window.offset + start, window.offset + end));
                    lines.push(counter.read_end_line(end));
                }
                shown.extend(met.map(|(_, line)| line.clone()));
                notes.extend(
                    window
                        .notes
                        .iter()
                        .map(|(at, line)| (window.offset + at, line.clone())),
                );
                assert_eq!(window.split_error, None);
            }
            let case = format!(
                "{:?} ({} bytes), block {block}, probe {probe}, in blocks {split}",
                String::from_utf8_lossy(&stream[..stream.len().min(300)]),
                stream.len()
            );
            if model {
                assert_eq!(
                    (spans, lines),
                    (want.spans.clone(), want.lines.clone()),
                    "{case}"
                );
            }
            // A warning met in text that was dropped with nothing to run is moved to
            // the front of what is left, so only the lines and their order are the
            // gate's; `shown` below holds their place among the values.
            let lines_of = |notes: &[(usize, String)]| -> Vec<String> {
                notes.iter().map(|(_, line)| line.clone()).collect()
            };
            if model {
                assert_eq!(lines_of(&notes), lines_of(&warnings), "{case}");
                assert!(
                    notes
                        .iter()
                        .zip(&warnings)
                        .all(|(got, want)| got.0 >= want.0),
                    "{case}"
                );
            }
            let handed = slot.borrow_mut().take();
            if model {
                assert_eq!(handed.is_some(), at.is_some(), "{case}");
            }
            // The run as jq's reader reads the stream whole.
            let mut jq_shown = Vec::new();
            {
                let sources = vec![(None, stream.to_vec())];
                let mut events: Vec<(usize, String)> = Vec::new();
                let walk = crate::cli::jq_seq_reader::walk_stream_collecting_at(
                    &sources,
                    false,
                    false,
                    crate::cli::jq_seq_reader::Tail::Opened,
                    &mut |warning, at| events.push((at, warning.to_string())),
                );
                let mut events = events.into_iter().peekable();
                for (&(start, end), &(_, line)) in walk.values.iter().zip(&walk.locations) {
                    while let Some((_, warning)) = events.next_if(|&(at, _)| at < start) {
                        jq_shown.push(warning);
                    }
                    let text = String::from_utf8_lossy(&stream[start..end]);
                    jq_shown.push(format!("{text} at {line}"));
                }
                jq_shown.extend(events.map(|(_, warning)| warning));
            }
            if let Some(handed) = handed {
                if model {
                    let at = at.expect("checked");
                    // #7260: the front holds one line break and the tail counts the rest.
                    assert_eq!(
                        (handed.stream.clone(), handed.line_base),
                        resumed_stream_counted(
                            count_newlines(&stream[..at]),
                            stream[at..].to_vec()
                        ),
                        "{case}"
                    );
                    assert_eq!(handed.token_byte, token, "{case}");
                }
                seed_token_byte(handed.token_byte);
                seed_line_base(handed.line_base);
                let (tail_warnings, tail_values) = walked(&handed.stream);
                seed_token_byte(None);
                seed_line_base(0);
                // The tail's warnings and values, in the order the whole walk has them.
                let tail_shown = &jq_shown[shown.len()..];
                let mut tail_events: Vec<&String> = tail_shown.iter().collect();
                tail_events.sort();
                let mut tail_made: Vec<String> = tail_warnings
                    .into_iter()
                    .chain(
                        tail_values
                            .iter()
                            .map(|(text, line)| format!("{text} at {line}")),
                    )
                    .collect();
                tail_made.sort();
                assert_eq!(tail_made.iter().collect::<Vec<_>>(), tail_events, "{case}");
                assert_eq!(shown, jq_shown[..shown.len()], "{case}");
            } else {
                assert_eq!(shown, jq_shown, "{case}");
            }
        }
        !warnings.is_empty()
    }

    /// #7322: the windows of a `--seq` stream carry on after a stretch that is not plain records, and
    /// with what is handed on they are the stream as jq's reader reads it whole: the same values, each
    /// on the line jq names for it, and the same warnings in the same order, wherever the stretches lie
    /// (the stream's first record among them) and whatever the reads' size.
    #[test]
    fn a_seq_stream_carries_on_after_a_stretch_that_is_not_plain_7322() {
        let stretches: [&[u8]; 14] = [
            b"\x1e{\"id\":-1,\"name\":\"cut\n",
            b"\x1en\n",
            b"\x1e1 2\n",
            b"\x1e} 5\n\x1e[1,]\n",
            b"\x1e12.5\x1e\"x\"\n",
            b"\x1e\x1e\x1e\n",
            b"\x1e\"a\" [1,\n2] {\"k\":null}\n",
            b"\x1enan\n",
            b"\x1e\"tab\there\"\n",
            b"\x1e\"\xff\"\n",
            b"\x1e1\"bu\"\n\x1en\n",
            b"\x1e[1,2]",
            b"\x1e\n\n\x1etrue false\n",
            b"\x1e{\"a\":[1,{\"b\":2}]}}\n",
        ];
        let mut resumed_streams = 0;
        for (n, first) in stretches.iter().enumerate() {
            for (m, second) in stretches.iter().enumerate() {
                // Each shape: the stretch first, in the middle, twice, and last.
                let shapes: [Vec<u8>; 4] = [
                    [first, seq_stream(7, b"").as_slice()].concat(),
                    seq_stream(3, &[first, seq_stream(4, second).as_slice()].concat()),
                    seq_stream(
                        2,
                        &[first, seq_stream(1, second).as_slice(), &seq_stream(3, b"")].concat(),
                    ),
                    seq_stream(5, first),
                ];
                for (shape, stream) in shapes.iter().enumerate() {
                    if shape != 2 && m != (n + 1) % stretches.len() {
                        continue;
                    }
                    resumed_streams +=
                        usize::from(check_gated_windows(stream, &[1, 7, 64, 4096], true));
                }
            }
        }
        assert!(resumed_streams > 40, "{resumed_streams}");
    }

    /// #7341: stretches that warn a lot end their window once [`MAX_QUEUED_NOTES`] warnings wait, at
    /// the plain record after the stretch, and the windows go on from there: whatever the reads' size,
    /// the stream read whole in one (where the cut window is not the last though the stream is read to
    /// its end) among them, the windows give every value on its line and every warning in order, and
    /// none waits for more than a window's worth.
    #[test]
    fn warnings_end_a_window_once_enough_wait_7341() {
        let stretch = [b"\x1e".as_slice(), &[b'}'; 1000], b"\n\x1e1\n"].concat();
        // Enough stretches for three windows' worth of warnings and some.
        let stretches = 3 * MAX_QUEUED_NOTES / 1000 + 5;
        let stream = [seq_stream(3, b"").as_slice(), &stretch.repeat(stretches)].concat();
        let (text, at, _, warnings) = gated(&stream);
        assert_eq!(at, None);
        assert_eq!(warnings.len(), stretches * 1000);
        let want = whole(&text);
        for block in [1 << 20, 4096, 997] {
            let mut file = tempfile::tempfile().expect("a temp file");
            file.write_all(&stream).expect("written");
            file.rewind().expect("rewound");
            let slot = SeqTailSlot::default();
            let mut reader =
                WindowedFile::from_seq_pipe(Rc::new(file), Vec::new(), block, Rc::clone(&slot));
            let (mut spans, mut lines, mut notes, mut windows) =
                (Vec::new(), Vec::new(), Vec::new(), 0);
            while let Some(window) = reader.next_window().expect("reads") {
                windows += 1;
                assert!(
                    window.notes.len() < MAX_QUEUED_NOTES + 1000,
                    "{} warnings in one window, block {block}",
                    window.notes.len()
                );
                let mut counter = LineCounter::new(window.raw);
                counter.set_lines_before(window.lines_before);
                for &(start, end) in &window.values {
                    spans.push((window.offset + start, window.offset + end));
                    lines.push(counter.read_end_line(end));
                }
                notes.extend(window.notes.iter().map(|(_, line)| line.clone()));
            }
            assert_eq!(
                (&spans, &lines),
                (&want.spans, &want.lines),
                "block {block}"
            );
            let warned: Vec<String> = warnings.iter().map(|(_, line)| line.clone()).collect();
            assert_eq!(notes, warned, "block {block}");
            assert!(windows >= 3, "block {block}: {windows} windows");
            assert!(slot.borrow().is_none(), "block {block}");
        }
    }

    /// #7341: a stretch several pieces long is read a piece at a time, cut at an RS that starts a line,
    /// and its windows with what is handed on are the stream as jq's reader reads it whole: in the
    /// middle of the stream, at its start, and running to its end, where the stream is handed on from
    /// the stretch's last RS that starts a line.
    #[test]
    fn a_long_stretch_is_read_in_pieces_as_jq_reads_it_whole_7341() {
        let bad: [&[u8]; 6] = [
            b"\x1e[1,]\n",
            b"\x1enan\n",
            b"\x1e} 5\n",
            b"\x1e012 [1,\n",
            b"\x1e\"\xff\"\n",
            b"\x1e1\x1e",
        ];
        let mut long = Vec::new();
        for i in 0.. {
            if long.len() > 3 * SEQ_STRETCH_PIECE + 100 {
                break;
            }
            long.extend_from_slice(bad[i % bad.len()]);
        }
        let streams = [
            [seq_stream(5, b"").as_slice(), &long, &seq_stream(5, b"")].concat(),
            [long.as_slice(), &seq_stream(3, b"")].concat(),
            [seq_stream(5, b"").as_slice(), &long].concat(),
        ];
        for stream in &streams {
            assert!(check_gated_windows(stream, &[4096, 65536, 1 << 20], true));
        }
    }

    /// #7341 review: the shapes the first version of the cuts got wrong, against jq's reader walking
    /// the stream whole. Lines that warn and hold no value, for many pieces: a cut window hands their
    /// warnings out with no value in it, so they never pile up. Lines of 1000 warnings for longer than a
    /// piece: each piece ends before the record that would take it past a piece, so none draws more
    /// warnings than a walk keeps. And a stream of short stretches each with invalid UTF-8 in a value,
    /// substituted once a window, with every position after each substitution moved.
    #[test]
    fn warning_lines_and_invalid_utf8_stretches_are_read_as_jq_reads_them_7341() {
        let records = seq_stream(5, b"");
        let warn_line = [b"\x1e".as_slice(), &[b'}'; 1000], b"\n"].concat();
        let lines = |n: usize| warn_line.repeat(n);
        // A line that draws more warnings than a piece's walk keeps, between bad records: the piece it
        // is cut into is handed on with the rest of the stream.
        let flood = [b"\x1e".as_slice(), &vec![b'}'; 70_000], b"\n"].concat();
        let streams = [
            [records.as_slice(), &lines(200), &records].concat(),
            [records.as_slice(), &lines(200)].concat(),
            b"\x1e\"\xff\"\n\x1e1\n".repeat(3000),
            [
                records.as_slice(),
                b"\x1e[1,]\n",
                &flood,
                b"\x1e[1,]\n",
                &records,
            ]
            .concat(),
        ];
        for stream in &streams {
            check_gated_windows(stream, &[4096, 65536, 1 << 20], false);
        }
        // The windows of the warning lines: none holds more than a walk's worth and a piece.
        let stream = [records.as_slice(), &lines(400), &records].concat();
        let mut file = tempfile::tempfile().expect("a temp file");
        file.write_all(&stream).expect("written");
        file.rewind().expect("rewound");
        let slot = SeqTailSlot::default();
        let mut reader = WindowedFile::from_seq_pipe(Rc::new(file), Vec::new(), 1 << 20, slot);
        let (mut windows, mut warned) = (0, 0);
        while let Some(window) = reader.next_window().expect("reads") {
            windows += 1;
            warned += window.notes.len();
            assert!(window.notes.len() <= MAX_QUEUED_NOTES + SEQ_STRETCH_PIECE);
        }
        assert_eq!(warned, 400_000);
        assert!(windows >= 4, "{windows}");
    }

    /// #7322, #7341: a stretch is read on to the first plain record at a line start after it; one
    /// longer than a piece ([`SEQ_STRETCH_PIECE`]) is cut at its last RS that starts a line, and only a
    /// line longer than the look-ahead ([`max_seq_stretch`]) with no plain record in sight yet is handed
    /// on with the rest of the stream.
    #[test]
    fn a_long_stretch_is_cut_at_a_line_start_and_a_long_line_handed_on_7341() {
        let gate = |stream: &[u8]| {
            let mut gate = SeqGate::new(SeqTailSlot::default());
            gate.advance(stream, false)
        };
        let records = seq_stream(3, b"");
        let near = [
            records.as_slice(),
            b"\x1e\"",
            &vec![b'x'; max_seq_stretch() - 100],
            b"\n\x1e1\n\x1e2\n",
        ]
        .concat();
        assert!(matches!(gate(&near), GateStep::Stretch { from, .. } if from == records.len()));
        // A line past the look-ahead that a plain record follows is read on, as it is already read.
        let far = [
            records.as_slice(),
            b"\x1e\"",
            &vec![b'x'; max_seq_stretch()],
            b"\n\x1e1\n\x1e2\n",
        ]
        .concat();
        assert!(matches!(gate(&far), GateStep::Stretch { from, .. } if from == records.len()));
        // While the record after a bad one is still being read, with no RS after it yet: a line past
        // the look-ahead is handed on, and the bad record before it is cut off first.
        let line = [
            records.as_slice(),
            b"\x1e[1,]\x1e\"",
            &vec![b'x'; max_seq_stretch()],
        ]
        .concat();
        assert!(matches!(gate(&line), GateStep::Unclean(at) if at == records.len()));
        let open = [
            records.as_slice(),
            b"\x1e[1,]\n\x1e\"",
            &vec![b'x'; max_seq_stretch()],
        ]
        .concat();
        let cut = records.len() + b"\x1e[1,]\n".len();
        assert!(
            matches!(gate(&open), GateStep::Cut { from, to } if from == records.len() && to == cut)
        );
        let short = [records.as_slice(), b"\x1e[1,]\n\x1e\"", &vec![b'x'; 1000]].concat();
        assert!(matches!(gate(&short), GateStep::Clean(at) if at == records.len()));
        // A plain record still being read, with no RS after it yet, is waited for.
        let pending = [records.as_slice(), b"\x1e{\"a\":", &vec![b'1'; 1000]].concat();
        assert!(matches!(gate(&pending), GateStep::Clean(at) if at == records.len()));
        // One still being read past the longest record held for it is handed on.
        let huge = [
            records.as_slice(),
            b"\x1e\"",
            &vec![b'x'; MAX_PENDING_RECORD + 1],
        ]
        .concat();
        assert!(matches!(gate(&huge), GateStep::Unclean(at) if at == records.len()));
        // Bad records, one a line, for longer than a piece: cut before the one that would take it past.
        let bad_record = b"\x1e[1,]\n";
        let bad: Vec<u8> = bad_record.repeat(SEQ_STRETCH_PIECE / bad_record.len() + 10);
        let many = [records.as_slice(), &bad, b"\x1e2\n"].concat();
        match gate(&many) {
            GateStep::Cut { from, to } => {
                assert_eq!(from, records.len());
                assert_eq!(many[to], RS);
                assert_eq!(many[to - 1], b'\n');
                let piece = to - from;
                // Before the record that would take it past a piece.
                assert!(piece <= SEQ_STRETCH_PIECE && piece > SEQ_STRETCH_PIECE - bad_record.len());
            }
            _ => panic!("a long stretch of lines is cut"), // patchcov: coverage tolerate-line reason="unreachable in a passing suite: reports a failed test invariant (#7341)"
        }
    }

    /// #7164: a `--seq` pipe hands out the documents and lines of its plain records, with each RS read as a
    /// space, and hands on the stream from its first record that is not plain and has no plain record at
    /// a line start after it (#7322): its bytes as read, the
    /// line breaks before it and the byte jq's token buffer holds there. Whatever the reads' size and
    /// wherever the probe's front ends, the windows are those of the plain records read whole.
    #[test]
    fn a_seq_pipe_runs_its_plain_records_and_hands_on_the_rest_7164() {
        let record = |i: usize| -> String {
            match i % 5 {
                0 => format!("\x1e{{\"id\":{i},\"s\":\"bus\"}}\n"),
                1 => format!("\x1e[{i},\n  \"\\u0041\"]\n"),
                2 => format!("\x1e{i}\n"),
                3 => "\x1enull\n".to_string(),
                _ => "\x1e\"a\"\n\n".to_string(),
            }
        };
        let tails: [&[u8]; 8] = [
            b"",
            b"\x1en\n\x1e1\n",
            b"\x1e{\"a\":1\n\x1e2\n",
            b"\x1e12.5",
            b"\x1enan\n",
            b"\x1e1 2\n\x1e3\n",
            b"\x1e\x1e\"x\"\n",
            b"\x1e[1,]\n",
        ];
        for count in [1usize, 2, 9, 40] {
            let records: String = (0..count).map(record).collect();
            for tail in tails {
                for front in [b"".as_slice(), b"\xef\xbb\xbf", b"x"] {
                    let stream = [front, records.as_bytes(), tail].concat();
                    // #7322: the windows carry on after a stretch with plain records after it.
                    let (blanked, at, token, _) = gated(&stream);
                    let want = whole(&blanked);
                    for block in [1usize, 3, 17, 4096] {
                        for probe in [0, 1, stream.len() / 2, stream.len()] {
                            let mut file = tempfile::tempfile().expect("a temp file");
                            file.write_all(&stream[probe..]).expect("written");
                            file.rewind().expect("rewound");
                            let slot = SeqTailSlot::default();
                            let mut reader = WindowedFile::from_seq_pipe(
                                Rc::new(file),
                                stream[..probe].to_vec(),
                                block,
                                Rc::clone(&slot),
                            );
                            let mut spans = Vec::new();
                            let mut lines = Vec::new();
                            while let Some(window) = reader.next_window().expect("reads") {
                                let mut counter = LineCounter::new(window.raw);
                                counter.set_lines_before(window.lines_before);
                                for &(start, end) in &window.values {
                                    spans.push((window.offset + start, window.offset + end));
                                    lines.push(counter.read_end_line(end));
                                }
                                assert_eq!(window.split_error, None);
                            }
                            let shown = String::from_utf8_lossy(&stream);
                            let case = format!("{shown:?}, block {block}, probe {probe}");
                            assert_eq!(
                                (spans, lines),
                                (want.spans.clone(), want.lines.clone()),
                                "{case}"
                            );
                            let handed = slot.borrow_mut().take();
                            match at {
                                None => assert!(handed.is_none(), "{case}"),
                                Some(at) => {
                                    let handed = handed.expect("the rest is handed on");
                                    assert_eq!(
                                        (handed.stream.clone(), handed.line_base),
                                        crate::cli::jq_seq_reader::resumed_stream_counted(
                                            count_newlines(&stream[..at]),
                                            stream[at..].to_vec()
                                        ),
                                        "{case}"
                                    );
                                    assert_eq!(handed.token_byte, token, "{case}");
                                    assert!(handed.error.is_none(), "{case}");
                                    // #7262: a regular file's tail is read into room reserved for all
                                    // of it, behind the front: one allocation, never grown or copied.
                                    assert_eq!(
                                        (handed.stream.as_ptr() as usize, handed.stream.capacity()),
                                        (handed.reserved_at, handed.stream.len()),
                                        "{case}"
                                    );
                                    assert_eq!(handed.reserved_room, handed.stream.len(), "{case}");
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// #7277: a `--seq` pipe's probe is handed out a block of records at a time from the buffer it was
    /// read into, where a block read onto it doubled the buffer and made one window of the probe and a
    /// block. A record longer than a block is a window of its own; the documents and lines are those of
    /// the records read whole.
    #[test]
    fn a_seq_pipes_probe_is_handed_out_a_block_at_a_time_7277() {
        let block = 1_000;
        let short = |i: usize| format!("\x1e{{\"id\":{i},\"s\":\"bus\"}}\n");
        let long = format!("\x1e[{}0]\n", "1,\n".repeat(700));
        let mut stream = String::new();
        for i in 0..400 {
            stream.push_str(&short(i));
            if i == 60 {
                stream.push_str(&long);
            }
        }
        let stream = stream.into_bytes();
        let mut blanked = stream.clone();
        blank_rs_bytes(&mut blanked);
        let want = whole(&blanked);
        let front = 4 * block;
        let mut file = tempfile::tempfile().expect("a temp file");
        file.write_all(&stream[front..]).expect("written");
        file.rewind().expect("rewound");
        let probe = stream[..front].to_vec();
        let capacity = probe.capacity();
        let mut reader =
            WindowedFile::from_seq_pipe(Rc::new(file), probe, block, SeqTailSlot::default());
        let (mut spans, mut peak) = (Vec::new(), 0);
        while let Some(window) = reader.next_window().expect("reads") {
            peak = peak.max(window.raw.len());
            spans.extend(
                window
                    .values
                    .iter()
                    .map(|&(start, end)| (window.offset + start, window.offset + end)),
            );
        }
        assert_eq!(spans, want.spans);
        // The long record and the short one kept before it; every other window is about a block.
        assert!(peak <= long.len() + block, "window of {peak}");
        assert_eq!(reader.buf.capacity(), capacity, "grown");
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

    /// #7218: the prepass vouches for several files when each is plain records ending in an LF, and for
    /// none when one is not: no RS to start, a cut-short record, no final LF, an empty file, or a file the
    /// run opens by its path.
    #[cfg(unix)]
    #[test]
    fn the_seq_prepass_vouches_for_several_files_when_each_is_plain_7218() {
        let open = |text: &[u8]| {
            let mut file = tempfile::tempfile().expect("a temp file");
            file.write_all(text).expect("written");
            (Rc::new(file), text.len() as u64)
        };
        let plain: &[u8] = b"\x1e{\"a\":1}\n";
        let many = |texts: &[&[u8]]| {
            let files: Vec<_> = texts.iter().map(|text| open(text)).collect();
            Reread::new_many(&files).is_plain_seq_records()
        };
        assert!(many(&[plain]));
        assert!(many(&[plain, plain]));
        assert!(many(&[plain, b"\x1e1\n\x1e[2]\n", plain]));
        for bad in [
            &b"{\"a\":1}\n"[..],          // no RS to start
            b"\x1e{\"a\":1}",             // no final LF
            b"\x1e{\"a\":\n",             // a cut-short record that ends in an LF
            b"\x1e{\"a\":1}\n\x1e{\"b\"", // a record cut short at the end
            b"",                          // no bytes
        ] {
            assert!(!many(&[plain, bad]), "{bad:?} second");
            assert!(!many(&[bad, plain]), "{bad:?} first");
        }
        assert!(!Reread::new_many(&[]).is_plain_seq_records());
        let named = Rc::new(NamedFile::new(PathBuf::from("/nonexistent")));
        assert!(!Reread::new_named(&[named]).is_plain_seq_records());
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

    /// The documents a [`WindowedFile`] of `block` bytes a read gives of a file that held `before` when it
    /// was opened (its length then) and `appended` after that, with the reads stopping at that length
    /// when `stop_at_len`; and the bytes it read.
    fn grown(
        before: &[u8],
        appended: &[u8],
        block: usize,
        stop_at_len: bool,
    ) -> (Vec<String>, u64) {
        let named = tempfile::NamedTempFile::new().expect("a temp file");
        std::fs::write(named.path(), before).expect("written");
        let file = File::open(named.path()).expect("opens");
        let mut reader = WindowedFile::new(file, before.len() as u64, block);
        reader.stop_at_len = stop_at_len;
        let mut appender = std::fs::OpenOptions::new()
            .append(true)
            .open(named.path())
            .expect("opens to append");
        appender.write_all(appended).expect("grown");
        let mut documents = Vec::new();
        while let Some(window) = reader.next_window().expect("reads") {
            assert!(window.split_error.is_none(), "no fault");
            for &(start, end) in &window.values {
                documents.push(String::from_utf8_lossy(&window.raw[start..end]).into_owned());
            }
        }
        (documents, reader.bytes_read)
    }

    /// #7446: a file that grows while it is read is read on past the length it had when it was opened, to
    /// the end it has when the reads get there, as jq's reader reads it: the lines a writer appended, and
    /// a last document that was cut short then and completed after, whatever the block. A `--seq` file,
    /// vouched for up to that length, stops there ([`WindowedFile::stop_at_len`]).
    #[test]
    fn a_file_that_grows_is_read_to_the_end_it_has_then_7446() {
        let all = |n: usize| (1..=n).map(|i| i.to_string()).collect::<Vec<_>>();
        for block in [1, 2, 4, 6, 7, 64, 4096] {
            let (documents, read) = grown(b"1\n2\n3\n", b"4\n5\n", block, false);
            assert_eq!(documents, all(5), "block {block}");
            assert_eq!(read, 10, "block {block}");
            let (documents, read) = grown(b"1\n2\n[3,", b"4]\n5\n", block, false);
            assert_eq!(documents, ["1", "2", "[3,4]", "5"], "block {block}");
            assert_eq!(read, 12, "block {block}");
            // A file nothing was appended to reads as it always did.
            let (documents, read) = grown(b"1\n2\n3\n", b"", block, false);
            assert_eq!(documents, all(3), "block {block}");
            assert_eq!(read, 6, "block {block}");
            let (documents, read) = grown(b"1\n2\n3\n", b"4\n5\n", block, true);
            assert_eq!(documents, all(3), "block {block}");
            assert_eq!(read, 6, "block {block}");
        }
        // Much more than was there when it was opened, read a block at a time past it.
        let more: String = (4..=2000).map(|i| format!("{i}\n")).collect();
        let (documents, _) = grown(b"1\n2\n3\n", more.as_bytes(), 64, false);
        assert_eq!(documents, all(2000));
    }

    /// #7446: a file cut short while it is read ends where the reads find its end, short of the length it
    /// had when it was opened, with no fault for what is gone.
    #[test]
    fn a_file_cut_short_while_it_is_read_ends_there_7446() {
        let named = tempfile::NamedTempFile::new().expect("a temp file");
        std::fs::write(named.path(), b"1\n2\n3\n4\n5\n").expect("written");
        let file = File::open(named.path()).expect("opens");
        let mut reader = WindowedFile::new(file, 10, 4);
        let first = reader.next_window().expect("reads").expect("a window");
        assert_eq!(first.values.len(), 2);
        std::fs::OpenOptions::new()
            .write(true)
            .open(named.path())
            .and_then(|file| file.set_len(6))
            .expect("cut short");
        let mut documents = 2;
        while let Some(window) = reader.next_window().expect("reads") {
            assert!(window.split_error.is_none());
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

    /// #7323: the window a pipe's probe makes is the probe's own buffer, not a copy of it, so the probe
    /// is not held twice. A window read onto text ([`LineWindows::next_window_after`]) is still the
    /// text's buffer.
    #[test]
    fn a_carried_front_is_the_windows_buffer_not_a_copy_7323() {
        let probe = b"a\nbc\nd".to_vec();
        let at = probe.as_ptr();
        let mut windows = LineWindows::new(
            Box::new(io::Cursor::new(b"ef\ngh\n".to_vec())),
            None,
            4,
            probe,
        );
        let window = windows
            .next_window(None, |_| ())
            .expect("the probe is a window");
        assert_eq!(window, b"a\nbc\n");
        assert_eq!(window.as_ptr(), at, "the probe was copied");
        let text = Vec::with_capacity(64);
        let at = text.as_ptr();
        let window = windows
            .next_window_after(None, text, |_| ())
            .expect("the rest is a window");
        assert_eq!(window, b"def\n");
        assert_eq!(window.as_ptr(), at, "the window was not read onto the text");
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

    /// #7264: the first window of a source started on a few KiB reads no more than the rest of the
    /// front, and the windows after it are the block's again; the lines are the same either way.
    #[test]
    fn the_first_window_reads_no_more_than_it_was_asked_to_7264() {
        let text: Vec<u8> = (0..40)
            .flat_map(|n| format!("{n:07}\n").into_bytes())
            .collect();
        let windows = |first: Option<usize>| {
            let mut w = LineWindows::new(
                Box::new(io::Cursor::new(text.clone())),
                Some(text.len() as u64),
                64,
                Vec::new(),
            );
            if let Some(first) = first {
                w = w.with_first_window(first);
            }
            std::iter::from_fn(|| w.next_window(Some(1000), |_| ())).collect::<Vec<_>>()
        };
        let plain = windows(None);
        let small = windows(Some(16));
        assert_eq!(plain.len(), 1, "one window of the 320 bytes");
        assert_eq!(
            small[0],
            text[..16].to_vec(),
            "the first stops at its 16 bytes"
        );
        assert!(small.len() >= 2, "{} windows", small.len());
        assert_eq!(small.concat(), plain.concat(), "the same lines");
    }

    /// #7264: a window read onto a text is that text and the window, cut where the window alone would
    /// be; a read that fails keeps the text whole and drops only the line it cut; no window hands the
    /// text back.
    #[test]
    fn a_window_read_onto_a_text_is_the_text_and_the_window_7264() {
        let text: Vec<u8> = (0..40)
            .flat_map(|n| format!("{n:07}\n").into_bytes())
            .collect();
        let source = || {
            LineWindows::new(
                Box::new(io::Cursor::new(text.clone())),
                None,
                20,
                b"carried\n".to_vec(),
            )
        };
        let mut alone = source();
        let mut onto = source();
        let prefix = b"no newline at the end of the text".to_vec();
        loop {
            let window = alone.next_window(None, |_| ());
            let after = onto.next_window_after(None, prefix.clone(), |_| ());
            match (window, after) {
                (Some(window), Ok(after)) => {
                    assert_eq!(after, [prefix.as_slice(), &window].concat());
                }
                (None, Err(back)) => {
                    assert_eq!(back, prefix);
                    break;
                }
                (window, after) => panic!("{window:?} against {after:?}"),
            }
        }
        let mut failing = LineWindows::new(
            Box::new(Failing(io::Cursor::new(b"a\nb\ncut".to_vec()))),
            None,
            64,
            Vec::new(),
        );
        let mut reported = 0;
        let kept = failing.next_window_after(None, b"head".to_vec(), |_| reported += 1);
        assert_eq!(kept, Ok(b"heada\nb\n".to_vec()));
        assert_eq!(reported, 1);
        let mut no_newline = LineWindows::new(
            Box::new(Failing(io::Cursor::new(b"cut".to_vec()))),
            None,
            64,
            Vec::new(),
        );
        assert_eq!(
            no_newline.next_window_after(None, b"x\ny".to_vec(), |_| ()),
            Err(b"x\ny".to_vec()),
            "the text's own line break is not the window's"
        );
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
        bare_token_lost_to_fault, first_with_bytes_is_a_cut_bom, parse_error_line_after,
        stitch_json_file_seams, strip_leading_json_bom, utf8_lossy_document, LineCounter,
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

    /// The seam in the terms a reader of it can tell apart: without `lead` (#7569) and `raw_ends`, which the whole-file route
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
            // The BOM's lead counts only on the file's first line, which a window that starts past
            // dropped blank lines no longer holds: the lines of the documents say whether it counted.
            lead: 0,
            ..seam.clone()
        }
    }

    fn whole_route(files: &[Vec<u8>]) -> Run {
        let mut raws: Vec<Vec<u8>> = files.to_vec();
        let cuts = strip_leading_json_bom(raws.iter_mut());
        let mut seams = stitch_json_file_seams(raws.iter_mut(), false, false);
        // The mark jq's first read held, as `get_inputs` records it (#7569).
        super::super::note_bom_leads(&mut seams, &cuts, raws.len());
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
        // A list that opens with a mark cut across files is not the windows' to read (#7566).
        if first_with_bytes_is_a_cut_bom(files) {
            return;
        }
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

    /// #7024: the rest a `LineWindows` hands over is what its windows would have read: up to the length
    /// the source had when it was opened, not what was appended since.
    #[test]
    fn the_rest_stops_at_the_length_known_at_the_start_7024() {
        let source = b"a\nb\nc\nappended later\n".to_vec();
        let mut windows = LineWindows::new(
            Box::new(std::io::Cursor::new(source)),
            Some(6),
            2,
            Vec::new(),
        );
        assert_eq!(windows.next_window(None, |_| {}), Some(b"a\n".to_vec()));
        assert_eq!(windows.take_rest(|_| {}), b"b\nc\n".to_vec());
        assert!(windows.finished());
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

    /// A file of a long list that `-R` reads is opened when it is first read (#7024): one that will not
    /// open reads as no bytes, however often it is read, and its failure is noted once.
    #[cfg(unix)]
    #[test]
    fn a_file_read_when_reached_that_will_not_open_reads_as_none_and_is_noted_once_7024() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let present = dir.path().join("present.txt");
        std::fs::write(&present, b"one\ntwo\n").expect("written");
        let mut seen = String::new();
        let mut reader = OpenWhenRead::new(present.clone(), 3, false);
        reader.read_to_string(&mut seen).expect("reads");
        assert_eq!(seen, "one\ntwo\n");
        // Read to its end, it holds no descriptor (#7197), and reads nothing more though the file grows.
        assert!(reader.file.is_none(), "closed at its end");
        std::fs::write(&present, b"one\ntwo\nthree\n").expect("written");
        let mut buf = [0u8; 16];
        assert_eq!(reader.read(&mut buf).expect("reads as empty"), 0);
        assert!(reader.file.is_none(), "not opened again");
        let missing = dir.path().join("missing.txt");
        super::super::PENDING_OPEN_FAILURES.with(|pending| pending.borrow_mut().clear());
        let mut reader = OpenWhenRead::new(missing, 7, true);
        assert_eq!(reader.read(&mut buf).expect("reads as empty"), 0);
        assert_eq!(reader.read(&mut buf).expect("still empty"), 0);
        let noted = super::super::PENDING_OPEN_FAILURES.with(|pending| pending.borrow().len());
        assert_eq!(noted, 1, "the failure is noted once");
        // A directory opens on Unix and fails at its first read: it is noted too.
        let mut reader = OpenWhenRead::new(dir.path().to_path_buf(), 8, true);
        assert_eq!(reader.read(&mut buf).expect("reads as empty"), 0);
        let noted = super::super::PENDING_OPEN_FAILURES.with(|pending| pending.borrow().len());
        assert_eq!(noted, 2);
        super::super::PENDING_OPEN_FAILURES.with(|pending| pending.borrow_mut().clear());
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

    /// The documents a `--seq` run over `files` opened by path reads in windows (#7221), each with its
    /// file's position, and where it says the stream stopped.
    #[cfg(unix)]
    fn seq_named_run(
        dir: &std::path::Path,
        files: &[Vec<u8>],
        remove: &[usize],
    ) -> (Vec<(usize, Vec<u8>)>, Option<SeqFilesTail>) {
        let named = named_files(dir, files);
        for &n in remove {
            std::fs::remove_file(dir.join(format!("f{n}.json"))).expect("removed");
        }
        let slot = SeqFilesTailSlot::default();
        let mut reader = WindowedFiles::named(&named, 4096)
            .blanking_rs()
            .certifying_seq_files(Rc::clone(&slot));
        let mut docs = Vec::new();
        while let Some((index, window)) = reader.next_window_noting(&mut |_, _| {}).expect("reads")
        {
            for &(start, end) in &window.values {
                docs.push((index, window.raw[start..end].to_vec()));
            }
        }
        assert!(
            reader.next_window().expect("reads").is_none(),
            "an ended stream stays ended"
        );
        let tail = slot.borrow_mut().take();
        (docs, tail)
    }

    /// #7221: a `--seq` list of files opened by path is judged a file at a time, on the handle the run
    /// opened when it reached each: the records of the plain files before the first that is not are read,
    /// and the stream ends before that file, which is handed on with the line breaks before it and the
    /// byte the records left at index 1 of jq's token buffer (the last that wrote one, across files).
    #[cfg(unix)]
    #[test]
    fn seq_files_opened_by_path_stop_at_the_first_that_is_not_plain_7221() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let doc = |text: &str| text.as_bytes().to_vec();
        let plain = [
            b"\x1e1\n\x1e\"ab\"\n".to_vec(),
            b"\x1e[]\n\x1e{\n}\n".to_vec(),
        ];
        let read_of_plain = vec![
            (0, doc("1")),
            (0, doc("\"ab\"")),
            (1, doc("[]")),
            (1, doc("{\n}")),
        ];
        for (name, turn) in [
            ("no RS to start", b"3\n".to_vec()),
            ("empty", Vec::new()),
            ("a BOM", b"\xef\xbb\xbf\x1e3\n".to_vec()),
        ] {
            let mut files = plain.to_vec();
            files.push(turn);
            files.push(b"\x1e5\n".to_vec());
            let (docs, tail) = seq_named_run(dir.path(), &files, &[]);
            assert_eq!(docs, read_of_plain, "{name}");
            assert_eq!(
                tail,
                Some(SeqFilesTail {
                    index: 2,
                    lines: 5,
                    token_byte: Some(b'b'),
                    front: None,
                }),
                "{name}"
            );
        }
        // #7341: a file that opens with an RS is read by a gate of its own: its plain records are, and
        // the rest of it, from a stretch that runs to its end, is handed on as its bytes behind the front
        // that resumes jq's reader after its line before it.
        for (name, turn, rest) in [
            ("no final LF", b"\x1e3\n\x1e4".to_vec(), b"\x1e4".to_vec()),
            (
                "a record that is no document",
                b"\x1e3\n\x1e{\n".to_vec(),
                b"\x1e{\n".to_vec(),
            ),
        ] {
            let mut files = plain.to_vec();
            files.push(turn);
            files.push(b"\x1e5\n".to_vec());
            let (docs, tail) = seq_named_run(dir.path(), &files, &[]);
            let mut read = read_of_plain.clone();
            read.push((2, doc("3")));
            assert_eq!(docs, read, "{name}");
            assert_eq!(
                tail,
                Some(SeqFilesTail {
                    index: 2,
                    lines: 5,
                    // The `3` the file's gate read ends jq's token with a NUL at index 1.
                    token_byte: Some(0),
                    front: Some(SeqFront {
                        stream: crate::cli::jq_seq_reader::resumed_stream(1, rest),
                        error: None,
                        faulted_rs: None,
                    }),
                }),
                "{name}"
            );
        }
        // A file with a stretch it carries on past is read to its end, and the files after it are.
        let mut files = plain.to_vec();
        files.push(b"\x1enan\n\x1e} 6\n\x1e7\n".to_vec());
        files.push(b"\x1e\"cd\"\n".to_vec());
        let (docs, tail) = seq_named_run(dir.path(), &files, &[]);
        let mut read = read_of_plain.clone();
        // jq's reader warns at the `}` and reads the `6` after it.
        read.extend([
            (2, doc("nan")),
            (2, doc("6")),
            (2, doc("7")),
            (3, doc("\"cd\"")),
        ]);
        assert_eq!(docs, read);
        assert_eq!(tail, None);
        // Every file plain: nothing is handed on.
        let mut files = plain.to_vec();
        files.push(b"\x1e5\n".to_vec());
        let (docs, tail) = seq_named_run(dir.path(), &files, &[]);
        assert_eq!(docs.len(), 5);
        assert_eq!(tail, None);
        // The first file is not plain: the whole stream is handed on, at its start.
        let (docs, tail) = seq_named_run(dir.path(), &[b"7\n".to_vec(), plain[0].clone()], &[]);
        assert_eq!(docs, vec![]);
        assert_eq!(
            tail,
            Some(SeqFilesTail {
                index: 0,
                lines: 0,
                token_byte: None,
                front: None,
            })
        );
        // A file that does not open is no bytes, judged as nothing: it neither stops the windows nor
        // counts a line.
        let files = [plain[0].clone(), b"\x1e8\n".to_vec(), b"9\n".to_vec()];
        let (docs, tail) = seq_named_run(dir.path(), &files, &[1]);
        assert_eq!(docs, vec![(0, doc("1")), (0, doc("\"ab\""))]);
        assert_eq!(
            tail,
            Some(SeqFilesTail {
                index: 2,
                lines: 2,
                token_byte: Some(b'b'),
                front: None,
            })
        );
    }

    /// #7221: the byte a file's plain records leave at index 1 of jq's token buffer is
    /// [`crate::cli::jq_seq_reader::token_byte_after`]'s over the whole file, read from the end a stretch
    /// at a time: past a last record bigger than a stretch, and past records that write nothing there.
    #[cfg(unix)]
    #[test]
    fn seq_token_byte_is_read_from_the_end_of_the_file_7221() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let mut big = b"\x1e\"xy\"\n\x1e[".to_vec();
        while big.len() < 3 * TAIL_PROBE as usize {
            big.extend_from_slice(b"[],");
        }
        big.extend_from_slice(b"[]]\n");
        let mut many_empty = b"\x1e\"pq\"\n".to_vec();
        while many_empty.len() < 2 * TAIL_PROBE as usize {
            many_empty.extend_from_slice(b"\x1e{}\n");
        }
        for (name, bytes) in [
            ("one record", b"\x1e\"ab\"\n".to_vec()),
            ("a number last", b"\x1e\"ab\"\n\x1e7\n".to_vec()),
            ("nothing writes it", b"\x1e[]\n\x1e\"\"\n".to_vec()),
            ("a big last record", big),
            ("many records that write nothing", many_empty),
        ] {
            let path = dir.path().join("f.seq");
            std::fs::write(&path, &bytes).expect("written");
            let file = File::open(&path).expect("opens");
            assert_eq!(
                seq_token_byte_in(&file, bytes.len() as u64).expect("reads"),
                crate::cli::jq_seq_reader::token_byte_after(&bytes),
                "{name}"
            );
        }
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

    /// `text` read as a pipe whose probe read it in `blocks` blocks of `block` bytes (#7260): the first is
    /// the front, the rest are read before the source.
    fn piped_blocks(text: &[u8], blocks: usize, block: usize) -> (Run, usize, Option<String>) {
        let mut file = tempfile::tempfile().expect("a temp file");
        file.write_all(text).expect("written");
        let probe = (blocks * block).min(text.len());
        file.seek(SeekFrom::Start(probe as u64)).expect("seeked");
        let mut read: std::collections::VecDeque<Vec<u8>> =
            text[..probe].chunks(block).map(<[u8]>::to_vec).collect();
        let front = read.pop_front().unwrap_or_default();
        let seen = SeenFault::default();
        let mut reader = WindowedFile::from_pipe(Rc::new(file), front, None, block, seen.clone())
            .then_reads(read);
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
        let bom = crate::cli::front_matter::leading_bom_len(text);
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
        // #7260: a probe read in blocks, of every count up to the whole text.
        for blocks in [1, 2, 4, text.len() / block.max(1) + 1] {
            let (run, _, fault) = piped_blocks(text, blocks, block);
            assert_eq!(run, file_run, "documents at block {block}, {blocks} blocks");
            assert_eq!(fault, file_fault, "fault at block {block}, {blocks} blocks");
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

    /// The probe's front is handed out a block at a time from the buffer it was read into (#7266): no
    /// window is much over a block, and the buffer is never grown, where a block read onto the front
    /// doubled it and made one window of the front and a block, which the certifier copied whole. The
    /// documents, their lines and a BOM skipped at the front are the file route's.
    #[test]
    fn a_pipe_hands_its_probe_out_a_block_at_a_time_7266() {
        let line = "{\"name\":\"n\",\"v\":[1,2,3]}\n";
        let block = 1_000;
        let mut text = b"\xEF\xBB\xBF".to_vec();
        text.extend(line.repeat(2_000).as_bytes());
        let front = 4 * block;
        for front in [front, front + 3, text.len()] {
            let mut file = tempfile::tempfile().expect("a temp file");
            file.write_all(&text).expect("written");
            file.seek(SeekFrom::Start(front as u64)).expect("seeked");
            let probe = text[..front].to_vec();
            let capacity = probe.capacity();
            let seen = SeenFault::default();
            let mut reader =
                WindowedFile::from_pipe(Rc::new(file), probe, None, block, seen.clone());
            let (run, peak) = drain(&mut reader);
            // A window keeps the line the last one ended on, and the read after the probe completes
            // the part of a line it ends with.
            assert!(
                peak <= block + 2 * line.len(),
                "front {front}: window of {peak}"
            );
            // #7260: a window the certifying thread still reads keeps its bytes in a buffer of its
            // own, a block and a line, so the text is never more than the probe's buffer either.
            assert!(
                reader.text_capacity() <= capacity,
                "front {front}: grown to {}",
                reader.text_capacity()
            );
            assert_eq!(seen.get(), None);
            assert_eq!(run, as_file(&text, block).0, "front {front}");
        }
    }

    /// #7260: the certifying thread reads each window's own buffer, not a copy. A window it still holds
    /// when the next is asked for keeps its bytes in a buffer of its own; one it is done with is the
    /// buffer again. Either way the windows are the file route's.
    #[test]
    fn a_window_the_thread_still_reads_is_kept_apart_7260() {
        let line = "{\"name\":\"n\",\"v\":[1,2,3]}\n";
        let text = line.repeat(400);
        let block = 1_000;
        for hold in [false, true] {
            let mut file = tempfile::tempfile().expect("a temp file");
            file.write_all(text.as_bytes()).expect("written");
            file.seek(SeekFrom::Start(block as u64)).expect("seeked");
            let seen = SeenFault::default();
            let mut reader = WindowedFile::from_pipe(
                Rc::new(file),
                text.as_bytes()[..block].to_vec(),
                None,
                block,
                seen.clone(),
            );
            let mut run = Vec::new();
            let mut windows = 0;
            loop {
                // The thread's share of the last window, while it would still read it.
                let held = hold.then(|| reader.held.clone()).flatten();
                let Some(window) = reader.next_window().expect("reads") else {
                    break;
                };
                windows += 1;
                run.extend(
                    window
                        .values
                        .iter()
                        .map(|&(start, end)| window.raw[start..end].to_vec()),
                );
                drop(held);
            }
            assert_eq!(seen.get(), None);
            assert_eq!(run.len(), 400, "hold {hold}");
            assert!(windows > 2, "{windows}");
            if hold {
                // Every window but the first is asked for while the one before is held.
                assert_eq!(reader.kept_apart, windows - 1, "hold {hold}");
            }
            let want: Vec<Vec<u8>> = as_file(text.as_bytes(), block)
                .0
                .docs
                .into_iter()
                .map(|(doc, _)| doc)
                .collect();
            assert_eq!(run, want, "hold {hold}");
        }
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
        let mut text = b"[".to_vec();
        text.extend("1,".repeat(certifier_copy_limit()).as_bytes());
        text.extend_from_slice(b"2]\n{\"x\":");
        // The whole stream is what the probe read, and its first line break ends the big document, so
        // the first window is that document (a window made of the probe ends at its first block's last
        // line break, #7266, and this one has none). A small line before it would be a window of its
        // own, which would start the thread.
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

    /// The last window is not certified as it is handed out (#7294): its buffer is shared with the
    /// certifier, not copied, and certified when the fault is asked for, as it was before; a fault an
    /// earlier window holds is the stream's first, and the last is then not certified at all.
    #[test]
    fn the_last_window_is_certified_when_the_fault_is_asked_for_7294() {
        let big = |tail: &str| {
            let mut text = b"{\"v\":[".to_vec();
            text.extend("1,".repeat(20_000).as_bytes());
            text.extend_from_slice(format!("1],\"m\":{tail}}}").as_bytes());
            text
        };
        let early = b"{\"e\":[1 2]}\n".to_vec();
        // The stream, how much of it the probe took, and how many reads by jq's parser certifying it
        // takes in all: one for a document the validator turns down, none for a fault at the end of
        // the stream (the end's own wording) or for none.
        for (front, text, reads) in [
            // One document, read on past the probe and read whole in it; one cut short; one after
            // small documents, which start the thread; one with no fault.
            (100, big("[1 2]"), 1),
            (usize::MAX, big("[1 2]"), 1),
            (100, big("[1 2]")[..30_000].to_vec(), 0),
            (
                100,
                [b"{\"a\":1}\n2 3\n".to_vec(), big("[1 2]")].concat(),
                1,
            ),
            (100, big("1"), 0),
            // The early fault is the stream's first: the last window's is never read.
            (
                100,
                [early.clone(), b"{\"a\":1}\n".repeat(500), big("[1 2]")].concat(),
                1,
            ),
        ] {
            let mut file = tempfile::tempfile().expect("a temp file");
            file.write_all(&text).expect("written");
            let front = front.min(text.len());
            file.seek(SeekFrom::Start(front as u64)).expect("seeked");
            let seen = SeenFault::default();
            let mut reader = WindowedFile::from_pipe(
                Rc::new(file),
                text[..front].to_vec(),
                None,
                4_096,
                seen.clone(),
            );
            let mut last_raw = None;
            while let Some(window) = reader.next_window().expect("reads") {
                last_raw = Some(window.raw.as_ptr());
            }
            let what = String::from_utf8_lossy(&text[..40]).into_owned();
            // Unless the thread found the early fault first, the window handed out last is the
            // buffer the certifier holds, not yet certified.
            if !text.starts_with(&early) {
                let held = match &seen.lock().last {
                    Some(Job {
                        text: JobText::Held(held, _),
                        ..
                    }) => Arc::clone(held),
                    _ => panic!("{what}: the last window is left for the reader of the fault"),
                };
                assert_eq!(Some(held.as_ptr()), last_raw, "{what}");
                assert!(Arc::ptr_eq(&held, reader.held.as_ref().expect("held")));
            }
            let expected = succinctly::jq::jq_stream_fault([&text[..]], 1, 0);
            assert_eq!(seen.get(), expected, "{what}");
            assert!(seen.lock().last.is_none(), "{what}");
            assert_eq!(reader.exact_reads(), reads, "{what}");
            // Asked again, it is not certified again.
            assert_eq!(seen.get(), expected, "{what}");
            assert_eq!(reader.exact_reads(), reads, "{what}");
        }
    }

    /// The same over-limit document after a small one (#7106's own shape): the small one is a window of
    /// its own, which starts the thread, and the big one is then certified here (`certify_here`, after
    /// the thread's window), with the fault after it worded on a later line, from a window that does
    /// not start the stream.
    #[test]
    fn an_over_limit_window_after_the_thread_started_is_certified_here_7266() {
        let mut text = b"{\"a\":1}\n[".to_vec();
        text.extend("1,".repeat(certifier_copy_limit()).as_bytes());
        text.extend_from_slice(b"2]\n{\"x\":");
        let file = tempfile::tempfile().expect("a temp file");
        let seen = SeenFault::default();
        let mut reader =
            WindowedFile::from_pipe(Rc::new(file), text.clone(), None, 4_096, seen.clone());
        let mut origins = Vec::new();
        while let Some(window) = reader.next_window().expect("reads") {
            origins.push((window.origin, window.raw.len()));
        }
        assert!(origins.len() >= 2, "{origins:?}");
        assert!(origins[1].1 > certifier_copy_limit(), "{origins:?}");
        assert_ne!(origins.last().expect("windows").0, (1, 0), "{origins:?}");
        let fault = seen.get();
        assert_eq!(fault, succinctly::jq::jq_stream_fault([&text[..]], 1, 0));
        assert!(fault.is_some_and(|fault| fault.contains("line 3")));
        assert!(reader.certifier.as_ref().expect("a pipe").worker.is_some());
    }

    /// A document over several lines that does not end within a block of the probe is read as the
    /// probe handed out whole read it (#7266): the rest of the probe and a block onto it, so no window
    /// is more than the probe and a block, where the look-ahead read eight blocks onto the probe. One
    /// that starts after windows of the probe were handed out reads its block into the room they
    /// freed, so the buffer is not grown; documents back to back from the stream's front grow it as
    /// the whole probe's first window did.
    #[test]
    fn a_document_over_several_lines_in_the_probe_reads_as_the_whole_probe_did_7266() {
        let block = 1_000;
        let pretty = |id: usize, len: usize| {
            let mut doc = format!("{{\"id\":{id},\"v\":[\n");
            while doc.len() < len {
                doc.push_str(" 12345,\n");
            }
            doc.push_str(" 0]}\n");
            doc
        };
        let record = "{\"id\":1,\"name\":\"n\"}\n";
        let back_to_back: String = (0..3).map(|id| pretty(id, 1_400)).collect();
        let partway = format!(
            "{}{}{}",
            record.repeat(60),
            pretty(9, 2_500),
            record.repeat(10)
        );
        let front = 4 * block;
        for (shape, grows) in [(back_to_back, true), (partway, false)] {
            let mut text = shape.into_bytes();
            text.extend(record.repeat(400).as_bytes());
            let mut file = tempfile::tempfile().expect("a temp file");
            file.write_all(&text).expect("written");
            file.seek(SeekFrom::Start(front as u64)).expect("seeked");
            let probe = text[..front].to_vec();
            let capacity = probe.capacity();
            let seen = SeenFault::default();
            let mut reader =
                WindowedFile::from_pipe(Rc::new(file), probe, None, block, seen.clone());
            let (run, peak) = drain(&mut reader);
            assert!(peak <= front + block, "grows {grows}: window of {peak}");
            if !grows {
                assert!(reader.text_capacity() <= capacity, "grown");
            }
            assert_eq!(seen.get(), None);
            assert_eq!(run, as_file(&text, block).0, "grows {grows}");
        }
    }

    /// A window of the probe is not handed out with half a block or more of an open document at its
    /// end, which the next window would hand out (and the certifier copy) again (#7266): records, then a
    /// document over several lines that starts late in a block and ends past the next.
    #[test]
    fn a_window_of_the_probe_does_not_end_in_a_long_open_document_7266() {
        let block = 1_000;
        let record = "{\"id\":1,\"name\":\"n\"}\n";
        let mut text = record.repeat(60).into_bytes();
        text.extend_from_slice(b"{\"v\":[\n");
        while text.len() < 3_800 {
            text.extend_from_slice(b" 12345,\n");
        }
        text.extend_from_slice(b" 0]}\n");
        text.extend(record.repeat(300).as_bytes());
        let mut file = tempfile::tempfile().expect("a temp file");
        file.write_all(&text).expect("written");
        file.seek(SeekFrom::Start(4 * block as u64))
            .expect("seeked");
        let seen = SeenFault::default();
        let probe = text[..4 * block].to_vec();
        let mut reader = WindowedFile::from_pipe(Rc::new(file), probe, None, block, seen.clone());
        let (mut docs, mut end) = (0, 0usize);
        while let Some(window) = reader.next_window().expect("reads") {
            docs += window.values.len();
            // What this window hands out again of the one before: the line its last document ended
            // on, and an open document's text.
            let again = end.saturating_sub(window.offset);
            assert!(again <= block / 2, "{again} bytes handed out again");
            end = window.offset + window.raw.len();
        }
        assert_eq!(docs, 361);
        assert_eq!(seen.get(), None);
    }

    /// A document over several lines much longer than the probe, at the stream's front, is scanned
    /// about as often as when the probe was handed out whole (#7266): the probe's first block, then
    /// the probe and a block, the look-ahead of eight blocks and the rest, not a block of the probe
    /// at a time.
    #[test]
    fn a_long_document_at_the_front_is_not_scanned_a_block_at_a_time_7266() {
        let block = 1_000;
        let mut text = b"{\"v\":[\n".to_vec();
        while text.len() < 10 * block {
            text.extend_from_slice(b" 12345,\n");
        }
        text.extend_from_slice(b" 0]}\n");
        let mut file = tempfile::tempfile().expect("a temp file");
        file.write_all(&text).expect("written");
        file.seek(SeekFrom::Start(4 * block as u64))
            .expect("seeked");
        let seen = SeenFault::default();
        let probe = text[..4 * block].to_vec();
        let mut reader = WindowedFile::from_pipe(Rc::new(file), probe, None, block, seen.clone());
        let (run, _) = drain(&mut reader);
        assert_eq!(run.docs.len(), 1);
        // 1 + 5 + 8 + 10 blocks, and a line's slack on each.
        assert!(
            reader.split_bytes <= 24 * block + 100,
            "scanned {}",
            reader.split_bytes
        );
        assert_eq!(seen.get(), None);
    }

    /// A document over several lines that is still open where the probe ends is read on a block at a
    /// time to two blocks past the probe before the look-ahead of eight blocks (#7266): its window is
    /// the document and about a block, which is what the probe handed out whole held for it.
    #[test]
    fn a_document_open_at_the_probes_end_is_read_on_a_block_at_a_time_7266() {
        let block = 1_000;
        let mut text = "{\"id\":1,\"name\":\"n\"}\n".repeat(75).into_bytes();
        let start = text.len();
        text.extend_from_slice(b"{\"v\":[\n");
        while text.len() < start + 3_900 {
            text.extend_from_slice(b" 12345,\n");
        }
        text.extend_from_slice(b" 0]}\n");
        let len = text.len() - start;
        assert!(start + len > 5 * block && start + len < 6 * block);
        text.extend("{\"id\":2,\"name\":\"m\"}\n".repeat(600).as_bytes());
        let mut file = tempfile::tempfile().expect("a temp file");
        file.write_all(&text).expect("written");
        file.seek(SeekFrom::Start(4 * block as u64))
            .expect("seeked");
        let seen = SeenFault::default();
        let probe = text[..4 * block].to_vec();
        let mut reader = WindowedFile::from_pipe(Rc::new(file), probe, None, block, seen.clone());
        let (run, peak) = drain(&mut reader);
        assert!(
            peak <= len + 2 * block,
            "a document of {len}: window of {peak}"
        );
        assert_eq!(seen.get(), None);
        assert_eq!(run, as_file(&text, block).0);
    }

    /// The probe's bytes move with a substitution in a window made of them (#7266): while nothing has
    /// been read onto the probe, they end where the buffer does.
    #[test]
    fn a_substitution_in_the_probe_moves_where_it_ends_7266() {
        let mut text = Vec::new();
        for i in 0..300 {
            text.extend_from_slice(format!("{{\"i\":{i},\"s\":\"").as_bytes());
            text.extend_from_slice(b"\xff\xfe");
            text.extend_from_slice(b"\"}\n");
        }
        let file = tempfile::tempfile().expect("a temp file");
        let seen = SeenFault::default();
        let mut reader = WindowedFile::from_pipe(Rc::new(file), text.clone(), None, 1_000, seen);
        let window = reader.next_window().expect("reads").expect("a window");
        assert!(window.substituted);
        assert!(window.raw.len() < text.len());
        // #7260: the window's text is shared with the certifying thread, so it is held, not the buffer.
        let held = reader
            .held
            .as_ref()
            .map_or(reader.buf.len(), |text| text.len());
        assert_eq!(reader.front_end, held);
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
