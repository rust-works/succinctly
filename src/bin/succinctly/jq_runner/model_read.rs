//! The materializing route of `read_inputs` through ADR-0033's reader model (#7987, step 5).
//!
//! The route reads its plain JSON input with the semi-index as it always has, and enters the model
//! only on a trigger, a fact the route already has:
//!
//! - **a byte-order mark that is not whole** ([`mark_not_whole`]): the stream's first byte is `EF`
//!   and its first three bytes are not the whole mark in one source. Known before anything is read
//!   as JSON, so the model reads the whole input, as it was read ([`read_rest`] from
//!   [`ReaderState::start`]);
//! - **invalid UTF-8** over several sources: the same, since those are never read lazily;
//! - **a fault**, the first the route met, and for a lone source that is not UTF-8 its first
//!   substituted byte too: the earliest place where the route's own reading can be wrong. The route
//!   has read every value before it, so [`entry_before`] finds the last sync point before it among
//!   them, carries the state there with [`ReaderState::skip_plain`] (it reads no byte), keeps the
//!   route's values before it, and the model reads the rest of the input.
//!
//! A file seam is not a trigger: jq's parser does not see it, and with a seam as a trigger the perf
//! guard's `split*` rows (a well-formed document cut across 2 or 16 files) failed at +15% to +375%.
//! Once entered, the model reads to the end of the input: handing back to the semi-index at the
//! next sync point is step 6's, on the default route.
//!
//! Each value is the JSON text jq accepted, decoded by the crate's own value reader; each fault is
//! jq's words, with the file and line jq names; the end of input is where jq's reader stands after
//! the call that finds it. A `Malformed BOM` leaves jq's parser in that fault for good, so the
//! input queue repeats it on every later read ([`BROKEN_MARK_STREAM`](super::BROKEN_MARK_STREAM)).

use std::rc::Rc;

use super::reader_model::{Item, PlainRun, PlainStretch, ReadMode, ReaderState};
use super::{
    count_newlines, line_seed, parse_json_stream_prefix, EofTail, EvalError, InputLocations,
    LossySource, OwnedValue, StreamOrigin, TrailingParseError, BROKEN_MARK_STREAM, NO_SOURCE,
    UNKNOWN_LINE,
};
use crate::cli::jq_reader_profile::JQ_READER;
use succinctly::jq::jv_parse::UTF8_BOM;

/// The route reads plain JSON, never `--seq`.
const NON_SEQ: ReadMode = ReadMode { seq: false };

/// Whether `sources`, as read, open with a byte-order mark that is not whole: the first source with
/// a byte starts with `EF`, and its first three bytes are not the mark. A mark cut across sources
/// is not whole in one source, so it is read by the model too (#7566's shapes).
pub(super) fn mark_not_whole<'s>(sources: impl IntoIterator<Item = &'s [u8]>) -> bool {
    sources
        .into_iter()
        .find(|source| !source.is_empty())
        .is_some_and(|source| source[0] == UTF8_BOM[0] && !source.starts_with(&UTF8_BOM))
}

/// Where the route's reads go.
pub(super) struct Sink<'a> {
    pub values: &'a mut Vec<OwnedValue>,
    pub locations: &'a mut InputLocations,
    pub errors: &'a mut Vec<(usize, TrailingParseError)>,
}

/// Reads `sources` (every input file as read, in order, each tagged by `tags`) with the model from
/// `state` to the end of the input, into `sink`: each value with its place, each fault ahead of the
/// values after it, and the end of input as `sink.locations.eof_tail`. A value whose text ends at
/// or before `drop_through` (an offset in the sources read as one stream) is not added: a lazy
/// prefix hands it out.
pub(super) fn read_rest(
    sources: &[&[u8]],
    tags: &[usize],
    state: ReaderState,
    drop_through: usize,
    sink: Sink<'_>,
) {
    let Sink {
        values,
        locations,
        errors,
    } = sink;
    // #7472: a test seeds the count every source starts at, in place of 4 GiB of input.
    let seed = line_seed() as u64;
    let starts: Vec<usize> = sources
        .iter()
        .scan(0usize, |at, source| {
            let start = *at;
            *at += source.len();
            Some(start)
        })
        .collect();
    let mut last = (NO_SOURCE, UNKNOWN_LINE);
    let stuck = state.read_to_end(sources, &JQ_READER, NON_SEQ, |read| {
        let (source, line) = match read.at.file {
            Some(file) => (
                u32::try_from(tags[file]).unwrap_or(u32::MAX),
                read.at.line as u64 + seed,
            ),
            None => (NO_SOURCE, UNKNOWN_LINE),
        };
        last = (source, line);
        let fault = |error| TrailingParseError {
            error,
            source,
            line,
            seq_warning: false,
            as_parse_error: false,
        };
        match read.item {
            Item::Value(span) => {
                if span.end <= drop_through {
                    return;
                }
                match decode(&span_bytes(sources, &starts, span.start, span.end)) {
                    Ok(value) => {
                        values.push(value);
                        locations.per_value.push(source, line);
                    }
                    // A text jq's parser accepts that the crate's value reader refuses: said as the
                    // reader's error where the value would have been, never dropped silently.
                    Err(error) => errors.push((values.len(), fault(error))),
                }
            }
            Item::Fault(found) => {
                errors.push((values.len(), fault(EvalError::new(found.message()))))
            }
            Item::End => {
                locations.eof_tail = Some(EofTail {
                    newlines: if read.at.file.is_some() { line } else { 0 },
                    unknown: read.at.file.is_none(),
                });
            }
        }
    });
    if stuck {
        // jq's parser never leaves `Malformed BOM`, so no read finds the end of the input; the queue
        // repeats the fault at the place the first one named.
        BROKEN_MARK_STREAM.with(|flag| flag.set(true));
        locations.eof_tail = Some(EofTail {
            newlines: if last.0 == NO_SOURCE { 0 } else { last.1 },
            unknown: last.0 == NO_SOURCE,
        });
    }
}

