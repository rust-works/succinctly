//! Real jq's `--stream-errors` reader (#5474).
//!
//! `--stream-errors` implies `--stream` and changes one thing about it: a parse error is not
//! raised, it becomes the next input, an array `[message, path]` whose `path` is where the
//! streaming parser stood (`null` for an object key it has not read yet). The reader goes on
//! with the next line, since jq throws away only the rest of the line the error was in.
//!
//! That cannot be had from the whole-document parse `--stream` builds its events from: the
//! events a malformed document produced before the fault, and the path at the fault, are
//! properties of jq's own incremental parser. So this is a port of the streaming half of jq
//! 1.7.1's `jv_parse.c` (`stream_token`, `stream_check_done`, `scan`, `check_literal`,
//! `found_string` and the EOF branch of `jv_parser_next`), checked against the pinned
//! `/usr/bin/jq` rather than written from the documentation. It reuses
//! [`crate::jq_seq_reader`]'s port of the tokenizer pieces both modes share: the `fgets` chunking,
//! the number grammar, `\uXXXX`.
//!
//! What it keeps from jq on purpose:
//!
//! - **The input is read a line at a time.** jq feeds its parser one `fgets` chunk (a line, or
//!   4095 bytes) per call and, on an error, drops what is left of the chunk
//!   (`p->curr_buf = 0`). Parser state, including the line and column counters, carries over
//!   from one chunk and one file to the next, but the dropped bytes are never counted.
//! - **A value that completes on the byte that then fails is lost.** `1}` reports
//!   `Unmatched '}' at the top-level` without the `[[],1]` event `1` had just produced, because
//!   `scan` returns the error and not the value.
//! - **There is no depth limit.** `parse_token` checks `MAX_PARSING_DEPTH`, `stream_token`
//!   does not.
//! - **A token that ends a file continues in the next one.** The parser is never reset between
//!   files.
//!
//! The scalars and strings are materialized by the checked document parser from the token's
//! bytes, so a number keeps its spelling and a string its decoding exactly as a document's would;
//! this module decides only where each one begins and ends and which error fires.

use crate::jq_seq_reader::{fgets_chunk_len_at, number_is_valid, unhex4, JQ_FGETS_CHUNK};
use succinctly::jq::OwnedValue;

/// What jq's `p->last_seen` can hold.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LastSeen {
    None,
    OpenArray,
    OpenObject,
    Colon,
    Comma,
    Value,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum St {
    Normal,
    Str,
    StrEscape,
}

#[derive(PartialEq, Eq)]
enum Cls {
    Literal,
    Whitespace,
    Structure,
    Quote,
}

fn classify(c: u8) -> Cls {
    match c {
        b' ' | b'\t' | b'\r' | b'\n' => Cls::Whitespace,
        b'"' => Cls::Quote,
        b'[' | b',' | b']' | b'{' | b':' | b'}' => Cls::Structure,
        _ => Cls::Literal,
    }
}

/// The inputs `--stream-errors` hands the program, with where jq's reader stood for each.
pub(crate) struct StreamErrorsRead {
    pub(crate) values: Vec<OwnedValue>,
    /// Parallel to `values[locations_from..]`: the index into the source list and the number of
    /// lines jq had read from that source when it yielded the value, or [`CLOSED`] for a value jq
    /// yielded after it had closed its input (`input_filename` is `null` and `input_line_number`
    /// `0` there).
    pub(crate) locations: Vec<(usize, usize)>,
    /// Where `locations` starts in `values`: 0, except for [`read_for_slurp`], which keeps only the
    /// last few.
    pub(crate) locations_from: usize,
    /// The message and location of the first parse error jq turned into an input, if there was one:
    /// the error jq raises without `--stream-errors` (#5498).
    pub(crate) first_fault: Option<(String, (usize, usize))>,
    /// How many of `values` jq's `--slurp` loop puts in its array. The loop ends at the first call
    /// that consumes the last byte of the last read, so what that call leaves pending (the close
    /// of a container) and the EOF branch's report come out after the array, as inputs of their
    /// own.
    pub(crate) inside: usize,
    /// Where jq stood when it closed the array: a source index and a line count.
    pub(crate) slurp_at: (usize, usize),
    /// The line count of the last source if an `input` that runs past the end of the stream is the
    /// call that reaches the end of the file, so that jq's input is still open to name a line: it
    /// is when no value came out of the last refill. `None` when an earlier call already
    /// reached the end and jq has closed the file (`<unknown>`).
    pub(crate) open_at_end: Option<usize>,
    /// The index in `values` of the first parse error jq turned into an input, if there was one.
    #[cfg(test)]
    pub(crate) first_error: Option<usize>,
}

/// The line count of a value yielded once jq's input is closed.
pub(crate) const CLOSED: usize = usize::MAX;

/// One refill of jq's input buffer (`jq_util_input_read_more`), with the bytes of it left in the
/// buffer [`RefillSource::next_refill`] was given.
pub(crate) struct Refill {
    source: usize,
    /// Whether the chunk holds a newline, which `read_more` counts as a line.
    newline: bool,
    /// Whether this refill is the last one, `read_more`'s return value.
    is_last: bool,
}

/// Where jq's refills come from: the sources' bytes already read ([`MemorySources`]), or files and
/// stdin read a chunk at a time ([`ChunkedSources`], #5498).
pub(crate) trait RefillSource {
    /// The next refill, its bytes left in `buf` (which is cleared first), or `None` once the empty
    /// closing refill, if there is one, has been handed out.
    fn next_refill(&mut self, buf: &mut Vec<u8>) -> Option<Refill>;

    /// The first read error that ended a source early, if one did: a source read from a file or
    /// stdin can fail part-way, where one already read cannot.
    fn take_error(&mut self) -> Option<std::io::Error> {
        None
    }
}

impl RefillSource for Box<dyn RefillSource> {
    fn next_refill(&mut self, buf: &mut Vec<u8>) -> Option<Refill> {
        (**self).next_refill(buf)
    }

    fn take_error(&mut self) -> Option<std::io::Error> {
        (**self).take_error()
    }
}

/// The sources' bytes as read, a leading BOM already removed.
pub(crate) struct MemorySources<S: AsRef<[(Option<usize>, Vec<u8>)]>> {
    sources: S,
    cursor: RefillCursor,
}

impl<S: AsRef<[(Option<usize>, Vec<u8>)]>> MemorySources<S> {
    pub(crate) fn new(sources: S) -> Self {
        Self {
            sources,
            cursor: RefillCursor::default(),
        }
    }
}

impl<S: AsRef<[(Option<usize>, Vec<u8>)]>> RefillSource for MemorySources<S> {
    fn next_refill(&mut self, buf: &mut Vec<u8>) -> Option<Refill> {
        let refill = self.cursor.next(self.sources.as_ref())?;
        buf.clear();
        buf.extend_from_slice(refill.bytes);
        Some(Refill {
            source: refill.source,
            newline: refill.newline,
            is_last: refill.is_last,
        })
    }
}

/// Opens one source when the reader reaches it.
pub(crate) type Opener = Box<dyn FnOnce() -> std::io::Result<Box<dyn std::io::BufRead>>>;

/// jq's refills over files and stdin, read a chunk at a time (#5498): what is held is the source
/// being read, with its buffer, and one `fgets` chunk, 4095 bytes at most, not the input.
///
/// The refills are the ones [`MemorySources`] makes over the same bytes: a chunk runs through the
/// next newline or 4095 bytes, an empty source is a refill of its own, and the last chunk of the
/// last source is the last refill unless it filled the buffer or ended in a newline.
///
/// A source is opened when it is reached and dropped when it is read, so many files hold one
/// descriptor and one buffer. Whether a chunk that ended at a newline or at 4095 bytes was the end
/// of its source is found by the next call, which reads first, so a stream that stops after a
/// line (a pipe from a slow producer) has handed over that line without waiting for the next one.
pub(crate) struct ChunkedSources {
    openers: Vec<Option<Opener>>,
    current: Option<Box<dyn std::io::BufRead>>,
    source: usize,
    /// The chunk handed out last ended at a newline or filled the buffer, so it is not yet known
    /// whether it was the end of `current`.
    unsettled: bool,
    last: Option<(usize, bool)>,
    closed: bool,
    error: Option<std::io::Error>,
}

impl ChunkedSources {
    /// `openers` in input order, each giving the reader of one source, already past a leading BOM
    /// that is to be skipped.
    pub(crate) fn new(openers: Vec<Opener>) -> Self {
        Self {
            openers: openers.into_iter().map(Some).collect(),
            current: None,
            source: 0,
            unsettled: false,
            last: None,
            closed: false,
            error: None,
        }
    }

    fn fail(&mut self, error: std::io::Error) {
        self.error.get_or_insert(error);
    }
}

/// How [`read_chunk`] stopped.
#[derive(PartialEq, Eq)]
enum ChunkEnd {
    /// After a newline.
    Newline,
    /// At [`JQ_FGETS_CHUNK`] bytes.
    Full,
    /// The source has no more bytes, or failed.
    Eof,
}

