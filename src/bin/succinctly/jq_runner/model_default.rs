//! The default route of `run_jq_on_stack_floor` through ADR-0033's reader model (#7987, step 6).
//!
//! The default route is the one a program that reads no `input`, `inputs` or `input_line_number`
//! takes over JSON read whole (`.`, `-c .`, `.field`): it splits each file's text into values with
//! the semi-index and runs the program on each, lazily (#2103). It enters the model only on a
//! trigger, a fact the route already has, and hands back to the semi-index at the next sync point:
//!
//! - **a byte-order mark that is not whole** ([`AsRead::mark`]): the stream opens with `EF` and its
//!   first three bytes are not the mark in one source. The model reads from the stream's start
//!   until the mark is decided: a mark that breaks off is jq's `Malformed BOM` (#5804 retired on
//!   this route), and one cut across files hands back after the read that completes it;
//! - **invalid UTF-8** the route substituted (a file's [`LossySource`] edits): the model is entered
//!   at the last sync point before the first substituted byte, reads the values there, which it
//!   validates as jq's parser does (ADR-0033 decision 3), and hands back at the first sync point
//!   after that byte. The next substituted byte past it is the next trigger;
//! - **a fault**, the splitter's (`split_error`), or one the model meets in a region it reads.
//!
//! A file seam is not a trigger.
//!
//! The model's values are spans of the sources as read. Each is mapped onto the text the route
//! reads (after the mark is taken off, the seams stitched and the bytes substituted), so the route
//! evaluates and prints it exactly as it would have printed its own value there; only which values
//! there are, and the fault, are the model's.
//!
//! Hand-back carries [`ReaderState`]: the state at the sync point the model stopped at is kept, and
//! the next region is entered from it with [`ReaderState::skip_plain`] over the plain stretch the
//! semi-index read in between, which reads no byte. A route that hands off without it is the ADR's
//! named main risk.
//!
//! **A fault is read from the start of the stream.** The route's main loop stops at the first
//! fault, as jq's does, so a fault ends the run and is read once. jq names the *first* fault of its
//! input, and the semi-index validated nothing it did not read before it (#3035), so a malformed
//! value it printed earlier can hold jq's first fault: entered at the last sync point, the model
//! would name a later one. The route already read the whole prefix again for the message before
//! this step (`jq_stream_fault`), so the model reads that prefix in its place, keeps the route's
//! values before the file the fault is in, and gives that file's values itself, up to the fault.

use std::collections::VecDeque;

use std::rc::Rc;

use super::reader_model::{Item, PlainRun, PlainStretch, ReadMode, ReaderState, Stop};
use super::{count_newlines, LossySource, Seam};
use crate::cli::jq_reader_profile::JQ_READER;
use succinctly::jq::jv_parse::{BOM_DONE, UTF8_BOM};
use succinctly::jq::utf8_document::substituted_offset;

/// The route reads plain JSON, never `--seq`.
const NON_SEQ: ReadMode = ReadMode { seq: false };

/// What the route keeps of its sources as read, before the mark is taken off and the seams
/// stitched: each one's length, and whether the stream opens with a mark that is not whole.
pub(super) struct AsRead {
    lens: Vec<usize>,
    mark: bool,
}

/// [`AsRead`], and the mark [`strip_leading_json_bom`](super::strip_leading_json_bom) took off each
/// source's front: what the model needs, beside the route's texts, to read the sources as read.
pub(super) struct Facts {
    as_read: AsRead,
    cuts: Vec<usize>,
}

