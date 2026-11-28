//! The routes that read a window at a time through ADR-0033's reader model (#7987, step 7).
//!
//! A windowed route holds a window of its input, never the whole of it, so the model is fed the
//! bytes as read of the window it needs ([`HeldBytes`], a [`Source`]) and is entered where the window
//! says the reader stands: at a line start no value holds, [`ReaderState::at_line_start`] is a
//! closed form of the window's line and offset counters, and at the stream's front it is
//! [`ReaderState::start`]. Nothing of the input is copied whole: the model's region is the bytes of
//! one window from the last sync point before its trigger.
//!
//! [`ReaderState::at_line_start`]: super::reader_model::ReaderState::at_line_start
//! [`ReaderState::start`]: super::reader_model::ReaderState::start

use super::count_newlines;
use super::reader_model::{Held, Item, ReadMode, ReaderState, Session, Source, Stop};
use super::window::ModelFacts;
use succinctly::jq::jv_parse::{BOM_DONE, UTF8_BOM};
use succinctly::jq::utf8_document::{original_offset, substituted_offset, SubstitutionEdit};

/// The windowed routes this module reads are plain JSON, never `--seq` (step 8).
const NON_SEQ: ReadMode = ReadMode { seq: false };

/// One file's bytes as read from `base` on, the part of its input a route holds: the bytes of a
/// window, given back where the substitution changed them. A read past them is short unless they
/// run to the file's end.
pub(super) struct HeldBytes<'b> {
    pub bytes: &'b [u8],
    /// The file they are of, by its position among the inputs.
    pub file: usize,
    /// Where they start in the file, as read.
    pub base: usize,
    /// They run to the file's end.
    pub to_end: bool,
    /// How many input files the run has.
    pub files: usize,
}

impl Source for HeldBytes<'_> {
    fn files(&self) -> usize {
        self.files
    }

    /// A session over held bytes is entered in their file, whose own offsets it names.
    fn start_of(&self, _file: usize) -> usize {
        0
    }

    fn held(&mut self, file: usize, offset: usize, _want: usize) -> Held<'_> {
        if file != self.file || offset < self.base {
            // Another file's bytes, which the window does not hold.
            return Held {
                bytes: &[],
                to_end: false,
            };
        }
        let from = (offset - self.base).min(self.bytes.len());
        Held {
            bytes: &self.bytes[from..],
            to_end: self.to_end,
        }
    }
}

/// The last newline of `text[from..bound]` that no value of `values` (spans of the text, in order)
/// holds: the byte after it is a sync point, once the mark is decided (ADR-0033).
pub(super) fn newline_between_values(
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

/// What the default route's window is, for [`plan_window`]: its text and where the substitution
/// changed it, the facts the window gives the model, and where the window stands in the input.
pub(super) struct Window<'w> {
    pub raw: &'w [u8],
    /// Where the substitution changed `raw`, relative to it ([`super::window::Window::edits`]).
    pub edits: &'w [SubstitutionEdit],
    pub facts: &'w ModelFacts<'w>,
    /// The file the window is of, among `files`.
    pub file: usize,
    pub files: usize,
    /// The file's own newlines before `raw`.
    pub lines_before: usize,
    /// The 1-based line of the stream `raw` starts on.
    pub line: usize,
    /// How many of `raw`'s first bytes were moved onto the file from the one before.
    pub base: usize,
}

