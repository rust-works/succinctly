//! Real jq's `--seq` reader, modelled closely enough to reproduce its
//! `jq: ignoring parse error: ...` stderr diagnostics (#1723).
//!
//! #1525 implemented one of jq's message templates -- the one for content
//! with no RS byte anywhere, where jq's reader never syncs onto anything
//! ([`super::jq_runner::seq_no_rs_byte_warning`]). Every *other* template
//! needs to know **where jq's own reader gave up**, which is what this
//! module answers.
//!
//! **Why a state machine rather than a classification table.** Several
//! earlier attempts tried to map "what is wrong with this record" onto
//! jq's wording and found no consistent rule -- the same malformed record
//! draws different messages depending on whether jq hit real EOF, an RS
//! byte, or an ordinary byte first. There is no table because the message
//! is not a property of the record; it is a property of the *moment of
//! detection*. Modelling jq's `scan()` loop reproduces all of them, and
//! the message is then composed from three facts:
//!
//! | detected on | rendering |
//! |---|---|
//! | an ordinary byte | `{category} at line L, column C (need RS to resync)` |
//! | real EOF | `{category} at EOF at line L, column C` |
//! | an RS byte | `Truncated value` (or `Potentially truncated top-level numeric value`) |
//!
//! Positions are jq's cursor at the moment of detection: 0-based columns
//! counting **raw bytes**, never reset per record or per input file.
//!
//! **`(need RS to resync)` is wording, not behaviour.** jq's own
//! `jv_parse.c` sets `JV_PARSER_WAITING_FOR_RS` and *then* calls
//! `parser_reset()`, which assigns `JV_PARSER_NORMAL` straight back over
//! it -- so the resync the message promises never happens and parsing
//! resumes at the very next byte. That is why one record can produce
//! several warnings (`\x1enot valid json\n` produces three) and why
//! `\x1e} 5\n` still prints `5`. The bug is reproduced here deliberately,
//! per ADR-0018's bug-for-bug default; the ordering of those two lines is
//! the whole reason this module resets instead of skipping.
//!
//! Derived by reading jq 1.7.1's `src/jv_parse.c` (`scan`, `parse_token`,
//! `check_literal`, `found_string`, `jv_parser_next`) and verified
//! case-by-case against the pinned `/usr/bin/jq`. The source enumerates
//! the categories; the binary decides what is actually emitted.
//!
//! Besides diagnostics, the same state machine reports the byte ranges of
//! values jq hands to its caller.  Keeping those two paths together matters:
//! a value which looks complete to a token scanner can still be discarded by
//! the scan call that discovers a malformed suffix (notably `1,2`).
//!
//! **Every byte of every file is read, as in jq 1.8.2 (#7718).** jq 1.7.1's
//! caller (`jq_util_input_next_input`, `src/util.c`) took the refill that
//! reached the end of the last file for the end of the stream (`is_last`), so
//! an empty record that was the first thing that call scanned (`jv_parser_next`'s
//! message-less return) ended the whole stream silently, and the records
//! behind it were never read (#2998): `\x1e1\n\x1etrue` printed only `1`. jq
//! 1.8.2 reports the end only once the last file is closed, one refill later
//! (jqlang/jq#3279, commit 4e3088f), so the buffer that holds the last bytes is
//! never the final one: the final buffer is always the empty one read after
//! them. The reader follows 1.8.2 there -- an empty record never ends the
//! stream, under a malformed BOM that empty buffer's `parser_reset` wipes what
//! the EOF branch would report ([`Reader::suppress_eof_warning`]), and under `-s`
//! the EOF branch reports in the call that reads the stream, before the array
//! is handed on. What 1.7.1 answers for the *position* after the last input
//! is kept (#7607): [`final_buffer_start`] still places 1.7.1's final buffer for
//! that.
//!
//! **1.7.1's `is_last` local decides `--seq -s`'s error location (#2947/#3003).**
//! Under `-s` nothing is dropped, but whether a later runtime error can
//! name `file:line` or must say `<unknown>` is again a question about
//! *which call* did what: the slurped array keeps its position iff the
//! call that performed the terminal refill returns it without first
//! returning an error -- which an error in the final buffer prevents, and
//! which the EOF branch prevents unless the buffer's last byte completed a
//! value and so ended the call before that branch ran. See
//! [`SeqStreamWalk::slurp_position_lost`] for the full derivation.

use crate::cli::front_matter::UTF8_BOM;
use crate::cli::jq_reader_profile::JQ_READER;
use crate::cli::jq_runner::reader_model::{Fgets, JQ_LINE_READS};

// #7718: the reader reads every record, as jq 1.8.2 does (the module's comment above). There is no
// code for jq 1.7.1's end at an empty record, so the profile may not ask for it.
const _: () = assert!(JQ_READER.seq_reads_every_record);

const ASCII_RS: u8 = 0x1e;

/// jq 1.7.1's `main` line for a `--seq` parse error it skips
/// (`fprintf(stderr, "jq: ignoring parse error: %s\n", ...)`), for the
/// parse error `body`. The one spelling of it: the reader's warnings, #1525's
/// no-RS-byte template and the driver's deferred warning (#3201) all use it.
pub(crate) fn ignored_parse_error(body: &str) -> String {
    format!("jq: ignoring parse error: {body}")
}

/// jq's `fgets` chunks over one source, as their lengths in order: each
/// ends just after a newline or after [`JQ_READER.read_len`](JQ_READER) bytes, whichever
/// comes first. The rule is the reader model's [`Fgets`] (#7987), shared by
/// [`final_buffer_start`] and [`refill_boundaries`] (#3200).
pub(crate) fn fgets_chunk_lens(raw: &[u8]) -> impl Iterator<Item = usize> + '_ {
    fgets_reads(raw).map(|read| read.len)
}

/// [`fgets_chunk_lens`] with whether each chunk ends at a newline: the reader model's [`Fgets`]
/// reads of one source, in order.
pub(crate) fn fgets_reads(raw: &[u8]) -> impl Iterator<Item = Fgets> + '_ {
    let mut pos = 0usize;
    std::iter::from_fn(move || {
        let read = Fgets::of(&raw[pos..], JQ_READER.read_len)?;
        pos += read.len;
        Some(read)
    })
}

/// The length of the `fgets` chunk that starts at `pos` in `raw`, or `None` at its end: one step
/// of [`fgets_chunk_lens`], for a reader that finds the chunks as it goes (#5498).
pub(crate) fn fgets_chunk_len_at(raw: &[u8], pos: usize) -> Option<usize> {
    Fgets::of(raw.get(pos..)?, JQ_READER.read_len).map(|read| read.len)
}

/// `bytes` read as a C string, up to its first NUL: what jq's number parser reads of a token
/// (`check_literal` hands `tokenbuf` to `jv_number_with_literal`, which takes its `strlen`). A bare
/// token holds no NUL (#7966), so only an open string's bytes an RS asks about can hold one. Shared
/// by both ports of `check_literal` (#7340).
pub(crate) fn c_string(bytes: &[u8]) -> &[u8] {
    &bytes[..find_byte(0, bytes).unwrap_or(bytes.len())]
}

/// The absolute offsets, ascending, at which jq starts a `jv_parser_next`
/// that the reader's own byte loop does not already reset at: each source
/// after the first, since jq refills one file at a time (#3002), and, with
/// `size_limit`, each refill after a full [`JQ_READER.read_len`](JQ_READER)-byte chunk that
/// did not end on a newline (#3200). A chunk ending on a newline needs no
/// entry, since the reader resets as it scans the newline. A chunk ending at
/// its source's own end needs none either: the next source's start is
/// already here, and at the stream's end it is the empty final buffer,
/// which [`Reader::suppress_eof_warning`] covers.
fn refill_boundaries(raw_bytes: &[(Option<usize>, Vec<u8>)], size_limit: bool) -> Vec<usize> {
    let mut boundaries = Vec::new();
    let ends = source_ends(raw_bytes);
    for (index, (_, raw)) in raw_bytes.iter().enumerate() {
        let source_start = ends[index] - raw.len();
        if index > 0 {
            boundaries.push(source_start);
        }
        if size_limit {
            let mut chunk_end = 0usize;
            for read in fgets_reads(raw) {
                chunk_end += read.len;
                if read.fills(JQ_READER.read_len) && !read.newline && chunk_end < raw.len() {
                    boundaries.push(source_start + chunk_end);
                }
            }
        }
    }
    boundaries
}

/// [`Reader::yielded_at`]'s marker for a value jq's EOF branch yields.
const YIELDED_AT_EOF: usize = usize::MAX;

/// The offset [`walk_stream_collecting_at`] gives a warning of jq's EOF branch: met after every
/// byte of every file, so after every file that did not open.
pub(crate) const AT_EOF: usize = YIELDED_AT_EOF;

/// jq's `(source index, line)` for each yield offset (ascending, as
/// [`Reader::yielded_at`] records them), per jq 1.7.1's `src/util.c`:
/// `jq_util_input_read_more` names the file it is reading and bumps
/// `current_line` for every `fgets` chunk holding a newline, and a value is
/// reported with whatever those were when it was yielded (#3250).
///
/// So the file is the one holding the yielding byte -- a string or container
/// yields on its own closing byte, which is why `"x"` at the very end of one
/// file belongs to that file, not the next -- and the line counts every
/// newline up to the end of the [`fgets_chunk_lens`] chunk holding that
/// byte, since the whole chunk was read before any of it was parsed. A value
/// yielded by the EOF branch is named after the last source, with all of
/// its newlines. (jq would name a last file operand it failed to open
/// there; `get_inputs` doesn't reach this walk with one.)
fn value_locations(
    raw_bytes: &[(Option<usize>, Vec<u8>)],
    yielded_at: &[usize],
) -> Vec<(usize, usize)> {
    let ends = source_ends(raw_bytes);
    let last_source = raw_bytes.len().saturating_sub(1);
    // The source being walked, its chunks still unread, how far they have
    // been read, and the newlines in them.
    let mut source = usize::MAX;
    let mut chunks = fgets_chunk_lens(&[]);
    let mut chunk_end = 0usize;
    let mut newlines = 0usize;
    let mut out = Vec::with_capacity(yielded_at.len());
    for &at in yielded_at {
        let (index, local) = if at == YIELDED_AT_EOF {
            (last_source, usize::MAX)
        } else {
            let index = ends.partition_point(|&end| end <= at).min(last_source);
            (index, at - (ends[index] - raw_bytes[index].1.len()))
        };
        let raw: &[u8] = raw_bytes.get(index).map_or(&[], |(_, raw)| raw.as_slice());
        if index != source {
            source = index;
            chunks = fgets_chunk_lens(raw);
            chunk_end = 0;
            // #7260: a resumed stream's first source starts that many line breaks in.
            newlines = if index == 0 { seeded_line_base() } else { 0 };
        }
        // Read chunks until the one holding `local` has been read.
        while chunk_end <= local {
            let Some(len) = chunks.next() else { break };
            chunk_end += len;
            if raw[chunk_end - 1] == b'\n' {
                newlines += 1;
            }
        }
        out.push((index, newlines));
    }
    out
}

/// Where jq's `fgets`-chunked *final* buffer begins, as a raw byte offset
/// into the whole concatenated raw stream (all sources, in order).
///
/// `read_more` (`jq_util_input_read_more`, `src/util.c`) calls `fgets` on a
/// [`JQ_READER.read_len`](JQ_READER)+1-byte buffer, so a chunk ends at a newline or after
/// [`JQ_READER.read_len`](JQ_READER) bytes, whichever comes first -- and `feof` is set
/// only by an `fgets` that actually ran out of input, so a chunk ending on
/// a newline or filling the buffer exactly is followed by one more
/// (possibly empty) buffer. jq refills one *file* at a time, so chunking
/// restarts at the last source's own start; that is also what keeps an
/// empty (or newline-free) trailing source from inheriting an earlier
/// source's chunk boundary.
///
/// The positions jq 1.7.1 names after the last input hang off "does offset X fall in jq's last
/// buffer" (#7607): `-s`'s EOF-location answer ([`SeqStreamWalk::slurp_position_lost`]) and the
/// `<unknown>` of an EOF report raised after the last value ([`Reader::fault_position_lost`]). What
/// is read no longer does: jq 1.8.2 reads one more, empty, buffer after this one, and the reader
/// follows it (#7718; 1.7.1 ended a non-slurp stream at an empty record that was this buffer's
/// first event, #2998). Returns the stream's own total length when the final buffer is
/// empty, so a caller comparing `offset >= final_buffer_start` correctly
/// finds nothing: no real offset ever reaches the stream's own length --
/// and `final_buffer_start == total` is how that emptiness is read back.
///
/// `total` is the stream's byte length, which [`walk_stream`] already
/// has from its own `boundaries` walk; a second `Vec`-of-lengths reduction
/// over the same sources on every call was pure waste.
fn final_buffer_start(raw_bytes: &[(Option<usize>, Vec<u8>)], total: usize) -> usize {
    let last: &[u8] = raw_bytes.last().map_or(&[], |(_, raw)| raw.as_slice());
    let last_start = total - last.len();
    let mut chunk_start = 0usize;
    let mut final_chunk = None;
    for len in fgets_chunk_lens(last) {
        final_chunk = Some((chunk_start, len));
        chunk_start += len;
    }
    let final_chunk = final_chunk.map(|(start, len)| (start, len, last[start + len - 1] == b'\n'));
    last_start + final_buffer_start_after(final_chunk, last.len())
}

/// [`final_buffer_start`] for a stream of `total` bytes whose last source's last chunk holding data
/// is `final_chunk` (`(start, len)`, and whether it ends at a newline), found as the chunks were.
fn final_buffer_start_after(final_chunk: Option<(usize, usize, bool)>, total: usize) -> usize {
    match final_chunk {
        // An empty last source is itself the (empty) final buffer.
        None => total,
        // The last chunk holding data. An `fgets` that stopped on a
        // newline or on the size limit has not reached EOF yet, so an
        // *empty* buffer follows and that one is final; otherwise this
        // chunk is.
        Some((start, len, newline)) => {
            if (Fgets { len, newline }).meets_end(JQ_READER.read_len) {
                start
            } else {
                total
            }
        }
    }
}

/// What jq's `jv_parser_set_buf` consumes as a leading BOM, over all
/// sources concatenated.
///
/// jq consumes the matching *prefix* byte by byte, so a partial BOM is
/// eaten even though it never completes one. Those bytes never reach the
/// scanner, so they never advance a column either -- getting this wrong
/// shifts every position in the stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BomPrefix {
    /// Bytes jq swallowed before parsing began.
    consumed: usize,
    /// A prefix that started a BOM and then contradicted it. jq flags this
    /// (`bom_strip_position = 0xff`) and thereafter re-runs `parser_reset`
    /// at the top of every `jv_parser_next` -- which, through the same
    /// clobber described above, leaves the parser in `Normal` rather than
    /// `WaitingForRs`. So a malformed BOM makes jq read the bytes before
    /// the first RS as a record, where it would normally discard them.
    pub(crate) malformed: bool,
}

/// Every byte takes part, a NUL included: jq 1.7.1 never saw the bytes from a NUL to the end of an
/// `fgets` read with no newline, but the reader does not reproduce that (#7966), so `EF BB` then NUL
/// is a malformed BOM. `consumed` is the offset the scanner starts at.
pub(crate) fn bom_prefix(raw_bytes: &[(Option<usize>, Vec<u8>)]) -> BomPrefix {
    // How many BOM bytes have matched, and where the source being read starts (an absolute offset).
    let mut matched = 0;
    let mut source_start = 0usize;
    for (_, raw) in raw_bytes {
        for (index, &byte) in raw.iter().enumerate() {
            if matched == UTF8_BOM.len() || byte != UTF8_BOM[matched] {
                // A mismatch at offset 0 just means "no BOM here"; one after a
                // partial match is the malformed case. Running out of input
                // mid-BOM is neither -- jq is still waiting for the rest.
                return BomPrefix {
                    consumed: source_start + index,
                    malformed: matched > 0 && matched < UTF8_BOM.len(),
                };
            }
            matched += 1;
        }
        source_start += raw.len();
    }
    BomPrefix {
        consumed: source_start,
        malformed: false,
    }
}

/// Each source's exclusive end offset in the concatenation of all of
/// them, in order -- which is also where the next source starts. The one
/// definition of those offsets for the walk's boundaries, its value
/// locations ([`value_locations`]) and `-R`'s line remap.
pub(crate) fn source_ends<S: AsRef<[u8]>>(sources: &[(Option<usize>, S)]) -> Vec<usize> {
    sources
        .iter()
        .scan(0usize, |end, (_, raw)| {
            *end += raw.as_ref().len();
            Some(*end)
        })
        .collect()
}

/// Every byte of the input stream, as jq's scanner sees it: all sources
/// concatenated, with [`bom_prefix`]'s bytes already removed.
///
/// The byte-at-a-time statement of what [`NoRsScan`] counts, which the tests hold it to (#7263,
/// #7260).
#[cfg(test)]
pub(crate) fn stream_bytes(
    raw_bytes: &[(Option<usize>, Vec<u8>)],
) -> impl Iterator<Item = u8> + '_ {
    raw_bytes
        .iter()
        .flat_map(|(_, raw)| raw.iter().copied())
        .skip(bom_prefix(raw_bytes).consumed)
}

/// Move `line` and `column` past `span` as jq's reader counts them, a byte at a time: a newline
/// starts the next line at column 0, and every other byte is a column (#7263). `span` holds no
/// RS byte; the callers stop at one. One definition for the walk's discarded bytes and
/// [`super::jq_runner`]'s no-RS-byte position, which must agree.
pub(crate) fn advance_position(span: &[u8], line: &mut usize, column: &mut usize) {
    match rfind_byte(b'\n', span) {
        Some(last) => {
            *line += span.iter().filter(|&&b| b == b'\n').count();
            *column = span.len() - last - 1;
        }
        None => *column += span.len(),
    }
}

/// The bytes [`find_byte`] and [`rfind_byte`] test at once: a block with no match costs one
/// comparison per byte with no branch (an OR of bytes, not a short-circuiting `||`), which the
/// compiler vectorizes.
const SEARCH_BLOCK: usize = 64;

/// The bytes [`find_byte`] reads one at a time before it takes blocks: a match this near costs no
/// block, which keeps a stream of short lines (one `fgets` chunk each) as cheap as it was.
const SEARCH_HEAD: usize = 16;

/// The window [`SeqStreamWalker`] probes for a newline (#7775): a run of
/// [`JQ_READER.read_len`](JQ_READER) bytes with no newline holds a whole window of this size
/// wherever it starts, so a stretch whose every window holds a newline has none, and its `fgets`
/// chunks are its lines.
const LINE_PROBE: usize = JQ_READER.read_len.div_ceil(2);

/// The offset of the first `needle` in `hay` (#7263): the first [`SEARCH_HEAD`] bytes one at a
/// time, then a [`SEARCH_BLOCK`] at a time, where only the block that holds it is searched a byte
/// at a time.
pub(crate) fn find_byte(needle: u8, hay: &[u8]) -> Option<usize> {
    let head = hay.len().min(SEARCH_HEAD);
    if let Some(at) = hay[..head].iter().position(|&b| b == needle) {
        return Some(at);
    }
    let mut start = head;
    for block in hay[head..].chunks(SEARCH_BLOCK) {
        if block
            .iter()
            .fold(0u8, |seen, &b| seen | u8::from(b == needle))
            != 0
        {
            return block.iter().position(|&b| b == needle).map(|at| start + at);
        }
        start += block.len();
    }
    None
}

/// jq's reader over a `--seq` stream before it has seen an RS (#1525's template), read a piece at a
/// time (#7260): the line and column it stands at, which is where `Unfinished abandoned text at EOF`
/// is reported when the stream ends without one. jq's parser counts the bytes it is handed while it
/// waits for an RS and does nothing else with them, so this is all it holds of them, and a stream with
/// no RS need not be held to be answered.
///
/// The bytes are fed a source at a time, each in pieces that start where one of jq's `fgets` reads
/// starts ([`JQ_READER.read_len`](JQ_READER)); [`NoRsScan::feed`] says how far it read, which is
/// always such a start, so the caller keeps the rest in front of the next piece. Every byte of a
/// read is counted, the bytes after a NUL in a read with no newline included (#7966); a BOM, whole
/// or a prefix of one, is consumed at the stream's front and not counted ([`bom_prefix`]). The one
/// definition of the position: [`super::jq_runner`]'s whole-stream answer feeds every source whole,
/// and the windowed `--seq` route feeds a file it does not hold.
pub(crate) struct NoRsScan {
    line: usize,
    column: usize,
    /// How many bytes of a BOM the stream's front has matched, while it may still be one.
    bom_matched: usize,
    bom_open: bool,
}

impl Default for NoRsScan {
    fn default() -> Self {
        Self {
            line: 1,
            column: 0,
            bom_matched: 0,
            bom_open: true,
        }
    }
}

impl NoRsScan {
    /// Reads `bytes`, the next bytes of a source from the start of one of jq's reads, through the last
    /// read that ends in them (all of them when `end`: they run to the source's end, where jq's last
    /// read of it ends). The bytes read, or `None` once jq's reader would stop waiting for an RS: it
    /// sees one, or the stream opens with a malformed BOM, which leaves its parser reading
    /// ([`BomPrefix::malformed`]). Each source starts a read of its own.
    pub(crate) fn feed(&mut self, bytes: &[u8], end: bool) -> Option<usize> {
        let upto = if end {
            bytes.len()
        } else {
            // Reads start after a newline and then every JQ_READER.read_len bytes along a line;
            // `bytes` starts at one, so the line in it after its last newline starts at one too.
            let line_start = rfind_byte(b'\n', bytes).map_or(0, |at| at + 1);
            JQ_LINE_READS.start_before(line_start, bytes.len())
        };
        self.visible(&bytes[..upto])?;
        Some(upto)
    }

    /// Bytes jq's parser is handed, in order.
    fn visible(&mut self, mut bytes: &[u8]) -> Option<()> {
        while self.bom_open {
            let Some(&byte) = bytes.first() else {
                return Some(());
            };
            if self.bom_matched == UTF8_BOM.len() || byte != UTF8_BOM[self.bom_matched] {
                self.bom_open = false;
                if self.bom_matched > 0 && self.bom_matched < UTF8_BOM.len() {
                    return None;
                }
            } else {
                self.bom_matched += 1;
                bytes = &bytes[1..];
            }
        }
        if find_byte(ASCII_RS, bytes).is_some() {
            return None;
        }
        advance_position(bytes, &mut self.line, &mut self.column);
        Some(())
    }

    /// The 1-based line and the column jq's reader stands at after what was fed.
    pub(crate) fn position(&self) -> (usize, usize) {
        (self.line, self.column)
    }
}

/// The offset of the last `needle` in `hay`, as [`find_byte`] finds the first.
fn rfind_byte(needle: u8, hay: &[u8]) -> Option<usize> {
    let mut end = hay.len();
    for block in hay.rchunks(SEARCH_BLOCK) {
        let start = end - block.len();
        if block
            .iter()
            .fold(0u8, |seen, &b| seen | u8::from(b == needle))
            != 0
        {
            return block
                .iter()
                .rposition(|&b| b == needle)
                .map(|at| start + at);
        }
        end = start;
    }
    None
}

/// jq's own reader over this `--seq` stream's raw bytes: the one walk that
/// decides every `jq: ignoring parse error: ...` line (handed to `emit`, in
/// order), the values jq yields ([`SeqStreamWalk::values`]) and `-s`'s EOF
/// location ([`SeqStreamWalk::slurp_position_lost`]). A caller that wants
/// only the values or the location passes a no-op `emit`.
///
/// A sink rather than a `Vec`: an adversarial stream can warn once per
/// byte, and collecting those first cost ~140x the input in peak RSS
/// (2 MB of `}` measured at 286 MB, against jq's 2.4 MB). Streaming them
/// keeps the reader flat.
///
/// `slurp` decides one position: the `<unknown>` of an EOF report raised after
/// the last value, which only a run without `-s` can name ([`Reader::fault_position_lost`]).
/// The walk's other answer, [`SeqStreamWalk::slurp_position_lost`], is
/// computed either way and only read under `-s`. (jq 1.7.1's non-slurp driver
/// could also end the stream at an empty record in its final buffer, #2998;
/// jq 1.8.2's does not, and the walk follows it, #7718.)
pub(crate) fn walk_stream(
    raw_bytes: &[(Option<usize>, Vec<u8>)],
    slurp: bool,
    emit: &mut dyn FnMut(&str),
) -> SeqStreamWalk {
    walk_stream_collecting(raw_bytes, slurp, false, Tail::Opened, emit)
}

/// [`walk_stream`], also recording each diagnostic as a [`SeqFault`] when `collect_faults` is
/// set (#5640): `-n`'s `input`/`inputs` raise what a plain run only warns about, and need the
/// message with where the reader stood. Off by default, because a stream can warn once per
/// byte and the faults would be held in memory (see the note on [`walk_stream`]).
pub(crate) fn walk_stream_collecting(
    raw_bytes: &[(Option<usize>, Vec<u8>)],
    slurp: bool,
    collect_faults: bool,
    tail: Tail,
    emit: &mut dyn FnMut(&str),
) -> SeqStreamWalk {
    walk_stream_placing(
        raw_bytes,
        slurp,
        collect_faults,
        true,
        tail,
        &mut |warning, _| {
            emit(warning);
        },
    )
}

/// [`walk_stream_collecting`], handing `emit` with each warning the absolute offset of the byte
/// jq's reader met it on, or [`AT_EOF`] for the EOF branch's report (#6589): `--seq -s` places
/// each warning among the open failures of the files by where jq's reader stood.
pub(crate) fn walk_stream_collecting_at(
    raw_bytes: &[(Option<usize>, Vec<u8>)],
    slurp: bool,
    collect_faults: bool,
    tail: Tail,
    emit: &mut dyn FnMut(&str, usize),
) -> SeqStreamWalk {
    walk_stream_placing(raw_bytes, slurp, collect_faults, true, tail, emit)
}

