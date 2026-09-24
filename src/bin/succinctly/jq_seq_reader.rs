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
//! **Non-slurp `--seq` can end the whole stream silently, with nothing left
//! to explain it (#2998).** This is not a property of any record's own
//! content, so it sits outside the "moment of detection" model above:
//! real jq's caller (`jq_util_input_next_input`, `src/util.c`) tracks "was
//! this buffer just refilled" in a local variable, reset on every call. The
//! one scan-loop event this module already modelled as an "unreachable in
//! practice" branch -- an RS byte arriving with nothing pending, which
//! `jv_parser_next` returns as a message-less invalid -- is exactly what
//! that caller reads as "no more input", but *only* when it is the first
//! event scanned by the specific call that just performed the terminal
//! `fgets` refill. A later call reusing the same (already-final) buffer,
//! because an earlier call already returned something from it, has no
//! fresh refill to reset that local, so it keeps looping internally
//! instead and absorbs any number of further empty records.
//! `final_buffer_start`/`yielded_in_final_buffer`/`stopped` on [`Reader`]
//! model exactly that: the drop point is [`final_buffer_start`], not a byte
//! the scanner itself finds anything wrong with.
//!
//! **The same local decides `--seq -s`'s error location (#2947/#3003).**
//! Under `-s` nothing is dropped, but whether a later runtime error can
//! name `file:line` or must say `<unknown>` is again a question about
//! *which call* did what: the slurped array keeps its position iff the
//! call that performed the terminal refill returns it without first
//! returning an error -- which an error in the final buffer prevents, and
//! which the EOF branch prevents unless the buffer's last byte completed a
//! value and so ended the call before that branch ran. See
//! [`SeqWarningWalk::slurp_position_lost`] for the full derivation.

use crate::front_matter::UTF8_BOM;

const ASCII_RS: u8 = 0x1e;

/// jq's `MAX_PARSING_DEPTH` (`jv_parse.c`), confirmed against the oracle:
/// 256 nested `[` parse and the 257th reports `Exceeds depth limit`.
const MAX_PARSING_DEPTH: usize = 256;

/// The most bytes one of jq's `fgets` refills can hold: `jq_util_input_read_more`
/// (`src/util.c`) calls `fgets(state->buf, sizeof(state->buf), f)` on a
/// `char buf[4096]`, and `fgets` reserves one byte for the terminating NUL.
const JQ_FGETS_CHUNK: usize = 4095;

