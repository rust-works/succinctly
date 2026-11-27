//! jq 1.7.1's input reader, entered at sync points: the reader half of ADR-0033's model (#7987,
//! step 3).
//!
//! jq reads every input file through one `jq_util_input_state` (`src/util.c`) that feeds one parser
//! (`src/jv_parse.c`), and its answer at the edges of the input (a read boundary, a mark, a fault, a
//! file seam, the end) is a consequence of that pair's state. The parser half is
//! `succinctly::jq::jv_parse`, shared with `fromjson` and `--argjson` (decision 4). This module is
//! the rest:
//!
//! - [`ReaderState`] is jq's whole reader at a sync point: the file, how far it is read and whether
//!   `fgets` has met its end, `current_line`, and the parser's [`ParserSync`]. It is the only thing a
//!   route may carry across a window, a file or a hand-off.
//! - [`ReaderState::step`] is the faithful port of `jq_util_input_read_more`'s reads and of
//!   `jq_util_input_next_input`'s loop (parser mode, without `--slurp`), feeding the parser half.
//!   It runs from one sync point to the next and hands back every value, fault and end of input
//!   jq's `input` would have returned on the way, with the position each was returned at.
//! - [`ReaderState::skip_plain`] is the closed form of `step` over a stretch already known plain
//!   (complete values and white space only), from facts a route already holds: lengths, newline
//!   counts and where each file starts. It reads no byte. `tests.rs` holds the two to each other.
//!
//! Every reader fact comes from a [`ReaderProfile`]: the read length, the parser's depth limit and
//! `--seq`'s read of every record (#7718). The profile's other two exceptions do not arise here:
//! `--stream --slurp` (#7523, #7807) has no streaming parser in this model (step 9), and `-R`
//! (#5476) reads through no parser at all.
//!
//! Departures from jq 1.7.1's source, each a recorded rule: a newline-less read is not cut at its
//! first NUL (`strlen`), since a NUL outside a string is a parse fault of its own (E2, #7966), and
//! the parser half's own departures (#7522, #7525). Input is in memory: a file that fails to open
//! and a read error (`ferror`) are not modelled, and standard input is one more source.
//!
//! The read arithmetic both are built on is the one the routes call (step 4): [`Fgets`], one
//! `fgets` read, and [`LineReads`], where a line's reads fall as a closed form of the offset into
//! it. No route enters [`ReaderState`] itself until step 5, so the items only the model and its
//! tests read are marked as such.

use crate::cli::jq_reader_profile::{ReaderProfile, JQ_READER};
use crate::cli::jq_seq_reader::find_byte;
use succinctly::jq::jv_parse::{Fault, FaultPlace, JvParser, Next, ParserSync, Span, BOM_DONE};

/// One `fgets` call of `jq_util_input_read_more` (`src/util.c`): the bytes it returns from the
/// front of what is left of the file. They end just after the first newline or after `room` bytes,
/// whichever comes first: `room` is the profile's [`read_len`](ReaderProfile::read_len), less what
/// the buffer already holds (the first read of a source whose byte-order mark a route has taken
/// off its text, #7569). The one definition of a read, for [`ReaderState::step`] and every route
/// that finds jq's reads one at a time (#7987 step 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Fgets {
    /// How many bytes it returns.
    pub len: usize,
    /// Whether the last of them is a newline.
    pub newline: bool,
}

impl Fgets {
    /// The read at the front of `rest`, the file's bytes not yet read, or `None` when there are
    /// none: `fgets` returns NULL there and meets the end of the file.
    #[inline]
    pub(crate) fn of(rest: &[u8], room: usize) -> Option<Self> {
        if rest.is_empty() {
            return None;
        }
        let window = &rest[..rest.len().min(room)];
        // #7263: searched a block at a time; a byte at a time this was most of a long line's walk.
        Some(match find_byte(b'\n', window) {
            Some(at) => Self {
                len: at + 1,
                newline: true,
            },
            None => Self {
                len: window.len(),
                newline: false,
            },
        })
    }