/// [`read_rest`] from the start of the stream: the whole input through the model.
pub(super) fn read_whole(sources: &[&[u8]], tags: &[usize], sink: Sink<'_>) {
    read_rest(sources, tags, ReaderState::start(NON_SEQ), 0, sink);
}

/// The bytes of `start..end` of the sources read as one stream (`starts` is where each begins),
/// borrowed when they are in one source.
fn span_bytes<'s>(
    sources: &[&'s [u8]],
    starts: &[usize],
    start: usize,
    end: usize,
) -> std::borrow::Cow<'s, [u8]> {
    let first = starts.partition_point(|&at| at <= start) - 1;
    let local = start - starts[first];
    if local + (end - start) <= sources[first].len() {
        return std::borrow::Cow::Borrowed(&sources[first][local..local + (end - start)]);
    }
    // A value that runs across a file seam, which jq's parser does not see.
    let mut bytes = Vec::with_capacity(end - start);
    for (source, &at) in sources.iter().zip(starts).skip(first) {
        if at >= end {
            break;
        }
        let from = start.saturating_sub(at);
        let to = (end - at).min(source.len());
        bytes.extend_from_slice(&source[from..to]);
    }
    std::borrow::Cow::Owned(bytes)
}

/// A value's text as the crate's value reader reads it, which is how the route reads every value:
/// a string's invalid UTF-8 is jq's substitution of it (`jvp_utf8_next`).
fn decode(text: &[u8]) -> Result<OwnedValue, EvalError> {
    let substituted;
    let text = match std::str::from_utf8(text) {
        Ok(text) => text,
        Err(_) => {
            substituted = succinctly::jq::utf8_document::substitute_invalid_utf8_jq_document(text);
            substituted.as_str()
        }
    };
    match parse_json_stream_prefix(text, Some(StreamOrigin::ALONE)) {
        (mut values, None) if values.len() == 1 => Ok(values.pop().expect("one value")),
        (_, Some(error)) => Err(error),
        _ => Err(EvalError::new("Invalid JSON text")), // patchcov: coverage tolerate-line reason="defensive: a span the model hands back is one value's text, which the reader reads as one value"
    }
}

/// What the route read of one source before it met a trigger, for [`entry_before`].
pub(super) struct EagerFile {
    /// The text the route read: the source after the stitch moved a cut token onto or off it and
    /// the mark was taken off its front (and, for a source that is not UTF-8, the substitution).
    pub text: Rc<String>,
    /// How many of its first bytes came from earlier files (`Seam::base`).
    pub base: usize,
    /// Where each value the route read in it ends, in order; the first `lazy` of them are a lazy
    /// prefix's, which the route's own values do not hold.
    pub ends: Vec<usize>,
    /// How many of `ends` a lazy prefix hands out.
    pub lazy: usize,
    /// How many values the route held when it came to the source.
    pub values_before: usize,
    /// The earliest offset of the text where the route's reading can be wrong: where the value it
    /// met its first fault in starts, or the first byte the substitution changed.
    pub trigger: Option<usize>,
}

/// Where the model is entered after the route met a trigger: the state at the last sync point
/// before it, how many of the route's values come before that point, and the offset through which
/// the model's values are a lazy prefix's.
pub(super) struct Entry {
    pub state: ReaderState,
    pub keep: usize,
    pub drop_through: usize,
}