/// [`walk_stream`] for a caller that wants the values and the verdicts but not where jq's reader
/// stood at each one (#5674): `locations` comes back empty, which spares the pass over the
/// stream's `fgets` chunks that [`value_locations`] makes.
pub(crate) fn walk_stream_values(
    raw_bytes: &[(Option<usize>, Vec<u8>)],
    slurp: bool,
    emit: &mut dyn FnMut(&str),
) -> SeqStreamWalk {
    walk_stream_placing(
        raw_bytes,
        slurp,
        false,
        false,
        Tail::Opened,
        &mut |warning, _| emit(warning),
    )
}

/// [`walk_stream_values`] for a non-slurped stream whose caller may hand the walk on to the reader
/// that places values and queues faults (#6620): it records faults when `collect_faults` is set, as
/// [`walk_stream_collecting`] does, and leaves the values unplaced for [`SeqStreamWalk::place_values`].
/// A stream with one source that opened, so the tail is [`Tail::Opened`].
pub(crate) fn walk_stream_keeping(
    raw_bytes: &[(Option<usize>, Vec<u8>)],
    collect_faults: bool,
    emit: &mut dyn FnMut(&str),
) -> SeqStreamWalk {
    walk_stream_placing(
        raw_bytes,
        false,
        collect_faults,
        false,
        Tail::Opened,
        &mut |warning, _| emit(warning),
    )
}

/// The guards of [`plain_record_sequence`], as data so that a test can drop one and show the
/// check unsound without it (#6501). Each closes one way for a stream of valid records to still
/// not be clean; the shipped check has all three ([`PlainGuards::ALL`]).
#[derive(Clone, Copy, Debug)]
pub(crate) struct PlainGuards {
    /// The stream starts on an RS: bytes before the first RS are discarded by jq's reader without a
    /// word (`WaitingForRs`), so `false\n` is no value at all.
    pub(crate) starts_on_rs: bool,
    /// The stream ends on a newline: a last record with none after it ends in the EOF branch, which
    /// warns on a number with nothing after it (`\x1e12.5`), and jq 1.7.1 dropped such a record
    /// (#2998, which jq 1.8.2 and the reader no longer do, #7718). The record guard below rejects
    /// the same streams, but only after the check has read every record before the last: this one
    /// is a precheck that fails them first.
    pub(crate) ends_on_newline: bool,
    /// Every record ends on a newline: an RS straight after a number or a literal is a truncation
    /// warning (`\x1e1\x1e`).
    pub(crate) record_ends_on_newline: bool,
}

impl PlainGuards {
    /// What the runner asks.
    pub(crate) const ALL: Self = Self {
        starts_on_rs: true,
        ends_on_newline: true,
        record_ends_on_newline: true,
    };
}

/// Whether `raw` is RFC 7464's own framing, every record `RS value LF` with `value` one RFC 8259
/// document (#6501). One-way: `true` claims [`walk_stream_values`] finds the stream clean, one
/// value per record and every RS between them; `false` claims nothing, and the walk decides.
///
/// Sound because the strict validator accepts a subset of what jq's parser does (every RFC number,
/// narrower strings, literals and structure, and a depth limit of 128 under jq's), so a record that
/// passes draws no diagnostic, and the three [`PlainGuards`] close the ways valid records can still
/// leave the stream unclean. The walk is a byte-at-a-time model of jq's reader (about 200 MB/s);
/// the validator runs at about 1 GB/s. The caller's UTF-8 and byte-order-mark gate stays in front.
///
/// The last record is read first: a stream that fails does so most often at its end (a truncated
/// tail, a record that is not RFC 8259), and the check then costs one record, not a pass over every
/// record before it. The verdict does not depend on the order.
///
/// Not complete on purpose: a stream with a blank line between records, a record that is not
/// RFC 8259 (`nan`, a leading zero) or one jq reads with a warning keeps the walk.
pub(crate) fn plain_record_sequence(raw: &[u8], guards: PlainGuards) -> bool {
    if guards.starts_on_rs && raw.first() != Some(&ASCII_RS) {
        return false;
    }
    if guards.ends_on_newline && raw.last() != Some(&b'\n') {
        return false;
    }
    // The record after the last RS (the whole stream when it holds none).
    let last = raw.rsplit(|&b| b == ASCII_RS).next().unwrap_or(raw);
    if !PlainRecords::record_ok(last, guards) {
        return false;
    }
    PlainRecords::new(guards).finish_with(raw)
}

/// [`plain_record_sequence`] over a stream fed in pieces (#7082): the same verdict for the same
/// bytes however they are cut, which a test holds it to, with the memory of the biggest record
/// and not of the stream. It is the only definition of the framing: the whole-slice function above
/// is this checker run over one piece.
pub(crate) struct PlainRecords {
    guards: PlainGuards,
    /// The bytes of the record in progress that earlier pieces held.
    record: Vec<u8>,
    /// The first byte of the stream has been seen.
    started: bool,
    /// An RS has been seen: the record in progress is the one it opened.
    opened: bool,
    /// How many records were checked and found plain.
    records: usize,
    /// The last byte fed.
    last: Option<u8>,
}

impl PlainRecords {
    pub(crate) fn new(guards: PlainGuards) -> Self {
        Self {
            guards,
            record: Vec::new(),
            started: false,
            opened: false,
            records: 0,
            last: None,
        }
    }

    /// One record's verdict: ends on a newline when the guard says so, and is one RFC 8259 document.
    /// The validator takes only valid UTF-8 (in a string, and nothing but ASCII outside one), so a
    /// stream of records that pass is valid UTF-8 whole: an RS is a character of its own, which no
    /// multi-byte sequence holds. A test holds the checker to that.
    pub(crate) fn record_ok(record: &[u8], guards: PlainGuards) -> bool {
        (!guards.record_ends_on_newline || record.last() == Some(&b'\n'))
            && succinctly::json::validate::validate(record).is_ok()
    }

    /// How many bytes of the record in progress earlier pieces held.
    #[cfg_attr(not(unix), allow(dead_code))] // the windowed `--seq` reads are Unix only
    pub(crate) fn pending_len(&self) -> usize {
        self.record.len()
    }

    /// The next piece of the stream. `false` means the stream is not plain, whatever follows.
    #[cfg_attr(not(unix), allow(dead_code))] // the windowed `--seq` reads are Unix only
    pub(crate) fn feed(&mut self, piece: &[u8]) -> bool {
        self.step(piece, false)
    }

    /// The last piece of the stream (possibly empty): whether the whole stream was plain.
    pub(crate) fn finish_with(mut self, piece: &[u8]) -> bool {
        self.step(piece, true) && self.records > 0
    }

    fn step(&mut self, piece: &[u8], is_last: bool) -> bool {
        if piece.is_empty() && !is_last {
            return true;
        }
        if !self.started && !piece.is_empty() {
            self.started = true;
            if self.guards.starts_on_rs && piece[0] != ASCII_RS {
                return false;
            }
        }
        if let Some(&last) = piece.last() {
            self.last = Some(last);
        }
        let mut from = 0;
        while let Some(at) = piece[from..].iter().position(|&b| b == ASCII_RS) {
            let at = from + at;
            // The record this RS ends. A stream that starts on its first RS has nothing before it:
            // no record, where bytes before it are one (the `starts_on_rs` guard turns those down).
            let nothing_before = !self.opened && self.record.is_empty() && at == 0;
            if !nothing_before && !self.close(&piece[from..at]) {
                return false;
            }
            self.opened = true;
            from = at + 1;
        }
        let rest = &piece[from..];
        if !is_last {
            self.record.extend_from_slice(rest);
            return true;
        }
        // The record after the last RS, which is the whole stream when it holds none.
        (!self.guards.ends_on_newline || self.last == Some(b'\n')) && self.close(rest)
    }

    /// The record that `tail` ends, with what earlier pieces held of it.
    fn close(&mut self, tail: &[u8]) -> bool {
        let ok = if self.record.is_empty() {
            Self::record_ok(tail, self.guards)
        } else {
            self.record.extend_from_slice(tail);
            Self::record_ok(&self.record, self.guards)
        };
        self.record.clear();
        self.records += usize::from(ok);
        ok
    }
}

/// A stream that jq's reader walks as it walks `tail` read after `lines` line breaks of RFC 7464's own
/// framing (#7164): a pipe is read a window at a time while its records are `RS value LF`, and the rest of
/// it, from the first record that is not, is walked by the reader that reads a stream whole.
///
/// After such a record jq's reader stands at the start of a line, in `Normal` (an RS is a record of its
/// own there, where at the start of a stream it only opens one), with nothing pending, and its `fgets` chunks start afresh at the line break. So the
/// bytes from there are read as they are read after an RS and `lines` line breaks, each a chunk of its
/// own: every line, value, warning and position rule comes out as in the whole stream, the column too (it
/// is 0 at a line's start). Nothing before the tail yields a value or a warning. The one other state the
/// reader carries over a clean record is the stale byte of its token buffer, which
/// [`token_byte_after`] finds and [`seed_token_byte`] gives the walk.
///
/// `lines` of 0 is a tail that is the whole stream (no record was clean), read as it is.
pub(crate) fn resumed_stream(lines: usize, tail: Vec<u8>) -> Vec<u8> {
    if lines == 0 {
        return tail;
    }
    let mut stream = resumed_front(lines, tail.len());
    stream.extend_from_slice(&tail);
    stream
}

/// The front of a [`resumed_stream`] after `lines` line breaks, with room for `tail` more bytes, so a
/// reader can read the tail straight in behind it rather than read it and copy it (#7262): a tail can be
/// most of a big stream. Empty for `lines` of 0, where the tail is the stream.
///
/// The room is asked for, not required: room no allocation can hold (the length of a sparse file of
/// terabytes) is not reserved, and the reader grows the stream as it reads, whose failure is reported as
/// the read's own error where an allocation that must succeed would abort the process.
pub(crate) fn resumed_front(lines: usize, tail: usize) -> Vec<u8> {
    let front = if lines == 0 { 0 } else { 1 + lines };
    let mut stream = Vec::new();
    if stream
        .try_reserve_exact(front.saturating_add(tail))
        .is_err()
    {
        stream.reserve_exact(front);
    }
    if lines > 0 {
        stream.push(ASCII_RS);
        stream.resize(front, b'\n');
    }
    stream
}

/// [`resumed_front`] with one line break, and the line breaks it stands for besides (#7260), which
/// [`seed_line_base`] gives the walks of the stream: the front of a stream resumed after a million
/// records was a megabyte of line breaks, which every walk of it read a chunk at a time (some 200
/// instructions each). An RS and a line break put jq's reader where the RS and all of them do, at a
/// line start in `Normal` with its `fgets` reads starting afresh; only the count of lines differs.
pub(crate) fn resumed_front_counted(lines: usize, tail: usize) -> (Vec<u8>, usize) {
    (resumed_front(lines.min(1), tail), lines.saturating_sub(1))
}

/// [`resumed_stream`] with [`resumed_front_counted`]'s front (#7260): the bytes and the line breaks
/// they stand for besides.
#[cfg(test)]
pub(crate) fn resumed_stream_counted(lines: usize, tail: Vec<u8>) -> (Vec<u8>, usize) {
    let (mut stream, base) = resumed_front_counted(lines, tail.len());
    stream.extend_from_slice(&tail);
    (stream, base)
}

thread_local! {
    /// The line breaks the first source of every walk made on this thread from now on is taken to
    /// start after ([`seed_line_base`]).
    static SEEDED_LINE_BASE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Starts the first source of every walk made on this thread from now on `lines` line breaks into the
/// stream (#7260): a stream resumed with [`resumed_front_counted`]'s front, whose line breaks it does
/// not hold. Its warnings count them in their lines, and its values in theirs.
pub(crate) fn seed_line_base(lines: usize) {
    SEEDED_LINE_BASE.with(|seed| seed.set(lines));
}

/// What [`seed_line_base`] set: 0 for a stream that is not resumed.
pub(crate) fn seeded_line_base() -> usize {
    SEEDED_LINE_BASE.with(std::cell::Cell::get)
}

/// The byte jq's token buffer holds at index 1 once its reader has read `records`, a run of records
/// that each are one RFC 8259 document (#7164), or `None` when they wrote none there (a buffer that
/// never grew, or one only tokens of a byte or less filled). `check_literal` reads it for a bare `n`
/// ([`Reader::token_buf`]), so it is all a reader resumed after them ([`resumed_stream`]) needs of the
/// tokens before.
///
/// The last token that wrote index 1 decides, so the records are read from their end: a number writes
/// its second byte, or the NUL jq ends a one-byte number with; a literal writes its second byte; a
/// string of two bytes or more writes the second byte of the text it decodes to, or the second byte it
/// held when that is one byte ([`decode_string_in_place`]). A string of a byte or less, white space,
/// structure and RS write nothing there. A test holds this to the reader's own buffer.
pub(crate) fn token_byte_after(records: &[u8]) -> Option<u8> {
    let mut end = records.len();
    while end > 0 {
        let byte = records[end - 1];
        if byte == b'"' {
            // The quote that opens the string: one with an even run of backslashes before it.
            let mut open = end - 1;
            loop {
                open = records[..open].iter().rposition(|&b| b == b'"')?;
                let escapes = records[..open]
                    .iter()
                    .rev()
                    .take_while(|&&b| b == b'\\')
                    .count();
                if escapes % 2 == 0 {
                    break;
                }
            }
            let text = &records[open + 1..end - 1];
            if text.len() >= 2 {
                let mut token = text.to_vec();
                // The records are RFC 8259, so the decode does not fail.
                let _ = decode_string_in_place(&mut token);
                return Some(token[1]);
            }
            end = open;
        } else if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'+' | b'.') {
            let start = records[..end]
                .iter()
                .rposition(|&b| !(b.is_ascii_alphanumeric() || matches!(b, b'-' | b'+' | b'.')))
                .map_or(0, |at| at + 1);
            let token = &records[start..end];
            // A one-byte number is NUL-terminated over index 1; the token holds the byte it was found by.
            return Some(if token.len() == 1 { 0 } else { token[1] });
        } else {
            end -= 1;
        }
    }
    None
}

thread_local! {
    /// The byte the token buffer of every reader made on this thread starts with at index 1
    /// ([`seed_token_byte`]).
    static SEEDED_TOKEN_BYTE: std::cell::Cell<Option<u8>> = const { std::cell::Cell::new(None) };
}

/// Starts the token buffer of every reader made on this thread from now on with `byte` at index 1
/// (#7164): the walks of a [`resumed_stream`] read on from the records before its tail, whose last
/// token left it there ([`token_byte_after`]). `None` starts them empty, as a stream's own start does.
pub(crate) fn seed_token_byte(byte: Option<u8>) {
    SEEDED_TOKEN_BYTE.with(|seed| seed.set(byte));
}

/// What jq's reader makes of a stretch of a `--seq` stream that is not RFC 7464's own framing
/// ([`walk_stretch`], #7322).
pub(crate) struct StretchWalk {
    /// The byte range of every value the reader yields, in order, as offsets into the stretch.
    pub(crate) values: Vec<(usize, usize)>,
    /// Each line the reader warns with (`jq: ignoring parse error: ...`), in order, with the offset
    /// in the stretch of the byte it met it on. No value is yielded on that byte, and none holds it.
    pub(crate) warnings: Vec<(usize, String)>,
    /// What the reader's token buffer holds at index 1 after the stretch ([`token_byte_after`]).
    pub(crate) token_byte: Option<u8>,
}

/// The most warnings a [`walk_stretch`] keeps: a stretch can warn once a byte (a run of `}`), and each
/// is a line held until the values before it have run. A stretch is read in pieces of 64 KiB, or a line
/// (#7341), so this is a piece's worth: only a line longer than that which warns once a byte is past it.
const MAX_STRETCH_WARNINGS: usize = 1 << 16;

/// Walks `stretch` as jq's reader walks it in the middle of a stream (#7322): from the RS that opens a
/// record which is not `RS value LF`, through the next RS at a line start that opens a record which
/// is (an RS before it may start a line too, and open a record that is not). `lines` line breaks of
/// RFC 7464's own framing came before it and left `token_byte` in the token buffer; `lines` of 0 is a
/// stretch at the stream's very start.
///
/// The reader stands before it as [`resumed_stream`] says: at a line start, in `Normal` with nothing
/// pending (or waiting for the stream's first RS). After its last byte, an RS, the reader is there
/// again whatever the stretch held: an RS resets the parser, with a warning or without. So the
/// records after the stretch are read as the records before it were, and the stretch is all of the
/// stream this walk needs. It is never the stream's end, so neither the final buffer's position rules
/// nor the EOF branch applies.
///
/// `None` when the stretch warns more than [`MAX_STRETCH_WARNINGS`] times, or (defensive) when the
/// walk ends in any other state: the caller then leaves the stream from the stretch on to the walk
/// that reads it whole.
pub(crate) fn walk_stretch(
    stretch: &[u8],
    lines: usize,
    token_byte: Option<u8>,
) -> Option<StretchWalk> {
    debug_assert_eq!(stretch.first(), Some(&ASCII_RS));
    debug_assert_eq!(stretch.last(), Some(&ASCII_RS));
    let mut warnings: Vec<(usize, String)> = Vec::new();
    let mut overflowed = false;
    let mut emit = |warning: &str, at: usize| {
        if warnings.len() < MAX_STRETCH_WARNINGS {
            warnings.push((at, warning.to_string()));
        } else {
            overflowed = true;
        }
    };
    let bom = BomPrefix {
        consumed: 0,
        malformed: false,
    };
    let mut reader = Reader::new(bom, &mut emit);
    reader.token_buf = token_byte.map_or_else(Vec::new, |byte| vec![0, byte]);
    if lines > 0 {
        // As after an RS and `lines` line breaks.
        reader.st = St::Normal;
        reader.last_ch_was_ws = true;
        reader.line += lines;
    }
    reader.total = stretch.len();
    reader.run(stretch, 0, &[]);
    let settled = reader.st == St::Normal
        && reader.stack.is_empty()
        && reader.token_len == 0
        && reader.next.is_none();
    let token_byte = reader.token_buf.get(1).copied();
    let values = std::mem::take(&mut reader.values);
    let at_eof = reader.yielded_at.contains(&YIELDED_AT_EOF);
    drop(reader);
    if overflowed || !settled || at_eof || warnings.iter().any(|&(at, _)| at == AT_EOF) {
        return None;
    }
    Some(StretchWalk {
        values,
        warnings,
        token_byte,
    })
}

/// What follows the last of the files a walk reads (#6588): a file that did not open means jq's
/// reader asks that file for more before it is at its end, so the refill that sets `is_last` is
/// the empty one at the missing file, and the last opened file's last chunk is not the final
/// buffer: [`final_buffer_start`] is the stream's end.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Tail {
    /// The last file is the last source: its last chunk is the final buffer.
    Opened,
    /// A file that did not open follows the last file that did.
    UnopenedFollows,
}

impl Tail {
    pub(crate) fn of(unopened_follows: bool) -> Self {
        if unopened_follows {
            Self::UnopenedFollows
        } else {
            Self::Opened
        }
    }
}

/// Whether the walk discards the bytes before an RS one at a time, as it did before #7263, rather
/// than a run at a time: off in the shipped build, and a switch in the tests, which hold the two
/// to the same answers.
mod discards {
    #[cfg(test)]
    std::thread_local! {
        static BYTEWISE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    #[cfg(not(test))]
    #[inline(always)]
    pub(super) fn bytewise() -> bool {
        false
    }

    #[cfg(test)]
    pub(super) fn bytewise() -> bool {
        BYTEWISE.with(std::cell::Cell::get)
    }

    /// Run `f` with the walk discarding a byte at a time.
    #[cfg(test)]
    pub(super) fn bytewise_in<T>(f: impl FnOnce() -> T) -> T {
        /// Clears the switch when dropped, so a closure that panics leaves it off for the rest
        /// of the test thread.
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                BYTEWISE.with(|on| on.set(false));
            }
        }
        BYTEWISE.with(|on| on.set(true));
        let _reset = Reset;
        f()
    }
}

/// Test-only count of the walks made on this thread, so that a test can pin how many a run makes
/// (#6620).
#[cfg(test)]
pub(crate) mod walks {
    use std::cell::Cell;

    thread_local! {
        static MADE: Cell<usize> = const { Cell::new(0) };
    }

    pub(super) fn bump() {
        MADE.with(|n| n.set(n.get() + 1));
    }

    pub(crate) fn made() -> usize {
        MADE.with(Cell::get)
    }
}

fn walk_stream_placing(
    raw_bytes: &[(Option<usize>, Vec<u8>)],
    slurp: bool,
    collect_faults: bool,
    place_values: bool,
    tail: Tail,
    emit: &mut dyn FnMut(&str, usize),
) -> SeqStreamWalk {
    #[cfg(test)]
    walks::bump();
    let bom = bom_prefix(raw_bytes);
    let mut reader = Reader::new(bom, emit);
    // #7260: a resumed stream stands that many line breaks into the stream it resumes.
    reader.line += seeded_line_base();
    reader.collect_faults = collect_faults;
    // jq refills its reader one file at a time, so each source after the
    // first begins its own `jv_parser_next` -- whose malformed-BOM reset the
    // walk must reproduce (#3002). The offsets are the sources' absolute
    // boundaries (the end of each but the last, i.e. the start of each
    // subsequent one), matching the undecorated
    // numbering. The last of these ends is the stream's whole length,
    // reused below instead of a second `.sum()` over the same sources --
    // both `final_buffer_start` and the EOF answer need it.
    let total = source_ends(raw_bytes).last().copied().unwrap_or(0);
    // With a malformed BOM jq re-runs `parser_reset` at the top of *every*
    // `jv_parser_next`, and each `fgets` refill starts one: resets after a
    // newline happen as the reader scans it, and a refill after a full
    // 4095-byte chunk with no newline is one more (#3200).
    let boundaries = refill_boundaries(raw_bytes, bom.malformed);
    // #6588: past a last file that did not open, the final buffer is the empty one read there.
    let final_buffer_start = if tail == Tail::UnopenedFollows {
        total
    } else {
        final_buffer_start(raw_bytes, total)
    };
    reader.final_buffer_start = final_buffer_start;
    reader.total = total;
    // The call for the *empty* final buffer resets as well, wiping the accumulated state before the
    // EOF branch can report on it, so no `at EOF` warning survives. jq 1.8.2's final buffer is always
    // that empty one (#7718); jq 1.7.1's held data unless the last chunk ended on a newline or filled
    // the buffer exactly, and reported there.
    reader.suppress_eof_warning = bom.malformed;
    reader.slurp = slurp;
    // One buffer, so `run` can look ahead; only a multi-source stream is copied into one.
    let combined: std::borrow::Cow<'_, [u8]> = match raw_bytes {
        [(_, only)] => std::borrow::Cow::Borrowed(only.as_slice()),
        _ => std::borrow::Cow::Owned(
            raw_bytes
                .iter()
                .flat_map(|(_, raw)| raw.iter().copied())
                .collect(),
        ),
    };
    reader.run(&combined, bom.consumed, &boundaries);
    // The same pass answers `--seq -s`'s EOF-location question; see the
    // field's docs for the rule. `final_buffer_start == total` is how
    // [`final_buffer_start`] spells an *empty* final buffer.
    let slurp_position_lost = reader.warned_in_final_buffer
        || (reader.last_warning_at_eof
            && !(reader.last_byte_yielded_value && final_buffer_start < total));
    let faults = if collect_faults {
        let at: Vec<usize> = reader.faults.iter().map(|fault| fault.at).collect();
        value_locations(raw_bytes, &at)
            .into_iter()
            .zip(std::mem::take(&mut reader.faults))
            .map(|((source, line), fault)| SeqFault {
                body: fault.body,
                values_before: fault.values_before,
                source,
                line,
                position_lost: fault.position_lost,
            })
            .collect()
    } else {
        Vec::new()
    };
    // Only non-slurp output names a location per value; `-s` would throw this pass away.
    let (locations, unplaced) = if slurp {
        (Vec::new(), Vec::new())
    } else if place_values {
        debug_assert_eq!(reader.yielded_at.len(), reader.values.len());
        (value_locations(raw_bytes, &reader.yielded_at), Vec::new())
    } else {
        // Kept, so a caller that finds it needs them after all places them without a second walk
        // ([`SeqStreamWalk::place_values`], #6620).
        (Vec::new(), std::mem::take(&mut reader.yielded_at))
    };
    SeqStreamWalk {
        faults,
        slurp_position_lost,
        locations,
        unplaced,
        values: reader.values,
    }
}

