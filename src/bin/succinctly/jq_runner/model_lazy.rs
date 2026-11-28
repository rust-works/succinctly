//! The lazy routes of `read_inputs` through ADR-0033's reader model (#7987, step 7b).
//!
//! A program that reads `input`, `inputs` or `input_line_number` over a lone source longer than its
//! front, over several regular files, or over input the materializing route read whole and scans a
//! chunk at a time, takes its documents from a lazy producer: the text is read a
//! window at a time, scanned a chunk at a time (both parsers check every document), and each document
//! is converted when the program reads it. Before this step a fault the scan met handed the rest of
//! the stream to the eager reader, which read it whole (a file to its end, every later file too) and
//! named each fault with the imitation of jq's positions, and carried nothing of where jq's reader
//! stood. Now the producer enters the model at the last sync point before the fault, reads a read at a
//! time to the first sync point past it, and hands back to the scan there:
//!
//! - **Entry.** The last newline before the fault that no value holds, among what the text has not
//!   handed out yet; else where the model last handed back in this file, with the [`ReaderState`] it
//!   handed back with; else the file's front when nothing was dropped or moved onto it. At a line
//!   start the state is [`ReaderState::at_line_start`] of the producer's counters, less the
//!   [`Carry::lag`] the faults before have left in the parser's line count (what
//!   [`ReaderState::skip_plain`] carries over a plain stretch with line breaks).
//! - **The region** is fed one `fgets` read at a time from the text as read ([`RegionSource`]): the
//!   bytes a substitution replaced, and the mark the scan skipped at the stream's front. A read the
//!   text does not hold yet reads the source's next window; a read in another file ends the attempt,
//!   and the producer's old hand-off reads the stream (a value that runs on across a file seam after a
//!   fault). Nothing of the input is copied whole: the region holds one read.
//! - **Hand-back** is at the first sync point past the byte the scan stopped at, with the mark
//!   decided: the scan reads on from there, and the [`ReaderState`] there is carried
//!   ([`Carry::at`]) to the next fault, in this file or, as its lag, in the next.
//!
//! What jq's `input` returned in the region (its values, decoded as the materializing route decodes
//! them, its faults in jq's words, and the end of input) is handed out in order, after the documents
//! the scan found before the entry and before the ones it finds after the hand-back.

use std::collections::VecDeque;

use super::reader_model::{Held, Item, Position, ReadMode, ReaderState, Session, Source, Stop};
use super::{jq, line_seed, EvalError, LossySource, NO_SOURCE, UNKNOWN_LINE};
use crate::cli::jq_reader_profile::JQ_READER;
use succinctly::jq::jv_parse::{BOM_DONE, UTF8_BOM};

/// The lazy routes read plain JSON, never `--seq`.
const NON_SEQ: ReadMode = ReadMode { seq: false };

/// What the producer carries from one region to the next.
#[derive(Default)]
pub(super) struct Carry {
    /// Where the model last handed back in the source being read, as a file offset as read, and the
    /// reader there: the next region in that source is entered there when no later sync point is
    /// known. Cleared when the producer moves on to the next file.
    pub at: Option<(usize, ReaderState)>,
    /// How many line breaks jq's parser has not counted: one for each read a fault dropped the rest
    /// of that ended on a line break. jq has one parser for the run, so it carries across files.
    pub lag: usize,
    /// What the last region returned, not handed out yet, in order.
    pub pending: VecDeque<jq::ProducedInput>,
    /// The last value the last region read: its first byte, and where it ends as a file offset as
    /// read. How the stream ends is read off the last value before it.
    pub last_value: Option<(u8, usize)>,
}

/// The `(source, line)` the queue places an answer at `at` by: the file is its source tag (every file
/// of these routes opened, so a file's position among them is its tag).
fn place(at: Position) -> (u32, u64) {
    match at.file {
        Some(file) => (
            u32::try_from(file).unwrap_or(u32::MAX),
            (at.line + line_seed()) as u64,
        ),
        None => (NO_SOURCE, UNKNOWN_LINE),
    }
}

/// Where the queue says the stream ended, for the end of input found at `at`.
pub(super) fn exhausted_at(at: Position, last_source: u32) -> Option<(u32, u64)> {
    at.file
        .map(|_| (last_source, (at.line + line_seed()) as u64))
}