/// One `fgets` chunk of `reader` appended to `buf`: through the next newline, or 4095 bytes. An
/// error ends the chunk where it fell, as the end of the source does, and is handed back.
fn read_chunk(
    reader: &mut dyn std::io::BufRead,
    buf: &mut Vec<u8>,
) -> (ChunkEnd, Option<std::io::Error>) {
    while buf.len() < JQ_FGETS_CHUNK {
        let available = match reader.fill_buf() {
            Ok(available) => available,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return (ChunkEnd::Eof, Some(e)),
        };
        if available.is_empty() {
            return (ChunkEnd::Eof, None);
        }
        let window = &available[..available.len().min(JQ_FGETS_CHUNK - buf.len())];
        match window.iter().position(|&byte| byte == b'\n') {
            Some(newline) => {
                buf.extend_from_slice(&window[..=newline]);
                reader.consume(newline + 1);
                return (ChunkEnd::Newline, None);
            }
            None => {
                buf.extend_from_slice(window);
                let taken = window.len();
                reader.consume(taken);
            }
        }
    }
    (ChunkEnd::Full, None)
}

impl RefillSource for ChunkedSources {
    fn next_refill(&mut self, buf: &mut Vec<u8>) -> Option<Refill> {
        let count = self.openers.len();
        while self.source < count {
            let source = self.source;
            let last_source = source + 1 == count;
            if self.current.is_none() {
                let opened = self.openers[source].take().map_or_else(
                    || {
                        Err(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            "a source opened twice",
                        ))
                    },
                    |open| open(),
                );
                self.current = Some(match opened {
                    Ok(reader) => reader,
                    Err(e) => {
                        // A source that opened when the files were checked and no longer does
                        // reads as empty, with the error kept.
                        self.fail(e);
                        Box::new(std::io::BufReader::new(std::io::empty()))
                    }
                });
            }
            let reader = self.current.as_mut().expect("a source is open");
            if self.unsettled {
                self.unsettled = false;
                // Whether the last chunk ended its source is found now, by the first read of the
                // next one.
                let ended = match reader.fill_buf() {
                    Ok(available) => available.is_empty(),
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => false,
                    Err(e) => {
                        self.fail(e);
                        true
                    }
                };
                if ended {
                    self.current = None;
                    self.source += 1;
                    continue;
                }
            }
            let reader = self.current.as_mut().expect("a source is open");
            buf.clear();
            let (end, error) = read_chunk(reader.as_mut(), buf);
            if let Some(e) = error {
                self.fail(e);
            }
            if buf.is_empty() {
                // A source with no bytes is a refill of its own.
                self.current = None;
                self.source += 1;
                self.last = Some((source, last_source));
                return Some(Refill {
                    source,
                    newline: false,
                    is_last: last_source,
                });
            }
            let newline = buf.ends_with(b"\n");
            let hit_eof = end == ChunkEnd::Eof && !newline;
            if end == ChunkEnd::Eof {
                self.current = None;
                self.source += 1;
            } else {
                self.unsettled = true;
            }
            if !newline {
                // The parser sees a chunk with no newline up to its first NUL (`strlen`).
                if let Some(nul) = buf.iter().position(|&byte| byte == 0) {
                    buf.truncate(nul);
                }
            }
            let is_last = last_source && hit_eof;
            self.last = Some((source, is_last));
            return Some(Refill {
                source,
                newline,
                is_last,
            });
        }
        if !self.closed {
            self.closed = true;
            if let Some((source, false)) = self.last {
                buf.clear();
                return Some(Refill {
                    source,
                    newline: false,
                    is_last: true,
                });
            }
        }
        None
    }

    fn take_error(&mut self) -> Option<std::io::Error> {
        self.error.take()
    }
}

/// Where the refills jq makes over a list of sources have got to, so that the next one can be
/// found without the whole list being built (the list is as long as the input has lines).
///
/// A file is read a line at a time (`fgets` into 4096 bytes), and the refill is the last one when
/// `fgets` hit the end of the last file: a chunk that ends in a newline, or fills the buffer, has
/// not hit it yet and is followed by a refill that returns nothing. The parser sees a chunk with no
/// newline up to its first NUL (`strlen`).
#[derive(Default)]
struct RefillCursor {
    source: usize,
    /// Where the next chunk starts in `source`.
    pos: usize,
    /// The source and `is_last` of the refill handed out last, which decides whether the
    /// empty closing refill follows.
    last: Option<(usize, bool)>,
    closed: bool,
}

/// A refill of the in-memory sources, with its bytes borrowed from them.
struct CursorRefill<'a> {
    source: usize,
    bytes: &'a [u8],
    newline: bool,
    is_last: bool,
}

impl RefillCursor {
    fn next<'a>(&mut self, sources: &'a [(Option<usize>, Vec<u8>)]) -> Option<CursorRefill<'a>> {
        while let Some((_, raw)) = sources.get(self.source) {
            let last_source = self.source + 1 == sources.len();
            let source = self.source;
            if raw.is_empty() {
                self.source += 1;
                let is_last = last_source;
                self.last = Some((source, is_last));
                return Some(CursorRefill {
                    source,
                    bytes: &[],
                    newline: false,
                    is_last,
                });
            }
            let Some(len) = fgets_chunk_len_at(raw, self.pos) else {
                self.source += 1;
                self.pos = 0;
                continue;
            };
            let chunk = &raw[self.pos..self.pos + len];
            self.pos += len;
            let at_end = self.pos == raw.len();
            if at_end {
                self.source += 1;
                self.pos = 0;
            }
            let newline = chunk.ends_with(b"\n");
            let hit_eof = !newline && len < JQ_FGETS_CHUNK;
            let bytes = if newline {
                chunk
            } else {
                &chunk[..chunk.iter().position(|&b| b == 0).unwrap_or(len)]
            };
            let is_last = last_source && at_end && hit_eof;
            self.last = Some((source, is_last));
            return Some(CursorRefill {
                source,
                bytes,
                newline,
                is_last,
            });
        }
        if !self.closed {
            self.closed = true;
            if let Some((source, false)) = self.last {
                return Some(CursorRefill {
                    source,
                    bytes: &[],
                    newline: false,
                    is_last: true,
                });
            }
        }
        None
    }
}

/// One input jq's parser yielded.
pub(crate) struct Yielded {
    pub(crate) value: OwnedValue,
    /// The source index and the lines jq had read of it ([`CLOSED`] after jq closed its input).
    pub(crate) location: (usize, usize),
    /// The message, when the input is a parse error: `[message, path]` is the input under
    /// `--stream-errors`, and the message is the error jq raises without it.
    pub(crate) error: Option<String>,
}

/// How jq's reader ended, known once [`Reader::next`] has returned `None`.
pub(crate) struct ReadEnd {
    /// How many of the values jq's `--slurp` loop puts in its array: see
    /// [`StreamErrorsRead::inside`].
    pub(crate) inside: usize,
    /// Where jq stood when it closed the array: see [`StreamErrorsRead::slurp_at`].
    pub(crate) slurp_at: (usize, usize),
    /// See [`StreamErrorsRead::open_at_end`].
    pub(crate) open_at_end: Option<usize>,
    /// The index among the values of the first parse error jq turned into an input, if any.
    #[cfg(test)]
    pub(crate) first_error: Option<usize>,
}

/// jq's reader over `R`, where its refills come from, handing out the inputs one at a time as the
/// refills that complete them are fed to the parser: the parser's state and the refill being
/// read are all it holds, not the values it has yielded.
pub(crate) struct Reader<R: RefillSource> {
    source: R,
    /// The bytes of the refill being fed to the parser.
    buf: Vec<u8>,
    parser: Parser,
    /// What the last byte of a refill left for the next call to hand out.
    deferred: Vec<OwnedValue>,
    inside: Option<usize>,
    slurp_at: (usize, usize),
    finished: bool,
    values_before_last_refill: usize,
    done: bool,
}

impl<S: AsRef<[(Option<usize>, Vec<u8>)]>> Reader<MemorySources<S>> {
    /// The reader over sources already read, a leading BOM removed.
    pub(crate) fn new(sources: S) -> Self {
        Self::from_source(MemorySources::new(sources))
    }
}

impl<R: RefillSource> Reader<R> {
    pub(crate) fn from_source(source: R) -> Self {
        Self {
            source,
            buf: Vec::new(),
            parser: Parser::new(),
            deferred: Vec::new(),
            inside: None,
            slurp_at: (0, 0),
            finished: false,
            values_before_last_refill: 0,
            done: false,
        }
    }

    /// The next input and the `(source index, lines read)` jq's reader stood at when it yielded
    /// it ([`CLOSED`] for the lines of one yielded after jq closed its input), or `None` once
    /// jq's reader has nothing left.
    #[cfg(test)]
    pub(crate) fn next(&mut self) -> Option<(OwnedValue, (usize, usize))> {
        self.next_yielded().map(|y| (y.value, y.location))
    }

    /// [`next`](Self::next), with whether the input is a parse error.
    pub(crate) fn next_yielded(&mut self) -> Option<Yielded> {
        loop {
            if let Some(item) = self.parser.pending.pop_front() {
                return Some(item);
            }
            if self.done {
                return None;
            }
            self.step();
        }
    }