    /// Whether it filled its buffer of `room` bytes.
    #[inline]
    pub(crate) const fn fills(self, room: usize) -> bool {
        self.len == room
    }

    /// `feof` after it: it stopped short of both a newline and a full buffer of `room` bytes, so it
    /// met the end of the file. A read that ends on a newline or fills its buffer leaves the end to
    /// the next read, which returns nothing.
    #[inline]
    pub(crate) const fn meets_end(self, room: usize) -> bool {
        !self.newline && self.len < room
    }
}

/// Where jq's reads fall along one line (#7987 step 4). `fgets` reads a line in pieces of
/// [`read_len`](ReaderProfile::read_len) bytes, counted from where the line's reads start (the
/// front of the file, or just after a newline), and the line's newline or the file's end ends the
/// last piece. Which read holds a byte of the line, and where that read ends, are therefore a closed
/// form of the byte's offset into the line: what [`Fgets`] reaches one read at a time, and what
/// [`ReaderState::skip_plain`] reads a stretch's last line with. Offsets here count from where the
/// line's reads start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LineReads {
    read_len: usize,
}

/// The reads of a line under the profile the CLI ships.
pub(crate) const JQ_LINE_READS: LineReads = LineReads::of(&JQ_READER);

impl LineReads {
    /// The reads of a line under `profile`.
    pub(crate) const fn of(profile: &ReaderProfile) -> Self {
        Self {
            read_len: profile.read_len,
        }
    }

    /// Which of the line's reads holds its byte `into`, counted from 0.
    #[inline]
    pub(crate) const fn index(self, into: usize) -> usize {
        into / self.read_len
    }

    /// Where the read holding the line's byte `into` ends, unless the line's newline or the
    /// file's end comes first.
    #[inline]
    pub(crate) const fn end(self, into: usize) -> usize {
        (self.index(into) + 1) * self.read_len
    }

    /// How many of the line's first `len` bytes the read in progress after them holds: 0 when
    /// they fill their reads exactly.
    #[inline]
    pub(crate) const fn in_progress(self, len: usize) -> usize {
        len % self.read_len
    }

    /// Where the read in progress at `at` starts, on a line whose reads start at `line_start`
    /// (both offsets in the same text): `at` itself when the line's bytes before it fill their
    /// reads. At a file's end that is where the read that meets the end starts, and the file's
    /// length when that read is empty.
    #[inline]
    pub(crate) const fn start_before(self, line_start: usize, at: usize) -> usize {
        at - self.in_progress(at - line_start)
    }
}

/// How a run reads its input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct ReadMode {
    /// `--seq` (`JV_PARSE_SEQ`).
    pub seq: bool,
}

/// `current_input`: the file being read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct OpenFile {
    /// How many of its bytes `fgets` has read.
    pub offset: usize,
    /// `feof(current_input)`: a read met its end before a newline or a full buffer, or found
    /// nothing. The next read closes it.
    pub eof: bool,
}

/// jq's reader at a sync point: its parser between top-level values at the start of one of its
/// reads (ADR-0033, "Sync points and state").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct ReaderState {
    /// `curr_file`: how many files `next_file` has handed out. The open file, when there is one,
    /// is the last of them.
    pub next_file: usize,
    /// `current_input`. jq 1.7.1 clears `current_filename` whenever it closes a file, so the file
    /// is named exactly while one is open.
    pub input: Option<OpenFile>,
    /// `current_line`: the reads of the open file that held a newline.
    pub current_line: usize,
    /// The parser.
    pub parser: ParserSync,
}

/// What `input_filename` and `input_line_number` answer (`jq_util_input_get_position`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct Position {
    /// The file, by its index among the inputs; `None` is `<unknown>` (`null`).
    pub file: Option<usize>,
    /// `current_line`.
    pub line: usize,
}

/// What one `jq_util_input_next_input` call returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum Item {
    /// A value, as its span in the inputs read as one stream.
    Value(Span),
    /// A parse error.
    Fault(Fault),
    /// No more input (a plain `jv_invalid()`).
    End,
}

/// One call's answer and the position right after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct Read {
    pub item: Item,
    pub at: Position,
}