impl Facts {
    /// The first fault jq's reader meets in the sources as read, rebuilt from the route's `texts`
    /// and `lossy`, in jq's words: what the route words a fault the program met inside a value with
    /// (#5576), in place of `jq_stream_fault`'s reading.
    pub(super) fn first_fault(
        &self,
        texts: &[Vec<u8>],
        lossy: &[Option<LossySource>],
    ) -> Option<String> {
        let texts = Texts {
            texts,
            lossy,
            seams: &[],
        };
        let stream = Stream::of(self, &texts)?;
        let sources = stream.slices();
        let mut state = ReaderState::start(NON_SEQ);
        let mut reads = Vec::new();
        loop {
            let stop = state.step(&sources, &JQ_READER, NON_SEQ, &mut reads);
            for read in reads.drain(..) {
                match read.item {
                    Item::Value(_) => {}
                    Item::Fault(fault) => return Some(fault.message()),
                    Item::End => return None,
                }
            }
            if stop == Stop::Stuck {
                // patchcov: coverage tolerate reason="unreachable: a stuck step ends on its fault, returned above"
                return None;
                // patchcov: coverage end
            }
        }
    }
}

impl AsRead {
    /// The facts of `raws`, the sources as read.
    pub(super) fn of(raws: &[Vec<u8>]) -> Self {
        let mark = raws
            .iter()
            .find(|raw| !raw.is_empty())
            .is_some_and(|raw| raw[0] == UTF8_BOM[0] && !raw.starts_with(&UTF8_BOM));
        Self {
            lens: raws.iter().map(Vec::len).collect(),
            mark,
        }
    }
}

/// The route's texts, as the model maps its spans onto them: each file's text, the substitution
/// that made it from the bytes as read (`None` where it changed nothing), and what the stitch moved
/// onto its front from earlier files (`Seam::base`; empty when nothing was stitched).
#[derive(Clone, Copy)]
pub(super) struct Texts<'t> {
    pub texts: &'t [Vec<u8>],
    pub lossy: &'t [Option<LossySource>],
    pub seams: &'t [Seam],
}

impl Texts<'_> {
    fn lossy(&self, file: usize) -> Option<&LossySource> {
        self.lossy.get(file).and_then(Option::as_ref)
    }

    /// How many bytes of file `file`'s text came from earlier files.
    fn base(&self, file: usize) -> usize {
        self.seams.get(file).map_or(0, |seam| seam.base)
    }

    /// The bytes as read that file `file`'s text was made from.
    fn original(&self, file: usize) -> &[u8] {
        self.lossy(file)
            .map_or(self.texts[file].as_slice(), |lossy| &lossy.original)
    }
}

/// The sources as read, rebuilt from the route's texts (the stitch only moves bytes across a seam,
/// so the texts in order, each after the mark it lost, are the stream), with where each source and
/// each text starts in it. `None` when they do not add up.
struct Stream {
    bytes: Vec<u8>,
    /// Where each source starts, and the stream's end last.
    sources: Vec<usize>,
    /// Where each text's bytes as read start, after the mark, and the stream's end last.
    texts: Vec<usize>,
}

impl Stream {
    fn of(facts: &Facts, texts: &Texts<'_>) -> Option<Self> {
        let as_read = &facts.as_read;
        let total: usize = as_read.lens.iter().sum();
        let mut bytes = Vec::with_capacity(total);
        let mut text_starts = Vec::with_capacity(texts.texts.len() + 1);
        // The mark's bytes each source lost: a mark cut across sources (#7566) goes on in the next.
        let mut mark = 0;
        for file in 0..texts.texts.len() {
            let cut = facts.cuts.get(file).copied().unwrap_or(0);
            let next = (mark + cut).min(UTF8_BOM.len());
            bytes.extend_from_slice(&UTF8_BOM[mark..next]);
            mark = next;
            text_starts.push(bytes.len());
            bytes.extend_from_slice(texts.original(file));
        }
        text_starts.push(bytes.len());
        if bytes.len() != total || texts.texts.len() != as_read.lens.len() {
            return None;
        }
        let mut sources = Vec::with_capacity(as_read.lens.len() + 1);
        let mut at = 0;
        for &len in &as_read.lens {
            sources.push(at);
            at += len;
        }
        sources.push(at);
        Some(Self {
            bytes,
            sources,
            texts: text_starts,
        })
    }

    fn slices(&self) -> Vec<&[u8]> {
        self.sources
            .windows(2)
            .map(|w| &self.bytes[w[0]..w[1]])
            .collect()
    }