    /// One refill of jq's input buffer, or what follows the last one.
    fn step(&mut self) {
        let Some(refill) = self.source.next_refill(&mut self.buf) else {
            // Whatever is left comes from calls made after jq has closed its input.
            self.parser.closed = true;
            for value in std::mem::take(&mut self.deferred) {
                self.parser.yield_value(value);
            }
            if !self.finished {
                self.parser.finish();
            }
            self.done = true;
            return;
        };
        let parser = &mut self.parser;
        if refill.is_last {
            self.values_before_last_refill = parser.yielded;
        }
        if parser.source != refill.source {
            parser.source = refill.source;
            parser.lines_read = 0;
        }
        if refill.newline {
            parser.lines_read += 1;
        }
        // The call that follows a refill starts with `stream_check_done`.
        let drained = !self.deferred.is_empty();
        for value in std::mem::take(&mut self.deferred) {
            parser.yield_value(value);
        }
        let ended_with_value = parser.feed(&self.buf, &mut self.deferred);
        if refill.is_last {
            // The slurp loop ends with the first call that leaves nothing in the buffer.
            let first_call_empties = if self.buf.is_empty() {
                drained
            } else {
                ended_with_value
            };
            if !first_call_empties {
                parser.finish();
                self.finished = true;
            }
            self.inside = Some(parser.yielded);
            self.slurp_at = (parser.source, parser.lines_read);
        }
    }

    /// The read error that ended a source early, if one did (see [`RefillSource::take_error`]).
    pub(crate) fn take_error(&mut self) -> Option<std::io::Error> {
        self.source.take_error()
    }

    /// How the read ended. Meaningful once [`next`](Self::next) has returned `None`.
    pub(crate) fn end(&self) -> ReadEnd {
        ReadEnd {
            inside: self.inside.unwrap_or(self.parser.yielded),
            slurp_at: self.slurp_at,
            open_at_end: (self.parser.yielded == self.values_before_last_refill)
                .then_some(self.parser.lines_read),
            #[cfg(test)]
            first_error: self.parser.first_error,
        }
    }
}

/// Read `sources` (each source's bytes as read, a leading BOM already removed) the way
/// `jq --stream-errors` does.
pub(crate) fn read(sources: &[(Option<usize>, Vec<u8>)]) -> StreamErrorsRead {
    read_keeping(sources, usize::MAX, false)
}

/// How many trailing locations [`read_for_slurp`] keeps: the values jq's `--slurp` loop leaves
/// after its array are the close it still held and the EOF report.
const SLURP_TAIL: usize = 8;

/// [`read`] for `--slurp`, which puts every value in one array and so needs the location of only the
/// values after it (when `locations_from` is past `inside`, they were not all kept: see
/// [`StreamErrorsRead::locations_from`] and read again with [`read`]) and of the first parse error
/// (#5498). A location is 16 bytes a value, and a stream of events has one per event.
///
/// With `stop_at_fault` the read ends at the first parse error, whose message is
/// [`StreamErrorsRead::first_fault`]: `--slurp` without `--stream-errors` raises it and prints
/// nothing else, so what follows is never read.
pub(crate) fn read_for_slurp(
    sources: &[(Option<usize>, Vec<u8>)],
    stop_at_fault: bool,
) -> StreamErrorsRead {
    read_keeping(sources, SLURP_TAIL, stop_at_fault)
}

/// [`read`], keeping the location of the last `keep` values only, and stopping at the first parse
/// error if asked to.
fn read_keeping(
    sources: &[(Option<usize>, Vec<u8>)],
    keep: usize,
    stop_at_fault: bool,
) -> StreamErrorsRead {
    let mut reader = Reader::new(sources);
    let mut values = Vec::new();
    let mut locations = std::collections::VecDeque::new();
    let mut first_fault = None;
    while let Some(yielded) = reader.next_yielded() {
        if let (Some(message), None) = (yielded.error, &first_fault) {
            first_fault = Some((message, yielded.location));
            if stop_at_fault {
                values.push(yielded.value);
                break;
            }
        }
        values.push(yielded.value);
        if locations.len() == keep {
            locations.pop_front();
        }
        locations.push_back(yielded.location);
    }
    let end = reader.end();
    StreamErrorsRead {
        inside: end.inside,
        slurp_at: end.slurp_at,
        open_at_end: end.open_at_end,
        #[cfg(test)]
        first_error: end.first_error,
        locations_from: values.len() - locations.len(),
        first_fault,
        values,
        locations: locations.into(),
    }
}

/// What `-n -s --stream` hands the program, in the order jq's `input` returns it (#6005).
///
/// Under `-n` jq reads on the first `input`, and a fault ends the call that meets it: the next call
/// goes on with the rest of the stream. So every fault the slurp loop meets before it closes its
/// array comes out first, one per call, then the array, and what the loop left for the calls after
/// it (the close it still held, a fault at the end of the stream) comes out in order after it.
pub(crate) struct DeferredSlurp {
    /// The events the array holds.
    pub(crate) array: Vec<OwnedValue>,
    /// Where jq stood when it closed the array: a source index and a line count.
    pub(crate) slurp_at: (usize, usize),
    /// The faults raised before the array, in order, each with where jq stood.
    pub(crate) before: Vec<(String, (usize, usize))>,
    /// The inputs after the array, in order.
    pub(crate) after: Vec<Afterwards>,
}

/// One input of [`DeferredSlurp::after`].
pub(crate) enum Afterwards {
    Value(OwnedValue, (usize, usize)),
    Fault(String, (usize, usize)),
}

/// [`read_for_slurp`] for `-n -s --stream`: the whole stream, with every fault set apart from the
/// events instead of stopping the read at the first one.
pub(crate) fn read_for_deferred_slurp(sources: &[(Option<usize>, Vec<u8>)]) -> DeferredSlurp {
    let mut reader = Reader::new(sources);
    let mut yielded = Vec::new();
    while let Some(item) = reader.next_yielded() {
        yielded.push(item);
    }
    let end = reader.end();
    let mut read = DeferredSlurp {
        array: Vec::new(),
        slurp_at: end.slurp_at,
        before: Vec::new(),
        after: Vec::new(),
    };
    for (index, item) in yielded.into_iter().enumerate() {
        match (index < end.inside, item.error) {
            (true, None) => read.array.push(item.value),
            (true, Some(message)) => read.before.push((message, item.location)),
            (false, None) => read
                .after
                .push(Afterwards::Value(item.value, item.location)),
            (false, Some(message)) => read.after.push(Afterwards::Fault(message, item.location)),
        }
    }
    read
}

/// A parse error as `--stream-errors` hands it over (`[message, path]`) and the message alone.
struct ParseFault {
    value: OwnedValue,
    message: String,
}

struct Parser {
    line: usize,
    column: usize,
    st: St,
    /// jq's `p->path`; its length is jq's `p->stacklen`. An element is a number (inside an array),
    /// a string (inside an object, once its key is known) or `null` (inside an object, key not yet
    /// read).
    path: Vec<OwnedValue>,
    last_seen: LastSeen,
    /// jq's `p->output`: zero to three elements. `[path]` closes a container, `[path, value]` is a
    /// leaf, and `[path, value, true]` is a leaf whose container closes right after it, which
    /// [`Parser::check_done`] hands out as the leaf and then `[path]`.
    output: Vec<OwnedValue>,
    next: Option<OwnedValue>,
    /// jq's `tokenbuf`/`tokenpos`, as in [`crate::jq_seq_reader`]: a buffer that is not cleared
    /// with the token, because `check_literal` reads `tokenbuf[1]` without checking `tokenpos`.
    token_buf: Vec<u8>,
    token_len: usize,
    source: usize,
    lines_read: usize,
    /// Set once jq has closed its input, for the calls made after the last refill.
    closed: bool,
    /// What has been yielded and not yet handed to the caller, with the `(source, lines read)`
    /// jq stood at.
    pending: std::collections::VecDeque<Yielded>,
    /// How many values the parser has yielded in all, handed out or not.
    yielded: usize,
    #[cfg(test)]
    first_error: Option<usize>,
}

impl Parser {
    fn new() -> Self {
        Self {
            line: 1,
            column: 0,
            st: St::Normal,
            path: Vec::new(),
            last_seen: LastSeen::None,
            output: Vec::new(),
            next: None,
            token_buf: Vec::new(),
            token_len: 0,
            source: 0,
            lines_read: 0,
            closed: false,
            pending: std::collections::VecDeque::new(),
            yielded: 0,
            #[cfg(test)]
            first_error: None,
        }
    }

    /// jq's `parser_reset` for a streaming parser.
    fn reset(&mut self) {
        self.path.clear();
        self.last_seen = LastSeen::None;
        self.output.clear();
        self.next = None;
        self.token_len = 0;
        self.st = St::Normal;
    }

    fn yield_value(&mut self, value: OwnedValue) {
        self.push(value, None);
    }

    fn push(&mut self, value: OwnedValue, error: Option<String>) {
        let line = if self.closed { CLOSED } else { self.lines_read };
        self.pending.push_back(Yielded {
            value,
            location: (self.source, line),
            error,
        });
        self.yielded += 1;
    }

    /// [`Parser::yield_value`] for a parse error, remembering where the first one fell.
    fn yield_error(&mut self, fault: ParseFault) {
        #[cfg(test)]
        self.first_error.get_or_insert(self.yielded);
        self.push(fault.value, Some(fault.message));
    }

    fn path_value(&self) -> OwnedValue {
        OwnedValue::array_from(self.path.clone())
    }

    /// jq's `make_error` with `JV_PARSE_STREAM_ERRORS`: the message and the path as it stands.
    fn error_value(&self, message: String) -> ParseFault {
        ParseFault {
            value: OwnedValue::array_from(vec![
                OwnedValue::String(message.clone().into()),
                self.path_value(),
            ]),
            message,
        }
    }