/// Where [`ReaderState::step`] stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum Stop {
    /// At the next sync point.
    Synced,
    /// On `Malformed BOM` outside `--seq`: the parser never leaves that state and consumes
    /// nothing in it, so every later call is the same fault, read with no new read.
    Stuck,
}

/// What a route knows about one file's share of a plain stretch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct PlainRun {
    /// The file, by its index among the inputs.
    pub file: usize,
    /// The stretch's bytes in it, from where the reader stands in it (the front of the file, or
    /// for the file being read, its read position).
    pub len: usize,
    /// The newlines among them.
    pub newlines: usize,
    /// The bytes after the last of them (all of them when there is none).
    pub tail: usize,
    /// Whether the stretch reaches the end of the file.
    pub to_end: bool,
    /// Whether the bytes start with the whole UTF-8 byte-order mark. Read only while the parser
    /// has seen no byte: a mark anywhere else is not plain.
    pub bom: bool,
}

/// A stretch the fast path has read and found plain, as [`ReaderState::skip_plain`] takes it.
///
/// The runs name every file the stretch reaches, in order and none skipped, empty ones included,
/// starting with the file the reader stands in when it has one open that it has not read to its
/// end. Every run but the last reaches its file's end. The stretch ends between top-level values
/// (after white space, `]`, `}` or `"`, or at the end of the input) and at the end of a read: after
/// a newline, a whole number of reads after the last one, or at the end of its file.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct PlainStretch<'r> {
    pub runs: &'r [PlainRun],
    /// How many input files the run has.
    pub files: usize,
    /// Read when the stretch reaches the end of the input: whether a top-level value completes in
    /// the buffer jq passes as the last one (the read that met the end of the last file, or the
    /// empty read after it). A bare token the end of input ends completes there, and so does any
    /// value whose last byte, or the delimiter after it, is in that read.
    pub value_in_final_buffer: bool,
}

/// The offset in the inputs read as one stream at which `file` starts.
#[cfg_attr(not(test), allow(dead_code))]
fn file_start(sources: &[&[u8]], file: usize) -> usize {
    sources[..file.min(sources.len())]
        .iter()
        .map(|source| source.len())
        .sum()
}

/// `jq_util_input_state` and its parser, between two sync points.
#[cfg_attr(not(test), allow(dead_code))]
struct Reader<'s, 'b> {
    profile: &'s ReaderProfile,
    mode: ReadMode,
    sources: &'s [&'b [u8]],
    next_file: usize,
    input: Option<OpenFile>,
    current_line: usize,
    parser: JvParser<'b>,
}