    /// Where the reader of `state` stands in the stream.
    fn reader_at(&self, state: &ReaderState) -> usize {
        match state.input {
            Some(file) => self.sources[state.next_file - 1] + file.offset,
            None => self.sources[state.next_file.min(self.sources.len() - 1)],
        }
    }

    /// The stream offset of `at` in file `file`'s text.
    fn of_text(&self, texts: &Texts<'_>, file: usize, at: usize) -> usize {
        let original = texts
            .lossy(file)
            .map_or(at, |lossy| lossy.original_offset(at));
        self.texts[file] + original
    }

    /// The text that holds stream offset `at` (the last one starting at or before it, so a place
    /// between two texts is the later one's start), and the offset there.
    fn to_text(&self, texts: &Texts<'_>, at: usize) -> (usize, usize) {
        let files = self.texts.len() - 1;
        let file = self.texts[..files]
            .partition_point(|&start| start <= at)
            .saturating_sub(1);
        (file, self.text_offset(texts, file, at))
    }

    /// The offset in file `file`'s text of stream offset `at`, which is in or at the end of it.
    fn text_offset(&self, texts: &Texts<'_>, file: usize, at: usize) -> usize {
        let raw = at.saturating_sub(self.texts[file]);
        texts
            .lossy(file)
            .map_or(raw, |lossy| substituted_offset(&lossy.edits, raw))
    }

    /// The span `start..end` of the stream as a value of the text holding it.
    fn value(&self, texts: &Texts<'_>, start: usize, end: usize) -> (usize, usize, usize) {
        let (file, from) = self.to_text(texts, start);
        debug_assert!(
            end <= self.texts[file + 1],
            "a value is in one text: the stitch moved it whole"
        );
        (file, from, self.text_offset(texts, file, end))
    }

    /// The plain stretch `from..to` as [`ReaderState::skip_plain`] takes it from `state`, which
    /// stands at `from`.
    fn runs(&self, state: &ReaderState, from: usize, to: usize) -> Vec<PlainRun> {
        let files = self.sources.len() - 1;
        let first = match state.input {
            Some(file) if !file.eof => state.next_file - 1,
            _ => state.next_file,
        };
        // The source the stretch's last byte is in.
        let last = self.sources[1..]
            .partition_point(|&end| end < to)
            .min(files - 1);
        (first..=last.max(first))
            .filter(|&file| file < files)
            .map(|file| {
                let start = self.sources[file].max(from);
                let end = self.sources[file + 1].min(to);
                let bytes = &self.bytes[start.min(end)..end];
                let tail = bytes
                    .iter()
                    .rposition(|&b| b == b'\n')
                    .map_or(bytes.len(), |last| bytes.len() - last - 1);
                PlainRun {
                    file,
                    len: bytes.len(),
                    newlines: count_newlines(bytes),
                    tail,
                    to_end: end == self.sources[file + 1],
                    bom: bytes.starts_with(&UTF8_BOM),
                }
            })
            .collect()
    }
}

/// Where the route reads its own values again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Resume {
    /// From the start of every file from here on.
    Route,
    /// From this offset of this file's text; the model holds the values before it.
    At(usize, usize),
    /// Nowhere: the model read to the end of the input.
    Never,
}

/// What the model met in a region it read.
enum Outcome {
    /// It handed back at this stream offset, with this state.
    HandBack(usize, ReaderState),
    /// It read to the end of the input.
    End,
    /// It met a fault.
    Fault(String),
}

/// The model on the default route, once a trigger has been met.
struct Entered {
    stream: Stream,
    /// The facts the stream was rebuilt from (the mark taken off each source).
    cuts: Rc<Facts>,
    /// The last sync point the model handed back at, and the reader there: where the next region
    /// is entered from when no later sync point is known.
    carried: (ReaderState, usize),
    /// The model's values the route has not evaluated yet, as spans of the texts, in order.
    queued: VecDeque<(usize, usize, usize)>,
    resume: Resume,
    /// The fault the run stops at, and the file after whose values it is reported.
    fault: Option<(usize, String)>,
}