/// The default route's window with invalid UTF-8 in it read by jq's reader model (#7987, step 7, as
/// step 6 reads a file read whole): for each substituted byte past where the route reads, the model
/// is entered at the last sync point before it, reads the values there (validating them as jq's
/// parser does, ADR-0033 decision 3), and hands back at the first sync point after it, carrying
/// [`ReaderState`] to the next substituted byte when no line start lies between. `values` (the
/// route's spans of `raw`) is replaced by the route's values outside the regions and the model's
/// inside them. A fault the model meets in a region ends the values there and becomes
/// `split_error`, the place the route reports the stream's first fault at (which the windowed
/// fault readers name, from the stream's start). A substituted byte the window holds no sync
/// point before (it is on the first line of a later file, or of a document a window before began),
/// or no sync point after (its document runs on into the next window), is read as before.
#[cold]
#[inline(never)]
pub(super) fn plan_window(
    window: &Window<'_>,
    values: &mut Vec<(usize, usize)>,
    split_error: &mut Option<usize>,
) {
    #[cfg(test)]
    PLANNED.with(|n| n.set(n.get() + 1));
    let raw = window.raw;
    let route = std::mem::take(values);
    let facts = window.facts;
    // Where the route reads its own values from, and the reader there once the model handed back.
    let mut from = 0usize;
    let mut carried: Option<(usize, ReaderState)> = None;
    let as_read = |text: usize| facts.file_front + original_offset(window.edits, text) as i64;
    // What the window decides: through its last value, or all of it at the file's end. Past that is a
    // document the next window goes on with, which a later window's region reads (and whose fault
    // the windowed fault readers have not met yet).
    let decided = if facts.ends_file {
        raw.len()
    } else {
        route.last().map_or(0, |&(_, end)| end)
    };
    for edit in window.edits {
        let trigger = edit.text.start;
        // Before `run_from` is the end of a document an earlier window ran, whose bytes it read.
        if trigger < from.max(window.base).max(facts.run_from) || trigger >= decided {
            continue;
        }
        if split_error.is_some_and(|fault| fault <= trigger) {
            break;
        }
        let low = from.max(window.base).max(facts.run_from);
        let newline = newline_between_values(raw, low, trigger, &route).map(|at| at + 1);
        let entry = match (newline, carried) {
            (Some(entry), Some((at, _))) if entry <= at => Some(at),
            (Some(entry), _) => Some(entry),
            (None, Some((at, _))) if at <= trigger => Some(at),
            (None, _) if from == 0 && window.base == 0 && facts.front.is_some() => Some(0),
            (None, _) => None,
        };
        let Some(entry) = entry else {
            continue;
        };
        let stream_front = entry == 0 && facts.front.is_some() && window.base == 0;
        let state = match carried {
            Some((at, state)) if at == entry => state,
            _ if stream_front => ReaderState::start(NON_SEQ),
            _ => {
                let Ok(offset) = usize::try_from(as_read(entry)) else {
                    continue;
                };
                let lines = count_newlines(&raw[window.base.min(entry)..entry]);
                ReaderState::at_line_start(
                    window.file,
                    offset,
                    window.lines_before + lines,
                    window.line - 1 + count_newlines(&raw[..entry]),
                )
            }
        };
        let mut source = WindowSource {
            window,
            empty_before: stream_front,
            read: Vec::new(),
        };
        let past = facts.file_front + edit.raw.end as i64;
        let Some(region) = read_region(state, &mut source, window.file, past) else {
            continue;
        };
        // The bytes as read the region is in, back to `raw`.
        let to_text = |at: usize| {
            substituted_offset(window.edits, (at as i64 - facts.file_front).max(0) as usize)
        };
        if let RegionEnd::Fault(at) = region.end {
            if to_text(at) >= decided {
                continue;
            }
        }
        let first = route.partition_point(|&(start, _)| start < from);
        let kept = route[first..].partition_point(|&(_, end)| end <= entry);
        values.extend_from_slice(&route[first..first + kept]);
        values.extend(
            region
                .values
                .iter()
                .map(|&(start, end)| (to_text(start), to_text(end)))
                // Past what the window decides is the next window's (a bare token before a document
                // the window holds open waits for it, #4311), and before `run_from` an earlier one's.
                .filter(|&(start, end)| start >= entry.max(facts.run_from) && end <= decided),
        );
        #[cfg(test)]
        REGIONS.with(|n| n.set(n.get() + 1));
        match region.end {
            RegionEnd::HandBack(at, state) => {
                from = to_text(at);
                carried = Some((from, state));
            }
            RegionEnd::Fault(at) => {
                *split_error = Some(to_text(at).min(raw.len().saturating_sub(1)));
                return;
            }
            RegionEnd::End => return,
        }
    }
    let first = route.partition_point(|&(start, _)| start < from);
    values.extend_from_slice(&route[first..]);
}

/// How a region the model read ended.
enum RegionEnd {
    /// At a sync point: the file offset as read, and the reader there.
    HandBack(usize, ReaderState),
    /// At a fault, the file offset as read of the byte it was raised on.
    Fault(usize),
    /// At the end of the input.
    End,
}

/// What the model read in a region: its values, as file offsets as read, and how it ended.
struct Region {
    values: Vec<(usize, usize)>,
    end: RegionEnd,
}

/// The model from `state` over `source` to the first sync point at or past file offset `past` of
/// `file`, with the mark decided; `None` when the bytes held end first (the region runs on past the
/// window).
#[allow(clippy::iter_with_drain)]
fn read_region(
    state: ReaderState,
    source: &mut WindowSource<'_, '_>,
    file: usize,
    past: i64,
) -> Option<Region> {
    let profile = &crate::cli::jq_reader_profile::JQ_READER;
    let mut session = Session::resume(state, &*source, profile, NON_SEQ);
    let mut reads = Vec::new();
    let mut values = Vec::new();
    loop {
        let stop = session.step(source, &mut reads);
        for read in reads.drain(..) {
            match read.item {
                Item::Value(span) => values.push((span.start, span.end)),
                Item::Fault(fault) => {
                    return Some(Region {
                        values,
                        end: RegionEnd::Fault(fault.offset),
                    })
                }
                Item::End => {
                    return Some(Region {
                        values,
                        end: RegionEnd::End,
                    })
                }
            }
        }
        if stop != Stop::Synced {
            return None;
        }
        let (at_file, offset) = session.stands();
        let state = session.state()?;
        if at_file > file {
            return None;
        }
        if at_file == file && offset as i64 >= past && state.parser.bom == BOM_DONE {
            return Some(Region {
                values,
                end: RegionEnd::HandBack(offset, state),
            });
        }
    }
}