#[cfg_attr(not(test), allow(dead_code))]
impl<'s, 'b> Reader<'s, 'b> {
    fn resume(
        state: ReaderState,
        sources: &'s [&'b [u8]],
        profile: &'s ReaderProfile,
        mode: ReadMode,
    ) -> Self {
        let offset = match state.input {
            Some(file) => file_start(sources, state.next_file - 1) + file.offset,
            None => file_start(sources, state.next_file),
        };
        Self {
            profile,
            mode,
            sources,
            next_file: state.next_file,
            input: state.input,
            current_line: state.current_line,
            parser: JvParser::resume(state.parser, mode.seq, profile.max_depth, offset),
        }
    }

    fn state(&self) -> ReaderState {
        ReaderState {
            next_file: self.next_file,
            input: self.input,
            current_line: self.current_line,
            parser: self
                .parser
                .sync()
                .expect("the reader stops at a sync point"),
        }
    }

    fn position(&self) -> Position {
        Position {
            file: self.input.map(|_| self.next_file - 1),
            line: self.current_line,
        }
    }

    /// `jq_util_input_read_more`: the next buffer, and whether it is the last (`is_last`).
    fn read_more(&mut self) -> (&'b [u8], bool) {
        // `map_or(true, ..)`, not `is_none_or`: clippy's `msrv` is 1.73 (clippy.toml).
        if self.input.map_or(true, |file| file.eof) {
            if self.input.take().is_some() {
                // `fclose`; jq 1.7.1 clears `current_filename` and `current_line` here.
                self.current_line = 0;
            }
            if self.next_file < self.sources.len() {
                self.next_file += 1;
                self.input = Some(OpenFile {
                    offset: 0,
                    eof: false,
                });
                self.current_line = 0;
            }
        }
        let mut chunk: &'b [u8] = &[];
        if let Some(file) = self.input.as_mut() {
            let source: &'b [u8] = self.sources[self.next_file - 1];
            let rest = &source[file.offset..];
            let read_len = self.profile.read_len;
            match Fgets::of(rest, read_len) {
                // `fgets` returns NULL at the end of the file.
                None => file.eof = true,
                Some(read) => {
                    file.offset += read.len;
                    // `fgets` met the end of the file before a newline or a full buffer.
                    if read.meets_end(read_len) {
                        file.eof = true;
                    }
                    if read.newline {
                        self.current_line += 1;
                    }
                    // jq 1.7.1 measures a newline-less read with `strlen` and loses it from its
                    // first NUL; under E2 the NUL reaches the parser, which refuses it (#7966).
                    chunk = &rest[..read.len];
                }
            }
        }
        let is_last =
            self.next_file == self.sources.len() && self.input.map_or(true, |file| file.eof);
        (chunk, is_last)
    }

    /// `jq_util_input_next_input`'s loop, from the reader's sync point to the next one.
    fn run(&mut self, out: &mut Vec<Read>) -> Stop {
        if !self.mode.seq && self.parser.malformed_bom() {
            // The bytes after the broken mark are still in jq's buffer, so it reads nothing more.
            let fault = self.bom_fault();
            self.emit(Item::Fault(fault), out);
            return Stop::Stuck;
        }
        // `is_last` is local to one call: a call that returns starts the next one afresh.
        let mut is_last = false;
        let mut reads = 0usize;
        loop {
            if self.parser.remaining() == 0 {
                if reads > 0 && self.parser.sync().is_some() {
                    return Stop::Synced;
                }
                let (chunk, last) = self.read_more();
                is_last = last;
                reads += 1;
                self.parser.set_buf(chunk, !is_last);
            }
            let next = self.parser.next();
            // jq 1.7.1 reads on past an empty record only while slurping (`has_more`); the profile
            // reads every record under `--seq`, as jq 1.8.2 does (#7718).
            let has_more = self.mode.seq
                && self.profile.seq_reads_every_record
                && self.parser.remaining() != 0;
            match next {
                Next::Value(span) => {
                    self.emit(Item::Value(span), out);
                    is_last = false;
                }
                Next::Fault(fault) => {
                    self.emit(Item::Fault(fault), out);
                    is_last = false;
                    if fault.place == FaultPlace::Bom {
                        return Stop::Stuck;
                    }
                }
                Next::None => {
                    if is_last && !has_more {
                        self.emit(Item::End, out);
                        is_last = false;
                    }
                }
            }
        }
    }

    fn emit(&self, item: Item, out: &mut Vec<Read>) {
        out.push(Read {
            item,
            at: self.position(),
        });
    }

    /// The `Malformed BOM` a stuck parser answers every call with.
    fn bom_fault(&mut self) -> Fault {
        // A parser rebuilt in the stuck state has no buffer; any will do, as it consumes nothing.
        self.parser.set_buf(&[], true);
        match self.parser.next() {
            Next::Fault(fault) => fault,
            _ => unreachable!("a malformed mark is a fault outside --seq"),
        }
    }
}

// `position` is read by no test either: the routes read it once they enter the model (step 5).
#[allow(dead_code)]
impl ReaderState {
    /// The reader before its first read.
    pub(crate) const fn start(mode: ReadMode) -> Self {
        Self {
            next_file: 0,
            input: None,
            current_line: 0,
            parser: ParserSync::start(mode.seq),
        }
    }

    /// What `input_filename` and `input_line_number` answer here.
    pub(crate) fn position(&self) -> Position {
        Position {
            file: self.input.map(|_| self.next_file - 1),
            line: self.current_line,
        }
    }