/// The default route's model: nothing until a trigger.
pub(super) struct DefaultModel {
    facts: Rc<Facts>,
    mark_pending: bool,
    entered: Option<Entered>,
    /// The model could not rebuild the stream (it does not add up): the route's own reading stands.
    off: bool,
}

impl DefaultModel {
    /// The model of a run whose sources as read are `as_read`, with the mark `cuts` took off them.
    pub(super) fn new(as_read: AsRead, cuts: &[usize]) -> Self {
        let mark_pending = as_read.mark;
        Self {
            facts: Rc::new(Facts {
                as_read,
                cuts: cuts.to_vec(),
            }),
            mark_pending,
            entered: None,
            off: false,
        }
    }

    /// What the model reads the sources with, for a fault worded away from the loop.
    pub(super) fn facts(&self) -> Rc<Facts> {
        Rc::clone(&self.facts)
    }

    /// Whether file `file` needs [`plan`](Self::plan): it has a trigger, or the model has been
    /// entered, which plans every file after.
    #[inline]
    pub(super) fn plans(&self, split_error: Option<usize>, substituted: bool) -> bool {
        !self.off
            && (split_error.is_some() || substituted || self.mark_pending || self.entered.is_some())
    }

    /// The values the route evaluates in file `file`, whose text the splitter read as `values`
    /// (spans of `texts.texts[file]`) and `split_error`, with the triggers in it read by the model:
    /// `values` is replaced and `split_error` taken. The fault the run stops at once those values are
    /// out comes back, in jq's words.
    pub(super) fn plan(
        &mut self,
        file: usize,
        texts: &Texts<'_>,
        values: &mut Vec<(usize, usize)>,
        split_error: &mut Option<usize>,
    ) -> Option<String> {
        if self.entered.is_none() {
            let Some(stream) = Stream::of(&self.facts, texts) else {
                // patchcov: coverage tolerate reason="defensive: the stitch only moves bytes across a seam, so the texts add up to the sources"
                debug_assert!(false, "the texts read are the sources read");
                self.off = true;
                return None;
                // patchcov: coverage end
            };
            #[cfg(test)]
            ENTERED.with(|n| n.set(n.get() + 1));
            self.entered = Some(Entered {
                stream,
                cuts: Rc::clone(&self.facts),
                carried: (ReaderState::start(NON_SEQ), 0),
                queued: VecDeque::new(),
                resume: Resume::Route,
                fault: None,
            });
        }
        let entered = self.entered.as_mut().expect("entered above");
        let route = std::mem::take(values);
        let fault_at = split_error.take();
        if std::mem::take(&mut self.mark_pending) {
            // The mark is decided at the stream's start, before any value.
            match entered.region((ReaderState::start(NON_SEQ), 0), 0, 0, texts) {
                Outcome::HandBack(at, state) => entered.hand_back(texts, at, state),
                Outcome::End => entered.resume = Resume::Never,
                Outcome::Fault(_) => return entered.fault_mode(file, texts, values),
            }
        }
        entered.plan(file, texts, route, fault_at, values)
    }
}