/// Where jq's `fgets`-chunked *final* buffer begins, as a raw byte offset
/// into the whole concatenated raw stream (all sources, in order).
///
/// `read_more` (`jq_util_input_read_more`, `src/util.c`) calls `fgets` on a
/// [`JQ_FGETS_CHUNK`]+1-byte buffer, so a chunk ends at a newline or after
/// [`JQ_FGETS_CHUNK`] bytes, whichever comes first -- and `feof` is set
/// only by an `fgets` that actually ran out of input, so a chunk ending on
/// a newline or filling the buffer exactly is followed by one more
/// (possibly empty) buffer. jq refills one *file* at a time, so chunking
/// restarts at the last source's own start; that is also what keeps an
/// empty (or newline-free) trailing source from inheriting an earlier
/// source's chunk boundary.
///
/// Two rules hang off "does offset X fall in jq's last buffer": `-s`'s
/// EOF-location answer ([`SeqWarningWalk::slurp_position_lost`] -- a
/// diagnostic's position) and the non-slurp end-of-stream rule (a *value*,
/// #2998). Returns the stream's own total length when the final buffer is
/// empty, so a caller comparing `offset >= final_buffer_start` correctly
/// finds nothing: no real offset ever reaches the stream's own length --
/// and `final_buffer_start == total` is how that emptiness is read back.
///
/// `total` is the stream's byte length, which [`for_each_warning`] already
/// has from its own `boundaries` walk; a second `Vec`-of-lengths reduction
/// over the same sources on every call was pure waste.
fn final_buffer_start(raw_bytes: &[(Option<usize>, Vec<u8>)], total: usize) -> usize {
    let last: &[u8] = raw_bytes.last().map_or(&[], |(_, raw)| raw.as_slice());
    let last_start = total - last.len();

    let mut chunk = 0usize;
    loop {
        let rest = &last[chunk..];
        let len = rest
            .iter()
            .take(JQ_FGETS_CHUNK)
            .position(|&byte| byte == b'\n')
            .map_or_else(|| rest.len().min(JQ_FGETS_CHUNK), |index| index + 1);
        if chunk + len < last.len() {
            chunk += len;
            continue;
        }
        // The last chunk holding data. An `fgets` that stopped on a
        // newline or on the size limit has not reached EOF yet, so an
        // *empty* buffer follows and that one is final; otherwise this
        // chunk is.
        let stopped_early = len == JQ_FGETS_CHUNK || (len > 0 && rest[len - 1] == b'\n');
        break if stopped_early {
            total
        } else {
            last_start + chunk
        };
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

#[cfg(test)]
impl BomPrefix {
    /// No BOM at all: nothing consumed, nothing malformed.
    pub(crate) const NONE: Self = Self {
        consumed: 0,
        malformed: false,
    };
}

pub(crate) fn bom_prefix(raw_bytes: &[(Option<usize>, Vec<u8>)]) -> BomPrefix {
    let mut consumed = 0;
    for byte in raw_bytes.iter().flat_map(|(_, raw)| raw.iter().copied()) {
        if consumed == UTF8_BOM.len() || byte != UTF8_BOM[consumed] {
            // A mismatch at offset 0 just means "no BOM here"; one after a
            // partial match is the malformed case. Running out of input
            // mid-BOM is neither -- jq is still waiting for the rest.
            return BomPrefix {
                consumed,
                malformed: consumed > 0 && consumed < UTF8_BOM.len(),
            };
        }
        consumed += 1;
    }
    BomPrefix {
        consumed,
        malformed: false,
    }
}

/// Readies `raw_bytes` for [`value_ranges`], which runs over the stream
/// *after* the UTF-8 substitution `get_inputs` applies, and returns the
/// [`BomPrefix`] that walk should apply to it. `bom` is [`bom_prefix`] of
/// these same raw bytes.
///
/// The verdict has to come from the raw bytes, because substitution
/// rewrites exactly the bytes a BOM prefix is made of (#3195/#3199). An
/// invalid lead byte becomes U+FFFD, `EF BF BD`, which itself reads as a
/// malformed BOM prefix: `\xe0\x1e"0` flipped the value walk into
/// malformed-BOM mode and read the string's `0` as a value. And a raw
/// malformed prefix is itself invalid UTF-8, replaced by a U+FFFD of a
/// different width: `\xef\xbb1 2` became `EF BF BD 1 2`, the walk swallowed
/// only the `EF`, and `BF BD 1` read as one junk token, losing jq's `1`.
/// Outside a string a short segment can even collapse into a single U+FFFD
/// and drop the byte after it (`\xef1 2`), so no offset mapping could
/// recover the prefix's end.
///
/// So a malformed prefix, or one split across sources (each fragment
/// invalid on its own), is removed here, before substituting, and the walk
/// skips nothing. A complete BOM inside one source is valid UTF-8 and
/// survives substitution byte for byte, so it stays and the walk skips it,
/// which spares a large BOM-prefixed file a whole-buffer copy.
pub(crate) fn value_walk_bom(
    raw_bytes: &mut [(Option<usize>, Vec<u8>)],
    bom: BomPrefix,
) -> BomPrefix {
    let intact = raw_bytes
        .iter()
        .find(|(_, raw)| !raw.is_empty())
        .map_or(true, |(_, raw)| raw.len() >= bom.consumed);
    if !bom.malformed && intact {
        return bom;
    }
    let mut left = bom.consumed;
    for (_, raw) in raw_bytes.iter_mut() {
        if left == 0 {
            break;
        }
        let n = left.min(raw.len());
        raw.drain(..n);
        left -= n;
    }
    BomPrefix {
        consumed: 0,
        malformed: bom.malformed,
    }
}

/// Every byte of the input stream, as jq's scanner sees it: all sources
/// concatenated, with [`bom_prefix`]'s bytes already removed.
///
/// Shared with [`super::jq_runner::seq_no_rs_byte_warning`] so the two
/// walkers over this same stream cannot drift apart on BOM handling.
pub(crate) fn stream_bytes(
    raw_bytes: &[(Option<usize>, Vec<u8>)],
) -> impl Iterator<Item = u8> + '_ {
    raw_bytes
        .iter()
        .flat_map(|(_, raw)| raw.iter().copied())
        .skip(bom_prefix(raw_bytes).consumed)
}

/// [`stream_bytes`], with each byte's offset in the *undecorated* stream --
/// BOM bytes counted, though (as there) never yielded.
///
/// Absolute offsets, so a caller can compare one against a position it
/// worked out itself over the same sources: a file boundary, a newline.
/// Numbering the yielded bytes from zero instead would silently shift
/// every offset by the BOM's width.
fn indexed_stream_bytes(
    raw_bytes: &[(Option<usize>, Vec<u8>)],
) -> impl Iterator<Item = (usize, u8)> + '_ {
    raw_bytes
        .iter()
        .flat_map(|(_, raw)| raw.iter().copied())
        .enumerate()
        .skip(bom_prefix(raw_bytes).consumed)
}