    /// Runs jq's reader over `sources` (every input file, in command-line order) from this sync
    /// point to the next one, at least one read on, and appends what each `input` call returned on
    /// the way. Outside `--seq` a `Malformed BOM` stops it on the fault ([`Stop::Stuck`]), and a
    /// call from that state answers the fault again without reading.
    pub(crate) fn step(
        &mut self,
        sources: &[&[u8]],
        profile: &ReaderProfile,
        mode: ReadMode,
        out: &mut Vec<Read>,
    ) -> Stop {
        let mut reader = Reader::resume(*self, sources, profile, mode);
        let stop = reader.run(out);
        *self = reader.state();
        stop
    }

    /// The state [`step`](Self::step) reaches over `stretch`, from its facts alone: the sync point
    /// right after the read that takes the stretch's last byte, or, when the stretch reaches the
    /// end of the last file, the state after the call that reports the end of input. `None` when
    /// the closed form does not apply: under `--seq` (step 8), after the end of input, with the mark
    /// not decided whole, or for runs that break the rules [`PlainStretch`] lists.
    pub(crate) fn skip_plain(
        &self,
        stretch: &PlainStretch<'_>,
        profile: &ReaderProfile,
        mode: ReadMode,
    ) -> Option<Self> {
        let mut state = *self;
        if mode.seq
            || state.parser.eof
            || state.parser.waiting_for_rs
            || !matches!(state.parser.bom, 0 | BOM_DONE)
        {
            return None;
        }
        let reads = LineReads::of(profile);
        let (last, _) = stretch.runs.split_last()?;
        for (i, run) in stretch.runs.iter().enumerate() {
            let consistent = run.tail <= run.len
                && (run.newlines > 0 || run.tail == run.len)
                && (!run.bom || run.len >= 3)
                && run.file < stretch.files
                && (run.to_end || i + 1 == stretch.runs.len());
            if !consistent {
                return None;
            }
            let reading = state.input.filter(|file| !file.eof);
            let current = reading.is_some() && run.file + 1 == state.next_file;
            if current && i > 0 {
                return None; // a file is listed once
            }
            if !current {
                // Only the file the reader stands in is continued; an open one it has not read to
                // its end has to be the first run.
                if run.file != state.next_file || (i == 0 && reading.is_some()) {
                    return None;
                }
                // `read_more` closes the file before (after the empty read that met its end, when
                // its last read ended on a newline or a full buffer) and opens this one.
                state.next_file += 1;
                state.input = Some(OpenFile {
                    offset: 0,
                    eof: false,
                });
                state.current_line = 0;
            }
            let file = state.input.as_mut()?;
            if run.len == 0 {
                if !current {
                    // The read that opened it found nothing.
                    file.eof = true;
                }
                continue;
            }
            let mut counted = run.len;
            if state.parser.bom == 0 {
                // The first bytes the parser is given decide the mark: a whole one is skipped and
                // not counted in the column.
                if run.bom {
                    counted -= 3;
                }
                state.parser.bom = BOM_DONE;
            }
            file.offset += run.len;
            state.current_line += run.newlines;
            if run.newlines > 0 {
                state.parser.line += run.newlines;
                state.parser.column = run.tail;
            } else {
                state.parser.column += counted;
            }
            if run.to_end {
                // The last read is short, and meets the end, unless it ended on a newline or was a
                // whole read.
                file.eof = reads.in_progress(run.tail) != 0;
            } else if reads.in_progress(run.tail) != 0 {
                return None; // not at the end of a read
            }
        }
        if last.to_end && last.file + 1 == stretch.files {
            // The end of the input: what is left is the reads that report it.
            let file = state.input.as_mut()?;
            // A last read that ended on a newline or a whole buffer left the file open: one more
            // read finds nothing and meets its end.
            file.eof = true;
            // The call that takes the last buffer reports the end, unless a value completes in
            // it: then the end is the next call's, whose read closes the file.
            if stretch.value_in_final_buffer {
                state.input = None;
                state.current_line = 0;
            }
            state.parser.eof = true;
        }
        Some(state)
    }
}

#[cfg(test)]
mod tests;