impl Entered {
    fn plan(
        &mut self,
        file: usize,
        texts: &Texts<'_>,
        route: Vec<(usize, usize)>,
        fault_at: Option<usize>,
        values: &mut Vec<(usize, usize)>,
    ) -> Option<String> {
        let mut cursor = match self.resume {
            Resume::Route => Some(0),
            Resume::At(at_file, at) if at_file == file => Some(at),
            Resume::At(at_file, _) if at_file < file => Some(0),
            Resume::At(..) | Resume::Never => None,
        };
        self.take_queued(file, values);
        if cursor.is_some() {
            self.resume = Resume::Route;
        }
        while let Some(from) = cursor {
            let edit = texts
                .lossy(file)
                .and_then(|lossy| {
                    let first = lossy.edits.partition_point(|edit| edit.text.start < from);
                    lossy.edits.get(first)
                })
                .map(|edit| edit.text.start);
            let fault = fault_at.filter(|&at| at >= from);
            match (edit, fault) {
                (_, Some(fault)) if edit.map_or(true, |edit| fault <= edit) => {
                    // The route met a fault: read from the stream's start (the module's comment).
                    values.clear();
                    return self.fault_mode(file, texts, values);
                }
                (None, _) => {
                    values.extend_from_slice(&route[starting_at(&route, from)..]);
                    cursor = None;
                }
                (Some(edit), _) => {
                    let trigger = self.stream.of_text(texts, file, edit);
                    let (entry, state) = self.entry(file, texts, &route, from, edit);
                    let entry_text = (entry >= self.stream.of_text(texts, file, from))
                        .then(|| self.stream.text_offset(texts, file, entry));
                    if let Some(entry) = entry_text {
                        // The route's values from `from` that end by the entry.
                        let first = starting_at(&route, from);
                        let kept = route[first..].partition_point(|&(_, end)| end <= entry);
                        values.extend_from_slice(&route[first..first + kept]);
                    }
                    let drop_through = entry.max(self.stream.of_text(texts, file, from));
                    match self.region((state, entry), drop_through, trigger + 1, texts) {
                        Outcome::HandBack(at, state) => {
                            self.hand_back(texts, at, state);
                            self.take_queued(file, values);
                            cursor = match self.resume {
                                Resume::At(at_file, at) if at_file == file => {
                                    self.resume = Resume::Route;
                                    Some(at)
                                }
                                _ => None,
                            };
                        }
                        Outcome::End => {
                            self.resume = Resume::Never;
                            self.take_queued(file, values);
                            cursor = None;
                        }
                        Outcome::Fault(_) => {
                            values.clear();
                            return self.fault_mode(file, texts, values);
                        }
                    }
                }
            }
        }
        self.fault_after(file)
    }

    /// Moves the queued values of file `file` to `values`.
    fn take_queued(&mut self, file: usize, values: &mut Vec<(usize, usize)>) {
        while let Some(&(at, start, end)) = self.queued.front() {
            if at > file {
                break;
            }
            self.queued.pop_front();
            if at == file {
                values.push((start, end));
            }
        }
    }

    /// The fault to report after file `file`'s values, when it is that file's.
    fn fault_after(&mut self, file: usize) -> Option<String> {
        match &self.fault {
            Some((at, _)) if *at <= file => self.fault.take().map(|(_, message)| message),
            _ => None,
        }
    }

    /// The model reads from the stream's start to its first fault: its values before file `file`'s
    /// text are the route's, already out, and the rest are queued, the fault after them.
    fn fault_mode(
        &mut self,
        file: usize,
        texts: &Texts<'_>,
        values: &mut Vec<(usize, usize)>,
    ) -> Option<String> {
        #[cfg(test)]
        FAULT_READS.with(|n| n.set(n.get() + 1));
        self.queued.clear();
        let drop_through = self.stream.texts[file];
        let outcome = self.region(
            (ReaderState::start(NON_SEQ), 0),
            drop_through,
            usize::MAX,
            texts,
        );
        let last = self.queued.back().map_or(file, |&(at, _, _)| at.max(file));
        match outcome {
            Outcome::Fault(message) => self.fault = Some((last, message)),
            // patchcov: coverage tolerate reason="defensive: the splitter stops only at a fault jq's parser also meets"
            Outcome::End | Outcome::HandBack(..) => {} // patchcov: coverage end
        }
        self.resume = Resume::Never;
        self.take_queued(file, values);
        self.fault_after(file)
    }