    /// One refill's bytes, which are one call to `jv_parser_next` or more: the values it completes
    /// are yielded as they are found, and an error ends the refill. What the refill's last byte
    /// leaves pending goes to `deferred`, since the call that hands it out comes after the next
    /// refill. Whether the last call yielded a value, which an error does wherever it fell, is
    /// the result.
    fn feed(&mut self, chunk: &[u8], deferred: &mut Vec<OwnedValue>) -> bool {
        for (index, &ch) in chunk.iter().enumerate() {
            let last = index + 1 == chunk.len();
            match self.scan(ch) {
                Ok(Some(value)) => {
                    self.yield_value(value);
                    // The next call starts with `stream_check_done`, which hands out whatever the
                    // byte left pending (the close of a container) before it scans another.
                    while let Some(value) = self.check_done() {
                        if last {
                            deferred.push(value);
                        } else {
                            self.yield_value(value);
                        }
                    }
                    if last {
                        return true;
                    }
                }
                Ok(None) => {}
                Err(message) => {
                    let error = self.error_value(format!(
                        "{message} at line {}, column {}",
                        self.line, self.column
                    ));
                    self.reset();
                    self.yield_error(error);
                    // `p->curr_buf = 0`: the rest of this refill is thrown away.
                    return true;
                }
            }
        }
        false
    }

    /// The EOF branch of `jv_parser_next`.
    fn finish(&mut self) {
        let (line, column) = (self.line, self.column);
        if self.st != St::Normal {
            let error = self.error_value(format!(
                "Unfinished string at EOF at line {line}, column {column}"
            ));
            self.yield_error(error);
            return;
        }
        if let Err(message) = self.check_literal() {
            let error =
                self.error_value(format!("{message} at EOF at line {line}, column {column}"));
            self.yield_error(error);
            return;
        }
        if !self.path.is_empty() {
            let error = self.error_value(format!(
                "Unfinished JSON term at EOF at line {line}, column {column}"
            ));
            self.yield_error(error);
            return;
        }
        if let Some(value) = self.next.take() {
            let event = OwnedValue::array_from(vec![self.path_value(), value]);
            self.yield_value(event);
        }
    }

    /// jq's `scan`, for a streaming parser that is not reading a `--seq`.
    fn scan(&mut self, ch: u8) -> Result<Option<OwnedValue>, &'static str> {
        self.column += 1;
        if ch == b'\n' {
            self.line += 1;
            self.column = 0;
        }
        // A value found before the byte fails is dropped with the failure: jq returns the error
        // and not `*out`.
        let mut answer = None;
        if self.st == St::Normal {
            let cls = classify(ch);
            if cls != Cls::Literal {
                self.check_literal()?;
                if let Some(value) = self.check_done() {
                    answer = Some(value);
                }
            }
            match cls {
                Cls::Literal => self.token_add(ch),
                Cls::Whitespace => {}
                Cls::Quote => self.st = St::Str,
                Cls::Structure => self.stream_token(ch)?,
            }
            if let Some(value) = self.check_done() {
                answer = Some(value);
            }
        } else if ch == b'"' && self.st == St::Str {
            self.found_string()?;
            self.st = St::Normal;
            if let Some(value) = self.check_done() {
                answer = Some(value);
            }
        } else {
            self.token_add(ch);
            self.st = if ch == b'\\' && self.st == St::Str {
                St::StrEscape
            } else {
                St::Str
            };
        }
        Ok(answer)
    }

    /// jq's `tokenadd`, growth policy included.
    fn token_add(&mut self, ch: u8) {
        if self.token_len == self.token_buf.len() {
            self.token_buf.resize(self.token_buf.len() * 2 + 256, 0);
        }
        self.token_buf[self.token_len] = ch;
        self.token_len += 1;
    }

    /// jq's `stream_check_done`: the next value to hand out, if any. A top-level scalar goes out as
    /// `[[], scalar]`; otherwise the pending `output`, a leaf-and-close pair as the leaf first.
    fn check_done(&mut self) -> Option<OwnedValue> {
        if self.path.is_empty() {
            if let Some(value) = self.next.take() {
                return Some(OwnedValue::array_from(vec![self.path_value(), value]));
            }
        }
        if self.output.is_empty() {
            return None;
        }
        if self.output.len() > 2 {
            let leaf = OwnedValue::array_from(vec![self.output[0].clone(), self.output[1].clone()]);
            self.output.truncate(1);
            return Some(leaf);
        }
        Some(OwnedValue::array_from(std::mem::take(&mut self.output)))
    }

    /// jq's `value`, for a streaming parser.
    fn value(&mut self, value: OwnedValue) -> Result<(), &'static str> {
        if self.next.is_some() || self.last_seen == LastSeen::Value {
            return Err("Expected separator between values");
        }
        self.last_seen = if self.path.is_empty() {
            LastSeen::None
        } else {
            LastSeen::Value
        };
        self.next = Some(value);
        Ok(())
    }

    fn last_is_number(&self) -> bool {
        matches!(
            self.path.last(),
            Some(OwnedValue::Int(_) | OwnedValue::Float(_))
        )
    }

    fn last_is_string(&self) -> bool {
        matches!(self.path.last(), Some(OwnedValue::String(_)))
    }

    /// jq's `stream_token`.
    fn stream_token(&mut self, ch: u8) -> Result<(), &'static str> {
        match ch {
            b'[' => {
                if self.next.is_some() {
                    return Err("Expected a separator between values");
                }
                if self.last_seen == LastSeen::OpenObject {
                    return Err("Expected string key after '{', not '['");
                }
                if self.last_seen == LastSeen::Comma && !self.last_is_number() {
                    return Err("Expected string key after ',' in object, not '['");
                }
                self.path.push(OwnedValue::Int(0));
                self.last_seen = LastSeen::OpenArray;
            }
            b'{' => {
                if self.last_seen == LastSeen::Value {
                    return Err("Expected a separator between values");
                }
                if self.last_seen == LastSeen::OpenObject {
                    return Err("Expected string key after '{', not '{'");
                }
                if self.last_seen == LastSeen::Comma && !self.last_is_number() {
                    return Err("Expected string key after ',' in object, not '{'");
                }
                self.path.push(OwnedValue::Null);
                self.last_seen = LastSeen::OpenObject;
            }
            b':' => {
                if self.path.is_empty() || self.last_is_number() {
                    return Err("':' not as part of an object");
                }
                if self.next.is_none() || self.last_seen == LastSeen::None {
                    return Err("Expected string key before ':'");
                }
                if !matches!(self.next, Some(OwnedValue::String(_))) {
                    return Err("Object keys must be strings");
                }
                if self.last_seen != LastSeen::Value {
                    return Err("':' should follow a key");
                }
                self.last_seen = LastSeen::Colon;
                if let (Some(key), Some(slot)) = (self.next.take(), self.path.last_mut()) {
                    *slot = key;
                }
            }
            b',' => {
                if self.last_seen != LastSeen::Value {
                    return Err("Expected value before ','");
                }
                if self.path.is_empty() {
                    return Err("',' not as part of an object or array");
                }
                let after = match self.path.last() {
                    Some(OwnedValue::Int(index)) => OwnedValue::Int(index + 1),
                    Some(OwnedValue::String(_)) => OwnedValue::Null,
                    // this case hits on input like `{,}`, and on `{"a", "b"}` and `{"a":, ...}`
                    _ => return Err("Objects must consist of key:value pairs"),
                };
                if let Some(value) = self.next.take() {
                    self.output = vec![self.path_value(), value];
                }
                if let Some(slot) = self.path.last_mut() {
                    *slot = after;
                }
                self.last_seen = LastSeen::Comma;
            }
            b']' => {
                if self.path.is_empty() {
                    return Err("Unmatched ']' at the top-level");
                }
                if self.last_seen == LastSeen::Comma {
                    return Err("Expected another array element");
                }
                if !self.last_is_number() {
                    return Err("Unmatched ']' in the middle of an object");
                }
                if let Some(value) = self.next.take() {
                    self.output = vec![self.path_value(), value, OwnedValue::Bool(true)];
                } else if self.last_seen != LastSeen::OpenArray {
                    self.output = vec![self.path_value()];
                }
                self.path.pop();
                self.next = None;
                if self.last_seen == LastSeen::OpenArray {
                    // Empty arrays are leaves.
                    self.output = vec![self.path_value(), OwnedValue::array_from(Vec::new())];
                }
                self.last_seen = if self.path.is_empty() {
                    LastSeen::None
                } else {
                    LastSeen::Value
                };
            }
            b'}' => {
                if self.path.is_empty() {
                    return Err("Unmatched '}' at the top-level");
                }
                if self.last_seen == LastSeen::Comma {
                    return Err("Expected another key:value pair");
                }
                if self.last_is_number() {
                    return Err("Unmatched '}' in the middle of an array");
                }
                if let Some(value) = self.next.take() {
                    if !self.last_is_string() {
                        return Err("Objects must consist of key:value pairs");
                    }
                    self.output = vec![self.path_value(), value, OwnedValue::Bool(true)];
                } else {
                    // Perhaps `{"a":[]}`.
                    match self.last_seen {
                        // Looks like `{"a":}`.
                        LastSeen::Colon => return Err("Missing value in key:value pair"),
                        // Looks like `{"a":0,}`.
                        LastSeen::Comma => return Err("Expected another key-value pair"),
                        LastSeen::OpenArray => {
                            return Err("Unmatched '}' in the middle of an array")
                        }
                        LastSeen::Value | LastSeen::OpenObject => {}
                        LastSeen::None => return Err("Unmatched '}'"),
                    }
                    if self.last_seen != LastSeen::OpenObject {
                        self.output = vec![self.path_value()];
                    }
                }
                self.path.pop();
                self.next = None;
                if self.last_seen == LastSeen::OpenObject {
                    // Empty objects are leaves.
                    self.output = vec![self.path_value(), OwnedValue::object()];
                }
                self.last_seen = if self.path.is_empty() {
                    LastSeen::None
                } else {
                    LastSeen::Value
                };
            }
            // Only `classify`'s `Structure` bytes reach this function.
            _ => {}
        }
        Ok(())
    }

    /// jq's `check_literal`.
    fn check_literal(&mut self) -> Result<(), &'static str> {
        if self.token_len == 0 {
            return Ok(());
        }
        let pattern: Option<&[u8]> = match self.token_buf[0] {
            b't' => Some(b"true"),
            b'f' => Some(b"false"),
            // Deliberately the whole buffer, not just the token: see the field's docs.
            b'n' if self.token_buf.get(1) == Some(&b'u') => Some(b"null"),
            _ => None,
        };
        let value = match pattern {
            Some(pattern) => {
                if &self.token_buf[..self.token_len] != pattern {
                    return Err("Invalid literal");
                }
                match pattern[0] {
                    b't' => OwnedValue::Bool(true),
                    b'f' => OwnedValue::Bool(false),
                    _ => OwnedValue::Null,
                }
            }
            None => {
                // jq NUL-terminates the token in place before its number parser reads it, which
                // scrubs the stale byte read above.
                if self.token_len == self.token_buf.len() {
                    self.token_buf.resize(self.token_buf.len() * 2 + 256, 0);
                }
                self.token_buf[self.token_len] = 0;
                // ... which also means the number parser reads the token as a C string: a NUL in
                // it ends the number (`2.5` then NUL then `{}` is the number `2.5`).
                let token = &self.token_buf[..self.token_len];
                let token = &token[..token.iter().position(|&b| b == 0).unwrap_or(token.len())];
                if !number_is_valid(token) {
                    return Err("Invalid numeric literal");
                }
                fast_number(token)
            }
        };
        self.value(value)?;
        self.token_len = 0;
        Ok(())
    }

    /// jq's `found_string`.
    fn found_string(&mut self) -> Result<(), &'static str> {
        let buf = &self.token_buf[..self.token_len];
        let mut i = 0;
        while i < buf.len() {
            let c = buf[i];
            i += 1;
            if c != b'\\' {
                // jq compares a *signed* char, so only U+0000..=U+001F qualifies.
                if c <= 0x1f {
                    return Err(
                        "Invalid string: control characters from U+0000 through U+001F must be escaped",
                    );
                }
                continue;
            }
            // Unreachable: a trailing backslash leaves the scanner in `StrEscape`, so the next byte
            // is consumed as the escaped one. jq carries the arm anyway.
            if i >= buf.len() {
                return Err("Expected escape character at end of string");
            }
            let escape = buf[i];
            i += 1;
            match escape {
                b'\\' | b'"' | b'/' | b'b' | b'f' | b't' | b'n' | b'r' => {}
                b'u' => {
                    if i + 4 > buf.len() {
                        return Err("Invalid \\uXXXX escape");
                    }
                    let Some(codepoint) = unhex4(&buf[i..i + 4]) else {
                        return Err("Invalid characters in \\uXXXX escape");
                    };
                    i += 4;
                    if (0xD800..=0xDBFF).contains(&codepoint) {
                        if i + 6 > buf.len() || buf[i] != b'\\' || buf[i + 1] != b'u' {
                            return Err("Invalid \\uXXXX\\uXXXX surrogate pair escape");
                        }
                        match unhex4(&buf[i + 2..i + 6]) {
                            Some(low) if (0xDC00..=0xDFFF).contains(&low) => {}
                            _ => return Err("Invalid \\uXXXX\\uXXXX surrogate pair escape"),
                        }
                        i += 6;
                    }
                }
                _ => return Err("Invalid escape"),
            }
        }
        // The validation above passed. A string with no escape and valid UTF-8 is its own bytes, so
        // it needs neither the checked document parser nor a quoted copy to hand it (#5498).
        let plain = if buf.contains(&b'\\') {
            None
        } else {
            core::str::from_utf8(buf).ok()
        };
        let value = match plain {
            Some(text) => OwnedValue::string(text),
            None => {
                let mut quoted = Vec::with_capacity(self.token_len + 2);
                quoted.push(b'"');
                quoted.extend_from_slice(&self.token_buf[..self.token_len]);
                quoted.push(b'"');
                materialize(&quoted)
            }
        };
        self.value(value)?;
        self.token_len = 0;
        Ok(())
    }
}