/// A `--seq` stream walked as its bytes are read (#7775), for a run that reads it through the `input`
/// queue: the walk [`walk_stream_collecting_at`] makes of the whole stream, a piece at a time, so a
/// program that reads only the first records reads only the bytes they are in, as jq does.
///
/// The bytes are read a source at a time ([`SeqStreamWalker::start_source`],
/// [`SeqStreamWalker::read_from`], [`SeqStreamWalker::end_source`], then [`SeqStreamWalker::end`]), and
/// [`SeqStreamWalker::advance`] walks every byte of the `fgets` chunks that are whole by then: a chunk
/// ends at a newline or after [`JQ_READER.read_len`](JQ_READER) bytes, or at its source's end. Each
/// value, each fault and each place is the one the whole walk gives, in the same order, so a run
/// that reads to the end reads what it did: a test holds the two to each other. Not for `-s`, a
/// stream with a file that did not open, or a resumed stream (`seed_*`): their walks need the whole
/// stream first.
///
/// The walk holds what jq's reader holds and no more: the values are taken out as they are handed
/// on ([`SeqStreamWalker::pop_value`], [`SeqStreamWalker::pop_fault`]), and the bytes before the first
/// one still needed (by a value not yet taken, the value being read, the next byte to walk, or the
/// next place to count) are released, so a stream of any length is read in the room of its longest
/// record and a read or two.
pub(crate) struct SeqStreamWalker {
    /// The bytes still held: the stream from offset `base` on, the sources one after another. Every
    /// offset the walk deals in is the stream's own.
    data: Vec<u8>,
    base: usize,
    /// Each source's tag and where it starts in the stream.
    sources: Vec<(Option<usize>, usize)>,
    /// Whether the last source has ended, and whether every source has.
    source_ended: bool,
    ended: bool,
    /// Where the next `fgets` chunk of the last source starts; the stream up to `whole` is in whole
    /// chunks.
    chunk_at: usize,
    whole: usize,
    /// Whether the last whole chunk of the last source filled the buffer without a newline: a
    /// malformed BOM's reader resets at the refill after it ([`refill_boundaries`]).
    after_full_chunk: bool,
    /// The last whole chunk of the last source, `(start, len)`, and whether it ends at a newline:
    /// where jq's final buffer starts ([`final_buffer_start`]) is read from it once the stream has ended.
    last_chunk: Option<(usize, usize, bool)>,
    /// What a read that is not a regular file's is read into, kept so it is zeroed once
    /// ([`Self::read_from`]).
    scratch: Vec<u8>,
    /// How far the stream is known to be UTF-8, and its first byte that is not.
    valid_to: usize,
    first_invalid: Option<usize>,
    /// The reader, once the stream's front has said whether it is a BOM ([`bom_prefix`]).
    reader: Option<Reader<'static>>,
    /// What [`Self::whole_chunks`] found before the reader was made: the starts of chunks after a
    /// full one.
    pending: Vec<usize>,
    /// Where the reader stands, and the refill boundaries: offsets into `data`, as every offset the
    /// reader holds is, so its byte loop is the whole stream's ([`Reader::rebase`]).
    at: RunAt,
    boundaries: Vec<usize>,
    /// Where the next place is counted from: each value's and each fault's, in the stream's order.
    cursor: PlaceCursor,
    /// The values walked and not yet taken, each with its place, and the faults with theirs.
    values: std::collections::VecDeque<((usize, usize), (usize, usize))>,
    faults: std::collections::VecDeque<SeqFault>,
    /// How many values have been taken ([`Self::pop_value`]).
    taken: usize,
    /// The first byte and the end of the last value walked, for where the stream's end is read.
    last_value: Option<(u8, usize)>,
    /// The newlines of the last source in the bytes released, and where the last of them ends.
    released_newlines: usize,
    released_line_end: Option<usize>,
    finished: bool,
}

/// The fewest bytes [`SeqStreamWalker`] releases at once: below this a stream is held whole. The tests
/// release a few bytes at a time, so that their short streams are walked through the release too.
const RELEASE_MIN: usize = if cfg!(test) { 16 } else { 1 << 16 };

/// [`value_locations`] a yield at a time, over the sources a [`SeqStreamWalker`] has read: one cursor
/// for the values and the faults together, asked in the stream's order.
#[derive(Default)]
struct PlaceCursor {
    source: Option<usize>,
    chunk_end: usize,
    newlines: usize,
}

impl PlaceCursor {
    /// Moves to source `index`, if it is not already there.
    fn enter(&mut self, index: usize) {
        if self.source != Some(index) {
            self.source = Some(index);
            self.chunk_end = 0;
            // #7260: a resumed stream's first source starts that many line breaks in.
            self.newlines = if index == 0 { seeded_line_base() } else { 0 };
        }
    }

    /// The source holding offset `at`, and how far into it `at` is.
    fn locate(sources: &[(Option<usize>, usize)], at: usize) -> (usize, usize) {
        let index = sources
            .partition_point(|&(_, start)| start <= at)
            .saturating_sub(1);
        (
            index,
            at - sources.get(index).map_or(0, |&(_, start)| start),
        )
    }

    /// Steps over the chunks of the current source while `go(chunk_end)` holds of where the next one
    /// starts and `fits(chunk_end, len)` of it, in `data`, the stream from `base` on.
    fn step(
        &mut self,
        data: &[u8],
        base: usize,
        sources: &[(Option<usize>, usize)],
        go: impl Fn(usize) -> bool,
        fits: impl Fn(usize, usize) -> bool,
    ) {
        let index = self.source.unwrap_or(0);
        let start = sources.get(index).map_or(0, |&(_, start)| start);
        let end = sources
            .get(index + 1)
            .map_or(base + data.len(), |&(_, next)| next);
        // The bytes of the source the walk still holds: the cursor never stands before them.
        let skip = base.saturating_sub(start);
        let raw = &data[start + skip - base..end - base];
        debug_assert!(self.chunk_end >= skip);
        while go(self.chunk_end) {
            let Some(len) = fgets_chunk_len_at(raw, self.chunk_end - skip) else {
                break;
            };
            if !fits(self.chunk_end, len) {
                break;
            }
            self.chunk_end += len;
            if raw[self.chunk_end - skip - 1] == b'\n' {
                self.newlines += 1;
            }
        }
    }

    /// The `(source index, line)` of a yield at `at` ([`YIELDED_AT_EOF`] for the EOF branch), the
    /// yields asked for in ascending order.
    fn place(
        &mut self,
        data: &[u8],
        base: usize,
        sources: &[(Option<usize>, usize)],
        at: usize,
    ) -> (usize, usize) {
        let (index, local) = if at == YIELDED_AT_EOF {
            (sources.len().saturating_sub(1), usize::MAX)
        } else {
            Self::locate(sources, at)
        };
        self.enter(index);
        // Read chunks until the one holding `local` has been read.
        self.step(
            data,
            base,
            sources,
            |chunk_end| chunk_end <= local,
            |_, _| true,
        );
        (index, self.newlines)
    }

    /// Steps over every chunk that ends at or before `offset`, where the walk stands: no yield to come
    /// is in one, so their bytes need not be held to count it.
    fn catch_up(
        &mut self,
        data: &[u8],
        base: usize,
        sources: &[(Option<usize>, usize)],
        offset: usize,
    ) {
        if sources.is_empty() {
            return;
        }
        let (index, local) = Self::locate(sources, offset);
        if self.source.is_some_and(|source| source > index) {
            return;
        }
        self.enter(index);
        self.step(
            data,
            base,
            sources,
            |_| true,
            |chunk_end, len| chunk_end + len <= local,
        );
    }

    /// The offset in the stream the cursor stands at.
    fn position(&self, sources: &[(Option<usize>, usize)]) -> usize {
        self.source
            .and_then(|index| sources.get(index))
            .map_or(0, |&(_, start)| start + self.chunk_end)
    }
}

impl Default for SeqStreamWalker {
    fn default() -> Self {
        Self::new()
    }
}

impl SeqStreamWalker {
    pub(crate) fn new() -> Self {
        Self {
            data: Vec::new(),
            base: 0,
            sources: Vec::new(),
            source_ended: true,
            ended: false,
            chunk_at: 0,
            whole: 0,
            after_full_chunk: false,
            last_chunk: None,
            scratch: Vec::new(),
            valid_to: 0,
            first_invalid: None,
            reader: None,
            pending: Vec::new(),
            at: RunAt::default(),
            boundaries: Vec::new(),
            cursor: PlaceCursor::default(),
            values: std::collections::VecDeque::new(),
            faults: std::collections::VecDeque::new(),
            taken: 0,
            last_value: None,
            released_newlines: 0,
            released_line_end: None,
            finished: false,
        }
    }

    /// The stream's length so far.
    fn len(&self) -> usize {
        self.base + self.data.len()
    }

    /// The bytes still held, and the offset in the stream of the first of them.
    pub(crate) fn window(&self) -> (&[u8], usize) {
        (&self.data, self.base)
    }

    /// Each source's tag and where it starts in the stream.
    pub(crate) fn sources(&self) -> &[(Option<usize>, usize)] {
        &self.sources
    }

    /// The stream's first byte that is not UTF-8, or how far it is known to be UTF-8 (the next read
    /// can still complete a character cut at the end of this one).
    pub(crate) fn utf8_up_to(&self) -> usize {
        self.first_invalid.unwrap_or(self.valid_to)
    }

    /// Starts the next source, tagged `tag`; the one before has ended ([`Self::end_source`]).
    pub(crate) fn start_source(&mut self, tag: Option<usize>) {
        debug_assert!(self.source_ended && !self.ended);
        let start = self.len();
        if !self.sources.is_empty() {
            // jq refills one file at a time, so a later source's start begins a `jv_parser_next`.
            self.boundaries.push(start - self.base);
        }
        self.sources.push((tag, start));
        self.source_ended = false;
        self.chunk_at = start;
        self.after_full_chunk = false;
        self.last_chunk = None;
        self.released_newlines = 0;
        self.released_line_end = None;
    }

    /// Reads at most `want` more bytes of the current source from `reader`; `Ok(0)` at its end. A
    /// `regular` source (a regular file, or bytes in memory) is read until `want` bytes or its end,
    /// straight into the stream. Any other (a pipe, a socket, a terminal) is read with one `read`,
    /// which hands over what it holds, so a record that has arrived is answered without waiting for
    /// more; it goes through a buffer zeroed once rather than at every read.
    pub(crate) fn read_from(
        &mut self,
        reader: &mut dyn std::io::Read,
        want: usize,
        regular: bool,
    ) -> std::io::Result<usize> {
        use std::io::Read as _;
        debug_assert!(!self.source_ended);
        let read = if regular {
            reader.take(want as u64).read_to_end(&mut self.data)
        } else {
            if self.scratch.len() < want {
                self.scratch.resize(want, 0);
            }
            let read = loop {
                match reader.read(&mut self.scratch[..want]) {
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    read => break read,
                }
            };
            if let Ok(read) = read {
                self.data.extend_from_slice(&self.scratch[..read]);
            }
            read
        };
        if read.is_ok() {
            self.check_utf8();
            self.whole_chunks();
        }
        read
    }

    /// More bytes of the current source (the tests feed a walk in pieces).
    #[cfg(test)]
    pub(crate) fn push(&mut self, bytes: &[u8]) {
        debug_assert!(!self.source_ended);
        self.data.extend_from_slice(bytes);
        self.check_utf8();
        self.whole_chunks();
    }

    /// Extends what is known of the stream being UTF-8 past the bytes just read.
    fn check_utf8(&mut self) {
        if self.first_invalid.is_some() {
            return;
        }
        match std::str::from_utf8(&self.data[self.valid_to - self.base..]) {
            Ok(_) => self.valid_to = self.len(),
            Err(e) => match e.error_len() {
                Some(_) => self.first_invalid = Some(self.valid_to + e.valid_up_to()),
                // A character cut at the end of what is read so far: the rest of it comes next.
                None => self.valid_to += e.valid_up_to(),
            },
        }
    }

    /// The current source has no more bytes.
    pub(crate) fn end_source(&mut self) {
        self.source_ended = true;
        self.whole_chunks();
    }

    /// Every source has ended.
    pub(crate) fn end(&mut self) {
        if !self.source_ended {
            self.end_source();
        }
        self.ended = true;
    }

    /// Whether the walk is done: every source ended and its last byte walked, the EOF branch run.
    #[cfg(test)]
    pub(crate) fn finished(&self) -> bool {
        self.finished
    }

    /// Finds the chunks of the current source that are whole now.
    fn whole_chunks(&mut self) {
        // Where the last newline read so far ends: the chunks before it can be taken as lines.
        let lines_end = rfind_byte(b'\n', &self.data[self.chunk_at - self.base..])
            .map(|at| self.chunk_at + at + 1);
        let mut lines = true;
        loop {
            if let Some(end) = lines_end.filter(|&end| lines && end > self.chunk_at) {
                if self.reader.is_some() && !self.after_full_chunk {
                    lines = self.take_lines(end);
                }
            }
            if !self.whole_chunk() {
                break;
            }
            // Past a newline-free window, the chunks go one at a time until one ends at a newline.
            lines |= self.data[self.chunk_at - 1 - self.base] == b'\n';
        }
    }

    /// Takes the chunks from `chunk_at` to `end` (just past a newline) at once, when every
    /// [`LINE_PROBE`] bytes there hold a newline: no run of [`JQ_READER.read_len`](JQ_READER) bytes
    /// is then free of one, so every chunk ends at a newline, and a chunk that ends at a newline is
    /// not a full chunk. Otherwise takes the lines before the first window that holds none, and
    /// says so with `false`.
    fn take_lines(&mut self, end: usize) -> bool {
        let from = self.chunk_at;
        let span = &self.data[from - self.base..end - self.base];
        let mut upto = span.len();
        let mut all = true;
        for (index, window) in span.chunks_exact(LINE_PROBE).enumerate() {
            if find_byte(b'\n', window).is_none() {
                let window_start = index * LINE_PROBE;
                upto = rfind_byte(b'\n', &span[..window_start]).map_or(0, |at| at + 1);
                all = false;
                break;
            }
        }
        if upto > 0 {
            let last_start = rfind_byte(b'\n', &span[..upto - 1]).map_or(0, |at| at + 1);
            self.last_chunk = Some((from + last_start, upto - last_start, true));
            self.chunk_at = from + upto;
            self.whole = self.chunk_at;
        }
        all
    }

    /// Finds the next chunk of the current source if it is whole now; `false` when there is none.
    fn whole_chunk(&mut self) -> bool {
        let rest = &self.data[self.chunk_at - self.base..];
        let Some(read) = Fgets::of(rest, JQ_READER.read_len) else {
            return false;
        };
        // A read that would meet the end is whole only once the source has ended.
        if read.meets_end(JQ_READER.read_len) && !self.source_ended {
            return false;
        }
        let Fgets { len, newline } = read;
        let chunk_start = self.chunk_at;
        let full = read.fills(JQ_READER.read_len) && !newline;
        let after_full = std::mem::replace(&mut self.after_full_chunk, full);
        if let Some(reader) = self.reader.as_mut() {
            if after_full && reader.resets_per_call {
                self.boundaries.push(chunk_start - self.base);
            }
        } else if after_full {
            // Held until the front decides the BOM, which makes the reader ([`Self::advance`]).
            self.pending.push(chunk_start);
        }
        self.last_chunk = Some((chunk_start, len, newline));
        self.chunk_at += len;
        self.whole = self.chunk_at;
        true
    }

    /// Walks every byte of the whole chunks read so far, and, once the stream has ended, the EOF
    /// branch; then releases what no value, fault or place to come needs.
    pub(crate) fn advance(&mut self) {
        if self.finished {
            return;
        }
        if self.reader.is_none() && !self.make_reader() {
            return;
        }
        let Some(reader) = self.reader.as_mut() else {
            return;
        };
        let (base, total) = (self.base, self.base + self.data.len());
        reader.run_to(
            &mut self.at,
            &self.data,
            self.whole - base,
            &self.boundaries,
        );
        if self.ended && self.at.offset + base >= total {
            reader.final_buffer_start = final_buffer_start_after(self.last_chunk, total) - base;
            reader.total = total - base;
            reader.run_end(&self.at, &self.boundaries);
            self.finished = true;
        }
        self.count_places();
        self.release();
    }

    /// Makes the reader once the front of the stream says what jq's BOM handling makes of it: a
    /// mismatch, or the end of the stream. Nothing is released before then.
    fn make_reader(&mut self) -> bool {
        debug_assert_eq!(self.base, 0);
        // The front as whole chunks, a source at a time, as `bom_prefix` reads the sources.
        let front: Vec<(Option<usize>, Vec<u8>)> = self
            .sources
            .iter()
            .enumerate()
            .map(|(index, &(tag, start))| {
                let end = self
                    .sources
                    .get(index + 1)
                    .map_or(self.whole, |&(_, next)| next.min(self.whole));
                (tag, self.data[start.min(end)..end].to_vec())
            })
            .collect();
        let bom = bom_prefix(&front);
        let known: usize = front.iter().map(|(_, raw)| raw.len()).sum();
        if bom.consumed >= known && !(self.ended && self.whole == self.len()) {
            // Every byte so far matched, or was never seen: more can still change the verdict.
            return false;
        }
        // The faults are recorded, not printed: the queue hands them out (#6147). A closure that
        // captures nothing is zero-sized, so leaking its box allocates nothing.
        let emit: &'static mut dyn FnMut(&str, usize) = Box::leak(Box::new(|_: &str, _: usize| {}));
        let mut reader = Reader::new(bom, emit);
        reader.line += seeded_line_base();
        reader.collect_faults = true;
        reader.suppress_eof_warning = bom.malformed;
        let full_starts = std::mem::take(&mut self.pending);
        if bom.malformed {
            self.boundaries.extend(full_starts);
            self.boundaries.sort_unstable();
        }
        self.at = RunAt::from(bom.consumed);
        self.reader = Some(reader);
        true
    }

    /// Takes the values and faults the reader has yielded since the last call out of it, each with its
    /// place, counted in the stream's order; then counts the places up to where the walk stands.
    fn count_places(&mut self) {
        let Some(reader) = self.reader.as_mut() else {
            return;
        };
        let values = std::mem::take(&mut reader.values);
        let yielded_at = std::mem::take(&mut reader.yielded_at);
        let faults = std::mem::take(&mut reader.faults);
        reader.values_drained += values.len();
        let (data, base, sources) = (&self.data, self.base, &self.sources);
        // The reader's offsets are into `data`: the stream's are `base` on.
        let in_stream = |at: usize| if at == YIELDED_AT_EOF { at } else { at + base };
        let mut faults = faults.into_iter().peekable();
        for ((start, end), at) in values.into_iter().zip(yielded_at) {
            while let Some(fault) = faults.next_if(|fault| fault.at < at) {
                let place = self.cursor.place(data, base, sources, in_stream(fault.at));
                self.faults.push_back(seq_fault(fault, place));
            }
            let place = self.cursor.place(data, base, sources, in_stream(at));
            self.last_value = Some((data[start], end + base));
            self.values.push_back(((start + base, end + base), place));
        }
        for fault in faults {
            let place = self.cursor.place(data, base, sources, in_stream(fault.at));
            self.faults.push_back(seq_fault(fault, place));
        }
        if !self.finished {
            self.cursor
                .catch_up(data, base, sources, self.at.offset + base);
        }
    }

    /// Lets go of the bytes before the first one still needed: by a value not yet taken, the value
    /// the reader is in, the next byte to walk, the next place to count, the last chunk and the UTF-8
    /// check. They go
    /// once they are at least [`RELEASE_MIN`] and eight times the bytes kept, so the bytes moved to the
    /// front are an eighth of the stream at most.
    fn release(&mut self) {
        let Some(reader) = self.reader.as_mut() else {
            return;
        };
        let mut keep = (self.at.offset + self.base).min(self.cursor.position(&self.sources));
        if let Some(start) = reader.value_start {
            keep = keep.min(start + self.base);
        }
        if let Some(&((start, _), _)) = self.values.front() {
            keep = keep.min(start);
        }
        // The last chunk, where jq's final buffer may start, stays: the stream's end is read from it.
        if let Some((start, _, _)) = self.last_chunk {
            keep = keep.min(start);
        }
        if self.first_invalid.is_none() {
            keep = keep.min(self.valid_to);
        }
        let gone = keep.saturating_sub(self.base);
        if gone < RELEASE_MIN || gone / 8 < self.data.len() - gone {
            return;
        }
        // Where the stream's end is read hangs off the last source's newlines.
        let last_start = self.sources.last().map_or(0, |&(_, start)| start);
        let from = last_start.max(self.base);
        if from < keep {
            let part = &self.data[from - self.base..keep - self.base];
            self.released_newlines += count_byte(b'\n', part);
            if let Some(at) = rfind_byte(b'\n', part) {
                self.released_line_end = Some(from + at + 1);
            }
        }
        self.data.drain(..gone);
        self.base = keep;
        // The refill boundaries the walk is past; then every offset into `data` moves with it.
        reader.rebase(gone);
        self.boundaries.drain(..self.at.boundary);
        self.at.boundary = 0;
        // One the walk stepped over inside a cut is still to fire, at the next byte walked: at 0 it does.
        for boundary in &mut self.boundaries {
            *boundary = boundary.saturating_sub(gone);
        }
        self.at.offset -= gone;
        self.at.eof_offset = self.at.eof_offset.saturating_sub(gone);
    }

    /// The next fault to hand out, if any has been walked.
    pub(crate) fn peek_fault(&self) -> Option<&SeqFault> {
        self.faults.front()
    }

    pub(crate) fn pop_fault(&mut self) -> Option<SeqFault> {
        self.faults.pop_front()
    }

    /// The next value's byte range and `(source index, line)`, if one has been walked. Its bytes stay
    /// held ([`Self::window`]) until the next [`Self::advance`].
    pub(crate) fn pop_value(&mut self) -> Option<((usize, usize), (usize, usize))> {
        let value = self.values.pop_front()?;
        self.taken += 1;
        Some(value)
    }

    /// Whether a value has been walked and not taken.
    pub(crate) fn has_value(&self) -> bool {
        !self.values.is_empty()
    }

    /// How many values have been taken.
    pub(crate) fn taken(&self) -> usize {
        self.taken
    }

    /// The first byte and the end of the last value walked.
    pub(crate) fn last_value(&self) -> Option<(u8, usize)> {
        self.last_value
    }

    /// The newlines of the last source, and where its last line starts.
    pub(crate) fn last_source_lines(&self) -> (usize, usize) {
        let last_start = self.sources.last().map_or(0, |&(_, start)| start);
        let from = last_start.max(self.base);
        let held = &self.data[from - self.base..];
        let line_start = rfind_byte(b'\n', held)
            .map(|at| from + at + 1)
            .or(self.released_line_end)
            .unwrap_or(last_start);
        (self.released_newlines + count_byte(b'\n', held), line_start)
    }

    /// The stream's length so far.
    pub(crate) fn total(&self) -> usize {
        self.len()
    }
}

/// A fault the reader recorded, placed.
fn seq_fault(fault: RecordedFault, (source, line): (usize, usize)) -> SeqFault {
    SeqFault {
        body: fault.body,
        values_before: fault.values_before,
        source,
        line,
        position_lost: fault.position_lost,
    }
}

/// How many of `bytes` are `needle`: a block of 64 at a time, which the compiler vectorizes, as
/// `count_newlines` in the runner (#4172).
fn count_byte(needle: u8, bytes: &[u8]) -> usize {
    let mut blocks = bytes.chunks_exact(64);
    let mut total = 0usize;
    for block in &mut blocks {
        total += usize::from(
            block
                .iter()
                .fold(0u8, |n, &b| n.wrapping_add(u8::from(b == needle))),
        );
    }
    total + blocks.remainder().iter().filter(|&&b| b == needle).count()
}

impl SeqStreamWalk {
    /// The locations a walk made with `place_values` off would have returned (#6620): the walk is
    /// not repeated, only the pass over the stream's `fgets` chunks that places each value. A no-op
    /// on a walk that placed them, or that is slurped (which names no location per value).
    pub(crate) fn place_values(&mut self, raw_bytes: &[(Option<usize>, Vec<u8>)]) {
        if !self.unplaced.is_empty() {
            self.locations = value_locations(raw_bytes, &std::mem::take(&mut self.unplaced));
        }
    }
}

/// One diagnostic of the `--seq` reader: what jq prints as `jq: ignoring parse error: <body>`
/// and what `input`/`inputs` raise as an error (#5640).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SeqFault {
    /// The parse error's text, without the `jq: ignoring parse error: ` prefix.
    pub(crate) body: String,
    /// How many values the reader had yielded before it met the fault.
    pub(crate) values_before: usize,
    /// jq's `(source index, line)` for the fault, by the same rule as a value's (#3250).
    pub(crate) source: usize,
    pub(crate) line: usize,
    /// Whether jq has lost its position by the time it raises the fault, so the error names
    /// `<unknown>` where it would name `file:line`: the EOF report jq defers to the read after
    /// the one that returned the stream's last value (see [`Reader::fault_position_lost`]).
    pub(crate) position_lost: bool,
}