/// Hands `emit` every `jq: ignoring parse error: ...` line real jq would
/// write for this `--seq` stream, in order.
///
/// A sink rather than a `Vec`: an adversarial stream can warn once per
/// byte, and collecting those first cost ~140x the input in peak RSS
/// (2 MB of `}` measured at 286 MB, against jq's 2.4 MB). Streaming them
/// keeps the reader flat.
///
/// `slurp` matters beyond the warnings: real jq's non-slurp `--seq` driver
/// (`jq_util_input_next_input`, `src/util.c`) can end the *entire* stream
/// silently once it reaches jq's own `fgets`-chunked final buffer -- see
/// [`SeqWarningWalk::value_cap`] and [`final_buffer_start`] (#2998). `-s`
/// keeps looping (`has_more`), so this never applies there; passing
/// `slurp: true` disables it, matching jq's own mode split exactly. The
/// walk's other answer, [`SeqWarningWalk::slurp_position_lost`], is
/// computed either way and only read under `-s`.
pub(crate) fn for_each_warning(
    raw_bytes: &[(Option<usize>, Vec<u8>)],
    slurp: bool,
    emit: &mut dyn FnMut(&str),
) -> SeqWarningWalk {
    let bom = bom_prefix(raw_bytes);
    // With a malformed BOM jq re-runs `parser_reset` at the top of *every*
    // `jv_parser_next` -- including the call for the empty final buffer
    // that a newline-terminated stream produces, since `fgets` stops at
    // the newline without reaching EOF. That reset wipes the accumulated
    // state before the EOF branch can report on it, so no `at EOF` warning
    // survives. A stream not ending in a newline hits EOF in the same read
    // and does report one.
    let ends_with_newline = raw_bytes
        .iter()
        .rev()
        .find_map(|(_, raw)| raw.last().copied())
        == Some(b'\n');
    let mut reader = Reader::new(bom, emit);
    reader.suppress_eof_warning = bom.malformed && ends_with_newline;
    // jq refills its reader one file at a time, so each source after the
    // first begins its own `jv_parser_next` -- whose malformed-BOM reset the
    // walk must reproduce (#3002). The offsets are the sources' absolute
    // boundaries (the end of each but the last, i.e. the start of each
    // subsequent one), matching `indexed_stream_bytes`'s undecorated
    // numbering. The running total this scan ends on is also the stream's
    // whole length, reused below instead of a second `.sum()` over the same
    // sources -- both `final_buffer_start` and the EOF answer need it.
    let mut boundaries = raw_bytes
        .iter()
        .scan(0usize, |end, (_, raw)| {
            *end += raw.len();
            Some(*end)
        })
        .collect::<Vec<usize>>();
    let total = boundaries.last().copied().unwrap_or(0);
    boundaries.pop();
    let final_buffer_start = final_buffer_start(raw_bytes, total);
    reader.final_buffer_start = final_buffer_start;
    reader.drops_at_first_empty_record = !slurp;
    reader.run(indexed_stream_bytes(raw_bytes), &boundaries, None);
    // The same pass answers `--seq -s`'s EOF-location question; see the
    // field's docs for the rule. `final_buffer_start == total` is how
    // [`final_buffer_start`] spells an *empty* final buffer.
    let slurp_position_lost = reader.warned_in_final_buffer
        || (reader.last_warning_at_eof
            && !(reader.last_byte_yielded_value && final_buffer_start < total));
    SeqWarningWalk {
        slurp_position_lost,
        value_cap: reader.stopped.then_some(reader.values.len()),
    }
}

/// What one raw-byte walk over a `--seq` stream ([`for_each_warning`])
/// answers, bundled so a caller that needs both questions (the ordinary
/// non-`-n` case, `jq_runner`'s own call site) pays for the walk once.
pub(crate) struct SeqWarningWalk {
    /// Whether `--seq -s`'s runtime-error location is lost entirely -- real
    /// jq answering `(at <unknown>)` where it would otherwise name a file
    /// and line (#1542/#1550/#1568/#2947/#3003). Only meaningful under
    /// `-s`; a non-slurp stream carries a location per value instead.
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
    /// `Some(n)` when real jq's non-slurp `--seq` driver silently ends the
    /// stream after producing exactly `n` values (#2998) -- `None` under
    /// `-s`, or when nothing is dropped.
    ///
    /// [`value_ranges`] walks the *substituted* byte stream (jq's raw
    /// bytes have already been repaired for invalid UTF-8 by the time
    /// that function's caller has them), so it cannot derive this rule
    /// itself: the drop point is a property of jq's raw `fgets` chunking,
    /// not of the JSON content. Capping by *value count* rather than by
    /// byte offset sidesteps the offset shift -- the two walks scan the same
    /// structural bytes (RS, whitespace, `{}[]:,"`) in the same order, so
    /// they agree on how many values exist and in what order.
    ///
    /// That agreement has two known exceptions. The BOM prefix is one:
    /// substitution rewrites it, so [`value_walk_bom`] hands the value walk
    /// the raw bytes' verdict instead (#3195/#3199). The other is still
    /// open: outside a string, jq's short-tail rule can collapse an invalid
    /// lead byte *and* the RS or whitespace after it into one U+FFFD, so
    /// the substituted walk loses a boundary the raw one sees (#3247).
    pub(crate) value_cap: Option<usize>,
}

/// Byte ranges of the values jq yields from one complete `--seq` stream.
/// Ranges are into `bytes`, which must be the same UTF-8-normalized stream
/// later handed to the materializer.  Diagnostics use the original bytes at
/// their separate call site, so invalid UTF-8 retains jq's raw-byte columns.
///
/// `bom` is [`value_walk_bom`]'s answer for the raw bytes `bytes` was
/// substituted from; the substituted stream cannot be asked (#3195/#3199).
///
/// `boundaries` are the absolute offsets into `bytes` at which each source
/// after the first begins -- the cumulative lengths `build_seq_values`
/// already records for its file remap -- so the walker can apply the
/// malformed-BOM reset jq performs when it refills its own reader with the
/// next file (#3002). A single already-concatenated buffer passes `&[]`.
///
/// `value_cap` is [`SeqWarningWalk::value_cap`], from the matching raw-byte
/// walk over the same stream (#2998): a *count*, not an offset into `bytes`
/// -- see that field's docs for why a count is what the two walks can agree
/// on. Passed to [`Reader::run`] as its own early-stop, so the value past
/// the drop point is never parsed in the first place, not parsed and then
/// discarded; the trailing `truncate` stays as the correctness guarantee
/// (it is what actually enforces the cap; the early stop is purely the
/// optimization) rather than the only enforcement. `None` runs this
/// function exactly as before the drop rule existed.
pub(crate) fn value_ranges(
    bytes: &[u8],
    bom: BomPrefix,
    boundaries: &[usize],
    value_cap: Option<usize>,
) -> Vec<(usize, usize)> {
    let mut ignore = |_: &str| {};
    let mut reader = Reader::new(bom, &mut ignore);
    reader.run(
        bytes.iter().copied().enumerate().skip(bom.consumed),
        boundaries,
        value_cap,
    );
    let mut values = reader.values;
    if let Some(cap) = value_cap {
        values.truncate(cap);
    }
    values
}