/// The number a token spells, read as the document path reads it but without that path's index
/// build and re-parse (#5498): the literal keeps its spelling, so `1.0`, `1e2` and `-0` print as
/// they were written. A token `from_number_bytes` cannot read (it answers `null`, which no number
/// is) goes to [`materialize`], which decides what it is.
fn fast_number(token: &[u8]) -> OwnedValue {
    match OwnedValue::from_number_bytes::<succinctly::jq::JqSemantics>(token) {
        OwnedValue::Null => materialize(token),
        number => number,
    }
}

/// The value a token spells, built by the checked document parser so that it reads exactly as the
/// same text would in a document: a number keeps its literal, a string its escapes decoded, and an
/// invalid byte becomes U+FFFD the way jq's `jv_string_sized` makes it.
fn materialize(token: &[u8]) -> OwnedValue {
    let substituted;
    let bytes = if succinctly::text::utf8::validate_utf8(token).is_ok() {
        token
    } else {
        substituted = succinctly::jq::utf8_document::substitute_invalid_utf8_jq_document(token);
        substituted.as_bytes()
    };
    crate::jq_runner::json_bytes_to_owned_value_checked(bytes).unwrap_or(OwnedValue::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_one(input: &[u8]) -> StreamErrorsRead {
        read(&[(None, input.to_vec())])
    }

    /// The events the reader yielded before the first parse error, each with the lines it had read.
    fn before_fault(input: &str) -> Vec<(String, usize)> {
        let read = read_one(input.as_bytes());
        let Some(end) = read.first_error else {
            return Vec::new();
        };
        read.values
            .iter()
            .zip(read.locations.iter().map(|&(_, lines)| lines))
            .take(end)
            .map(|(event, lines)| (event.to_json(), lines))
            .collect()
    }

    /// A `Read` that hands over `limit` bytes at most per call, as a pipe does.
    struct Trickle {
        data: Vec<u8>,
        pos: usize,
        limit: usize,
    }

    impl std::io::Read for Trickle {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            let n = out.len().min(self.limit).min(self.data.len() - self.pos);
            out[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }

    type Observed = (
        Vec<(String, (usize, usize), Option<String>)>,
        (usize, (usize, usize), Option<usize>),
    );

    fn observe<R: RefillSource>(mut reader: Reader<R>) -> Observed {
        let mut yielded = Vec::new();
        while let Some(item) = reader.next_yielded() {
            yielded.push((item.value.to_json(), item.location, item.error));
        }
        let end = reader.end();
        (yielded, (end.inside, end.slurp_at, end.open_at_end))
    }

    /// #5498: the chunked reader makes the refills the in-memory one makes, however the underlying
    /// reads are cut, so every input, location and the end of the stream agree.
    #[test]
    fn a_chunked_reader_yields_what_the_in_memory_reader_does_5498() {
        let long_line = format!("[{}1]\n[2]\n", "1,".repeat(3000));
        let exact: String = format!("[{}]\n{{\"a\":1}}\n", "1".repeat(4095 - 3)); // 4095-byte first line
        let one_over: String = format!("[{}]\n7", "1".repeat(4096 - 3));
        let streams: Vec<Vec<&str>> = vec![
            vec![""],
            vec!["1"],
            vec!["[1,2]"],
            vec!["[1,2]\n"],
            vec!["{\"a\":[1,{\"b\":null}],\"c\":\"x\"}\n\"s\"\n3\n"],
            vec!["[1,\n}\n]\n}\n7"],
            vec!["\u{feff}[1]\n"],
            vec!["[1,", "2]\n"],
            vec!["", "[1]", "", "\n[2]", ""],
            vec!["1", "2", "3 }", "4\n"],
            vec!["tru", "e\n[", "]\n"],
            vec!["[1]\n", "}\n", "[2"],
            vec!["a\u{0}b [1]\n", "[2]\u{0}[3]"],
            vec![&long_line],
            vec![&exact],
            vec![&one_over],
            vec![&long_line, &exact, "7"],
            vec!["\"x\ny\"\n[1]\n", "{\"a\"}\n"],
            vec!["\"\u{e9}\u{20ac}\"\n[\"\u{1f600}\"]\n"],
        ];
        let mut compared = 0;
        for files in &streams {
            let sources: Vec<(Option<usize>, Vec<u8>)> = files
                .iter()
                .enumerate()
                .map(|(i, text)| (Some(i), text.as_bytes().to_vec()))
                .collect();
            let want = observe(Reader::new(&sources));
            for limit in [1, 2, 3, 7, 64, 100_000] {
                for capacity in [1, 3, 16, 8192] {
                    let openers: Vec<Opener> = files
                        .iter()
                        .map(|text| -> Opener {
                            let data = text.as_bytes().to_vec();
                            Box::new(move || -> std::io::Result<Box<dyn std::io::BufRead>> {
                                Ok(Box::new(std::io::BufReader::with_capacity(
                                    capacity,
                                    Trickle {
                                        data,
                                        pos: 0,
                                        limit,
                                    },
                                )))
                            })
                        })
                        .collect();
                    let got = observe(Reader::from_source(ChunkedSources::new(openers)));
                    assert_eq!(
                        got, want,
                        "{files:?} read {limit} at a time through {capacity}"
                    );
                    if files
                        .iter()
                        .any(|text| text.contains('[') || text.contains('{'))
                    {
                        assert!(!got.0.is_empty(), "{files:?} yields inputs");
                    }
                    compared += 1;
                }
            }
        }
        assert_eq!(compared, streams.len() * 6 * 4);
    }

    /// A `Read` that counts its calls, to see when the chunked reader reads.
    struct Counting {
        data: Vec<u8>,
        pos: usize,
        reads: std::rc::Rc<std::cell::Cell<usize>>,
    }

    impl std::io::Read for Counting {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            self.reads.set(self.reads.get() + 1);
            let n = out.len().min(self.data.len() - self.pos);
            out[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }

    /// #5498: a line is handed over without a read to find out whether another follows, so a pipe
    /// from a producer that writes a line and waits gets that line's events at once.
    #[test]
    fn a_line_is_handed_over_before_the_source_is_read_again_5498() {
        let reads = std::rc::Rc::new(std::cell::Cell::new(0));
        let counter = std::rc::Rc::clone(&reads);
        let opener: Opener = Box::new(move || {
            Ok(Box::new(std::io::BufReader::with_capacity(
                8192,
                Counting {
                    data: b"[1]\n[2]\n".to_vec(),
                    pos: 0,
                    reads: counter,
                },
            )))
        });
        let mut sources = ChunkedSources::new(vec![opener]);
        let mut buf = Vec::new();
        let first = sources.next_refill(&mut buf).expect("a first refill");
        assert_eq!(
            (buf.as_slice(), first.newline, first.is_last),
            (&b"[1]\n"[..], true, false)
        );
        assert_eq!(
            reads.get(),
            1,
            "the buffer was filled once, and no read looked past the line"
        );
        let second = sources.next_refill(&mut buf).expect("a second refill");
        assert_eq!((buf.as_slice(), second.is_last), (&b"[2]\n"[..], false));
        assert_eq!(reads.get(), 1, "the second line was in the buffer");
        // The closing refill is found by the read that finds the end.
        let closing = sources.next_refill(&mut buf).expect("the closing refill");
        assert_eq!((buf.is_empty(), closing.is_last), (true, true));
        assert_eq!(reads.get(), 2);
        assert!(sources.next_refill(&mut buf).is_none());
    }

    /// #5498: a source opens when it is reached and is dropped when it is read, and one that no
    /// longer opens reads as empty with its error kept.
    #[test]
    fn a_source_opens_when_reached_and_a_failed_open_reads_as_empty_5498() {
        let opened = std::rc::Rc::new(std::cell::Cell::new(0));
        let make = |text: &'static str, opened: std::rc::Rc<std::cell::Cell<usize>>| -> Opener {
            Box::new(move || {
                opened.set(opened.get() + 1);
                Ok(Box::new(std::io::Cursor::new(text.as_bytes().to_vec())))
            })
        };
        let failing: Opener =
            Box::new(|| Err(std::io::Error::new(std::io::ErrorKind::Other, "gone")));
        let mut reader = Reader::from_source(ChunkedSources::new(vec![
            make("[1]\n", std::rc::Rc::clone(&opened)),
            failing,
            make("[2]\n", std::rc::Rc::clone(&opened)),
        ]));
        assert_eq!(opened.get(), 0, "nothing is opened before the first read");
        let first = reader.next_yielded().expect("an input");
        assert_eq!(first.value.to_json(), "[[0],1]");
        assert_eq!(opened.get(), 1);
        let mut rest = Vec::new();
        while let Some(item) = reader.next_yielded() {
            rest.push(item.value.to_json());
        }
        assert_eq!(rest, ["[[0]]", "[[0],2]", "[[0]]"]);
        assert_eq!(opened.get(), 2);
        assert_eq!(
            reader.take_error().map(|e| e.to_string()),
            Some("gone".to_string())
        );
    }

    /// #5498: a read that fails part-way ends its source there and is kept for the caller.
    #[test]
    fn a_read_error_ends_the_source_and_is_kept_5498() {
        struct Failing(usize);
        impl std::io::Read for Failing {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                if self.0 == 0 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "input/output error",
                    ));
                }
                self.0 -= 1;
                out[..4].copy_from_slice(b"[1]\n");
                Ok(4)
            }
        }
        let opener: Opener = Box::new(|| Ok(Box::new(std::io::BufReader::new(Failing(1)))));
        let mut reader = Reader::from_source(ChunkedSources::new(vec![opener]));
        let mut values = Vec::new();
        while let Some(item) = reader.next_yielded() {
            values.push(item.value.to_json());
        }
        assert_eq!(values, ["[[0],1]", "[[0]]"]);
        assert_eq!(
            reader.take_error().map(|e| e.to_string()),
            Some("input/output error".to_string())
        );
    }

    /// #5498: `--slurp` keeps the location of the last few values only, and of the first error, and
    /// reads the same values as the full read.
    #[test]
    fn a_read_for_slurp_keeps_the_tail_of_the_locations_5498() {
        let events: String = (0..40).map(|n| format!("[{n}]\n")).collect();
        let sources = [(None, events.into_bytes())];
        let (full, slurp) = (read(&sources), read_for_slurp(&sources, false));
        let json = |read: &StreamErrorsRead| {
            read.values
                .iter()
                .map(OwnedValue::to_json)
                .collect::<Vec<_>>()
        };
        assert_eq!(json(&full), json(&slurp));
        assert_eq!((full.inside, full.slurp_at), (slurp.inside, slurp.slurp_at));
        assert_eq!(slurp.locations.len(), SLURP_TAIL);
        assert_eq!(slurp.locations_from, full.values.len() - SLURP_TAIL);
        assert_eq!(slurp.locations[..], full.locations[slurp.locations_from..]);
        assert_eq!(full.locations_from, 0);
        assert!(full.first_fault.is_none() && slurp.first_fault.is_none());
        // The first error's message and location are kept wherever it falls, and a read told to stop
        // there reads no further.
        let broken = [(None, b"[1]\n}\n[2]\n}\n".to_vec())];
        let (full, slurp) = (read(&broken), read_for_slurp(&broken, false));
        let stopped = read_for_slurp(&broken, true);
        let first = full.first_error.expect("a fault");
        assert_eq!(
            full.first_fault.as_ref().map(|f| f.1),
            Some(full.locations[first])
        );
        assert_eq!(full.first_fault, slurp.first_fault);
        assert_eq!(full.first_fault, stopped.first_fault);
        assert_eq!(stopped.values.len(), first + 1);
        assert!(stopped.values.len() < full.values.len());
    }

    /// #5498: what jq's `--slurp` loop leaves after its array stays within the locations
    /// [`read_for_slurp`] keeps, for the shapes that end a stream in different ways.
    #[test]
    fn the_values_after_the_slurp_array_fit_the_kept_locations_5498() {
        for stream in [
            "",
            "1",
            "1 2",
            "[1,2]",
            "[1,2]\n",
            "[1,[2]]",
            "{\"a\":1}",
            "{\"a\":[1,2]}",
            "[[1,2],[3]]",
            "[1,2",
            "}",
            "}}}}}}}}}}",
            "[1,2] }",
            "[1,\n}\n}\n}\n]",
            "tru",
            "1 2 3 4 5 6 7 8 9 10 11 12",
            "[]",
            "{}",
            "[[],{}]",
            "\"x\"",
            "[\"a\",\"b\"]\n[\"c\"]",
        ] {
            let read = read_for_slurp(&[(None, stream.as_bytes().to_vec())], false);
            assert!(read.inside >= read.locations_from, "{stream:?}");
            assert!(read.values.len() - read.inside <= SLURP_TAIL, "{stream:?}");
        }
    }

    /// The fast paths (#5498) must build what the checked document parser builds, to the literal:
    /// a number keeps its spelling, a string its decoded text.
    #[test]
    fn the_scalar_fast_paths_agree_with_the_document_parser_5498() {
        let numbers = [
            "0",
            "-0",
            "1",
            "-1",
            "9",
            "10",
            "007",
            "-007",
            "00",
            "1.0",
            "1e2",
            "-1E+2",
            "0.5",
            ".5",
            "-.5",
            "1.",
            "1.50",
            "0e0",
            "1e400",
            "-1e400",
            "1E-400",
            "123456789012345678",
            "-123456789012345678",
            "1234567890123456789",
            "9223372036854775807",
            "9223372036854775808",
            "-9223372036854775808",
            "100000000000000000000",
            "0.1e1",
            "1e5",
            "1e-5",
            "12345678901234567890.5",
        ];
        let mut compared = 0;
        for text in numbers {
            if !number_is_valid(text.as_bytes()) {
                continue;
            }
            compared += 1;
            let (fast, slow) = (fast_number(text.as_bytes()), materialize(text.as_bytes()));
            assert_eq!(fast.to_json(), slow.to_json(), "{text}");
            assert_eq!(
                core::mem::discriminant(&fast),
                core::mem::discriminant(&slow),
                "{text}"
            );
            assert_eq!(fast.number_str(), slow.number_str(), "{text}");
        }
        // The spellings the document parser tolerates must be among those compared.
        for text in ["007", ".5", "-.5", "1.", "1e400"] {
            assert!(
                number_is_valid(text.as_bytes()),
                "{text} is no longer a valid token"
            );
        }
        assert!(compared >= 25, "only {compared} spellings were compared");
        // Bytes that are not UTF-8 take the substituting path, not the fast one.
        for bytes in [&b"\"a\xffb\""[..], b"\"\x80\"", b"\"\xe3\x81\""] {
            let events = read_one(&[b"[", bytes, b"]"].concat()).values;
            assert_eq!(
                events[0].to_json(),
                format!("[[0],{}]", materialize(bytes).to_json())
            );
        }
        for input in [
            "\"\"",
            "\"abc\"",
            "\"h\u{e9}llo \u{1F600}\"",
            "\"a b\"",
            "\"\\n\"",
            "\"\\u00e9\"",
            "\"\\ud83d\\ude00\"",
            "\"\\\"\"",
        ] {
            let inner = &input[1..input.len() - 1];
            let events = run(&format!("[{input}]"));
            let expected = materialize(input.as_bytes()).to_json();
            assert_eq!(events[0], format!("[[0],{expected}]"), "{inner}");
        }
    }

    /// #5635: the events a malformed document yielded before its fault, with the lines read.
    #[test]
    fn the_events_before_a_fault_stop_at_the_first_error_5635() {
        let row = |event: &str, lines| (event.to_string(), lines);
        assert_eq!(
            before_fault("[1,2,3 x]"),
            [row("[[0],1]", 0), row("[[1],2]", 0)]
        );
        // The count is of newlines read, so it is 0 on the first line and goes up with each line.
        assert_eq!(
            before_fault("[1,\n2,\n3 x]\n"),
            [row("[[0],1]", 1), row("[[1],2]", 2)]
        );
        assert_eq!(before_fault("{{\"a\":\"b\"}}"), []);
        // A fault in a later document leaves the earlier ones' events in.
        assert_eq!(
            before_fault("[1]\n[3,x"),
            [row("[[0],1]", 1), row("[[0]]", 1), row("[[0],3]", 1)]
        );
    }

    #[test]
    fn no_events_come_before_a_fault_the_parser_does_not_meet_5635() {
        assert_eq!(before_fault("[1,2]\n"), []);
        assert_eq!(before_fault(""), []);
        // Truncation is a fault at EOF: the events are the ones before it.
        assert_eq!(before_fault("[1,2"), [("[[0],1]".to_string(), 0)]);
    }

    fn run(input: &str) -> Vec<String> {
        read_one(input.as_bytes())
            .values
            .iter()
            .map(OwnedValue::to_json)
            .collect()
    }

    #[test]
    fn a_truncated_array_reports_the_events_before_it_and_the_path() {
        assert_eq!(
            run("[1,"),
            [
                "[[0],1]",
                r#"["Unfinished JSON term at EOF at line 1, column 3",[1]]"#
            ]
        );
    }

    #[test]
    fn an_object_key_not_yet_read_is_null_in_the_path() {
        assert_eq!(
            run("{"),
            [r#"["Unfinished JSON term at EOF at line 1, column 1",[null]]"#]
        );
    }

    #[test]
    fn a_nul_ends_a_number_token_but_not_a_keyword_5474() {
        // The number parser reads the token as a C string.
        assert_eq!(run("2.5\u{0}{}\n"), ["[[],2.5]", "[[],{}]"]);
        // `tru` + NUL is five bytes where `true` is four.
        assert_eq!(
            run("tru\u{0}\n"),
            [r#"["Invalid literal at line 2, column 0",[]]"#]
        );
    }

    #[test]
    fn a_chunk_with_no_newline_stops_at_its_first_nul() {
        assert_eq!(run("1 \u{0}2 3"), ["[[],1]"]);
        // ... but a chunk that ends in a newline is scanned whole, the NUL included.
        assert_eq!(
            run("1 \u{0}2 3\n"),
            [
                "[[],1]",
                r#"["Invalid numeric literal at line 1, column 5",[]]"#
            ]
        );
    }

    #[test]
    fn the_slurp_array_closes_at_the_call_that_empties_the_buffer() {
        // No final newline: the read ends on the `]` that yields the leaf, so the close is outside.
        let read = read_one(b"[1,2]");
        assert_eq!((read.values.len(), read.inside), (3, 2));
        // A final newline: the close is handed out by a call before the read that hits the end.
        let read = read_one(b"[1,2]\n");
        assert_eq!((read.values.len(), read.inside), (3, 3));
    }

    #[test]
    fn a_value_yielded_after_the_input_is_closed_has_no_location() {
        let read = read_one(b"[1,2]");
        let lines: Vec<usize> = read.locations.iter().map(|&(_, line)| line).collect();
        assert_eq!(lines, [0, 0, CLOSED]);
        assert_eq!(read.open_at_end, None);
        // A newline-terminated end leaves the input open for the call that finds the end.
        assert_eq!(read_one(b"[1,2]\n").open_at_end, Some(1));
    }

    #[test]
    fn a_token_continues_into_the_next_source() {
        let read = read(&[(Some(0), b"[1".to_vec()), (Some(1), b"2]\n".to_vec())]);
        let events: Vec<String> = read.values.iter().map(OwnedValue::to_json).collect();
        assert_eq!(events, ["[[0],12]", "[[0]]"]);
    }

    /// Each row's output was captured from `jq --stream-errors -c .` (jq 1.7.1) and covers one of
    /// the parser's error arms or an event shape.
    #[test]
    #[allow(clippy::literal_string_with_formatting_args)]
    fn every_error_arm_and_event_shape_matches_jq_5474() {
        let rows: &[(&str, &str)] = &[
        ("[1 2]", "[\"Expected separator between values at line 1, column 5\",[0]]\n"),
        ("{[\"a\"]}", "[\"Expected string key after '{', not '[' at line 1, column 2\",[null]]\n"),
        ("{\"x\":\"y\",[\"foo\"]}", "[[\"x\"],\"y\"]\n[\"Expected string key after ',' in object, not '[' at line 1, column 10\",[null]]\n"),
        ("{{\"a\":1}}", "[\"Expected string key after '{', not '{' at line 1, column 2\",[null]]\n"),
        ("{\"x\":\"y\",{\"foo\":\"bar\"}}", "[[\"x\"],\"y\"]\n[\"Expected string key after ',' in object, not '{' at line 1, column 10\",[null]]\n"),
        (":", "[\"':' not as part of an object at line 1, column 1\",[]]\n"),
        ("[1:2]", "[\"':' not as part of an object at line 1, column 3\",[0]]\n"),
        ("{\"a\":1:2}", "[\"Object keys must be strings at line 1, column 7\",[\"a\"]]\n"),
        ("{1:2}", "[\"Object keys must be strings at line 1, column 3\",[null]]\n"),
        ("{\"a\" \"b\"}", "[\"Expected separator between values at line 1, column 8\",[null]]\n"),
        ("1:2", "[\"':' not as part of an object at line 1, column 2\",[]]\n"),
        ("{,}", "[\"Expected value before ',' at line 1, column 2\",[null]]\n"),
        ("{\"a\",1}", "[\"Objects must consist of key:value pairs at line 1, column 5\",[null]]\n"),
        ("[,]", "[\"Expected value before ',' at line 1, column 2\",[0]]\n"),
        (",", "[\"Expected value before ',' at line 1, column 1\",[]]\n"),
        ("[1,,2]", "[[0],1]\n[\"Expected value before ',' at line 1, column 4\",[1]]\n"),
        ("]", "[\"Unmatched ']' at the top-level at line 1, column 1\",[]]\n"),
        ("[1,2,]", "[[0],1]\n[[1],2]\n[\"Expected another array element at line 1, column 6\",[2]]\n"),
        ("{\"a\":[1]]}", "[[\"a\",0],1]\n[[\"a\",0]]\n[\"Unmatched ']' in the middle of an object at line 1, column 9\",[\"a\"]]\n"),
        ("}", "[\"Unmatched '}' at the top-level at line 1, column 1\",[]]\n"),
        ("{\"a\":1,}", "[[\"a\"],1]\n[\"Expected another key:value pair at line 1, column 8\",[null]]\n"),
        ("{\"a\":}", "[\"Missing value in key:value pair at line 1, column 6\",[\"a\"]]\n"),
        ("{\"a\":1]", "[\"Unmatched ']' in the middle of an object at line 1, column 7\",[\"a\"]]\n"),
        ("[}", "[\"Unmatched '}' in the middle of an array at line 1, column 2\",[0]]\n"),
        ("[1}", "[\"Unmatched '}' in the middle of an array at line 1, column 3\",[0]]\n"),
        ("{1}", "[\"Objects must consist of key:value pairs at line 1, column 3\",[null]]\n"),
        ("{\"a\"}", "[\"Objects must consist of key:value pairs at line 1, column 5\",[null]]\n"),
        ("[1,2] }", "[[0],1]\n[[1],2]\n[[1]]\n[\"Unmatched '}' at the top-level at line 1, column 7\",[]]\n"),
        ("\"\\x\"", "[\"Invalid escape at line 1, column 4\",[]]\n"),
        ("\"\\u12\"", "[\"Invalid \\\\uXXXX escape at line 1, column 6\",[]]\n"),
        ("\"\\u12zz\"", "[\"Invalid characters in \\\\uXXXX escape at line 1, column 8\",[]]\n"),
        ("\"\\ud800x\"", "[\"Invalid \\\\uXXXX\\\\uXXXX surrogate pair escape at line 1, column 9\",[]]\n"),
        ("\"\\ud800\\u0041\"", "[\"Invalid \\\\uXXXX\\\\uXXXX surrogate pair escape at line 1, column 14\",[]]\n"),
        ("\"a\tb\"", "[\"Invalid string: control characters from U+0000 through U+001F must be escaped at line 1, column 5\",[]]\n"),
        ("tru", "[\"Invalid literal at EOF at line 1, column 3\",[]]\n"),
        ("nul", "[\"Invalid literal at EOF at line 1, column 3\",[]]\n"),
        ("1e", "[\"Invalid numeric literal at EOF at line 1, column 2\",[]]\n"),
        ("-", "[\"Invalid numeric literal at EOF at line 1, column 1\",[]]\n"),
        ("\"abc", "[\"Unfinished string at EOF at line 1, column 4\",[]]\n"),
        ("[1,2", "[[0],1]\n[\"Unfinished JSON term at EOF at line 1, column 4\",[1]]\n"),
        ("[[]]", "[[0],[]]\n[[0]]\n"),
        ("{\"a\":{}}", "[[\"a\"],{}]\n[[\"a\"]]\n"),
        ("[[1],[2]]", "[[0,0],1]\n[[0,0]]\n[[1,0],2]\n[[1,0]]\n[[1]]\n"),
        ("{\"a\":[]}", "[[\"a\"],[]]\n[[\"a\"]]\n"),
        ("{\"a\":{\"b\":[]}}", "[[\"a\",\"b\"],[]]\n[[\"a\",\"b\"]]\n[[\"a\"]]\n"),
        ("[{}]", "[[0],{}]\n[[0]]\n"),
        ("{\"a\":1,\"b\":[2,3]}", "[[\"a\"],1]\n[[\"b\",0],2]\n[[\"b\",1],3]\n[[\"b\",1]]\n[[\"b\"]]\n"),
        ("\"\\ud83d\\ude00\"", "[[],\"😀\"]\n"),
        ];
        for (input, want) in rows {
            let got: String = run(input).iter().map(|line| format!("{line}\n")).collect();
            assert_eq!(got, *want, "{input:?}");
        }
    }

    #[test]
    fn an_empty_source_is_a_refill_that_hands_out_what_the_last_byte_left() {
        // `[1]` leaves its close pending after the last byte; the empty source after it is the
        // call that hands it out, at that source's own position.
        let read = read(&[(Some(0), b"[1]".to_vec()), (Some(1), Vec::new())]);
        let events: Vec<String> = read.values.iter().map(OwnedValue::to_json).collect();
        assert_eq!(events, ["[[0],1]", "[[0]]"]);
        assert_eq!(read.locations, [(0, 0), (1, 0)]);
        assert_eq!(read.inside, 2);
        // An empty input has no value, and nothing to slurp.
        assert_eq!(read_one(b"").values.len(), 0);
    }

    /// The refills as the reader first found them: the whole list up front, one `Vec` entry per
    /// line of the input. The cursor must hand out the same refills one at a time (#5498).
    fn reference_refills(
        sources: &[(Option<usize>, Vec<u8>)],
    ) -> Vec<(usize, Vec<u8>, bool, bool)> {
        let mut refills: Vec<(usize, Vec<u8>, bool, bool)> = Vec::new();
        for (index, (_, raw)) in sources.iter().enumerate() {
            let last_source = index + 1 == sources.len();
            let lens: Vec<usize> = crate::jq_seq_reader::fgets_chunk_lens(raw).collect();
            if lens.is_empty() {
                refills.push((index, Vec::new(), false, last_source));
                continue;
            }
            let mut start = 0;
            for (k, &len) in lens.iter().enumerate() {
                let chunk = &raw[start..start + len];
                start += len;
                let newline = chunk.ends_with(b"\n");
                let hit_eof = !newline && len < JQ_FGETS_CHUNK;
                let bytes = if newline {
                    chunk
                } else {
                    &chunk[..chunk.iter().position(|&b| b == 0).unwrap_or(len)]
                };
                refills.push((
                    index,
                    bytes.to_vec(),
                    newline,
                    last_source && k + 1 == lens.len() && hit_eof,
                ));
            }
        }
        if let Some(last) = refills.last() {
            if !last.3 {
                refills.push((last.0, Vec::new(), false, true));
            }
        }
        refills
    }

    /// #5498: the cursor finds the refills the up-front list held, over sources that end on a
    /// newline, in the middle of a line, exactly on a full buffer, past one, with NULs, or empty.
    #[test]
    fn the_refill_cursor_hands_out_the_refills_the_list_held() {
        let long = |n: usize, tail: &[u8]| {
            let mut v = vec![b'a'; n];
            v.extend_from_slice(tail);
            v
        };
        let pieces: Vec<Vec<u8>> = vec![
            Vec::new(),
            b"1".to_vec(),
            b"1\n".to_vec(),
            b"[1,2]\n[3]\n".to_vec(),
            b"[1,2]\n[3]".to_vec(),
            b"\n".to_vec(),
            b"\n\n\n".to_vec(),
            b"a\0b\n".to_vec(),
            b"a\0b".to_vec(),
            long(JQ_FGETS_CHUNK - 1, b"\n"),
            long(JQ_FGETS_CHUNK, b""),
            long(JQ_FGETS_CHUNK, b"\n"),
            long(JQ_FGETS_CHUNK + 1, b""),
            long(2 * JQ_FGETS_CHUNK, b"\nx"),
            long(JQ_FGETS_CHUNK - 1, b"x\ny"),
        ];
        let mut cases: Vec<Vec<(Option<usize>, Vec<u8>)>> = vec![Vec::new()];
        for a in &pieces {
            cases.push(vec![(Some(0), a.clone())]);
            for b in &pieces {
                cases.push(vec![(Some(0), a.clone()), (Some(1), b.clone())]);
            }
        }
        cases.push(vec![
            (None, pieces[3].clone()),
            (None, Vec::new()),
            (None, pieces[10].clone()),
        ]);
        for sources in cases {
            let want = reference_refills(&sources);
            let mut cursor = RefillCursor::default();
            let mut got = Vec::new();
            while let Some(r) = cursor.next(&sources) {
                got.push((r.source, r.bytes.to_vec(), r.newline, r.is_last));
            }
            assert_eq!(
                got,
                want,
                "{:?}",
                sources.iter().map(|s| s.1.len()).collect::<Vec<_>>()
            );
            assert!(
                cursor.next(&sources).is_none(),
                "the cursor stays at the end"
            );
        }
    }

    /// #5498: handing the inputs out one at a time ends in the state the batch read reported.
    #[test]
    fn a_reader_pulled_one_input_at_a_time_ends_as_the_batch_read_does() {
        let sources = vec![
            (Some(0), b"[1,2]\n{\"a\" 2}\n[3".to_vec()),
            (Some(1), b"]\n4 5\n".to_vec()),
        ];
        let batch = read(&sources);
        let mut reader = Reader::new(&sources);
        let mut pulled = Vec::new();
        while let Some(item) = reader.next() {
            pulled.push(item);
        }
        let end = reader.end();
        assert_eq!(pulled.len(), batch.values.len());
        for (i, (value, location)) in pulled.iter().enumerate() {
            assert_eq!(value.to_json(), batch.values[i].to_json());
            assert_eq!(*location, batch.locations[i]);
        }
        assert_eq!(end.inside, batch.inside);
        assert_eq!(end.slurp_at, batch.slurp_at);
        assert_eq!(end.open_at_end, batch.open_at_end);
        assert_eq!(end.first_error, batch.first_error);
        assert!(reader.next().is_none(), "a finished reader stays finished");
    }

    /// #5498: a reader that has handed out one input has fed the parser the refills that
    /// completed it and no more, and holds none of the values after it.
    #[test]
    fn a_reader_feeds_the_parser_only_as_far_as_the_input_asked_for() {
        let lines: String = (0..5000).map(|i| format!("[{i}]\n")).collect();
        let sources = vec![(Some(0), lines.into_bytes())];
        let total = sources[0].1.len();
        let mut reader = Reader::new(&sources);
        let (first, _) = reader.next().expect("a first input");
        assert_eq!(first.to_json(), "[[0],0]");
        assert!(
            reader.source.cursor.pos < 64,
            "{} of {total} bytes fed",
            reader.source.cursor.pos
        );
        // `[0]` completed two events (the leaf and the close); the second waits in the queue.
        assert_eq!(reader.parser.pending.len(), 1);
        assert_eq!(reader.next().expect("the close").0.to_json(), "[[0]]");
        assert!(reader.source.cursor.pos < 64);
        // Reading on to the end hands out every event and then nothing.
        let mut count = 2;
        while reader.next().is_some() {
            count += 1;
        }
        assert_eq!(count, 10_000);
        assert!(
            reader.source.cursor.source == sources.len() && reader.source.cursor.closed,
            "the cursor has moved past the last source"
        );
    }
}