/// What one raw-byte walk over a `--seq` stream ([`walk_stream`])
/// answers -- the warnings go to its `emit` as it runs; the values and
/// `-s`'s location come back here -- so a caller that needs several of them
/// pays for the walk once.
pub(crate) struct SeqStreamWalk {
    /// Where the reader yielded each value, when the walk was asked not to place them
    /// ([`walk_stream_keeping`]); [`Self::place_values`] turns it into [`Self::locations`].
    unplaced: Vec<usize>,
    /// Every diagnostic the walk reported, in order, when [`walk_stream_collecting`] was asked
    /// to record them (#5640); empty otherwise.
    pub(crate) faults: Vec<SeqFault>,
    /// Whether `--seq -s`'s runtime-error location is lost entirely -- real
    /// jq answering `(at <unknown>)` where it would otherwise name a file
    /// and line (#1542/#1550/#1568/#2947/#3003). Only meaningful under
    /// `-s`; a non-slurp stream carries a location per value instead.
    ///
    /// jq 1.7.1's answer, kept where what is *read* follows jq 1.8.2 (#7718):
    /// 1.8.2 never forgets the file name, so it always names one (#7607).
    ///
    /// **The mechanism, from jq 1.7.1's own `src/util.c`.** `<unknown>` is
    /// not a property of the trailing record at all:
    /// `jq_util_input_get_position` renders it whenever `current_filename`
    /// is not a string, and the only thing that clears that field is
    /// `jq_util_input_read_more` closing the stream it was already standing
    /// on -- which it does on entry, but *only* when that stream is already
    /// at `feof`, and then immediately re-sets the field if another file
    /// follows. So the question reduces to: does jq call `read_more` one
    /// more time after the input is exhausted, *before* the slurped array
    /// is handed to the filter?
    ///
    /// `jq_util_input_next_input` keeps `is_last` -- "did this call perform
    /// the refill that set `feof`" -- in a **local**, reset on every call.
    /// Under `-s` a value never returns early (it is appended to `slurped`);
    /// only a parser **error** does, and an early `return` makes `main` call
    /// back in with `is_last` at 0 again, so that re-entered call is forced
    /// through one more `read_more`, which closes the stream. The slurped
    /// array therefore reaches the filter with the filename intact iff the
    /// call that performed the terminal refill exits through its own
    /// `while (!is_last || has_more)` condition -- iff it returns no error.
    ///
    /// **Which errors are fatal, then, is decided by jq's `fgets`
    /// chunking**: an error detected on a chunk that still has a refill
    /// behind it (or a next file to open) survives; only one detected while
    /// jq is working on the stream's **final buffer** leaves nothing to
    /// refill from. `read_more` calls `fgets(buf, sizeof(buf), f)` on a
    /// `char buf[4096]`, so a chunk ends at a newline or after 4095 bytes,
    /// whichever comes first (see [`final_buffer_start`]) -- and `feof` is
    /// set only by an `fgets` that actually ran out of input, which is why
    /// a chunk ending on a newline, or filling the buffer exactly, is
    /// followed by one more (empty) buffer, where a truncation is finally
    /// reported `at EOF`.
    ///
    /// **The EOF branch is not always reached in that call (#3003).**
    /// `jv_parser_next` returns the moment `scan()` completes a top-level
    /// value; if that happens on the final buffer's *last* byte, the buffer
    /// is fully consumed (`has_more == 0`), the loop exits, and the array
    /// is dispatched -- the `Unfinished JSON term at EOF` for whatever that
    /// byte opened is only detected by the *next* `next_input` call, after
    /// the filter already ran with the position intact (which is also why
    /// real jq prints the runtime error *before* that warning). A number or
    /// keyword is completed by the byte after it, so `\x1e1{` and
    /// `\x1etrue"` keep `file:0`; a string or container completes on its
    /// own last byte, so `\x1e"a"{` and `\x1e{}{` still scan the `{` in the
    /// same call, fall into the EOF branch there, and lose it.
    ///
    /// Hence the rule, all of it read off the same scan that produces the
    /// diagnostics: lost iff a mid-stream error was detected in the final
    /// buffer, or the EOF branch reported one and that branch ran in the
    /// terminal-refill call -- i.e. unless the final buffer is non-empty and
    /// its last byte's scan handed a value over.
    ///
    /// Worked examples, all oracle-verified against jq 1.7.1 -- note that
    /// the record text is identical within each pair, and only the bytes
    /// *after* the failure differ:
    ///
    /// ```text
    /// \x1e[0,]        error on `]` @4, final buffer starts at 0    => <unknown>
    /// \x1e[0,]\n      error on `]` @4, final buffer starts at 6    => file:1
    /// \x1e0\x1e       error on RS  @2, final buffer starts at 0    => <unknown>
    /// \x1e0\x1e\n     error on RS  @2, final buffer starts at 4    => file:1
    /// \x1e"unterm\n   error at EOF, final buffer empty             => <unknown>
    /// \x1e1{          `{` yields 1 on the last byte; EOF error later => file:0
    /// \x1e"a"{        `"` yields "a", `{` scanned in-call; EOF error => <unknown>
    /// \x1e[0,]\x1e1{  error on `]` @4 in the final buffer           => <unknown>
    /// \x1e1}\n\x1e2{  error on `}` @2, before the final buffer @4   => file:1
    /// ```
    ///
    /// The buffer size is observable, not a detail: `\x1e[0,]` padded with
    /// spaces to 4094 bytes answers `<unknown>`, and one byte more answers
    /// `file:0`, because at 4095 `fgets` stops on the size limit rather
    /// than on EOF, so a further (empty) buffer follows. The same boundary
    /// cuts the other way for `\x1e` + spaces + `1{`: 4094 and 4096 bytes
    /// keep `file:0`, 4095 loses it -- an empty final buffer has no last
    /// byte to complete anything on.
    ///
    /// The `\x1e"unterm\n` row is why a newline cannot simply be read as
    /// "recovery": it restores the position after a record jq has
    /// *finished* rejecting, but a newline swallowed by an unterminated
    /// string is just more string, and the failure still lands at EOF, in
    /// the final buffer.
    ///
    /// Three shapes fall out of this rule rather than needing their own
    /// cases, each oracle-verified:
    ///
    /// - **No RS byte anywhere.** RFC 7464 requires every record to start
    ///   with one, so jq's reader never syncs onto anything and reports
    ///   `Unfinished abandoned text at EOF` -- an error, in the final
    ///   buffer, for *any* content including none at all. A lone empty
    ///   file therefore answers `<unknown>` too.
    /// - **An empty trailing file after real content.** It is opened,
    ///   which re-sets `current_filename` and zeroes the line, and
    ///   contributes no bytes to fail on -- so `a.json` holding `\x1e[0,]`
    ///   and an empty `b.json` reports `b.json:0`, not `<unknown>`.
    ///   Measuring the final buffer from the *last file's* start, not the
    ///   stream's, is what gets this right.
    /// - **A malformed record earlier in the stream**, resynced by a later
    ///   valid record (#1542): its error is not in the final buffer, so the
    ///   position is intact.
    ///
    /// Raw bytes, before the UTF-8 substitution `get_inputs` applies, for
    /// the same reason `seq_no_rs_byte_warning` takes them: an invalid byte
    /// becomes a 3-byte U+FFFD, which both moves every offset and changes
    /// what the reader sees. `\x80` substitutes to `EF BF BD`, whose first
    /// two bytes are a *malformed* BOM prefix -- which flips the reader out
    /// of `WaitingForRs` and makes it read the replacement character as a
    /// record. Handing it the normalized string answered `<unknown>` for
    /// `\x80\x1e`, where jq answers line 0.
    pub(crate) slurp_position_lost: bool,
    /// The byte range of every value jq yields, in order, as absolute
    /// offsets into the concatenated raw sources (BOM bytes counted, the
    /// numbering [`Reader::run`] uses). Every value of the stream: jq 1.8.2
    /// reads to the end, where 1.7.1 could end a non-slurp stream at an empty
    /// record (#2998, #7718).
    ///
    /// Raw, like everything else this walk answers (#3247). The values used
    /// to come from a second walk over the stream *after* UTF-8
    /// substitution, which rewrites the very bytes this reader decides on.
    /// A BOM prefix became a U+FFFD of another width, or an invalid lead
    /// byte became one that read as a malformed BOM (#3195/#3199). Outside
    /// a string, jq's short-tail rule could fold an invalid lead byte *and*
    /// the RS or whitespace after it into one U+FFFD, losing the next value
    /// (`\xe0\x1e"a"`). The caller substitutes each value's own slice
    /// instead. A value the reader accepted has invalid bytes only inside
    /// its strings, and substitution is scoped per string (#1743), so the
    /// result is the same as substituting the whole document.
    pub(crate) values: Vec<(usize, usize)>,
    /// Parallel to `values` (empty under `-s`): each one's `(source index,
    /// line)`, jq's
    /// `input_filename`/`input_line_number` and `(at file:line)` for it
    /// (#3250). jq reports where its reader stood when the value was
    /// *yielded* -- by its closing byte, or for a number or keyword by the
    /// delimiter after it -- not where the value ends: that byte's file, and
    /// the newlines of every `fgets` chunk read by then, the whole chunk
    /// holding it included ([`value_locations`]).
    pub(crate) locations: Vec<(usize, usize)>,
}

/// [`SeqStreamWalk::slurp_position_lost`] on its own: literally
/// [`walk_stream`] with the warnings thrown away. The stderr
/// diagnostics and this question are the same walk, and the ordinary call
/// site runs it once and uses both answers rather than paying for a second
/// pass over the whole stream; this is for the `-s` call sites that run no
/// [`walk_stream`] of their own (#1525's no-RS-byte template, and
/// `--input-dsv`) but still need the location.
///
/// Takes the same raw, pre-UTF-8-substitution sources that function does,
/// and for the same reason -- a substituted byte is a 3-byte U+FFFD, which
/// moves every offset *and* can masquerade as a malformed BOM, whose
/// handling above changes what the reader treats as a record at all.
pub(crate) fn slurp_eof_position_lost(raw_bytes: &[(Option<usize>, Vec<u8>)]) -> bool {
    walk_stream(raw_bytes, true, &mut |_| {}).slurp_position_lost
}

/// The kinds jq's parser distinguishes while classifying a failure. It
/// tracks full values; only these distinctions affect which message fires
/// (`Object keys must be strings` needs `String`, the top-level truncated
/// number rule needs `Number`, and the rest only need "a value is here").
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    String,
    Number,
    /// `true`, `false`, `null`.
    Scalar,
    Array,
    Object,
}

/// One entry of jq's parser stack. `Key` is an object key awaiting its
/// value, which jq pushes on `:` and pops on `,`/`}`.
#[derive(Clone, Copy)]
enum Frame {
    Array { nonempty: bool },
    Object { nonempty: bool },
    Key,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum St {
    Normal,
    Str,
    StrEscape,
    /// jq starts every `--seq` parse here, which is why content before the
    /// first RS byte is discarded without a word.
    WaitingForRs,
}

#[derive(PartialEq, Eq)]
enum Cls {
    Literal,
    Whitespace,
    Structure,
    Quote,
    /// A NUL, which jq master's `classify` refuses (#7966, [`succinctly::jq::INVALID_CHARACTER`]).
    Invalid,
}

fn classify(c: u8) -> Cls {
    match c {
        b' ' | b'\t' | b'\r' | b'\n' => Cls::Whitespace,
        b'"' => Cls::Quote,
        b'[' | b',' | b']' | b'{' | b':' | b'}' => Cls::Structure,
        0 => Cls::Invalid,
        _ => Cls::Literal,
    }
}

/// A diagnostic as the [`Reader`] records it, before [`value_locations`] gives it a place.
struct RecordedFault {
    body: String,
    values_before: usize,
    position_lost: bool,
    /// The offset the reader stood at, or [`YIELDED_AT_EOF`] for the EOF branch.
    at: usize,
}

/// Where [`Reader::run_to`] stopped: the next byte, the next of `boundaries`, and the offset just
/// past the last byte it scanned (where the EOF branch stands).
#[derive(Clone, Copy, Debug, Default)]
struct RunAt {
    boundary: usize,
    eof_offset: usize,
    offset: usize,
}

impl RunAt {
    fn from(skip: usize) -> Self {
        Self {
            offset: skip,
            ..Self::default()
        }
    }
}

struct Reader<'a> {
    line: usize,
    column: usize,
    st: St,
    stack: Vec<Frame>,
    /// jq's `tokenbuf`/`tokenpos`, modelled as a buffer that is *not* cleared when the token is.
    /// jq 1.7.1's `check_literal` read `tokenbuf[1]` without checking `tokenpos`, so a one-byte
    /// `n` token was classified against whatever the previous token left at index 1 (an earlier
    /// `iu` token made it "Invalid literal", and before any token had written index 1 it was
    /// uninitialised heap, #7256). jq 1.8.2 tests `tokenpos > 1` first (jqlang/jq#3324), and so
    /// does [`Reader::check_literal`] since #7525: a bare `n` is always "Invalid numeric literal".
    /// What an earlier token left at index 1 therefore no longer decides anything, though the
    /// buffer, the bytes a resumed reader is seeded with ([`token_byte_after`]) and the test that
    /// holds the two together still model it.
    token_buf: Vec<u8>,
    token_len: usize,
    next: Option<Kind>,
    last_ch_was_ws: bool,
    /// A malformed BOM makes jq re-run `parser_reset` at the top of every
    /// `jv_parser_next`. That call returns on each value, on each error,
    /// and when the buffer runs out -- and jq refills one `fgets` chunk at a
    /// time, up to a newline or 4095 bytes -- so under this flag the parser
    /// is wiped after every value, every newline, and every size-limit
    /// refill ([`refill_boundaries`], #3200) as well. An unterminated string therefore stops swallowing
    /// the rest of the stream, and `\x1e9"-si-\n` reports on `-si-`
    /// because emitting `9` dropped the string it had just opened.
    resets_per_call: bool,
    /// Set by [`Reader::check_done`] when a value was handed off, which is
    /// one of the points jq's reader returns at.
    produced_value: bool,
    /// The byte offset currently being scanned. `check_literal` finishes
    /// before its delimiter, while strings and containers finish on the
    /// current byte, so the completion site records the precise end here.
    offset: usize,
    value_start: Option<usize>,
    value_end: usize,
    /// A value is only committed after its entire `scan()` call succeeds.
    /// jq therefore drops the tentative `1` in `1,2`, but has already handed
    /// off the `1` in `1 {invalid}` when the space scan completed.
    completed: Option<(usize, usize)>,
    values: Vec<(usize, usize)>,
    /// How many values were taken out of `values` before the ones it holds: a walk made as the stream
    /// is read moves them out as it goes ([`SeqStreamWalker`], #7775), and a fault still counts them.
    values_drained: usize,
    /// Parallel to `values`: the absolute offset of the byte whose scan
    /// yielded each one, or [`YIELDED_AT_EOF`] for jq's EOF branch. jq names
    /// a value's position from where its reader stood *then* (#3250).
    yielded_at: Vec<usize>,
    /// See [`walk_stream`]: a malformed BOM plus the empty final `fgets`
    /// buffer jq 1.8.2 always reads last (#7718) means jq's own EOF report is
    /// wiped before it is written. It models jq's reset at that empty buffer,
    /// so [`Reader::finish`] yields no value either: the reset wipes a number
    /// or literal still waiting for the byte after it (`\xef\xbb1 2` is `1`
    /// alone), and the value walk is this walk.
    suppress_eof_warning: bool,
    /// Whether the most recent [`Reader::warn`] came from [`Reader::finish`],
    /// jq's EOF branch. `finish` warns at most once and runs last, so this
    /// is also "did the EOF branch report anything". Tracked rather than
    /// inferred from the offset: a stream whose bytes are *all* consumed as
    /// a BOM prefix scans nothing at all, so the running offset never
    /// leaves 0 and cannot stand in for "at the end".
    last_warning_at_eof: bool,
    /// Set once the byte loop is done, so [`Reader::warn`] can tell jq's
    /// EOF branch apart from a mid-stream detection.
    at_eof: bool,
    /// The raw offset where jq's final `fgets` buffer begins (see
    /// [`final_buffer_start`]). [`walk_stream`] always sets it; the
    /// `usize::MAX` it starts at is past every real offset, so a `Reader`
    /// built without it would silently skip the position rules that hang
    /// off it: `-s`'s EOF-location answer (#3003,
    /// [`SeqStreamWalk::slurp_position_lost`]) and
    /// [`Reader::fault_position_lost`], both jq 1.7.1's (#7607, #7718).
    final_buffer_start: usize,
    /// Whether a *mid-stream* warning (not the EOF branch's) was raised at
    /// an offset `>= final_buffer_start`. Under `-s` that is an early
    /// `return` out of the very `next_input` call that performed the
    /// terminal refill, which is what costs jq its position (#2947/#3003);
    /// see [`SeqStreamWalk::slurp_position_lost`].
    warned_in_final_buffer: bool,
    /// [`walk_stream`]'s `slurp`. jq 1.8.2's slurp loop reads the empty final
    /// buffer before it hands the array on, so the EOF branch's report is the
    /// reading call's, with its position, before the filter runs (#7718); jq
    /// 1.7.1 raised it in the call after when the last byte completed a value
    /// (#3201).
    slurp: bool,
    /// Whether scanning the stream's *last* byte handed a value to the
    /// caller -- jq's `jv_parser_next` returning `OK` with its buffer fully
    /// consumed, so `has_more == 0` and the EOF branch is never reached in
    /// that call (#3003). Captured per byte inside [`Reader::run`]'s loop:
    /// a value [`Reader::finish`] itself completes is not this.
    last_byte_yielded_value: bool,
    /// Whether [`Reader::emit_warning`] records each diagnostic (#5640); see
    /// [`walk_stream_collecting`].
    collect_faults: bool,
    /// The recorded diagnostics: the body and the number of values yielded before it.
    faults: Vec<RecordedFault>,
    /// The stream's whole length in bytes, for [`Reader::fault_position_lost`].
    total: usize,
    /// Each warning, with the offset it was met at ([`walk_stream_collecting_at`]).
    emit: &'a mut dyn FnMut(&str, usize),
}

impl<'a> Reader<'a> {
    fn new(bom: BomPrefix, emit: &'a mut dyn FnMut(&str, usize)) -> Self {
        Self {
            line: 1,
            column: 0,
            // A malformed BOM has already cost jq a `parser_reset`, so it
            // starts in `Normal` and reads pre-RS bytes as a record.
            st: if bom.malformed {
                St::Normal
            } else {
                St::WaitingForRs
            },
            stack: Vec::new(),
            // #7164: a resumed stream's reader has the byte the records before it left.
            token_buf: SEEDED_TOKEN_BYTE
                .with(std::cell::Cell::get)
                .map_or_else(Vec::new, |byte| vec![0, byte]),
            token_len: 0,
            next: None,
            last_ch_was_ws: false,
            resets_per_call: bom.malformed,
            produced_value: false,
            offset: 0,
            value_start: None,
            value_end: 0,
            completed: None,
            values: Vec::new(),
            values_drained: 0,
            yielded_at: Vec::new(),
            suppress_eof_warning: false,
            last_warning_at_eof: false,
            at_eof: false,
            final_buffer_start: usize::MAX,
            warned_in_final_buffer: false,
            slurp: false,
            last_byte_yielded_value: false,
            collect_faults: false,
            faults: Vec::new(),
            total: 0,
            emit,
        }
    }

    fn warn(&mut self, body: &str) {
        self.last_warning_at_eof = self.at_eof;
        if self.offset >= self.final_buffer_start && !self.at_eof {
            self.warned_in_final_buffer = true;
        }
        self.emit_warning(body);
    }

    /// Whether the diagnostic being reported now is raised with jq's position lost (#5640).
    ///
    /// Only the EOF branch's report can be: when the stream's last byte handed a value over, jq
    /// returns that value from its terminal refill and raises the report in the *next* read,
    /// after `read_more` has closed the stream (#3201, #3003) -- so `input` names `<unknown>`
    /// where it would name `file:line` for the same report raised in the reading call. A
    /// mid-stream fault is raised by the call that met it, position intact. That is jq 1.7.1's
    /// position, which jq 1.8.2 keeps the file name for (#7607). Never under `-s`, whose loop reads
    /// the stream to its end before it hands anything on (#7718), so the reading call raises it.
    fn fault_position_lost(&self) -> bool {
        !self.slurp
            && self.at_eof
            && self.last_byte_yielded_value
            && self.final_buffer_start < self.total
    }

    fn emit_warning(&mut self, body: &str) {
        let at = if self.at_eof { AT_EOF } else { self.offset };
        (self.emit)(&ignored_parse_error(body), at);
        if self.collect_faults {
            self.record_fault(body);
        }
    }

    /// Record a diagnostic for [`walk_stream_collecting`], with where the reader stands.
    fn record_fault(&mut self, body: &str) {
        self.faults.push(RecordedFault {
            body: body.to_string(),
            values_before: self.values_drained + self.values.len(),
            position_lost: self.fault_position_lost(),
            at: if self.at_eof {
                YIELDED_AT_EOF
            } else {
                self.offset
            },
        });
    }

    /// jq's `parser_reset`. Note it restores `Normal` -- including over a
    /// `WaitingForRs` that jq's own error path has just assigned, which is
    /// the upstream bug this module reproduces: a malformed BOM starts the
    /// reader in `Normal` (see `Reader::new`) and nothing here ever assigns
    /// `WaitingForRs` again.
    fn reset(&mut self) {
        self.stack.clear();
        self.token_len = 0;
        self.next = None;
        self.st = St::Normal;
        self.value_start = None;
        self.completed = None;
    }

    /// Walk the stream's scanned bytes, applying the malformed-BOM
    /// `parser_reset` jq runs at the top of every `jv_parser_next`: at a
    /// value, a newline or an error as the bytes are scanned, and at each of
    /// `boundaries` ([`refill_boundaries`]).
    ///
    /// Those are where each source after the first begins, and (under a
    /// malformed BOM) where each full 4095-byte `fgets` chunk ends without a
    /// newline (#3200). jq refills its reader one *file* at a time, so a
    /// source boundary is another `jv_parser_next`, and its reset wipes
    /// whatever survived the previous source's scan
    /// (#3002) -- `EF BF` across two sources therefore vanishes where the
    /// same bytes in one buffer fail (`EF BF` then `1E` cleanly starts a new
    /// record, where `EF BF 1E` still reports `Truncated value`). Resetting
    /// on the first scanned byte at or after the boundary covers it; a
    /// boundary whose source yields no byte at all (an empty trailing file,
    /// or content eaten wholesale by the BOM prefix) still cost jq that
    /// refill, so its reset fires before the EOF branch instead.
    ///
    /// Walks `data` from `skip` (the bytes jq's BOM handling consumed, which are never scanned
    /// but are counted in every offset).
    fn run(&mut self, data: &[u8], skip: usize, boundaries: &[usize]) {
        let mut at = RunAt::from(skip);
        self.run_to(&mut at, data, data.len(), boundaries);
        self.run_end(&at, boundaries);
    }

    /// [`Reader::run`]'s byte loop over `data[at.offset..end]`, for a walk that is handed the stream as
    /// it is read (#7775): `at` is where it stopped, and `end` how far the bytes are known. A walk of the
    /// whole stream is one call with `end` its length.
    #[inline]
    fn run_to(&mut self, at: &mut RunAt, data: &[u8], known: usize, boundaries: &[usize]) {
        // Sliced, so the byte loop's bounds are the slice's own, as they were for the whole stream.
        let data = &data[..known];
        let RunAt {
            mut boundary,
            mut eof_offset,
            mut offset,
        } = *at;
        let limit = data.len();
        while offset < limit {
            let ch = data[offset];
            self.offset = offset;
            eof_offset = offset + 1;
            while boundary < boundaries.len() && boundaries[boundary] <= offset {
                if self.resets_per_call {
                    self.reset();
                }
                boundary += 1;
            }
            // #5674: a run of ordinary string bytes is `scan`'s `token_add` and a column each, and
            // nothing else -- no value completes, no diagnostic is raised, no line starts -- so it
            // is taken at once. Not under `resets_per_call`, which wipes the parser at the
            // `fgets` boundaries inside the run.
            if self.st == St::Str && !self.resets_per_call {
                let run = data[offset..limit]
                    .iter()
                    .position(|&b| matches!(b, b'"' | b'\\' | b'\n' | ASCII_RS))
                    .unwrap_or(limit - offset);
                if run > 0 {
                    self.token_add_all(&data[offset..offset + run]);
                    self.column += run;
                    self.last_ch_was_ws = false;
                    self.offset = offset + run - 1;
                    eof_offset = offset + run;
                    self.last_byte_yielded_value = false;
                    self.produced_value = false;
                    offset += run;
                    continue;
                }
            }
            offset += 1;
            if self.st == St::WaitingForRs {
                if ch == ASCII_RS {
                    self.column += 1;
                    self.st = St::Normal;
                    continue;
                }
                // #7263: jq discards each byte before an RS with a line or a column and nothing
                // else, so the run up to the next RS is taken at once, as #5674 takes a run of
                // string bytes. `resets_per_call` is never set here: `WaitingForRs` is assigned
                // only in `Reader::new`, and only for a BOM that is not malformed, while
                // `resets_per_call` is `bom.malformed`; `reset` only ever restores `Normal`. (Were
                // both set, a reset at an `fgets` boundary inside the run would put the parser
                // back to reading, and the run could not be taken at once.)
                debug_assert!(!self.resets_per_call);
                let end = if discards::bytewise() {
                    offset
                } else {
                    find_byte(ASCII_RS, &data[offset..limit]).map_or(limit, |at| offset + at)
                };
                advance_position(&data[offset - 1..end], &mut self.line, &mut self.column);
                self.offset = end - 1;
                eof_offset = end;
                offset = end;
                continue;
            }
            if self.st == St::Normal
                && self.stack.is_empty()
                && self.next.is_none()
                && self.token_len == 0
                && !ch.is_ascii_whitespace()
                && ch != ASCII_RS
            {
                self.value_start = Some(self.offset);
            }
            if let Err(category) = self.scan(ch) {
                let (line, column) = (self.line, self.column);
                if ch == ASCII_RS {
                    self.warn(&format!("{category} at line {line}, column {column}"));
                } else {
                    self.warn(&format!(
                        "{category} at line {line}, column {column} (need RS to resync)"
                    ));
                }
                self.reset();
            } else {
                self.flush_completed();
                if self.resets_per_call && (self.produced_value || ch == b'\n') {
                    self.reset();
                }
            }
            // Captured here, per byte, rather than read after the loop:
            // `finish` re-sets `produced_value` for a value jq's EOF branch
            // completes, which is not an `OK` return on the last byte
            // (#3003).
            self.last_byte_yielded_value = self.produced_value;
            self.produced_value = false;
        }
        *at = RunAt {
            boundary,
            eof_offset,
            offset,
        };
    }

    /// Moves every offset the reader holds `gone` bytes towards the front, for a walk that has let go
    /// of the stream's first `gone` bytes (#7775): where it stands and what it is in. The values,
    /// yields and faults it has recorded are taken out before ([`SeqStreamWalker`]).
    fn rebase(&mut self, gone: usize) {
        debug_assert!(
            self.values.is_empty() && self.yielded_at.is_empty() && self.faults.is_empty()
        );
        let shift = |at: &mut usize| *at = at.saturating_sub(gone);
        shift(&mut self.offset);
        shift(&mut self.value_end);
        if let Some(at) = self.value_start.as_mut() {
            shift(at);
        }
        if let Some((start, end)) = self.completed.as_mut() {
            shift(start);
            shift(end);
        }
        if self.final_buffer_start != usize::MAX {
            shift(&mut self.final_buffer_start);
            shift(&mut self.total);
        }
    }

    /// The end of [`Reader::run`]: what follows the stream's last byte, the EOF branch.
    fn run_end(&mut self, at: &RunAt, boundaries: &[usize]) {
        // A boundary with no scanned byte at or after it (a trailing source
        // that is empty or consumed wholesale by the BOM prefix) still cost
        // jq the refill and its reset -- with a malformed BOM that wipes the
        // pending token before the EOF branch can report on it, so an empty
        // terminal source reads `EF BF` as nothing at all.
        if self.resets_per_call {
            for _ in at.boundary..boundaries.len() {
                self.reset();
            }
        }
        self.offset = at.eof_offset;
        self.at_eof = true;
        self.finish();
    }