/// [`SeqWarningWalk::slurp_position_lost`] on its own: literally
/// [`for_each_warning`] with the warnings thrown away. The stderr
/// diagnostics and this question are the same walk, and the ordinary call
/// site runs it once and uses both answers rather than paying for a second
/// pass over the whole stream; this is for the two `-s` call sites that
/// print no warnings from the walk (#1525's own template, and `-n`/DSV) but
/// still need the location.
///
/// Takes the same raw, pre-UTF-8-substitution sources that function does,
/// and for the same reason -- a substituted byte is a 3-byte U+FFFD, which
/// moves every offset *and* can masquerade as a malformed BOM, whose
/// handling above changes what the reader treats as a record at all.
pub(crate) fn slurp_eof_position_lost(raw_bytes: &[(Option<usize>, Vec<u8>)]) -> bool {
    for_each_warning(raw_bytes, true, &mut |_| {}).slurp_position_lost
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
}

fn classify(c: u8) -> Cls {
    match c {
        b' ' | b'\t' | b'\r' | b'\n' => Cls::Whitespace,
        b'"' => Cls::Quote,
        b'[' | b',' | b']' | b'{' | b':' | b'}' => Cls::Structure,
        _ => Cls::Literal,
    }
}

struct Reader<'a> {
    line: usize,
    column: usize,
    st: St,
    stack: Vec<Frame>,
    /// jq's `tokenbuf`/`tokenpos`, modelled as a buffer that is *not*
    /// cleared when the token is. `check_literal` reads `tokenbuf[1]`
    /// without checking `tokenpos`, so a one-byte `n` token is classified
    /// against whatever the previous token left at index 1: `\x1en` alone
    /// reports "Invalid numeric literal", but the same `n` after an
    /// earlier `iu` token reports "Invalid literal" instead. Reproduced
    /// deliberately (ADR-0018 rule: bug-for-bug), and it is deterministic
    /// for any given input even though jq is reading a stale byte.
    token_buf: Vec<u8>,
    token_len: usize,
    next: Option<Kind>,
    last_ch_was_ws: bool,
    /// A malformed BOM makes jq re-run `parser_reset` at the top of every
    /// `jv_parser_next`. That call returns on each value, on each error,
    /// and when the buffer runs out -- and jq refills a line at a time --
    /// so under this flag the parser is wiped after every value and every
    /// newline as well. An unterminated string therefore stops swallowing
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
    /// See [`for_each_warning`]: a malformed BOM plus a newline-terminated
    /// stream means jq's own EOF report is wiped before it is written.
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
    /// [`final_buffer_start`]), or `usize::MAX` when the caller has no
    /// stream shape to enforce ([`value_ranges`]) -- no real offset ever
    /// reaches that, so none of the comparisons below can fire. Two rules
    /// hang off it: the non-slurp end-of-stream drop (#2998, gated by
    /// [`Reader::drops_at_first_empty_record`]) and `-s`'s EOF-location
    /// answer (#3003, [`SeqWarningWalk::slurp_position_lost`]).
    final_buffer_start: usize,
    /// Whether real jq's non-slurp end-of-stream rule (#2998) applies:
    /// `-s` keeps its own `has_more` loop going past the event that ends a
    /// non-slurp stream, so the drop below never fires there.
    drops_at_first_empty_record: bool,
    /// Whether a value or a warning has already been produced at an offset
    /// `>= final_buffer_start`. jq's own driver (`jq_util_input_next_input`,
    /// `src/util.c`) tracks "is this the buffer that just got refilled"
    /// (`is_last`) as a *local*, reset on every call: only the specific
    /// call that performs the terminal refill can exit silently on a
    /// message-less empty-record reset, and only if that is the very
    /// *first* event the call scans. Any later call reusing the same
    /// buffer (because an earlier call already returned a value or an
    /// error from it) loops internally instead, absorbing any number of
    /// further empty records without dropping anything. This flag is that
    /// "already returned something from this buffer" condition.
    yielded_in_final_buffer: bool,
    /// Whether a *mid-stream* warning (not the EOF branch's) was raised at
    /// an offset `>= final_buffer_start`. Under `-s` that is an early
    /// `return` out of the very `next_input` call that performed the
    /// terminal refill, which is what costs jq its position (#2947/#3003);
    /// see [`SeqWarningWalk::slurp_position_lost`].
    warned_in_final_buffer: bool,
    /// Whether scanning the stream's *last* byte handed a value to the
    /// caller -- jq's `jv_parser_next` returning `OK` with its buffer fully
    /// consumed, so `has_more == 0` and the EOF branch is never reached in
    /// that call (#3003). Captured per byte inside [`Reader::run`]'s loop:
    /// a value [`Reader::finish`] itself completes is not this.
    last_byte_yielded_value: bool,
    /// Set once [`Reader::scan`]'s empty-record reset hits jq's drop point:
    /// [`Reader::run`] stops scanning immediately and skips [`Reader::finish`]
    /// entirely, since real jq's `p->eof` is never set on this path -- no
    /// `at EOF` diagnostic for anything left unscanned (#2998).
    stopped: bool,
    emit: &'a mut dyn FnMut(&str),
}

