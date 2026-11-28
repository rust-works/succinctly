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
//! it.
//!
//! Since step 5 the materializing route of `read_inputs` enters the model on its triggers
//! (`jq_runner/model_read.rs`): it carries a [`ReaderState`] from the stream's start over the plain
//! stretch before the trigger with [`ReaderState::skip_plain`], then reads the rest of the input
//! with [`ReaderState::read_to_end`]. Since step 6 the default route enters it on its triggers too
//! and hands back to the semi-index at the next sync point, carrying the [`ReaderState`] there
//! (`jq_runner/model_default.rs`). [`enabled`] is the switch that keeps the deciders the model
//! replaces selectable until step 10.

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
pub(crate) struct ReadMode {
    /// `--seq` (`JV_PARSE_SEQ`).
    pub seq: bool,
}

/// `current_input`: the file being read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
pub(crate) struct Position {
    /// The file, by its index among the inputs; `None` is `<unknown>` (`null`).
    pub file: Option<usize>,
    /// `current_line`.
    pub line: usize,
}

/// What one `jq_util_input_next_input` call returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
pub(crate) struct Read {
    pub item: Item,
    pub at: Position,
}

/// Where [`ReaderState::step`] stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stop {
    /// At the next sync point.
    Synced,
    /// On `Malformed BOM` outside `--seq`: the parser never leaves that state and consumes
    /// nothing in it, so every later call is the same fault, read with no new read.
    Stuck,
    /// Before a read whose bytes the [`Source`] does not hold ([`Session::step`]); never over
    /// [`Whole`] sources.
    Short,
}

/// What a route knows about one file's share of a plain stretch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// What a [`Session`] reads its input through: every input file whole ([`Whole`]), or the part of
/// them a route holds, which may be a window of one file (#7987, step 7). The session asks for one
/// `fgets` read at a time, so a source holds a read's worth of bytes past where the reader stands,
/// not the input.
pub(crate) trait Source {
    /// How many input files the run has.
    fn files(&self) -> usize;
    /// The offset in the stream the spans and faults are offsets of at which file `file` starts. A
    /// source that holds one file's bytes may say 0 for it: a session started in that file names
    /// the file's own offsets.
    fn start_of(&self, file: usize) -> usize;
    /// The bytes of file `file` the source holds from `offset` on: at least `want` of them where
    /// the file has that many and the source can hold them, and whether they run to the file's end.
    fn held(&mut self, file: usize, offset: usize, want: usize) -> Held<'_>;
}

/// What a [`Source`] holds of a file from where the reader stands.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Held<'a> {
    pub bytes: &'a [u8],
    /// The bytes run to the end of the file.
    pub to_end: bool,
}

/// Every input file, whole, in command-line order.
pub(crate) struct Whole<'s, 'b> {
    sources: &'s [&'b [u8]],
    starts: Vec<usize>,
}

impl<'s, 'b> Whole<'s, 'b> {
    pub(crate) fn new(sources: &'s [&'b [u8]]) -> Self {
        let starts = sources
            .iter()
            .scan(0usize, |at, source| {
                let start = *at;
                *at += source.len();
                Some(start)
            })
            .collect();
        Self { sources, starts }
    }
}

impl Source for Whole<'_, '_> {
    fn files(&self) -> usize {
        self.sources.len()
    }

    fn start_of(&self, file: usize) -> usize {
        match self.starts.get(file) {
            Some(&start) => start,
            None => self.sources.iter().map(|source| source.len()).sum(),
        }
    }

    fn held(&mut self, file: usize, offset: usize, _want: usize) -> Held<'_> {
        Held {
            bytes: &self.sources[file][offset..],
            to_end: true,
        }
    }
}

/// `jq_util_input_state` and its parser, from a sync point on (ADR-0033). A [`Session`] reads from
/// a [`Source`] one `fgets` read at a time and keeps its place between reads, a value in progress
/// included, so the bytes of each read can come from a different window: a route that holds only
/// some of its input runs the model over what it holds, and goes on with it when it holds more.
pub(crate) struct Session {
    profile: ReaderProfile,
    mode: ReadMode,
    next_file: usize,
    input: Option<OpenFile>,
    current_line: usize,
    /// Between reads it holds no buffer ([`JvParser::rebind`]), so it outlives each one.
    parser: Option<JvParser<'static>>,
    /// The reads since the session last stopped at a sync point.
    reads: usize,
}

/// The read a [`Source`] did not hold the bytes of: the session is where it was before it.
struct Short;