    fn flush_completed(&mut self) {
        if let Some(range) = self.completed.take() {
            self.values.push(range);
            self.yielded_at.push(if self.at_eof {
                YIELDED_AT_EOF
            } else {
                self.offset
            });
            self.produced_value = true;
        }
    }

    /// jq's `scan`.
    fn scan(&mut self, ch: u8) -> Result<(), &'static str> {
        self.column += 1;
        if ch == b'\n' {
            self.line += 1;
            self.column = 0;
        }

        if ch == ASCII_RS {
            if self.check_truncation() {
                // jq consults `check_literal` for its side effect here: a
                // pending *number* token becomes the value that makes this
                // the softer "potentially truncated" case, while anything
                // else (an unterminated string's contents, say) fails to
                // parse as a literal and collapses to "Truncated value".
                if self.check_literal().is_ok() && self.is_top_num() {
                    return Err("Potentially truncated top-level numeric value");
                }
                return Err("Truncated value");
            }
            self.check_literal()?;
            // Unreachable in practice, and kept because jq has it: a
            // completed top-level value is cleared by `check_done` the
            // moment it completes, so nothing is ever still pending here.
            if self.st == St::Normal && self.check_done() {
                return Ok(());
            }
            // jq's own "shouldn't happen" branch: `jv_parser_next` returns
            // a message-less invalid here (`parser_reset` then
            // `*out = jv_invalid()`), which jq 1.8.2's caller
            // (`jq_util_input_next_input`, `src/util.c`) reads past: it ends
            // only once the last file is closed. jq 1.7.1's ended the whole
            // stream here when this was the first event of the call that read
            // the last file's last bytes (#2998), and `input` raised `break`
            // (#5662); the reader follows 1.8.2 (#7718).
            self.reset();
            return Ok(());
        }

        self.last_ch_was_ws = false;
        if self.st == St::Normal {
            let cls = classify(ch);
            if cls == Cls::Whitespace {
                self.last_ch_was_ws = true;
            }
            if cls != Cls::Literal {
                self.check_literal()?;
                self.check_done();
            }
            match cls {
                Cls::Literal => self.token_add(ch),
                // A value the token before it completed is dropped with the fault (`reset`).
                Cls::Invalid => return Err(succinctly::jq::INVALID_CHARACTER),
                Cls::Whitespace => {}
                Cls::Quote => {
                    // As with `1[1]`, a pending scalar can be followed by
                    // an adjacent self-delimiting root string.
                    if self.stack.is_empty() && self.next.is_none() {
                        self.value_start = Some(self.offset);
                    }
                    self.st = St::Str;
                }
                Cls::Structure => self.token_structure(ch)?,
            }
            self.check_done();
        } else if ch == b'"' && self.st == St::Str {
            self.found_string()?;
            self.st = St::Normal;
            self.check_done();
        } else {
            self.token_add(ch);
            self.st = if ch == b'\\' && self.st == St::Str {
                St::StrEscape
            } else {
                St::Str
            };
        }
        Ok(())
    }

    /// [`Reader::token_add`] for each byte of `bytes`. The growth differs from byte-at-a-time
    /// (the buffer only ever holds zeros past `token_len` or bytes an earlier token left, in both),
    /// and `check_literal` reads nothing past index 1.
    fn token_add_all(&mut self, bytes: &[u8]) {
        let end = self.token_len + bytes.len();
        while end > self.token_buf.len() {
            self.token_buf.resize(self.token_buf.len() * 2 + 256, 0);
        }
        self.token_buf[self.token_len..end].copy_from_slice(bytes);
        self.token_len = end;
    }

    /// jq's `tokenadd`, including its growth policy -- the buffer only
    /// ever grows, which is what leaves stale bytes visible to
    /// `check_literal`.
    fn token_add(&mut self, ch: u8) {
        if self.token_len == self.token_buf.len() {
            self.token_buf.resize(self.token_buf.len() * 2 + 256, 0);
        }
        self.token_buf[self.token_len] = ch;
        self.token_len += 1;
    }

    /// jq's `seq_check_truncation`: whether an RS byte has arrived while
    /// something is still in flight. Trailing whitespace resolves it,
    /// which is why `\x1e1 \x1e` is clean but `\x1e1\x1e` is not.
    fn check_truncation(&self) -> bool {
        !self.last_ch_was_ws
            && (!self.stack.is_empty() || self.token_len > 0 || self.next == Some(Kind::Number))
    }

    fn is_top_num(&self) -> bool {
        self.stack.is_empty() && self.next == Some(Kind::Number)
    }

    /// jq's `parse_check_done`: a completed top-level value is handed to
    /// the caller and cleared. Modelling this is what makes `\x1e{}extra`
    /// report on `extra` rather than "Expected separator between values".
    fn check_done(&mut self) -> bool {
        if self.stack.is_empty() && self.next.is_some() {
            self.next = None;
            if let Some(start) = self.value_start.take() {
                self.completed = Some((start, self.value_end));
            }
            true
        } else {
            false
        }
    }

    /// jq's `value`.
    fn value(&mut self, kind: Kind) -> Result<(), &'static str> {
        if self.next.is_some() {
            return Err("Expected separator between values");
        }
        self.next = Some(kind);
        Ok(())
    }

    /// jq's `parse_token`.
    fn token_structure(&mut self, ch: u8) -> Result<(), &'static str> {
        match ch {
            b'[' | b'{' => {
                if self.stack.len() >= JQ_READER.max_depth {
                    return Err("Exceeds depth limit for parsing");
                }
                if self.next.is_some() {
                    return Err("Expected separator between values");
                }
                // A pending scalar can be followed immediately by a
                // self-delimiting root value (`1[1]`). Its completion was
                // queued earlier in this scan call; this delimiter begins
                // the next root value.
                if self.stack.is_empty() {
                    self.value_start = Some(self.offset);
                }
                self.stack.push(if ch == b'[' {
                    Frame::Array { nonempty: false }
                } else {
                    Frame::Object { nonempty: false }
                });
            }
            b':' => {
                let Some(kind) = self.next else {
                    return Err("Expected string key before ':'");
                };
                if !matches!(self.stack.last(), Some(Frame::Object { .. })) {
                    return Err("':' not as part of an object");
                }
                if kind != Kind::String {
                    return Err("Object keys must be strings");
                }
                self.stack.push(Frame::Key);
                self.next = None;
            }
            b',' => {
                if self.next.is_none() {
                    return Err("Expected value before ','");
                }
                match self.stack.last() {
                    // Same as the RS branch above: at the top level the
                    // pending value is already gone, so the `is_none`
                    // check has answered first. jq has the arm; so do we.
                    None => return Err("',' not as part of an object or array"),
                    Some(Frame::Array { .. }) => {
                        if let Some(Frame::Array { nonempty }) = self.stack.last_mut() {
                            *nonempty = true;
                        }
                    }
                    Some(Frame::Key) => {
                        self.stack.pop();
                        if let Some(Frame::Object { nonempty }) = self.stack.last_mut() {
                            *nonempty = true;
                        }
                    }
                    // Reached by input like `{"a", "b"}`.
                    Some(Frame::Object { .. }) => {
                        return Err("Objects must consist of key:value pairs")
                    }
                }
                self.next = None;
            }
            b']' => {
                let Some(Frame::Array { nonempty }) = self.stack.last().copied() else {
                    return Err("Unmatched ']'");
                };
                if self.next.is_some() {
                    self.next = None;
                } else if nonempty {
                    // Reached by input like `[1,2,3,]`.
                    return Err("Expected another array element");
                }
                self.stack.pop();
                self.next = Some(Kind::Array);
                self.value_end = self.offset + 1;
            }
            b'}' => {
                if self.stack.is_empty() {
                    return Err("Unmatched '}'");
                }
                if self.next.is_some() {
                    if !matches!(self.stack.last(), Some(Frame::Key)) {
                        return Err("Objects must consist of key:value pairs");
                    }
                    self.stack.pop();
                    if let Some(Frame::Object { nonempty }) = self.stack.last_mut() {
                        *nonempty = true;
                    }
                    self.next = None;
                } else {
                    match self.stack.last() {
                        Some(Frame::Object { nonempty }) => {
                            if *nonempty {
                                return Err("Expected another key-value pair");
                            }
                        }
                        _ => return Err("Unmatched '}'"),
                    }
                }
                self.stack.pop();
                self.next = Some(Kind::Object);
                self.value_end = self.offset + 1;
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
        // Only `t`, `f` and `nu` take the keyword path. A bare `n` (and so
        // `nan`) falls through to the number path, which is why `\x1en`
        // reports "Invalid numeric literal" and `\x1enul` reports
        // "Invalid literal", whatever an earlier token left in the buffer.
        let pattern: Option<&[u8]> = match self.token_buf[0] {
            b't' => Some(b"true"),
            b'f' => Some(b"false"),
            // jq 1.8.2 reads index 1 only when the token has one (`tokenpos > 1`, #7525);
            // 1.7.1 read whatever an earlier token left there.
            b'n' if self.token_len > 1 && self.token_buf[1] == b'u' => Some(b"null"),
            _ => None,
        };
        let kind = match pattern {
            Some(pattern) => {
                if &self.token_buf[..self.token_len] != pattern {
                    return Err("Invalid literal");
                }
                Kind::Scalar
            }
            None => {
                // jq NUL-terminates the token in place before handing it
                // to its number parser. That write is what scrubs the
                // stale byte read above: a one-character number token
                // zeroes index 1, so a later bare `n` is classified as a
                // number rather than against some earlier token's `u`.
                if self.token_len == self.token_buf.len() {
                    self.token_buf.resize(self.token_buf.len() * 2 + 256, 0);
                }
                self.token_buf[self.token_len] = 0;
                // ... and so reads it as a C string. A bare token holds no NUL (#7966), but an RS asks
                // whether an open string's bytes are a number, and those end at a NUL: `"-9` then
                // NUL is the number `-9`.
                let token = &self.token_buf[..self.token_len];
                let number = if self.st == St::Normal {
                    token
                } else {
                    c_string(token)
                };
                if !number_is_valid(number) {
                    return Err("Invalid numeric literal");
                }
                Kind::Number
            }
        };
        self.value(kind)?;
        self.value_end = self.offset;
        self.token_len = 0;
        Ok(())
    }

    /// jq's `found_string`: the escape validation, which is all that can raise a diagnostic, and
    /// the decode jq does in place over `tokenbuf` (#6513), in [`decode_string_in_place`]. What
    /// `check_literal` reads at index 1 afterwards is the second byte of the decoded text when it
    /// has two or more, and the byte the string held when it has one. See [`Reader::token_buf`].
    fn found_string(&mut self) -> Result<(), &'static str> {
        decode_string_in_place(&mut self.token_buf[..self.token_len])?;
        self.value(Kind::String)?;
        self.value_end = self.offset + 1;
        self.token_len = 0;
        Ok(())
    }

    /// jq's EOF branch in `jv_parser_next`. At most one diagnostic: jq
    /// sets `p->eof` before reaching any of these, so the reader never
    /// runs again.
    fn finish(&mut self) {
        if self.suppress_eof_warning {
            return;
        }
        let (line, column) = (self.line, self.column);
        if self.st == St::WaitingForRs {
            // #1525's template. The *stderr* call site never reaches this
            // -- `walk_stream` routes a stream with no RS byte to
            // `seq_no_rs_byte_warning` instead -- but
            // `slurp_eof_position_lost` does, and depends on it: this arm
            // is what makes an RS-less stream (down to an empty one, which
            // jq abandons just the same) answer `<unknown>` rather than a
            // position. Not dead code.
            self.warn(&format!(
                "Unfinished abandoned text at EOF at line {line}, column {column}"
            ));
            return;
        }
        if self.st != St::Normal {
            self.warn(&format!(
                "Unfinished string at EOF at line {line}, column {column}"
            ));
            return;
        }
        if let Err(category) = self.check_literal() {
            self.warn(&format!(
                "{category} at EOF at line {line}, column {column}"
            ));
            return;
        }
        if !self.stack.is_empty() {
            self.warn(&format!(
                "Unfinished JSON term at EOF at line {line}, column {column}"
            ));
            return;
        }
        if !self.last_ch_was_ws && self.next == Some(Kind::Number) {
            self.warn(&format!(
                "Potentially truncated top-level numeric value at EOF at line {line}, column {column}"
            ));
        } else {
            self.check_done();
            self.flush_completed();
        }
    }
}

/// The byte an escape of one character stands for (`\n` is a newline, `\/` a slash), which is
/// every escape but `\\u`.
fn simple_escape(escape: u8) -> Option<u8> {
    match escape {
        b'\\' | b'"' | b'/' => Some(escape),
        b'b' => Some(0x08),
        b'f' => Some(0x0c),
        b't' => Some(b'\t'),
        b'n' => Some(b'\n'),
        b'r' => Some(b'\r'),
        _ => None,
    }
}

/// jq's `found_string` over the bytes of a string token: the escape validation and the decode jq
/// does *in place* over `tokenbuf` (#6513), the one both `--seq` and `--stream` parsers share. The
/// text the string decodes to is written from index 0 and nothing clears what follows, so what
/// `check_literal` reads at index 1 afterwards is the second byte of the decoded text when it has
/// two or more, and the byte the string held when it has one: after `"a"` or the escape for `A` the
/// buffer reads `a` or `u`, after the escape for `U+00E9`, a lone surrogate or a pair it reads a
/// byte of the encoding. The diagnostic of the first fault is the error, and what was decoded
/// before it stays. A decoded escape is never longer than its text, so the write never passes the
/// read.
pub(crate) fn decode_string_in_place(token: &mut [u8]) -> Result<(), &'static str> {
    // The bytes before the first backslash or control byte decode to themselves.
    let mut i = token
        .iter()
        .position(|&c| c == b'\\' || c <= 0x1f)
        .unwrap_or(token.len());
    let mut out = i;
    while i < token.len() {
        let c = token[i];
        i += 1;
        if c != b'\\' {
            // jq compares a *signed* char, so only U+0000..=U+001F
            // qualifies -- bytes >= 0x80 are negative there and pass.
            if c <= 0x1f {
                return Err(
                    "Invalid string: control characters from U+0000 through U+001F must be escaped",
                );
            }
            token[out] = c;
            out += 1;
            continue;
        }
        // Unreachable: a trailing `\\` puts the scanner in `StrEscape`, so the next byte is
        // consumed as the escaped one and the string cannot end here. jq carries the arm anyway.
        if i >= token.len() {
            return Err("Expected escape character at end of string");
        }
        let escape = token[i];
        i += 1;
        if let Some(byte) = simple_escape(escape) {
            token[out] = byte;
            out += 1;
            continue;
        }
        if escape != b'u' {
            return Err("Invalid escape");
        }
        if i + 4 > token.len() {
            return Err("Invalid \\uXXXX escape");
        }
        let Some(mut codepoint) = unhex4(&token[i..i + 4]) else {
            return Err("Invalid characters in \\uXXXX escape");
        };
        i += 4;
        if (0xD800..=0xDBFF).contains(&codepoint) {
            if i + 6 > token.len() || token[i] != b'\\' || token[i + 1] != b'u' {
                return Err("Invalid \\uXXXX\\uXXXX surrogate pair escape");
            }
            match unhex4(&token[i + 2..i + 6]) {
                Some(low) if (0xDC00..=0xDFFF).contains(&low) => {
                    codepoint = 0x10000 + (((codepoint - 0xD800) << 10) | (low - 0xDC00));
                }
                _ => return Err("Invalid \\uXXXX\\uXXXX surrogate pair escape"),
            }
            i += 6;
        }
        debug_assert!(out + 4 <= token.len() || codepoint <= 0xffff);
        out += encode_utf8_unchecked(codepoint, &mut token[out..]);
        debug_assert!(out <= i);
    }
    Ok(())
}

/// jq's `jvp_utf8_encode`: `codepoint` as UTF-8 at the start of `out`, returning the length. It
/// takes a surrogate for a three-byte code point as any other (`U+DC00` is `ED B0 80`), which is
/// what [`Reader::found_string`] leaves in the token buffer.
fn encode_utf8_unchecked(codepoint: u32, out: &mut [u8]) -> usize {
    if codepoint <= 0x7f {
        out[0] = codepoint as u8;
        1
    } else if codepoint <= 0x7ff {
        out[0] = 0xc0 | (codepoint >> 6) as u8;
        out[1] = 0x80 | (codepoint & 0x3f) as u8;
        2
    } else if codepoint <= 0xffff {
        out[0] = 0xe0 | (codepoint >> 12) as u8;
        out[1] = 0x80 | ((codepoint >> 6) & 0x3f) as u8;
        out[2] = 0x80 | (codepoint & 0x3f) as u8;
        3
    } else {
        out[0] = 0xf0 | (codepoint >> 18) as u8;
        out[1] = 0x80 | ((codepoint >> 12) & 0x3f) as u8;
        out[2] = 0x80 | ((codepoint >> 6) & 0x3f) as u8;
        out[3] = 0x80 | (codepoint & 0x3f) as u8;
        4
    }
}

pub(crate) fn unhex4(bytes: &[u8]) -> Option<u32> {
    let mut value = 0u32;
    for &b in bytes {
        value = (value << 4) | (b as char).to_digit(16)?;
    }
    Some(value)
}