/// A window of the default route as the model reads it ([`plan_window`]): its text given back as the
/// bytes it was read as a read at a time, each substituted run as the bytes it replaced, and at the
/// stream's front the mark the route skipped. Nothing of the window is copied but the read in hand.
struct WindowSource<'w, 'f> {
    window: &'w Window<'f>,
    /// The session starts at the stream's front: the files before the window's are empty.
    empty_before: bool,
    /// The read in hand, as read.
    read: Vec<u8>,
}

impl Source for WindowSource<'_, '_> {
    fn files(&self) -> usize {
        self.window.files
    }

    /// A session over a window is entered in its file, whose own offsets it names.
    fn start_of(&self, _file: usize) -> usize {
        0
    }

    fn held(&mut self, file: usize, offset: usize, want: usize) -> Held<'_> {
        let window = self.window;
        if file < window.file && self.empty_before {
            return Held {
                bytes: &[],
                to_end: true,
            };
        }
        let front = window.facts.file_front;
        if file != window.file || (offset as i64) < front - window.facts.front.unwrap_or(0) as i64 {
            // Another file's bytes, or bytes before the window: not held.
            return Held {
                bytes: &[],
                to_end: false,
            };
        }
        self.read.clear();
        // At the stream's front, the mark the route skipped before the text.
        let mut raw_at = offset as i64 - front;
        if raw_at < 0 {
            let lead = window.facts.front.unwrap_or(0);
            let from = (lead as i64 + raw_at) as usize;
            self.read.extend_from_slice(&UTF8_BOM[from..lead]);
            raw_at = 0;
        }
        let raw_at = raw_at as usize;
        let (edits, replaced) = (window.edits, window.facts.edit_bytes);
        // The first edit that ends after the offset (in the bytes as read), and where the text is.
        let mut next = edits.partition_point(|edit| edit.raw.end <= raw_at);
        let mut text = match edits.get(next) {
            Some(edit) if edit.raw.start < raw_at => {
                // Inside a substituted run: the rest of the bytes it replaced.
                let bytes = &replaced[next][raw_at - edit.raw.start..];
                self.read.extend_from_slice(bytes);
                next += 1;
                edit.text.end
            }
            _ => substituted_offset(edits, raw_at),
        };
        let raw = window.raw;
        // Up to a read's worth, a newline or the end of the text.
        while self.read.len() < want && text < raw.len() {
            let stop = edits
                .get(next)
                .map_or(raw.len(), |edit| edit.text.start.min(raw.len()));
            let room = want - self.read.len();
            let piece = &raw[text..stop.min(text + room)];
            if let Some(newline) = piece.iter().position(|&b| b == b'\n') {
                self.read.extend_from_slice(&piece[..=newline]);
                text += newline + 1;
                break;
            }
            self.read.extend_from_slice(piece);
            text += piece.len();
            if text == stop && stop < raw.len() {
                self.read.extend_from_slice(&replaced[next]);
                text = edits[next].text.end;
                next += 1;
            }
        }
        Held {
            bytes: &self.read,
            to_end: text >= raw.len() && window.facts.ends_file,
        }
    }
}