impl Session {
    /// The reader of `state`, reading `source`.
    pub(crate) fn resume(
        state: ReaderState,
        source: &impl Source,
        profile: &ReaderProfile,
        mode: ReadMode,
    ) -> Self {
        let offset = match state.input {
            Some(file) => source.start_of(state.next_file - 1) + file.offset,
            None => source.start_of(state.next_file),
        };
        Self {
            profile: *profile,
            mode,
            next_file: state.next_file,
            input: state.input,
            current_line: state.current_line,
            parser: Some(JvParser::resume(
                state.parser,
                mode.seq,
                profile.max_depth,
                offset,
            )),
            reads: 0,
        }
    }

    /// The reader as [`ReaderState`], at a sync point; `None` in the middle of a value, which a
    /// [`Stop::Short`] can leave it in.
    pub(crate) fn state(&self) -> Option<ReaderState> {
        Some(ReaderState {
            next_file: self.next_file,
            input: self.input,
            current_line: self.current_line,
            parser: self.parser.as_ref()?.sync()?,
        })
    }

    /// What `input_filename` and `input_line_number` answer here.
    pub(crate) fn position(&self) -> Position {
        Position {
            file: self.input.map(|_| self.next_file - 1),
            line: self.current_line,
        }
    }

    /// Where the reader stands: the open file and how far it has read it, or the next file's front.
    pub(crate) fn stands(&self) -> (usize, usize) {
        match self.input {
            Some(file) => (self.next_file - 1, file.offset),
            None => (self.next_file, 0),
        }
    }

    /// `jq_util_input_read_more`: the next buffer, and whether it is the last (`is_last`); `Short`
    /// when `source` does not hold the bytes `fgets` would return.
    fn read_more<'x>(&mut self, source: &'x mut impl Source) -> Result<(&'x [u8], bool), Short> {
        let files = source.files();
        // `map_or(true, ..)`, not `is_none_or`: clippy's `msrv` is 1.73 (clippy.toml).
        if self.input.map_or(true, |file| file.eof) {
            if self.input.take().is_some() {
                // `fclose`; jq 1.7.1 clears `current_filename` and `current_line` here.
                self.current_line = 0;
            }
            if self.next_file < files {
                self.next_file += 1;
                self.input = Some(OpenFile {
                    offset: 0,
                    eof: false,
                });
                self.current_line = 0;
            }
        }
        let mut chunk: &'x [u8] = &[];
        if let Some(file) = self.input.as_mut() {
            let read_len = self.profile.read_len;
            let held = source.held(self.next_file - 1, file.offset, read_len);
            match Fgets::of(held.bytes, read_len) {
                // `fgets` returns NULL at the end of the file.
                None if held.to_end => file.eof = true,
                None => return Err(Short),
                // A read the bytes held end before: not a newline, a full buffer or the file's end.
                Some(read) if !(read.newline || read.fills(read_len) || held.to_end) => {
                    return Err(Short)
                }
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
                    chunk = &held.bytes[..read.len];
                }
            }
        }
        let is_last = self.next_file == files && self.input.map_or(true, |file| file.eof);
        Ok((chunk, is_last))
    }

    /// `jq_util_input_next_input`'s loop (parser mode, without `--slurp`), from where the session
    /// stands to the next sync point, at least one read on: what each `input` call returned on the
    /// way is appended to `out`, with the position it was returned at. Outside `--seq` a `Malformed
    /// BOM` stops it on the fault ([`Stop::Stuck`]), and a call from that state answers the fault
    /// again without reading. [`Stop::Short`] is a read `source` does not hold the bytes of: the
    /// session stays where it was before that read, and a later call with more bytes goes on.
    pub(crate) fn step(&mut self, source: &mut impl Source, out: &mut Vec<Read>) -> Stop {
        let mut parser = self.parser.take().expect("a session holds its parser");
        if !self.mode.seq && parser.malformed_bom() {
            // The bytes after the broken mark are still in jq's buffer, so it reads nothing more.
            // A parser rebuilt in the stuck state has no buffer; any will do, as it consumes nothing.
            let mut stuck: JvParser<'_> = parser.rebind();
            stuck.set_buf(&[], true);
            let fault = match stuck.next() {
                Next::Fault(fault) => fault,
                _ => unreachable!("a malformed mark is a fault outside --seq"),
            };
            self.parser = Some(stuck.rebind());
            self.emit(Item::Fault(fault), out);
            return Stop::Stuck;
        }
        loop {
            // Between reads, the parser's buffer is read through.
            if self.reads > 0 && parser.sync().is_some() {
                self.reads = 0;
                self.parser = Some(parser);
                return Stop::Synced;
            }
            let Ok((chunk, last)) = self.read_more(source) else {
                self.parser = Some(parser);
                return Stop::Short;
            };
            // `is_last` is local to one call: a call that returns starts the next one afresh.
            let mut is_last = last;
            self.reads += 1;
            let mut reading: JvParser<'_> = parser.rebind();
            reading.set_buf(chunk, !is_last);
            loop {
                let next = reading.next();
                // jq 1.7.1 reads on past an empty record only while slurping (`has_more`); the
                // profile reads every record under `--seq`, as jq 1.8.2 does (#7718).
                let has_more = self.mode.seq
                    && self.profile.seq_reads_every_record
                    && reading.remaining() != 0;
                match next {
                    Next::Value(span) => {
                        self.emit(Item::Value(span), out);
                        is_last = false;
                    }
                    Next::Fault(fault) => {
                        self.emit(Item::Fault(fault), out);
                        is_last = false;
                        if fault.place == FaultPlace::Bom {
                            self.parser = Some(reading.rebind());
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
                if reading.remaining() == 0 {
                    break;
                }
            }
            parser = reading.rebind();
        }
    }

    fn emit(&self, item: Item, out: &mut Vec<Read>) {
        out.push(Read {
            item,
            at: self.position(),
        });
    }
}

/// Whether the routes enter the model on their triggers. `SUCCINCTLY_READER_MODEL=0` turns it off,
/// so a run reads with the deciders the model replaces: ADR-0033's two readers coexist until step
/// 10, and the switch is how the old one is selected for an A/B or a bisection. On by default.
pub(crate) fn enabled() -> bool {
    #[cfg(test)]
    if let Some(on) = MODEL_ON.with(std::cell::Cell::get) {
        return on;
    }
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var_os("SUCCINCTLY_READER_MODEL").map_or(true, |value| value != "0")
    })
}