/// Whether jq's number parser accepts this token.
///
/// jq builds with decNumber, whose grammar is materially wider than RFC
/// 8259 -- so this deliberately does *not* reuse
/// [`succinctly::json::validate::is_valid_number`]. All oracle-verified
/// against the pinned binary: `1.`, `.5`, `+1` and `01` are accepted,
/// while `1e`, `1e+`, `-` and `1.2.3` are not, and decNumber's special
/// forms (`nan`, `NaN5` with a payload, `snan`, `inf`, `-Infinity`) are
/// numbers too. Getting this wrong shows up as "Invalid numeric literal"
/// where jq says "Potentially truncated top-level numeric value", or vice
/// versa.
pub(crate) fn number_is_valid(token: &[u8]) -> bool {
    // The special-value words (`nan`, `sNaN12`, `-Infinity`, `+inf`) are
    // one shared definition since #2877 -- the document dispatchers and
    // the lenient validator read the same grammar from the same function.
    if succinctly::json::validate::jq_special_number(token).is_some() {
        return true;
    }
    let body = match token.first() {
        Some(b'+' | b'-') => &token[1..],
        _ => token,
    };
    if body.is_empty() {
        return false;
    }

    let (mantissa, exponent) = match body.iter().position(|&b| b == b'e' || b == b'E') {
        Some(i) => (&body[..i], Some(&body[i + 1..])),
        None => (body, None),
    };
    let (integer, fraction) = match mantissa.iter().position(|&b| b == b'.') {
        Some(i) => (&mantissa[..i], Some(&mantissa[i + 1..])),
        None => (mantissa, None),
    };
    // `map_or(true, ..)` rather than `is_none_or`: clippy's `msrv`
    // (clippy.toml) is 1.73 and `Option::is_none_or` is 1.82.
    if integer.is_empty() && fraction.map_or(true, <[u8]>::is_empty) {
        return false;
    }
    if !integer.iter().all(u8::is_ascii_digit) {
        return false;
    }
    if fraction.is_some_and(|f| !f.iter().all(u8::is_ascii_digit)) {
        return false;
    }
    if let Some(exponent) = exponent {
        let digits = match exponent.first() {
            Some(b'+' | b'-') => &exponent[1..],
            _ => exponent,
        };
        if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn warnings(sources: &[&[u8]]) -> Vec<String> {
        let owned: Vec<(Option<usize>, Vec<u8>)> =
            sources.iter().map(|s| (None, s.to_vec())).collect();
        let mut got = Vec::new();
        // Non-slurp: these templates were all captured against plain
        // `jq --seq -c .`, not `-s`.
        walk_stream(&owned, false, &mut |w| {
            got.push(
                w.strip_prefix("jq: ignoring parse error: ")
                    .expect("every warning carries jq's prefix")
                    .to_string(),
            );
        });
        got
    }

    fn assert_warnings(input: &[u8], expected: &[&str]) {
        assert_eq!(warnings(&[input]), expected, "input: {input:?}");
    }

    /// #5674: the walk takes a run of ordinary string bytes at once. A string long enough to
    /// grow the token buffer, one cut by every byte the run stops at, and one ending the stream
    /// all give the values and warnings the byte-at-a-time walk gives.
    #[test]
    fn string_runs_are_walked_as_bytes_5674() {
        let long = "x".repeat(700);
        let stream = format!("\x1e{{\"a\":\"{long}\",\"b\":\"p\\\"q\\\\r\"}}\n\x1e\"{long}\"\n");
        let bytes = stream.as_bytes();
        let first_end = stream.find("}\n").unwrap() + 1;
        assert_eq!(
            walk_values(&[bytes], false),
            vec![(1, first_end), (first_end + 2, bytes.len() - 1)]
        );
        assert_warnings(bytes, &[]);
        // Cut inside a long string by the next RS: the same warning at the same place.
        let cut = format!("\x1e\"{long}\x1e1\n");
        assert_warnings(cut.as_bytes(), &["Truncated value at line 1, column 703"]);
        // A string with a raw newline in it counts the line, and one still open at EOF reports it.
        assert_warnings(
            format!("\x1e\"{long}\n{long}").as_bytes(),
            &["Unfinished string at EOF at line 2, column 700"],
        );
    }

    /// #7966: a NUL outside a string is `Invalid character` at its own column, the token before it
    /// checked first and a value it completes dropped, as jq master (fbee8f4, jqlang/jq#3606) reads
    /// it. jq 1.7.1 never saw the bytes from a NUL to the end of an `fgets` read with no newline, and
    /// read a NUL glued to a number as the number's end (#7340), as 1.8.2 does; neither is
    /// reproduced. Each row was captured from a driver over master's `jv_parse.c` with
    /// `JV_PARSE_SEQ`:
    ///
    /// | input                      | jq master                                                         |
    /// |----------------------------|-------------------------------------------------------------------|
    /// | `\x1e[1\0\n\x1e2\n`        | `Invalid character at line 1, column 4 (need RS to resync)`, `2`  |
    /// | `\x1e2.50\0\n\x1e3\n`      | `Invalid character at line 1, column 6 (need RS to resync)`, `3`  |
    /// | `\x1etrue\0\n\x1e\0\n`     | `Invalid character` at line 1, column 6 and at line 2, column 2   |
    /// | `\x1e"ab" \0\n`            | `"ab"`, `Invalid character at line 1, column 7 (need RS to resync)` |
    /// | `\x1e1 \0\x1e2\x1e3\n`     | `1`, `Invalid character` at column 4, `Potentially truncated` at column 7, `3` |
    /// | `\x1e"-9\0x\x1e1\n`        | `Potentially truncated top-level numeric value at line 1, column 7`, `1` |
    /// | `\x1e"a\0b"\n\x1e1\n`      | the control-character error at line 1, column 6, `1`              |
    ///
    /// Only white space comes between each NUL and the next RS: after a fault, jq master waits for
    /// the RS where both releases read on (their `parser_reset` undoes the wait, which master fixed
    /// in the same series), and what follows a fault is the reader's best effort (E1, #7967). So
    /// master's `Unfinished abandoned text at EOF` after a fault the stream ends in is not asserted
    /// here. Before #7966 the first two read `[1]` and `2.50`, the third was `Invalid literal` and
    /// `Invalid numeric literal`, and the bytes after a NUL in a read with no newline were never
    /// scanned.
    #[test]
    fn a_nul_outside_a_string_is_invalid_character_7966() {
        let texts = |input: &[u8]| -> Vec<String> {
            let owned = vec![(None, input.to_vec())];
            let walk = walk_stream(&owned, false, &mut |_| {});
            walk.values
                .iter()
                .map(|&(start, end)| String::from_utf8(input[start..end].to_vec()).unwrap())
                .collect()
        };
        assert_warnings(
            b"\x1e[1\x00\n\x1e2\n",
            &["Invalid character at line 1, column 4 (need RS to resync)"],
        );
        assert_eq!(texts(b"\x1e[1\x00\n\x1e2\n"), ["2"]);
        assert_warnings(
            b"\x1e2.50\x00\n\x1e3\n",
            &["Invalid character at line 1, column 6 (need RS to resync)"],
        );
        assert_eq!(texts(b"\x1e2.50\x00\n\x1e3\n"), ["3"]);
        assert_warnings(
            b"\x1etrue\x00\n\x1e\x00\n",
            &[
                "Invalid character at line 1, column 6 (need RS to resync)",
                "Invalid character at line 2, column 2 (need RS to resync)",
            ],
        );
        assert_warnings(
            b"\x1e\"ab\" \x00\n",
            &["Invalid character at line 1, column 7 (need RS to resync)"],
        );
        assert_eq!(texts(b"\x1e\"ab\" \x00\n"), ["\"ab\""]);
        assert_warnings(
            b"\x1e1 \x00\x1e2\x1e3\n",
            &[
                "Invalid character at line 1, column 4 (need RS to resync)",
                "Potentially truncated top-level numeric value at line 1, column 7",
            ],
        );
        assert_eq!(texts(b"\x1e1 \x00\x1e2\x1e3\n"), ["1", "3"]);
        // Unchanged: an RS asks whether an open string's bytes are a number, and those still end
        // at a NUL (`"-9` then NUL is the number `-9`); a NUL in a string is a control character.
        assert_warnings(
            b"\x1e\"-9\x00x\x1e1\n",
            &["Potentially truncated top-level numeric value at line 1, column 7"],
        );
        assert_warnings(
            b"\x1e\"a\x00b\"\n\x1e1\n",
            &["Invalid string: control characters from U+0000 through U+001F must be escaped at line 1, column 6 (need RS to resync)"],
        );
        // A NUL in a read with no newline is scanned like any other: past a full read, the fault
        // is at its own column (master: `"ab"`, the fault, `4`), and an RS after it is an RS.
        let pad = |head: &[u8], tail: &[u8]| [head, &[b' '; 4200], tail].concat();
        let padded = pad(b"\x1e\"ab\" \x00", b"\n\x1e4\n");
        assert_warnings(
            &padded,
            &["Invalid character at line 1, column 7 (need RS to resync)"],
        );
        assert_eq!(texts(&padded), ["\"ab\"", "4"]);
        assert_eq!(
            texts(&pad(b"\x1e1 \x00\x1e2\x1e3", b"\n\x1e4\n")),
            ["1", "3", "4"]
        );
    }

    /// #7966: every byte takes part in the BOM, a NUL included: `EF BB` then NUL is a malformed BOM,
    /// whether or not the read it is in ends in a newline. Before #7966 a NUL that ended a read with
    /// no newline was no byte to the match (#7340), so the BOM could still complete after it.
    /// `consumed` is where the scanner starts.
    #[test]
    fn bom_prefix_reads_every_byte_7966() {
        let one = |raw: &[u8]| bom_prefix(&[(None, raw.to_vec())]);
        let bom = |consumed, malformed| BomPrefix {
            consumed,
            malformed,
        };
        assert_eq!(one(b"\xef\xbb\x00x"), bom(2, true));
        assert_eq!(one(b"\xef\xbb\x00x\n"), bom(2, true));
        let split = [&b"\xef\xbb\x00"[..], &[b' '; 4092], b"\xbf\x1e1\n"].concat();
        assert_eq!(one(&split), bom(2, true));
        let files = [(None, b"\xef".to_vec()), (None, b"\x00x".to_vec())];
        assert_eq!(bom_prefix(&files), bom(1, true));
        let across = [
            (None, b"\xef\xbb".to_vec()),
            (None, b"\xbf\x1e1\n".to_vec()),
        ];
        assert_eq!(bom_prefix(&across), bom(3, false));
        // Without a NUL, as before.
        assert_eq!(one(b"\xef\xbb\xbf\x1e1\n"), bom(3, false));
        assert_eq!(one(b"\xef1"), bom(1, true));
        assert_eq!(one(b"\x1e1"), bom(0, false));
    }

    /// [`SeqStreamWalk::values`] over the given sources.
    fn walk_values(sources: &[&[u8]], slurp: bool) -> Vec<(usize, usize)> {
        let owned: Vec<(Option<usize>, Vec<u8>)> =
            sources.iter().map(|s| (None, s.to_vec())).collect();
        walk_stream(&owned, slurp, &mut |_| {}).values
    }

    /// The decoded texts of the values that survive for one single-source
    /// stream, the way `build_seq_values` slices them in production.
    fn seq_value_texts(input: &[u8], slurp: bool) -> Vec<String> {
        walk_values(&[input], slurp)
            .into_iter()
            .map(|(start, end)| String::from_utf8(input[start..end].to_vec()).unwrap())
            .collect()
    }

    /// #2653: values are committed only after the scan call which completed
    /// them has succeeded. Whitespace commits `1` before a malformed object;
    /// a comma instead causes the same scan call to fail, so it is discarded.
    #[test]
    fn values_follow_jq_recovery_2653() {
        for slurp in [false, true] {
            assert_eq!(walk_values(&[b"\x1e1 {invalid\n"], slurp), vec![(1, 2)]);
            assert_eq!(walk_values(&[b"\x1e1,2\n"], slurp), vec![(3, 4)]);
            assert_eq!(walk_values(&[b"\x1e1-2\n"], slurp), vec![]);
            assert_eq!(walk_values(&[b"\x1e5-3 7\n"], slurp), vec![(5, 6)]);
        }
    }

    /// jq 1.7.1's non-slurp `--seq` driver ended the *entire* stream
    /// silently at the first empty record of its own final `fgets` buffer
    /// (#2998); jq 1.8.2 reads every record (#7718), and so does the walk.
    /// Every row is `/opt/homebrew/bin/jq` 1.8.2's (1.7.1 printed nothing for
    /// each, and no warning).
    #[test]
    fn an_empty_record_in_the_last_buffer_does_not_end_the_stream_7718() {
        for (input, values) in [
            (&b"\x1e\x1etrue"[..], &["true"][..]),
            (b"\x1e\x1e\"s\"", &["\"s\""]),
            (b"\x1e \x1e{}", &["{}"]),
            (b"\x1e\n\x1etrue", &["true"]),
            (b"\x1e1\n\x1etrue", &["1", "true"]),
            (b"\x1e1\n\x1e\x1e\"abc", &["1"]),
            (b"\x1e\x1e5", &[]),
        ] {
            assert_eq!(seq_value_texts(input, false), values, "input: {input:?}");
        }
        // What the EOF branch reports is reported: 1.7.1 never reached it here.
        assert_warnings(
            b"\x1e\x1e5",
            &["Potentially truncated top-level numeric value at EOF at line 1, column 3"],
        );
        assert_warnings(
            b"\x1e1\n\x1e\x1e\"abc",
            &["Unfinished string at EOF at line 2, column 6"],
        );
        assert_warnings(b"\x1e1\n\x1etrue", &[]);
    }

    /// An earlier value or error in the same buffer never mattered, and
    /// still does not.
    #[test]
    fn an_empty_record_after_a_value_in_the_same_buffer_is_read_past_2998() {
        assert_eq!(seq_value_texts(b"\x1e1 \x1etrue", false), vec!["1", "true"]);
        assert_eq!(
            seq_value_texts(b"\x1e{}\x1etrue", false),
            vec!["{}", "true"]
        );
        assert_eq!(
            seq_value_texts(b"\x1e\"a\"\x1etrue", false),
            vec!["\"a\"", "true"]
        );
        // The first event is an *error* (truncated `1`).
        assert_eq!(
            seq_value_texts(b"\x1e1\x1e2 \x1etrue", false),
            vec!["2", "true"]
        );
    }

    /// Controls: a trailing newline, which empties jq 1.7.1's final buffer
    /// too, so both releases read these alike.
    #[test]
    fn a_trailing_newline_reads_alike_in_both_releases_2998() {
        assert_eq!(seq_value_texts(b"\x1e\x1etrue\n", false), vec!["true"]);
        assert_eq!(
            seq_value_texts(b"\x1e\x1etrue\n\x1e5\n", false),
            vec!["true", "5"]
        );
    }

    /// `-s` reads alike in both releases: jq 1.7.1's `has_more` kept its read
    /// loop going past the empty record too.
    #[test]
    fn an_empty_record_under_slurp_is_read_past_2998() {
        assert_eq!(seq_value_texts(b"\x1e\x1etrue", true), vec!["true"]);
        assert_eq!(
            warnings_slurp(&[b"\x1e1\n\x1e\x1e\"abc"]),
            ["Unfinished string at EOF at line 2, column 6"],
        );
    }

    fn warnings_slurp(sources: &[&[u8]]) -> Vec<String> {
        let owned: Vec<(Option<usize>, Vec<u8>)> =
            sources.iter().map(|s| (None, s.to_vec())).collect();
        let mut got = Vec::new();
        walk_stream(&owned, true, &mut |w| {
            got.push(
                w.strip_prefix("jq: ignoring parse error: ")
                    .expect("every warning carries jq's prefix")
                    .to_string(),
            );
        });
        got
    }

    fn position_lost(sources: &[&[u8]]) -> bool {
        let owned: Vec<(Option<usize>, Vec<u8>)> =
            sources.iter().map(|s| (None, s.to_vec())).collect();
        slurp_eof_position_lost(&owned)
    }

    /// `--seq -s`'s EOF-location rule, on the shapes #2947 tabulated and the
    /// same-call exit #3003 adds. Every row is jq 1.7.1's own answer
    /// (`(at file:N)` = kept, `(at <unknown>)` = lost); the CLI tests pin
    /// the rendered line, this pins the walk's answer without a process
    /// spawn.
    #[test]
    fn slurp_position_is_lost_exactly_when_jqs_terminal_call_returns_an_error_2947_3003() {
        // #2947: an error in the final buffer, or at EOF, loses it; one an
        // earlier refill can recover from does not.
        for (stream, lost) in [
            (&b"\x1e[0,]"[..], true),
            (b"\x1e[0,]\n", false),
            (b"\x1e0\x1e", true),
            (b"\x1e0\x1e\n", false),
            (b"\x1e\"unterm\n", true),
            (b"\x1e1\n\x1e[0,]", true),
            (b"\x1e1\n\x1e[0,]\n", false),
            (b"", true),
            (b"1\n", true),
            (b"\xef\xbb\xbf", true),
        ] {
            assert_eq!(position_lost(&[stream]), lost, "{stream:?}");
        }
        // #3003: the final buffer's last byte completing a value ends the
        // terminal call before its EOF branch -- unless an error in that
        // buffer already returned early, or the buffer is empty.
        for (stream, lost) in [
            (&b"\x1e1{"[..], false),
            (b"\x1e1[", false),
            (b"\x1etrue{", false),
            (b"\x1etrue\"", false),
            (b"\x1e1 2{", false),
            (b"\x1e1 \x1e2{", false),
            (b"\x1e1\n\x1e2{", false),
            (b"\x1e1}\n\x1e2{", false),
            (b"\xef\xbb\xbf\x1e1{", false),
            // Value completed before the last byte: `{` is scanned in-call.
            (b"\x1e\"a\"{", true),
            (b"\x1e{}{", true),
            (b"\x1e[1]{", true),
            (b"\x1e1 {", true),
            // An error in the final buffer.
            (b"\x1e[0,]\x1e1{", true),
            (b"\x1e1\x1e{", true),
            (b"\x1e1{\x1e", true),
            (b"\x1e1{\"", true),
            // The issue's own controls.
            (b"\x1e1{ ", true),
            (b"\x1e1{\n", true),
            (b"\x1e1}", true),
            (b"\x1e{", true),
            // Nothing unfinished at all.
            (b"\x1etrue", false),
            (b"\x1e1{}", false),
            (b"\x1e1{ \x1e", false),
        ] {
            assert_eq!(position_lost(&[stream]), lost, "{stream:?}");
        }
        // The final buffer must be non-empty: at exactly 4095 bytes `fgets`
        // fills its buffer and an empty one follows.
        let padded = |spaces: usize| {
            let mut v = b"\x1e".to_vec();
            v.extend(std::iter::repeat_n(b' ', spaces));
            v.extend_from_slice(b"1{");
            v
        };
        assert!(!position_lost(&[&padded(4094 - 3)]));
        assert!(position_lost(&[&padded(4095 - 3)]));
        assert!(!position_lost(&[&padded(4096 - 3)]));
        assert!(position_lost(&[&padded(8190 - 3)]));
        assert!(!position_lost(&[&padded(8191 - 3)]));
        // Multi-file: the final buffer is the last file's; an earlier
        // file's newline-free tail is partial and joins it.
        assert!(!position_lost(&[b"\x1e1", b"{"]));
        assert!(position_lost(&[b"\x1e1{", b""]));
        assert!(position_lost(&[b"\x1e1{", b" "]));
        assert!(position_lost(&[b"\x1e1{", b"\x1e2{"]));
        assert!(!position_lost(&[b"\x1e1{", b"}"]));
        assert!(!position_lost(&[b"\x1e[0,]", b""]));
    }

    /// The #3003 answer is read off the same walk as the diagnostics and
    /// the values: keeping the position changes neither the warning's text
    /// nor the values. Under `-s` jq 1.8.2 reports it in the reading call
    /// (#7718), as the non-slurp walk does.
    #[test]
    fn final_byte_rule_leaves_warnings_and_values_alone_3003() {
        assert_eq!(
            warnings_slurp(&[b"\x1e1{"]),
            ["Unfinished JSON term at EOF at line 1, column 3"],
        );
        assert_eq!(
            warnings(&[b"\x1e1{"]),
            ["Unfinished JSON term at EOF at line 1, column 3"],
        );
        assert_eq!(seq_value_texts(b"\x1e1{", true), vec!["1"]);
        assert_eq!(seq_value_texts(b"\x1e1{", false), vec!["1"]);
    }

    /// #7718: jq 1.8.2's slurp loop reads the empty buffer after the last bytes before it hands the
    /// array on, so the EOF warning is the reading call's, before the program runs, whatever the
    /// last byte completed. jq 1.7.1 raised it in the call after when the last byte yielded a value
    /// (#3201: the first group, printed after `error("x")`'s line). Captured from
    /// `/opt/homebrew/bin/jq` 1.8.2 as `--seq -s 'error("x")'`.
    #[test]
    fn slurp_reports_the_eof_warning_in_the_reading_call_7718() {
        for (input, warning) in [
            (
                &b"\x1e1{"[..],
                "Unfinished JSON term at EOF at line 1, column 3",
            ),
            (
                b"\x1etrue\"",
                "Unfinished string at EOF at line 1, column 6",
            ),
            (b"\x1e1\"", "Unfinished string at EOF at line 1, column 3"),
            (
                b"\x1e1 2{",
                "Unfinished JSON term at EOF at line 1, column 5",
            ),
            (
                b"\x1e\"a\"{",
                "Unfinished JSON term at EOF at line 1, column 5",
            ),
            (
                b"\x1e1 {",
                "Unfinished JSON term at EOF at line 1, column 4",
            ),
            (
                b"\x1e[1]{",
                "Unfinished JSON term at EOF at line 1, column 5",
            ),
        ] {
            assert_eq!(warnings_slurp(&[input]), [warning], "{input:?}");
        }
        assert_eq!(
            warnings_slurp(&[b"\x1e1{\x1e1{"]),
            [
                "Truncated value at line 1, column 4",
                "Unfinished JSON term at EOF at line 1, column 6"
            ]
        );
    }

    /// [`final_buffer_start`]'s own boundary arithmetic, which places jq
    /// 1.7.1's final buffer for the positions (#7607): `fgets` stops at a
    /// newline *or* after 4095 bytes, whichever comes first. What is read
    /// does not hang off it any more (#7718): jq 1.7.1 dropped `true` from
    /// 4094 spaces on (#2998), jq 1.8.2 keeps it at every padding.
    #[test]
    fn final_buffer_start_matches_jqs_4095_byte_fgets_chunk() {
        let padded = |spaces: usize| {
            let mut v = b"\x1e1\n\x1e".to_vec();
            v.extend(std::iter::repeat_n(b' ', spaces));
            v.extend_from_slice(b"\x1etrue");
            v
        };
        let start = |raw: Vec<u8>| {
            let total = raw.len();
            final_buffer_start(&[(None, raw)], total)
        };
        // The tail (from the second RS on) is `spaces + 6` bytes, starting at 3.
        assert_eq!(start(padded(4088)), 3, "one chunk holds the tail");
        assert_eq!(start(padded(4089)), 3 + 4095, "the tail's last chunk");
        assert_eq!(start(padded(8183)), 3 + 4095, "two chunks hold it");
        assert_eq!(start(padded(8184)), 3 + 2 * 4095);
        for spaces in [4088, 4089, 4093, 4094, 4095, 8188, 8189] {
            assert_eq!(
                seq_value_texts(&padded(spaces), false),
                vec!["1", "true"],
                "{spaces} spaces"
            );
        }
    }

    /// Multi-file: an earlier file's newline-free tail joins the next file
    /// (#3002), and a last file, empty or not, ends nothing early (#7718).
    #[test]
    fn several_files_read_every_record_2998() {
        let a: &[u8] = b"\x1e\x1etrue"; // no trailing newline
        let b: &[u8] = b"\x1e5\n";
        assert_eq!(
            walk_values(&[a, b], false),
            walk_values(&[a, b], true),
            "a joins b"
        );
        let combined: Vec<u8> = [a, b].concat();
        assert_eq!(
            walk_values(&[a, b], false)
                .into_iter()
                .map(|(s, e)| String::from_utf8(combined[s..e].to_vec()).unwrap())
                .collect::<Vec<_>>(),
            vec!["5"],
            "a's own leading-RS content still joins b and is malformed there"
        );

        let empty: &[u8] = b"";
        let combined: Vec<u8> = [a, empty].concat();
        let kept = |slurp| -> Vec<String> {
            walk_values(&[a, empty], slurp)
                .into_iter()
                .map(|(s, e)| String::from_utf8(combined[s..e].to_vec()).unwrap())
                .collect()
        };
        assert_eq!(kept(false), vec!["true"], "an empty trailing file");
        assert_eq!(kept(false), kept(true));
    }

    /// Every message template, captured from `/usr/bin/jq` 1.7.1. The
    /// three renderings are all here: `at EOF` (real end of input), a
    /// bare `(need RS to resync)` (an ordinary byte), and the `Truncated
    /// value` collapse (an RS byte).
    #[test]
    fn templates_match_the_oracle_1723() {
        // Real EOF, mid-value.
        assert_warnings(
            b"\x1e\"abc",
            &["Unfinished string at EOF at line 1, column 5"],
        );
        assert_warnings(
            b"\x1e[1,2\n",
            &["Unfinished JSON term at EOF at line 2, column 0"],
        );
        assert_warnings(
            b"\x1e{\"a\":1",
            &["Unfinished JSON term at EOF at line 1, column 7"],
        );
        assert_warnings(b"\x1etru", &["Invalid literal at EOF at line 1, column 4"]);
        assert_warnings(
            b"\x1e-",
            &["Invalid numeric literal at EOF at line 1, column 2"],
        );
        assert_warnings(
            b"\x1e1e",
            &["Invalid numeric literal at EOF at line 1, column 3"],
        );

        // Mid-buffer, on an ordinary byte.
        assert_warnings(
            b"\x1exyz\n",
            &["Invalid numeric literal at line 2, column 0 (need RS to resync)"],
        );
        assert_warnings(
            b"\x1e[1,]",
            &["Expected another array element at line 1, column 5 (need RS to resync)"],
        );
        assert_warnings(
            b"\x1e{\"a\":1,}",
            &["Expected another key-value pair at line 1, column 9 (need RS to resync)"],
        );
        assert_warnings(
            b"\x1e}",
            &["Unmatched '}' at line 1, column 2 (need RS to resync)"],
        );
        assert_warnings(
            b"\x1e]",
            &["Unmatched ']' at line 1, column 2 (need RS to resync)"],
        );
        assert_warnings(
            b"\x1e[1 2]",
            &["Expected separator between values at line 1, column 6 (need RS to resync)"],
        );
        assert_warnings(
            b"\x1e:",
            &["Expected string key before ':' at line 1, column 2 (need RS to resync)"],
        );
        assert_warnings(
            b"\x1e,",
            &["Expected value before ',' at line 1, column 2 (need RS to resync)"],
        );

        // An RS byte, mid-value.
        assert_warnings(
            b"\x1e\"unterminated\x1e\"ok\"\n",
            &["Truncated value at line 1, column 15"],
        );
        assert_warnings(
            b"\x1e[1,2\x1e\"ok\"\n",
            &["Truncated value at line 1, column 6"],
        );
    }

    /// A container error leaves the stack behind, and the object/array
    /// distinction picks the message -- both of these report twice
    /// because jq resumes rather than resyncing.
    #[test]
    fn container_errors_and_their_continuations_1723() {
        assert_warnings(
            b"\x1e{1:2}",
            &[
                "Object keys must be strings at line 1, column 4 (need RS to resync)",
                "Unmatched '}' at line 1, column 6 (need RS to resync)",
            ],
        );
        assert_warnings(
            b"\x1e{\"a\", \"b\"}",
            &[
                "Objects must consist of key:value pairs at line 1, column 6 (need RS to resync)",
                "Unmatched '}' at line 1, column 11 (need RS to resync)",
            ],
        );
        assert_warnings(
            b"\x1e{,}\n",
            &[
                "Expected value before ',' at line 1, column 3 (need RS to resync)",
                "Unmatched '}' at line 1, column 4 (need RS to resync)",
            ],
        );
    }

    /// Every string-escape category `found_string` can raise.
    #[test]
    fn string_escape_errors_1723() {
        assert_warnings(
            b"\x1e\"\\q\"",
            &["Invalid escape at line 1, column 5 (need RS to resync)"],
        );
        assert_warnings(
            b"\x1e\"\\u00\"",
            &["Invalid \\uXXXX escape at line 1, column 7 (need RS to resync)"],
        );
        assert_warnings(
            b"\x1e\"\\uZZZZ\"",
            &["Invalid characters in \\uXXXX escape at line 1, column 9 (need RS to resync)"],
        );
        assert_warnings(
            b"\x1e\"\\ud800\"",
            &["Invalid \\uXXXX\\uXXXX surrogate pair escape at line 1, column 9 (need RS to resync)"],
        );
        assert_warnings(
            b"\x1e\"\\ud800\\u0041\"",
            &[
                "Invalid \\uXXXX\\uXXXX surrogate pair escape at line 1, column 15 (need RS to resync)",
            ],
        );
        assert_warnings(
            b"\x1e\"a\x01b\"",
            &[
                "Invalid string: control characters from U+0000 through U+001F must be escaped at line 1, column 6 (need RS to resync)",
            ],
        );
        // A well-formed surrogate pair is not an error.
        assert_warnings(b"\x1e\"\\ud800\\udc00\" ", &[]);
    }

    /// jq's number grammar is decNumber's, not RFC 8259's. Getting this
    /// wrong swaps "Invalid numeric literal" for "Potentially truncated
    /// top-level numeric value" and vice versa.
    #[test]
    fn number_grammar_is_decnumbers_1723() {
        for accepted in [
            &b"\x1e1."[..],
            b"\x1e.5",
            b"\x1e+1",
            b"\x1e01",
            b"\x1enan",
            b"\x1eNaN0",
            b"\x1esnan",
            b"\x1einf",
            b"\x1e-Infinity",
        ] {
            let got = warnings(&[accepted]);
            assert_eq!(
                got.len(),
                1,
                "expected the truncated-number template for {accepted:?}, got {got:?}"
            );
            assert!(
                got[0].starts_with("Potentially truncated top-level numeric value at EOF"),
                "{accepted:?} should parse as a number, got {got:?}"
            );
        }
        for rejected in [
            &b"\x1e1e"[..],
            b"\x1e1e+",
            b"\x1e-",
            b"\x1e1.2.3",
            b"\x1e0x10",
        ] {
            let got = warnings(&[rejected]);
            assert!(
                got[0].starts_with("Invalid numeric literal at EOF"),
                "{rejected:?} should not parse as a number, got {got:?}"
            );
        }
    }

    /// `nu` takes jq's keyword path while a bare `n` falls through to the
    /// number parser, whatever an earlier token left in the buffer (#7525: jq 1.7.1 read the
    /// stale byte, 1.8.2 checks the token's own length first).
    #[test]
    fn keyword_path_depends_on_the_tokens_second_byte_1723() {
        assert_warnings(b"\x1enul", &["Invalid literal at EOF at line 1, column 4"]);
        assert_warnings(
            b"\x1en",
            &["Invalid numeric literal at EOF at line 1, column 2"],
        );
        // `iu` leaves a `u` at index 1, which jq 1.7.1 measured the following bare `n` against
        // (#1723); 1.8.2 does not read it (#7525), and neither does this.
        assert_warnings(
            b"\x1eiu\"n",
            &[
                "Invalid numeric literal at line 1, column 4 (need RS to resync)",
                "Invalid numeric literal at EOF at line 1, column 5",
            ],
        );
    }

    /// #6513, #7525: jq decodes a string's escapes in place over its token buffer, so jq 1.7.1's
    /// next bare `n` found the second byte of the decoded text at index 1 when that has two or
    /// more, and the one the string held there when it has one: a `u` (the escape for `A`) took
    /// the keyword path and said "Invalid literal". jq 1.8.2 never reads the byte, so every row is
    /// its wording for `\x1e"<string>"\n\x1en\n`: "Invalid numeric literal".
    #[test]
    fn a_bare_n_after_a_string_is_a_numeric_literal_whatever_it_decoded_to_6513() {
        let bs = '\\';
        let numeric = "Invalid numeric literal at line 3, column 0 (need RS to resync)";
        for (string, expected) in [
            ("a".to_string(), numeric),
            (format!("{bs}n"), numeric),
            // One byte decoded: the `u` of the escape is still at index 1.
            (format!("{bs}u0041"), numeric),
            (format!("{bs}u0000"), numeric),
            // Two or more: index 1 is a byte of the decoded text.
            (format!("x{bs}u0075"), numeric),
            (format!("{bs}u0041a"), numeric),
            (format!("{bs}u0041{bs}u0042"), numeric),
            (format!("{bs}u00e9"), numeric),
            (format!("{bs}u0800"), numeric),
            (format!("{bs}uffff"), numeric),
            (format!("{bs}udc00"), numeric),
            (format!("{bs}udc00x"), numeric),
            (format!("{bs}ud83d{bs}ude00"), numeric),
            // An error leaves what was decoded before it.
            (format!("{bs}u00e9{bs}ud800"), numeric),
            (format!("{bs}u0041{bs}ud800"), numeric),
            (format!("{bs}u00e9{bs}x"), numeric),
            (format!("{bs}u0041{bs}x"), numeric),
            (format!("{bs}u00e9\t"), numeric),
            (format!("{bs}u0041\t"), numeric),
        ] {
            let input = format!("\x1e\"{string}\"\n\x1en\n").into_bytes();
            let got = warnings(&[&input]);
            assert_eq!(
                got.last().map(String::as_str),
                Some(expected),
                "{string}: {got:?}"
            );
        }
    }

    /// #6513: [`decode_string_in_place`] on its own: the text it decodes to is written from index
    /// 0 and the rest of the token is left as it was, so a string that ends in a lone backslash
    /// (which the scanner never hands it) is the one fault that stops at the end of the text.
    #[test]
    fn the_decode_leaves_the_rest_of_the_token_where_it_was_6513() {
        let bs = '\\';
        let decoded = |token: &str| {
            let mut bytes = token.as_bytes().to_vec();
            let result = decode_string_in_place(&mut bytes);
            (result, bytes)
        };
        // The bytes before the first escape decode to themselves; the escape is written over
        // what follows it, and the tail it leaves is the text the token held.
        let (result, bytes) = decoded(&format!("ab{bs}u00e9c"));
        assert_eq!(result, Ok(()));
        assert_eq!(bytes, b"ab\xc3\xa9c0e9c");
        let (result, bytes) = decoded(&format!("{bs}n{bs}t"));
        assert_eq!(result, Ok(()));
        assert_eq!(bytes, b"\n\t\\t");
        let (result, bytes) = decoded("plain");
        assert_eq!((result, bytes), (Ok(()), b"plain".to_vec()));
        // A fault keeps what was decoded before it.
        let (result, bytes) = decoded(&format!("{bs}u00e9{bs}x"));
        assert_eq!(result, Err("Invalid escape"));
        assert_eq!(&bytes[..2], b"\xc3\xa9");
        let (result, _) = decoded("a\x01");
        assert!(result.is_err());
        let (result, _) = decoded(&format!("a{bs}"));
        assert_eq!(result, Err("Expected escape character at end of string"));
    }

    /// A completed top-level value is handed off and cleared, so trailing
    /// garbage is reported on its own rather than as a separator error.
    #[test]
    fn complete_value_then_garbage_1723() {
        assert_warnings(
            b"\x1e{}extra",
            &["Invalid numeric literal at EOF at line 1, column 8"],
        );
        assert_warnings(
            b"\x1e[1,2]extra",
            &["Invalid numeric literal at EOF at line 1, column 11"],
        );
    }

    /// The RS collapse spares a pending top-level number, and a
    /// mid-buffer error that already fired outranks it.
    #[test]
    fn rs_collapse_exceptions_1723() {
        assert_warnings(
            b"\x1e1.\x1e\"ok\"\n",
            &["Potentially truncated top-level numeric value at line 1, column 4"],
        );
        assert_warnings(
            b"\x1e[1,]\x1e\"ok\"\n",
            &["Expected another array element at line 1, column 5 (need RS to resync)"],
        );
    }

    /// Trailing whitespace resolves an otherwise-ambiguous bare number.
    #[test]
    fn trailing_bare_number_is_ambiguous_only_unterminated_1723() {
        assert_warnings(
            b"\x1e1 2",
            &["Potentially truncated top-level numeric value at EOF at line 1, column 4"],
        );
        assert_warnings(b"\x1e1 2 ", &[]);
    }

    /// Columns count raw bytes, not characters, and never reset across
    /// input files.
    #[test]
    fn positions_count_bytes_and_span_files_1723() {
        assert_warnings(
            b"\x1e\xff]",
            &["Invalid numeric literal at line 1, column 3 (need RS to resync)"],
        );
        assert_warnings(
            b"\x1e\xc3\xa9]",
            &["Invalid numeric literal at line 1, column 4 (need RS to resync)"],
        );
        assert_eq!(
            warnings(&[b"\x1e[1,", b"2\n"]),
            ["Unfinished JSON term at EOF at line 2, column 0"],
        );
    }

    /// A BOM prefix that never completes is still *consumed*, and it costs
    /// jq a `parser_reset` that leaves it reading rather than waiting for
    /// an RS byte. Both halves matter: forget the first and every column
    /// in the stream shifts, forget the second and the pre-RS content is
    /// discarded when jq would have parsed it. Captured from `/usr/bin/jq`
    /// 1.7.1, and from jq 1.8.2 where they differ (#7718).
    #[test]
    fn malformed_bom_is_consumed_and_starts_the_parser_reading_1723() {
        assert_warnings(
            b"\xef\xbb\x1e[1,]\n",
            &["Expected another array element at line 1, column 5 (need RS to resync)"],
        );
        // Read as a record, so this is *not* #1525's abandoned-text
        // template even though the stream holds no RS byte at all: `1` is
        // read. jq 1.7.1 then warned `Potentially truncated top-level numeric
        // value at EOF at line 1, column 3` for the `2`; jq 1.8.2's empty last
        // call resets first, so the `2` and the warning are gone (#7718).
        for input in [&b"\xef\xbb1 2"[..], b"\xef1 2"] {
            assert_warnings(input, &[]);
            assert_eq!(seq_value_texts(input, false), ["1"], "{input:?}");
        }
        // A stream that simply runs out mid-BOM is not malformed -- jq is
        // still waiting for the rest of it, and has consumed no content.
        for input in [&b"\xef"[..], b"\xef\xbb"] {
            assert_warnings(
                input,
                &["Unfinished abandoned text at EOF at line 1, column 0"],
            );
        }
    }

    /// After a malformed BOM jq's parser is wiped at the top of every
    /// `jv_parser_next`, so it never carries state across a value, a
    /// newline, or the end of input. All captured from `/usr/bin/jq` 1.7.1.
    #[test]
    fn malformed_bom_wipes_the_parser_on_every_read_1723() {
        // Emitting `9` drops the string opened in the same breath, so the
        // next line's `-si-` is read as a token rather than swallowed.
        assert_warnings(
            b"\xef\x1exl+uita[9\"-si-\n",
            &[
                "Invalid numeric literal at line 1, column 9 (need RS to resync)",
                "Invalid numeric literal at line 2, column 0 (need RS to resync)",
            ],
        );
        // A newline does the same: the unterminated string ends with the
        // line instead of eating the rest of the stream.
        assert_warnings(
            b"\xef\x1e:\"[:x:0r.\\ {\n:s-\\ba\n",
            &[
                "Expected string key before ':' at line 1, column 2 (need RS to resync)",
                "Expected string key before ':' at line 2, column 1 (need RS to resync)",
                "Invalid numeric literal at line 3, column 0 (need RS to resync)",
            ],
        );
        // And the wipe reaches the empty final buffer a newline-terminated
        // stream produces, so no `at EOF` report survives at all.
        assert_warnings(
            b"\xef\x1er  -{\"f+\n",
            &[
                "Invalid numeric literal at line 1, column 3 (need RS to resync)",
                "Invalid numeric literal at line 1, column 6 (need RS to resync)",
            ],
        );
    }

    /// #3002: the malformed-BOM `parser_reset` fires at every *source*
    /// boundary too -- jq refills its reader one file at a time, and each
    /// refill costs the same wipe the model already applies per value/
    /// newline/error. A partial BOM like `EF BF` that fails mid-buffer
    /// (`EF BF` then RS still reporting `Truncated value`) instead
    /// vanishes when the RS byte opens the *next* source. All captured
    /// from `/usr/bin/jq` 1.7.1.
    #[test]
    fn malformed_bom_resets_at_source_boundaries_3002() {
        // Headline: `EF BF` alone in the first source, RS opening the next
        // -- no warning, where the same bytes in one buffer fail.
        assert_eq!(warnings(&[b"\xef\xbf", b"\x1e"]), Vec::<String>::new());
        // An empty trailing source still costs jq the refill and its reset.
        assert_eq!(warnings(&[b"\xef\xbf", b""]), Vec::<String>::new());
        // A boundary landing *inside* the still-undecided BOM prefix costs a reset
        // too, but nothing has been scanned yet when it fires, so it's a no-op:
        // `BF` is still scanned fresh as the first token of the stream, and the RS
        // in the same source still truncates it -- identical to the single-buffer
        // control below (confirmed live: jq 1.7.1 prints the same warning here).
        assert_eq!(
            warnings(&[b"\xef", b"\xbf\x1e"]),
            ["Truncated value at line 1, column 2"]
        );
        // Control: the same bytes in one buffer still report the truncation.
        assert_eq!(
            warnings(&[b"\xef\xbf\x1e"]),
            ["Truncated value at line 1, column 2"]
        );
        // Control: a *complete* BOM never sets `resets_per_call`, so a
        // source boundary is not a wipe there -- matches one-file behavior.
        assert_eq!(
            warnings(&[b"\xef\xbb\xbf", b"\x1e]"]),
            ["Unmatched ']' at line 1, column 2 (need RS to resync)"]
        );
    }

    /// The same boundary reset belongs on the value walk: without it,
    /// `EF BF`'s dangling `BF` deranges the next source's first record into
    /// being dropped entirely -- `--seq -s '.'` over two sources `EF BF`
    /// then `1 ` yields `[1]` in real jq and nothing in one buffer.
    #[test]
    fn malformed_bom_value_walk_resets_at_source_boundaries_3002() {
        // jq swallows the `EF`; the dangling `BF` is left for the walk.
        // Source `a`'s exclusive end is the boundary (`b` starts there).
        assert_eq!(walk_values(&[b"\xef\xbf", b"1 \n"], true), vec![(2, 3)]);
        // Same bytes in one buffer: `BF` derails and the record is dropped.
        assert_eq!(walk_values(&[b"\xef\xbf1 \n"], true), vec![]);
    }

    /// #3247: the values come from the raw bytes, so no UTF-8 substitution
    /// can rewrite what the reader decides on. Outside a string, jq-style
    /// substitution of `\xe0\x1e` up to the `"` folds the RS into one
    /// U+FFFD; a walk over that lost `"a"`. Ranges index the raw stream,
    /// BOM bytes included. Captured from `/usr/bin/jq` 1.7.1.
    #[test]
    fn values_are_ranges_into_the_raw_stream_3247() {
        // Both modes: none of these reaches #2998's drop, so they agree.
        let texts = |sources: &[&[u8]]| -> Vec<Vec<u8>> {
            let raw: Vec<u8> = sources.concat();
            let slurped = walk_values(sources, true);
            assert_eq!(walk_values(sources, false), slurped, "{sources:?}");
            slurped
                .into_iter()
                .map(|(start, end)| raw[start..end].to_vec())
                .collect()
        };
        assert_eq!(texts(&[b"\xe0\x1e\"a\"\n"]), [b"\"a\"".to_vec()]);
        assert_eq!(
            texts(&[b"\x1e1\n\xe0\x1e\"a\"\n"]),
            [b"1".to_vec(), b"\"a\"".to_vec()]
        );
        assert_eq!(texts(&[b"\xef\xbb\xe0 \"a\""]), [b"\"a\"".to_vec()]);
        // An invalid byte inside a string stays in the range, raw.
        assert_eq!(
            texts(&[b"\xef\xbb\xbf\x1e\"\xff\"\n"]),
            [b"\"\xff\"".to_vec()]
        );
        // BOM prefixes split across sources, with empty sources around
        // them: the ranges still count the BOM bytes. Captured from jq as
        // files.
        assert_eq!(texts(&[b"\xef", b"\xbb1 2"]), [b"1".to_vec()]);
        assert_eq!(texts(&[b"\xef", b"", b"\xbb1 2"]), [b"1".to_vec()]);
        assert_eq!(texts(&[b"\xef\xbb", b"\xbf\x1e1\n"]), [b"1".to_vec()]);
        assert_eq!(texts(&[b"", b"\xef\xbb\xbf\x1e1\n"]), [b"1".to_vec()]);
    }

    /// #3200: with a malformed BOM, jq resets at *every* `fgets` refill,
    /// including one after a full 4095-byte chunk with no newline, and an
    /// empty final buffer (a stream filling its last chunk exactly) wipes
    /// the EOF report just as a trailing newline does. Rows are the issue's
    /// three, captured from `/usr/bin/jq` 1.7.1 as `--seq -s 'error("x")'`,
    /// and from jq 1.8.2 where they differ (#7718).
    #[test]
    fn malformed_bom_resets_at_every_fgets_refill_3200() {
        let padded = |head: &[u8], pad: usize, tail: &[u8]| -> Vec<u8> {
            let mut v = head.to_vec();
            v.resize(head.len() + pad, b' ');
            v.extend_from_slice(tail);
            v
        };
        // 4095 bytes: the chunk fills exactly, an empty final buffer
        // follows, and its reset wipes the `{`: no warning, position kept.
        let exact = padded(b"\xef\xbb{", 4092, b"");
        assert_eq!(exact.len(), 4095);
        assert_eq!(warnings_slurp(&[&exact]), Vec::<String>::new());
        assert!(!position_lost(&[&exact]));
        // 4096 bytes: the refill at 4095 resets before the last space.
        let over = padded(b"\xef\xbb{", 4093, b"");
        assert_eq!(warnings_slurp(&[&over]), Vec::<String>::new());
        assert!(!position_lost(&[&over]));
        // The reverse: the refill drops the open string and the trailing `"`
        // opens a new one, which jq 1.7.1's EOF branch reported (`Unfinished
        // string at EOF at line 1, column 4094`, position lost). jq 1.8.2 reads
        // one more, empty, buffer, whose reset wipes it: no warning, `file:0`.
        let reopened = padded(b"\xef\xbb\"a", 4091, b"\"");
        assert_eq!(reopened.len(), 4096);
        assert_eq!(warnings_slurp(&[&reopened]), Vec::<String>::new());
        assert!(!position_lost(&[&reopened]));
        // Control: without a malformed BOM there is no reset at a refill.
        let plain = padded(b"\x1e{", 4093, b"");
        assert_eq!(
            warnings_slurp(&[&plain]),
            ["Unfinished JSON term at EOF at line 1, column 4095"]
        );
    }

    /// #3200: the refill boundaries are the later sources' starts plus, with
    /// `size_limit`, each full chunk's end that is not a newline or its
    /// source's own end; the count restarts at each source.
    #[test]
    fn refill_boundaries_restart_per_source_3200() {
        let bounds = |sources: &[Vec<u8>], size_limit| {
            let owned: Vec<(Option<usize>, Vec<u8>)> =
                sources.iter().map(|s| (None, s.clone())).collect();
            refill_boundaries(&owned, size_limit)
        };
        let spaces = |n| vec![b' '; n];
        assert_eq!(bounds(&[spaces(4095)], true), Vec::<usize>::new());
        assert_eq!(bounds(&[spaces(4096)], true), vec![4095]);
        assert_eq!(bounds(&[spaces(8191)], true), vec![4095, 8190]);
        // A newline inside the first 4095 bytes ends that chunk early.
        let mut nl = spaces(5000);
        nl[10] = b'\n';
        assert_eq!(bounds(&[nl], true), vec![11 + 4095]);
        // A full chunk ending on its newline needs no entry.
        let mut full_nl = spaces(5000);
        full_nl[4094] = b'\n';
        assert_eq!(bounds(&[full_nl], true), Vec::<usize>::new());
        // A later source's start, and its own count restarting there.
        assert_eq!(
            bounds(&[spaces(100), spaces(4096)], true),
            vec![100, 100 + 4095]
        );
        // A non-final source filling its chunk exactly adds only the next
        // source's start.
        assert_eq!(
            bounds(&[spaces(4095), spaces(4096)], true),
            vec![4095, 4095 + 4095]
        );
        // Without `size_limit`, only the source starts (#3002).
        assert_eq!(
            bounds(&[spaces(4096), spaces(10), spaces(0)], false),
            vec![4096, 4106]
        );
    }

    /// #3250: each value's `(source index, line)` is where jq's reader stood
    /// when it yielded the value -- the yielding byte's file, and the
    /// newlines of every `fgets` chunk read by then, including the whole
    /// chunk holding that byte. Captured from `/usr/bin/jq` 1.7.1 as
    /// `[., input_filename, input_line_number]` over the same files.
    #[test]
    fn value_locations_follow_the_yielding_byte_3250() {
        let locations = |sources: &[&[u8]]| {
            let owned: Vec<(Option<usize>, Vec<u8>)> =
                sources.iter().map(|s| (None, s.to_vec())).collect();
            walk_stream(&owned, false, &mut |_| {}).locations
        };
        // `"x"` yields on its own closing quote, at the end of the first
        // file; `1` yields on the newline after it, in the second.
        assert_eq!(locations(&[b"\x1e\"x\"", b" 1\n"]), vec![(0, 0), (1, 1)]);
        // `1` ending the first file yields on the space opening the second.
        assert_eq!(locations(&[b"\x1e1", b" \n\x1e2\n"]), vec![(1, 1), (1, 2)]);
        // The whole chunk holding the yielding byte is read first, so its
        // newline counts though it comes after the value.
        assert_eq!(locations(&[b"\x1e\"x\"1\x1e}b\na "]), vec![(0, 1)]);
        // A value yielded by the EOF branch is named after the last source,
        // with all of its newlines -- even an empty one.
        assert_eq!(locations(&[b"\x1e\ntrue"]), vec![(0, 1)]);
        assert_eq!(locations(&[b"\x1e\ntrue", b""]), vec![(1, 0)]);
        assert_eq!(locations(&[b"\x1e\ntrue", b"\n\n"]), vec![(1, 1)]);
        // A value completed at a full 4095-byte chunk's end is yielded
        // before the chunk holding its newline is read.
        let mut full = b"\xef\xbb[".to_vec();
        full.resize(4094, b' ');
        full.extend_from_slice(b"]\n");
        assert_eq!(locations(&[&full]), vec![(0, 0)]);
    }

    /// One leading BOM is consumed without advancing the column, and only
    /// at the stream's start -- an empty leading source does not use it up.
    #[test]
    fn bom_is_consumed_but_not_counted_1723() {
        assert_warnings(
            b"\xef\xbb\xbf\x1e}",
            &["Unmatched '}' at line 1, column 2 (need RS to resync)"],
        );
        assert_eq!(
            warnings(&[b"", b"\xef\xbb\xbf\x1e}"]),
            ["Unmatched '}' at line 1, column 2 (need RS to resync)"],
        );
        // A second BOM is ordinary content.
        assert_warnings(
            b"\xef\xbb\xbf\x1e\xef\xbb\xbf]",
            &["Invalid numeric literal at line 1, column 5 (need RS to resync)"],
        );
    }

    #[test]
    fn depth_limit_matches_jq_1723() {
        let mut input = vec![ASCII_RS];
        input.extend(std::iter::repeat_n(b'[', 300));
        assert_eq!(
            warnings(&[&input]),
            [
                "Exceeds depth limit for parsing at line 1, column 258 (need RS to resync)"
                    .to_string(),
                "Unfinished JSON term at EOF at line 1, column 301".to_string(),
            ],
        );
    }

    /// The remaining `parse_token` categories, each needing a value already
    /// in hand when the structural byte arrives -- which only happens
    /// inside a container, since a completed top-level value is handed off
    /// and cleared before the next byte is read.
    #[test]
    fn structural_errors_needing_a_pending_value_1723() {
        assert_warnings(
            b"\x1e[1[",
            &["Expected separator between values at line 1, column 4 (need RS to resync)"],
        );
        assert_warnings(
            b"\x1e[1:",
            &["':' not as part of an object at line 1, column 4 (need RS to resync)"],
        );
        assert_warnings(
            b"\x1e{\"a\"}",
            &["Objects must consist of key:value pairs at line 1, column 6 (need RS to resync)"],
        );
        assert_warnings(
            b"\x1e[}",
            &["Unmatched '}' at line 1, column 3 (need RS to resync)"],
        );
    }

    /// A token longer than the buffer's initial growth step, and the
    /// ordinary two-character escapes -- neither reaches a diagnostic, so
    /// what is asserted is that neither invents one.
    #[test]
    fn long_tokens_and_plain_escapes_1723() {
        // 256 exactly fills the buffer's first growth step, which is the
        // one length at which `check_literal`'s own NUL write has to grow
        // it again; 300 then exercises a second growth during scanning.
        for (digits, column) in [(256usize, 257usize), (300, 301)] {
            let mut input = vec![ASCII_RS];
            input.extend(std::iter::repeat_n(b'9', digits));
            assert_eq!(
                warnings(&[&input]),
                [format!(
                    "Potentially truncated top-level numeric value at EOF at line 1, column {column}"
                )],
            );
        }
        assert_warnings(b"\x1e\"a\\nb\\t\\\\\\/\\\"\\b\\f\\rc\"", &[]);
    }

    /// #1525's template, reached through the model rather than
    /// `seq_no_rs_byte_warning`: with no RS byte anywhere, jq's parser is
    /// still waiting for one when EOF arrives. The wired call site routes
    /// this case to #1525's helper instead, so the two never both fire.
    #[test]
    fn no_rs_byte_anywhere_abandons_at_eof_1723() {
        assert_warnings(
            b"1 2",
            &["Unfinished abandoned text at EOF at line 1, column 3"],
        );
        // Newlines still advance the line counter while waiting for an RS.
        assert_warnings(b"junk\nmore\x1e\"ok\"\n", &[]);
    }

    /// Content before the first RS byte is discarded without a word: jq's
    /// `--seq` parser starts out waiting for one.
    #[test]
    fn content_before_the_first_rs_is_silent_1723() {
        assert_warnings(b"garbage\x1e\"ok\"\n", &[]);
        assert_warnings(b"\x1e\"ok\"\n", &[]);
        assert_warnings(b"\x1e{\"a\": [1, 2]}\n", &[]);
    }

    /// A small xorshift, so the generated streams below are the same on every run.
    struct Rng(u64);

    impl Rng {
        fn below(&mut self, n: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % n as u64) as usize
        }

        fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
            items[self.below(items.len())]
        }
    }

    /// The tokens a clean record is made of: every kind that writes index 1 of jq's token buffer
    /// and every kind that does not ([`token_byte_after`]).
    const CLEAN_TOKENS: &[&str] = &[
        "null",
        "true",
        "false",
        "1",
        "12",
        "-3.5e2",
        "\"\"",
        "\"a\"",
        "\"bus\"",
        "\"\\u0041\"",
        "\"x\\u0075\"",
        "\"\\n\"",
        "\"\\u00e9\"",
        "\"q\\\"u\"",
        "\"\\\\\"",
        "{}",
        "[]",
    ];

    /// One clean record, `RS value LF`, sometimes spanning lines.
    fn clean_record(rng: &mut Rng) -> String {
        let mut value = String::new();
        let width = rng.below(4);
        if width == 0 {
            value.push_str(rng.pick(CLEAN_TOKENS));
        } else if rng.below(2) == 0 {
            value.push('[');
            for i in 0..width {
                if i > 0 {
                    value.push_str(if rng.below(3) == 0 { ",\n " } else { "," });
                }
                value.push_str(rng.pick(CLEAN_TOKENS));
            }
            value.push(']');
        } else {
            value.push('{');
            for i in 0..width {
                if i > 0 {
                    value.push(',');
                }
                value.push_str(rng.pick(&["\"k\"", "\"\"", "\"\\u0075\"", "\"nu\""]));
                value.push(':');
                value.push_str(rng.pick(CLEAN_TOKENS));
            }
            value.push('}');
        }
        format!("\x1e{value}\n")
    }

    /// The tail of a stream after its clean records: what a record that is not plain can hold, a bare
    /// `n` most of all (it reads the stale byte), RS bytes where a reader is in the final buffer, and
    /// clean records after them.
    fn tail(rng: &mut Rng) -> Vec<u8> {
        let pieces = [
            "\x1en\n",
            "\x1e[n]\n",
            "\x1enul\n",
            "\x1e1 2\n",
            "\x1e{\"a\":1\n",
            "\x1e\"open\n",
            "\x1e12.5",
            "\x1e",
            "\x1e\x1e",
            "garbage\n",
            "\n\n",
            "\x1enan\n",
            "\x1e[1,]\n",
            "\x1e}\n",
            "\x1etrue",
            " ",
            "\x1e\"\\u0041\"\n",
        ];
        let mut text = String::new();
        for _ in 0..=rng.below(5) {
            if rng.below(3) == 0 {
                text.push_str(&clean_record(rng));
            } else {
                text.push_str(rng.pick(&pieces));
            }
        }
        text.into_bytes()
    }

    /// What a walk of one stream answers that a run shows: every warning, and each value's text
    /// with the line jq names for it.
    fn walked(stream: &[u8]) -> (Vec<String>, Vec<(String, usize)>) {
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
    }

    /// #7164: the rest of a stream after its clean records, walked as [`resumed_stream`] with the byte
    /// [`token_byte_after`] finds, gives the warnings, values and lines that the walk of the whole stream
    /// gives after those records. The streams hold every kind of token before a bare `n`, multi-line
    /// records, and tails that reach the end-of-stream drop (#2998) and the EOF branch.
    #[test]
    fn a_resumed_walk_reads_the_rest_of_the_stream_as_the_whole_walk_7164() {
        let mut rng = Rng(0x7164_7164);
        for case in 0..4000 {
            let records: String = (0..=rng.below(6)).map(|_| clean_record(&mut rng)).collect();
            let rest = tail(&mut rng);
            let whole = [records.as_bytes(), &rest].concat();
            let (whole_warnings, whole_values) = walked(&whole);
            let clean_values = walked(records.as_bytes()).1.len();
            seed_token_byte(token_byte_after(records.as_bytes()));
            let lines = records.bytes().filter(|&b| b == b'\n').count();
            let resumed = walked(&resumed_stream(lines, rest.clone()));
            // #7260: the same with a front of one line break that counts the rest.
            let (counted, base) = resumed_stream_counted(lines, rest.clone());
            seed_line_base(base);
            let resumed_counted = walked(&counted);
            seed_line_base(0);
            seed_token_byte(None);
            let want = (whole_warnings, whole_values[clean_values..].to_vec());
            assert_eq!(
                resumed,
                want,
                "case {case}: {records:?} then {:?}",
                String::from_utf8_lossy(&rest)
            );
            assert_eq!(
                resumed_counted,
                want,
                "counted, case {case}: {records:?} then {:?}",
                String::from_utf8_lossy(&rest)
            );
        }
    }

    /// #7322: a stretch that is not plain records, from the RS at a line start that opens it through the
    /// next RS at a line start, is walked on its own as the walk of the whole stream reads it: the same
    /// warnings and values, and the stream after it reads as [`resumed_stream`] with the byte the stretch
    /// left. With no record before it, it is the stream's start.
    #[test]
    fn a_stretch_is_walked_as_the_whole_walk_reads_it_7322() {
        let pieces = [
            "\x1en\n",
            "\x1e[n]\n",
            "\x1enul\n",
            "\x1e1 2\n",
            "\x1e{\"a\":1\n",
            "\x1e\"open\n",
            "\x1e12.5",
            "\x1e",
            "\x1e\x1e",
            "garbage\n",
            "\n\n",
            "\x1enan\n",
            "\x1e[1,]\n",
            "\x1e} 5\n",
            "\x1etrue",
            " ",
            "\x1e\"\\u0041\"\n",
            "\x1e\"a\" [1,\n2]\n",
            "\x1e1\"bu\"\n",
        ];
        let mut rng = Rng(0x7322_7322);
        let mut walked_alone = 0;
        for case in 0..6000 {
            let records: String = (0..rng.below(5)).map(|_| clean_record(&mut rng)).collect();
            let mut stretch = String::from("\x1e");
            for _ in 0..=rng.below(4) {
                stretch.push_str(rng.pick(&pieces));
            }
            if !stretch.ends_with('\n') {
                stretch.push('\n');
            }
            let mut rest: Vec<u8> = (0..=rng.below(3))
                .flat_map(|_| clean_record(&mut rng).into_bytes())
                .collect();
            if rng.below(2) == 0 {
                rest.extend(tail(&mut rng));
            }
            let whole = [records.as_bytes(), stretch.as_bytes(), &rest].concat();
            let shown = format!("case {case}: {:?}", String::from_utf8_lossy(&whole));
            let (whole_warnings, whole_values) = walked(&whole);
            let lines = records.bytes().filter(|&b| b == b'\n').count();
            // The stretch, through the RS that opens the first record after it.
            let through = [stretch.as_bytes(), b"\x1e"].concat();
            let Some(walk) = walk_stretch(&through, lines, token_byte_after(records.as_bytes()))
            else {
                continue;
            };
            walked_alone += 1;
            let clean_values = walked(records.as_bytes()).1.len();
            seed_token_byte(walk.token_byte);
            let after = lines + stretch.bytes().filter(|&b| b == b'\n').count();
            let (rest_warnings, rest_values) = walked(&resumed_stream(after, rest.clone()));
            seed_token_byte(None);
            let warnings: Vec<String> = walk
                .warnings
                .iter()
                .map(|(_, warning)| warning.clone())
                .chain(rest_warnings)
                .collect();
            assert_eq!(warnings, whole_warnings, "{shown}");
            let values: Vec<String> = walk
                .values
                .iter()
                .map(|&(start, end)| String::from_utf8_lossy(&through[start..end]).into_owned())
                .chain(rest_values.iter().map(|(text, _)| text.clone()))
                .collect();
            let whole_texts: Vec<String> = whole_values[clean_values..]
                .iter()
                .map(|(text, _)| text.clone())
                .collect();
            assert_eq!(values, whole_texts, "{shown}");
            // The lines of the values after the stretch are the whole walk's.
            let rest_lines: Vec<usize> = rest_values.iter().map(|&(_, line)| line).collect();
            let whole_lines: Vec<usize> = whole_values[whole_values.len() - rest_values.len()..]
                .iter()
                .map(|&(_, line)| line)
                .collect();
            assert_eq!(rest_lines, whole_lines, "{shown}");
            // A warning is met on a byte no value holds, and each is where the reader stood.
            for &(at, _) in &walk.warnings {
                assert!(
                    walk.values
                        .iter()
                        .all(|&(start, end)| at < start || at >= end),
                    "{shown}"
                );
            }
            assert!(
                walk.warnings.windows(2).all(|pair| pair[0].0 < pair[1].0),
                "{shown}"
            );
        }
        assert!(walked_alone > 5000, "{walked_alone}");
    }

    /// #7322: a stretch that warns more often than the walk keeps is declined.
    #[test]
    fn a_stretch_that_warns_without_end_is_declined_7322() {
        let few = format!("\x1e{}\n\x1e", "}".repeat(MAX_STRETCH_WARNINGS - 1));
        assert!(walk_stretch(few.as_bytes(), 0, None).is_some());
        let many = format!("\x1e{}\n\x1e", "}".repeat(MAX_STRETCH_WARNINGS + 1));
        assert!(walk_stretch(many.as_bytes(), 0, None).is_none());
    }

    /// #7164: [`token_byte_after`] is the byte the reader's own buffer holds at index 1 once it has read
    /// the records, whenever a token wrote there; when none did, the buffer holds nothing there or a 0.
    #[test]
    fn the_token_byte_is_the_readers_own_7164() {
        let mut rng = Rng(0x1e1e_7164);
        for case in 0..4000 {
            let records: String = (0..=rng.below(4)).map(|_| clean_record(&mut rng)).collect();
            // A record that draws a warning first: its reset leaves the token buffer alone.
            let warned = std::cell::Cell::new(0);
            let mut emit = |_: &str, _: usize| warned.set(warned.get() + 1);
            let mut reader = Reader::new(bom_prefix(&[]), &mut emit);
            reader.run(format!("\x1e}}\n{records}").as_bytes(), 0, &[]);
            let held = reader.token_buf.get(1).copied();
            assert_eq!(warned.get(), 1, "case {case}: {records:?}");
            match token_byte_after(records.as_bytes()) {
                Some(byte) => assert_eq!(held, Some(byte), "case {case}: {records:?}"),
                None => assert!(matches!(held, None | Some(0)), "case {case}: {records:?}"),
            }
        }
        // A string whose text decodes to one byte leaves the second byte it held.
        assert_eq!(token_byte_after(b"\x1e\"\\u0041\"\n"), Some(b'u'));
        assert_eq!(token_byte_after(b"\x1e[\"bus\",\"\"]\n"), Some(b'u'));
        assert_eq!(token_byte_after(b"\x1enull\n\x1e{}\n"), Some(b'u'));
        assert_eq!(token_byte_after(b"\x1e7\n"), Some(0));
        assert_eq!(token_byte_after(b"\x1e[\"a\",{}]\n"), None);
        assert_eq!(resumed_stream(0, b"\x1e1\n".to_vec()), b"\x1e1\n");
        // #7262: room for a tail no allocation can hold is not reserved, rather than abort the process.
        for room in [usize::MAX, usize::MAX - 2, isize::MAX as usize] {
            let front = resumed_front(2, room);
            assert_eq!(front, b"\x1e\n\n", "{room}");
            assert!(front.capacity() < room, "{room}");
        }
        assert!(resumed_front(2, 100).capacity() >= 103);
        assert_eq!(resumed_front(0, usize::MAX), b"");
        assert_eq!(resumed_stream(2, b"\x1e1\n".to_vec()), b"\x1e\n\n\x1e1\n");
    }

    /// A small deterministic generator for the #7263 sweeps: the same streams on every run.
    struct Lcg(u64);

    impl Lcg {
        fn below(&mut self, n: usize) -> usize {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((self.0 >> 33) as usize) % n
        }

        /// `len` bytes drawn from `alphabet`.
        fn bytes(&mut self, alphabet: &[u8], len: usize) -> Vec<u8> {
            (0..len)
                .map(|_| alphabet[self.below(alphabet.len())])
                .collect()
        }
    }

    /// #7263: the block search finds what a byte-at-a-time search finds, first and last, at every
    /// offset around the head and the block edges, and in no position at all.
    #[test]
    fn find_byte_and_rfind_byte_agree_with_a_bytewise_search_7263() {
        for len in 0..200 {
            let none = vec![b'a'; len];
            assert_eq!(find_byte(b'\n', &none), None, "len {len}");
            assert_eq!(rfind_byte(b'\n', &none), None, "len {len}");
            for at in 0..len {
                let mut one = none.clone();
                one[at] = b'\n';
                assert_eq!(find_byte(b'\n', &one), Some(at), "len {len} at {at}");
                assert_eq!(rfind_byte(b'\n', &one), Some(at), "len {len} at {at}");
            }
        }
        let mut rng = Lcg(7263);
        for _ in 0..2000 {
            let len = rng.below(400);
            let hay = rng.bytes(b"ab\n\x1e", len);
            for needle in [b'\n', ASCII_RS, b'z'] {
                assert_eq!(
                    find_byte(needle, &hay),
                    hay.iter().position(|&b| b == needle),
                    "{hay:?}"
                );
                assert_eq!(
                    rfind_byte(needle, &hay),
                    hay.iter().rposition(|&b| b == needle),
                    "{hay:?}"
                );
            }
        }
    }

    /// #7263: `fgets_chunk_len_at` keeps the byte-at-a-time rule it replaced: a chunk ends just
    /// after a newline or after `JQ_READER.read_len` bytes.
    #[test]
    fn fgets_chunk_len_at_keeps_the_bytewise_rule_7263() {
        let bytewise = |raw: &[u8], pos: usize| {
            let rest = raw.get(pos..).filter(|rest| !rest.is_empty())?;
            Some(
                rest.iter()
                    .take(JQ_READER.read_len)
                    .position(|&byte| byte == b'\n')
                    .map_or_else(|| rest.len().min(JQ_READER.read_len), |index| index + 1),
            )
        };
        let mut rng = Lcg(4095);
        for round in 0..300 {
            // Long runs without a newline, so that chunks fill to the limit as well as end early.
            let alphabet: &[u8] = if round % 2 == 0 { b"xxxxxxxx\n" } else { b"x" };
            let len = rng.below(3 * JQ_READER.read_len);
            let raw = rng.bytes(alphabet, len);
            let mut pos = 0;
            loop {
                let got = fgets_chunk_len_at(&raw, pos);
                assert_eq!(got, bytewise(&raw, pos), "len {len} pos {pos}");
                let Some(step) = got else { break };
                pos += step;
            }
            assert_eq!(fgets_chunk_len_at(&raw, len + 1), None);
        }
    }

    /// jq's reader over a stream with no RS it sees, a byte at a time, past the BOM [`bom_prefix`]
    /// consumes: `None` where an RS is seen or the BOM is malformed.
    fn no_rs_position_bytewise(sources: &[(Option<usize>, Vec<u8>)]) -> Option<(usize, usize)> {
        let bom = bom_prefix(sources);
        if bom.malformed {
            return None;
        }
        let (mut line, mut column) = (1usize, 0usize);
        for (at, &b) in sources.iter().flat_map(|(_, raw)| raw.iter()).enumerate() {
            if at < bom.consumed {
                continue;
            }
            if b == ASCII_RS {
                return None;
            }
            if b == b'\n' {
                line += 1;
                column = 0;
            } else {
                column += 1;
            }
        }
        Some((line, column))
    }

    /// #7260: [`NoRsScan`] fed a source in pieces of any size, each read up to where it says and the
    /// rest kept in front of the next piece (as the windowed route reads a file), stands where it does
    /// fed each source whole, and where jq's reader stands counted a byte at a time: over BOM
    /// prefixes of every kind, NULs in reads with and without a newline, lines longer than one of
    /// jq's reads, and RS bytes seen and hidden.
    #[test]
    fn no_rs_scan_in_pieces_is_the_bytewise_count_7260() {
        let prefixes: [&[u8]; 6] = [
            b"",
            b"\xEF",
            b"\xEF\xBB",
            b"\xEF\xBB\xBF",
            b"\xEF\xBBx",
            b"\xEFx",
        ];
        let mut rng = Lcg(7260);
        for prefix in prefixes {
            for round in 0..300 {
                let mut stream = prefix.to_vec();
                // Lines longer than one read (JQ_READER.read_len), and an RS or NUL now and then.
                let len = rng.below(if round % 3 == 0 {
                    3 * JQ_READER.read_len
                } else {
                    40
                });
                let alphabet: &[u8] = match round % 5 {
                    0 => b"ab\n",
                    1 => b"aaaaaaaaab\n\0",
                    2 => b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaab\0\x1e\n",
                    // Lines of thousands of bytes, with a NUL in some of their reads.
                    3 => &[b'a'; 1500][..],
                    _ => b"a\n\0\xEF\xBB\xBF",
                };
                stream.extend(rng.bytes(alphabet, len));
                if round % 5 == 3 {
                    for _ in 0..rng.below(4) {
                        let at = prefix.len() + rng.below(len + 1);
                        stream.insert(at, [b'\0', b'\n', ASCII_RS][rng.below(3)]);
                    }
                }
                if round % 7 == 0 {
                    // A long line with a NUL in its second read and an RS after it, unseen.
                    stream.extend(std::iter::repeat_n(b'a', JQ_READER.read_len + 7));
                    stream.extend(b"\0x\x1ey");
                }
                let mut sources = Vec::new();
                let mut rest = stream.as_slice();
                while !rest.is_empty() {
                    let take = rng.below(rest.len() + 1).max(rng.below(2));
                    sources.push((None, rest[..take].to_vec()));
                    rest = &rest[take..];
                }
                let want = no_rs_position_bytewise(&sources);
                let mut whole = NoRsScan::default();
                let whole = sources
                    .iter()
                    .try_for_each(|(_, raw)| whole.feed(raw, true).map(|_| ()))
                    .map(|()| whole.position());
                assert_eq!(whole, want, "{sources:?}");
                let mut scan = NoRsScan::default();
                let pieces = sources.iter().try_for_each(|(_, raw)| {
                    let mut held = Vec::new();
                    let mut rest = raw.as_slice();
                    loop {
                        let take = rng.below(rest.len() + 1).max(rng.below(2)).min(rest.len());
                        held.extend_from_slice(&rest[..take]);
                        rest = &rest[take..];
                        let read = scan.feed(&held, rest.is_empty())?;
                        if rest.is_empty() {
                            assert_eq!(read, held.len());
                            return Some(());
                        }
                        // What is left is less than one of jq's reads, which never holds a newline.
                        assert!(held.len() - read < JQ_READER.read_len);
                        assert!(!held[read..].contains(&b'\n'));
                        held.drain(..read);
                    }
                });
                assert_eq!(pieces.map(|()| scan.position()), want, "{sources:?}");
            }
        }
    }

    /// #7263: `advance_position` moves a line and column as a byte-at-a-time count does, from any
    /// starting point.
    #[test]
    fn advance_position_counts_as_a_byte_at_a_time_7263() {
        let mut rng = Lcg(3);
        for _ in 0..2000 {
            let len = rng.below(300);
            let span = rng.bytes(b"aaaaaaa\n", len);
            let (start_line, start_column) = (1 + rng.below(5), rng.below(9));
            let (mut line, mut column) = (start_line, start_column);
            advance_position(&span, &mut line, &mut column);
            let (mut want_line, mut want_column) = (start_line, start_column);
            for &b in &span {
                if b == b'\n' {
                    want_line += 1;
                    want_column = 0;
                } else {
                    want_column += 1;
                }
            }
            assert_eq!((line, column), (want_line, want_column), "{span:?}");
        }
    }

    /// Everything a walk answers, with each warning and where it was met, for the #7263 sweep.
    type WalkAnswers = (
        Vec<(String, usize)>,
        Vec<SeqFault>,
        Vec<(usize, usize)>,
        Vec<(usize, usize)>,
        bool,
    );

    fn walk_answers(
        sources: &[(Option<usize>, Vec<u8>)],
        slurp: bool,
        faults: bool,
    ) -> WalkAnswers {
        let mut warnings = Vec::new();
        let walk = walk_stream_collecting_at(sources, slurp, faults, Tail::Opened, &mut |w, at| {
            warnings.push((w.to_string(), at));
        });
        (
            warnings,
            walk.faults,
            walk.values,
            walk.locations,
            walk.slurp_position_lost,
        )
    }

    /// #7263: the walk that takes the bytes before an RS a run at a time answers what the walk that
    /// took them one at a time answered, over streams that open with text and newlines before
    /// their first RS, across sources, behind every BOM prefix (a malformed one keeps the byte at
    /// a time), with and without `-s` and the recorded faults.
    #[test]
    fn discarding_before_an_rs_a_run_at_a_time_answers_as_bytewise_7263() {
        let prefixes: [&[u8]; 4] = [b"", b"\xEF\xBB\xBF", b"\xEF\xBB", b"\xEFx"];
        let mut rng = Lcg(1723);
        for prefix in prefixes {
            for _ in 0..300 {
                let mut stream = prefix.to_vec();
                let lead = rng.below(150);
                stream.extend(rng.bytes(b"ab {}1\"\n", lead));
                let tail = rng.below(60);
                stream.extend(rng.bytes(b"\x1e\x1e1 2[]{}\"a:,\n\n", tail));
                if rng.below(2) == 0 {
                    stream.push(b'\n');
                }
                let mut sources = Vec::new();
                let mut rest = stream.as_slice();
                loop {
                    let take = if rng.below(3) == 0 {
                        rest.len()
                    } else {
                        rng.below(rest.len() + 1)
                    };
                    sources.push((None, rest[..take].to_vec()));
                    rest = &rest[take..];
                    if rest.is_empty() {
                        break;
                    }
                }
                for slurp in [false, true] {
                    for faults in [false, true] {
                        let run = walk_answers(&sources, slurp, faults);
                        let bytewise =
                            discards::bytewise_in(|| walk_answers(&sources, slurp, faults));
                        assert_eq!(run, bytewise, "{sources:?} slurp {slurp} faults {faults}");
                    }
                }
            }
        }
    }

    /// #7320: the test switch is off again after its closure panics, not left on for the rest of
    /// the thread.
    #[test]
    fn the_bytewise_switch_is_off_after_a_panicking_closure_7320() {
        assert!(!discards::bytewise());
        let on_at_the_panic = std::cell::Cell::new(false);
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            discards::bytewise_in(|| {
                on_at_the_panic.set(discards::bytewise());
                panic!("closure panics with the switch on");
            });
        }));
        assert!(panicked.is_err());
        assert!(
            on_at_the_panic.get(),
            "the switch was on when the closure panicked"
        );
        assert!(!discards::bytewise());
    }

    /// #7263: a run long enough to span `fgets` chunks and refill boundaries, before and after the
    /// first RS, answers as the byte-at-a-time walk does.
    #[test]
    fn a_long_discarded_run_answers_as_bytewise_7263() {
        let mut stream = vec![b'x'; 3 * JQ_READER.read_len + 7];
        stream[100] = b'\n';
        stream[JQ_READER.read_len + 3] = b'\n';
        // No newline after the run, so the column of the fault at the end counts every byte of it.
        stream.extend_from_slice(b"\x1e{\"a\":1} \x1e[1,");
        let halves = vec![
            (None, stream[..5000].to_vec()),
            (None, stream[5000..].to_vec()),
        ];
        let whole = vec![(None, stream)];
        for sources in [whole, halves] {
            for slurp in [false, true] {
                let run = walk_answers(&sources, slurp, true);
                assert_eq!(
                    run,
                    discards::bytewise_in(|| walk_answers(&sources, slurp, true))
                );
                assert_eq!(run.2.len(), 1, "the one complete record is a value");
            }
        }
    }

    /// What a test takes out of a [`SeqStreamWalker`] as it goes: each value's range, place and
    /// bytes (read while the walk still holds them), and the faults.
    #[derive(Default)]
    struct Taken {
        values: Vec<(usize, usize)>,
        places: Vec<(usize, usize)>,
        bytes: Vec<Vec<u8>>,
        faults: Vec<SeqFault>,
    }

    impl Taken {
        fn take(&mut self, walker: &mut SeqStreamWalker) {
            while let Some(((start, end), place)) = walker.pop_value() {
                let (data, base) = walker.window();
                self.values.push((start, end));
                self.places.push(place);
                self.bytes.push(data[start - base..end - base].to_vec());
            }
            while let Some(fault) = walker.pop_fault() {
                self.faults.push(fault);
            }
        }
    }

    /// #7775: the walk made as the bytes arrive gives what the walk of the whole stream gives -- every
    /// value, its place, every fault with its place, and the cuts -- however the bytes are handed over:
    /// over random streams (RS, newlines, NUL, BOM bytes, lines past a 4095-byte `fgets` chunk), cut
    /// into one to three sources and fed in pieces of random sizes: handed over, read whole from a
    /// regular source, or read a `read` at a time from one that hands over less than is asked.
    #[test]
    fn a_walk_fed_in_pieces_answers_as_the_whole_walk_7775() {
        /// A source whose every `read` hands over at most the next of `pieces` bytes.
        struct Trickle<'a> {
            raw: &'a [u8],
            at: usize,
            pieces: std::vec::IntoIter<usize>,
        }
        impl std::io::Read for Trickle<'_> {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                let piece = self.pieces.next().unwrap_or(usize::MAX);
                let n = buf.len().min(piece).min(self.raw.len() - self.at);
                buf[..n].copy_from_slice(&self.raw[self.at..self.at + n]);
                self.at += n;
                Ok(n)
            }
        }
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = move |bound: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % bound as u64) as usize
        };
        let alphabet: &[u8] = b"\x1e\x1e\x1e\n\n {}[]\":,1a0.tnul-\x00\xef\xbb\xbf\\u";
        let mut checked = 0;
        for case in 0..3000 {
            let len = if case % 50 == 0 {
                9000 + next(4000)
            } else {
                1 + next(60)
            };
            let mut stream: Vec<u8> = (0..len).map(|_| alphabet[next(alphabet.len())]).collect();
            if case % 5 == 2 {
                // Short lines around newline-free runs that fill `fgets` chunks, so the walk takes
                // its lines at once around the windows that hold no newline.
                stream.clear();
                for _ in 0..=next(6) {
                    let run = if next(2) == 0 {
                        1 + next(9000)
                    } else {
                        1 + next(30)
                    };
                    stream.extend(
                        (0..run)
                            .map(|_| alphabet[4 + next(alphabet.len() - 4)])
                            .map(|byte| if byte == b'\n' { b'x' } else { byte }),
                    );
                    stream.extend_from_slice(b"\n\x1e1\n");
                }
                stream.truncate(stream.len() - next(3));
            } else if case % 7 == 0 {
                // Records of RFC 7464's own framing, the shape the walk mostly reads.
                stream = (0..=next(40))
                    .flat_map(|i| format!("\x1e{{\"n\":{i}}}\n").into_bytes())
                    .collect();
                if case % 14 == 0 {
                    stream.extend_from_slice(b"\x1e[1,");
                }
            }
            // One to three sources.
            let mut cuts = vec![0, stream.len()];
            for _ in 0..next(3) {
                cuts.push(next(stream.len() + 1));
            }
            cuts.sort_unstable();
            let sources: Vec<(Option<usize>, Vec<u8>)> = cuts
                .windows(2)
                .enumerate()
                .map(|(at, pair)| (Some(at), stream[pair[0]..pair[1]].to_vec()))
                .collect();
            let whole =
                walk_stream_collecting_at(&sources, false, true, Tail::Opened, &mut |_, _| {});
            let mut walker = SeqStreamWalker::new();
            let mut got = Taken::default();
            for (tag, raw) in &sources {
                walker.start_source(*tag);
                if case % 3 == 0 {
                    let mut at = 0;
                    while at < raw.len() {
                        let piece = (1 + next(5000)).min(raw.len() - at);
                        walker.push(&raw[at..at + piece]);
                        walker.advance();
                        got.take(&mut walker);
                        at += piece;
                    }
                } else {
                    let pieces: Vec<usize> = (0..64).map(|_| 1 + next(3000)).collect();
                    let mut source = Trickle {
                        raw,
                        at: 0,
                        pieces: pieces.into_iter(),
                    };
                    let regular = case % 3 == 1;
                    while walker
                        .read_from(&mut source, 1 + next(6000), regular)
                        .unwrap()
                        > 0
                    {
                        walker.advance();
                        got.take(&mut walker);
                    }
                }
                walker.end_source();
                walker.advance();
                got.take(&mut walker);
            }
            walker.end();
            walker.advance();
            got.take(&mut walker);
            assert!(walker.finished(), "{stream:?}");
            assert_eq!(got.values, whole.values, "values of {stream:?}");
            assert_eq!(got.places, whole.locations, "places of {stream:?}");
            assert_eq!(got.faults, whole.faults, "faults of {stream:?}");
            // Each value's bytes are the whole stream's, though the bytes before it may have been
            // let go.
            let want: Vec<Vec<u8>> = whole
                .values
                .iter()
                .map(|&(start, end)| stream[start..end].to_vec())
                .collect();
            assert_eq!(got.bytes, want, "bytes of {stream:?}");
            checked += 1;
        }
        assert_eq!(checked, 3000);
    }

    /// #7775: a value is out as soon as the chunks it is in are whole, before the stream's end.
    #[test]
    fn a_walk_hands_out_a_record_before_the_stream_ends_7775() {
        let mut walker = SeqStreamWalker::new();
        walker.start_source(None);
        walker.push(b"\x1e{\"a\":1}\n\x1e[2");
        walker.advance();
        let mut got = Taken::default();
        got.take(&mut walker);
        assert_eq!(got.values, [(1, 8)]);
        assert_eq!(got.places, [(0, 1)]);
        assert!(got.faults.is_empty() && !walker.finished());
        // The cut-short record is reported only at the end, where jq meets it.
        walker.end();
        walker.advance();
        got.take(&mut walker);
        assert!(walker.finished());
        assert_eq!(got.faults.len(), 1);
        assert_eq!(
            got.faults[0].body,
            "Unfinished JSON term at EOF at line 2, column 3"
        );
    }

    /// #7775: a walk whose values are taken as they come holds the bytes of the records not yet
    /// taken and a read or so, not the stream: 7 MB of records read 4 KiB at a time, with a record cut
    /// short in the middle and a long line, never holds more than a few reads, and answers as the walk
    /// of the whole stream does.
    #[test]
    fn a_walk_lets_go_of_what_it_has_handed_on_7775() {
        let record = |i: usize| match i {
            50_000 => b"\x1e[1,\n".to_vec(),
            70_000 => format!("\x1e\"{}\"\n", "y".repeat(10_000)).into_bytes(),
            _ => format!("\x1e{{\"id\":{i},\"pad\":\"{}\"}}\n", "p".repeat(i % 80)).into_bytes(),
        };
        let stream: Vec<u8> = (0..100_000).flat_map(record).collect();
        let mut walker = SeqStreamWalker::new();
        walker.start_source(Some(0));
        let mut got = Taken::default();
        let mut held = 0usize;
        for piece in stream.chunks(4096) {
            walker.push(piece);
            walker.advance();
            got.take(&mut walker);
            held = held.max(walker.window().0.len());
        }
        walker.end();
        walker.advance();
        got.take(&mut walker);
        let whole = walk_stream_collecting_at(
            &[(Some(0), stream.clone())],
            false,
            true,
            Tail::Opened,
            &mut |_, _| {},
        );
        assert_eq!(got.values, whole.values);
        assert_eq!(got.places, whole.locations);
        assert_eq!(got.faults, whole.faults);
        assert!(held < 256 * 1024, "held {held} bytes at once");
    }
}