/// The sources as read, rebuilt from what the route read of them: the stitch only moves bytes
/// across a seam, so the texts in order are the stream after the mark, and `lens` (each source's
/// length as read) cuts it back into sources. `None` when they do not add up, which would leave the
/// route's own answer standing.
pub(super) fn original_stream(
    files: &[EagerFile],
    lossy: &[Option<LossySource>],
    cuts: &[usize],
    lens: &[usize],
) -> Option<Vec<u8>> {
    let total: usize = lens.iter().sum();
    let mut stream = Vec::with_capacity(total);
    for (at, file) in files.iter().enumerate() {
        let cut = cuts.get(at).copied().unwrap_or(0);
        stream.extend_from_slice(&UTF8_BOM[..cut.min(UTF8_BOM.len())]);
        match lossy.get(at).and_then(Option::as_ref) {
            Some(lossy) => stream.extend_from_slice(&lossy.original),
            None => stream.extend_from_slice(file.text.as_bytes()),
        }
    }
    (stream.len() == total && files.len() == lens.len()).then_some(stream)
}

/// Where the model is entered for the first trigger among `files` (the route's view of each of
/// `sources`, the sources as read): the last sync point before it, a place where jq's parser is
/// between top-level values at the start of one of its reads. Those are the start of the stream,
/// the start of a file nothing was carried onto, and the byte after a newline that no value the
/// route read holds. `cuts` is the mark taken off each source's front. `None` when no source has a
/// trigger.
pub(super) fn entry_before(
    files: &[EagerFile],
    sources: &[&[u8]],
    cuts: &[usize],
) -> Option<Entry> {
    let first = files.iter().position(|file| file.trigger.is_some())?;
    // The lazy prefix of a lone source: the model's values through its end are its.
    let drop_through = files
        .first()
        .filter(|file| file.lazy > 0)
        .map_or(0, |file| {
            cuts.first().copied().unwrap_or(0) + file.ends[file.lazy - 1]
        });
    let mut at = None;
    for j in (0..=first).rev() {
        let file = &files[j];
        let bound = if j == first {
            file.trigger.unwrap_or(0)
        } else {
            file.text.len()
        };
        if let Some(newline) = newline_between_values(file, bound) {
            // The source's bytes up to and through that newline.
            let len = cuts.get(j).copied().unwrap_or(0) + newline + 1 - file.base;
            let read = file.ends.partition_point(|&end| end <= newline);
            at = Some((j, len, file.values_before + read.saturating_sub(file.lazy)));
            break;
        }
        if file.base == 0 {
            at = Some((j, 0, file.values_before));
            break;
        }
    }
    let (file, len, keep) = at.unwrap_or((0, 0, 0));
    let mut runs: Vec<PlainRun> = (0..file).map(|j| plain_run(j, sources[j], true)).collect();
    if len > 0 {
        runs.push(plain_run(
            file,
            &sources[file][..len],
            len == sources[file].len(),
        ));
    }
    let start = ReaderState::start(NON_SEQ);
    if runs.is_empty() {
        return Some(Entry {
            state: start,
            keep,
            drop_through,
        });
    }
    let stretch = PlainStretch {
        runs: &runs,
        files: sources.len(),
        value_in_final_buffer: false,
    };
    Some(match start.skip_plain(&stretch, &JQ_READER, NON_SEQ) {
        Some(state) => Entry {
            state,
            keep,
            drop_through,
        },
        // patchcov: coverage tolerate reason="defensive: every stretch handed over ends after a newline or at a file's end, between values, which skip_plain takes"
        None => Entry {
            state: start,
            keep: 0,
            drop_through,
        },
        // patchcov: coverage end
    })
}

/// The last newline in `file`'s own bytes before `bound` (offsets in its text) that no value the
/// route read in it holds.
fn newline_between_values(file: &EagerFile, mut bound: usize) -> Option<usize> {
    let text = file.text.as_bytes();
    while bound > file.base {
        let newline = file.base + text[file.base..bound].iter().rposition(|&b| b == b'\n')?;
        // The first value that ends after the newline, and where it starts.
        let after = file.ends.partition_point(|&end| end <= newline);
        let Some(&end) = file.ends.get(after) else {
            return Some(newline);
        };
        let from = after.checked_sub(1).map_or(0, |before| file.ends[before]);
        let start = from
            + text[from..end]
                .iter()
                .position(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
                .unwrap_or(end - from);
        if start > newline {
            return Some(newline);
        }
        bound = start;
    }
    None
}

/// What [`ReaderState::skip_plain`] is told of `bytes`, the stretch's share of source `file`.
fn plain_run(file: usize, bytes: &[u8], to_end: bool) -> PlainRun {
    let newlines = count_newlines(bytes);
    let tail = bytes
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(bytes.len(), |last| bytes.len() - last - 1);
    PlainRun {
        file,
        len: bytes.len(),
        newlines,
        tail,
        to_end,
        bom: bytes.starts_with(&UTF8_BOM),
    }
}