#[cfg(test)]
thread_local! {
    /// Test-only: [`enabled`]'s answer on this thread, in place of the environment's.
    pub(crate) static MODEL_ON: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

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

    /// What `input_filename` and `input_line_number` answer here. Read by no route yet: the
    /// materializing route takes each answer's position from the answer (step 5).
    #[allow(dead_code)]
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
        let mut whole = Whole::new(sources);
        let mut session = Session::resume(*self, &whole, profile, mode);
        let stop = session.step(&mut whole, out);
        *self = session
            .state()
            .expect("over whole sources the reader stops at a sync point");
        stop
    }

    /// The reader at the byte after a newline of file `file`'s own bytes that no top-level value
    /// holds (#7987, step 7): `offset` is where that byte is in the file, as read, `file_lines` the
    /// newlines of the file before it and `stream_lines` those of every file before it, its own
    /// included. A newline ends a `fgets` read and decides the mark, so the point is a sync point,
    /// and the state there is a closed form of those counts: what [`step`](Self::step) reaches over
    /// any input, plain or not, that has them (`tests.rs` holds the two to each other). It is how a
    /// route that holds a window of its input, and so not the bytes before it, enters the model at
    /// the last sync point before a trigger: the window's line and offset counters are the facts.
    /// Outside `--seq` only, whose parser waits for a record separator (step 8).
    pub(crate) const fn at_line_start(
        file: usize,
        offset: usize,
        file_lines: usize,
        stream_lines: usize,
    ) -> Self {
        Self {
            next_file: file + 1,
            input: Some(OpenFile { offset, eof: false }),
            current_line: file_lines,
            parser: ParserSync {
                line: stream_lines + 1,
                column: 0,
                bom: BOM_DONE,
                ..ParserSync::start(false)
            },
        }
    }

    /// [`step`](Self::step) from this sync point on until the input is over: each answer goes to
    /// `on_read` as it is read, the first [`Item::End`] last. Outside `--seq` a `Malformed BOM`
    /// ends it instead, as the last answer, and it returns `true`: every `input` after it is that
    /// fault again ([`Stop::Stuck`]). Each step's answers are handed on and dropped before the next
    /// step, so what it holds is one read's worth, not the input's.
    // `reads` is drained, not consumed: the buffer is the next step's.
    #[allow(clippy::iter_with_drain)]
    pub(crate) fn read_to_end(
        self,
        sources: &[&[u8]],
        profile: &ReaderProfile,
        mode: ReadMode,
        mut on_read: impl FnMut(Read),
    ) -> bool {
        let mut whole = Whole::new(sources);
        let mut session = Session::resume(self, &whole, profile, mode);
        let mut reads = Vec::new();
        loop {
            let stop = session.step(&mut whole, &mut reads);
            for read in reads.drain(..) {
                let end = read.item == Item::End;
                on_read(read);
                if end {
                    return false;
                }
            }
            if stop == Stop::Stuck {
                return true;
            }
        }
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