    /// Where the model is entered for a trigger at `edit` of file `file`'s text, which the route
    /// reads from `from` on as `route`: the last sync point before it, and the reader there. That
    /// is the byte after a newline of the file's own bytes that no value the route read holds, else
    /// the file's start when nothing was carried onto it, else where the model last handed back.
    fn entry(
        &self,
        file: usize,
        texts: &Texts<'_>,
        route: &[(usize, usize)],
        from: usize,
        edit: usize,
    ) -> (usize, ReaderState) {
        let base = texts.base(file);
        let (carried, carried_at) = self.carried;
        let own = from.max(base);
        let entry = match newline_between_values(&texts.texts[file], own, edit, route) {
            Some(newline) => Some(self.stream.of_text(texts, file, newline + 1)),
            None if from == 0 && base == 0 => {
                // The source's start, mark included: jq's first read of it starts there.
                let cut = self.cuts.cuts.get(file).copied().unwrap_or(0);
                let start = self.stream.texts[file] - cut;
                Some(start)
            }
            None => None,
        };
        match entry.filter(|&entry| entry > carried_at) {
            Some(entry) => {
                let runs = self.stream.runs(&carried, carried_at, entry);
                let stretch = PlainStretch {
                    runs: &runs,
                    files: self.stream.sources.len() - 1,
                    value_in_final_buffer: false,
                };
                match carried.skip_plain(&stretch, &JQ_READER, NON_SEQ) {
                    Some(state) => {
                        #[cfg(test)]
                        ENTRIES.with(|log| log.borrow_mut().push((entry, state)));
                        (entry, state)
                    }
                    // patchcov: coverage tolerate reason="defensive: the stretch ends after a newline or at a file's start, between values, which skip_plain takes"
                    None => (carried_at, carried),
                    // patchcov: coverage end
                }
            }
            None => (carried_at, carried),
        }
    }

    /// The model reads from `from` (a sync point and the reader there) to the first sync point past
    /// stream offset `past` with the mark decided: its values that end after `drop_through` are
    /// queued.
    fn region(
        &mut self,
        from: (ReaderState, usize),
        drop_through: usize,
        past: usize,
        texts: &Texts<'_>,
    ) -> Outcome {
        let (mut state, _) = from;
        let sources = self.stream.slices();
        let mut reads = Vec::new();
        loop {
            let stop = state.step(&sources, &JQ_READER, NON_SEQ, &mut reads);
            for read in reads.drain(..) {
                match read.item {
                    Item::Value(span) => {
                        if span.end > drop_through {
                            let value = self.stream.value(texts, span.start, span.end);
                            self.queued.push_back(value);
                        }
                    }
                    Item::Fault(fault) => return Outcome::Fault(fault.message()),
                    Item::End => return Outcome::End,
                }
            }
            if stop == Stop::Stuck {
                // patchcov: coverage tolerate reason="unreachable: a stuck step ends on its fault, returned above"
                return Outcome::End;
                // patchcov: coverage end
            }
            let at = self.stream.reader_at(&state);
            if at >= past && state.parser.bom == BOM_DONE {
                return Outcome::HandBack(at, state);
            }
        }
    }

    /// The model handed back at stream offset `at`, with `state`: the route reads on from there.
    fn hand_back(&mut self, texts: &Texts<'_>, at: usize, state: ReaderState) {
        self.carried = (state, at);
        let (file, offset) = self.stream.to_text(texts, at);
        self.resume = Resume::At(file, offset);
    }
}

/// The first of the route's `values` (spans in order) that starts at or after `from`.
fn starting_at(values: &[(usize, usize)], from: usize) -> usize {
    values.partition_point(|&(start, _)| start < from)
}

/// The last newline of `text[from..bound]` that no value of `values` (the route's spans of the
/// text, in order) holds.
fn newline_between_values(
    text: &[u8],
    from: usize,
    mut bound: usize,
    values: &[(usize, usize)],
) -> Option<usize> {
    while bound > from {
        let newline = from + text[from..bound].iter().rposition(|&b| b == b'\n')?;
        // The first value that ends after the newline.
        let after = values.partition_point(|&(_, end)| end <= newline);
        match values.get(after) {
            Some(&(start, _)) if start <= newline => bound = start,
            _ => return Some(newline),
        }
    }
    None
}

#[cfg(test)]
thread_local! {
    /// Test-only: how many runs on this thread entered the model on the default route.
    pub(super) static ENTERED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test-only: how many of them read a fault from the stream's start.
    pub(super) static FAULT_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test-only: each region entered from a carried state, with the state carried there.
    static ENTRIES: std::cell::RefCell<Vec<(usize, ReaderState)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
mod tests;