/// The text of one source as the region reads it.
#[derive(Clone, Copy)]
pub(super) struct TextView<'t> {
    /// The text the scan reads: the source's bytes from where its windows have dropped up to, after
    /// the mark, with the bytes an earlier file's cut document moved onto its front.
    pub text: &'t [u8],
    /// The bytes as read, where `text` is a substitution of them.
    pub lossy: Option<&'t LossySource>,
    /// How many of `text`'s first bytes were moved onto it from an earlier file.
    pub base: usize,
    /// The file offset as read of `text[base]`.
    pub front: usize,
    /// The mark the scan skipped before the text, at the stream's front only.
    pub lead: usize,
    /// The text holds the file's end.
    pub to_end: bool,
}

impl<'t> TextView<'t> {
    /// The offset in the bytes as read (relative to `text[0]`) of `at` in the text.
    fn as_read(&self, at: usize) -> usize {
        self.lossy.map_or(at, |lossy| lossy.original_offset(at))
    }

    /// The bytes as read, from `text[base]` on.
    fn bytes(&self) -> &'t [u8] {
        let all = self
            .lossy
            .map_or(self.text, |lossy| lossy.original.as_slice());
        &all[self.as_read(self.base).min(all.len())..]
    }

    /// The file offset as read of `at` in the text (`at >= base`).
    pub(super) fn file_offset(&self, at: usize) -> usize {
        self.front + self.as_read(at) - self.as_read(self.base)
    }

    /// The text offset of file offset `at` as read, at or past `text[base]`.
    pub(super) fn text_offset(&self, at: usize) -> usize {
        let raw = at - self.front + self.as_read(self.base);
        self.lossy.map_or(raw, |lossy| {
            succinctly::jq::utf8_document::substituted_offset(&lossy.edits, raw)
        })
    }
}

/// The source the producer reads a region from: a view of its text, which it can read on.
pub(super) trait LazyHeld {
    fn view(&self) -> TextView<'_>;
    /// Reads the source's next window onto the text; `false` when it has none.
    fn read_on(&mut self) -> bool;
}

/// One file's text as read, for a [`Session`]: the file `file` among `files`, with the files before it
/// empty when the session starts at the stream's front.
struct RegionSource<'t> {
    view: TextView<'t>,
    file: usize,
    files: usize,
    empty_before: bool,
    /// The read in hand when it starts in the mark.
    read: Vec<u8>,
}

impl Source for RegionSource<'_> {
    fn files(&self) -> usize {
        self.files
    }

    /// A region names its file's own offsets.
    fn start_of(&self, _file: usize) -> usize {
        0
    }

    fn held(&mut self, file: usize, offset: usize, want: usize) -> Held<'_> {
        let short = Held {
            bytes: &[],
            to_end: false,
        };
        if file < self.file && self.empty_before {
            return Held {
                bytes: &[],
                to_end: true,
            };
        }
        let view = self.view;
        let held_from = view.front;
        if file != self.file || offset + view.lead < held_from {
            return short;
        }
        let bytes = view.bytes();
        if offset < held_from {
            // The mark before the stream's first text, then the text.
            self.read.clear();
            self.read
                .extend_from_slice(&UTF8_BOM[offset + view.lead - held_from..view.lead]);
            let room = want.saturating_sub(self.read.len());
            self.read.extend_from_slice(&bytes[..room.min(bytes.len())]);
            let to_end = view.to_end && room >= bytes.len();
            return Held {
                bytes: &self.read,
                to_end,
            };
        }
        let from = (offset - held_from).min(bytes.len());
        Held {
            bytes: &bytes[from..],
            to_end: view.to_end,
        }
    }
}

/// How a region ended.
pub(super) enum RegionEnd {
    /// At a sync point: the file offset as read, and the reader there.
    HandBack(usize, ReaderState),
    /// At the end of the input, where the position stands after the call that found it.
    End(Position),
}