#[cfg(test)]
thread_local! {
    /// Test-only: the windows [`plan_window`] read, and the regions the model read in them.
    pub(super) static PLANNED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static REGIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use succinctly::jq::utf8_document::substitute_invalid_utf8_jq_document_mapped;

    /// A window over `bytes` as read, at the front of a lone file's stream: its text is jq's
    /// substitution of them.
    fn window_of(bytes: &[u8]) -> (Vec<u8>, Vec<SubstitutionEdit>, Vec<Vec<u8>>) {
        let (text, edits) = substitute_invalid_utf8_jq_document_mapped(bytes);
        let replaced = edits
            .iter()
            .map(|edit| bytes[edit.raw.clone()].to_vec())
            .collect();
        (text.into_bytes(), edits, replaced)
    }

    /// Generated lines of values, invalid UTF-8 in and out of strings among them.
    fn lines(seed: u64) -> Vec<u8> {
        const PIECES: &[&[u8]] = &[
            b"1",
            b"\"\xff\"",
            b"[\"\xc3(\"]",
            b"{\"a\":\"\xe9b\"}",
            b"\"a\xf0\x9f\"",
            b"[1,2]",
            b"\"\xef\xbb\"",
            b"\"x\"",
            b"\"\x80\x80\x80\"",
        ];
        let mut rng = seed;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng as usize
        };
        let mut bytes = Vec::new();
        for _ in 0..=(next() % 40) {
            bytes.extend_from_slice(PIECES[next() % PIECES.len()]);
            bytes.extend_from_slice([&b"\n"[..], b" ", b"\n\n", b"  \n"][next() % 4]);
            if next() % 7 == 0 {
                bytes.extend(std::iter::repeat(b' ').take(4090 + next() % 10));
            }
        }
        bytes
    }

    /// The window's source gives back the bytes as read, a read at a time from any offset, the
    /// middle of a substituted run included: what the model reads in a region is what jq read.
    #[test]
    fn a_window_reads_back_as_the_bytes_it_was_read_as_7987() {
        for seed in 1..=40u64 {
            let bytes = lines(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let (raw, edits, replaced) = window_of(&bytes);
            let facts = ModelFacts {
                file_front: 0,
                edit_bytes: &replaced,
                run_from: 0,
                ends_file: true,
                front: Some(0),
            };
            let window = Window {
                raw: &raw,
                edits: &edits,
                facts: &facts,
                file: 0,
                files: 1,
                lines_before: 0,
                line: 1,
                base: 0,
            };
            let mut source = WindowSource {
                window: &window,
                empty_before: false,
                read: Vec::new(),
            };
            for from in 0..=bytes.len() {
                let held = source.held(0, from, 7);
                let want = &bytes[from..];
                let newline = want.iter().position(|&b| b == b'\n');
                let len = held.bytes.len();
                assert!(want.starts_with(held.bytes), "seed {seed} at {from}");
                assert!(
                    len >= 7.min(want.len()) || newline.is_some_and(|at| at + 1 == len),
                    "seed {seed} at {from}: {len} bytes"
                );
                assert_eq!(held.to_end, len == want.len(), "seed {seed} at {from}");
            }
        }
    }

    /// Over a window whose values are all well formed, the model's regions give the values the
    /// splitter gave, whatever was substituted: only a fault, or which values there are, is ever
    /// the model's to change.
    #[test]
    fn regions_over_well_formed_values_keep_the_routes_values_7987() {
        let mut regions = 0;
        for seed in 1..=200u64 {
            let bytes = lines(seed.wrapping_mul(0xD1B5_4A32_D192_ED03));
            let (raw, edits, replaced) = window_of(&bytes);
            let facts = ModelFacts {
                file_front: 0,
                edit_bytes: &replaced,
                run_from: 0,
                ends_file: true,
                front: Some(0),
            };
            let window = Window {
                raw: &raw,
                edits: &edits,
                facts: &facts,
                file: 0,
                files: 1,
                lines_before: 0,
                line: 1,
                base: 0,
            };
            let (route, split_error) = super::super::split_json_values(&raw);
            assert_eq!(split_error, None, "seed {seed}");
            let mut values = route.clone();
            let mut fault = None;
            let before = REGIONS.with(std::cell::Cell::get);
            plan_window(&window, &mut values, &mut fault);
            regions += REGIONS.with(std::cell::Cell::get) - before;
            assert_eq!((values, fault), (route, None), "seed {seed}: {bytes:?}");
        }
        assert!(regions > 200, "{regions} regions");
    }

    /// A region gives no value past what the window decides: a bare token before a document the
    /// window holds open is not handed out until the next window, which reads it (the differential's
    /// second draft of step 7 printed `true` twice), and no value before `run_from`, which an earlier
    /// window ran.
    #[test]
    fn a_region_gives_only_what_the_window_decides_7987() {
        let bytes = b"0\n{\"\xe9\":1}true\r\n{\"id\":1,\"p\":\"yyy";
        let (raw, edits, replaced) = window_of(bytes);
        let object = (2, 2 + "{\"\u{fffd}\":1}".len());
        for (run_from, route) in [(0, vec![(0, 1), object]), (2, vec![object])] {
            let facts = ModelFacts {
                file_front: 0,
                edit_bytes: &replaced,
                run_from,
                ends_file: false,
                front: Some(0),
            };
            let window = Window {
                raw: &raw,
                edits: &edits,
                facts: &facts,
                file: 0,
                files: 1,
                lines_before: 0,
                line: 1,
                base: 0,
            };
            let mut values = route.clone();
            let mut fault = None;
            let before = REGIONS.with(std::cell::Cell::get);
            plan_window(&window, &mut values, &mut fault);
            assert_eq!(
                REGIONS.with(std::cell::Cell::get),
                before + 1,
                "a region was read"
            );
            assert_eq!((values, fault), (route, None), "run_from {run_from}");
        }
    }
}