impl<'a> Reader<'a> {
    fn new(bom: BomPrefix, emit: &'a mut dyn FnMut(&str)) -> Self {
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
            token_buf: Vec::new(),
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
            suppress_eof_warning: false,
            last_warning_at_eof: false,
            at_eof: false,
            final_buffer_start: usize::MAX,
            drops_at_first_empty_record: false,
            yielded_in_final_buffer: false,
            warned_in_final_buffer: false,
            last_byte_yielded_value: false,
            stopped: false,
            emit,
        }
    }

    fn warn(&mut self, body: &str) {
        self.last_warning_at_eof = self.at_eof;
        if self.offset >= self.final_buffer_start {
            self.yielded_in_final_buffer = true;
            if !self.at_eof {
                self.warned_in_final_buffer = true;
            }
        }
        (self.emit)(&format!("jq: ignoring parse error: {body}"));
    }

    /// jq's `parser_reset`. Note it restores `Normal` -- including over a
    /// `WaitingForRs` just assigned by the mid-buffer error path, which is
    /// the upstream bug this module reproduces.
    fn reset(&mut self) {
        self.stack.clear();
        self.token_len = 0;
        self.next = None;
        self.st = St::Normal;
        self.value_start = None;
        self.completed = None;
    }

    /// Walk the stream's scanned bytes, applying the malformed-BOM
    /// `parser_reset` jq runs at the top of every `jv_parser_next` where
    /// this model already does (a value, a newline, an error).
    ///
    /// [`for_each_warning`] additionally hands over the absolute offsets at
    /// which each source after the first begins: jq refills its reader one
    /// *file* at a time, so a source boundary is another `jv_parser_next`,
    /// and its reset wipes whatever survived the previous source's scan
    /// (#3002) -- `EF BF` across two sources therefore vanishes where the
    /// same bytes in one buffer fail (`EF BF` then `1E` cleanly starts a new
    /// record, where `EF BF 1E` still reports `Truncated value`). Resetting
    /// on the first scanned byte at or after the boundary covers it; a
    /// boundary whose source yields no byte at all (an empty trailing file,
    /// or content eaten wholesale by the BOM prefix) still cost jq that
    /// refill, so its reset fires before the EOF branch instead.
    ///
    /// `value_limit`, when given, stops the walk (skipping [`Reader::finish`]
    /// too, exactly like the `stopped` early-return below) the moment
    /// [`Reader::values`] reaches that length. [`value_ranges`] uses this to
    /// avoid parsing the value #2998's drop rule is about to truncate away
    /// anyway -- [`for_each_warning`] passes `None`, since its own diagnostics
    /// need the whole stream scanned regardless of how many values complete.
    fn run(
        &mut self,
        bytes: impl Iterator<Item = (usize, u8)>,
        boundaries: &[usize],
        value_limit: Option<usize>,
    ) {
        let mut boundary = 0;
        let mut eof_offset = 0;
        for (offset, ch) in bytes {
            self.offset = offset;
            eof_offset = offset + 1;
            while boundary < boundaries.len() && boundaries[boundary] <= offset {
                if self.resets_per_call {
                    self.reset();
                }
                boundary += 1;
            }
            if self.st == St::WaitingForRs {
                if ch == b'\n' {
                    self.line += 1;
                    self.column = 0;
                } else {
                    self.column += 1;
                }
                if ch == ASCII_RS {
                    self.st = St::Normal;
                }
                continue;
            }
            if self.st == St::Normal
                && self.stack.is_empty()
                && self.next.is_none()
                && self.token_len == 0
                && !ch.is_ascii_whitespace()
                && ch != ASCII_RS
            {
                self.value_start = Some(offset);
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
            if self.stopped {
                // jq's stream ends *here*: `p->eof` is never set on this
                // path, so nothing past this byte is ever scanned and no
                // `at EOF` diagnostic follows (#2998).
                return;
            }
            if value_limit.is_some_and(|limit| self.values.len() >= limit) {
                return;
            }
        }
        // A boundary with no scanned byte at or after it (a trailing source
        // that is empty or consumed wholesale by the BOM prefix) still cost
        // jq the refill and its reset -- with a malformed BOM that wipes the
        // pending token before the EOF branch can report on it, so an empty
        // terminal source reads `EF BF` as nothing at all.
        if self.resets_per_call {
            while boundary < boundaries.len() {
                self.reset();
                boundary += 1;
            }
        }
        self.offset = eof_offset;
        self.at_eof = true;
        self.finish();
    }

    fn flush_completed(&mut self) {
        if let Some(range) = self.completed.take() {
            if self.offset >= self.final_buffer_start {
                self.yielded_in_final_buffer = true;
            }
            self.values.push(range);
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
            // `*out = jv_invalid()`). Real jq's caller
            // (`jq_util_input_next_input`, `src/util.c`) tracks whether the
            // *current* buffer was just refilled in a local variable reset
            // on every call, so it treats that message-less return as "no
            // more input" only when this is the first event scanned from
            // the stream's final `fgets` buffer -- any later call reusing
            // the same buffer (because an earlier call already returned a
            // value or an error from it) simply loops and keeps parsing.
            // #2998. `-s` has its own `has_more` loop and never stops here.
            if self.drops_at_first_empty_record
                && self.offset >= self.final_buffer_start
                && !self.yielded_in_final_buffer
            {
                self.stopped = true;
            }
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
                if self.stack.len() >= MAX_PARSING_DEPTH {
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
        // "Invalid literal".
        let pattern: Option<&[u8]> = match self.token_buf[0] {
            b't' => Some(b"true"),
            b'f' => Some(b"false"),
            // Deliberately the whole buffer, not just the token: see the
            // field's docs.
            b'n' if self.token_buf.get(1) == Some(&b'u') => Some(b"null"),
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
                if !number_is_valid(&self.token_buf[..self.token_len]) {
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

    /// jq's `found_string`: the escape-validation half, which is all that
    /// can raise a diagnostic.
    fn found_string(&mut self) -> Result<(), &'static str> {
        let buf = self.token_buf[..self.token_len].to_vec();
        let mut i = 0;
        while i < buf.len() {
            let c = buf[i];
            i += 1;
            if c != b'\\' {
                // jq compares a *signed* char, so only U+0000..=U+001F
                // qualifies -- bytes >= 0x80 are negative there and pass.
                if c <= 0x1f {
                    return Err(
                        "Invalid string: control characters from U+0000 through U+001F must be escaped",
                    );
                }
                continue;
            }
            // Unreachable: a trailing `\\` puts the scanner in
            // `StrEscape`, so the next byte is consumed as the escaped one
            // and the string cannot end here. jq carries the arm anyway.
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
            // -- `for_each_warning` routes a stream with no RS byte to
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

fn unhex4(bytes: &[u8]) -> Option<u32> {
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
fn number_is_valid(token: &[u8]) -> bool {
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
    // `map_or(true, ..)` rather than `is_none_or`: the crate's MSRV is
    // 1.73 and `Option::is_none_or` is 1.82.
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
        // `jq --seq -c .`, not `-s` (#2998 gives the two modes different
        // end-of-stream behavior, but none of these inputs reach it --
        // see `warnings_are_unaffected_by_the_2998_drop_rule` below).
        for_each_warning(&owned, false, &mut |w| {
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

    /// [`SeqWarningWalk::value_cap`] plus [`value_ranges`], wired the way
    /// `build_seq_values` wires them in production (#2998) -- the decoded
    /// texts of the values that survive for one single-source, plain-ASCII
    /// stream (so the raw and "substituted" byte streams are identical,
    /// letting this test stay at the byte level rather than routing
    /// through UTF-8 substitution too).
    fn seq_value_texts(input: &[u8], slurp: bool) -> Vec<String> {
        let mut owned = vec![(None, input.to_vec())];
        let cap = for_each_warning(&owned, slurp, &mut |_| {}).value_cap;
        let bom = {
            let raw = bom_prefix(&owned);
            value_walk_bom(&mut owned, raw)
        };
        let stream = &owned[0].1;
        value_ranges(stream, bom, &[], cap)
            .into_iter()
            .map(|(start, end)| String::from_utf8(stream[start..end].to_vec()).unwrap())
            .collect()
    }

    /// #2653: values are committed only after the scan call which completed
    /// them has succeeded. Whitespace commits `1` before a malformed object;
    /// a comma instead causes the same scan call to fail, so it is discarded.
    #[test]
    fn value_ranges_follow_jq_recovery_2653() {
        assert_eq!(
            value_ranges(b"\x1e1 {invalid\n", BomPrefix::NONE, &[], None),
            vec![(1, 2)]
        );
        assert_eq!(
            value_ranges(b"\x1e1,2\n", BomPrefix::NONE, &[], None),
            vec![(3, 4)]
        );
        assert_eq!(
            value_ranges(b"\x1e1-2\n", BomPrefix::NONE, &[], None),
            Vec::<(usize, usize)>::new()
        );
        assert_eq!(
            value_ranges(b"\x1e5-3 7\n", BomPrefix::NONE, &[], None),
            vec![(5, 6)]
        );
    }

    /// Real jq's non-slurp `--seq` driver can end the *entire* stream
    /// silently at the first empty record of jq's own final `fgets`
    /// buffer (#2998) -- oracle-verified against `/usr/bin/jq` 1.7.1,
    /// every row here confirmed live. None of these produce a warning
    /// either: jq's `p->eof` is never set on this path, so there is no
    /// `at EOF` diagnostic for whatever went unscanned.
    #[test]
    fn end_of_stream_rule_drops_the_final_buffers_leading_empty_record_2998() {
        for input in [
            &b"\x1e\x1etrue"[..],
            b"\x1e\x1e\"s\"",
            b"\x1e\x1e5",
            b"\x1e \x1e{}",
            b"\x1e\n\x1etrue",
        ] {
            assert_eq!(
                seq_value_texts(input, false),
                Vec::<String>::new(),
                "input: {input:?}"
            );
            assert_eq!(warnings(&[input]), Vec::<String>::new(), "input: {input:?}");
        }
    }

    /// An earlier record's own *value* leaves the parser in the same state
    /// a genuinely empty one would (both return to `NORMAL` with nothing
    /// pending) -- the drop doesn't need the empty record to be the
    /// stream's first, just the final buffer's own first event. `1` still
    /// prints: it was returned by an earlier `fgets` chunk, before the
    /// buffer this rule ever looks at existed.
    #[test]
    fn end_of_stream_rule_only_needs_the_final_buffers_own_first_event_to_be_empty_2998() {
        assert_eq!(seq_value_texts(b"\x1e1\n\x1etrue", false), vec!["1"]);
        assert_eq!(warnings(&[b"\x1e1\n\x1etrue"]), Vec::<String>::new());
        // The suppression reaches later warnings in the same (final)
        // buffer too: no "Unfinished string at EOF" here, even though
        // `"abc` never gets a closing quote -- jq's `p->eof` is never set
        // on this path.
        assert_eq!(seq_value_texts(b"\x1e1\n\x1e\x1e\"abc", false), vec!["1"]);
        assert_eq!(warnings(&[b"\x1e1\n\x1e\x1e\"abc"]), Vec::<String>::new());
    }

    /// The drop fires only when the empty record is the *first* event
    /// scanned from the final buffer -- an earlier value or error in the
    /// same (single, newline-free) buffer means every later empty record
    /// is just absorbed, matching real jq's own per-call `is_last` reset.
    #[test]
    fn end_of_stream_rule_spares_a_buffer_whose_first_event_is_not_empty_2998() {
        assert_eq!(seq_value_texts(b"\x1e1 \x1etrue", false), vec!["1", "true"]);
        assert_eq!(
            seq_value_texts(b"\x1e{}\x1etrue", false),
            vec!["{}", "true"]
        );
        assert_eq!(
            seq_value_texts(b"\x1e\"a\"\x1etrue", false),
            vec!["\"a\"", "true"]
        );
        // The first event is an *error* (truncated `1`), not a value --
        // still not empty, so the rest of the buffer parses normally.
        assert_eq!(
            seq_value_texts(b"\x1e1\x1e2 \x1etrue", false),
            vec!["2", "true"]
        );
    }

    /// Controls: a trailing newline empties the final buffer entirely (no
    /// offset in the stream can reach it), so nothing is ever dropped by
    /// this rule regardless of what came before.
    #[test]
    fn end_of_stream_rule_never_fires_on_a_trailing_newline_2998() {
        assert_eq!(seq_value_texts(b"\x1e\x1etrue\n", false), vec!["true"]);
        assert_eq!(
            seq_value_texts(b"\x1e\x1etrue\n\x1e5\n", false),
            vec!["true", "5"]
        );
    }

    /// `-s` is unaffected: real jq's `has_more` keeps its own read loop
    /// going regardless of which buffer produced the empty record, so the
    /// value survives and the warning the non-slurp rule would have
    /// suppressed fires normally.
    #[test]
    fn end_of_stream_rule_does_not_apply_under_slurp_2998() {
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
        for_each_warning(&owned, true, &mut |w| {
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
    /// the values: keeping the position changes neither.
    #[test]
    fn final_byte_rule_leaves_warnings_and_values_alone_3003() {
        assert_eq!(
            warnings_slurp(&[b"\x1e1{"]),
            ["Unfinished JSON term at EOF at line 1, column 3"],
        );
        assert_eq!(seq_value_texts(b"\x1e1{", true), vec!["1"]);
        assert_eq!(seq_value_texts(b"\x1e1{", false), vec!["1"]);
    }

    /// [`final_buffer_start`]'s own boundary arithmetic: real jq's `fgets`
    /// stops at a newline *or* after 4095 bytes, whichever comes first, so
    /// padding a stream by one byte flips which side of the boundary the
    /// trailing record lands on (#2998).
    #[test]
    fn final_buffer_start_matches_jqs_4095_byte_fgets_chunk() {
        let padded = |spaces: usize| {
            let mut v = b"\x1e1\n\x1e".to_vec();
            v.extend(std::iter::repeat_n(b' ', spaces));
            v.extend_from_slice(b"\x1etrue");
            v
        };
        // Tail (from the second RS onward) is `spaces + 6` bytes; jq's
        // chunk is 4095 bytes, so 4093 keeps `true` and 4094 drops it --
        // and the same one-byte flip recurs at the next chunk (8188/8189).
        assert_eq!(seq_value_texts(&padded(4093), false), vec!["1", "true"]);
        assert_eq!(seq_value_texts(&padded(4094), false), vec!["1"]);
        assert_eq!(seq_value_texts(&padded(4095), false), vec!["1"]);
        assert_eq!(seq_value_texts(&padded(8188), false), vec!["1", "true"]);
        assert_eq!(seq_value_texts(&padded(8189), false), vec!["1"]);
    }

    /// Multi-file: only the *last* file's own final buffer can trigger the
    /// drop -- an earlier file's newline-free tail is `is_partial` and
    /// joins the next file instead (#3002), and an empty last file has no
    /// final buffer to drop from at all.
    #[test]
    fn end_of_stream_rule_only_applies_to_the_last_file_2998() {
        let a: &[u8] = b"\x1e\x1etrue"; // no trailing newline
        let b: &[u8] = b"\x1e5\n";
        let owned = vec![(Some(0), a.to_vec()), (Some(1), b.to_vec())];
        let cap = for_each_warning(&owned, false, &mut |_| {}).value_cap;
        assert_eq!(cap, None, "a's own drop is masked by joining with b");
        let combined: Vec<u8> = a.iter().chain(b.iter()).copied().collect();
        assert_eq!(
            value_ranges(&combined, BomPrefix::NONE, &[a.len()], cap)
                .into_iter()
                .map(|(s, e)| String::from_utf8(combined[s..e].to_vec()).unwrap())
                .collect::<Vec<_>>(),
            vec!["5"],
            "a's own leading-RS content still joins b and is malformed there"
        );

        let empty: &[u8] = b"";
        let owned = vec![(Some(0), a.to_vec()), (Some(1), empty.to_vec())];
        let cap = for_each_warning(&owned, false, &mut |_| {}).value_cap;
        assert_eq!(
            cap, None,
            "an empty trailing file has no final buffer to drop from"
        );
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
            b"\x1eNaN5",
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
    /// number parser -- and a one-character number token NUL-terminates
    /// itself, which is what keeps a later `n` on the number path too.
    #[test]
    fn keyword_path_depends_on_the_second_buffer_byte_1723() {
        assert_warnings(b"\x1enul", &["Invalid literal at EOF at line 1, column 4"]);
        assert_warnings(
            b"\x1en",
            &["Invalid numeric literal at EOF at line 1, column 2"],
        );
        // `iu` leaves a `u` at index 1, so the following bare `n` is
        // measured against `null` -- jq reads the stale byte (#1723).
        assert_warnings(
            b"\x1eiu\"n",
            &[
                "Invalid numeric literal at line 1, column 4 (need RS to resync)",
                "Invalid literal at EOF at line 1, column 5",
            ],
        );
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
    /// discarded when jq would have parsed it. All captured from
    /// `/usr/bin/jq` 1.7.1.
    #[test]
    fn malformed_bom_is_consumed_and_starts_the_parser_reading_1723() {
        assert_warnings(
            b"\xef\xbb\x1e[1,]\n",
            &["Expected another array element at line 1, column 5 (need RS to resync)"],
        );
        // Read as a record, so this is *not* #1525's abandoned-text
        // template even though the stream holds no RS byte at all.
        for input in [&b"\xef\xbb1 2"[..], b"\xef1 2"] {
            assert_warnings(
                input,
                &["Potentially truncated top-level numeric value at EOF at line 1, column 3"],
            );
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
        // Stripped as production strips it: jq swallows the `EF`, and the
        // dangling `BF` is left for the walk.
        let mut sources = vec![(None, b"\xef\xbf".to_vec()), (None, b"1 \n".to_vec())];
        let bom = {
            let raw = bom_prefix(&sources);
            value_walk_bom(&mut sources, raw)
        };
        assert_eq!(
            bom,
            BomPrefix {
                consumed: 0,
                malformed: true
            }
        );
        let a = &sources[0].1;
        let combined: Vec<u8> = a.iter().chain(&sources[1].1).copied().collect();
        // Source `a`'s exclusive end is the boundary (`b` starts there).
        assert_eq!(value_ranges(&combined, bom, &[a.len()], None), vec![(1, 2)]);
        // Same bytes in one buffer: `BF` derails and the record is dropped.
        assert_eq!(
            value_ranges(&combined, bom, &[], None),
            Vec::<(usize, usize)>::new()
        );
    }

    /// #3195/#3199: `value_walk_bom` keeps the raw verdict, removes a
    /// malformed or source-straddling prefix before substitution would
    /// rewrite it, and leaves an intact complete BOM for the walk to skip.
    #[test]
    fn value_walk_bom_takes_the_raw_prefix_and_verdict_3195_3199() {
        fn prepare(sources: &[&[u8]]) -> (BomPrefix, Vec<Vec<u8>>) {
            let mut owned: Vec<(Option<usize>, Vec<u8>)> =
                sources.iter().map(|s| (None, s.to_vec())).collect();
            let bom = {
                let raw = bom_prefix(&owned);
                value_walk_bom(&mut owned, raw)
            };
            (bom, owned.into_iter().map(|(_, b)| b).collect())
        }
        let stripped = BomPrefix {
            consumed: 0,
            malformed: true,
        };
        assert_eq!(
            prepare(&[b"\xef\xbb1 2"]),
            (stripped, vec![b"1 2".to_vec()])
        );
        assert_eq!(prepare(&[b"\xef1 2"]), (stripped, vec![b"1 2".to_vec()]));
        // A complete BOM in one source stays, for the walk to skip.
        assert_eq!(
            prepare(&[b"\xef\xbb\xbf\x1e1"]),
            (
                BomPrefix {
                    consumed: 3,
                    malformed: false
                },
                vec![b"\xef\xbb\xbf\x1e1".to_vec()]
            )
        );
        // An invalid lead byte that is not a BOM prefix stays, unflagged.
        assert_eq!(
            prepare(&[b"\xe0\x1e\"0"]),
            (BomPrefix::NONE, vec![b"\xe0\x1e\"0".to_vec()])
        );
        // Split across sources, including an empty one in between: removed,
        // since each fragment alone would substitute to a U+FFFD.
        assert_eq!(
            prepare(&[b"\xef", b"", b"\xbb1"]),
            (stripped, vec![vec![], vec![], b"1".to_vec()])
        );
        assert_eq!(
            prepare(&[b"\xef\xbb", b"\xbf\x1e1"]),
            (BomPrefix::NONE, vec![vec![], b"\x1e1".to_vec()])
        );
        // A complete BOM after an empty leading source is still intact.
        assert_eq!(
            prepare(&[b"", b"\xef\xbb\xbf1"]),
            (
                BomPrefix {
                    consumed: 3,
                    malformed: false
                },
                vec![vec![], b"\xef\xbb\xbf1".to_vec()]
            )
        );
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
}