/// Where a region is entered.
pub(super) struct Entry {
    pub state: ReaderState,
    /// The file the region is in, among `files`.
    pub file: usize,
    pub files: usize,
    /// The session starts at the stream's front, so the files before `file` are empty.
    pub front: bool,
    /// Values that end at or before this file offset were handed out already.
    pub drop_through: usize,
    /// The region hands back at the first sync point past this file offset.
    pub past: usize,
}

/// Reads the region `entry` names over `held`, into `carry.pending`: `None` when a read it needs is
/// in another file, so the caller's old reading stands and nothing was handed out.
// `reads` is drained, not consumed: the buffer is the next step's.
#[allow(clippy::iter_with_drain)]
pub(super) fn read_region(
    held: &mut impl LazyHeld,
    entry: &Entry,
    carry: &mut Carry,
) -> Option<RegionEnd> {
    let mut session = {
        let source = region_source(held.view(), entry);
        Session::resume(entry.state, &source, &JQ_READER, NON_SEQ)
    };
    let mut reads = Vec::new();
    let mut items = Vec::new();
    loop {
        let stop = {
            let view = held.view();
            let mut source = region_source(view, entry);
            let stop = session.step(&mut source, &mut reads);
            for read in reads.drain(..) {
                match read.item {
                    Item::Value(span) => {
                        if span.end <= entry.drop_through {
                            continue;
                        }
                        let bytes = view.bytes();
                        let from = span.start.saturating_sub(view.front);
                        let to = (span.end - view.front).min(bytes.len());
                        let text = &bytes[from.min(to)..to];
                        let value = super::model_read::decode(text);
                        let (source, line) = place(read.at);
                        items.push(jq::ProducedInput::converted(value, source, line));
                        carry.last_value = text.first().map(|&first| (first, span.end));
                    }
                    Item::Fault(fault) => items.push(jq::ProducedInput::Error(
                        EvalError::new(fault.message()),
                        place(read.at),
                    )),
                    Item::End => {
                        carry.pending.extend(items);
                        return Some(RegionEnd::End(read.at));
                    }
                }
            }
            stop
        };
        match stop {
            Stop::Short => {
                if !held.read_on() {
                    return None;
                }
            }
            // patchcov: coverage tolerate-line reason="unreachable: the mark is decided before the lazy route's first chunk, which is all a region follows"
            Stop::Stuck => return None,
            Stop::Synced => {
                let (file, offset) = session.stands();
                let state = session.state()?;
                // At the end of the last file the next call finds the end of input, which closes the
                // file: that is the model's to read (a fault at EOF is followed by `<unknown>`).
                let input_over =
                    file + 1 == entry.files && state.input.map_or(true, |input| input.eof);
                if file == entry.file
                    && offset > entry.past
                    && state.parser.bom == BOM_DONE
                    && !input_over
                {
                    carry.pending.extend(items);
                    return Some(RegionEnd::HandBack(offset, state));
                }
            }
        }
    }
}

fn region_source<'t>(view: TextView<'t>, entry: &Entry) -> RegionSource<'t> {
    RegionSource {
        view,
        file: entry.file,
        files: entry.files,
        empty_before: entry.front,
        read: Vec::new(),
    }
}

/// The reader at a line start of file `file` with `file_lines` line breaks of its own before it and
/// `stream_lines` in the stream, `offset` bytes into the file as read, after the faults `lag`
/// accounts for: [`ReaderState::at_line_start`] with the parser's count behind by the line breaks
/// the faults dropped.
pub(super) fn at_line_start(
    file: usize,
    offset: usize,
    file_lines: usize,
    stream_lines: usize,
    lag: usize,
) -> ReaderState {
    let mut state = ReaderState::at_line_start(file, offset, file_lines, stream_lines);
    state.parser.line = state.parser.line.saturating_sub(lag);
    state
}

/// The reader at the front of file `file`, which starts at 1-based `line` and `column` of the stream
/// (nothing before it in the stream when `front`), less `lag`.
pub(super) fn at_file_start(
    file: usize,
    line: usize,
    column: usize,
    front: bool,
    lag: usize,
) -> ReaderState {
    if front {
        return ReaderState::start(NON_SEQ);
    }
    let mut state = at_line_start(file, 0, 0, line.saturating_sub(1), lag);
    state.parser.column = column;
    state
}
