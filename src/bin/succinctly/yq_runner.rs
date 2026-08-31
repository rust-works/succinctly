//! yq-compatible command runner for succinctly.
//!
//! This module implements a yq-compatible CLI interface using the succinctly
//! YAML semi-indexing and jq expression evaluator.

use anyhow::{Context, Result};
use indexmap::IndexMap;
use std::cell::Cell;
use std::io::{BufWriter, IsTerminal, Read, Write};
use std::path::Path;

use succinctly::jq::document::{
    effective_keys, key_delimiter_ok, resolve_display_key, value_delimiter_ok, DisplayKeyGuard,
    DocumentCursor, DocumentElements, DocumentFields, DocumentValue, IndentSpec,
};
use succinctly::jq::escape::AsciiEscapeWriter;
use succinctly::jq::eval_generic::{
    assert_nesting_depth, eval_with_cursor_using, to_owned as generic_to_owned,
    to_owned_with_comments, AnchorMark, CommentTree, GenericResult, NodeMeta,
};
use succinctly::jq::stream::StreamFailure;
use succinctly::jq::{
    self, assert_value_tree_depth, nonfinite_display_string, sync_aliased_paths, Builtin,
    EvalError, Expr, NumberRepr, OwnedValue, QueryResult, YqSemantics,
};
use succinctly::json::JsonIndex;
use succinctly::yaml::{
    format_float_yq_yaml, format_float_yq_yaml_nested, resolve_plain, resolve_tagged,
    stream_json_sequence, stream_yaml_sequence, YamlCursor, YamlIndex, YamlValue,
};

use super::{FrontMatterMode, InputFormat, OutputFormat, YqCommand};
use crate::front_matter;
use crate::output::{
    self, exit_codes, flush_then_err, ColorScheme, ControlEscape, DiagStyle, ErrorSink, FloatStyle,
    InputLocation, JsonFormatOpts, LoudFlushWriter, Terminator,
};

/// yq's diagnostics carry no `(at <file>:<line>)` marker, so the yq paths have
/// no location to report — unlike jq, whose marker names the input value (#355).
fn no_location() -> InputLocation {
    InputLocation::unknown()
}

/// Route a streamed `GenericResult`'s terminating outcome into `sink`: a halt
/// (#791) outranks an error, since `StreamStats::halt` carries the real exit
/// code and must reach `sink.request_halt` directly, never `report_stream` —
/// that path would both misreport the exit code and print a spurious "not
/// propagated" diagnostic no real jq/yq ever emits. Streaming never writes a
/// diagnostic to stdout; an error is handed back here so it reaches stderr
/// and fails the run (#355). Shared by `stream_cursor!`'s YAML and JSON arms,
/// which otherwise duplicated this precedence check verbatim.
fn absorb_stream_stats(sink: &mut ErrorSink, stats: &succinctly::jq::stream::StreamStats) {
    if let Some(code) = stats.halt {
        sink.request_halt(code);
    } else if let Some(err) = &stats.error {
        sink.report_stream(DiagStyle::Yq, err, &no_location());
    }
}

/// Matches real yq's own message text (Homebrew v4.53.3, live-verified,
/// #1709) for a `-0`/`--nul-output` result whose rendered bytes contain a
/// raw NUL character.
const NUL_OUTPUT_ERROR_MESSAGE: &str =
    "can't serialise value because it contains NUL char and you are using NUL separated output";

/// Adapter to use `std::io::Write` with `core::fmt::Write` methods.
/// This enables streaming JSON output without intermediate String allocation.
struct FmtWriter<W>(W);

impl<W: Write> core::fmt::Write for FmtWriter<W> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.0.write_all(s.as_bytes()).map_err(|_| core::fmt::Error)
    }
}

/// Either a direct passthrough to the real output writer, or an in-memory
/// buffer collecting output to be colorized afterward. `colorize_yaml`/
/// `output::colorize_json` are pure text-level re-lexers over an already
/// fully-rendered string, so buffering the duplicate-key-safe cursor
/// streamer's output and running it through them unmodified reuses that
/// existing coloring code without teaching the streamers anything about
/// ANSI codes (#748).
enum ColorSink<'a, W: Write> {
    Buffered(String, Vec<usize>),
    Direct(FmtWriter<&'a mut W>),
    /// Per-result NUL-checked buffering (#1709), used instead of `Direct`
    /// when `-0`/`--nul-output` is active and `--color` isn't. See
    /// [`NulCheckedSink`]'s own doc comment for why this needs to buffer
    /// one result at a time rather than either streaming straight through
    /// (`Direct`, which would leak an invalid result's own already-rendered
    /// prefix bytes before detecting its embedded NUL) or buffering the
    /// whole document like `Buffered` does (which would reintroduce the
    /// #789-class memory regression a prior attempt at this issue, PR
    /// #1767, measured and got closed over).
    NulChecked(NulCheckedSink<'a, W>),
}

impl<W: Write> core::fmt::Write for ColorSink<'_, W> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        match self {
            ColorSink::Buffered(buf, _) => buf.write_str(s),
            ColorSink::Direct(w) => w.write_str(s),
            ColorSink::NulChecked(sink) => sink.buf.write_str(s),
        }
    }
}

/// Threads `stream_cursor!`'s own `$doc_streamed`/`no_doc` state into
/// [`stream_maybe_colored`] for lazy `---` emission (#1709). `None` for
/// JSON output call sites, which have no document-separator concept at
/// all.
struct DocSeparatorArgs<'a> {
    doc_streamed: &'a mut bool,
    no_doc: bool,
}

/// Lazy `---` document-separator state carried inside [`NulCheckedSink`]
/// (#1709). `emit_yaml_doc_separator`'s ordinary eager write (before the
/// body even renders) is wrong for NUL-checked output: verified live
/// against the pinned oracle (Homebrew yq v4.53.3) that real yq does NOT
/// write a document's `---` when that document's *first* result fails the
/// NUL check, even though the document would otherwise structurally
/// produce output -- but DOES keep the separator once at least one earlier
/// result in the same document already flushed clean. So the separator has
/// to be decided at the same per-result granularity as the NUL check
/// itself, not before it.
struct PendingSeparator<'a> {
    /// Mirrors `stream_cursor!`'s own `$doc_streamed`: has any *prior*
    /// document already produced real output. Only ever set, never
    /// cleared, so a document that fails outright doesn't un-flag one that
    /// already succeeded earlier.
    doc_streamed: &'a mut bool,
    no_doc: bool,
    /// Whether this document's own separator question has already been
    /// settled (written, or correctly skipped for `no_doc`/being first) --
    /// distinct from `doc_streamed`, which is global: without this, every
    /// result after the first in the *same* document would see
    /// `doc_streamed` already `true` (set by the first result's own flush)
    /// and try to insert another `---` before itself.
    settled: bool,
}

/// Per-result buffered writer used when `-0`/`--nul-output` is active and
/// `--color` isn't (#1709). Buffers ONE streamed result's rendered text at
/// a time -- not the whole document/stream, unlike `ColorSink::Buffered`
/// above (which exists for an unrelated reason, color re-lexing, and pays
/// for materializing every result of a whole document up front as an
/// accepted, pre-existing cost of `--color` specifically). A per-*result*
/// buffer keeps bounded, O(one result) memory regardless of document size,
/// matching this crate's M2 streaming architecture's own memory guarantee.
///
/// A prior attempt at this exact issue (PR #1767, closed without merging)
/// buffered the entire rendered stream instead whenever `-0` was set, and
/// measured +65% peak RSS on a 100MB document as a result -- buffering the
/// wrong granularity, not buffering itself, was the regression.
///
/// Verified live against the pinned oracle (Homebrew yq v4.53.3) that
/// per-result is also the *correct* granularity, not just the cheap one:
/// earlier results in a stream flush with their real terminator even when
/// a later one fails (`1,2,("a"+NUL+"b"),4` under `-0` prints `1\0002\000`
/// then errors, producing nothing for the third result and never reaching
/// the fourth) -- exactly the shape one flush-buffer-per-result gives for
/// free, with no separate "is this scalar being unwrapped" mode predicate
/// to derive (see the issue's own second post-mortem comment): the gate is
/// simply "does this result's rendered buffer contain a raw NUL byte."
struct NulCheckedSink<'a, W: Write> {
    writer: &'a mut W,
    buf: String,
    nul_detected: &'a Cell<bool>,
    separator: Option<PendingSeparator<'a>>,
}

impl<W: Write> NulCheckedSink<'_, W> {
    /// Checks the current per-result buffer for an embedded NUL byte,
    /// clearing the buffer either way. On success, writes the pending
    /// `---` separator first (if this is the document's first surviving
    /// result), then the buffer's own contents -- but not the terminator,
    /// which callers add separately: [`Self::flush_result`] for the
    /// evaluated multi-result path's per-result hook, or
    /// [`stream_maybe_colored`]'s own post-render step for the identity
    /// path, whose `stream_yaml_as_document` has no per-result callback to
    /// hook a flush into.
    fn flush_body(&mut self) -> core::fmt::Result {
        if self.buf.as_bytes().contains(&0) {
            self.nul_detected.set(true);
            self.buf.clear();
            return Err(core::fmt::Error);
        }
        if let Some(sep) = &mut self.separator {
            if !sep.settled {
                // Reuses the same guard/write/set-flag logic every other
                // terminator mode's separator goes through, rather than
                // hand-duplicating it here (#1709 code review) -- `true`
                // for `will_output` since reaching this point already
                // means this result survived the NUL check above.
                // `Terminator::Nul` is a literal, not `self.terminator`:
                // this sink is only ever constructed for that terminator
                // (see its one call site in `stream_maybe_colored`), so a
                // stored field would just be a second name for the same
                // constant.
                emit_yaml_doc_separator(
                    self.writer,
                    sep.doc_streamed,
                    true,
                    sep.no_doc,
                    Terminator::Nul,
                )
                .map_err(|_| core::fmt::Error)?;
                sep.settled = true;
            }
        }
        self.writer
            .write_all(self.buf.as_bytes())
            .map_err(|_| core::fmt::Error)?;
        self.buf.clear();
        Ok(())
    }

    /// [`Self::flush_body`] plus the real terminator -- the evaluated
    /// multi-result path's per-result hook, reached through
    /// [`ColorSink::write_result_terminator`].
    fn flush_result(&mut self, terminator: Terminator) -> core::fmt::Result {
        self.flush_body()?;
        terminator
            .write_io(self.writer)
            .map_err(|_| core::fmt::Error)
    }
}

impl<W: Write> ColorSink<'_, W> {
    /// Records a streamed result's boundary as a byte offset into the
    /// buffer, instead of writing a real terminator character into it
    /// (#1708). `colorize_yaml` re-lexes the whole buffer once, at the end,
    /// and inserts the real terminator at each recorded offset, closing
    /// whatever color span is still open there first -- which an in-band
    /// marker character can't do safely, since this crate's own writers can
    /// and do emit any raw byte (including a chosen marker's own value)
    /// when the source YAML/JSON explicitly encodes one. A no-op in
    /// `Direct` mode, whose caller writes its terminator immediately
    /// instead (see `stream_cursor!`).
    fn record_boundary(&mut self) {
        if let ColorSink::Buffered(buf, boundaries) = self {
            boundaries.push(buf.len());
        }
    }

    /// Writes a streamed result's terminator through whichever mode `self`
    /// is in: records a boundary for `colorize_yaml` to place later when
    /// `Buffered`, or writes it immediately when `Direct` -- so a caller
    /// never needs its own `use_color` check to pick between the two, only
    /// `ColorSink`'s own variant (which already reflects it). Named
    /// distinctly from the free [`write_terminator`] function (which writes
    /// to the real `$writer`/`OutputConfig` outside any `ColorSink` at all)
    /// to keep the two apart when searching this file.
    fn write_result_terminator(&mut self, terminator: Terminator) -> core::fmt::Result {
        match self {
            ColorSink::Buffered(..) => {
                self.record_boundary();
                Ok(())
            }
            ColorSink::Direct(_) => terminator.write_fmt(self),
            ColorSink::NulChecked(sink) => sink.flush_result(terminator),
        }
    }
}

/// Reaches [`ColorSink::write_result_terminator`] through whichever of the
/// two concrete types a `json_ascii!`-wrapped `on_value` callback actually
/// holds (#1709): a bare `ColorSink` when `--ascii-output` is off, or an
/// `AsciiEscapeWriter` around one when it's on. `write_result_terminator`
/// is a `ColorSink`-specific method, not part of `core::fmt::Write`, so it
/// isn't reachable through the wrapper's own `Write` impl the way a plain
/// `terminator.write_fmt(w)` call is -- and going through the wrapper's
/// escaping for the terminator's own bytes would be a no-op in any case
/// (every terminator this crate writes is already ASCII), so bypassing it
/// via [`AsciiEscapeWriter::inner_mut`] is sound, not just convenient.
trait DispatchResultTerminator {
    fn dispatch_result_terminator(&mut self, terminator: Terminator) -> core::fmt::Result;
}

impl<W: Write> DispatchResultTerminator for ColorSink<'_, W> {
    fn dispatch_result_terminator(&mut self, terminator: Terminator) -> core::fmt::Result {
        self.write_result_terminator(terminator)
    }
}

impl<W: Write> DispatchResultTerminator for AsciiEscapeWriter<'_, ColorSink<'_, W>> {
    fn dispatch_result_terminator(&mut self, terminator: Terminator) -> core::fmt::Result {
        self.inner_mut().write_result_terminator(terminator)
    }
}

/// Apply `--ascii-output` to a JSON render (#1700).
///
/// Wraps `$sink` in [`AsciiEscapeWriter`] when the flag is set and passes it
/// through untouched when it is not, so the default path keeps exactly the
/// code it had before — no per-write branch, and no extra layer between the
/// M2 streamers and the writer (the `if` costs one predictable test per
/// streaming call, not one per character). `$body` is duplicated across the
/// two arms for that reason; it is a single streaming call at every use site.
/// The duplication is not free, and the cost is code size rather than any
/// branch: it monomorphizes each streamer twice, worth +108 KB (+1.3%) of
/// release binary on arm64 and +123 KB (+1.1%) on x86_64 across the six use
/// sites (measured against this commit's parent on the pinned benchmark
/// hosts). That shows up in time on x86_64. An interleaved A/B of the
/// *default*, non-`--ascii-output` path — `.` and `.[]` over 1 MB and 10 MB
/// `users`/`navigation` documents, 7 reps, output identity gated — reads
/// +0.49% median with **every one of 8 rows slower** on a 7950X, against a
/// control floor there of −0.00% median / 4-of-8-slower; the same run on an
/// M4 Pro is −1.21% median, 0 of 8 slower, against a −0.48% floor. Small, but
/// the sign consistency on x86 makes it a real regression rather than noise,
/// and it is the effect #965 was sensitive to arriving by a different route
/// (code size, not a lost inline). It is accepted here because the flag it
/// buys back was silently dropping duplicate mapping keys; if it ever needs
/// recovering, the lever is to stop instantiating the streamers a second time
/// — give the `--ascii-output` arm a `&mut dyn Write` and leave the default
/// arm concrete, which pays a vtable call only on the branch that is already
/// doing per-character escaping.
///
/// Escaping the *sink* rather than teaching the streamers an `ascii` flag is
/// sound because of a property of JSON rather than of any particular writer —
/// see [`AsciiEscapeWriter`]'s own comment for that argument. What it buys is
/// that the flag cannot be missed at one of the streamers' three separate
/// string-writing routes (`stream_json_string` and the two quoted-scalar
/// transcoders); what it costs is that every JSON write route *in this file*
/// has to go through this macro instead. Since `can_stream_json_output_style`
/// no longer excludes `--ascii-output`, a route that skipped it would stream
/// raw UTF-8 under the flag.
///
/// Three of the six use sites are reachable — both arms of `stream_cursor!`
/// and `--slurp`'s `stream_json_sequence` — and
/// `test_ascii_output_covers_every_live_json_route_1700` in
/// `tests/yq_cli_tests.rs` pins those against exactly that failure. The other
/// three (the two `stream_json_document` calls and `--inplace`'s direct
/// `FmtWriter`) are in `_ =>` arms this file documents as defensive fallbacks
/// no input reaches, so they carry the wrap without a test to hold it: a
/// probe at each of the six sites showed every non-`--slurp` invocation —
/// file, multi-document, `--inplace` and `-C` alike — landing on the identity
/// arm. Wrap them anyway; #757's lesson in `docs/compliance/yq/limitations.md`
/// is that routes move, and this is the loss such a move would cause.
macro_rules! json_ascii {
    ($ascii:expr, $sink:expr, |$out:ident| $body:expr) => {{
        let sink = $sink;
        if $ascii {
            let mut escaped = AsciiEscapeWriter::new(sink);
            let $out = &mut escaped;
            $body
        } else {
            let $out = sink;
            $body
        }
    }};
}

/// Streams through `render` directly when `use_color` is false. When true,
/// renders into a buffer instead (still via the duplicate-key-safe cursor
/// streamer passed in as `render`), then runs the buffer through `colorize`
/// before writing the colorized result (#748).
/// Render `render`'s output to `writer`, colorizing first if asked.
///
/// The nested `Result` is #1615's decode-failure channel, not redundancy: the
/// outer `anyhow::Result` is a genuine *write* failure (the process cannot
/// continue), while `Ok(Err(e))` is a well-formed run over a document holding
/// a scalar that will not decode — a diagnostic for stderr and an exit code,
/// which only the caller's [`ErrorSink`] can set. Collapsing the two would put
/// a data error on the I/O path and lose the message, which is exactly the
/// "bare, undiagnosed abort" that kept this gap open (see
/// `docs/plan/decode-failure-routing.md`, Stage 6).
///
/// Whatever reached `out` before the failure is still written, colorized path
/// included — the same keep-the-prefix-and-diagnose trade #1641/#1679 settled
/// for their own streaming sites.
///
/// `terminator`/`separator` (#1709): when `terminator` is `Terminator::Nul`
/// and `use_color` is false, dispatches to `ColorSink::NulChecked` instead
/// of `Direct` — see that type's own doc comment for why per-result
/// buffering, not raw passthrough, is required to match real yq's own
/// flush-then-error atomicity without regressing this crate's streaming
/// memory guarantee. `separator` threads `stream_cursor!`'s own
/// `$doc_streamed`/`no_doc` state through for the YAML call sites' lazy
/// `---` emission; `None` for JSON, which has no separator concept.
///
/// The `use_color` path keeps its existing whole-buffer shape (color
/// re-lexing already requires materializing the whole rendered document, a
/// pre-existing cost unrelated to `-0`); a `-0`+`--color` combination is
/// checked for an embedded NUL over that same already-buffered content
/// before writing, coarser-grained (whole document, not per-result) than
/// the no-color path below -- bounded by the same memory this combination
/// already pays for coloring, but NOT flush-then-error atomic the way the
/// no-color path is: real yq's own `--colors -0` still flushes earlier
/// valid results before erroring on a later NUL-containing one
/// (live-verified), while this whole-buffer check discards every result in
/// the render, including already-clean earlier ones, on any NUL anywhere
/// in the stream. A real, currently-open divergence, not a documented
/// design choice — see `docs/compliance/yq/limitations.md`'s `-0`
/// multi-document separator entry and #2004.
fn stream_maybe_colored<W: Write, T>(
    writer: &mut W,
    use_color: bool,
    terminator: Terminator,
    separator: Option<DocSeparatorArgs<'_>>,
    colorize: impl FnOnce(&str, &[usize]) -> String,
    render: impl FnOnce(&mut ColorSink<'_, W>) -> Result<T, StreamFailure>,
) -> anyhow::Result<Result<T, EvalError>> {
    if use_color {
        let mut sink = ColorSink::Buffered(String::new(), Vec::new());
        let rendered = render(&mut sink);
        let ColorSink::Buffered(buf, boundaries) = sink else {
            unreachable!("stream_maybe_colored always constructs ColorSink::Buffered here")
        };
        // `separator.is_some()` doubles as "this is a YAML call site"
        // (#1709): YAML's `Buffered` mode never embeds a raw terminator
        // character into `buf` itself (`write_result_terminator` records a
        // boundary offset instead, exactly so `colorize_yaml` can place a
        // real terminator later, correctly relative to whatever color span
        // is open there). JSON's own on_value closures write
        // `terminator.write_fmt(w)` directly, so JSON's buffer *does*
        // contain raw `\0` terminator bytes between every result when
        // `-0` is active -- scanning it here would false-positive on the
        // terminator's own byte, not a value actually containing one.
        // Narrowing this check to YAML leaves `--color -o=json -0`
        // unvalidated; that combination already can't be safely consumed
        // by a NUL-delimited reader once ANSI escapes are mixed in, so
        // `-0`'s own reason for existing doesn't really apply to it either.
        if terminator == Terminator::Nul && separator.is_some() && buf.as_bytes().contains(&0) {
            return Err(anyhow::anyhow!(NUL_OUTPUT_ERROR_MESSAGE));
        }
        // #1709: `separator` is only ever `Some` for a NUL-checked YAML
        // call site (its caller skipped its own eager
        // `emit_yaml_doc_separator` call for exactly this reason -- see
        // the `NulChecked` branch below), and the NUL scan just above
        // already gates reaching this point on "the document's own
        // content survived." Write the separator now, directly to
        // `writer` -- never colorized, matching every other call site's
        // own separator write -- before the colorized body. A no-op for
        // every other `use_color` call (terminator != Nul, or JSON, both
        // of which pass `None`).
        if let Some(sep) = separator {
            if *sep.doc_streamed && !sep.no_doc {
                write_doc_separator_marker(writer, terminator)?;
            }
            *sep.doc_streamed = true;
        }
        write!(writer, "{}", colorize(&buf, &boundaries))?;
        match rendered {
            Ok(value) => Ok(Ok(value)),
            Err(StreamFailure::Decode(e)) => Ok(Err(e)),
            Err(StreamFailure::Fmt) => Err(anyhow::anyhow!("Write error")),
        }
    } else if terminator == Terminator::Nul {
        let nul_detected = Cell::new(false);
        let sink_state = NulCheckedSink {
            writer,
            buf: String::new(),
            nul_detected: &nul_detected,
            separator: separator.map(|s| PendingSeparator {
                doc_streamed: s.doc_streamed,
                no_doc: s.no_doc,
                settled: false,
            }),
        };
        let mut sink = ColorSink::NulChecked(sink_state);
        let rendered = render(&mut sink);
        let ColorSink::NulChecked(mut sink_state) = sink else {
            unreachable!("stream_maybe_colored always constructs ColorSink::NulChecked here")
        };
        // The identity path leaves exactly one un-flushed result in the
        // buffer -- `stream_yaml_as_document`/`stream_json_as_document`
        // have no per-result callback to flush through `write_result_
        // terminator`, unlike the evaluated multi-result path, whose own
        // `on_value` calls already drained the buffer via `flush_result`
        // before `render` ever returns. A no-op there.
        //
        // Flushed even on a `StreamFailure::Decode` (#1709 code review):
        // whatever rendered into the buffer before a mid-stream decode
        // failure is still real, already-checked content, and this file's
        // own established keep-the-prefix-and-diagnose trade (#1641/#1679,
        // this function's own doc comment above) says to write it rather
        // than silently drop it -- the `Buffered`/color arm above already
        // does this unconditionally. Only a genuine write failure
        // (`StreamFailure::Fmt`) means the underlying writer itself is
        // already broken, with nothing left to safely flush.
        let rendered = if !matches!(rendered, Err(StreamFailure::Fmt)) && !sink_state.buf.is_empty()
        {
            match sink_state.flush_body() {
                Ok(()) => rendered,
                Err(e) => Err(StreamFailure::from(e)),
            }
        } else {
            rendered
        };
        if nul_detected.get() {
            return Err(anyhow::anyhow!(NUL_OUTPUT_ERROR_MESSAGE));
        }
        match rendered {
            Ok(value) => Ok(Ok(value)),
            Err(StreamFailure::Decode(e)) => Ok(Err(e)),
            Err(StreamFailure::Fmt) => Err(anyhow::anyhow!("Write error")),
        }
    } else {
        let mut sink = ColorSink::Direct(FmtWriter(writer));
        match render(&mut sink) {
            Ok(value) => Ok(Ok(value)),
            Err(StreamFailure::Decode(e)) => Ok(Err(e)),
            Err(StreamFailure::Fmt) => Err(anyhow::anyhow!("Write error")),
        }
    }
}

/// [`stream_maybe_colored`] for a render that produces no value, reporting a
/// decode failure to `sink` instead of returning it (#1615).
///
/// Answers whether the render completed, so the caller can skip a trailing
/// terminator write for output that was cut short.
fn stream_or_report<W: Write>(
    writer: &mut W,
    use_color: bool,
    terminator: Terminator,
    sink: &mut ErrorSink,
    colorize: impl FnOnce(&str, &[usize]) -> String,
    render: impl FnOnce(&mut ColorSink<'_, W>) -> Result<(), StreamFailure>,
) -> anyhow::Result<bool> {
    match stream_maybe_colored(writer, use_color, terminator, None, colorize, render)? {
        Ok(()) => Ok(true),
        Err(e) => {
            report_stream_decode_failure(sink, &e);
            Ok(false)
        }
    }
}

/// Route a streamed decode failure to the same [`ErrorSink`] every
/// *materializing* route already reports through (#1615), so one document
/// gives one answer — message and exit code — however it was rendered.
fn report_stream_decode_failure(sink: &mut ErrorSink, err: &EvalError) {
    sink.report_stream(
        DiagStyle::Yq,
        &succinctly::jq::stream::StreamError {
            message: err.message.clone(),
            not_a_string: false,
        },
        &no_location(),
    );
}

/// Evaluation context for passing variables to the jq evaluator.
#[derive(Debug, Default)]
pub struct EvalContext {
    /// Named arguments from --arg, --argjson
    pub named: IndexMap<String, OwnedValue>,
}

/// Resolve `OutputFormat::Auto` against a concrete `InputFormat` (#1493):
/// real yq's `-o=auto` matches the input's own format -- YAML input stays
/// YAML, JSON input becomes JSON. Anything already concrete passes through
/// unchanged. `InputFormat::Auto` shouldn't reach here in practice
/// (`resolve_input_format`/`detect_format_from_path` already resolve it
/// before any source reaches this point), but it's handled the same as
/// `InputFormat::Yaml` (Yaml is the safer default of the two either way).
/// Real yq's own "nothing to match" case (`--null-input`, `--raw-input`,
/// mixed-format `--slurp`) also defaults to Yaml -- confirmed live: `yq -n
/// -p=json '{}' -o=auto` still prints YAML (with a backwards-compatibility
/// warning), not JSON, so callers with no per-source `InputFormat` of their
/// own should resolve against `InputFormat::Yaml` here too, not skip the
/// call.
fn resolve_output_format(output_format: OutputFormat, input_format: InputFormat) -> OutputFormat {
    if output_format != OutputFormat::Auto {
        return output_format;
    }
    match input_format {
        InputFormat::Json => OutputFormat::Json,
        InputFormat::Yaml | InputFormat::Auto => OutputFormat::Yaml,
    }
}

/// Output configuration
#[derive(Clone)]
struct OutputConfig {
    output_format: OutputFormat,
    compact: bool,
    raw_output: bool,
    join_output: bool,
    nul_output: bool,
    ascii_output: bool,
    sort_keys: bool,
    no_doc: bool,
    indent_str: String,
    use_color: bool,
    /// A JSON-sourced float never keeps a decimal point in output,
    /// computed or not, `-o=json` or `-o=yaml`, compact or pretty (#978,
    /// confirmed live against yq v4.53.3: `-p json '. + 1'` on `1.0` gives
    /// `2` in every output mode, where the same computation on genuine
    /// YAML input keeps `2.0` under pretty `-o=json`). The cursor-native
    /// evaluator (#1398) no longer routes JSON input through a text
    /// round-trip that happened to erase the decimal point as a side
    /// effect (`evaluate_input`'s reindex bridge), so this flag makes the
    /// override explicit instead of relying on that accident. Per-input-
    /// source, not global: set by the caller for the specific file/stream
    /// currently being formatted, since `--input-format auto` can mix
    /// JSON and YAML sources in one invocation.
    json_sourced_floats: bool,
}

impl OutputConfig {
    fn from_args(args: &YqCommand) -> Self {
        // Shares the jq runner's precedence, documented on `resolve_color`.
        let use_color = crate::env_config::resolve_color(
            crate::env_config::ColorChoice::from_flags(args.monochrome_output, args.color_output),
            crate::env_config::no_color_from_env(),
            std::io::stdout().is_terminal(),
        );

        // Compact output when indent is 0 (yq-compatible)
        let compact = args.indent == 0;

        // `args.output_format` itself, not yet resolved against any
        // source's `InputFormat` -- see `Self::for_source` below (#1493).
        let indent_str = Self::compute_indent_str(args, args.output_format);

        Self {
            output_format: args.output_format,
            compact,
            raw_output: args.raw_output || args.join_output || args.nul_output,
            join_output: args.join_output,
            nul_output: args.nul_output,
            ascii_output: args.ascii_output,
            sort_keys: args.sort_keys,
            no_doc: args.no_doc,
            indent_str,
            use_color,
            json_sourced_floats: false,
        }
    }

    /// The YAML clamp itself -- `-I0`/`-I1` both landing on width 2 -- is
    /// `IndentSpec::for_yaml`'s rule (#1486, #1575, #1685; shared with the
    /// M2 streaming fast path's own indent setup below, previously an
    /// independently hand-encoded copy of the identical formula). `indent_str`
    /// is shared by both formats' DOM emitters (see its use in the JSON
    /// branch of `output_value`), so the clamp only applies when
    /// `output_value` will actually take the YAML branch, i.e.
    /// `effective_output_format == Yaml` exactly -- matching that dispatch's
    /// own condition, not its complement. JSON has no such clamp (verified
    /// live: `-I1 -o=json` genuinely indents 1 space per level in real yq,
    /// and `-I0 -o=json` means compact/flow, handled separately by
    /// `OutputConfig::compact`).
    ///
    /// `--tab` is handled once here rather than delegated to
    /// `IndentSpec::for_yaml`: unlike the clamp, a single tab character
    /// applies to *both* output formats identically, so it belongs at this
    /// shared entry point, not inside the YAML-specific constructor.
    ///
    /// Takes the caller's *effective* format rather than reading
    /// `args.output_format` directly: `Self::from_args` passes
    /// `args.output_format` itself (nothing resolved yet), while
    /// `Self::for_source` passes the per-source *resolved* format, so an
    /// `Auto` that resolves to `Yaml` for a given source still gets the
    /// clamp there, and one that resolves to `Json` doesn't (#1493) --
    /// before that fix, `Auto` was treated as "not YAML" here
    /// unconditionally, since `-o=auto` always rendered JSON regardless of
    /// input format anyway.
    fn compute_indent_str(args: &YqCommand, effective_output_format: OutputFormat) -> String {
        if args.tab {
            return "\t".to_string();
        }
        let width = if effective_output_format == OutputFormat::Yaml {
            IndentSpec::for_yaml(args.indent, false).width
        } else {
            args.indent as usize
        };
        " ".repeat(width)
    }

    /// Per-source override (#1493): resolves `OutputFormat::Auto` against
    /// this specific source's own `InputFormat` (real yq's `-o=auto`
    /// matches the input format, confirmed live), recomputing `indent_str`
    /// to match, and folds in the pre-existing `json_sourced_floats`
    /// override (#978/#1398) for the same reason -- both need the
    /// *specific* source's own format, not the invocation-wide default,
    /// since `--input-format auto` can mix JSON and YAML sources in one
    /// run.
    fn for_source(&self, args: &YqCommand, input_format: InputFormat) -> Self {
        let output_format = resolve_output_format(self.output_format, input_format);
        // Every other field is a plain `Copy` bool -- written out
        // explicitly rather than `..self.clone()`, which would otherwise
        // heap-allocate a throwaway copy of the old `indent_str` only to
        // immediately discard it in favor of the recomputed one below.
        Self {
            output_format,
            compact: self.compact,
            raw_output: self.raw_output,
            join_output: self.join_output,
            nul_output: self.nul_output,
            ascii_output: self.ascii_output,
            sort_keys: self.sort_keys,
            no_doc: self.no_doc,
            indent_str: Self::compute_indent_str(args, output_format),
            use_color: self.use_color,
            json_sourced_floats: input_format == InputFormat::Json,
        }
    }
}

/// Convert a YAML value to an OwnedValue for jq evaluation.
///
/// Takes a cursor rather than a bare `YamlValue`: an explicit tag
/// (`!!str`, `!!int`, …) lives on the cursor's `bp_pos`
/// ([`YamlCursor::explicit_tag`]), not on the extracted value, and forces
/// resolution regardless of quoting style — `!!int "5"` converts to the
/// number 5, matching real `yq` (#224). Every recursive call passes a
/// cursor too (`field.value_cursor()`, `YamlElements::uncons_cursor`), so a
/// tag on a nested element is never lost.
///
/// Materializes through `to_owned_value_for_json_bridge`, not the plain
/// `to_owned_value`: everything this function builds is headed for
/// [`evaluate_input`]'s `to_json_for_reindex::<JqSemantics>` round trip,
/// the one bridge that would otherwise flatten a tag-forced `!!float 2`
/// to an `Int` (#1176). That variant's doc comment explains why no other
/// `ResolvedScalar -> OwnedValue` caller wants the same treatment.
fn yaml_to_owned_value<W: AsRef<[u64]>>(cursor: YamlCursor<'_, W>) -> Result<OwnedValue> {
    match cursor.value() {
        YamlValue::String(s) => {
            let str_value = s
                .as_str()
                .map_err(|e| anyhow::anyhow!("invalid YAML string: {e}"))?;

            if let Some(explicit) = cursor.explicit_tag() {
                if let Some(resolved) = resolve_tagged(&str_value, explicit) {
                    return Ok(resolved.to_owned_value_for_json_bridge(str_value));
                }
            }

            // Quoted strings should always be treated as strings (yq-compatible behavior)
            // Only unquoted scalars should undergo type detection
            if !s.is_unquoted() {
                return Ok(OwnedValue::String(str_value.into_owned()));
            }

            // Resolve plain scalars per the YAML 1.2 core schema
            Ok(resolve_plain(&str_value).to_owned_value_for_json_bridge(str_value))
        }
        YamlValue::Mapping(fields) => {
            let mut map = IndexMap::new();
            // #1749: two complex/undecodable keys (e.g. two different
            // sequence keys) both stringify to "" per #222 -- real yq's own
            // streaming/DOM paths keep both entries (its underlying
            // representation isn't a plain map), but `OwnedValue::Object`'s
            // `IndexMap<String, _>` cannot hold two values under one key.
            // Guarding against that silent overwrite the same way
            // #1642/#1738 guard a JSON decode-failure collision: an
            // *ordinary* repeated genuine key still overwrites without
            // complaint (matching jq's own last-key-wins), only a
            // fallback-spelling collision raises.
            let mut guard = DisplayKeyGuard::default();
            for field in fields {
                let (key, is_fallback) = field.key().key_string_kind();
                let key = key.into_owned();
                if !guard.check(&map, &key, is_fallback) {
                    return Err(anyhow::anyhow!(
                        "{}",
                        EvalError::colliding_display_key(&key)
                    ));
                }
                let value = yaml_to_owned_value(field.value_cursor())?;
                map.insert(key, value);
            }
            Ok(OwnedValue::Object(map))
        }
        YamlValue::Sequence(elements) => {
            let mut arr = Vec::new();
            let mut rest = elements;
            // `uncons_resolved_cursor`, not `uncons_cursor`: the recursive
            // call's own `cursor.explicit_tag()` above doesn't resolve a
            // bare `-` sequence-item wrapper itself (see
            // `YamlCursor::anchor`'s doc comment for why), so an
            // unresolved cursor here would silently drop an explicit tag
            // on a bare-dash-deferred scalar (#835).
            while let Some((elem_cursor, next)) = rest.uncons_resolved_cursor() {
                arr.push(yaml_to_owned_value(elem_cursor)?);
                rest = next;
            }
            Ok(OwnedValue::Array(arr))
        }
        YamlValue::Alias { target, .. } => {
            // Resolve the *entire* alias chain first (#1193), not just this
            // one hop: the resolved cursor's own `.value()` is guaranteed
            // non-`Alias`, so this recursive call terminates in exactly one
            // more step regardless of chain length.
            match target.and_then(|t| t.resolve_alias_target_cursor()) {
                Some(resolved) => yaml_to_owned_value(resolved),
                // Unresolved (dangling) target - treat as null
                None => Ok(OwnedValue::Null),
            }
        }
        YamlValue::Error(msg) => Err(anyhow::anyhow!("YAML error: {msg}")),
        YamlValue::Null => Ok(OwnedValue::Null),
    }
}

/// Read input from stdin as bytes.
fn read_stdin() -> Result<Vec<u8>> {
    let mut buffer = Vec::new();
    std::io::stdin()
        .read_to_end(&mut buffer)
        .context("failed to read from stdin")?;
    Ok(buffer)
}

/// Read input from stdin as a string.
fn read_stdin_string() -> Result<String> {
    let mut buffer = String::new();
    std::io::stdin()
        .read_to_string(&mut buffer)
        .context("failed to read from stdin")?;
    Ok(buffer)
}

/// Read input from a file.
fn read_file(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).with_context(|| format!("failed to read file: {}", path.display()))
}

/// For a YAML/Auto-format input, always rejects a non-UTF-8 byte stream
/// outright (#1242) -- this half is unconditional, not gated on
/// `--validate`, matching real yq's own refusal of a stray byte. Then, only
/// when `--validate` is set, additionally runs the opt-in strict validator
/// (`succinctly::yaml::validate`) before indexing and, on the first
/// violation, prints a rustc-style diagnostic. Either failure returns the
/// exit code to bail with. The `--validate` half mirrors `sjq --validate`
/// (`jq_runner::validate_json_input`), an opt-in strict check for its own
/// primary format the same way this one is for YAML.
///
/// This function returns `None` unconditionally for `InputFormat::Json`
/// (`-p json`) -- **neither half runs for JSON input**, not "runs
/// elsewhere": `validate_json_input` above is `sjq`'s own flag, reached only
/// from the `jq` subcommand, and is never called from this file. `-p json`
/// therefore gets no encoding or grammar check of any kind today, a known
/// silent-data-loss gap tracked as #1616.
fn yaml_validate_guard(
    input: &[u8],
    format: InputFormat,
    validate: bool,
    filename: Option<&str>,
) -> Option<i32> {
    if !matches!(format, InputFormat::Yaml | InputFormat::Auto) {
        return None;
    }
    // Encoding is checked *unconditionally*, not only under `--validate`
    // (#1242). YAML 1.2 requires a UTF-8/16/32 stream and real yq rejects a
    // document with a stray byte outright (`invalid trailing UTF-8 octet`,
    // exit 1); succinctly used to accept it, index it, and then hand back
    // `null` for the scalar that byte was in -- with `--validate` no help,
    // since the strict validator had no encoding check either. The pass is
    // whole-input SIMD UTF-8 validation, ~1.1 ms on 8.4 MB, so this is the
    // one always-on cost the fix adds; the strict grammar walk below stays
    // opt-in because it is roughly eight times dearer.
    //
    // Wording follows this crate's own `YAML parse error:` convention (see
    // the `YamlIndex::build` failure path) rather than yq's `bad file '-':`
    // shape, which succinctly already diverges from for every other parse
    // error.
    if let Err(err) = succinctly::text::utf8::validate_utf8(input) {
        eprintln!("Error: YAML parse error: {err}");
        return Some(exit_codes::YQ_FAILURE);
    }
    if !validate {
        return None;
    }
    // `Validator` directly, not the top-level `validate()` wrapper: the
    // encoding pass above already confirmed `input` is valid UTF-8, so
    // `validate()`'s own leading `validate_utf8` call (needed for its other
    // caller, `succinctly yaml validate`, which has no such upfront check)
    // would just re-scan the same unmodified buffer for a result already
    // known.
    match succinctly::yaml::validate::Validator::new(input).validate() {
        Ok(()) => None,
        Err(err) => {
            print_yaml_validation_error(&err, input, filename);
            Some(exit_codes::COMPILE_ERROR)
        }
    }
}

/// Print a YAML validation error with a line/column location and a caret snippet.
fn print_yaml_validation_error(
    err: &succinctly::yaml::validate::YamlValidationError,
    input: &[u8],
    filename: Option<&str>,
) {
    let pos = &err.position;
    eprintln!("yq: validation error: {}", err.kind);
    let location = filename.map_or_else(
        || format!("<stdin>:{}:{}", pos.line, pos.column),
        |f| format!("{}:{}:{}", f, pos.line, pos.column),
    );
    eprintln!("  --> {location}");

    let text = String::from_utf8_lossy(input);
    if let Some(line_content) = text.lines().nth(pos.line.saturating_sub(1)) {
        let width = pos.line.to_string().len().max(3);
        let pad = " ".repeat(width + 2);
        eprintln!("{pad}|");
        eprintln!(" {:>width$} | {}", pos.line, line_content, width = width);
        eprintln!("{}| {}^", pad, " ".repeat(pos.column.saturating_sub(1)));
    }
    eprintln!();
}

/// Detect input format from file extension.
fn detect_format_from_path(path: &Path) -> InputFormat {
    match path.extension().and_then(|e| e.to_str()) {
        Some("json") => InputFormat::Json,
        Some("yaml" | "yml") => InputFormat::Yaml,
        _ => InputFormat::Yaml, // Default to YAML
    }
}

/// Get effective input format, resolving Auto to a specific format.
fn resolve_input_format(format: InputFormat, path: Option<&Path>) -> InputFormat {
    match format {
        InputFormat::Auto => path.map_or(InputFormat::Yaml, detect_format_from_path),
        other => other,
    }
}

/// Applies `--front-matter`, if set, to raw input bytes before format
/// resolution and validation: the raw bytes (e.g. Markdown) aren't valid
/// standalone YAML, so extraction must happen first. Returns
/// `(bytes, format, body)`; `body` is `Some` only in `process` mode, where
/// the caller must reattach it verbatim after the transformed front matter.
///
/// Once a mode is set, the returned format is always `InputFormat::Yaml`
/// regardless of `resolved_format` -- front matter is YAML by definition,
/// and the `run_yq` compat guard already rejects an explicit
/// `--input-format json` paired with `--front-matter`, so this never
/// actually overrides a caller's real preference.
fn apply_front_matter(
    raw_bytes: Vec<u8>,
    resolved_format: InputFormat,
    front_matter: Option<FrontMatterMode>,
    name: &str,
) -> Result<(Vec<u8>, InputFormat, Option<Vec<u8>>)> {
    let Some(mode) = front_matter else {
        return Ok((raw_bytes, resolved_format, None));
    };
    let fm =
        front_matter::split_front_matter(&raw_bytes).map_err(|e| anyhow::anyhow!("{name}: {e}"))?;
    let body = (mode == FrontMatterMode::Process).then(|| fm.body.to_vec());
    Ok((fm.yaml.to_vec(), InputFormat::Yaml, body))
}

/// One gathered input source: its bytes, resolved format, and (only under
/// `--front-matter=process`) the untouched body carried alongside it.
type GatheredSource = (Vec<u8>, InputFormat, Option<Vec<u8>>);

/// Reads each input source -- stdin if `input_files` is empty, else each
/// file in listed order -- applying `--front-matter` (a no-op unless the
/// flag is set, via [`apply_front_matter`]) and resolving its format, then
/// running the shared `--validate` guard. Shared by `--eval-all`,
/// `--split-exp`, `--slurp`, and the default path, which all start from
/// this identical stdin-or-per-file gather step; each `--front-matter`
/// body is `None` unless `front_matter` is `Some(Process)`, same contract
/// as `apply_front_matter`.
///
/// Stops at the first source that fails `--validate` (matching jq's own
/// stop-at-first-failure semantics, #1558) and returns the exit code for
/// it alongside every earlier source that DID pass. The caller decides
/// what to do with that valid prefix: `--slurp`/`--eval-all` discard it
/// (both combine every source into one value, so a partial set can't
/// stand in for "every file combined"), while the per-file-independent
/// paths (the default path, `--split-exp`) process and emit it before
/// reporting the failure -- matching the M2-streaming path's own
/// already-established "earlier valid output survives" behavior (#1564).
fn gather_input_sources(
    input_files: &[String],
    input_format: InputFormat,
    front_matter: Option<FrontMatterMode>,
    validate: bool,
) -> Result<(Vec<GatheredSource>, Option<i32>)> {
    let mut sources = Vec::new();
    if input_files.is_empty() {
        let raw_bytes = read_stdin()?;
        let resolved_format = resolve_input_format(input_format, None);
        let (input_bytes, format, body) =
            apply_front_matter(raw_bytes, resolved_format, front_matter, "<stdin>")?;
        if let Some(code) = yaml_validate_guard(&input_bytes, format, validate, None) {
            return Ok((sources, Some(code)));
        }
        sources.push((input_bytes, format, body));
    } else {
        for file_path in input_files {
            let path = Path::new(file_path);
            let raw_bytes = read_file(path)?;
            let resolved_format = resolve_input_format(input_format, Some(path));
            let (input_bytes, format, body) =
                apply_front_matter(raw_bytes, resolved_format, front_matter, file_path)?;
            if let Some(code) = yaml_validate_guard(&input_bytes, format, validate, Some(file_path))
            {
                return Ok((sources, Some(code)));
            }
            sources.push((input_bytes, format, body));
        }
    }
    Ok((sources, None))
}

/// Materializes a `DocumentValue` into an `OwnedValue` the same way
/// `eval_generic::to_owned` does, except every number collapses straight to
/// `Int`/`Float` in this single walk rather than boxing a `NumberLiteral`
/// first. Real yq's own `--input-format json` path never preserves a
/// JSON-sourced number's exact spelling regardless of whether the filter
/// touches it (#978) -- unlike YAML input (#918, correctly preserved) --
/// so this used to be a second full pass here, `canonicalize_json_numbers`,
/// over `to_owned`'s own already-materialized tree (rebuilding every
/// `Array`/`Object` a second time, even subtrees with no numbers at all,
/// just to strip the spelling `to_owned` had just boxed) (#999).
///
/// Kept local to this file rather than added to the shared
/// `eval_generic`/`jq` library, which `to_owned` and every other
/// `DocumentValue` materializer there live in and stay
/// format/tool-agnostic: this is a yq-CLI-specific, `--input-format json`-
/// specific oracle quirk (matching real yq's own number handling, not a
/// property of JSON or of this evaluator in general), and the library's own
/// `to_owned_at_depth` is also called from its internal recursion (YAML
/// explicit-tag resolution), which must keep preserving a document's own
/// number spelling unconditionally -- a `pub`, `DocumentValue`-generic
/// twin living in the library next to it would be reachable (with no
/// compiler guard) by a future YAML or `succinctly jq` call site, silently
/// reintroducing the #918 bug this project was built to avoid (#999
/// review). `OwnedValue::from_number_literal_plain` is the one small,
/// clearly-scoped primitive the library exports for this -- everything
/// else about the walk stays here, matching where `canonicalize_json_numbers`
/// always lived before this fix, just fused into one pass instead of two.
///
/// Returns `Err` when two colliding decode-failure keys (#1642/#1738) would
/// otherwise silently overwrite one another in the same object -- see
/// `eval_generic::to_owned_at_depth`'s identical guard, which this function
/// now shares via [`resolve_display_key`] rather than the hand-rolled
/// `key_display_string` call it used to make (#1738: that gap meant this,
/// the `--input-format json` bridge, was the one materializer PR #1672 never
/// reached).
///
/// Also raises on a #1194/#1677 structurally malformed document (a bare
/// non-string key, an unpaired trailing member, a missing/doubled `,`/`:`,
/// or a value token the semi-index couldn't classify) -- #1975 found this
/// function missing every one of `to_owned_at_depth`'s checks for these,
/// despite this doc comment's own "mirrors ... exactly" claim above.
fn to_owned_canonicalizing_numbers<V: DocumentValue>(value: &V) -> Result<OwnedValue, EvalError> {
    to_owned_canonicalizing_numbers_at_depth(value, 0)
}

fn to_owned_canonicalizing_numbers_at_depth<V: DocumentValue>(
    value: &V,
    depth: usize,
) -> Result<OwnedValue, EvalError> {
    // `assert_nesting_depth` (256, matching `eval_generic::to_owned_at_depth`
    // exactly), not `assert_value_tree_depth` (384) -- this walks a
    // `DocumentCursor`-backed `V` directly, the same recursion shape
    // `to_owned_at_depth` guards with the tighter limit, not the looser
    // guard the old two-pass code used for its *second* pass over an
    // already-materialized `OwnedValue` tree (#999 review: reusing the
    // looser guard here would have quietly raised the effective limit for
    // `--input-format json` documents from 256 to 384, since this is now
    // the only pass -- there's no first, 256-guarded `to_owned` call ahead
    // of it anymore to bind the real ceiling).
    assert_nesting_depth(depth);
    Ok(if let Some(fields) = value.as_object() {
        let mut map = IndexMap::new();
        let mut guard = DisplayKeyGuard::default();
        let mut f = fields;
        let mut is_first = true;
        while let Some((field, rest)) = f.uncons() {
            // `resolve_display_key`, not `field.key_str()`: a key that will
            // not *decode* (#1247/#1385) is preserved via its raw source
            // span rather than dropped (#1642) -- this loop used to
            // silently drop such a field under `--slurp`/`--eval-all`/
            // `--inplace` (the only callers of `parse_input`, hence of this
            // function), one short of `length`'s real field count. A key
            // that will not *stringify* at all -- a key JSON's grammar never
            // allowed (`{123: 1}`) -- is a different, structural fault
            // (#1194) and now raises instead of silently dropping the whole
            // field (#1975: this used to be an `if let Some(key) = ... {}`
            // with no `else`, the exact pattern #1679 already fixed at five
            // other call sites, but missed here). Two colliding
            // decode-failure keys now raise instead of silently overwriting
            // one another (#1642/#1738), matching every other materializer.
            let Some(key) = resolve_display_key(&field.key, &map, &mut guard)? else {
                return Err(f.malformed_member_error());
            };
            // #1975: this walk was missing the #1677 malformed-`,`/`:`
            // delimiter check entirely -- unlike its
            // `eval_generic::to_owned_at_depth` sibling it otherwise
            // mirrors, which has always had it. This is why
            // `--input-format json --slurp`/`--eval-all`/`--inplace`
            // (the only callers of `parse_input`, hence of this
            // function) silently accepted `{"a" 1, "b": 2}` when every
            // other route into the evaluator correctly raised.
            if !key_delimiter_ok::<V::Fields>(&field.key, &field.key_cursor, is_first)
                || !value_delimiter_ok::<V::Fields>(Some(&field.value), &field.value_cursor)
            {
                return Err(f.malformed_member_error());
            }
            map.insert(
                key,
                to_owned_canonicalizing_numbers_at_depth(&field.value, depth + 1)?,
            );
            f = rest;
            is_first = false;
        }
        // #1975: the other half of the same gap -- an unpaired trailing
        // child (#1194) that `resolve_display_key`'s per-field loop above
        // never sees at all, matching `eval_generic::to_owned_at_depth`'s
        // identical post-loop check.
        if f.ends_unpaired() {
            return Err(f.malformed_member_error());
        }
        OwnedValue::Object(map)
    } else if let Some(elements) = value.as_array() {
        let mut items = Vec::new();
        let mut elems = elements;
        let mut is_first = true;
        while let Some((elem_cursor, rest)) = elems.uncons_cursor() {
            // #1975: matches `eval_generic::to_owned_at_depth`'s array arm --
            // a missing/doubled `,` between elements was silently accepted
            // here too. `element_gap_ok` (`DocumentCursor`, #1597) rather
            // than re-deriving `text_position()`/`expected` inline, unlike
            // that sibling's own still-inline copy (pre-dates the
            // extraction).
            if !elem_cursor.element_gap_ok(is_first) {
                return Err(elem_cursor.malformed_delimiter_error());
            }
            items.push(to_owned_canonicalizing_numbers_at_depth(
                &elem_cursor.value(),
                depth + 1,
            )?);
            elems = rest;
            is_first = false;
        }
        OwnedValue::Array(items)
    } else if value.is_null() {
        OwnedValue::Null
    } else if let Some(b) = value.as_bool() {
        OwnedValue::Bool(b)
    } else if let Some(literal) = value.number_literal() {
        OwnedValue::from_number_literal_plain(&literal)
    } else if let Some(i) = value.as_i64() {
        // #999 review: unlike `as_f64` below (reached by a lenient
        // trailing-/leading-dot span like `5.`/`.5`, pinned by
        // `to_owned_canonicalizing_numbers_falls_back_to_as_f64_for_lenient_spans`),
        // this arm is unreachable for JSON specifically -- every lenient
        // span `is_valid_number_rejects_lenient_semi_index_spans`
        // (`json/validate.rs`) enumerates either succeeds via `as_f64` or
        // fails both (live-probed exhaustively: `1.2.3`, `1-2`, `1e`,
        // `1e+`, `-`, empty all materialize `null`, matching `to_owned`'s
        // own behavior for the same inputs). Kept for structural symmetry
        // with `to_owned_at_depth`, which this function otherwise mirrors
        // exactly -- a future `DocumentValue` implementor passed here
        // (there is none today; this function is private to this file)
        // could still reach it.
        OwnedValue::Int(i)
    } else if let Some(f) = value.as_f64() {
        OwnedValue::Float(f)
    } else if let Some(s) = value.as_str() {
        OwnedValue::String(s.into_owned())
    } else if let Some(reason) = value.string_decode_error() {
        // #1975: matches `eval_generic::to_owned_at_depth`'s identical arm
        // (#1247) -- `as_str` above answered `None`, but the value *is* a
        // string token whose bytes just don't decode. This function used to
        // fall all the way to the final `else` for this case, materializing
        // an undecodable string as `null` instead of raising.
        return Err(EvalError::decode_failure(reason));
    } else if value.is_error() {
        // #1975: matches `eval_generic::to_owned_at_depth`'s identical arm
        // (#1194) -- a structurally malformed value (`[xyz123]`, `[tru]`)
        // the semi-index accepted as a span but could not classify as any
        // JSON token. This function used to materialize it as `null`
        // instead of raising the semi-index's own, more specific message.
        return Err(EvalError::new(
            value
                .error_message()
                .unwrap_or("malformed value in document")
                .to_string(),
        ));
    } else {
        // Matches `to_owned_at_depth`'s own final `else` arm (#1098) --
        // a genuinely unknown type no format implements today, unaffected
        // by number canonicalization.
        OwnedValue::Null
    })
}

/// Parse input bytes according to the specified format.
fn parse_input(bytes: &[u8], format: InputFormat) -> Result<Vec<OwnedValue>> {
    match format {
        InputFormat::Json => {
            let index = JsonIndex::build(bytes);
            let cursor = index.root(bytes);
            let value = to_owned_canonicalizing_numbers(&cursor.value())
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(vec![value])
        }
        InputFormat::Yaml | InputFormat::Auto => {
            // Parse as YAML (Auto defaults to YAML when no extension hint)
            let index =
                YamlIndex::build(bytes).map_err(|e| anyhow::anyhow!("YAML parse error: {e}"))?;
            let root = index.root(bytes);

            match root.value() {
                YamlValue::Sequence(docs) => {
                    let mut values = Vec::new();
                    let mut rest = docs;
                    while let Some((doc_cursor, next)) = rest.uncons_cursor() {
                        values.push(yaml_to_owned_value(doc_cursor)?);
                        rest = next;
                    }
                    Ok(values)
                }
                // Documents are always wrapped in a virtual root sequence, so
                // this is defensive; `root` itself is this single document's
                // cursor either way.
                _ => Ok(vec![yaml_to_owned_value(root)?]),
            }
        }
    }
}

/// A single jq result value paired with its parallel [`CommentTree`] (issue #710).
type ResultWithComments = (OwnedValue, CommentTree);

/// Flags for [`evaluate_yaml_direct_filtered`], grouped into one struct
/// rather than four bool parameters (clippy's `fn_params_excessive_bools`/
/// `too_many_arguments`, #1398).
struct DirectEvalOptions {
    need_comments: bool,
    strip_style: bool,
    sort_keys: bool,
    /// Route JSON input through `YamlIndex::mark_json_sourced` instead of
    /// evaluating genuine YAML source — see the function's own doc comment.
    mark_json_sourced: bool,
}

/// Evaluate YAML *or JSON* input directly using the generic evaluator with
/// per-document processing.
///
/// This processes documents directly without intermediate `OwnedValue`
/// conversion, preserving position metadata for `line`/`column` builtins
/// (YAML) and, just as importantly, preserving duplicate mapping keys
/// through builtins like `length`/`to_entries` (#1398) — `OwnedValue::Object`
/// is `IndexMap`-backed and structurally can't represent them. JSON is a
/// YAML subset, so `mark_json_sourced` routes JSON input through the same
/// `YamlIndex`/cursor-native path the M2 streaming fast path already uses
/// for JSON (`index.mark_json_sourced()` there too), rather than the
/// eager-materializing `parse_input`/`evaluate_input` pair — this is what
/// stops yq mode's duplicate-key behavior from depending on which format the
/// same logical document arrived as.
///
/// Returns results grouped by document for proper multi-doc handling (with
/// `---` separators).
///
/// If `doc_filter` is Some((target_doc, global_offset)), only the document at global index
/// `target_doc` will be evaluated (where global index = global_offset + local_doc_index).
/// Returns the number of documents in this file for proper global index tracking.
fn evaluate_yaml_direct_filtered(
    bytes: &[u8],
    expr: &Expr,
    doc_filter: Option<(usize, usize)>,
    sink: &mut ErrorSink,
    opts: DirectEvalOptions,
) -> Result<(Vec<Vec<ResultWithComments>>, usize)> {
    let DirectEvalOptions {
        need_comments,
        strip_style,
        sort_keys,
        mark_json_sourced,
    } = opts;
    let mut index =
        YamlIndex::build(bytes).map_err(|e| anyhow::anyhow!("YAML parse error: {e}"))?;
    if mark_json_sourced {
        index.mark_json_sourced();
    }
    let root = index.root(bytes);

    // YAML documents are wrapped in a sequence at the root
    match root.value() {
        YamlValue::Sequence(mut docs) => {
            let mut doc_results = Vec::new();
            let mut local_idx = 0;
            while let Some((cursor, rest)) = docs.uncons_cursor() {
                // Check if this document should be evaluated
                let should_eval = match doc_filter {
                    Some((target_doc, global_offset)) => global_offset + local_idx == target_doc,
                    None => true,
                };

                if should_eval {
                    let results = evaluate_yaml_cursor(
                        cursor,
                        expr,
                        sink,
                        need_comments,
                        strip_style,
                        sort_keys,
                    )?;
                    // Only include documents that have results (select may filter them out)
                    if !results.is_empty() {
                        doc_results.push(results);
                    }
                }

                local_idx += 1;
                docs = rest;

                // halt/halt_error (#791) outranks evaluating any further
                // documents in this file — the caller checks `sink.halted()`
                // too, to stop further *files*.
                if sink.halted().is_some() {
                    break;
                }
            }
            Ok((doc_results, local_idx))
        }
        _ => {
            // Defensive fallback only, same as the inplace fast path's
            // identical `_ =>` arm below: `root.value()` always reports the
            // virtual document sequence (single documents included), so
            // this arm is unreachable through any real input today.
            let should_eval = match doc_filter {
                Some((target_doc, global_offset)) => global_offset == target_doc,
                None => true,
            };

            if should_eval {
                if let Some(content_cursor) = root.first_child() {
                    let results = evaluate_yaml_cursor(
                        content_cursor,
                        expr,
                        sink,
                        need_comments,
                        strip_style,
                        sort_keys,
                    )?;
                    Ok((vec![results], 1))
                } else {
                    // Empty document
                    Ok((vec![vec![]], 1))
                }
            } else {
                Ok((vec![], 1))
            }
        }
    }
}

/// Evaluate a jq expression on an OwnedValue by converting to JSON and back.
///
/// Variables (`--arg`/`--argjson`, `$ARGS`) are substituted into `expr` up
/// front in `run_yq`, so this function needs no evaluation context (#284).
///
/// This is `yq_runner.rs`'s own function (`jq_runner.rs` has an unrelated,
/// separate `evaluate_input` for jq mode). It always *evaluates* with
/// `YqSemantics` below, but the round-trip's own formatter deliberately
/// stays `to_json_for_reindex::<JqSemantics>()`, not `<YqSemantics>` or
/// `to_json_yq()` (#1051):
///
/// - Its `NumberLiteral` echo is unconditional either way (#1051's actual
///   fix), so a document-sourced literal like `1e2` survives this round trip
///   with its exact spelling untouched regardless of which `S` is passed —
///   the DOM fallback taken by `--slurp` and by `--inplace` for any
///   expression `can_use_m2_streaming` doesn't allow-list (`tostring` is not
///   on that list) used to reformat it via jq's `format_number_jq_compat`
///   before the evaluator ever ran, silently rewriting fields the query
///   never touched on an in-place write.
/// - Its NaN/Infinity handling is also unconditional (code review on this
///   fix's first draft caught this live — `-i '.b = (.a | tostring)'` on
///   `a: .inf` silently rewrote it to `a: null` through `to_json_yq()`'s
///   RFC-8259 "null" substitution, wrong for this purely-internal round
///   trip), matching `eval_owned_input`'s identical reindex bridge for
///   `reduce`/`foreach` (#561, #472).
/// - Only the *fallback* arm — a plain, already-literal-less `Float` — is
///   `S`-gated, and `YqSemantics` there is actively wrong for this call
///   site specifically: `parse_input`'s `--input-format json` path already
///   collapses every number straight to a plain `Float`/`Int` via
///   `to_owned_canonicalizing_numbers` (#978, matching real yq's "a
///   JSON-sourced number never keeps its own spelling" convention) *before*
///   the value ever reaches here — forcing `YqSemantics`'s decimal point back onto
///   that already-canonicalized value reintroduced exactly the bug #978
///   fixed (`--slurp --input-format json '.'` on `{"a":1e2}` regressed from
///   `[{"a":100}]` to `[{"a":100.0}]`, caught by CI). `JqSemantics`'s bare
///   fallback (no forced point) is correct for both the JSON-canonicalized
///   case and the untouched-overflow-scalar case (the latter is already
///   lossy through this whole-document round trip regardless of the point —
///   confirmed live, real yq keeps an untouched i64-overflow scalar
///   byte-for-byte via `-i`, e.g. `99999999999999999999` verbatim, which
///   this reindex-through-`f64` architecture cannot match either way).
fn evaluate_input(
    input: &OwnedValue,
    expr: &jq::Expr,
    sink: &mut ErrorSink,
) -> Result<Vec<OwnedValue>> {
    // Convert OwnedValue to JSON bytes for indexing
    let json_str = input.to_json_for_reindex::<jq::JqSemantics>();
    let json_bytes = json_str.as_bytes();

    // Build index and evaluate
    let index = JsonIndex::build(json_bytes);
    let cursor = index.root(json_bytes);

    let result = jq::eval::<Vec<u64>, YqSemantics>(expr, cursor);
    Ok(query_result_to_owned_values(result, sink))
}

/// Convert a `QueryResult` into `Vec<OwnedValue>`, reporting an uncaught
/// error/break through `sink` (evaluation continues past one, per yq's
/// convention -- see `ErrorSink`'s own doc comment) rather than failing the
/// whole run. Factored out of [`evaluate_input`] so `--eval-all`'s
/// `eval_owned_with_file_index` call site (#715) shares the exact same
/// conversion/error-reporting policy instead of a second, divergence-prone
/// copy.
fn query_result_to_owned_values(
    result: QueryResult<'_, Vec<u64>>,
    sink: &mut ErrorSink,
) -> Vec<OwnedValue> {
    // A decode failure while materializing is an uncaught error like any
    // other (#1247): report it and yield nothing, exactly as the
    // `QueryResult::Error` arm below does.
    //
    // Both callers ([`evaluate_input`] above, `eval_owned_with_file_index`
    // below) only ever produce a `result` rooted in an already-decoded
    // `OwnedValue` re-serialized to JSON and reindexed -- `eval_owned_input`
    // (this function's non-path-context caller) explicitly converts every
    // `One`/`Many` it sees into `Owned`/`ManyOwned` before returning, and the
    // path-tracking branch builds its own `Owned`/`ManyOwned` results
    // throughout since a path is always a concrete `Vec<OwnedValue>`, never
    // a lazy cursor. So a genuine decode failure in the `One`/`OneCursor`/
    // `Many` arms below is defense-in-depth, not a reachable path today --
    // same argument `eval_generic.rs`'s textually-similar bridge relies on
    // (search "defense-in-depth" there). Kept as `Result` rather than
    // `.unwrap()` so a real failure, if this invariant is ever violated,
    // surfaces as a normal `EvalError` instead of a panic.
    match result {
        QueryResult::One(v) => {
            let Some(v) = sink.materialize(DiagStyle::Yq, generic_to_owned(&v), &no_location())
            else {
                return Vec::new();
            };
            vec![v]
        }
        QueryResult::OneCursor(c) => {
            let Some(v) =
                sink.materialize(DiagStyle::Yq, generic_to_owned(&c.value()), &no_location())
            else {
                return Vec::new();
            };
            vec![v]
        }
        QueryResult::Many(vs) => {
            let Some(vs) = sink.materialize(
                DiagStyle::Yq,
                vs.iter()
                    .map(generic_to_owned)
                    .collect::<core::result::Result<Vec<_>, _>>(),
                &no_location(),
            ) else {
                return Vec::new();
            };
            vs
        }
        QueryResult::None => vec![],
        QueryResult::Error(e) => {
            sink.report(DiagStyle::Yq, &e, &no_location());
            vec![]
        }
        QueryResult::Owned(v) => vec![v],
        QueryResult::ManyOwned(vs) => vs,
        QueryResult::Break(label) => {
            sink.report_break(DiagStyle::Yq, &label, &no_location());
            vec![]
        }
        // `halt`/`halt_error` (#791): not a diagnostic, so no `sink.report*`
        // call — `request_halt` records the exit code for the caller's loop
        // to short-circuit on, without touching `hit`/`report_count`.
        QueryResult::Halt(code) => {
            sink.request_halt(code);
            vec![]
        }
        // The outputs already produced no longer vanish behind the failure
        // (#400, #494).
        QueryResult::Partial(vs, jq::Control::Error(e)) => {
            sink.report(DiagStyle::Yq, &e, &no_location());
            vs
        }
        QueryResult::Partial(vs, jq::Control::Break(label)) => {
            sink.report_break(DiagStyle::Yq, &label, &no_location());
            vs
        }
        QueryResult::Partial(vs, jq::Control::Halt(code)) => {
            sink.request_halt(code);
            vs
        }
    }
}

/// Whether `expr` is itself an assignment-family write: `.path = value`,
/// `|=`, a compound assign, `//=`, or `del(...)`. Unwraps `Paren`/`Optional`
/// so `(.a = 1)?` still counts, and recurses into `Pipe` so a chain counts
/// as soon as *any* stage writes.
///
/// Split out from [`is_alias_sensitive_assign`] so a pipe made entirely of
/// pass-through stages (`select(true) | debug`, no write at all) doesn't
/// pay for alias-sync snapshotting -- only a pipe that both preserves shape
/// *and* actually writes somewhere needs the pristine-vs-result diff.
fn contains_assign(expr: &Expr) -> bool {
    match expr {
        Expr::Assign { .. }
        | Expr::Update { .. }
        | Expr::CompoundAssign { .. }
        | Expr::AlternativeAssign { .. }
        | Expr::Builtin(Builtin::Del(_)) => true,
        Expr::Paren(inner) | Expr::Optional(inner) => contains_assign(inner),
        Expr::Pipe(stages) => stages.iter().any(contains_assign),
        _ => false,
    }
}

/// Whether `expr`'s top-level shape is "rewrite the document at specific
/// paths, leaving everything else identical" -- the class of expression for
/// which comparing a path's value before and after the write is meaningful.
/// Unwraps `Paren`/`Optional` so `(.a = 1)?` still matches, and recurses into
/// `Pipe` so a chain matches when every stage does, whether the stage is a
/// write (`.a = 1 | .b = 2`) or one of a small allow-list of pass-through
/// stages that provably either emit the input document completely unchanged
/// or emit nothing at all: `.` (`Identity`), `select(...)`, `empty`,
/// `debug`/`debug(msg)`. Mixing one into an assignment pipe (`.a = 1 |
/// select(.a > 0)`, the guard-style `yq -i` idiom from #764) still leaves
/// "the same path means the same thing" true for every document that comes
/// out the other end, since none of these four ever rewrite or reshape the
/// value they pass through -- unlike `map`, `select`'s own predicate or
/// `debug`'s own message expression never appear in the pipeline's output,
/// only their pass/fail or side-effect result does, so `contains_assign`
/// deliberately does not recurse into either.
///
/// Used to gate the alias-sync post-process (#711): outside this class (a
/// bare `map`, `.a, .b`, ...) the result document doesn't necessarily share
/// the input's shape at all, so diffing "the same path" in both would be
/// meaningless at best and could clobber it at worst. A pipe with a stage
/// outside both the write list and this pass-through allow-list is
/// conservatively excluded for the same reason -- verifying more stages
/// preserve paths is left for a future extension, not assumed here.
fn is_alias_sensitive_assign(expr: &Expr) -> bool {
    fn is_shape_preserving(expr: &Expr) -> bool {
        match expr {
            Expr::Assign { .. }
            | Expr::Update { .. }
            | Expr::CompoundAssign { .. }
            | Expr::AlternativeAssign { .. }
            | Expr::Identity
            | Expr::Builtin(
                Builtin::Del(_)
                | Builtin::Select(_)
                | Builtin::Empty
                | Builtin::Debug
                | Builtin::DebugMsg(_),
            ) => true,
            Expr::Paren(inner) | Expr::Optional(inner) => is_shape_preserving(inner),
            Expr::Pipe(stages) => stages.iter().all(is_shape_preserving),
            _ => false,
        }
    }

    is_shape_preserving(expr) && contains_assign(expr)
}

/// Walk `cursor`'s document collecting, for every anchor with at least one
/// alias, its definition path and the path of every alias that resolves to
/// it (#711). Paths are plain `String`/`i64` key sequences -- the same shape
/// `sync_aliased_paths` (and jq's own `getpath`/`setpath`) use -- so they
/// line up with coordinates in the `OwnedValue` tree `to_owned` builds from
/// this same document.
///
/// Mirrors `yaml_to_owned_value`'s recursion, including its treatment of
/// merge keys (`<<: *base`): `Mapping`'s `fields` iterator already resolves
/// them transparently, so this walk does too, unchanged. Fixing merge-key/
/// anchor interaction is issue #712's territory, not this one.
fn collect_alias_groups<W: AsRef<[u64]> + Clone>(
    cursor: YamlCursor<'_, W>,
) -> Vec<(Vec<OwnedValue>, Vec<Vec<OwnedValue>>)> {
    let mut defs: IndexMap<String, Vec<OwnedValue>> = IndexMap::new();
    let mut aliases: IndexMap<String, Vec<Vec<OwnedValue>>> = IndexMap::new();
    let mut path = Vec::new();
    walk_alias_groups(cursor, &mut path, &mut defs, &mut aliases);
    defs.into_iter()
        .filter_map(|(name, def_path)| Some((def_path, aliases.swap_remove(&name)?)))
        .collect()
}

fn walk_alias_groups<W: AsRef<[u64]> + Clone>(
    cursor: YamlCursor<'_, W>,
    path: &mut Vec<OwnedValue>,
    defs: &mut IndexMap<String, Vec<OwnedValue>>,
    aliases: &mut IndexMap<String, Vec<Vec<OwnedValue>>>,
) {
    // `anchor()` returns `None` for alias nodes, so an alias is never
    // mistaken for its own definition here.
    if let Some(name) = cursor.anchor() {
        defs.insert(name.to_string(), path.clone());
    }
    match cursor.value() {
        YamlValue::Mapping(fields) => {
            for field in fields {
                path.push(OwnedValue::String(field.key().key_string().into_owned()));
                walk_alias_groups(field.value_cursor(), path, defs, aliases);
                path.pop();
            }
        }
        YamlValue::Sequence(elements) => {
            let mut idx = 0i64;
            let mut rest = elements;
            // `uncons_resolved_cursor`, not `uncons_cursor`: a bare `-`
            // item's own anchor (if any) sits on the deferred value's line,
            // not the wrapper's, and this function's own `cursor.anchor()`
            // check above doesn't resolve through the wrapper itself (see
            // `YamlCursor::anchor`'s doc comment) — an unresolved cursor
            // here would silently miss the anchor and break alias-sync
            // bookkeeping for a bare-dash-deferred anchored value (#835).
            while let Some((elem_cursor, next_rest)) = rest.uncons_resolved_cursor() {
                path.push(OwnedValue::Int(idx));
                walk_alias_groups(elem_cursor, path, defs, aliases);
                path.pop();
                idx += 1;
                rest = next_rest;
            }
        }
        YamlValue::Alias { anchor_name, .. } => {
            aliases
                .entry(anchor_name.to_string())
                .or_default()
                .push(path.clone());
        }
        _ => {}
    }
}

/// Reconcile a pristine (pre-write) presentation tree against a post-write
/// value (issue #739, ADR-0017's mechanism 1, applied to path-mutation
/// queries: `=`, `|=`, `+=`, `del()`, ...).
///
/// Verified against the pinned real `yq` binary: a node keeps its own
/// comment and style across a write as long as the write leaves it the
/// same *kind* (`Object`/`Array`/scalar) — regardless of whether its
/// *value* actually changed. `.b = 2` on `b: 1 # keep` still prints
/// `b: 2 # keep`; `.a = "new"` on `a: 'old'` still prints `a: 'new'`
/// (single-quote style survives even though the string content didn't).
/// Real `yq`'s in-place node-mutation model (go-yaml) explains why: `Set`
/// overwrites the existing `Node.Value` but never touches that node's own
/// `Style`/`LineComment`. Only a kind change (scalar becomes a container or
/// vice versa) discards them, since there's no such node to keep - matching
/// `.a = {"x": 1}` on `a: 'old'` dropping the quote style entirely (real
/// `yq`: `a:\n  x: 1`).
///
/// Walks `pristine_value`/`result_value` in lockstep rather than computing
/// which path(s) an expression's AST touches (`resolve_dynamic_indexes` in
/// `eval.rs`) and invalidating just those - this is exact for every
/// mutation shape uniformly (a single assign, a chained pipe of several, a
/// computed-key write, a `del()`) with no per-expression-shape logic, and
/// the YAML documents this rewrites are config-file-sized, not
/// data-file-sized.
///
/// **Known gap, filed as #870**: this function matches a child to
/// its pristine counterpart purely by key/index, so any write that
/// reshuffles an `Array`'s or `Object`'s *positions* - not just a
/// wholesale replacement like `.a = [4, 5, 6]` on `a: ['x', 'y', 'z']`,
/// but an everyday `.arr = ["new"] + .arr` prepend or a `del()` that
/// shifts later indices down - misattributes old elements' style/comments
/// to whichever new element now sits at the same key/index, instead of
/// giving every element under the write fresh (empty) metadata the way
/// real `yq`'s node-mutation model does (confirmed against the pinned
/// binary: prepending to `arr:\n  - "x"\n  - y` should drop the new
/// element's style entirely and leave `"x"`'s own quotes exactly where
/// they were; this function instead lets the new element inherit `"x"`'s
/// old quote style and leaves `"x"` unquoted). This function has no way
/// to tell "recursing into an untouched sibling subtree" apart from
/// "recursing into a reshuffled subtree that happens to share
/// positions/keys with the old one" - both look identical from a pure
/// before/after value diff; only a path-based approach (the
/// `resolve_dynamic_indexes` alternative above) could distinguish them.
/// Purely cosmetic (no data loss, no incorrect *values*, still strictly
/// better than every write losing all style/comments unconditionally,
/// which was the pre-#739 baseline).
fn reconcile_presentation(
    pristine_value: &OwnedValue,
    pristine_tree: &CommentTree,
    result_value: &OwnedValue,
) -> CommentTree {
    reconcile_presentation_at_depth(pristine_value, pristine_tree, result_value, 0)
}

/// Panics past `succinctly::jq::MAX_VALUE_TREE_DEPTH` levels of nesting
/// (#1005) — see that constant's own doc comment for why. `result_value` is
/// the evaluated filter output, which can be constructed via `reduce`/
/// `foreach`/etc. with no adversarial document involved on either side.
fn reconcile_presentation_at_depth(
    pristine_value: &OwnedValue,
    pristine_tree: &CommentTree,
    result_value: &OwnedValue,
    depth: usize,
) -> CommentTree {
    assert_value_tree_depth(depth);
    match (pristine_value, result_value) {
        (OwnedValue::Object(p_fields), OwnedValue::Object(r_fields)) => {
            let own_meta = pristine_tree.meta().clone();
            let mut fields = IndexMap::new();
            let mut key_comments = IndexMap::new();
            for (k, r_v) in r_fields {
                let child = match p_fields.get(k) {
                    Some(p_v) => {
                        reconcile_presentation_at_depth(p_v, pristine_tree.field(k), r_v, depth + 1)
                    }
                    None => CommentTree::empty(),
                };
                fields.insert(k.clone(), child);
                // A key's own trailing comment (#765) belongs to the key's
                // line, not its value, so it survives regardless of
                // whether the value under it changed - only a removed key
                // (not iterated here, since this loop is over `r_fields`)
                // drops it. The "deferred value materialized as nothing"
                // flag, though, is re-derived from `r_v`, not copied from
                // pristine: a write that gives the key a real value must
                // not carry over a stale "absent" flag, or
                // `key_comment_if_value_absent`'s own consumer
                // (`emit_yaml_value`'s block-mapping arm) renders only the
                // key and comment, silently dropping the write's value
                // entirely (found in review).
                if let CommentTree::Object(_, _, pristine_key_comments) = pristine_tree {
                    if let Some((kc, _)) = pristine_key_comments.get(k) {
                        let value_absent = matches!(r_v, OwnedValue::Null);
                        key_comments.insert(k.clone(), (kc.clone(), value_absent));
                    }
                }
            }
            CommentTree::Object(own_meta, fields, key_comments)
        }
        (OwnedValue::Array(p_items), OwnedValue::Array(r_items)) => {
            let own_meta = pristine_tree.meta().clone();
            let items = r_items
                .iter()
                .enumerate()
                .map(|(i, r_v)| match p_items.get(i) {
                    Some(p_v) => reconcile_presentation_at_depth(
                        p_v,
                        pristine_tree.at_index(i),
                        r_v,
                        depth + 1,
                    ),
                    None => CommentTree::empty(),
                })
                .collect();
            CommentTree::Array(own_meta, items)
        }
        // A kind change (container <-> scalar, or Object <-> Array) is a
        // fresh node with no presentation memory of its own.
        (OwnedValue::Object(_) | OwnedValue::Array(_), _)
        | (_, OwnedValue::Object(_) | OwnedValue::Array(_)) => CommentTree::empty(),
        // Both scalars, any variant/value: same node, only its value
        // changed - its own comment, style and anchor mark survive. Real
        // `yq` keeps `&x` across `.a = 99` for the same reason it keeps the
        // comment: the write overwrites the node's value, not its identity
        // (#763). Whether a surviving `*x` mark is still *emittable* is a
        // separate question, settled afterwards by
        // [`enforce_anchor_soundness`] once the whole result is known.
        _ => CommentTree::Leaf(pristine_tree.meta().clone()),
    }
}

/// Evaluate `split_expr` against `result` with `$index` bound to
/// `output_index`, expect exactly one string back, and write `result`
/// (serialized through `output_config`, color forced off, matching
/// `--inplace`) to that path.
///
/// Non-string, empty, or multi-value split-expression results are reported
/// through `sink` — the same "continue, exit 1 at the end" convention used
/// for other uncaught evaluation errors in this file — rather than aborting
/// the run outright.
fn write_split_result(
    result: &OwnedValue,
    comments: &CommentTree,
    split_expr: &Expr,
    output_index: i64,
    output_config: &OutputConfig,
    written_files: &mut std::collections::HashSet<String>,
    sink: &mut ErrorSink,
) -> Result<()> {
    let index_val = OwnedValue::Int(output_index);
    let per_result_expr = jq::substitute_vars(split_expr, [("index", &index_val)]);

    // Snapshotted before evaluating the split-filename expression, so the
    // check below can tell "this call's own expression halted" from "the
    // *main* expression already halted and `result` is an output-bearing
    // `Partial` prefix that still owes its file" (#791): `sink.halted()` is
    // sticky for the whole run (`request_halt`'s "first halt wins"), so
    // without this snapshot every `write_split_result` call after a
    // mid-stream halt would misread the already-set flag as its own and
    // skip writing a result that must still be split out.
    let halted_before = sink.halted().is_some();

    let reports_before = sink.report_count();
    let filename_results = evaluate_input(result, &per_result_expr, sink)?;

    // halt/halt_error (#791) inside *this* split expression: not a
    // diagnostic (no `report_count` bump), so an empty result must be
    // checked before the `[]` arms below, or it would be misreported as
    // "produced no output". But a halt that already produced a value (e.g.
    // `"out\($index).yml", halt`) must still fall through to the match below
    // so that value gets written -- only bail early when the halt left
    // nothing behind, or a legitimately-produced filename is silently lost.
    if !halted_before && sink.halted().is_some() && filename_results.is_empty() {
        return Ok(());
    }

    let filename = match filename_results.as_slice() {
        [OwnedValue::String(s)] => s.clone(),
        // `evaluate_input` already reported the underlying error (e.g. an
        // undefined variable, or an explicit `error(...)`) via `sink`.
        // `report_count()` (not `hit()`, which is sticky for the whole run)
        // is what lets this tell "this call just reported" from "some
        // earlier result already tripped the sink" -- otherwise every
        // result after the first real error in the run double-reports here
        // (#715 follow-up).
        [] if sink.report_count() > reports_before => return Ok(()),
        [] => {
            sink.report(
                DiagStyle::Yq,
                &EvalError::new(format!(
                    "--split-exp expression produced no output for result #{output_index}"
                )),
                &no_location(),
            );
            return Ok(());
        }
        [other] => {
            sink.report(
                DiagStyle::Yq,
                &EvalError::new(format!(
                    "--split-exp expression must evaluate to a string, got {} for result #{output_index}",
                    other.type_name()
                )),
                &no_location(),
            );
            return Ok(());
        }
        many => {
            sink.report(
                DiagStyle::Yq,
                &EvalError::new(format!(
                    "--split-exp expression must evaluate to exactly one string, got {} results for result #{output_index}",
                    many.len()
                )),
                &no_location(),
            );
            return Ok(());
        }
    };

    if !written_files.insert(filename.clone()) {
        eprintln!("Warning: --split-exp path '{filename}' written more than once; overwriting");
    }

    let mut buf = Vec::new();
    let mut no_color_config = output_config.clone();
    no_color_config.use_color = false;
    output_value(&mut buf, result, comments, &no_color_config, None)?;

    std::fs::write(&filename, &buf)
        .with_context(|| format!("failed to write --split-exp output file: {filename}"))
}

/// One step of a path into a paired `OwnedValue`/[`CommentTree`] tree, used
/// by [`enforce_anchor_soundness`] to revisit a node it decided to strip.
///
/// Borrows its key straight out of the `OwnedValue` being scanned rather
/// than owning it: the scan pushes a step for every object field it walks,
/// so an owned `String` here would be one allocation per field on every DOM
/// emit. The borrow is sound because the value tree and the `CommentTree`
/// the second pass mutates are two distinct objects (see
/// [`enforce_anchor_soundness`]'s two parameters).
#[derive(Clone, Copy)]
enum TreeStep<'a> {
    Key(&'a str),
    Index(usize),
}

/// Drop every `*alias` mark this document cannot actually resolve, so the
/// YAML written out always re-reads as the same values succinctly would
/// have printed without any anchor syntax at all (#763).
///
/// A mark survives only when all three hold, checked in the emitter's own
/// traversal order:
///
/// 1. some node declares that anchor name in the output;
/// 2. it is emitted **before** this one — a `*x` above its `&x` is a
///    forward reference, which YAML forbids;
/// 3. the value there equals the value here — otherwise writing `*x` would
///    silently replace this node's real value with the anchor's.
///
/// This is deliberately a *divergence* from real `yq`, which fails all
/// three in practice: `yq 'del(.a)'` on `a: &x 1\nb: *x` prints `b: *x`
/// with no anchor left anywhere, and `yq` then rejects its own output with
/// `unknown anchor 'x' referenced` (verified against the pinned binary).
/// Rule 3 is also what keeps the gap in succinctly's own alias *value*
/// model safe rather than wrong: a write *through* an alias (`.b.p = 9`)
/// updates only `.b`, where real `yq` mutates the shared node, so the two
/// sides no longer agree and the mark is dropped — printing `b: {p: 9}`
/// (the value succinctly computed) instead of `b: *x` (which would discard
/// the write entirely).
///
/// Anchor *declarations* are never dropped: an unreferenced `&x` is valid
/// YAML and real `yq` keeps it (`yq 'del(.b)'` still prints `a: &x 1`).
fn enforce_anchor_soundness(value: &OwnedValue, comments: &mut CommentTree, sort_keys: bool) {
    // Scoped so the scan's immutable borrow of `*comments` (and the anchor
    // names `declared` keys on, which live in it) has certainly ended
    // before the second pass takes a mutable one. The flagged paths borrow
    // only from `value`, which the second pass never touches.
    let unresolvable: Vec<Vec<TreeStep<'_>>> = {
        let mut declared: IndexMap<&str, &OwnedValue> = IndexMap::new();
        let mut unresolvable = Vec::new();
        let mut path = Vec::new();
        scan_anchor_soundness(
            value,
            comments,
            sort_keys,
            &mut declared,
            &mut unresolvable,
            &mut path,
            0,
        );
        unresolvable
    };
    for steps in &unresolvable {
        if let Some(node) = comment_tree_at_path_mut(comments, steps) {
            node.meta_mut().anchor = None;
        }
    }
}

/// Pre-order half of [`enforce_anchor_soundness`]: record each declaration
/// as it is reached and flag every alias that fails the three rules.
///
/// Reads the tree immutably so the surviving declarations can borrow
/// straight out of `value` rather than cloning whole anchored subtrees; the
/// flagged paths are applied in a second pass. Panics past
/// `MAX_VALUE_TREE_DEPTH`, like every other walker over this pair.
fn scan_anchor_soundness<'v, 'c>(
    value: &'v OwnedValue,
    comments: &'c CommentTree,
    sort_keys: bool,
    declared: &mut IndexMap<&'c str, &'v OwnedValue>,
    unresolvable: &mut Vec<Vec<TreeStep<'v>>>,
    path: &mut Vec<TreeStep<'v>>,
    depth: usize,
) {
    assert_value_tree_depth(depth);
    match comments.anchor_mark() {
        // A repeated name shadows the earlier one for every *later* alias,
        // matching YAML's own "most recent preceding anchor wins".
        Some(AnchorMark::Declares(name)) => {
            declared.insert(name.as_str(), value);
        }
        // No such declaration yet (missing entirely, or emitted later than
        // this alias), or one whose value has since diverged from this one.
        // Spelled as a negated `matches!` rather than clippy's suggested
        // `is_none_or`, which is stable only since 1.82 and this crate's
        // MSRV is 1.73.
        Some(AnchorMark::Aliases(name)) if !matches!(declared.get(name.as_str()), Some(d) if *d == value) =>
        {
            unresolvable.push(path.clone());
        }
        Some(AnchorMark::Aliases(_)) => {}
        None => {}
    }
    match value {
        OwnedValue::Object(fields) => {
            // Mirror the block-mapping arm's own ordering, so "emitted
            // before" here means the same thing it will at write time. The
            // flow arm doesn't sort, but a flow container's keys can only
            // have come from source order, where a declaration always
            // precedes its aliases (a forward reference is rejected at
            // index build) — so walking sorted there can only drop a mark
            // that would have been fine, never keep one that wouldn't.
            let mut keys: Vec<&String> = fields.keys().collect();
            if sort_keys {
                keys.sort();
            }
            for k in keys {
                let Some(v) = fields.get(k) else { continue };
                path.push(TreeStep::Key(k.as_str()));
                scan_anchor_soundness(
                    v,
                    comments.field(k),
                    sort_keys,
                    declared,
                    unresolvable,
                    path,
                    depth + 1,
                );
                path.pop();
            }
        }
        OwnedValue::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                path.push(TreeStep::Index(i));
                scan_anchor_soundness(
                    v,
                    comments.at_index(i),
                    sort_keys,
                    declared,
                    unresolvable,
                    path,
                    depth + 1,
                );
                path.pop();
            }
        }
        _ => {}
    }
}

/// Mutable counterpart of [`CommentTree::field`]/[`CommentTree::at_index`],
/// following a whole path. `None` if any step is missing — which cannot
/// happen for a path [`scan_anchor_soundness`] just walked, but the
/// accessors have no infallible form.
fn comment_tree_at_path_mut<'t>(
    tree: &'t mut CommentTree,
    steps: &[TreeStep<'_>],
) -> Option<&'t mut CommentTree> {
    let mut node = tree;
    for step in steps {
        node = match (node, step) {
            (CommentTree::Object(_, fields, _), TreeStep::Key(k)) => fields.get_mut(*k)?,
            (CommentTree::Array(_, items), TreeStep::Index(i)) => items.get_mut(*i)?,
            _ => return None,
        };
    }
    Some(node)
}

/// Evaluate a jq expression directly on a YAML cursor.
///
/// This uses the generic evaluator to preserve position metadata (line/column).
/// Each result carries its parallel [`CommentTree`] (issue #710/#739) —
/// real for `OneCursor`/`ManyCursor` (still-live cursor); for every other
/// result (an already-materialized/computed value with no cursor of its
/// own) it's [`reconcile_presentation`]'s output against the pristine
/// document when `expr` is a shape-preserving write
/// ([`is_alias_sensitive_assign`]) and the caller wants comments at all
/// (`need_comments`), or [`CommentTree::empty`] otherwise — see
/// [`CommentTree`]'s own doc comment for why a cursor-less value can't
/// generally carry metadata.
fn evaluate_yaml_cursor<W: AsRef<[u64]> + Clone>(
    cursor: YamlCursor<'_, W>,
    expr: &Expr,
    sink: &mut ErrorSink,
    need_comments: bool,
    strip_style: bool,
    sort_keys: bool,
) -> Result<Vec<ResultWithComments>> {
    // Snapshot alias-sync context from the pristine document *before*
    // evaluation, only when it could possibly matter (#711): an
    // assignment-family expression against a document that actually has
    // aliases. Everything else (JSON, plain reads, alias-free YAML) pays
    // nothing beyond this one bool check.
    // A decode failure anywhere in this function is an uncaught error like
    // any other (#1247): report it and yield no documents, exactly as the
    // `GenericResult::Error` arm below does.
    let has_aliases = cursor.index().has_aliases();
    let alias_sync_ctx = match (is_alias_sensitive_assign(expr) && has_aliases)
        .then(|| generic_to_owned(&cursor.value()))
    {
        Some(pristine) => {
            let Some(pristine) = sink.materialize(DiagStyle::Yq, pristine, &no_location()) else {
                return Ok(Vec::new());
            };
            Some((pristine, collect_alias_groups(cursor)))
        }
        None => None,
    };

    // Snapshot the pristine presentation tree *before* evaluation too
    // (#739, ADR-0017): same shape-preserving-write gate as `alias_sync_ctx`
    // above (no aliases required here - a write-family expression against
    // any YAML document can lose style/comments, not just an aliased one),
    // and only when the caller can use the result at all (`need_comments`).
    // `no_comments` below reconciles this against each result document once
    // evaluation finishes.
    let presentation_sync_ctx = match (need_comments && is_alias_sensitive_assign(expr))
        .then(|| to_owned_with_comments(&cursor.value(), Some(&cursor)))
    {
        Some(snapshot) => {
            let Some(snapshot) = sink.materialize(DiagStyle::Yq, snapshot, &no_location()) else {
                return Ok(Vec::new());
            };
            Some(snapshot)
        }
        None => None,
    };

    let result = eval_with_cursor_using::<YqSemantics, _>(expr, cursor);
    // A value with no live cursor of its own (an assignment/`del()`
    // result, a computed value, ...) has no comment/style to read directly
    // - but if it came from a shape-preserving write, `presentation_sync_ctx`
    // lets it recover whatever the write didn't touch instead of falling
    // back to genuinely empty (#739).
    let no_comments = |v: OwnedValue| {
        let comments = presentation_sync_ctx
            .as_ref()
            .map_or_else(CommentTree::empty, |(pristine, tree)| {
                reconcile_presentation(pristine, tree, &v)
            });
        (v, comments)
    };
    // `to_owned_with_comments` builds a full parallel `IndexMap`/`Vec` tree
    // alongside the `OwnedValue` one, just to carry comment text - wasted
    // work when the caller can't use it (`-o json`'s output never reads
    // `CommentTree` at all; see `output_value`'s JSON branch) (#710).
    let owned_with_comments = |c: &YamlCursor<'_, W>| {
        if need_comments {
            to_owned_with_comments(&c.value(), Some(c))
        } else {
            generic_to_owned(&c.value()).map(&no_comments)
        }
    };

    // Convert GenericResult to Vec<ResultWithComments>
    //
    // `One`/`Many` (as opposed to `OneCursor`/`ManyCursor`) only ever arise
    // from `eval_generic.rs`'s cursor-loss cascade, which requires an
    // already-cursor-less value to begin with (e.g. via the cursor-less
    // `eval()` entry point `jq`'s DOM path uses) — this function always
    // starts `eval_with_cursor_using` from a real `cursor`, so these two
    // arms are defensive/unreachable here today, kept for exhaustiveness
    // over the shared `GenericResult` enum.
    let mut docs = match result {
        GenericResult::One(v) => {
            let Some(v) = sink.materialize(DiagStyle::Yq, generic_to_owned(&v), &no_location())
            else {
                return Ok(Vec::new());
            };
            Ok(vec![no_comments(v)])
        }
        GenericResult::OneCursor(c) => {
            let Some(v) = sink.materialize(DiagStyle::Yq, owned_with_comments(&c), &no_location())
            else {
                return Ok(Vec::new());
            };
            Ok(vec![v])
        }
        GenericResult::Many(vs) => {
            let Some(vs) = sink.materialize(
                DiagStyle::Yq,
                vs.iter()
                    .map(generic_to_owned)
                    .collect::<core::result::Result<Vec<_>, _>>(),
                &no_location(),
            ) else {
                return Ok(Vec::new());
            };
            Ok(vs.into_iter().map(&no_comments).collect())
        }
        GenericResult::ManyCursor(cs) => {
            let Some(vs) = sink.materialize(
                DiagStyle::Yq,
                cs.iter()
                    .map(owned_with_comments)
                    .collect::<core::result::Result<Vec<_>, _>>(),
                &no_location(),
            ) else {
                return Ok(Vec::new());
            };
            Ok(vs)
        }
        // This is the DOM/slow path (`evaluate_yaml_direct_filtered`'s
        // fallback), reached only when `can_use_m2_streaming` rejects the
        // expression or a flag (`--sort-keys`, color, `--tab`, `--slurp`,
        // `--null-input`, named vars, ...) forces DOM output for every query
        // shape, not just `keys_unsorted`. `syq 'keys_unsorted'` under
        // default flags takes the M2 fast path instead, which streams each
        // key from `fields` without materializing (#685); this arm stays a
        // plain materializing fallback since the DOM path materializes
        // everything else here too. Sort iff `sorted` (#683) -- though in
        // practice `sorted` is always `false` here: `run_yq` always parses
        // in `ParserMode::Yq`, where the `keys` keyword itself resolves to
        // `Builtin::KeysUnsorted` (matching real yq's document-order
        // semantics, see `parser.rs`'s `keys`/`keys_unsorted` handling), so
        // `Builtin::Keys` can never reach this arm through the `yq` CLI.
        // Handled anyway for exhaustiveness and because the generic
        // evaluator is shared with `jq` (#140's `Pipe` dispatch is generic
        // over `V: DocumentValue`).
        GenericResult::LazyKeys {
            fields,
            sorted,
            collapse,
        } => {
            let Some(mut keys) = sink.materialize(
                DiagStyle::Yq,
                effective_keys(&fields, collapse),
                &no_location(),
            ) else {
                return Ok(Vec::new());
            };
            if sorted {
                keys.sort();
            }
            Ok(vec![no_comments(OwnedValue::Array(
                keys.into_iter().map(OwnedValue::String).collect(),
            ))])
        }
        // Same reasoning as `LazyKeys` above, for array `keys`/
        // `keys_unsorted` (#684).
        GenericResult::LazyIndexRange(len) => Ok(vec![no_comments(OwnedValue::Array(
            (0..len).map(|i| OwnedValue::Int(i as i64)).collect(),
        ))]),
        // Same reasoning as `LazyKeys`/`LazyIndexRange` above, for a
        // composed `map` chain (#724, #725) that never resolved into a
        // narrower shape before reaching this materializing DOM boundary.
        GenericResult::LazySeq(seq) => match seq.materialize_atomic() {
            Ok(v) => Ok(vec![no_comments(v)]),
            Err(jq::Control::Error(e)) => {
                sink.report(DiagStyle::Yq, &e, &no_location());
                Ok(vec![])
            }
            Err(jq::Control::Break(label)) => {
                sink.report_break(DiagStyle::Yq, &label, &no_location());
                Ok(vec![])
            }
            Err(jq::Control::Halt(code)) => {
                sink.request_halt(code);
                Ok(vec![])
            }
        },
        GenericResult::None => Ok(vec![]),
        GenericResult::Error(e) => {
            sink.report(DiagStyle::Yq, &e, &no_location());
            Ok(vec![])
        }
        GenericResult::Owned(v) => Ok(vec![no_comments(v)]),
        GenericResult::ManyOwned(vs) => Ok(vs.into_iter().map(no_comments).collect()),
        GenericResult::Break(label) => {
            sink.report_break(DiagStyle::Yq, &label, &no_location());
            Ok(vec![])
        }
        // `halt`/`halt_error` (#791): not a diagnostic, so no `sink.report*`
        // call — `request_halt` records the exit code for the caller's loop
        // to short-circuit on, without touching `hit`/`report_count`.
        GenericResult::Halt(code) => {
            sink.request_halt(code);
            Ok(vec![])
        }
        // The outputs already produced no longer vanish behind the failure
        // (#400, #494).
        GenericResult::Partial(vs, jq::Control::Error(e)) => {
            sink.report(DiagStyle::Yq, &e, &no_location());
            Ok(vs.into_iter().map(no_comments).collect())
        }
        GenericResult::Partial(vs, jq::Control::Break(label)) => {
            sink.report_break(DiagStyle::Yq, &label, &no_location());
            Ok(vs.into_iter().map(no_comments).collect())
        }
        GenericResult::Partial(vs, jq::Control::Halt(code)) => {
            sink.request_halt(code);
            Ok(vs.into_iter().map(no_comments).collect())
        }
    };

    if let Some((pristine, groups)) = &alias_sync_ctx {
        if let Ok(docs) = &mut docs {
            for (value, _comments) in docs.iter_mut() {
                sync_aliased_paths(value, pristine, groups);
            }
        }
    }

    // A bare top-level/navigated scalar result drops all its own styling,
    // matching real `yq` (#852) - unlike a scalar nested inside a mapping/
    // sequence, which keeps it (`a: "x"` stays quoted when output as part
    // of the whole document; a standalone `.a` result doesn't). Both
    // `to_owned_with_comments` and `reconcile_presentation` above capture/
    // preserve style uniformly for every node including the root, so this
    // needs its own always-on, top-level-only pass, independent of
    // `need_comments`/`is_alias_sensitive_assign` — real `yq` does this
    // unconditionally, not just for shape-preserving writes.
    //
    // A bare scalar's own `&anchor` goes the same way (#763): real `yq`
    // prints `1`, not `&x 1`, for `.a` on `a: &x 1` — already pinned by
    // `test_yaml_bare_scalar_anchor_dropped_on_query_result_712`. Clearing
    // the whole `NodeMeta` here covers an alias mark too, which
    // `enforce_anchor_soundness` below would drop regardless (see
    // `output_value`'s own note on why a root `*name` is never emittable).
    if let Ok(docs) = &mut docs {
        for (value, comments) in docs.iter_mut() {
            if !matches!(value, OwnedValue::Object(_) | OwnedValue::Array(_)) {
                *comments = CommentTree::Leaf(NodeMeta::from_comment_and_style(
                    comments.own().map(str::to_string),
                    "",
                ));
            }
        }
    }

    // `-P`/`--pretty-print` (#705) forces block/plain style regardless of
    // source — a real style-clearing step, not just "there's no style data
    // to clear" like before #739's style tracking existed. Comments and
    // anchor marks stay (`-P` only ever claimed to affect style, and real
    // `yq -P '.'` does keep `&x`/`*x`), so this only touches the style slot
    // of every node, not the tree shape.
    if strip_style {
        if let Ok(docs) = &mut docs {
            for (_value, comments) in docs.iter_mut() {
                *comments = strip_presentation_style(comments);
            }
        }
    }

    // Last, once every other pass has settled the values and the marks:
    // strip any `*alias` this document can no longer resolve (#763). Must
    // run after `sync_aliased_paths` above, whose whole job is making an
    // alias's value agree with its anchor's again — running before it would
    // see stale values and drop marks that are about to become valid.
    //
    // `has_aliases` gates it the way it gates `alias_sync_ctx` above, and
    // for a stronger reason than "probably nothing to do": this pass only
    // ever clears `Aliases` marks and never touches a `Declares` one, so a
    // document with no `*name` anywhere has nothing it *could* change. The
    // walk is not free — it visits every node of the reconciled tree — and
    // an alias-free document is the overwhelmingly common case.
    if has_aliases {
        if let Ok(docs) = &mut docs {
            for (value, comments) in docs.iter_mut() {
                enforce_anchor_soundness(value, comments, sort_keys);
            }
        }
    }

    docs
}

/// Recursively clear every node's style (issue #705's `-P` gate) while
/// keeping its comments, anchor marks and tree shape untouched — see the
/// `strip_style` call in [`evaluate_yaml_cursor`].
///
/// Anchors deliberately survive `-P`: real `yq -P '.'` on
/// `a: &x 1\nb: *x\n` still prints `a: &x 1\nb: *x` (verified against the
/// pinned binary). `-P` is documented as `... style = ""`, and anchor/alias
/// syntax is identity, not style (#763).
fn strip_presentation_style(tree: &CommentTree) -> CommentTree {
    strip_presentation_style_at_depth(tree, 0)
}

/// Panics past `succinctly::jq::MAX_VALUE_TREE_DEPTH` levels of nesting
/// (#1017) -- unlike its sibling [`reconcile_presentation`], which #1015
/// already guards, this walker over the same `CommentTree` shape had no
/// guard at all. Currently only fed an already-reconciled/bounded tree,
/// so this is defense-in-depth against that call chain changing, not a
/// currently-live independent crash path.
fn strip_presentation_style_at_depth(tree: &CommentTree, depth: usize) -> CommentTree {
    assert_value_tree_depth(depth);
    let meta = NodeMeta {
        style: "",
        ..tree.meta().clone()
    };
    match tree {
        CommentTree::Leaf(_) => CommentTree::Leaf(meta),
        CommentTree::Array(_, items) => CommentTree::Array(
            meta,
            items
                .iter()
                .map(|v| strip_presentation_style_at_depth(v, depth + 1))
                .collect(),
        ),
        CommentTree::Object(_, fields, key_comments) => CommentTree::Object(
            meta,
            fields
                .iter()
                .map(|(k, v)| (k.clone(), strip_presentation_style_at_depth(v, depth + 1)))
                .collect(),
            key_comments.clone(),
        ),
    }
}

/// Write the `---` document separator for fast-path YAML output (#175).
///
/// yq separates YAML documents with `---` only between documents that produce
/// output: never before the first, and never around a document whose query
/// yields no results. `will_output` says whether the current document is about
/// to emit anything; `streamed` records that an earlier document already did.
///
/// `no_doc` suppresses the separator itself while still updating `streamed`
/// (#1699) -- mirroring `SplitDocState::write_separator`'s existing
/// `!config.no_doc` check, which this fast path had no equivalent of before:
/// the DOM path (the eval-all loop further below) already respects
/// `--no-doc`, but this macro-shared M2 path wrote `---` unconditionally.
///
/// `terminator` (#1701 code review) says whether the *previous* document's
/// own output already ended in a newline: only `Terminator::Newline` does.
/// `-0`/`--join-output` end a document in `\0`/nothing instead, and this
/// function used to assume a trailing `\n` unconditionally -- gluing `---`
/// directly onto the prior document's own last byte with no boundary
/// between them. Confirmed live: `--join-output` on two YAML documents
/// `a: 1`/`b: 2` produced `a: 1---\nb: 2`, which *reparses* as the single,
/// silently corrupted document `{"a": "1---", "b": 2}` -- `-i` persists
/// that corruption to disk. Inserting an explicit `\n` first whenever the
/// terminator wasn't already one keeps `---` a valid, position-independent
/// YAML document boundary regardless of which terminator style produced the
/// document before it (this needed no divergence-registry entry: `-0`'s own
/// glued-`---` case was verified byte-identical to real yq's own output for
/// the same input, but real yq can't parse that output back either --
/// ADR-0018's carve-out for reference output the tool can't itself read
/// back applies, so fixing it here isn't a fidelity violation).
fn emit_yaml_doc_separator<W: std::io::Write>(
    writer: &mut W,
    streamed: &mut bool,
    will_output: bool,
    no_doc: bool,
    terminator: Terminator,
) -> std::io::Result<()> {
    if *streamed && will_output && !no_doc {
        write_doc_separator_marker(writer, terminator)?;
    }
    *streamed |= will_output;
    Ok(())
}

/// Writes a `\n` iff `terminator` isn't already one -- the shared guard
/// behind every `---`/front-matter-fence write in this file (#1701 code
/// review): each one used to assume the byte immediately before it was
/// already a newline (true for the default terminator, false for
/// `-0`/`--join-output`), gluing the marker directly onto the previous
/// document's own last byte and corrupting it on reparse. Called
/// immediately before every such write, never after -- so it composes with
/// whichever exact trailing form (`writeln!(..., "---")`, a bare
/// `write_all(b"---")` followed by the caller's own line-ending logic, etc.)
/// that call site already used.
fn write_doc_marker_newline_guard<W: std::io::Write>(
    writer: &mut W,
    terminator: Terminator,
) -> std::io::Result<()> {
    if !matches!(terminator, Terminator::Newline) {
        writer.write_all(b"\n")?;
    }
    Ok(())
}

/// `write_doc_marker_newline_guard` immediately followed by the `---`
/// separator itself -- the pairing every ordinary (non-front-matter-fence)
/// `---` write in this file needs. Folded into one call after review found
/// the two-line pair repeated verbatim at every such site: each repeat was
/// itself a chance for a future writer to add a 6th and forget the guard,
/// which is exactly how #1701 needed four separate review rounds to find
/// every site that had. The two front-matter fence sites don't use this --
/// they follow the guard with `extend_from_slice`/`write_all` of `---` plus
/// a caller-supplied line ending and body bytes, not a bare `---`+`\n`, so
/// they call `write_doc_marker_newline_guard` directly instead.
fn write_doc_separator_marker<W: std::io::Write>(
    writer: &mut W,
    terminator: Terminator,
) -> std::io::Result<()> {
    write_doc_marker_newline_guard(writer, terminator)?;
    writeln!(writer, "---")
}

/// State for tracking split_doc output separators.
struct SplitDocState {
    has_split_doc: bool,
    /// Same polarity as `DocSeparatorArgs`/`PendingSeparator`'s own
    /// `doc_streamed` (`true` once *any* output has happened), not the
    /// original `is_first_output`'s inverted one -- needed so a `-0` call
    /// site can hand this field straight to `output_value` as its own
    /// deferred separator state (#1709 code review) without an extra
    /// polarity-flipping adapter.
    any_output: bool,
}

impl SplitDocState {
    fn new(has_split_doc: bool) -> Self {
        Self {
            has_split_doc,
            any_output: false,
        }
    }

    /// Write a separator if needed for split_doc mode -- unless `config`'s
    /// output is NUL-separated, in which case the separator is deferred
    /// into the returned `DocSeparatorArgs` for `output_value`'s own
    /// per-result NUL check to decide instead (#1709 code review): this
    /// method used to always write eagerly, the same bug class every other
    /// document-separator call site in this file was fixed for -- verified
    /// live that a `split_doc`-tagged result whose value contains an
    /// embedded NUL left a dangling `---` for content that was never
    /// actually emitted.
    ///
    /// Same `---`-must-start-on-its-own-line fix as `emit_yaml_doc_separator`
    /// (#1701 code review) for the eager case: the previous document's own
    /// terminator might have been `\0`/nothing rather than `\n`, and gluing
    /// `---` directly onto that document's last byte corrupts it on
    /// reparse.
    fn write_separator<W: Write>(
        &mut self,
        writer: &mut W,
        config: &OutputConfig,
    ) -> Result<Option<DocSeparatorArgs<'_>>> {
        if !self.has_split_doc || config.output_format != OutputFormat::Yaml || config.no_doc {
            return Ok(None);
        }
        if config.nul_output {
            return Ok(Some(DocSeparatorArgs {
                doc_streamed: &mut self.any_output,
                no_doc: config.no_doc,
            }));
        }
        if self.any_output {
            write_doc_separator_marker(writer, terminator_from_config(config))?;
        }
        self.any_output = true;
        Ok(None)
    }
}

/// yq-mode's own thin wrapper onto [`output::Terminator::from_flags`] --
/// see that function's own doc for the NUL/newline/join precedence rule
/// and its jq-vs-yq oracle verification. Exists only because
/// jq_runner.rs's own `OutputConfig` names the same flag `raw_output0`
/// rather than `nul_output` (#1711), so the two runners can't share one
/// `from_config`-style call; reading both fields once here means no call
/// site in this file can transpose them.
///
/// Threaded through `stream_cursor!`'s own `$output_config:expr` (#1701
/// code review) so the same three-way choice also drives the M2 fast
/// path's own terminator writes -- previously `stream_cursor!` hardcoded a
/// bare newline unconditionally, ignoring both flags whenever the fast
/// path was taken (confirmed live on unmodified `main`, same root cause
/// and fix shape as #1699's `--no-doc` gap in the same macro).
fn terminator_from_config(config: &OutputConfig) -> Terminator {
    Terminator::from_flags(config.nul_output, config.join_output)
}

/// Write the appropriate line terminator based on output config.
fn write_terminator<W: Write>(writer: &mut W, config: &OutputConfig) -> Result<()> {
    terminator_from_config(config).write_io(writer)?;
    Ok(())
}

/// Format and output a value, threading `comments` through to `emit_yaml_value`
/// (issue #710). Callers with no cursor-derived comment data (JSON input,
/// `--null-input`, `--raw-input`, `--slurp`, `--inplace`'s slow path) pass
/// `&CommentTree::empty()`.
/// DOM-path counterpart to `NulCheckedSink` (#1709) -- `output_value`
/// already fully materializes its rendered text as an owned `String`
/// before writing, unlike the M2 streaming path, so the check here needs
/// no per-result buffering of its own: this isn't a new allocation, only a
/// new scan over one `output_value` already made. Scans the pre-colorized
/// text (colorizing only wraps existing content in ANSI codes, never
/// alters or strips a NUL byte already there), and runs before
/// `write_terminator`, so a `Terminator::Nul` byte written separately,
/// after this check, is never mistaken for a rejected value's own content.
fn reject_nul_output(rendered: &str, config: &OutputConfig) -> Result<()> {
    if config.nul_output && rendered.contains('\0') {
        anyhow::bail!(NUL_OUTPUT_ERROR_MESSAGE);
    }
    Ok(())
}

/// Writes a pending `---` document separator, once its caller's own
/// [`reject_nul_output`] check has already passed (#1709 code review):
/// every `output_value` call site that precedes it with an eager
/// `write_doc_separator_marker` call has the identical bug the M2 path's
/// own `PendingSeparator` was built to close -- a document whose first
/// result fails the NUL check still leaves its separator already on the
/// writer. `None` for a call site with no separator concept of its own
/// (most of `output_value`'s callers -- `-n`, `--raw-input`, `--slurp`,
/// `--split-exp`, ...), matching `DocSeparatorArgs`'s own YAML-only use in
/// `stream_maybe_colored`.
fn write_pending_separator<W: Write>(
    writer: &mut W,
    separator: Option<DocSeparatorArgs<'_>>,
    terminator: Terminator,
) -> Result<()> {
    if let Some(sep) = separator {
        // Reuses the same guard/write/set-flag logic every other terminator
        // mode's separator goes through, rather than hand-duplicating it
        // here (#1709 code review) -- `true` for `will_output` since
        // reaching this point already means this result survived
        // `output_value`'s own NUL check.
        emit_yaml_doc_separator(writer, sep.doc_streamed, true, sep.no_doc, terminator)?;
    }
    Ok(())
}

fn output_value<W: Write>(
    writer: &mut W,
    value: &OwnedValue,
    comments: &CommentTree,
    config: &OutputConfig,
    separator: Option<DocSeparatorArgs<'_>>,
) -> Result<()> {
    // Handle raw output for scalars
    if config.raw_output {
        if let OwnedValue::String(s) = value {
            reject_nul_output(s, config)?;
            write_pending_separator(writer, separator, terminator_from_config(config))?;
            write!(writer, "{s}")?;
            write_terminator(writer, config)?;
            return Ok(());
        }
    }

    // For YAML output format (default)
    if config.output_format == OutputFormat::Yaml {
        // For YAML, scalars are printed without quotes by default (like -r in yq)
        // A bare top-level result drops all of its own styling (#852,
        // mirroring `YamlCursor::stream_yaml_as_document`'s and
        // `OwnedValue::stream_yaml`'s identical root-only special case for
        // the cursor/streaming paths) - `output_value` is the actual top
        // level for every result that reaches it (its own recursion never
        // calls back into `output_value`, only `emit_yaml_value`), so this
        // covers every caller uniformly: the main eval path, `-n`,
        // `--raw-input`, `--split-exp`, ... Redundant with (but harmless
        // alongside) `evaluate_yaml_cursor`'s equivalent root-scalar pass
        // for the cursor-based DOM path specifically.
        // No root case for an *alias* mark here, deliberately: a root
        // `*name` can never survive `enforce_anchor_soundness`, because its
        // `&name` would have to sit inside the alias's own subtree, and a
        // cyclic anchor is rejected outright at index build
        // (`YamlError::AliasCycle`). So the mark is always cleared before
        // this point, and a branch for it would be dead code (#763).
        //
        // That does mean `succinctly yq '.b'` on `b: *x` prints `*x` (the
        // streaming path) while `-P '.b'` prints the resolved value. Real
        // yq prints `*x` for both — and cannot read either back
        // (`unknown anchor 'x' referenced`). Making the streaming path
        // agree with this one is issue #1350.
        let body = if let OwnedValue::String(s) = value {
            s.clone()
        } else {
            // A root anchor is written only for a container, matching
            // `YamlCursor::write_leading_anchor` in `light.rs` — real yq
            // drops a bare scalar root's own `&x` (pinned by
            // `test_yaml_bare_scalar_anchor_dropped_on_query_result_712`),
            // and `evaluate_yaml_cursor`'s root-scalar pass has already
            // cleared the mark for that case anyway.
            let rendered = emit_yaml_value(value, comments, config, "", false);
            match comments.declared_anchor() {
                Some(anchor) if matches!(value, OwnedValue::Array(_) | OwnedValue::Object(_)) => {
                    if is_flow_safe(value, comments) {
                        format!("&{anchor} {rendered}")
                    } else {
                        format!("&{anchor}\n{rendered}")
                    }
                }
                _ => rendered,
            }
        };
        // Every non-root node's own trailing comment is appended by its
        // *parent* during `emit_yaml_value`'s recursion (see its Array/Object
        // arms), but the root has no parent call site to do that for it —
        // append it here instead, or a comment trailing the jq result's own
        // top-level node (e.g. `[1, 2, 3] # trailing`) is silently dropped
        // (#710). Scalars are excluded: verified against the pinned real
        // `yq` binary, a bare scalar document (`42 # trailing`) drops its
        // own trailing comment from output on both identity and `select`,
        // even though `line_comment` still returns it — real `yq`'s own
        // quirk, not a succinctly gap, so replicated here rather than
        // "fixed" into a new divergence.
        //
        // A flow-styled root (`comments.style() == "flow"`, #739) glues the
        // comment onto `body`'s one and only line instead
        // (`[1, 2, 3] # trailing`, matching real `yq` exactly) - there's no
        // nested child on that same line to collide with. A block-rendered
        // root instead appends it as a standalone comment line, or it would
        // be indistinguishable from the last child's own comment on
        // `body`'s last line (#793) - `append_own_comment_line`'s own doc
        // comment already flagged this as the reason it couldn't match real
        // `yq`'s flow-preserving output, before `CommentTree` carried style
        // data to tell the two cases apart.
        let output = if matches!(value, OwnedValue::Array(_) | OwnedValue::Object(_)) {
            if is_flow_safe(value, comments) {
                format!("{body}{}", trailing_comment_suffix(comments))
            } else {
                append_own_comment_line(body, comments.own(), "")
            }
        } else {
            body
        };
        reject_nul_output(&output, config)?;
        write_pending_separator(writer, separator, terminator_from_config(config))?;
        if config.use_color {
            // No boundary recorded here -- `write_terminator` below writes
            // directly to `writer`, outside this buffer (#1708).
            write!(writer, "{}", colorize_yaml(&output, Terminator::None, &[]))?;
        } else {
            write!(writer, "{output}")?;
        }
        write_terminator(writer, config)?;
        return Ok(());
    }

    // JSON output format. Both compact and pretty route through the shared
    // formatter with yq's control-char escaping so the two agree byte-for-byte
    // on control characters — `\u0008`/`\u000c` (not jq's `\b`/`\f`) and raw
    // DEL/C1 controls — matching `mikefarah/yq` and the M2 streaming fast path
    // (#262). Compact keeps jq-shortest floats (e.g. `1`) to match the streaming
    // path; pretty preserves whole floats (e.g. `1.0`).
    let json_str = output::format_json(
        value,
        &JsonFormatOpts {
            indent: if config.compact {
                ""
            } else {
                &config.indent_str
            },
            sort_keys: config.sort_keys,
            ascii: config.ascii_output,
            float_style: if config.compact {
                FloatStyle::Shortest
            } else {
                FloatStyle::PreserveWholeFloat
            },
            json_sourced: config.json_sourced_floats,
            control_escape: ControlEscape::Yq,
        },
    );

    reject_nul_output(&json_str, config)?;
    write_pending_separator(writer, separator, terminator_from_config(config))?;
    if config.use_color {
        write!(
            writer,
            "{}",
            output::colorize_json(&json_str, &ColorScheme::default())
        )?;
    } else {
        write!(writer, "{json_str}")?;
    }

    write_terminator(writer, config)?;

    Ok(())
}

/// Whether `value`/`comments` should render in YAML flow style (`[...]`/
/// `{...}`, issue #739) — `comments.style() == "flow"`, unless a child
/// (array element or object field) has its own trailing comment.
///
/// A `#` comment runs to end of line, so a flow collection has nowhere to
/// put one before its last item without breaking onto another line anyway
/// (real `yq` does this with a synthetic trailing comma:
/// `[1, 2, # child\n]`). Falling back to block style — already
/// comment-safe, since every comment gets its own line there — is simpler
/// and more general than replicating that exact placement, at the cost of
/// not matching real `yq`'s output byte-for-byte in this one narrow case
/// (a comment on a non-final flow element); losing the comment entirely
/// would be worse.
fn is_flow_safe(value: &OwnedValue, comments: &CommentTree) -> bool {
    if comments.style() != "flow" {
        return false;
    }
    match value {
        OwnedValue::Array(items) => !(0..items.len()).any(|i| comments.at_index(i).own().is_some()),
        OwnedValue::Object(fields) => !fields.keys().any(|k| comments.field(k).own().is_some()),
        _ => true,
    }
}

/// Whether this node's content is deferred to its own indented block below
/// the `key:`/`-` that introduces it, rather than sharing that line.
///
/// The single definition of a test the block-mapping and block-sequence arms
/// of [`emit_yaml_value_at_depth`] each used to spell inline — extracted
/// because #763 adds a third reason to stay on one line: an aliased node
/// renders as `*name`, so it is inline no matter how large the anchored
/// value it points at happens to be. Getting that wrong renders `b: *x` as
/// a block mapping with `*x` dangling above it.
/// DOM twin of `light.rs`'s `compact_child_indent` (#1485): the indent for
/// content nested one level inside a compact-rendered field/element's own
/// value. An ordinary `{recursion_base}{indent_str}` step, *unless* that
/// step wouldn't land past `indent` (the field's own compact-adjusted
/// indent) -- real yq's own rule, which only bites when `indent_str`'s
/// width is `<=` the compact `- `/first-field offset (2 columns) --
/// real yq's own default, `-I=2`, is exactly that boundary. See
/// `light.rs`'s `compact_child_indent` for the full live-verified
/// rationale (both writers are pinned against the same oracle behavior,
/// #763).
///
/// Reduces to the pre-existing, always-correct `format!("{indent}{step}")`
/// whenever `recursion_base == indent` (the non-compact case, the vast
/// majority of calls): the "normal" candidate is then always longer than
/// `indent` itself (appending at least one character), so the `if` always
/// takes that branch.
fn compact_child_indent(recursion_base: &str, indent: &str, indent_str: &str) -> String {
    let normal = format!("{recursion_base}{indent_str}");
    if normal.chars().count() > indent.chars().count() {
        normal
    } else {
        format!("{indent}{indent_str}")
    }
}

fn defers_to_own_block(value: &OwnedValue, comments: &CommentTree) -> bool {
    comments.alias_name().is_none()
        && !is_flow_safe(value, comments)
        && (matches!(value, OwnedValue::Object(o) if !o.is_empty())
            || matches!(value, OwnedValue::Array(a) if !a.is_empty()))
}

/// This node's `&name` anchor declaration as ` &name` (leading space), or
/// `""` if it declares none (#763).
///
/// The leading space is part of the returned string because every call site
/// appends it directly after a `:` or `-`, exactly as `write_deferred_value`
/// in `light.rs` does for the streaming writer.
fn anchor_decl_prefix(comments: &CommentTree) -> String {
    comments
        .declared_anchor()
        .map_or_else(String::new, |name| format!(" &{name}"))
}

/// Format a node's own trailing comment (issue #710) as `" # text"`, or
/// `""` if it has none — the single point of change for the separator
/// convention shared by every `emit_yaml_value` call site that appends one
/// *on the same line* as the rest of the value (safe only when that value
/// renders as a single line — a scalar, or an empty/flow-style container).
fn trailing_comment_suffix(comments: &CommentTree) -> String {
    comments.own().map_or_else(String::new, |c| format!(" {c}"))
}

/// Append a container's own trailing comment (#710/#793) as a standalone
/// comment line at `indent`, rather than concatenating it onto `body`'s
/// last line the way [`trailing_comment_suffix`] does. `body` here is
/// always genuinely multi-line block content (a non-empty, block-rendered
/// `Array`/`Object`) — gluing the container's *own* comment onto that last
/// line would make it indistinguishable from the last child's own trailing
/// comment on that same line, silently merging two distinct comments into
/// one (#793). `OwnedValue` carries no source flow/block style, so this
/// doesn't attempt to match real `yq`'s exact output (which keeps flow
/// style, staying on one line); it only guarantees the two comments never
/// collide.
fn append_own_comment_line(body: String, own_comment: Option<&str>, indent: &str) -> String {
    match own_comment {
        Some(c) => format!("{body}\n{indent}{c}"),
        None => body,
    }
}

/// Emit a YAML value as a string, appending each node's trailing same-line
/// comment from the parallel `comments` tree (issue #710). Flow-style
/// (`in_flow`) contexts never append one — flow items are comma-joined on
/// one line, so there's no meaningful "trailing" position between them.
///
/// `indent` is the exact indent *string* to prepend to this value's own
/// top-level line(s) — not a `depth: usize` repetition count. A block
/// sequence item whose value is a non-empty mapping/sequence renders in
/// real yq's "compact" form (`- ` shares its line with the value's first
/// field/element, and the rest of the value's own content aligns under
/// that first line's content rather than under a full `config.indent_str`
/// step further — see the `Array` arm below), so a plain `depth *
/// config.indent_str` formula can't express every line's indent; passing
/// the literal string lets a compact caller hand down `indent` plus a
/// fixed 2-character offset instead of a whole extra `config.indent_str`
/// step (#785).
fn emit_yaml_value(
    value: &OwnedValue,
    comments: &CommentTree,
    config: &OutputConfig,
    indent: &str,
    in_flow: bool,
) -> String {
    emit_yaml_value_at_depth(value, comments, config, indent, in_flow, 0, indent)
}

/// Panics past `succinctly::jq::MAX_VALUE_TREE_DEPTH` levels of nesting
/// (#1017) -- see [`reconcile_presentation_at_depth`], the sibling this
/// mirrors: both are the default YAML-target output emitters for a
/// filter's evaluated result (JSON-target has `output::format_json_impl`,
/// guarded by #1015; this is YAML-target's own copy), reachable on a
/// value constructed via `reduce`/`foreach`/etc. with no adversarial
/// document on either side.
///
/// `recursion_base` (#1485): the indent a *nested* container reached from
/// this value should step deeper from -- equal to `indent` everywhere
/// except right after a real-yq "compact" positioning (the `Array` arm's
/// two compact branches below, which pass this call's own pre-compact
/// `indent` rather than the compact-adjusted one they pass as `indent`
/// itself). See `light.rs`'s `stream_yaml_value` doc comment on its own
/// identical parameter, and `compact_child_indent` just above, for the
/// full live-verified rationale -- this DOM writer and that streaming one
/// are pinned against the same oracle behavior (#763).
fn emit_yaml_value_at_depth(
    value: &OwnedValue,
    comments: &CommentTree,
    config: &OutputConfig,
    indent: &str,
    in_flow: bool,
    depth: usize,
    recursion_base: &str,
) -> String {
    assert_value_tree_depth(depth);
    // An alias renders as `*name` and never writes the value it resolves
    // to — the DOM twin of `stream_yaml_value`'s own `YamlValue::Alias`
    // arm in `light.rs` (#763). This has to come before the value dispatch
    // below, not inside it: `to_owned` already cloned the *target's* value
    // into this position, so by here an alias is indistinguishable from a
    // plain copy by shape alone. Emitted structurally rather than as a
    // string, since `yaml_quote_string` quotes anything starting with `*`.
    //
    // `enforce_anchor_soundness` has already cleared any mark that could
    // not be resolved from this document, so reaching here guarantees a
    // matching `&name` is emitted earlier in the output.
    if let Some(name) = comments.alias_name() {
        return format!("*{name}");
    }
    match value {
        OwnedValue::Null => "null".to_string(),
        OwnedValue::Bool(b) => b.to_string(),
        OwnedValue::Int(n) => n.to_string(),
        OwnedValue::Float(f) if f.is_nan() || f.is_infinite() => {
            nonfinite_display_string::<YqSemantics>(*f).to_string()
        }
        // Real yq drops a computed whole float's decimal point (`2`, not
        // `2.0`) only at document-root scalar position, where it suppresses
        // every tag (issue #949; `echo '!!str 5' | yq '.'` prints a bare
        // `5` too). Anywhere nested -- an object field, an array element,
        // an `-i` in-place edit -- it keeps the same shortest spelling but
        // precedes it with an explicit `!!float` tag whenever that spelling
        // would read back as an int (`a: !!float 2`, issue #1090).
        OwnedValue::Float(f) if depth == 0 => format_float_yq_yaml(*f),
        OwnedValue::Float(f) => format_float_yq_yaml_nested(*f),
        OwnedValue::NumberLiteral(NumberRepr::Float(f), _) if f.is_nan() || f.is_infinite() => {
            nonfinite_display_string::<YqSemantics>(*f).to_string()
        }
        OwnedValue::NumberLiteral(_, literal) => {
            // Echo the source spelling verbatim (#1008) rather than
            // routing through `number_str()`/`format_number_jq_compat`,
            // which reformats per jq's own rules (uppercase `E`, forced
            // sign) -- this file is yq-CLI-only, no jq caller to protect,
            // and yq preserves a document literal's exact text.
            literal.to_string()
        }
        OwnedValue::String(s) => yaml_quote_string_with_style(s, comments.style()),
        OwnedValue::Array(arr) => {
            if arr.is_empty() {
                "[]".to_string()
            } else if in_flow || is_flow_safe(value, comments) {
                // Flow style for nested in flow context
                let items: Vec<_> = arr
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        let elem_comments = comments.at_index(i);
                        let item = emit_yaml_value_at_depth(
                            v,
                            elem_comments,
                            config,
                            indent,
                            true,
                            depth + 1,
                            indent,
                        );
                        // `[&x 1, *x]` — a flow item's own anchor sits
                        // immediately before it (#763), the DOM twin of
                        // `write_yaml_child_inline` in `light.rs`.
                        match elem_comments.declared_anchor() {
                            Some(anchor) => format!("&{anchor} {item}"),
                            None => item,
                        }
                    })
                    .collect();
                format!("[{}]", items.join(", "))
            } else {
                // Block style sequence
                let items: Vec<_> = arr
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        let elem_comments = comments.at_index(i);
                        if defers_to_own_block(v, elem_comments) {
                            // Both arms below share this same 2-column
                            // "compact" offset — the `- ` prefix's own
                            // width, not a full `config.indent_str` step
                            // (#785/#1362) — so it's computed once here
                            // rather than separately in each arm.
                            let compact_indent = format!("{indent}  ");
                            // An anchor written on the item's own line
                            // (`- &x\n  ...`) takes the slot the compact
                            // form would otherwise put the value's first
                            // field/element in, so the value defers to its
                            // own line below — at the same 2-column compact
                            // width as an unanchored compact element (#1362),
                            // not a full indent step — mirroring
                            // `stream_yaml_value`'s sequence arm in
                            // `light.rs`, which real yq is pinned against
                            // (#763).
                            if let Some(anchor) = elem_comments.declared_anchor() {
                                // Same "compact" rule as the plain block-sequence
                                // element below: the value's own content aligns
                                // under the `- ` prefix's 2-column width, not a
                                // full `config.indent_str` step (#1362 -- an
                                // anchor on its own line still occupies that `- `
                                // slot, so its value nests exactly as deep as an
                                // unanchored element's would).
                                let val_indent = compact_indent;
                                // #1485 (code review): `recursion_base` is
                                // *this invocation's own* `recursion_base`,
                                // forwarded unchanged -- not `indent`, which
                                // is only correct when this element was
                                // never itself compact-positioned. A stacked
                                // compact chain (an array directly inside
                                // another compact array element) must keep
                                // propagating the true pre-compact base
                                // through every level -- see `light.rs`'s
                                // identical fix for the full rationale.
                                let val = emit_yaml_value_at_depth(
                                    v,
                                    elem_comments,
                                    config,
                                    &val_indent,
                                    false,
                                    depth + 1,
                                    recursion_base,
                                );
                                let val =
                                    append_own_comment_line(val, elem_comments.own(), &val_indent);
                                return format!("{indent}- &{anchor}\n{val}");
                            }
                            // A non-empty mapping/sequence element renders
                            // in real yq's "compact" form: `- ` shares its
                            // line with the value's own first field/
                            // element, and the rest of the value's own
                            // content aligns under that first line's
                            // content (`indent` plus the 2-character width
                            // of `- `), not under `indent` plus a full
                            // `config.indent_str` step like an ordinary
                            // nested block (#785).
                            //
                            // `emit_yaml_value` derives every line's own
                            // indent purely from the `indent` string it's
                            // handed, so rendering the element at
                            // `compact_indent` and then stripping that
                            // exact prefix from just the start of the
                            // result (leaving every subsequent line's own
                            // copy of the prefix untouched) reproduces the
                            // "no separate indent for the first line"
                            // effect `stream_yaml_value`'s cursor-based
                            // sibling gets for free from its per-field/
                            // per-element loop only indenting 2nd+ items.
                            // #1485 (code review): `recursion_base` is
                            // *this invocation's own* `recursion_base`,
                            // forwarded unchanged -- see the anchor branch
                            // above for the full rationale.
                            let rendered = emit_yaml_value_at_depth(
                                v,
                                elem_comments,
                                config,
                                &compact_indent,
                                false,
                                depth + 1,
                                recursion_base,
                            );
                            // The element's own comment goes on its own
                            // line rather than glued onto its last
                            // grandchild's line (#793).
                            let rendered = append_own_comment_line(
                                rendered,
                                elem_comments.own(),
                                &compact_indent,
                            );
                            let first_line = rendered
                                .strip_prefix(compact_indent.as_str())
                                .unwrap_or(&rendered);
                            format!("{indent}- {first_line}")
                        } else {
                            let val_indent = format!("{indent}{}", config.indent_str);
                            let item = emit_yaml_value_at_depth(
                                v,
                                elem_comments,
                                config,
                                &val_indent,
                                false,
                                depth + 1,
                                &val_indent,
                            );
                            let comment_suffix = trailing_comment_suffix(elem_comments);
                            let anchor = anchor_decl_prefix(elem_comments);
                            format!("{indent}-{anchor} {item}{comment_suffix}")
                        }
                    })
                    .collect();
                items.join("\n")
            }
        }
        OwnedValue::Object(obj) => {
            if obj.is_empty() {
                "{}".to_string()
            } else if in_flow || is_flow_safe(value, comments) {
                // Flow style for nested in flow context
                let entries: Vec<_> = obj
                    .iter()
                    .map(|(k, v)| {
                        let key = yaml_quote_key(k);
                        let field_comments = comments.field(k);
                        let val = emit_yaml_value_at_depth(
                            v,
                            field_comments,
                            config,
                            indent,
                            true,
                            depth + 1,
                            indent,
                        );
                        // `{x: &y 1, z: *y}` (#763).
                        let anchor = anchor_decl_prefix(field_comments);
                        format!("{key}:{anchor} {val}")
                    })
                    .collect();
                format!("{{{}}}", entries.join(", "))
            } else {
                // Block style mapping
                let entries: Vec<_> = if config.sort_keys {
                    let mut sorted: Vec<_> = obj.iter().collect();
                    sorted.sort_by(|a, b| a.0.cmp(b.0));
                    sorted
                } else {
                    obj.iter().collect()
                };

                let items: Vec<_> = entries
                    .iter()
                    .map(|(k, v)| {
                        let key = yaml_quote_key(k);
                        let field_comments = comments.field(k);
                        let comment_suffix = trailing_comment_suffix(field_comments);
                        // #1485: steps from `recursion_base`, not `indent`
                        // -- see `compact_child_indent`'s own doc comment.
                        let val_indent =
                            compact_child_indent(recursion_base, indent, &config.indent_str);
                        let anchor = anchor_decl_prefix(field_comments);
                        // Check if value needs to be on next line - a
                        // flow-styled container stays on the key's own line
                        // instead, the same as a scalar (#739), and so does
                        // an alias, however large its target (#763).
                        if defers_to_own_block(v, field_comments) {
                            // A comment trailing the key's own line, when the
                            // value is deferred to the next line, belongs to
                            // the key, not the value (#765).
                            let key_comment_suffix = comments
                                .key_comment(k)
                                .map_or_else(String::new, |c| format!(" {c}"));
                            // For nested containers, emit one `config.indent_str`
                            // step deeper, which handles its own indentation.
                            // The value's own comment goes on its own line
                            // rather than glued onto its last grandchild's
                            // line (#793).
                            let val = emit_yaml_value_at_depth(
                                v,
                                field_comments,
                                config,
                                &val_indent,
                                false,
                                depth + 1,
                                &val_indent,
                            );
                            let val =
                                append_own_comment_line(val, field_comments.own(), &val_indent);
                            // `&x` goes before the key's own comment, not
                            // after it: a `#` runs to end of line, so the
                            // reverse order would bury the anchor inside the
                            // comment text (#763).
                            format!("{indent}{key}:{anchor}{key_comment_suffix}\n{val}")
                        } else if let Some(kc) = comments.key_comment_if_value_absent(k) {
                            // The deferred value materialized as nothing at
                            // all - the key's own comment stands alone with
                            // no value token, matching real yq (#765). An
                            // anchor on that absent value still writes
                            // (`? k # c\n: &anc` -> `k: &anc # c`), the DOM
                            // twin of `write_deferred_value`'s own rule
                            // (#1077/#1113).
                            format!("{indent}{key}:{anchor} {kc}")
                        } else {
                            let val = emit_yaml_value_at_depth(
                                v,
                                field_comments,
                                config,
                                &val_indent,
                                false,
                                depth + 1,
                                &val_indent,
                            );
                            // The value's own comment takes priority; fall
                            // back to the key's own comment when the value
                            // has none - covers an explicit key's trailing
                            // comment (`? k # key comment\n: v\n`), which
                            // otherwise has no write site once key and
                            // value collapse onto one output line (#795).
                            // A no-op for the ordinary implicit-key case
                            // (`comment_suffix` is already non-empty then).
                            let comment_suffix = if comment_suffix.is_empty() {
                                comments
                                    .key_comment(k)
                                    .map_or_else(String::new, |c| format!(" {c}"))
                            } else {
                                comment_suffix
                            };
                            format!("{indent}{key}:{anchor} {val}{comment_suffix}")
                        }
                    })
                    .collect();
                items.join("\n")
            }
        }
    }
}

/// Quote a YAML string if needed.
fn yaml_quote_string(s: &str) -> String {
    // Check if string needs quoting
    if s.is_empty() {
        return "''".to_string();
    }

    // Check for special YAML values that need quoting
    let lower = s.to_lowercase();
    let needs_quoting = lower == "null"
        || lower == "true"
        || lower == "false"
        || lower == "~"
        || lower == ".nan"
        || lower == ".inf"
        || lower == "-.inf"
        || s.parse::<f64>().is_ok()
        || s.starts_with('*')
        || s.starts_with('&')
        || s.starts_with('!')
        || s.starts_with('%')
        || s.starts_with('@')
        || s.starts_with('`')
        || s.starts_with('|')
        || s.starts_with('>')
        || s.starts_with('[')
        || s.starts_with('{')
        || s.starts_with('"')
        || s.starts_with('\'')
        || s.starts_with('#')
        || s.starts_with('-') && (s.len() == 1 || s.chars().nth(1) == Some(' '))
        || s.starts_with('?') && (s.len() == 1 || s.chars().nth(1) == Some(' '))
        || s.starts_with(':') && (s.len() == 1 || s.chars().nth(1) == Some(' '))
        || s.contains(": ")
        || s.contains(" #")
        || s.contains('\n')
        || s.contains('\r')
        || s.contains('\t')
        || s.ends_with(':')
        || s.ends_with(' ');

    if needs_quoting {
        yaml_double_quote_escaped(s)
    } else {
        s.to_string()
    }
}

/// Double-quote `s`, escaping as needed — the actual quoting mechanics
/// [`yaml_quote_string`]'s heuristic falls back to whenever it decides
/// quoting is required, factored out so [`yaml_quote_string_with_style`]
/// can also reach it directly to *force* double-quote style regardless of
/// whether the heuristic alone would have required it (#739).
fn yaml_double_quote_escaped(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 2);
    result.push('"');
    for c in s.chars() {
        match c {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            c if c.is_ascii_control() => {
                result.push_str(&format!("\\x{:02x}", c as u32));
            }
            _ => result.push(c),
        }
    }
    result.push('"');
    result
}

/// Single-quote `s`, doubling embedded single quotes per YAML's escaping
/// rule. Only meaningful when [`can_single_quote`] agrees this is safe (a
/// single-quoted flow scalar has no escape sequence for control
/// characters).
fn yaml_single_quote_escaped(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 2);
    result.push('\'');
    for c in s.chars() {
        if c == '\'' {
            result.push_str("''");
        } else {
            result.push(c);
        }
    }
    result.push('\'');
    result
}

/// Whether `s` can round-trip through single-quote style at all — a
/// single-quoted YAML scalar has no escape syntax for control characters
/// (no `\n`, `\t`, ...), unlike double-quoted.
fn can_single_quote(s: &str) -> bool {
    !s.chars().any(|c| c.is_ascii_control())
}

/// Quote a YAML string the way [`yaml_quote_string`] does, except honoring
/// a known original style (`"single"`/`"double"`, from [`CommentTree`]'s
/// per-node style — see [`CommentTree::style`]) when there is one and it's
/// safe to reproduce, instead of always falling back to the plain-or-
/// double-quote heuristic. This is what makes an untouched sibling of a
/// write keep its original quote style rather than losing it entirely —
/// `yaml_quote_string`'s heuristic alone only adds quotes where structurally
/// *required*, which is not the same as matching what the source actually
/// wrote (#739's `'single'` repro needs quotes at all, not just safe ones).
///
/// Any other style (`""`, `"flow"`, `"literal"`, `"folded"` — the last two
/// are block-scalar styles this DOM writer doesn't reproduce; see
/// `CommentTree`'s own doc comment) falls back to the plain heuristic
/// unchanged.
fn yaml_quote_string_with_style(s: &str, style: &str) -> String {
    // No empty-string special case needed here (unlike `yaml_quote_string`
    // below): every arm already renders `""` correctly on its own -
    // `yaml_double_quote_escaped`/`yaml_single_quote_escaped` produce
    // `""`/`''` for an empty `s`, and the `_` fallback defers to
    // `yaml_quote_string`, which has its own empty-string case. A
    // short-circuit here that always returned `''` regardless of `style`
    // used to flip an untouched double-quoted empty string to single-quote
    // style on a sibling write (found in review).
    match style {
        "single" if can_single_quote(s) => yaml_single_quote_escaped(s),
        "double" => yaml_double_quote_escaped(s),
        _ => yaml_quote_string(s),
    }
}

/// Quote a YAML key if needed.
fn yaml_quote_key(s: &str) -> String {
    // Keys have similar rules but are a bit more permissive
    if s.is_empty() {
        return "''".to_string();
    }

    let needs_quoting = s.contains(':')
        || s.contains('#')
        || s.contains('\n')
        || s.contains('\r')
        || s.starts_with('-')
        || s.starts_with('?')
        || s.starts_with('[')
        || s.starts_with('{')
        || s.starts_with('"')
        || s.starts_with('\'')
        || s.starts_with('*')
        || s.starts_with('&')
        || s.starts_with('!')
        || s.ends_with(' ');

    if needs_quoting {
        let mut result = String::with_capacity(s.len() + 2);
        result.push('"');
        for c in s.chars() {
            match c {
                '"' => result.push_str("\\\""),
                '\\' => result.push_str("\\\\"),
                '\n' => result.push_str("\\n"),
                '\r' => result.push_str("\\r"),
                '\t' => result.push_str("\\t"),
                _ => result.push(c),
            }
        }
        result.push('"');
        result
    } else {
        s.to_string()
    }
}

/// Whether a compact block-sequence item's remaining source (the text
/// right after `- `, as returned by `Chars::as_str()` at that point) opens
/// with a mapping key rather than a scalar value — i.e. whether an
/// unquoted `:` appears before the line ends.
///
/// `colorize_yaml`'s state machine colors a token *as it's written*, one
/// `char` at a time, with no ability to go back and re-color something it
/// already emitted — so at the position right after `- `, it has to decide
/// up front whether what follows is a key (color it) or a value (don't),
/// and the only way to tell is to look ahead. Before #785, `- ` was always
/// followed by either a scalar value or a newline (a container value's
/// mapping always started on its own line), so this ambiguity never arose.
/// A nested compact marker (`- - 1`) recurses naturally: the caller
/// re-invokes this same lookahead for the *inner* `-` too, so a mapping
/// arbitrarily many `- ` markers deep (`- - a: 1`) still finds its `:` and
/// colors `a`, while a purely scalar nested sequence (`- - 1\n  - 2`) does
/// not color the inner marker - real, but narrow and, like the rest of
/// this colorizer, not oracle-matched against real yq's own (differently
/// coded) `-C` output, so left as a known residual gap rather than chased
/// further (#785).
fn compact_item_opens_with_key(rest_of_line: &str) -> bool {
    let mut quote: Option<char> = None;
    let mut chars = rest_of_line.chars();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                if c == '\\' {
                    chars.next(); // Skip the escaped character.
                } else if c == q {
                    quote = None;
                }
            }
            None => match c {
                '\n' => return false,
                '"' | '\'' => quote = Some(c),
                ':' => return true,
                _ => {}
            },
        }
    }
    false
}

/// Colorize YAML output (basic ANSI colors).
///
/// `boundaries` are byte offsets into `yaml`, recorded by
/// [`ColorSink::record_boundary`] instead of writing a real terminator
/// character into the buffer (#1708): at each one, this closes whatever
/// color span is still open at that exact position first, then writes the
/// real terminator -- rather than leaving it for the unconditional trailing
/// reset below to close, which is what put the terminator *inside* the
/// color span in the first place. Driving this off recorded positions
/// rather than an in-band marker character avoids having to pick a marker
/// guaranteed absent from real content -- this crate's own writers can and
/// do emit any raw byte, including a chosen marker's own value, when the
/// source YAML/JSON explicitly encodes one. A caller with nothing to mark
/// (the identity/DOM paths, which write their terminator directly outside
/// any buffer) passes an empty slice, and `yaml` is colorized exactly as
/// before.
// A per-statement `#[allow]` on a `close_and_terminate!()` invocation below
// doesn't reach the assignments the macro expands to (rustc: "will be
// ignored, since it's applied to the macro invocation"), so this covers the
// whole function instead.
#[allow(unused_assignments)] // STYLE-0004: close_and_terminate!'s state
                             // resets serve its two call sites asymmetrically -- `at_key_start`/
                             // `escape_next` are read by the main loop below but dead at the trailing
                             // drain (nothing reads them after this function returns), while
                             // `trailing_reset_needed` is read by the final check below but dead inside
                             // the main loop (unconditionally overwritten by the very next line there).
                             // Sharing one macro between both sites, rather than hand-duplicating this
                             // close/terminate logic, is worth the resulting dead stores.
fn colorize_yaml(yaml: &str, terminator: Terminator, boundaries: &[usize]) -> String {
    let mut result = String::with_capacity(yaml.len() * 2 + boundaries.len() * 5);
    let mut in_string = false;
    let mut escape_next = false;
    let mut at_key_start = true;
    let mut in_key = false;
    let mut boundary_idx = 0;
    let mut byte_pos = 0usize;
    // Whether the unconditional trailing reset below is still needed.
    // `close_and_terminate!` (a boundary was just closed) clears it; any
    // subsequently pushed content sets it again. Stays `true` throughout
    // when `boundaries` is empty (the identity/DOM callers), so behaves
    // exactly as before this mechanism existed for them.
    let mut trailing_reset_needed = true;

    // One recorded boundary per streamed result, so this correctly places
    // every terminator in a multi-result stream, not just the last one --
    // and closes both spans a boundary can land inside (a still-open key
    // *or* string color), not just `in_key`, so a leftover open span from
    // one result can never bleed its missing reset into the next. A single
    // definition (rather than duplicating this at both call sites below)
    // keeps the in-loop and post-loop drains from drifting apart on a
    // future edit.
    macro_rules! close_and_terminate {
        () => {
            if in_key || in_string {
                result.push_str("\x1b[0m");
                in_key = false;
                in_string = false;
            }
            let _ = terminator.write_fmt(&mut result);
            at_key_start = true;
            escape_next = false;
            trailing_reset_needed = false;
        };
    }

    let mut chars = yaml.chars();
    while let Some(c) = chars.next() {
        while boundary_idx < boundaries.len() && boundaries[boundary_idx] <= byte_pos {
            close_and_terminate!();
            boundary_idx += 1;
        }
        // Real content follows -- if a boundary just fired above, its
        // terminator is no longer the buffer's final byte, so the trailing
        // safety-net reset is needed again.
        trailing_reset_needed = true;

        if escape_next {
            result.push(c);
            escape_next = false;
            byte_pos += c.len_utf8();
            continue;
        }

        if c == '\\' && (in_string || in_key) {
            result.push(c);
            escape_next = true;
            byte_pos += c.len_utf8();
            continue;
        }

        match c {
            '"' | '\'' => {
                if in_string {
                    result.push(c);
                    result.push_str("\x1b[0m");
                    in_string = false;
                } else {
                    result.push_str("\x1b[32m"); // Green for strings
                    result.push(c);
                    in_string = true;
                }
                at_key_start = false;
            }
            ':' if !in_string => {
                // Only close a color span that's actually open (`in_key`)
                // - otherwise this `:` belongs to a value that was never
                // colored (e.g. a quoted-key's colon after the closing
                // quote already reset color, or a `:` inside an uncolored
                // token), and unconditionally emitting a reset here would
                // write an orphaned `\x1b[0m` with no matching open.
                if in_key {
                    result.push_str("\x1b[0m");
                }
                result.push(c);
                in_key = false;
                at_key_start = false;
            }
            '\n' => {
                result.push(c);
                at_key_start = true;
                in_key = false;
            }
            '-' if at_key_start => {
                result.push_str("\x1b[33m"); // Yellow for list markers
                result.push(c);
                result.push_str("\x1b[0m");
                // A real-yq "compact" block-sequence item (`- key: value`,
                // #785) puts its mapping's first key directly after `- `
                // on the same line, instead of on a fresh line (which
                // already re-triggers `at_key_start` via the `\n` arm
                // above) - see `compact_item_opens_with_key`'s doc comment.
                at_key_start = compact_item_opens_with_key(chars.as_str());
            }
            _ if at_key_start && !c.is_whitespace() && !in_string => {
                result.push_str("\x1b[36m"); // Cyan for keys
                result.push(c);
                in_key = true;
                at_key_start = false;
            }
            _ => {
                result.push(c);
                if !c.is_whitespace() {
                    at_key_start = false;
                }
            }
        }
        byte_pos += c.len_utf8();
    }
    while boundary_idx < boundaries.len() {
        // A boundary at (or past) the very end of `yaml` -- e.g. the last
        // streamed result's terminator, with no further content after it.
        close_and_terminate!();
        boundary_idx += 1;
    }
    // Skipped when the buffer's last write was already a boundary's own
    // terminator (the ordinary case for every M2 evaluated-color result) --
    // otherwise this would place a redundant reset *after* that terminator,
    // the same "reset must precede terminator" defect #1708 was filed for,
    // just relocated to the tail of the buffer instead of the middle.
    if trailing_reset_needed {
        result.push_str("\x1b[0m");
    }
    result
}

/// Parse a `--argjson` value into an `OwnedValue`.
///
/// Validates strictly (RFC 8259) via `serde_json`, matching jq's own
/// `--argjson` validation *strategy* (see `jq_runner::parse_json_value`).
/// The lenient JSON semi-index would otherwise silently coerce malformed
/// input (e.g. `42 garbage` → `42`) instead of surfacing an error (#284).
///
/// Unlike `jq_runner::parse_json_value`, this does *not* preserve a number
/// literal's exact source spelling (#1058 fixed that for `succinctly jq`,
/// deliberately not for `succinctly yq`) -- real mikefarah/yq has no
/// `--argjson` flag at all, so there's no oracle to match on fidelity
/// specifically, and yq's own `--input-format json` path already discards
/// JSON-sourced number fidelity on purpose (#978,
/// `to_owned_canonicalizing_numbers`). Adding fidelity only here would make
/// `--argjson` *inconsistent* with that established convention rather than
/// fix a real divergence.
fn parse_json_value(s: &str) -> Result<OwnedValue> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(OwnedValue::Null);
    }
    let value: serde_json::Value =
        serde_json::from_str(s).with_context(|| format!("invalid JSON: {s}"))?;
    Ok(serde_json_to_owned(&value))
}

/// Convert a `serde_json::Value` into an `OwnedValue`.
fn serde_json_to_owned(value: &serde_json::Value) -> OwnedValue {
    serde_json_to_owned_at_depth(value, 0)
}

/// Panics past `succinctly::jq::MAX_VALUE_TREE_DEPTH` levels of nesting
/// (#1017). `serde_json::from_str`'s own `Deserializer` already enforces
/// an independent ~128-deep parse-time limit before `value` can exist at
/// all, so this is defense-in-depth against that upstream limit changing,
/// not a currently-live independent crash path.
fn serde_json_to_owned_at_depth(value: &serde_json::Value, depth: usize) -> OwnedValue {
    assert_value_tree_depth(depth);
    match value {
        serde_json::Value::Null => OwnedValue::Null,
        serde_json::Value::Bool(b) => OwnedValue::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                OwnedValue::Int(i)
            } else if let Some(f) = n.as_f64() {
                OwnedValue::Float(f)
            } else {
                OwnedValue::Null
            }
        }
        serde_json::Value::String(s) => OwnedValue::String(s.clone()),
        serde_json::Value::Array(arr) => OwnedValue::Array(
            arr.iter()
                .map(|v| serde_json_to_owned_at_depth(v, depth + 1))
                .collect(),
        ),
        serde_json::Value::Object(obj) => OwnedValue::Object(
            obj.iter()
                .map(|(k, v)| (k.clone(), serde_json_to_owned_at_depth(v, depth + 1)))
                .collect(),
        ),
    }
}

/// Parse variables from command line arguments.
fn parse_variables(args: &YqCommand) -> Result<EvalContext> {
    let mut context = EvalContext::default();

    // Process --arg (string values)
    for chunk in args.arg.chunks(2) {
        if chunk.len() == 2 {
            let name = chunk[0].clone();
            let value = OwnedValue::String(chunk[1].clone());
            context.named.insert(name, value);
        }
    }

    // Process --argjson (JSON values)
    for chunk in args.argjson.chunks(2) {
        if chunk.len() == 2 {
            let name = chunk[0].clone();
            let value = parse_json_value(&chunk[1])
                .with_context(|| format!("invalid JSON for --argjson {name}"))?;
            context.named.insert(name, value);
        }
    }

    Ok(context)
}

/// Build the `$ARGS` special variable (`{named, positional}`), mirroring
/// `jq_runner::build_args_var`.
///
/// yq has no positional-argument flags (`--args`/`--jsonargs`), so `positional`
/// is always an empty array; `named` carries the `--arg`/`--argjson` values.
fn build_args_var(context: &EvalContext) -> OwnedValue {
    let mut args_obj = IndexMap::new();
    args_obj.insert(
        "named".to_string(),
        OwnedValue::Object(context.named.clone()),
    );
    args_obj.insert("positional".to_string(), OwnedValue::Array(Vec::new()));
    OwnedValue::Object(args_obj)
}

/// Check if an expression can use M2 streaming path.
///
/// M2 streaming is used for simple navigation expressions that produce
/// cursor results without requiring OwnedValue construction:
/// - Identity: `.`
/// - Field access: `.field`
/// - Index access: `.[0]`, `.[-1]`
/// - Iteration: `.[]`
/// - Chained navigation: `.field[0].name`
/// - Optional variants: `.field?`, `.[0]?`, `.[]?`
/// - `keys_unsorted` (streams lazily via `GenericResult::LazyKeys { sorted: false, .. }`, #685)
///
/// Expressions that require OwnedValue construction cannot use M2:
/// - Builtins like `length`, `keys` (sorted), `map`
/// - Array/object construction: `[...]`, `{...}`
/// - Arithmetic, comparison, and logic operators
/// - String interpolation
/// - Variables and function calls
fn can_use_m2_streaming(expr: &Expr) -> bool {
    match expr {
        // Core M2 expressions
        Expr::Identity => true,
        Expr::Field(_) => true,
        Expr::Index(_) | Expr::IndexNumber { .. } => true,
        Expr::Iterate => true,

        // Chained navigation
        Expr::Pipe(exprs) => exprs.iter().all(can_use_m2_streaming),

        // Optional variants
        Expr::Optional(inner) => can_use_m2_streaming(inner),

        // Parentheses don't affect streamability
        Expr::Paren(inner) => can_use_m2_streaming(inner),

        // first(f)/last(f) (both AST spellings the parser produces, see
        // `Expr::FirstExpr`/`LastExpr` doc comments) thread a cursor through
        // natively in `eval_generic.rs` (#607) *only when `f` itself does* --
        // `first(.[])` streams a `GenericResult` cursor exactly like plain
        // navigation, but `first(.a * 1e100)` still has to materialize an
        // `OwnedValue::Float` from the arithmetic, which then needs the DOM
        // path's yq-mode scientific-notation formatting (#997) rather than
        // the M2 fast writers in `src/jq/stream.rs`, which don't have it.
        // Recursing here (like `Pipe`/`Optional`/`Paren` above) restricts the
        // fast path to exactly the inner shapes it can actually stream a
        // cursor for. Streaming through `eval_with_cursor_using` for the
        // eligible cases (rather than `evaluate_yaml_cursor`'s unconditional
        // `to_owned()` DOM path) is also what keeps duplicate mapping keys
        // intact for these shapes, matching `.[0]` on the same input (#631).
        Expr::FirstExpr(inner) | Expr::LastExpr(inner) => can_use_m2_streaming(inner),
        Expr::Builtin(Builtin::FirstStream(inner) | Builtin::LastStream(inner)) => {
            can_use_m2_streaming(inner)
        }

        // Same reasoning as `FirstExpr`/`LastExpr` above, now that #1607
        // gave `Expr::Limit`/`Builtin::NthStream` (the arm real
        // `nth(n; expr)` calls reach) their own native, cursor-threading
        // arms in `eval_generic.rs`: `limit(3; .[])`/`nth(0; .[])` stream a
        // `GenericResult` cursor exactly like plain navigation when `expr`
        // itself does, so route them through `eval_with_cursor_using`
        // here too rather than `evaluate_yaml_cursor`'s unconditional
        // `to_owned()` DOM path -- otherwise #1607's own fix is discarded
        // one layer up: a correctly cursor-preserving `GenericResult`
        // still gets flattened into an `IndexMap`-backed `OwnedValue` the
        // moment `evaluate_yaml_cursor` materializes it for output,
        // silently re-losing a duplicate key *inside* the captured item
        // (not the `limit`/`nth` walk itself, which #1607 already fixed
        // regardless of this gate). `n` is never recursed into: it's
        // always evaluated as a single control value, never streamed.
        Expr::Limit { n: _, expr }
        | Expr::NthExpr { n: _, expr }
        | Expr::Builtin(Builtin::NthStream(_, expr)) => can_use_m2_streaming(expr),
        Expr::IndexExpr { .. } => true,

        // `select(...)` never changes position - a truthy output is always
        // the input node unchanged - and `eval_generic.rs`'s own
        // `Builtin::Select` arm already forwards the incoming cursor as-is
        // (`OneCursor`/`ManyCursor`) rather than rebuilding a value. Routing
        // it here rather than through `evaluate_yaml_cursor`'s unconditional
        // `to_owned()` DOM path is what keeps duplicate mapping keys (and
        // their comments) intact, matching `FirstExpr`/`LastExpr` above
        // (#631) and `-S`/`--tab` (#733) - `select()` had the same latent
        // gap (#796).
        Expr::Builtin(Builtin::Select(_)) => true,

        // `keys_unsorted` on a mapping produces `GenericResult::LazyKeys { sorted: false, .. }`,
        // which `GenericResult::stream_json`/`stream_yaml` now stream directly
        // from the field cursor (#685) instead of materializing a `Vec<String>`
        // first. On an array input it already returns `GenericResult::Owned`
        // cheaply, so this only changes routing for the mapping case.
        Expr::Builtin(Builtin::KeysUnsorted) => true,

        // `map(f)` on a container produces `GenericResult::LazySeq` (#724,
        // #725), which `GenericResult::stream_json`/`stream_yaml` now render
        // one element at a time from each element's own live cursor (#757)
        // rather than through an `OwnedValue::Array`. That is both the
        // performance point and a fidelity fix: routing `map` through the DOM
        // path collapsed duplicate mapping keys and dropped comments,
        // anchors/aliases and flow style, all of which real yq keeps
        // (verified live against v4.53.3 — `map(.)` on `- a: 1`/`  a: 2`
        // prints both keys there, and on `- {a: 1, b: 2}` keeps flow style).
        // `.[]` on the same inputs already matched, because it already
        // streamed.
        //
        // Recursing into `f` rather than answering a flat `true` follows
        // `FirstExpr`/`LastExpr` above, for the same reason: a *computing*
        // body materializes an `OwnedValue::Float` that needs the DOM path's
        // yq-mode scientific-notation/decimal-point formatting (#997, #949,
        // #1090), which the M2 streamers don't have. So `map(.)`,
        // `map(.name)`, `map(.a.b)` and `map(select(...))` stream; `map(.+1)`
        // and `map(length)` keep the DOM path exactly as before.
        Expr::Builtin(Builtin::Map(f)) => can_use_m2_streaming(f),

        // Everything else requires OwnedValue
        _ => false,
    }
}

/// Whether `expr` mentions the `split_doc` builtin anywhere.
///
/// Decides whether output uses per-result document separators.
///
/// Was 199 lines of hand-written traversal, exhaustive over `Expr` but ending
/// its inner `Builtin` match in `_ => false` -- so twelve sub-expression-
/// carrying builtins were treated as leaves and `split_doc` hidden inside one
/// of them went unreported. `jq::walk::builtin_kids` has no wildcard arm, so
/// that class of miss is now a compile error rather than a silent wrong
/// answer (#1309).
fn contains_split_doc(expr: &Expr) -> bool {
    jq::walk::contains_builtin(expr, |b| matches!(b, Builtin::SplitDoc))
}

/// Get input files from arguments.
fn get_input_files(args: &YqCommand) -> Vec<String> {
    // When --from-file is used, the 'filter' field becomes the first input file
    // because the filter comes from a file instead of command line
    let mut files = Vec::new();

    if args.from_file.is_some() {
        // When --from-file is used, the first positional arg (if any) is an input file
        if let Some(ref first_file) = args.filter {
            files.push(first_file.clone());
        }
    }

    // Add remaining files
    files.extend(args.files.iter().cloned());

    files
}

/// Main entry point for the yq command.
pub fn run_yq(args: YqCommand) -> Result<i32> {
    // Handle --version
    if args.version {
        println!("succinctly-yq {}", env!("CARGO_PKG_VERSION"));
        return Ok(exit_codes::SUCCESS);
    }

    // Handle --build-configuration
    if args.build_configuration {
        output::print_build_configuration("yq");
        return Ok(exit_codes::SUCCESS);
    }

    // Get the filter expression
    let filter_str = if let Some(ref path) = args.from_file {
        std::fs::read_to_string(path)
            .with_context(|| format!("failed to read filter file: {}", path.display()))?
    } else {
        args.filter.clone().unwrap_or_else(|| ".".to_string())
    };

    // Get input files (with --from-file, the filter positional is an input file)
    let input_files = get_input_files(&args);

    // Validate flag compatibility
    if args.document.is_some() && args.raw_input {
        anyhow::bail!("--doc and --raw-input are incompatible");
    }

    if args.front_matter.is_some() {
        if args.document.is_some() {
            anyhow::bail!("--front-matter and --doc are incompatible");
        }
        if args.null_input {
            anyhow::bail!("--front-matter and --null-input are incompatible");
        }
        if args.raw_input {
            anyhow::bail!("--front-matter and --raw-input are incompatible");
        }
        if args.input_format == InputFormat::Json {
            // Front matter is YAML by definition (the `---`-fenced header),
            // so `apply_front_matter` always forces `InputFormat::Yaml` once
            // a mode is set -- reject an explicit, contradictory
            // `--input-format json` instead of silently overriding it.
            anyhow::bail!("--front-matter and --input-format json are incompatible");
        }
        if args.front_matter == Some(FrontMatterMode::Extract) && args.inplace {
            // `extract` never captures a body to reattach (only `process`
            // does, see `apply_front_matter`), so `--inplace` would
            // overwrite the file with just the transformed front matter,
            // silently discarding everything after the closing fence.
            anyhow::bail!(
                "--front-matter=extract and --inplace are incompatible (would discard the file's body); use --front-matter=process instead"
            );
        }
        if args.front_matter == Some(FrontMatterMode::Process) {
            if args.slurp {
                anyhow::bail!("--front-matter=process and --slurp are incompatible");
            }
            // Only explicit `Json` is rejected now (#1493): front-matter
            // content is always forced to `InputFormat::Yaml`
            // (`apply_front_matter`), so `Auto` always resolves to `Yaml`
            // here too (`Self::for_source`) -- accepting it is no longer
            // the "silently slips through and wraps a JSON body in `---`
            // fences" bug this guard was written to close, since `Auto`
            // can no longer resolve to anything but `Yaml` in this
            // specific context.
            if args.output_format == OutputFormat::Json {
                anyhow::bail!(
                    "--front-matter=process requires YAML output (got -o/--output-format json)"
                );
            }
        }
    }

    if args.split_exp.is_some() {
        if args.slurp {
            anyhow::bail!("--split-exp and --slurp are incompatible");
        }
        if args.inplace {
            anyhow::bail!("--split-exp and --inplace are incompatible");
        }
        if args.raw_input {
            anyhow::bail!("--split-exp with --raw-input is not yet supported");
        }
        if args.front_matter.is_some() {
            anyhow::bail!("--split-exp and --front-matter are incompatible");
        }
    }

    if args.eval_all {
        if args.slurp {
            anyhow::bail!(
                "--eval-all and --slurp are incompatible: both combine inputs into a single evaluation"
            );
        }
        if args.inplace {
            anyhow::bail!("--eval-all and --inplace are incompatible");
        }
        if args.raw_input {
            anyhow::bail!("--eval-all and --raw-input are incompatible");
        }
        if args.split_exp.is_some() {
            anyhow::bail!("--eval-all and --split-exp are incompatible");
        }
        if args.front_matter.is_some() {
            anyhow::bail!("--eval-all and --front-matter are incompatible");
        }
    }

    // Parse the jq program (use Yq mode for extended identifier syntax like kebab-case)
    let mut program = jq::parse_program_with_mode_and_extensions(
        &filter_str,
        jq::ParserMode::Yq,
        args.jq_extensions,
    )
    .map_err(|e| anyhow::anyhow!("parse error: {e}"))?;

    // Parse variables
    let context = parse_variables(&args)?;

    // Substitute named variables (--arg/--argjson) and the $ARGS special
    // variable into the expression AST before evaluating, mirroring the jq
    // runner (see jq_runner.rs). Without this, filter references like `$g`
    // error as "undefined variable" even though the values were parsed (#284).
    let args_value = build_args_var(&context);
    let mut all_vars: Vec<(&str, &OwnedValue)> =
        context.named.iter().map(|(k, v)| (k.as_str(), v)).collect();
    all_vars.push(("ARGS", &args_value));
    program.expr = jq::substitute_vars(&program.expr, all_vars.iter().copied());

    // #1473: same compile-time function resolution the jq runner does, before
    // any document is read.
    //
    // Real yq has no `def` at all — its lexer rejects `def f: 42; f` outright
    // (`invalid input text`, verified against v4.53.3) — so succinctly's `def`
    // support is an extension here and ADR-0018's reference-fidelity rule sets
    // no expectation to match. Rejecting an unresolvable call up front is the
    // directionally yq-ish answer anyway, since yq rejects an unknown
    // identifier at lex time; the wording and exit code stay yq's own uniform
    // `Error: …` / 1, not jq's compile-error shape.
    //
    // No `ModuleLoader` here (yq has no module system), so unlike the jq
    // runner there is nothing that must run first; the only ordering
    // constraint is that `--jq-extensions` gating already happened, which it
    // did — a gated jq-only builtin is a *parse* error above, never a residual
    // `FuncCall` reaching this pass.
    if let Err(unresolved) = jq::resolve_func_calls(&program.expr) {
        anyhow::bail!("{unresolved}");
    }

    // Parse the --split-exp expression, if given, once up front, applying
    // the same --arg/--argjson/$ARGS substitution as the main filter --
    // otherwise a filename expression referencing `--arg`-provided values
    // (e.g. an output directory prefix) fails as an undefined variable even
    // though the same flag works for the main filter. `$index` is bound
    // separately, per output result (see `write_split_result`).
    let split_expr: Option<Expr> = args
        .split_exp
        .as_deref()
        .map(|s| {
            let expr = jq::parse_program_with_mode_and_extensions(
                s,
                jq::ParserMode::Yq,
                args.jq_extensions,
            )
            .map(|p| jq::substitute_vars(&p.expr, all_vars.iter().copied()))
            .map_err(|e| anyhow::anyhow!("parse error in --split-exp expression: {e}"))?;
            // #1473: the filename expression is a second, independently parsed
            // program, so it needs its own resolution pass -- the main
            // filter's says nothing about it.
            if let Err(unresolved) = jq::resolve_func_calls(&expr) {
                anyhow::bail!("{unresolved} in --split-exp expression");
            }
            Ok(expr)
        })
        .transpose()?;

    // Output configuration
    let output_config = OutputConfig::from_args(&args);

    // Set up output
    let stdout = std::io::stdout();
    let mut writer = LoudFlushWriter::new(stdout.lock());

    // yq --exit-status semantics: exit 1 unless some result is truthy.
    // Unlike jq (which inspects only the last output value), yq treats empty
    // output and all-falsy output alike as "no matches found".
    let mut any_truthy = false;

    // #1564: set by the default path / --split-exp when
    // `gather_input_sources` finds a later file that fails `--validate` --
    // rather than bailing immediately (losing the earlier files' already-
    // valid output, the bug this exists to fix), those two branches still
    // process/emit whatever passed and defer reporting the failure's exit
    // code until the shared tail below, after that output is written.
    let mut deferred_validate_exit_code: Option<i32> = None;

    // Uncaught evaluation errors. Evaluation continues past one, so the failure
    // is remembered here and turned into yq's exit 1 below (#355).
    let mut sink = ErrorSink::default();
    // #1615: set when a streamed *identity* render was cut off part-way
    // through a document by a decode failure. Only the identity/P9 branches
    // can leave the output mid-document: they stream a container's structure
    // directly, so `a: 1\nb: ` is already written by the time the bad scalar
    // is reached, with no newline to close it. The *evaluated* branches
    // cannot -- `GenericResult`'s streamers report through `StreamStats`
    // without emitting a partial scalar, which is why an evaluated filter
    // still continues to the next document (#355) and only this flag stops
    // the loop. Declared here, before `stream_cursor!`, because a bare
    // identifier inside a `macro_rules!` resolves against definition-site
    // scope -- the same reason `sink` itself lives out here.
    // A `Cell`, not a plain `bool`: `stream_cursor!` expands in several
    // places where nothing downstream reads the flag (the `--inplace`
    // single-document arm has no loop to break out of), and a plain
    // assignment there is a dead store -- `unused_assignments` fires, which
    // `-D warnings` turns into a CI failure. `Cell::set` is a method call, so
    // the lint does not apply, and the flag stays one shared value.
    let stream_truncated = core::cell::Cell::new(false);

    // Check if expression contains split_doc - if so, each result is a separate document
    let has_split_doc = contains_split_doc(&program.expr);

    // M2/M2.5 streaming fast path: navigation queries stream results
    // directly from the document cursor, skipping the OwnedValue DOM (and
    // its IndexMap, which cannot represent duplicate mapping keys — #442).
    // Compact output always qualified; indented (pretty) output now does
    // too, since both the YAML- and JSON-target streamers are indent-aware.
    // This avoids building OwnedValue DOM for:
    // - Identity: `.`
    // - Field access: `.field`
    // - Index access: `.[0]`
    // - Iteration: `.[]`
    // - Chained navigation: `.field[0].name`
    //
    // Supports both JSON and YAML output formats.
    let is_identity = matches!(program.expr, Expr::Identity);
    // #978: `args.input_format` is the *raw*, unresolved CLI flag - it's
    // `Auto` whenever the user relies on `.json`-extension detection
    // instead of typing `--input-format json` explicitly, which
    // `resolve_input_format` (used everywhere else JSON input is handled)
    // still correctly resolves to `Json`. `mark_json_sourced_indices`
    // below needs that same resolution, not the raw flag, or a bare
    // `succinctly yq '.' file.json` (arguably the more common way to hit
    // this than the explicit flag) would still leak the #978 bug back in.
    //
    // #978 originally disabled M2 streaming outright for JSON input here
    // (`is_m2_streamable` used to `&& !any_input_is_json`), trading away
    // M2's performance *and* its incidental duplicate-key preservation
    // (#996) to fix a real bug: neither M2 streamer discarded a JSON
    // number's own decimal-point/exponent spelling the way the DOM path's
    // `to_owned_canonicalizing_numbers` does (`1.50` stayed `1.50`, `1e2` became
    // `100.0` instead of real yq's `100`). #996 fixed that at the source
    // instead — `YamlIndex::mark_json_sourced` (set below, right after
    // each M2-path `YamlIndex::build` call) makes `src/yaml/light.rs`'s
    // streaming formatters canonicalize a `Float` the same way the DOM
    // path already does, so M2 no longer needs to be disabled for JSON
    // input at all: it now gets *both* correct numbers and duplicate-key
    // preservation, matching real yq on both counts.
    let is_m2_streamable = can_use_m2_streaming(&program.expr);
    // pretty_print isn't implemented by the cursor streamers, so it still
    // falls back to the DOM path, unchanged, rather than silently ignoring
    // the flag the way compact mode already does today. `sort_keys` and
    // `tab` (#733) are implemented directly by the cursor/lazy streamers —
    // see `IndentSpec` and the `sort_keys` parameter threaded through
    // `DocumentCursor::stream_json`/`stream_yaml` and `GenericResult::
    // stream_json`/`stream_yaml` — so routing them through the DOM would
    // needlessly reintroduce #442's duplicate-mapping-key collapse
    // (`OwnedValue::Object`'s `IndexMap` cannot represent duplicate keys).
    // pretty_print's DOM-path rendering is currently indistinguishable from
    // the default (style preservation doesn't exist yet — #707); routing it
    // through DOM now gives it a single seam to implement real
    // style-clearing against once #707 lands (#705).
    //
    // `can_stream_pretty_or_colored` gates every M2 fast path below,
    // stdout, `--slurp`, and `--inplace` alike: color, when requested, is
    // handled by `stream_maybe_colored` buffering the (still
    // duplicate-key-safe) cursor-streamed output and running it through the
    // existing `colorize_yaml`/`colorize_json` post-processors, rather than
    // falling back to the DOM/IndexMap path that would collapse duplicate
    // mapping keys (#442, #748, #809).
    let can_stream_pretty_or_colored = !args.pretty_print;
    // `--ascii-output` no longer appears here (#1700). It used to gate the
    // whole disjunction (#1693) because no M2 JSON streamer could escape
    // non-ASCII, leaving the DOM path (`output_value`) as the only correct
    // renderer -- which cost the flag its duplicate mapping keys, since an
    // `OwnedValue::Object`'s `IndexMap` cannot represent them (#442/#748/
    // #809). That was a real, shipped data loss (`{"a":1,"a":2}` rendered
    // as `{"a":2}`), not merely a slower path.
    //
    // The escaping now happens at the sink instead, via `json_ascii!` at
    // every JSON write route in this file, so the streamers stay untouched
    // and `--ascii-output` keeps correct escaping and duplicate-key safety
    // at once. #1700 itself proposed threading an `ascii` flag into the
    // streamers; the sink adapter is sound for a stronger reason than any
    // such flag would be -- outside a string literal JSON's grammar admits
    // only ASCII, so every non-ASCII character in the stream is string
    // content by construction. See `AsciiEscapeWriter` (`src/jq/escape.rs`)
    // for that argument in full, and for why it also leaves
    // `stream_json_string`'s #965-sensitive SIMD scan compiled exactly as
    // before.
    //
    // Extracted into one local (rather than repeating the expression at
    // `can_json_fast_path`, `can_inplace_json_fast_path`, and
    // `can_slurp_fast_path`'s JSON arm) per /code-review on #1693's own PR:
    // `can_slurp_fast_path` originally diverged from this exact predicate
    // because #1577 copied it by hand instead of sharing it.
    // `!output_config.raw_output` (#1715): none of the M2 JSON streamers
    // consult `raw_output` at all, so `-r`/`-0`/`--join-output` (which all
    // set it, see `OutputConfig::from_args`) fail to strip JSON string
    // quoting on this path -- unlike the DOM path's `output_value`, whose
    // `if config.raw_output { if let OwnedValue::String(s) = value ... }`
    // arm handles it correctly. Excluded from fast-path eligibility here
    // rather than fixed inside the streamers themselves: the real fix needs
    // `GenericResult::stream_json`/`YamlCursor::stream_json` to special-case
    // a lone string value the same way, which is nontrivial for the
    // multi-result evaluated case (`stream_json` owns the whole per-value
    // streaming+separator logic internally) -- this is the low-risk
    // direction the issue itself names, giving up the fast path's
    // performance advantage only for this one flag combination rather than
    // risk a wrong fix to the streamers. Same reasoning YAML never needed:
    // YAML's own scalar rendering is already unquoted by default (`-r`-like
    // even without `-r`), so `can_yaml_fast_path` never had this gap.
    let can_stream_json_output_style =
        !output_config.raw_output && (output_config.compact || can_stream_pretty_or_colored);
    let can_json_fast_path = is_m2_streamable
        && can_stream_json_output_style
        && output_config.output_format == OutputFormat::Json
        && !args.null_input
        && !args.raw_input
        && !args.slurp
        && !args.inplace
        && args.front_matter.is_none()
        && split_expr.is_none()
        && !args.eval_all
        && context.named.is_empty();
    let can_yaml_fast_path = is_m2_streamable
        && (output_config.compact || can_stream_pretty_or_colored)
        && output_config.output_format == OutputFormat::Yaml
        && !args.null_input
        && !args.raw_input
        && !args.slurp
        && !args.inplace
        && args.front_matter.is_none()
        && split_expr.is_none()
        && !args.eval_all
        && context.named.is_empty();
    let can_fast_path = can_json_fast_path || can_yaml_fast_path;

    // `--inplace`'s own copy of the M2 gate (#478): identical conditions to
    // `can_json_fast_path`/`can_yaml_fast_path` above, but requiring
    // `args.inplace` instead of excluding it. Kept as a separate pair rather
    // than folding into the gate above because inplace output targets a
    // per-file buffer (then `fs::write`), not the shared stdout `writer`
    // the block below uses — the two loops have different write targets and
    // per-file `---` separator resets, so they stay as distinct branches
    // that happen to share the same underlying `stream_cursor!` macro. Color
    // no longer excludes the fast path here (#809): the inplace write loop
    // below passes `false` as `stream_cursor!`'s `$use_color` argument (and
    // shadows `output_config.use_color` to `false` for its own DOM-branch
    // writes), so `-C` reaching the fast path still never writes ANSI to
    // disk, but no longer forces a fallback to the duplicate-key-collapsing
    // DOM path either.
    // Shares `can_json_fast_path`'s `can_stream_json_output_style` above.
    let can_inplace_json_fast_path = is_m2_streamable
        && can_stream_json_output_style
        && output_config.output_format == OutputFormat::Json
        && !args.null_input
        && !args.raw_input
        && args.inplace
        && args.front_matter.is_none()
        && context.named.is_empty();
    let can_inplace_yaml_fast_path = is_m2_streamable
        && (output_config.compact || can_stream_pretty_or_colored)
        && output_config.output_format == OutputFormat::Yaml
        && !args.null_input
        && !args.raw_input
        && args.inplace
        && args.front_matter.is_none()
        && context.named.is_empty();
    let can_inplace_fast_path = can_inplace_json_fast_path || can_inplace_yaml_fast_path;

    // `--slurp`'s fast path (#478, extended to `-o=json` by #1577) is
    // narrower than the two gates above: scoped to plain identity only
    // (`is_identity`, not the broader `is_m2_streamable` set), since a
    // non-trivial filter over the slurped array needs real evaluation.
    // Color no longer excludes this gate either (#809): the call site wraps
    // `stream_yaml_sequence`/`stream_json_sequence` in `stream_maybe_colored`,
    // same as the stdout/inplace paths. Unlike the other gates above, this
    // one never OR's in `output_config.compact` for YAML -- preserved as-is
    // from before #1577.
    //
    // The JSON arm shares `can_json_fast_path`'s `can_stream_json_output_style`
    // above, which is what keeps all three JSON gates in step -- #1693 had to
    // fix each of them for `--ascii-output` separately before that local
    // existed, and #1700 then removed the flag from the predicate entirely
    // once the escaping moved to the sink. This route's own `json_ascii!`
    // wrap is on its `stream_json_sequence` call below.
    let can_slurp_fast_path = is_identity
        && match output_config.output_format {
            OutputFormat::Json => can_stream_json_output_style,
            OutputFormat::Yaml => can_stream_pretty_or_colored,
            _ => false,
        }
        && !args.null_input
        && !args.raw_input
        && args.front_matter.is_none()
        && context.named.is_empty();

    // Indent width/unit for the fast path's streamers. YAML's clamp
    // (`-I0`/`-I1` both landing on width 2, `--tab` always one tab per
    // level) is `IndentSpec::for_yaml`'s rule -- shared with
    // `OutputConfig::compute_indent_str`'s DOM-path equivalent above,
    // previously an independently hand-encoded copy of the identical
    // formula (#1685). JSON has no such clamp: `-I0` means compact/flow and
    // `-I1` genuinely means a literal 1-space step, for both real yq and
    // succinctly today (verified live: `-I1 -o=json` indents 1/2/3 spaces
    // per level in real yq, not clamped) -- `--tab` still means one tab per
    // level regardless of format, so `json_indent` threads `args.tab`
    // through its own `unit` the same way `yaml_indent`'s does.
    let indent_unit: char = if args.tab { '\t' } else { ' ' };
    let yaml_indent = IndentSpec::for_yaml(args.indent, args.tab);
    let json_indent_spaces: usize = if args.tab { 1 } else { args.indent as usize };
    let json_indent = IndentSpec {
        width: json_indent_spaces,
        unit: indent_unit,
    };
    let sort_keys = args.sort_keys;

    // Helper macro to stream cursor results (avoiding closure borrow issues).
    // Defined here (rather than inside the `if can_fast_path` block below) so
    // both the stdout M2 path and `--inplace`'s fast path (#478) can reuse
    // it. `$is_yaml`/`$use_color` are threaded through explicitly rather than
    // closing over `can_yaml_fast_path`/`output_config.use_color` by name:
    // both of those differ per call site (`--inplace` always forces color
    // off, #809), and — unlike `yaml_doc_streamed`/`any_truthy`/`sink`,
    // which each call site's enclosing block declares fresh right before
    // invoking this macro — `output_config` already existed when this macro
    // was *defined*, so a `let output_config = ...` shadow introduced later,
    // at a given call site, is invisible to a bare (non-`$`) reference here:
    // `macro_rules!` resolves such free identifiers against whatever was in
    // scope at definition time, not at each expansion site. `$fragment:expr`
    // parameters don't have that problem — they always evaluate in the
    // caller's own scope — hence passing color as `$use_color:expr`.
    // `no_doc`/`nul_output`/`join_output` need the same `$fragment:expr`
    // treatment (#1699, #1701 code review), even though nothing currently
    // shadows any of them to a different value the way `--inplace` does for
    // `.use_color` (every `OutputConfig::for_source` copies all three
    // through unchanged) -- a bare reference would only be safe by that
    // coincidence, and would silently read the wrong value the day
    // something *does* start varying one of them per source/file. Threaded
    // as one `$output_config:expr` (not three more individually-named bool
    // params) per repeated `/code-review` feedback on both issues: #1699
    // added one bool param this way, #1701 nearly added two more on top of
    // it, and every reviewer who looked at the growing positional-bool list
    // flagged the same risk -- a future call site transposing two same-typed
    // `bool` arguments compiles silently and inverts behavior. Reading the
    // three fields off one passed-by-caller-scope config value removes that
    // risk without losing the per-call-site-shadow safety `$use_color`
    // still needs its own parameter for (it, unlike these three, really is
    // overridden per call site).
    macro_rules! stream_cursor {
        ($cursor:expr, $writer:expr, $is_yaml:expr, $doc_streamed:expr, $use_color:expr, $output_config:expr) => {{
            let terminator = terminator_from_config(&$output_config);
            if $is_yaml {
                // M2 YAML path: YAML output streaming
                if is_identity {
                    // P9 path: stream directly without evaluation.
                    // `stream_yaml_as_document` (not `stream_yaml`) since
                    // `$cursor` here is the whole document being
                    // redisplayed as itself - its own trailing comment,
                    // if any, must be kept (#710).
                    //
                    // #1709: when the output is NUL-separated, the
                    // separator can't be written eagerly here -- real yq
                    // doesn't emit a document's `---` when that document's
                    // own (identity mode: only) result fails the NUL
                    // check, so the decision has to move inside
                    // `stream_maybe_colored`'s `NulChecked` sink, which
                    // only learns the outcome after rendering. Every other
                    // terminator keeps the existing eager write unchanged.
                    let nul_separator = if terminator == Terminator::Nul {
                        Some(DocSeparatorArgs {
                            doc_streamed: $doc_streamed,
                            no_doc: $output_config.no_doc,
                        })
                    } else {
                        emit_yaml_doc_separator(
                            $writer,
                            $doc_streamed,
                            true,
                            $output_config.no_doc,
                            terminator,
                        )?;
                        None
                    };
                    // No boundary recorded here -- the terminator is written
                    // directly to `$writer` below, outside this buffer (#1708).
                    let rendered = stream_maybe_colored(
                        $writer,
                        $use_color,
                        terminator,
                        nul_separator,
                        |s, boundaries| colorize_yaml(s, Terminator::None, boundaries),
                        |out| $cursor.stream_yaml_as_document(out, yaml_indent, sort_keys),
                    )?;
                    // #1615: this identity/P9 branch skips evaluation, so it
                    // has no `StreamStats` to carry a diagnostic -- route the
                    // decode failure to the same sink the evaluated branch
                    // reaches via `absorb_stream_stats`.
                    if let Err(e) = rendered {
                        report_stream_decode_failure(&mut sink, &e);
                        stream_truncated.set(true);
                    } else {
                        terminator.write_io($writer)?;
                        // Streaming skips evaluation, so inspect the document
                        // value directly to keep `-e` falsy tracking (#178).
                        if args.exit_status {
                            any_truthy |= !$cursor.is_falsy();
                        }
                    }
                } else {
                    // M2 YAML path: evaluate and stream YAML results
                    let result = eval_with_cursor_using::<YqSemantics, _>(&program.expr, $cursor);
                    // `produces_output` is an exhaustive match on
                    // `GenericResult`, not a hand-maintained exclusion
                    // list — a halt/halt_error (#791) with no prior
                    // output (`GenericResult::Halt`) answers `false`
                    // there; an output-bearing halt
                    // (`GenericResult::Partial`) answers `true`.
                    // #1709: see the identity branch's own comment above --
                    // `result.produces_output()` predicts the STRUCTURE of
                    // the result, not whether its first NUL-checked result
                    // will actually survive, so a NUL-separated document's
                    // separator has to be decided lazily too. Still gated
                    // on `result.produces_output()` itself (#1709 code
                    // review): a document producing zero results must
                    // never offer a separator opportunity at all, colored
                    // or not -- the color/`Buffered` arm of
                    // `stream_maybe_colored` has no empty-buffer guard of
                    // its own the way the non-color `NulChecked` arm does,
                    // so an ungated `Some(..)` here wrote a stray `---`
                    // around an empty middle document even when nothing
                    // failed the NUL check at all. Live-verified regression:
                    // `-0 --colors 'select(.a != 2)'` over three documents
                    // whose middle one matches nothing.
                    let nul_separator = if terminator == Terminator::Nul {
                        result.produces_output().then_some(DocSeparatorArgs {
                            doc_streamed: $doc_streamed,
                            no_doc: $output_config.no_doc,
                        })
                    } else {
                        emit_yaml_doc_separator(
                            $writer,
                            $doc_streamed,
                            result.produces_output(),
                            $output_config.no_doc,
                            terminator,
                        )?;
                        None
                    };
                    // #1708: the per-result terminator write runs *inside*
                    // this buffered render callback, so a real terminator
                    // byte here would become part of what gets colorized --
                    // record the result's boundary instead when colorizing,
                    // so `colorize_yaml` can place the real terminator
                    // itself afterward, correctly relative to whatever
                    // color span is open at each result boundary. The
                    // non-colored path (`$writer` written directly, no
                    // buffering) is unaffected and still writes the real
                    // terminator either way.
                    let stats = stream_maybe_colored(
                        $writer,
                        $use_color,
                        terminator,
                        nul_separator,
                        |s, boundaries| colorize_yaml(s, terminator, boundaries),
                        |out| {
                            result
                                .stream_yaml(out, yaml_indent, sort_keys, |w| {
                                    w.write_result_terminator(terminator)
                                })
                                .map_err(StreamFailure::from)
                        },
                    )?;
                    // `GenericResult::stream_yaml` reports a decode failure
                    // through `stats.error`, not by failing, so the outer
                    // `Result` is only ever `Ok` here. Handled rather than
                    // `unwrap_or_default()`ed anyway (#1615): defaulting would
                    // discard the diagnostic and exit 0, re-creating the exact
                    // silent swallow this change closes, if a future streamer
                    // ever did return `Decode` from this arm.
                    let stats = match stats {
                        Ok(stats) => stats,
                        Err(e) => {
                            report_stream_decode_failure(&mut sink, &e);
                            stream_truncated.set(true);
                            Default::default()
                        }
                    };
                    any_truthy |= stats.any_truthy;
                    absorb_stream_stats(&mut sink, &stats);
                    // #1615: an evaluated result can be a *container* holding
                    // an undecodable scalar, in which case the cursor stream
                    // is already part-written when it fails -- same truncation
                    // the identity branches produce, so it needs the same
                    // stop. Gated on `truncated`, never on `stats.error`: an
                    // ordinary uncaught error writes nothing to `out` and must
                    // still continue to the next document (#355).
                    if stats.truncated {
                        stream_truncated.set(true);
                    }
                }
            } else {
                // M2 path: JSON output streaming
                if is_identity {
                    // P9 path: stream directly without evaluation
                    // See the YAML identity branch above (#1615).
                    let rendered = stream_maybe_colored(
                        $writer,
                        $use_color,
                        terminator,
                        None,
                        |s, _boundaries| output::colorize_json(s, &ColorScheme::default()),
                        |sink| {
                            json_ascii!($output_config.ascii_output, sink, |out| {
                                $cursor.stream_json(out, json_indent, sort_keys)
                            })
                        },
                    )?;
                    if let Err(e) = rendered {
                        report_stream_decode_failure(&mut sink, &e);
                        stream_truncated.set(true);
                    } else {
                        terminator.write_io($writer)?;
                        // Streaming skips evaluation, so inspect the document
                        // value directly to keep `-e` falsy tracking (#178).
                        if args.exit_status {
                            any_truthy |= !$cursor.is_falsy();
                        }
                    }
                } else {
                    // M2 path: evaluate and stream results
                    let result = eval_with_cursor_using::<YqSemantics, _>(&program.expr, $cursor);
                    let stats = stream_maybe_colored(
                        $writer,
                        $use_color,
                        terminator,
                        None,
                        |s, _boundaries| output::colorize_json(s, &ColorScheme::default()),
                        |sink| {
                            json_ascii!($output_config.ascii_output, sink, |out| {
                                result
                                    .stream_json(out, json_indent, sort_keys, |w| {
                                        // #1709: `--color`'s `Buffered` mode
                                        // needs the raw terminator byte
                                        // written straight into its buffer
                                        // (JSON's own colorizer re-lexes
                                        // that unmodified, unlike YAML's
                                        // boundary-based one -- see
                                        // `stream_maybe_colored`'s own
                                        // comment on this). `write_result_
                                        // terminator` would silently drop
                                        // it there. Every other mode
                                        // (`Direct`, `NulChecked`) is
                                        // unaffected either way -- `Direct`'s
                                        // own arm is exactly `terminator.
                                        // write_fmt(self)`, and `NulChecked`
                                        // needs the dispatch to trigger its
                                        // per-result flush. `dispatch_
                                        // result_terminator` (not
                                        // `write_result_terminator`
                                        // directly) since `--ascii-output`
                                        // wraps `w` in `AsciiEscapeWriter`
                                        // here, which has no
                                        // `ColorSink`-specific methods of
                                        // its own.
                                        if $use_color {
                                            terminator.write_fmt(w)
                                        } else {
                                            w.dispatch_result_terminator(terminator)
                                        }
                                    })
                                    .map_err(StreamFailure::from)
                            })
                        },
                    )?;
                    // See the YAML branch above (#1615).
                    let stats = match stats {
                        Ok(stats) => stats,
                        Err(e) => {
                            report_stream_decode_failure(&mut sink, &e);
                            stream_truncated.set(true);
                            Default::default()
                        }
                    };
                    any_truthy |= stats.any_truthy;
                    absorb_stream_stats(&mut sink, &stats);
                    // #1615: an evaluated result can be a *container* holding
                    // an undecodable scalar, in which case the cursor stream
                    // is already part-written when it fails -- same truncation
                    // the identity branches produce, so it needs the same
                    // stop. Gated on `truncated`, never on `stats.error`: an
                    // ordinary uncaught error writes nothing to `out` and must
                    // still continue to the next document (#355).
                    if stats.truncated {
                        stream_truncated.set(true);
                    }
                }
            }
        }};
    }

    if can_fast_path {
        // M2 streaming fast path: evaluate expression and stream results directly
        // Track global document index across all files for --doc filtering
        let mut global_doc_index: usize = 0;
        // Whether any document has produced YAML output yet — drives `---`
        // separator placement between documents (#175).
        let mut yaml_doc_streamed = false;

        if input_files.is_empty() {
            let yaml_bytes = read_stdin()?;
            let fmt = resolve_input_format(args.input_format, None);
            if let Some(code) = yaml_validate_guard(&yaml_bytes, fmt, args.validate, None) {
                return Ok(code);
            }
            let mut index = YamlIndex::build(&yaml_bytes)
                .map_err(|e| anyhow::anyhow!("YAML parse error: {e}"))?;
            if fmt == InputFormat::Json {
                index.mark_json_sourced();
            }
            let root = index.root(&yaml_bytes);

            // Output each document using M2 streaming
            match root.value() {
                YamlValue::Sequence(mut docs) => {
                    while let Some((cursor, rest)) = docs.uncons_cursor() {
                        // Apply --doc filter if specified
                        let should_process = args
                            .document
                            .map_or(true, |target| global_doc_index == target);
                        if should_process {
                            stream_cursor!(
                                cursor,
                                &mut writer,
                                can_yaml_fast_path,
                                &mut yaml_doc_streamed,
                                output_config.use_color,
                                output_config
                            );
                            // #1615: a truncated identity render has no
                            // newline closing it, so continuing would weld the
                            // next document's `---` onto the cut-off line --
                            // producing output that reads back as valid YAML
                            // with a fabricated value (`b: ---`), which is
                            // worse than the silent `null` this change
                            // replaces. Real yq aborts the whole run on this
                            // input anyway.
                            if sink.halted().is_some() || stream_truncated.get() {
                                break;
                            }
                        }
                        global_doc_index += 1;
                        docs = rest;
                    }
                }
                _ => {
                    // Single document case. Defensive fallback only: the root
                    // cursor (bp_pos 0) always reports the virtual document
                    // sequence, so documents — including #175 `---` separator
                    // handling — go through the Sequence arm above.
                    if args.document.is_none() || args.document == Some(0) {
                        if is_identity {
                            // P9 path for identity on single doc
                            let streamed_ok = if can_yaml_fast_path {
                                // No boundary recorded here --
                                // `write_terminator` below writes directly,
                                // outside this buffer (#1708).
                                stream_or_report(
                                    &mut writer,
                                    output_config.use_color,
                                    terminator_from_config(&output_config),
                                    &mut sink,
                                    |s, boundaries| colorize_yaml(s, Terminator::None, boundaries),
                                    |out| root.stream_yaml_document(out, yaml_indent, sort_keys),
                                )?
                            } else {
                                stream_or_report(
                                    &mut writer,
                                    output_config.use_color,
                                    terminator_from_config(&output_config),
                                    &mut sink,
                                    |s, _boundaries| {
                                        output::colorize_json(s, &ColorScheme::default())
                                    },
                                    |sink| {
                                        json_ascii!(output_config.ascii_output, sink, |out| {
                                            root.stream_json_document(out, json_indent, sort_keys)
                                        })
                                    },
                                )?
                            };
                            if streamed_ok {
                                write_terminator(&mut writer, &output_config)?;
                                // `root` is the virtual document sequence; falsiness
                                // lives on the actual document value (#178).
                                if args.exit_status {
                                    any_truthy |= root.first_child().is_some_and(|c| !c.is_falsy());
                                }
                            }
                        } else {
                            // M2 path: need to get the actual document cursor
                            if let Some(doc_cursor) = root.first_child() {
                                stream_cursor!(
                                    doc_cursor,
                                    &mut writer,
                                    can_yaml_fast_path,
                                    &mut yaml_doc_streamed,
                                    output_config.use_color,
                                    output_config
                                );
                            }
                        }
                    }
                }
            }
        } else {
            'm2_files: for file_path in &input_files {
                let path = Path::new(file_path);
                // A later file's read failure can fire after `writer`
                // already buffered real output from earlier files in this
                // same loop (review of #1673, same class as the
                // `yaml_validate_guard`/`YamlIndex::build` returns just
                // below) -- `flush_then_err` keeps this error as the
                // reported one even if the flush also fails.
                let yaml_bytes = match read_file(path) {
                    Ok(bytes) => bytes,
                    Err(e) => return flush_then_err(&mut writer, e),
                };
                let fmt = resolve_input_format(args.input_format, Some(path));
                if let Some(code) =
                    yaml_validate_guard(&yaml_bytes, fmt, args.validate, Some(file_path))
                {
                    // Sibling of jq_runner.rs's identical fix (#1563): a
                    // later file's validation failure can fire after
                    // `writer` already buffered real output from earlier
                    // files in this same `'m2_files` loop. The halt case
                    // just above (`break 'm2_files`) reaches this
                    // function's own tail `writer.flush()?` by falling
                    // through; this early `return` doesn't, and used to
                    // rely on `writer`'s `Drop` impl instead -- which
                    // silently swallows a flush error rather than
                    // propagating it.
                    writer.flush()?;
                    return Ok(code);
                }
                let mut index = match YamlIndex::build(&yaml_bytes) {
                    Ok(index) => index,
                    Err(e) => {
                        let e = anyhow::anyhow!("YAML parse error in {file_path}: {e}");
                        return flush_then_err(&mut writer, e);
                    }
                };
                if fmt == InputFormat::Json {
                    index.mark_json_sourced();
                }
                let root = index.root(&yaml_bytes);

                match root.value() {
                    YamlValue::Sequence(mut docs) => {
                        while let Some((cursor, rest)) = docs.uncons_cursor() {
                            // Apply --doc filter if specified
                            let should_process = args
                                .document
                                .map_or(true, |target| global_doc_index == target);
                            if should_process {
                                stream_cursor!(
                                    cursor,
                                    &mut writer,
                                    can_yaml_fast_path,
                                    &mut yaml_doc_streamed,
                                    output_config.use_color,
                                    output_config
                                );
                                // Halt outranks every remaining document and
                                // file (#791): without this, a `halt` nested
                                // inside `first(...)`/`last(...)`/a computed
                                // index — the only shapes that reach the M2
                                // path with a halt at all, see
                                // `can_use_m2_streaming` — would keep
                                // streaming further documents and files
                                // instead of stopping immediately.
                                // #1615: see the `Sequence` arm's note above.
                                if sink.halted().is_some() || stream_truncated.get() {
                                    break 'm2_files;
                                }
                            }
                            global_doc_index += 1;
                            docs = rest;
                        }
                    }
                    _ => {
                        // Single document case. Defensive fallback only: the
                        // root cursor (bp_pos 0) always reports the virtual
                        // document sequence, so documents — including #175
                        // `---` separator handling — go through the Sequence
                        // arm above.
                        let should_process = args
                            .document
                            .map_or(true, |target| global_doc_index == target);
                        if should_process {
                            if is_identity {
                                // P9 path for identity on single doc
                                let streamed_ok = if can_yaml_fast_path {
                                    // No boundary recorded here --
                                    // `write_terminator` below writes
                                    // directly, outside this buffer (#1708).
                                    stream_or_report(
                                        &mut writer,
                                        output_config.use_color,
                                        terminator_from_config(&output_config),
                                        &mut sink,
                                        |s, boundaries| {
                                            colorize_yaml(s, Terminator::None, boundaries)
                                        },
                                        |out| {
                                            root.stream_yaml_document(out, yaml_indent, sort_keys)
                                        },
                                    )?
                                } else {
                                    stream_or_report(
                                        &mut writer,
                                        output_config.use_color,
                                        terminator_from_config(&output_config),
                                        &mut sink,
                                        |s, _boundaries| {
                                            output::colorize_json(s, &ColorScheme::default())
                                        },
                                        |sink| {
                                            json_ascii!(output_config.ascii_output, sink, |out| {
                                                root.stream_json_document(
                                                    out,
                                                    json_indent,
                                                    sort_keys,
                                                )
                                            })
                                        },
                                    )?
                                };
                                if streamed_ok {
                                    write_terminator(&mut writer, &output_config)?;
                                    // `root` is the virtual document sequence;
                                    // falsiness lives on the actual document
                                    // value (#178).
                                    if args.exit_status {
                                        any_truthy |=
                                            root.first_child().is_some_and(|c| !c.is_falsy());
                                    }
                                }
                            } else {
                                // M2 path: need to get the actual document cursor
                                if let Some(doc_cursor) = root.first_child() {
                                    stream_cursor!(
                                        doc_cursor,
                                        &mut writer,
                                        can_yaml_fast_path,
                                        &mut yaml_doc_streamed,
                                        output_config.use_color,
                                        output_config
                                    );
                                    // See the matching check in the `Sequence` arm above.
                                    // #1615 adds the truncation term.
                                    if sink.halted().is_some() || stream_truncated.get() {
                                        break 'm2_files;
                                    }
                                }
                            }
                        }
                        global_doc_index += 1;
                    }
                }
            }
        }
    } else if args.eval_all {
        // Handle --eval-all: combine every document from every file into one
        // evaluation context, exposing `file_index`/`fileIndex`/`fi` (#715).
        // Input-gathering mirrors --slurp's DOM path (all_docs collection),
        // but tracks each document's origin file index alongside it and
        // evaluates via `eval_owned_with_file_index` instead of plain
        // `evaluate_input`, so `file_index` resolves against that side table.
        // `--front-matter` is rejected in combination above, so every
        // gathered body is `None` here.
        let (input_sources, validate_exit_code) = gather_input_sources(
            &input_files,
            args.input_format,
            args.front_matter,
            args.validate,
        )?;
        if let Some(code) = validate_exit_code {
            // #1564: --eval-all combines every source into one evaluation
            // (see below), so a partial set missing the file that failed
            // (and everything after it) can't stand in for "every file
            // combined" -- bail before evaluating anything, unlike the
            // per-file-independent paths this issue actually fixes.
            return Ok(code);
        }

        // #1493 review: this branch was missed by the original fix --
        // every other output-producing branch resolves `Auto` per-source,
        // but `--eval-all` combines every source into one evaluation the
        // same way `--slurp` does, so it gets the identical "uniform
        // format across every source, falling back to Yaml if they
        // disagree" treatment `--slurp`'s own fix already established
        // (see that branch's own comment for the real-yq-mismatch
        // rationale on the mixed-format case).
        let mut source_formats = input_sources.iter().map(|(_, format, _)| *format);
        let uniform_format = match source_formats.next() {
            Some(first) if source_formats.all(|f| f == first) => first,
            _ => InputFormat::Yaml,
        };
        let output_config = output_config.for_source(&args, uniform_format);

        let mut all_docs: Vec<OwnedValue> = Vec::new();
        let mut file_origin: Vec<usize> = Vec::new();
        let mut global_doc_index: usize = 0;
        for (file_idx, (bytes, format, _)) in input_sources.iter().enumerate() {
            let inputs = parse_input(bytes, *format)?;
            for input in inputs {
                let should_include = args
                    .document
                    .map_or(true, |target| target == global_doc_index);
                if should_include {
                    all_docs.push(input);
                    file_origin.push(file_idx);
                }
                global_doc_index += 1;
            }
        }

        let combined = OwnedValue::Array(all_docs);
        let query_result: QueryResult<'_, Vec<u64>> = jq::eval_owned_with_file_index::<
            Vec<u64>,
            YqSemantics,
        >(
            &program.expr, &combined, &file_origin
        );
        let results = query_result_to_owned_values(query_result, &mut sink);

        let mut split_doc_state = SplitDocState::new(has_split_doc);
        for (i, result) in results.iter().enumerate() {
            let wants_doc_separator = !has_split_doc
                && output_config.output_format == OutputFormat::Yaml
                && !output_config.no_doc
                && results.len() > 1;
            let mut doc_streamed = i > 0;
            let separator = if has_split_doc {
                // The filter explicitly marks its outputs as separate
                // documents (`split_doc`); route through the same state
                // machine every other output path uses for that, or no
                // separator is ever written here at all. Deferred into
                // `output_value` under `-0`, same as every other arm here.
                split_doc_state.write_separator(&mut writer, &output_config)?
            } else if wants_doc_separator && i > 0 && !output_config.nul_output {
                // `---` BETWEEN results (not before the first) -- deliberately
                // different from --slurp's no-separator convention, since
                // eval-all is explicitly a multi-document-stream feature (#715).
                // Same `---`-must-start-on-its-own-line fix as
                // `emit_yaml_doc_separator`/`SplitDocState::write_separator`
                // (#1701 code review): the previous result's own
                // `write_terminator` call (inside `output_value` below)
                // might have written `\0`/nothing rather than `\n`.
                //
                // Skipped under `-0` (#1709 code review): a NUL-checked
                // result can still fail `output_value`'s own check, and
                // writing this eagerly would leave a stray `---` for a
                // document whose own content never made it out -- same bug
                // class the M2 streaming path's `PendingSeparator` exists
                // to close. `output_value` writes it lazily instead, below,
                // once its own check has passed.
                write_doc_separator_marker(&mut writer, terminator_from_config(&output_config))?;
                None
            } else {
                (wants_doc_separator && output_config.nul_output).then_some(DocSeparatorArgs {
                    doc_streamed: &mut doc_streamed,
                    no_doc: output_config.no_doc,
                })
            };
            any_truthy |= !matches!(result, OwnedValue::Null | OwnedValue::Bool(false));
            output_value(
                &mut writer,
                result,
                &CommentTree::empty(),
                &output_config,
                separator,
            )?;
        }
    } else if let Some(split_expr) = split_expr.as_ref() {
        // Handle --split-exp: write each result to its own file (named by
        // evaluating `split_expr` against it, with `$index` bound to its
        // zero-based output index) instead of stdout. `--front-matter` is
        // rejected in combination above, so no extraction step is needed
        // here; the input-gathering below otherwise mirrors the standard
        // path at the bottom of this function.
        let mut output_index: i64 = 0;
        let mut written_split_files: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        if args.null_input {
            let results = evaluate_input(&OwnedValue::Null, &program.expr, &mut sink)?;
            // Snapshotted before the loop: if the *main* filter already
            // halted after producing several values (e.g. `1,2,3,halt`),
            // `results` is the full legitimate pre-halt prefix and every
            // element still owes its file — `sink.halted()` must not be
            // misread as "this iteration halted" until a *new* halt (from
            // this batch's own split-filename evaluation) actually occurs
            // (#791, mirrors `write_split_result`'s own `halted_before`).
            let halted_before_batch = sink.halted().is_some();
            // No source to match against `--null-input` (#1493): real yq's
            // own `-o=auto` here still defaults to Yaml (confirmed live),
            // matching `Self::for_source`'s own "nothing to match" arm.
            let null_input_output_config = output_config.for_source(&args, InputFormat::Yaml);
            for result in &results {
                any_truthy |= !matches!(result, OwnedValue::Null | OwnedValue::Bool(false));
                write_split_result(
                    result,
                    &CommentTree::empty(),
                    split_expr,
                    output_index,
                    &null_input_output_config,
                    &mut written_split_files,
                    &mut sink,
                )?;
                output_index += 1;
                // halt/halt_error (#791) outranks writing any further split
                // files, but only once introduced during this batch.
                if !halted_before_batch && sink.halted().is_some() {
                    break;
                }
            }
        } else {
            // `--front-matter` is rejected in combination above, so every
            // gathered body is `None` here.
            let (input_sources, validate_exit_code) = gather_input_sources(
                &input_files,
                args.input_format,
                args.front_matter,
                args.validate,
            )?;
            // #1564: unlike --eval-all/--slurp, each file's split output is
            // already written independently of the others (the loop below)
            // -- process whatever passed and report the failure once done,
            // instead of losing every earlier file's already-valid output.
            deferred_validate_exit_code = validate_exit_code;

            let mut global_doc_index: usize = 0;
            'files: for (bytes, format, _) in &input_sources {
                // Yaml/Auto and Json both go through the same cursor-native
                // evaluator now (#1398) -- JSON is a YAML subset, and
                // routing it through `YamlIndex`/`mark_json_sourced` here
                // (rather than `parse_input`'s eager `OwnedValue`
                // materialization) is what keeps duplicate-key handling
                // consistent regardless of which format the same logical
                // document arrived as. `--slurp`/`--eval-all`/`--inplace`'s
                // DOM fallback still use the old `parse_input`/
                // `evaluate_input` pair for both formats -- a separate,
                // already-tracked issue (#1343), not touched here.
                let doc_filter = args.document.map(|target| (target, global_doc_index));
                // Per-file `OutputConfig` (#978/#1398's `json_sourced_floats`,
                // #1493's `Auto` resolution) -- both need this specific
                // source's own `InputFormat`, not the invocation-wide
                // default, since `--input-format auto` can mix JSON and
                // YAML sources in one run.
                let file_output_config = output_config.for_source(&args, *format);
                // Split-exp output files carry real comments when written
                // as YAML, same as the main output path (#710) -- resolved
                // per-file now, not the invocation-wide default (#1493).
                let need_comments = file_output_config.output_format == OutputFormat::Yaml;
                let (doc_results, num_docs) = evaluate_yaml_direct_filtered(
                    bytes,
                    &program.expr,
                    doc_filter,
                    &mut sink,
                    DirectEvalOptions {
                        need_comments,
                        strip_style: args.pretty_print,
                        sort_keys: output_config.sort_keys,
                        mark_json_sourced: *format == InputFormat::Json,
                    },
                )?;
                global_doc_index += num_docs;
                for results in doc_results {
                    // See the --null-input arm above: a halt already
                    // present when this document's batch starts
                    // means every element of `results` is a
                    // legitimate pre-halt prefix that still owes its
                    // file, so only a *new* halt (from this batch's
                    // own split-filename evaluation) should stop the
                    // loop early (#791).
                    let halted_before_batch = sink.halted().is_some();
                    for (result, comments) in &results {
                        any_truthy |= !matches!(result, OwnedValue::Null | OwnedValue::Bool(false));
                        write_split_result(
                            result,
                            comments,
                            split_expr,
                            output_index,
                            &file_output_config,
                            &mut written_split_files,
                            &mut sink,
                        )?;
                        output_index += 1;
                        // halt/halt_error (#791) outranks writing any
                        // further split files or evaluating any
                        // further documents/inputs/files.
                        if !halted_before_batch && sink.halted().is_some() {
                            break 'files;
                        }
                    }
                }
                if sink.halted().is_some() {
                    break 'files;
                }
            }
        }
    } else if args.null_input {
        // Handle --null-input. No source to resolve `Auto` against
        // (#1493): real yq's own `-o=auto` here still defaults to Yaml
        // (confirmed live), matching `Self::for_source`'s "nothing to
        // match" arm.
        let output_config = output_config.for_source(&args, InputFormat::Yaml);
        let mut split_doc_state = SplitDocState::new(has_split_doc);
        let results = evaluate_input(&OwnedValue::Null, &program.expr, &mut sink)?;
        for result in results {
            let split_separator = split_doc_state.write_separator(&mut writer, &output_config)?;
            any_truthy |= !matches!(&result, OwnedValue::Null | OwnedValue::Bool(false));
            output_value(
                &mut writer,
                &result,
                &CommentTree::empty(),
                &output_config,
                split_separator,
            )?;
        }
    } else if args.raw_input {
        // Handle --raw-input: read each line as a string instead of
        // parsing as YAML. Same "nothing to resolve `Auto` against" case
        // as `--null-input` above (#1493) -- a raw line is never
        // parsed/detected as YAML or JSON.
        let output_config = output_config.for_source(&args, InputFormat::Yaml);
        let input_content = if input_files.is_empty() {
            read_stdin_string()?
        } else {
            let mut content = String::new();
            for file_path in &input_files {
                let file_content = std::fs::read_to_string(file_path)
                    .with_context(|| format!("failed to read file: {file_path}"))?;
                content.push_str(&file_content);
            }
            content
        };

        let mut split_doc_state = SplitDocState::new(has_split_doc);
        if args.slurp {
            // yq -R -s (jq semantics): the entire input (all files
            // concatenated) becomes a single string; no line splitting and
            // no array wrap.
            let slurped = OwnedValue::String(input_content);
            let results = evaluate_input(&slurped, &program.expr, &mut sink)?;
            for result in results {
                let split_separator =
                    split_doc_state.write_separator(&mut writer, &output_config)?;
                any_truthy |= !matches!(&result, OwnedValue::Null | OwnedValue::Bool(false));
                output_value(
                    &mut writer,
                    &result,
                    &CommentTree::empty(),
                    &output_config,
                    split_separator,
                )?;
            }
        } else {
            // Without --slurp, process each line independently
            for line in input_content.lines() {
                let input = OwnedValue::String(line.to_string());
                let results = evaluate_input(&input, &program.expr, &mut sink)?;
                for result in results {
                    let split_separator =
                        split_doc_state.write_separator(&mut writer, &output_config)?;
                    any_truthy |= !matches!(&result, OwnedValue::Null | OwnedValue::Bool(false));
                    output_value(
                        &mut writer,
                        &result,
                        &CommentTree::empty(),
                        &output_config,
                        split_separator,
                    )?;
                }
                // halt/halt_error (#791) outranks evaluating any further lines.
                if sink.halted().is_some() {
                    break;
                }
            }
        }
    } else if args.slurp {
        // Handle --slurp: collect all documents from all inputs into an array

        // Collect input sources. `--front-matter` (extract only here; process
        // mode is rejected above since a slurped array can't reattach a body
        // per input file) is applied before validation, since the raw
        // file bytes (e.g. Markdown) aren't valid standalone YAML.
        let (input_sources, validate_exit_code) = gather_input_sources(
            &input_files,
            args.input_format,
            args.front_matter,
            args.validate,
        )?;
        if let Some(code) = validate_exit_code {
            // #1564: --slurp combines every source into one array (see
            // below), so a partial set can't stand in for "the slurped
            // array of every file" -- bail before evaluating anything,
            // unlike the per-file-independent paths this issue fixes.
            return Ok(code);
        }

        if can_slurp_fast_path {
            // M2 streaming fast path (#478): stream each source's document
            // cursor(s) directly into one combined YAML sequence, skipping
            // the OwnedValue DOM. `evaluate_input`'s JSON round-trip below
            // would otherwise re-collapse duplicate mapping keys even if the
            // initial conversion into it didn't (the array-builder step
            // has its own `IndexMap`-backed collapse).
            //
            // Two-phase: parse every source into an owned `(bytes, YamlIndex)`
            // pair first, so all sources stay alive together — `YamlCursor`
            // borrows both `text` and `index` with the same lifetime, so
            // cursors from different sources can't be collected into one
            // `Vec` unless their backing bytes/index all outlive that `Vec`.
            let mut parsed_sources: Vec<(Vec<u8>, YamlIndex<Vec<u64>>)> =
                Vec::with_capacity(input_sources.len());
            for (bytes, format, _) in input_sources {
                let mut index = YamlIndex::build(&bytes)
                    .map_err(|e| anyhow::anyhow!("YAML parse error: {e}"))?;
                if format == InputFormat::Json {
                    index.mark_json_sourced();
                }
                parsed_sources.push((bytes, index));
            }

            let mut cursors = Vec::new();
            let mut global_doc_index: usize = 0;
            for (bytes, index) in &parsed_sources {
                let root = index.root(bytes);
                match root.value() {
                    YamlValue::Sequence(mut docs) => {
                        while let Some((cursor, rest)) = docs.uncons_cursor() {
                            let should_process = args
                                .document
                                .map_or(true, |target| global_doc_index == target);
                            if should_process {
                                cursors.push(cursor);
                            }
                            global_doc_index += 1;
                            docs = rest;
                        }
                    }
                    _ => {
                        // Defensive fallback only, matching the stdout/inplace
                        // M2 paths: the root cursor always reports the virtual
                        // document sequence, so real documents go through the
                        // Sequence arm above.
                        let should_process = args
                            .document
                            .map_or(true, |target| global_doc_index == target);
                        if should_process {
                            if let Some(doc_cursor) = root.first_child() {
                                cursors.push(doc_cursor);
                            }
                        }
                        global_doc_index += 1;
                    }
                }
            }

            if args.exit_status {
                // `--slurp '.'` always yields exactly one (array) result, and
                // a non-empty array is truthy regardless of its elements —
                // matching jq/yq `-e` semantics for arrays.
                any_truthy = true;
            }
            // Buffer-and-colorize (#748, extended to `--slurp` by #809, and
            // to `-o=json` by #1577): `stream_yaml_sequence`/
            // `stream_json_sequence` are both generic over `core::fmt::Write`,
            // so each slots into `stream_maybe_colored` unmodified.
            let streamed_ok = if output_config.output_format == OutputFormat::Json {
                stream_or_report(
                    &mut writer,
                    output_config.use_color,
                    terminator_from_config(&output_config),
                    &mut sink,
                    |s, _boundaries| output::colorize_json(s, &ColorScheme::default()),
                    |sink| {
                        json_ascii!(output_config.ascii_output, sink, |out| {
                            stream_json_sequence(
                                cursors.iter().copied(),
                                out,
                                0,
                                json_indent_spaces,
                                indent_unit,
                                sort_keys,
                            )
                        })
                    },
                )?
            } else {
                // No boundary recorded here -- `write_terminator` below
                // writes directly, outside this buffer (#1708).
                stream_or_report(
                    &mut writer,
                    output_config.use_color,
                    terminator_from_config(&output_config),
                    &mut sink,
                    |s, boundaries| colorize_yaml(s, Terminator::None, boundaries),
                    |out| {
                        stream_yaml_sequence(
                            cursors.iter().copied(),
                            out,
                            0,
                            yaml_indent.width,
                            indent_unit,
                            sort_keys,
                        )
                    },
                )?
            };
            // #1701: this fast path streams straight to `&mut writer`
            // (like `stream_cursor!`'s own identity branch), so it needs
            // the same `Terminator`-based write instead of a hardcoded
            // `writeln!` -- confirmed live that `--slurp -0`/`--slurp
            // --join-output` still emitted a bare newline here before this
            // fix, unlike the `stream_cursor!`-based fast paths this same
            // issue's title names. Skipped when the stream was cut short by a
            // decode failure (#1615) -- the diagnostic is already on stderr
            // and there is no complete result for a terminator to close.
            if streamed_ok {
                write_terminator(&mut writer, &output_config)?;
            }
        } else {
            // #1493: resolve `Auto` against the uniform format of every
            // source, if they all agree (confirmed live: an all-JSON slurp
            // renders JSON, an all-YAML slurp renders YAML). A genuinely
            // mixed-format slurp falls back to Yaml, the same "nothing
            // single to match" treatment as `--null-input` above -- real
            // yq's own mixed-format `-o=auto --slurp` renders each element
            // in its own source format, a much deeper per-element behavior
            // this fix doesn't attempt to replicate (pinned as a known gap
            // in the test suite).
            let mut source_formats = input_sources.iter().map(|(_, format, _)| *format);
            let uniform_format = match source_formats.next() {
                Some(first) if source_formats.all(|f| f == first) => first,
                _ => InputFormat::Yaml,
            };
            let output_config = output_config.for_source(&args, uniform_format);

            let mut all_docs: Vec<OwnedValue> = Vec::new();

            // Parse all inputs and collect documents
            let mut global_doc_index: usize = 0;
            for (bytes, format, _) in &input_sources {
                let inputs = parse_input(bytes, *format)?;
                for input in inputs {
                    // Apply --doc filter if specified
                    if let Some(target_doc) = args.document {
                        if global_doc_index == target_doc {
                            all_docs.push(input);
                        }
                    } else {
                        all_docs.push(input);
                    }
                    global_doc_index += 1;
                }
            }

            // Create slurped array and evaluate
            let slurped = OwnedValue::Array(all_docs);
            let results = evaluate_input(&slurped, &program.expr, &mut sink)?;
            let mut split_doc_state = SplitDocState::new(has_split_doc);
            for result in results {
                let split_separator =
                    split_doc_state.write_separator(&mut writer, &output_config)?;
                any_truthy |= !matches!(&result, OwnedValue::Null | OwnedValue::Bool(false));
                output_value(
                    &mut writer,
                    &result,
                    &CommentTree::empty(),
                    &output_config,
                    split_separator,
                )?;
            }
        }
    } else if args.inplace {
        // Handle --inplace: process each file and write back to it
        if input_files.is_empty() {
            anyhow::bail!("--inplace requires at least one file argument");
        }

        let mut global_doc_index: usize = 0;
        'inplace_files: for file_path in &input_files {
            let path = Path::new(file_path);
            let raw_bytes = read_file(path)?;
            let resolved_format = resolve_input_format(args.input_format, Some(path));
            let (input_bytes, format, front_matter_body) =
                apply_front_matter(raw_bytes, resolved_format, args.front_matter, file_path)?;
            if let Some(code) =
                yaml_validate_guard(&input_bytes, format, args.validate, Some(file_path))
            {
                return Ok(code);
            }

            let mut output_buffer = Vec::new();
            // #1615: a decode failure must leave the file byte-identical, the
            // way the *materializing* `--inplace` path already does (`-i '.a =
            // 5'` on an undecodable scalar raises and does not touch the file)
            // and the way real yq does on every filter. The streamed path
            // reports through `sink` and keeps going, so it would otherwise
            // reach the `fs::write` below and commit a truncated buffer --
            // strictly worse than the silent `b: ""` this whole change
            // replaces, because it destroys the user's own data. Compared
            // rather than a bool so *any* diagnostic raised while building
            // this file's buffer protects it, not only the streamed ones.
            let reports_before_file = sink.report_count();
            // Reset per file, not just per run: `--inplace` keeps editing the
            // remaining files after one fails, so a flag left set by an
            // earlier file would break the *next* file's document loop on its
            // first iteration -- committing a buffer holding only that file's
            // first document and silently dropping the rest. (The stdout paths
            // need no such reset: their truncation break leaves the whole run
            // via `break 'm2_files`, and the stdin branch has no file loop at
            // all.)
            stream_truncated.set(false);

            // `--inplace` never writes ANSI to disk (#809): shadow
            // `output_config` for the rest of this file's write so the DOM
            // branch below sees `use_color: false` regardless of `-C` (the
            // fast-path branch instead passes `false` explicitly as
            // `stream_cursor!`'s `$use_color` argument — a bare
            // `output_config.use_color` reference inside that macro would
            // resolve against the *original*, unshadowed binding, since
            // `output_config` already existed when the macro was defined).
            // Forcing color off here also closes a live bug: compact mode
            // (`-I0`) already took the fast path unconditionally (the
            // `compact ||` gate short-circuits before color is checked),
            // and nothing forced color off on that path, so
            // `-C -I0 --inplace` wrote raw ANSI bytes straight into the
            // file. Also resolves `Auto` against this file's own format
            // (#1493) -- the fast paths above require `output_format ==
            // Json`/`== Yaml` exactly, so `Auto` always reaches this slow
            // DOM branch regardless, same pre-existing scope limit as
            // `--slurp`'s fast path.
            let mut output_config = output_config.for_source(&args, format);
            output_config.use_color = false;

            // halt/halt_error (#791): tracks whether any *real* evaluated
            // content (not a speculatively pre-written `---` separator or
            // front-matter fence) made it into `output_buffer`, so the
            // write-back guard below can tell "this file was never really
            // considered" apart from "the buffer merely has some bytes in
            // it" — see the guard's own comment for why that distinction
            // matters.
            let any_real_output = if can_inplace_fast_path {
                // M2 streaming fast path (#478): stream cursor results
                // directly into this file's buffer, skipping the OwnedValue
                // DOM (and its IndexMap, which cannot represent duplicate
                // mapping keys) the same way the stdout M2 path above does.
                let mut index = YamlIndex::build(&input_bytes)
                    .map_err(|e| anyhow::anyhow!("YAML parse error in {file_path}: {e}"))?;
                if format == InputFormat::Json {
                    index.mark_json_sourced();
                }
                let root = index.root(&input_bytes);

                {
                    let mut buf_writer = BufWriter::new(&mut output_buffer);
                    // `---` separators start fresh per file, unlike the
                    // stdout path where they persist across all input files.
                    let mut yaml_doc_streamed = false;

                    match root.value() {
                        YamlValue::Sequence(mut docs) => {
                            while let Some((cursor, rest)) = docs.uncons_cursor() {
                                let should_process = args
                                    .document
                                    .map_or(true, |target| global_doc_index == target);
                                if should_process {
                                    stream_cursor!(
                                        cursor,
                                        &mut buf_writer,
                                        can_inplace_yaml_fast_path,
                                        &mut yaml_doc_streamed,
                                        false,
                                        output_config
                                    );
                                    // halt/halt_error (#791): matches the DOM
                                    // `--inplace` branch below — stop
                                    // streaming further documents into this
                                    // file, but still let the shared
                                    // write-back-then-break-'inplace_files
                                    // logic after this `if`/`else` run, so
                                    // the prefix already streamed is still
                                    // committed to disk.
                                    if sink.halted().is_some() {
                                        break;
                                    }
                                    // #1615: same reasoning as the halt break
                                    // above. Once a document has been cut off
                                    // mid-value there is no newline to close
                                    // it, so continuing would weld the next
                                    // document's `---` onto the truncated
                                    // line. The buffer is discarded by the
                                    // write-back guard either way; stopping
                                    // keeps it from growing a fabricated
                                    // value first.
                                    if stream_truncated.get() {
                                        break;
                                    }
                                }
                                global_doc_index += 1;
                                docs = rest;
                            }
                        }
                        _ => {
                            // Single document case. See the stdout M2 path's
                            // identical fallback above: the root cursor
                            // always reports the virtual document sequence,
                            // so real documents go through the Sequence arm.
                            let should_process = args
                                .document
                                .map_or(true, |target| global_doc_index == target);
                            if should_process {
                                if is_identity {
                                    // #1615: a bare `map_err(|_| "Write
                                    // error")` here would collapse a `Decode`
                                    // into exactly the message-less error
                                    // `StreamFailure` exists to escape -- and
                                    // would leave `sink` unmarked, so the
                                    // write-back guard above would still
                                    // commit the truncated buffer.
                                    let streamed = if can_inplace_yaml_fast_path {
                                        root.stream_yaml_document(
                                            &mut FmtWriter(&mut buf_writer),
                                            yaml_indent,
                                            sort_keys,
                                        )
                                    } else {
                                        json_ascii!(
                                            output_config.ascii_output,
                                            &mut FmtWriter(&mut buf_writer),
                                            |out| root.stream_json_document(
                                                out,
                                                json_indent,
                                                sort_keys
                                            )
                                        )
                                    };
                                    match streamed {
                                        Ok(()) => {}
                                        Err(StreamFailure::Decode(e)) => {
                                            report_stream_decode_failure(&mut sink, &e);
                                        }
                                        Err(StreamFailure::Fmt) => {
                                            anyhow::bail!("Write error")
                                        }
                                    }
                                    write_terminator(&mut buf_writer, &output_config)?;
                                    if args.exit_status {
                                        any_truthy |=
                                            root.first_child().is_some_and(|c| !c.is_falsy());
                                    }
                                } else if let Some(doc_cursor) = root.first_child() {
                                    stream_cursor!(
                                        doc_cursor,
                                        &mut buf_writer,
                                        can_inplace_yaml_fast_path,
                                        &mut yaml_doc_streamed,
                                        false,
                                        output_config
                                    );
                                }
                            }
                            global_doc_index += 1;
                        }
                    }
                    buf_writer.flush()?;
                }
                // This branch never pre-writes speculative separator/fence
                // bytes (front matter forces the DOM branch below, and its
                // own `---` separators are only emitted after a document has
                // actually streamed real content), so the buffer's emptiness
                // already reflects whether any real output happened.
                !output_buffer.is_empty()
            } else {
                let inputs = parse_input(&input_bytes, format)?;

                let mut buf_writer = BufWriter::new(&mut output_buffer);
                // Count matching docs for multi-doc separator logic
                let matching_docs: usize = if let Some(target_doc) = args.document {
                    usize::from(
                        (global_doc_index..global_doc_index + inputs.len()).contains(&target_doc),
                    )
                } else {
                    inputs.len()
                };
                let is_multi_doc = matching_docs > 1;

                // `--front-matter=process` opens its own leading `---` fence
                // here; the body's closing fence + body text are appended
                // after `buf_writer` is dropped, below.
                if front_matter_body.is_some() {
                    writeln!(buf_writer, "---")?;
                }

                // halt/halt_error (#791): tracks real per-document output
                // only, separately from the speculative `---`/fence bytes
                // that may already be sitting in `buf_writer` above.
                let mut any_real_output = false;
                let mut split_doc_state = SplitDocState::new(has_split_doc);
                // Tracks "has any *earlier* document in this file already
                // had real output" (#1497 review) -- `doc_had_output`
                // below resets per document, deciding only where within
                // *that* document's own results the separator goes, not
                // whether this is the file's first document overall. Every
                // invocation that would reach this multi-doc separator
                // logic with `output_format == Yaml` used to take the M2
                // fast path instead (which already gets this right via its
                // own `yaml_doc_streamed` flag); `-o=auto` could never
                // satisfy that fast path's exact-format gate pre-#1493, so
                // this dormant "leading separator on the first document
                // too" bug in the DOM branch went unreachable until this
                // fix made `Auto` resolve to `Yaml` for real.
                let mut any_doc_output_this_file = false;
                for (local_idx, input) in inputs.iter().enumerate() {
                    let current_doc_index = global_doc_index + local_idx;
                    // Apply --doc filter if specified
                    if let Some(target_doc) = args.document {
                        if current_doc_index != target_doc {
                            continue;
                        }
                    }

                    let results = evaluate_input(input, &program.expr, &mut sink)?;
                    // `output_config` was already shadowed to `use_color:
                    // false` above — reused here rather than building a
                    // second, parallel no-color config.
                    let mut doc_had_output = false;
                    for result in results {
                        // For regular multi-doc (without split_doc), add ---
                        // before each doc's first real output — not
                        // unconditionally before the doc is even evaluated.
                        // A doc whose query yields no values gets no
                        // separator either side (#175), matching the M2 fast
                        // path's already-correct behavior above; writing it
                        // eagerly left a dangling `---` whenever a doc
                        // produced zero output, whether from an ordinary
                        // empty filter or from a halt partway through (#791).
                        let wants_doc_separator = !doc_had_output
                            && !has_split_doc
                            && output_config.output_format == OutputFormat::Yaml
                            && !output_config.no_doc
                            && is_multi_doc
                            && front_matter_body.is_none()
                            && any_doc_output_this_file;
                        // #1709 code review: deferred to `output_value`
                        // itself under `-0`, same reason as the `--eval-all`
                        // arm above -- an eager write here can't yet know
                        // whether this document's own first result will
                        // survive `output_value`'s NUL check.
                        let defer_to_nul_check = wants_doc_separator && output_config.nul_output;
                        if wants_doc_separator && !output_config.nul_output {
                            // #1701 code review: same fix as
                            // `emit_yaml_doc_separator` -- the previous
                            // document's own terminator might not have been
                            // `\n`.
                            write_doc_separator_marker(
                                &mut buf_writer,
                                terminator_from_config(&output_config),
                            )?;
                        }
                        if !defer_to_nul_check {
                            doc_had_output = true;
                            any_doc_output_this_file = true;
                        }
                        // `split_doc` (#1709 code review) takes priority
                        // over the inter-document separator above when both
                        // apply -- matches every other call site's own
                        // has_split_doc-first precedence (e.g. the
                        // --eval-all arm above).
                        let split_separator =
                            split_doc_state.write_separator(&mut buf_writer, &output_config)?;
                        any_truthy |=
                            !matches!(&result, OwnedValue::Null | OwnedValue::Bool(false));
                        let separator = split_separator.or_else(|| {
                            defer_to_nul_check.then_some(DocSeparatorArgs {
                                doc_streamed: &mut any_doc_output_this_file,
                                no_doc: output_config.no_doc,
                            })
                        });
                        output_value(
                            &mut buf_writer,
                            &result,
                            &CommentTree::empty(),
                            &output_config,
                            separator,
                        )?;
                        if defer_to_nul_check {
                            doc_had_output = true;
                        }
                        any_real_output = true;
                    }
                    // halt/halt_error (#791): still write this file's buffer
                    // so far (below, matching the "prefix already output
                    // survives" rule elsewhere), but evaluate no further
                    // documents in this file or any other.
                    if sink.halted().is_some() {
                        break;
                    }
                }
                buf_writer.flush()?;
                global_doc_index += inputs.len();
                any_real_output
            };

            if let Some(body) = &front_matter_body {
                // #1701 code review round 4: same fix as every other `---`
                // writer -- but gated on `any_real_output`, not applied
                // unconditionally: `output_buffer` at this point holds only
                // the opening fence's own hardcoded (always-`\n`-terminated)
                // `writeln!` when no document produced real output (e.g. a
                // halt before evaluating anything), and the guard would
                // insert a spurious leading blank line there instead of
                // fixing anything -- it's only the *document content*
                // `output_value` wrote, subject to `-0`/`--join-output`,
                // that can need it.
                if any_real_output {
                    write_doc_marker_newline_guard(
                        &mut output_buffer,
                        terminator_from_config(&output_config),
                    )?;
                }
                output_buffer.extend_from_slice(b"---");
                output_buffer.extend_from_slice(front_matter::body_line_ending(body));
                output_buffer.extend_from_slice(body);
            }

            // Write the output back to the file — unless a `halt`/
            // `halt_error` fired before this file produced any *real* output
            // at all. A halt aborts the whole process; this file was never
            // fully considered, so leaving its original content in place is
            // safer than truncating it to reflect an evaluation that never
            // really finished (#791) — `halt` used as an early-exit guard
            // clause is a natural way to trigger this under `-i`.
            //
            // Gated on `any_real_output`, not `output_buffer.is_empty()`:
            // the DOM branch above can pre-write a `--front-matter=process`
            // fence into `output_buffer` before evaluating the document that
            // follows it, so a halt with zero real output can still leave
            // the buffer non-empty. Checking emptiness directly would let
            // that speculative prefix defeat this guard and truncate the
            // file down to just the prefix. (The multi-doc `---` separator
            // is no longer speculative — it's written only after a document
            // has actually produced its first output, same as the M2 fast
            // path above.)
            //
            // Deliberately narrower than "output is empty": real yq (v4.53.3,
            // verified live) truncates a file to reflect a filter that
            // legitimately produces no output for it, e.g. `-i 'select(false)'`
            // or `-i 'del(.)'` both empty the file — so only a halt gets this
            // protection, not ordinary empty output. This is a deliberate,
            // tested product choice for `-i` specifically (see
            // `test_inplace_halt_before_any_output_does_not_truncate_file`
            // and its neighbors): unlike every other halt-propagation fix in
            // this codebase, "should halt-caused emptiness in one document
            // of a file still truncate the file because an earlier document
            // legitimately produced nothing" is not dictated by any jq
            // semantics `halt`/`halt_error` must uphold — mikefarah/yq (the
            // real-yq oracle used elsewhere in this codebase) does not even
            // parse `halt`/`if`/`then`/`end` the same way, so there is no
            // external contract to defer to here, only this project's own
            // considered choice to err toward preserving user data whenever
            // a halt is involved.
            // #1615 adds the `report_count` term; see `reports_before_file`.
            if (any_real_output || sink.halted().is_none())
                && sink.report_count() == reports_before_file
            {
                std::fs::write(path, &output_buffer)
                    .with_context(|| format!("failed to write to file: {}", path.display()))?;
            }

            // halt/halt_error (#791) outranks editing any further files.
            if sink.halted().is_some() {
                break 'inplace_files;
            }
        }
    } else {
        // Standard path: evaluate inputs. Both YAML and JSON input go
        // through `evaluate_yaml_direct_filtered`'s cursor-native evaluator
        // (#1398) -- see its own doc comment for why.

        // Collect input sources with their bytes and formats. `--front-matter`
        // is applied before validation, since the raw file bytes (e.g.
        // Markdown) aren't valid standalone YAML; `process` mode's body is
        // carried alongside each source for reattachment in the output loop
        // below.
        let (input_sources, validate_exit_code) = gather_input_sources(
            &input_files,
            args.input_format,
            args.front_matter,
            args.validate,
        )?;
        // #1564: each file's results are collected/emitted independently
        // of the others (the two loops below) -- process whatever passed
        // and report the failure once done, instead of losing every
        // earlier file's already-valid output the way this path used to.
        deferred_validate_exit_code = validate_exit_code;

        // Process all inputs first to collect results, then determine multi-doc status
        // This avoids double-parsing YAML for document counting
        // Each entry in all_results is a Vec of document results from one file.
        // Each result carries its parallel CommentTree (issue #710); JSON
        // input has none, so it's paired with CommentTree::empty()
        // internally by `evaluate_yaml_direct_filtered`'s `need_comments`
        // gate below.
        let mut all_results: Vec<Vec<Vec<ResultWithComments>>> = Vec::new();
        let mut global_doc_index: usize = 0;
        'collect: for (bytes, format, _) in &input_sources {
            // Yaml/Auto and Json both go through the same cursor-native
            // evaluator now (#1398) -- see the matching comment on the
            // `--split-exp` loop above for why (JSON is a YAML subset;
            // routing it through `YamlIndex`/`mark_json_sourced` keeps
            // duplicate-key handling format-independent, unlike the old
            // `parse_input`/`evaluate_input` eager-`OwnedValue` path).
            let doc_filter = args.document.map(|target| (target, global_doc_index));
            // JSON output never reads `CommentTree` (see
            // `output_value`'s JSON branch), so don't build one (#710) --
            // resolved per-source, not the invocation-wide default (#1493:
            // `Auto` must match this source's own format).
            let need_comments =
                resolve_output_format(output_config.output_format, *format) == OutputFormat::Yaml;
            let (doc_results, num_docs) = evaluate_yaml_direct_filtered(
                bytes,
                &program.expr,
                doc_filter,
                &mut sink,
                DirectEvalOptions {
                    need_comments,
                    strip_style: args.pretty_print,
                    sort_keys: output_config.sort_keys,
                    mark_json_sourced: *format == InputFormat::Json,
                },
            )?;
            global_doc_index += num_docs;
            all_results.push(doc_results);
            // halt/halt_error (#791) outranks evaluating any further
            // files — whatever was collected so far still prints below.
            if sink.halted().is_some() {
                break 'collect;
            }
        }

        // Count total documents from collected results (after filtering)
        let total_docs: usize = all_results.iter().map(std::vec::Vec::len).sum();
        let is_multi_doc = total_docs > 1;

        // Output all results with proper separators
        // For split_doc: add --- BETWEEN each result (not before first)
        // For regular multi-doc: add --- BETWEEN each YAML-resolved
        // document's results (not before the first -- `any_yaml_doc_output`
        // below, #1493 review: a pre-existing gap in this "standard"/slow
        // path specifically, only now reachable with `output_format ==
        // Yaml` for real, since any invocation that would have hit it
        // before either took the M2 fast path instead (which already
        // tracks this correctly, see `yaml_doc_streamed` in the fast-path
        // arm above) or -- for `-o=auto` specifically, pre-#1493 -- never
        // satisfied the `Yaml` check here at all).
        // For --front-matter=process: each file's own leading/closing ---
        // fences wrap its transformed front matter, followed by its
        // untouched body (carried alongside `input_sources`, gathered above).
        let mut split_doc_state = SplitDocState::new(has_split_doc);
        let mut any_yaml_doc_output = false;
        for (file_idx, doc_results) in all_results.into_iter().enumerate() {
            let front_matter_body = input_sources
                .get(file_idx)
                .and_then(|(_, _, body)| body.as_ref());
            // Per-file `OutputConfig` (#978/#1398's `json_sourced_floats`,
            // #1493's `Auto` resolution) -- both need this specific
            // source's own `InputFormat`, not the invocation-wide default,
            // since `--input-format auto` can mix JSON and YAML sources in
            // one invocation.
            let source_format = input_sources
                .get(file_idx)
                .map_or(InputFormat::Yaml, |(_, format, _)| *format);
            let file_output_config = output_config.for_source(&args, source_format);
            if front_matter_body.is_some() {
                writeln!(writer, "---")?;
            }
            // Tracks "did this file's own document loop write any real
            // output" (#1701 code review round 4), separately from
            // `any_yaml_doc_output` above (invocation-wide, drives the
            // *inter-document* `---` decision, not this file's closing
            // front-matter fence) -- mirrors the `--inplace` path's own
            // `any_real_output`.
            let mut any_output_this_file = false;
            for results in doc_results {
                // Add document separator in YAML mode for multi-doc,
                // between YAML-resolved documents only, not before the
                // first one (`any_yaml_doc_output`, see this loop's own
                // comment above) -- resolved per-file, not the
                // invocation-wide default (#1493). Tracked per *this
                // document's own resolved format*, not "any document at
                // all": a mixed-format run's JSON documents never
                // participate in the YAML `---` convention either way
                // (the condition already requires `== Yaml`), so they
                // must not count toward "has the first YAML document
                // already appeared" -- doing so made the separator
                // decision file-*order*-dependent (#1497 review): a
                // YAML document immediately after a JSON one would
                // wrongly get a leading separator, while the same YAML
                // document appearing first would not.
                let wants_doc_separator = !has_split_doc
                    && file_output_config.output_format == OutputFormat::Yaml
                    && !output_config.no_doc
                    && is_multi_doc
                    && front_matter_body.is_none()
                    && any_yaml_doc_output;
                // #1709 code review: deferred to `output_value` itself
                // under `-0`, same reason as the `--eval-all`/`--inplace`
                // arms above -- an eager write here can't yet know whether
                // this document's own first result will survive
                // `output_value`'s NUL check.
                let defer_to_nul_check = wants_doc_separator && file_output_config.nul_output;
                if wants_doc_separator && !file_output_config.nul_output {
                    // #1701 code review: same fix as
                    // `emit_yaml_doc_separator` -- the previous document's
                    // own terminator might not have been `\n`.
                    write_doc_separator_marker(
                        &mut writer,
                        terminator_from_config(&file_output_config),
                    )?;
                }
                if file_output_config.output_format == OutputFormat::Yaml && !defer_to_nul_check {
                    any_yaml_doc_output = true;
                }
                // Only the first result of this document gets a chance at
                // the deferred separator -- `defer_to_nul_check` alone is
                // constant across every result in `results`, so without
                // this, `output_value`'s own successful flush of result 1
                // would flip `any_yaml_doc_output` true in time for result
                // 2's own (unwanted) separator check to fire on it too.
                let mut first_result_in_doc = true;
                for (result, comments) in results {
                    // `split_doc` (#1709 code review) takes priority over
                    // the inter-document separator above when both apply --
                    // matches every other call site's own has_split_doc-
                    // first precedence.
                    let split_separator =
                        split_doc_state.write_separator(&mut writer, &file_output_config)?;
                    any_truthy |= !matches!(&result, OwnedValue::Null | OwnedValue::Bool(false));
                    let separator = split_separator.or_else(|| {
                        (defer_to_nul_check && first_result_in_doc).then_some(DocSeparatorArgs {
                            doc_streamed: &mut any_yaml_doc_output,
                            no_doc: output_config.no_doc,
                        })
                    });
                    output_value(
                        &mut writer,
                        &result,
                        &comments,
                        &file_output_config,
                        separator,
                    )?;
                    first_result_in_doc = false;
                    any_output_this_file = true;
                }
            }
            if let Some(body) = front_matter_body {
                let line_ending = front_matter::body_line_ending(body);
                // #1701 code review round 4: same fix as the `--inplace`
                // path's closing fence -- gated on whether this file's
                // document loop actually wrote anything (subject to
                // `-0`/`--join-output`), not applied unconditionally: with
                // zero real output, `writer` at this point holds only the
                // opening fence's own always-`\n`-terminated `writeln!`.
                if any_output_this_file {
                    write_doc_marker_newline_guard(
                        &mut writer,
                        terminator_from_config(&file_output_config),
                    )?;
                }
                writer.write_all(b"---")?;
                writer.write_all(line_ending)?;
                writer.write_all(body)?;
                // A body with no trailing line break would otherwise run
                // straight into the next file's opening fence (or whatever
                // follows), corrupting the stream -- ensure one separates
                // them, matching this body's own line-ending convention.
                if !body.is_empty() && !body.ends_with(b"\n") {
                    writer.write_all(line_ending)?;
                }
            }
        }
    }

    writer.flush()?;

    // halt/halt_error (#791) outranks everything below — every branch above
    // stops evaluating further input as soon as it's requested, but still
    // finishes writing whatever output it already had buffered/collected.
    if let Some(code) = sink.halted() {
        return Ok(code);
    }

    // #1564: a `--validate` failure on a later file, discovered up front
    // during gathering, outranks the normal success/hit/falsy
    // determination below -- but every earlier file that DID pass
    // validation was still processed/emitted above (see
    // `deferred_validate_exit_code`'s own comment). A halt during that
    // processing still wins, same precedence the M2-streaming path
    // already gives it (checked immediately above).
    if let Some(code) = deferred_validate_exit_code {
        return Ok(code);
    }

    // Determine exit code. An uncaught error outranks -e: the filter failed,
    // which is not the same as it succeeding with a falsy result (#355 vs #178).
    // yq collapses both to 1, but the diagnostic already went to stderr, so
    // reporting "no matches found" on top of it would be misleading.
    if sink.hit() {
        return Ok(DiagStyle::Yq.error_exit_code());
    }

    // yq compat: empty output and all-falsy output are the same failure,
    // reported on stderr with a fixed message and exit 1.
    if args.exit_status && !any_truthy {
        eprintln!("Error: no matches found");
        return Ok(exit_codes::FALSE_OR_NULL);
    }

    Ok(exit_codes::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Builtin::FirstStream`/`LastStream` (the "Phase 13" builtin-table
    /// spelling of `first(expr)`/`last(expr)`, distinct from the
    /// `Expr::FirstExpr`/`LastExpr` the parser actually produces --
    /// `parse_first_expr`/`parse_last_expr` are checked earlier in the
    /// primary-expression dispatch, so `try_parse_builtin`'s construction of
    /// these variants is unreachable from CLI input today) still needs the
    /// same #997 recursion as its `Expr::FirstExpr`/`LastExpr` sibling, since
    /// both arms share the exact risk this PR closes -- exercised directly
    /// here since no filter string can reach it through the parser.
    #[test]
    fn test_can_use_m2_streaming_recurses_into_builtin_first_last_stream_997() {
        let navigation = Expr::Builtin(Builtin::FirstStream(Box::new(Expr::Iterate)));
        assert!(can_use_m2_streaming(&navigation));

        let arithmetic = Expr::Builtin(Builtin::LastStream(Box::new(Expr::Arithmetic {
            op: succinctly::jq::ArithOp::Add,
            left: Box::new(Expr::Identity),
            right: Box::new(Expr::Literal(succinctly::jq::Literal::Float(1e100))),
        })));
        assert!(!can_use_m2_streaming(&arithmetic));
    }

    /// #757: `map(f)` recurses into `f` for the same reason
    /// `FirstExpr`/`LastExpr` do above -- a navigational body streams every
    /// element from its own cursor, while a computing one materializes an
    /// `OwnedValue::Float` that only the DOM path formats correctly (#997,
    /// #949, #1090). Pinned here so a later "just return `true`" simplification
    /// has to argue with a test rather than silently route arithmetic at the
    /// M2 streamers.
    #[test]
    fn test_can_use_m2_streaming_recurses_into_map_body_757() {
        for navigational in [
            Expr::Identity,
            Expr::Field("name".to_string()),
            Expr::Builtin(Builtin::Select(Box::new(Expr::Identity))),
        ] {
            let map = Expr::Builtin(Builtin::Map(Box::new(navigational)));
            assert!(can_use_m2_streaming(&map), "{map:?} should stream");
        }

        let computing = Expr::Builtin(Builtin::Map(Box::new(Expr::Arithmetic {
            op: succinctly::jq::ArithOp::Add,
            left: Box::new(Expr::Identity),
            right: Box::new(Expr::Literal(succinctly::jq::Literal::Float(1e100))),
        })));
        assert!(!can_use_m2_streaming(&computing));

        // The `Pipe` arm already requires every stage to qualify, so a
        // rejected `map` body disqualifies the whole chain it sits in.
        let piped = Expr::Pipe(vec![Expr::Field("r".to_string()), computing]);
        assert!(!can_use_m2_streaming(&piped));
    }

    #[test]
    fn test_yaml_to_owned_value_string() {
        let yaml = b"name: Alice";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        // Root is a document array, get first doc
        if let YamlValue::Sequence(docs) = root.value() {
            if let Some((doc, _)) = docs.uncons_cursor() {
                let value = yaml_to_owned_value(doc).unwrap();
                if let OwnedValue::Object(map) = value {
                    assert_eq!(
                        map.get("name"),
                        Some(&OwnedValue::String("Alice".to_string()))
                    );
                } else {
                    panic!("expected object");
                }
            }
        }
    }

    #[test]
    fn test_yaml_to_owned_value_number() {
        let yaml = b"age: 30";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Sequence(docs) = root.value() {
            if let Some((doc, _)) = docs.uncons_cursor() {
                let value = yaml_to_owned_value(doc).unwrap();
                if let OwnedValue::Object(map) = value {
                    assert_eq!(map.get("age"), Some(&OwnedValue::Int(30)));
                } else {
                    panic!("expected object");
                }
            }
        }
    }

    #[test]
    fn test_emit_yaml_value_number_literal_nan_and_infinite() {
        // A `NumberLiteral` that parses to a non-finite float (NaN or +/-
        // infinity -- reachable from a document number that overflows f64,
        // e.g. `1e400`) must render the same YAML sentinels as a plain
        // non-finite Float, not fall through to `number_str`.
        let config = OutputConfig {
            output_format: OutputFormat::Yaml,
            compact: true,
            raw_output: false,
            join_output: false,
            nul_output: false,
            ascii_output: false,
            sort_keys: false,
            no_doc: false,
            indent_str: String::new(),
            use_color: false,
            json_sourced_floats: false,
        };

        let nan = OwnedValue::NumberLiteral(NumberRepr::Float(f64::NAN), "nan".into());
        assert_eq!(
            emit_yaml_value(&nan, &CommentTree::empty(), &config, "", false),
            ".nan"
        );

        let pos_inf = OwnedValue::NumberLiteral(NumberRepr::Float(f64::INFINITY), "1e400".into());
        assert_eq!(
            emit_yaml_value(&pos_inf, &CommentTree::empty(), &config, "", false),
            ".inf"
        );

        let neg_inf =
            OwnedValue::NumberLiteral(NumberRepr::Float(f64::NEG_INFINITY), "-1e400".into());
        assert_eq!(
            emit_yaml_value(&neg_inf, &CommentTree::empty(), &config, "", false),
            "-.inf"
        );

        // #1064: the plain `Float` arm (this test's own doc comment above
        // claims parity with it, but never actually exercised it).
        assert_eq!(
            emit_yaml_value(
                &OwnedValue::Float(f64::NAN),
                &CommentTree::empty(),
                &config,
                "",
                false
            ),
            ".nan"
        );
        assert_eq!(
            emit_yaml_value(
                &OwnedValue::Float(f64::INFINITY),
                &CommentTree::empty(),
                &config,
                "",
                false
            ),
            ".inf"
        );
        assert_eq!(
            emit_yaml_value(
                &OwnedValue::Float(f64::NEG_INFINITY),
                &CommentTree::empty(),
                &config,
                "",
                false
            ),
            "-.inf"
        );
    }

    /// `in_flow: true` is never reached through any CLI-observable path
    /// today (`output_value`'s only call site always starts at `false`, and
    /// nothing downstream re-enters flow style) - `can_use_m2_streaming`'s
    /// doc comment notes there's no `--flow`-style output flag yet. Call the
    /// private helper directly, mirroring the NaN/Infinity test above, to
    /// pin the flow-style Array/Object arms' comment threading (#710):
    /// `comments.at_index`/`comments.field` must recurse correctly even
    /// though flow style never appends a trailing comment of its own (see
    /// `emit_yaml_value`'s own doc comment for why).
    #[test]
    fn test_emit_yaml_value_flow_style_threads_comments_without_appending_them() {
        let config = OutputConfig {
            output_format: OutputFormat::Yaml,
            compact: true,
            raw_output: false,
            join_output: false,
            nul_output: false,
            ascii_output: false,
            sort_keys: false,
            no_doc: false,
            indent_str: String::new(),
            use_color: false,
            json_sourced_floats: false,
        };

        let mut obj = IndexMap::new();
        obj.insert("k".to_string(), OwnedValue::Int(1));
        let value = OwnedValue::Array(vec![OwnedValue::Object(obj)]);

        let mut obj_comments = IndexMap::new();
        obj_comments.insert(
            "k".to_string(),
            CommentTree::Leaf(NodeMeta::from_comment_and_style(
                Some("# k trailing".to_string()),
                "",
            )),
        );
        let comments = CommentTree::Array(
            NodeMeta::empty(),
            vec![CommentTree::Object(
                NodeMeta::from_comment_and_style(Some("# obj trailing".to_string()), ""),
                obj_comments,
                IndexMap::new(),
            )],
        );

        // Flow style renders compactly and drops every trailing comment,
        // whether on the nested object or its field - unlike block style.
        assert_eq!(
            emit_yaml_value(&value, &comments, &config, "", true),
            "[{k: 1}]"
        );
    }

    #[test]
    fn test_yaml_to_owned_value_bool() {
        let yaml = b"active: true";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Sequence(docs) = root.value() {
            if let Some((doc, _)) = docs.uncons_cursor() {
                let value = yaml_to_owned_value(doc).unwrap();
                if let OwnedValue::Object(map) = value {
                    assert_eq!(map.get("active"), Some(&OwnedValue::Bool(true)));
                } else {
                    panic!("expected object");
                }
            }
        }
    }

    #[test]
    fn test_yaml_to_owned_value_null() {
        let yaml = b"value: null";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Sequence(docs) = root.value() {
            if let Some((doc, _)) = docs.uncons_cursor() {
                let value = yaml_to_owned_value(doc).unwrap();
                if let OwnedValue::Object(map) = value {
                    assert_eq!(map.get("value"), Some(&OwnedValue::Null));
                } else {
                    panic!("expected object");
                }
            }
        }
    }

    #[test]
    fn test_yaml_to_owned_value_flow_sequence() {
        // Flow-style sequence
        let yaml = b"items: [one, two, three]";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Sequence(docs) = root.value() {
            if let Some((doc, _)) = docs.uncons_cursor() {
                let value = yaml_to_owned_value(doc).unwrap();
                if let OwnedValue::Object(map) = value {
                    if let Some(OwnedValue::Array(arr)) = map.get("items") {
                        assert_eq!(arr.len(), 3);
                        assert_eq!(arr[0], OwnedValue::String("one".to_string()));
                        assert_eq!(arr[1], OwnedValue::String("two".to_string()));
                        assert_eq!(arr[2], OwnedValue::String("three".to_string()));
                    } else {
                        panic!("expected array for items, got {:?}", map.get("items"));
                    }
                } else {
                    panic!("expected object");
                }
            }
        }
    }

    #[test]
    fn test_yaml_to_owned_value_flow_nested() {
        // Flow-style nested mapping
        let yaml = b"person: {name: Alice, age: 30}";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Sequence(docs) = root.value() {
            if let Some((doc, _)) = docs.uncons_cursor() {
                let value = yaml_to_owned_value(doc).unwrap();
                if let OwnedValue::Object(map) = value {
                    if let Some(OwnedValue::Object(person)) = map.get("person") {
                        assert_eq!(
                            person.get("name"),
                            Some(&OwnedValue::String("Alice".to_string()))
                        );
                        assert_eq!(person.get("age"), Some(&OwnedValue::Int(30)));
                    } else {
                        panic!("expected object for person");
                    }
                } else {
                    panic!("expected object");
                }
            }
        }
    }

    #[test]
    fn test_yaml_to_owned_value_block_sequence() {
        // Block-style nested sequence (value on next line)
        let yaml = b"items:\n  - one\n  - two\n  - three";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Sequence(docs) = root.value() {
            if let Some((doc, _)) = docs.uncons_cursor() {
                let value = yaml_to_owned_value(doc).unwrap();
                if let OwnedValue::Object(map) = value {
                    if let Some(OwnedValue::Array(arr)) = map.get("items") {
                        assert_eq!(arr.len(), 3);
                        assert_eq!(arr[0], OwnedValue::String("one".to_string()));
                        assert_eq!(arr[1], OwnedValue::String("two".to_string()));
                        assert_eq!(arr[2], OwnedValue::String("three".to_string()));
                    } else {
                        panic!("expected array for items, got {:?}", map.get("items"));
                    }
                } else {
                    panic!("expected object");
                }
            }
        }
    }

    #[test]
    fn test_yaml_to_owned_value_block_nested_mapping() {
        // Block-style nested mapping (value on next line)
        let yaml = b"person:\n  name: Alice\n  age: 30";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Sequence(docs) = root.value() {
            if let Some((doc, _)) = docs.uncons_cursor() {
                let value = yaml_to_owned_value(doc).unwrap();
                if let OwnedValue::Object(map) = value {
                    if let Some(OwnedValue::Object(person)) = map.get("person") {
                        assert_eq!(
                            person.get("name"),
                            Some(&OwnedValue::String("Alice".to_string()))
                        );
                        assert_eq!(person.get("age"), Some(&OwnedValue::Int(30)));
                    } else {
                        panic!("expected object for person, got {:?}", map.get("person"));
                    }
                } else {
                    panic!("expected object");
                }
            }
        }
    }

    #[test]
    fn test_yaml_to_owned_value_deeply_nested() {
        // Deeply nested block-style structure
        let yaml = b"root:\n  level1:\n    level2:\n      value: deep";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        if let YamlValue::Sequence(docs) = root.value() {
            if let Some((doc, _)) = docs.uncons_cursor() {
                let value = yaml_to_owned_value(doc).unwrap();
                if let OwnedValue::Object(map) = value {
                    if let Some(OwnedValue::Object(level1)) = map.get("root") {
                        if let Some(OwnedValue::Object(level2)) = level1.get("level1") {
                            if let Some(OwnedValue::Object(level3)) = level2.get("level2") {
                                assert_eq!(
                                    level3.get("value"),
                                    Some(&OwnedValue::String("deep".to_string()))
                                );
                            } else {
                                panic!("expected object for level2");
                            }
                        } else {
                            panic!("expected object for level1");
                        }
                    } else {
                        panic!("expected object for root");
                    }
                } else {
                    panic!("expected object");
                }
            }
        }
    }

    // Tests for the generic evaluator integration

    /// Evaluate YAML through the production path and flatten per-document groups.
    fn eval_yaml(bytes: &[u8], expr: &Expr) -> Vec<OwnedValue> {
        eval_yaml_with_comments(bytes, expr)
            .into_iter()
            .map(|(v, _)| v)
            .collect()
    }

    /// Like [`eval_yaml`], but keeps each result's parallel `CommentTree`
    /// (issue #710) instead of discarding it.
    fn eval_yaml_with_comments(bytes: &[u8], expr: &Expr) -> Vec<ResultWithComments> {
        let (groups, _) = evaluate_yaml_direct_filtered(
            bytes,
            expr,
            None,
            &mut ErrorSink::default(),
            DirectEvalOptions {
                need_comments: true,
                strip_style: false,
                sort_keys: false,
                mark_json_sourced: false,
            },
        )
        .unwrap();
        groups.into_iter().flatten().collect()
    }

    #[test]
    fn test_evaluate_yaml_identity() {
        let yaml = b"name: Alice\nage: 30";
        let expr = Expr::Identity;
        let results = eval_yaml(yaml, &expr);

        assert_eq!(results.len(), 1);
        if let OwnedValue::Object(map) = &results[0] {
            assert_eq!(
                map.get("name"),
                Some(&OwnedValue::String("Alice".to_string()))
            );
            assert_eq!(map.get("age"), Some(&OwnedValue::Int(30)));
        } else {
            panic!("expected object, got {:?}", results[0]);
        }
    }

    #[test]
    fn test_evaluate_yaml_field() {
        let yaml = b"name: Alice\nage: 30";
        let expr = Expr::Field("name".to_string());
        let results = eval_yaml(yaml, &expr);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0], OwnedValue::String("Alice".to_string()));
    }

    #[test]
    fn test_evaluate_yaml_line_builtin() {
        use succinctly::jq::Builtin;

        let yaml = b"name: Alice\nage: 30";
        let expr = Expr::Builtin(Builtin::Line);
        let results = eval_yaml(yaml, &expr);

        assert_eq!(results.len(), 1);
        // The mapping starts at line 1
        assert_eq!(results[0], OwnedValue::Int(1));
    }

    #[test]
    fn test_evaluate_yaml_pipe() {
        let yaml = b"users:\n  - name: Alice\n  - name: Bob";
        // .users | .[0] | .name
        let expr = Expr::Pipe(vec![
            Expr::Field("users".to_string()),
            Expr::Index(0),
            Expr::Field("name".to_string()),
        ]);
        let results = eval_yaml(yaml, &expr);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0], OwnedValue::String("Alice".to_string()));
    }

    #[test]
    fn test_slurp_multi_doc_yaml() {
        // Test slurp behavior by parsing multi-doc YAML manually
        let yaml = b"---\nname: Alice\n---\nname: Bob\n---\nname: Charlie";
        let inputs = parse_input(yaml, InputFormat::Yaml).unwrap();

        // Multi-doc YAML should parse into 3 documents
        assert_eq!(inputs.len(), 3);

        // When slurped, they become an array
        let slurped = OwnedValue::Array(inputs);
        if let OwnedValue::Array(arr) = slurped {
            assert_eq!(arr.len(), 3);

            // Verify each document
            if let OwnedValue::Object(map) = &arr[0] {
                assert_eq!(
                    map.get("name"),
                    Some(&OwnedValue::String("Alice".to_string()))
                );
            } else {
                panic!("expected object");
            }
            if let OwnedValue::Object(map) = &arr[1] {
                assert_eq!(
                    map.get("name"),
                    Some(&OwnedValue::String("Bob".to_string()))
                );
            } else {
                panic!("expected object");
            }
            if let OwnedValue::Object(map) = &arr[2] {
                assert_eq!(
                    map.get("name"),
                    Some(&OwnedValue::String("Charlie".to_string()))
                );
            } else {
                panic!("expected object");
            }
        } else {
            panic!("expected array");
        }
    }

    #[test]
    fn test_slurp_with_length() {
        // Test that slurped docs can have length computed
        let yaml = b"---\nname: Alice\n---\nname: Bob\n---\nname: Charlie";
        let inputs = parse_input(yaml, InputFormat::Yaml).unwrap();
        let slurped = OwnedValue::Array(inputs);

        let expr = succinctly::jq::parse("length").unwrap();
        let results = evaluate_input(&slurped, &expr, &mut ErrorSink::default()).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0], OwnedValue::Int(3));
    }

    #[test]
    fn test_explicit_empty_key() {
        // Test explicit key syntax with empty key
        // YAML: ?\n: value
        let yaml = b"?\n: value\n";
        let inputs = parse_input(yaml, InputFormat::Yaml).unwrap();

        assert_eq!(inputs.len(), 1);
        if let OwnedValue::Object(map) = &inputs[0] {
            println!("map len: {}", map.len());
            for (k, v) in map {
                println!("  key={k:?}, value={v:?}");
            }
            assert_eq!(map.len(), 1);
            // Empty key (null key in YAML becomes empty string in our representation)
            // The key should be preserved - could be "" or "null" depending on conversion
            // Let's check what we have
            assert!(map.contains_key("") || map.contains_key("null"));
        } else {
            panic!("expected object, got {:?}", inputs[0]);
        }
    }

    #[test]
    fn test_explicit_empty_key_direct_eval() {
        // Test explicit key syntax with direct YAML evaluation
        let yaml = b"?\n: value\n";
        let expr = Expr::Identity;
        let results = eval_yaml(yaml, &expr);

        assert_eq!(results.len(), 1);
        if let OwnedValue::Object(map) = &results[0] {
            println!("direct eval map len: {}", map.len());
            for (k, v) in map {
                println!("  key={k:?}, value={v:?}");
            }
            assert_eq!(map.len(), 1, "expected 1 key but got {} keys", map.len());
        } else {
            panic!("expected object, got {:?}", results[0]);
        }
    }

    /// `depth` levels of single-element array nesting: `[[[...[Null]...]]]`.
    fn linear_array_nest(depth: usize) -> OwnedValue {
        let mut v = OwnedValue::Null;
        for _ in 0..depth {
            v = OwnedValue::Array(vec![v]);
        }
        v
    }

    /// #1005: `result_value` is a filter's evaluated output (e.g. a
    /// `reduce` accumulator growing one level per iteration), which has no
    /// adversarial *document* behind it — `reconcile_presentation` must
    /// independently refuse to recurse past the same limit rather than
    /// overflow the stack. `pristine_tree` is `CommentTree::empty()` here
    /// since only `pristine_value`'s/`result_value`'s own nesting drives
    /// the recursion depth (`CommentTree::at_index` on a non-`Array` variant
    /// falls back to the empty tree at every level, which is exactly what a
    /// computed value with no live cursor of its own already looks like).
    #[test]
    fn reconcile_presentation_panics_past_nesting_depth_limit_1005() {
        use succinctly::jq::MAX_VALUE_TREE_DEPTH;

        let under = linear_array_nest(MAX_VALUE_TREE_DEPTH - 1);
        let _ = reconcile_presentation(&under, &CommentTree::empty(), &under);

        let over = linear_array_nest(MAX_VALUE_TREE_DEPTH);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            reconcile_presentation(&over, &CommentTree::empty(), &over)
        }));
        assert!(
            result.is_err(),
            "reconcile_presentation should panic at MAX_VALUE_TREE_DEPTH"
        );
    }

    /// #1017: `emit_yaml_value` (the YAML-target default output emitter —
    /// see its own doc comment) had no guard, reachable on a value
    /// constructed via `reduce`/`foreach`/etc. with no adversarial document
    /// involved.
    #[test]
    fn emit_yaml_value_panics_past_nesting_depth_limit_1017() {
        use succinctly::jq::MAX_VALUE_TREE_DEPTH;

        let config = OutputConfig {
            output_format: OutputFormat::Yaml,
            compact: false,
            raw_output: false,
            join_output: false,
            nul_output: false,
            ascii_output: false,
            sort_keys: false,
            no_doc: false,
            indent_str: String::new(),
            use_color: false,
            json_sourced_floats: false,
        };

        let under = linear_array_nest(MAX_VALUE_TREE_DEPTH - 1);
        let _ = emit_yaml_value(&under, &CommentTree::empty(), &config, "", false);

        let over = linear_array_nest(MAX_VALUE_TREE_DEPTH);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            emit_yaml_value(&over, &CommentTree::empty(), &config, "", false)
        }));
        assert!(
            result.is_err(),
            "emit_yaml_value should panic at MAX_VALUE_TREE_DEPTH"
        );
    }

    /// `{"k":{"k":...{}...}}`, `depth` levels of `"k"` nesting, terminating
    /// in `{}` -- mirrors `eval_generic.rs`'s own `linear_nest`.
    fn linear_json_nest(depth: usize) -> String {
        format!("{}{{}}{}", "{\"k\":".repeat(depth), "}".repeat(depth))
    }

    /// #999: `to_owned_canonicalizing_numbers` is a genuinely separate
    /// recursion from `to_owned` (kept local to this file rather than
    /// added to the shared library, #999 review), so its own
    /// `assert_nesting_depth` call needed its own pinning test. 255/256,
    /// not `MAX_VALUE_TREE_DEPTH`'s 384: this walks a `DocumentCursor`
    /// directly, the same recursion shape `to_owned_at_depth` guards with
    /// the tighter `MAX_NESTING_DEPTH`, not the looser guard the old
    /// two-pass code's *second* pass (over an already-materialized
    /// `OwnedValue` tree) used.
    #[test]
    fn to_owned_canonicalizing_numbers_panics_past_nesting_depth_limit_999() {
        let json = linear_json_nest(255);
        let index = JsonIndex::build(json.as_bytes());
        let cursor = index.root(json.as_bytes());
        let owned = to_owned_canonicalizing_numbers(&cursor.value()).unwrap();
        assert!(matches!(owned, OwnedValue::Object(_)));

        let json = linear_json_nest(256);
        let index = JsonIndex::build(json.as_bytes());
        let cursor = index.root(json.as_bytes());
        let value = cursor.value();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            to_owned_canonicalizing_numbers(&value)
        }));
        assert!(
            result.is_err(),
            "to_owned_canonicalizing_numbers should panic at depth 256"
        );
    }

    /// #999: `to_owned_canonicalizing_numbers` collapses every number
    /// straight to `Int`/`Float`, unlike `to_owned`'s `NumberLiteral` --
    /// pins the exact behavior the fused single-pass replaces `to_owned` +
    /// `canonicalize_json_numbers`'s two-pass sequence with
    /// (`yq_cli_tests.rs`'s golden/CLI tests cover the end-to-end CLI
    /// behavior; this pins the function's own contract directly).
    #[test]
    fn to_owned_canonicalizing_numbers_collapses_number_literals() {
        let json = br#"{"int": 1, "float": 1.50, "exp": 1e2, "neg": -3, "arr": [1.0, 2], "s": "x", "b": true, "n": null}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let owned = to_owned_canonicalizing_numbers(&cursor.value()).unwrap();
        let OwnedValue::Object(map) = owned else {
            panic!("expected an object")
        };
        assert_eq!(map.get("int"), Some(&OwnedValue::Int(1)));
        assert_eq!(map.get("float"), Some(&OwnedValue::Float(1.5)));
        assert_eq!(map.get("exp"), Some(&OwnedValue::Float(100.0)));
        assert_eq!(map.get("neg"), Some(&OwnedValue::Int(-3)));
        assert_eq!(
            map.get("arr"),
            Some(&OwnedValue::Array(vec![
                OwnedValue::Float(1.0),
                OwnedValue::Int(2)
            ]))
        );
        assert_eq!(map.get("s"), Some(&OwnedValue::String("x".to_string())));
        assert_eq!(map.get("b"), Some(&OwnedValue::Bool(true)));
        assert_eq!(map.get("n"), Some(&OwnedValue::Null));

        // No `NumberLiteral` anywhere in the result -- every number
        // collapsed, unlike `to_owned`'s own output for the same input.
        for v in map.values() {
            assert!(
                !matches!(v, OwnedValue::NumberLiteral(..)),
                "found a NumberLiteral in canonicalized output: {v:?}"
            );
        }
    }

    /// #999 review: `to_owned_canonicalizing_numbers_at_depth`'s `as_i64`/
    /// `as_f64` fallback arms (reached when `number_literal()` returns
    /// `None`) looked unreachable via JSON at first glance -- but the
    /// semi-index scanner accepts a *wider* set of number spans than
    /// `is_valid_number`'s strict RFC 8259 check (#966's own doc comment on
    /// `number_literal()`), and `as_i64`/`as_f64` parse that same raw span
    /// with Rust's own, more lenient `str::parse`. A trailing- or
    /// leading-dot span (`5.`, `.5`) is exactly that: RFC 8259 requires a
    /// digit on both sides of the decimal point, so `is_valid_number`
    /// rejects it and `number_literal()` returns `None` -- but Rust's
    /// `f64::from_str` accepts both forms outright, so `as_f64` succeeds
    /// where the strict path declined. Confirmed live (`succinctly yq
    /// --input-format json 'to_entries'`) before writing this as a unit
    /// test: real yq itself has no opinion here (Go's own JSON reader
    /// rejects both spans as a parse error, unlike this crate's lenient
    /// semi-indexer), so this pins *this* crate's own accepted behavior,
    /// not an oracle-matched one.
    #[test]
    fn to_owned_canonicalizing_numbers_falls_back_to_as_f64_for_lenient_spans() {
        for json in [br#"{"n": 5.}"#.as_slice(), br#"{"n": .5}"#.as_slice()] {
            let index = JsonIndex::build(json);
            let cursor = index.root(json);
            let owned = to_owned_canonicalizing_numbers(&cursor.value()).unwrap();
            let OwnedValue::Object(map) = owned else {
                panic!("expected an object for {json:?}")
            };
            assert!(
                matches!(map.get("n"), Some(OwnedValue::Float(_))),
                "expected a Float for {json:?}, got {:?}",
                map.get("n")
            );
        }
    }

    /// #999 review: a lenient-but-unparseable span (multiple decimal
    /// points, a bare minus, a trailing exponent marker with no digits, ...)
    /// falls all the way through to the final `else` arm -- `as_i64`/
    /// `as_f64` both decline too, since the raw text isn't a valid number by
    /// any of Rust's own parsers either. Matches `to_owned`'s identical
    /// degrade-to-`Null` behavior for the same inputs (confirmed live).
    #[test]
    fn to_owned_canonicalizing_numbers_degrades_unparseable_spans_to_null() {
        for json in [
            br#"{"n": 1.2.3}"#.as_slice(),
            br#"{"n": 1-2}"#.as_slice(),
            br#"{"n": 1e}"#.as_slice(),
        ] {
            let index = JsonIndex::build(json);
            let cursor = index.root(json);
            let owned = to_owned_canonicalizing_numbers(&cursor.value()).unwrap();
            let OwnedValue::Object(map) = owned else {
                panic!("expected an object for {json:?}")
            };
            assert_eq!(
                map.get("n"),
                Some(&OwnedValue::Null),
                "expected Null for {json:?}, got {:?}",
                map.get("n")
            );
        }
    }

    /// #1975: `to_owned_canonicalizing_numbers` (the `parse_input` bridge
    /// behind `--input-format json` under `--slurp`/`--eval-all`) had
    /// neither the #1677 malformed-`,`/`:` delimiter check nor the #1194
    /// unpaired-tail check at all -- unlike its `eval_generic::to_owned_at_depth`
    /// sibling this function's own doc comment claims to mirror. Confirmed
    /// live before this fix: `yq --input-format json --slurp -o=json '.[0] |
    /// keys_unsorted'` on `{"a" 1, "b": 2}` silently returned `["a","b"]`
    /// with exit 0, where every other route into the evaluator (`jq`,
    /// `jq --slurp`, `yq --input-format json` without `--slurp`) correctly
    /// raised.
    #[test]
    fn to_owned_canonicalizing_numbers_raises_on_malformed_delimiter_1975() {
        for json in [
            &br#"{"a" 1, "b": 2}"#[..], // missing ':'
            &br#"{"a": 1 "b": 2}"#[..], // missing ','
            &br#"{"a"}"#[..],           // unpaired tail
        ] {
            let index = JsonIndex::build(json);
            let cursor = index.root(json);
            to_owned_canonicalizing_numbers(&cursor.value())
                .expect_err(&format!("{json:?} is not well-formed JSON"));
        }

        // Well-formed data is unaffected.
        let json = br#"{"a": 1, "b": 2}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let owned = to_owned_canonicalizing_numbers(&cursor.value()).unwrap();
        assert_eq!(
            owned,
            OwnedValue::Object(IndexMap::from([
                ("a".to_string(), OwnedValue::Int(1)),
                ("b".to_string(), OwnedValue::Int(2)),
            ]))
        );

        // The array arm has the identical gap for a missing ','.
        let json = br"[1 2, 3]";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        to_owned_canonicalizing_numbers(&cursor.value())
            .expect_err("a missing ',' between array elements is not well-formed JSON");
    }

    /// #1975 review round 1: two more gaps in the same function, found by
    /// comparing against `eval_generic::to_owned_at_depth` field-by-field
    /// rather than trusting this function's own "mirrors exactly" doc
    /// comment. Both used to silently drop or null out a structurally
    /// invalid document instead of raising, the same #1194 class the
    /// delimiter-fault fix above addresses.
    #[test]
    fn to_owned_canonicalizing_numbers_raises_on_structural_faults_1975() {
        // A key JSON's grammar never allowed at all (not a decode failure --
        // a bare, non-string key) used to be silently dropped along with its
        // value, the exact `if let Some(key) = ... {}`-no-`else` pattern
        // #1679 already fixed at five other call sites.
        let json = br#"{"a": 1, 123: 2, "b": 3}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        to_owned_canonicalizing_numbers(&cursor.value())
            .expect_err("a non-string key is not well-formed JSON");

        // A structurally malformed value token (not a decode failure -- a
        // span the semi-index could not classify as any JSON token) used to
        // materialize as `null` instead of raising.
        let json = br"[xyz123]";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        to_owned_canonicalizing_numbers(&cursor.value())
            .expect_err("an unclassifiable value token is not well-formed JSON");
    }

    /// `depth` levels of single-child `CommentTree::Array` nesting, mirroring
    /// [`linear_array_nest`]'s `OwnedValue` shape.
    fn linear_comment_tree_nest(depth: usize) -> CommentTree {
        let mut t = CommentTree::empty();
        for _ in 0..depth {
            t = CommentTree::Array(NodeMeta::empty(), vec![t]);
        }
        t
    }

    /// #1017: `strip_presentation_style` had no guard, reachable on a
    /// `CommentTree` built up alongside a computed value with no live
    /// document cursor behind it.
    #[test]
    fn strip_presentation_style_panics_past_nesting_depth_limit_1017() {
        use succinctly::jq::MAX_VALUE_TREE_DEPTH;

        let under = linear_comment_tree_nest(MAX_VALUE_TREE_DEPTH - 1);
        let _ = strip_presentation_style(&under);

        let over = linear_comment_tree_nest(MAX_VALUE_TREE_DEPTH);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            strip_presentation_style(&over)
        }));
        assert!(
            result.is_err(),
            "strip_presentation_style should panic at MAX_VALUE_TREE_DEPTH"
        );
    }

    /// `depth` levels of single-element array nesting in a `serde_json::Value`.
    fn linear_serde_json_nest(depth: usize) -> serde_json::Value {
        let mut v = serde_json::Value::Null;
        for _ in 0..depth {
            v = serde_json::Value::Array(vec![v]);
        }
        v
    }

    /// #1017: `serde_json_to_owned` (used by `--argjson`/`--args`-style CLI
    /// value parsing) had no guard of its own, unlike the `serde_json`
    /// parse step feeding it, which enforces an independent ~128-deep
    /// limit before this conversion ever runs.
    #[test]
    fn serde_json_to_owned_panics_past_nesting_depth_limit_1017() {
        use succinctly::jq::MAX_VALUE_TREE_DEPTH;

        let under = linear_serde_json_nest(MAX_VALUE_TREE_DEPTH - 1);
        let owned = serde_json_to_owned(&under);
        assert!(matches!(owned, OwnedValue::Array(_)));

        let over = linear_serde_json_nest(MAX_VALUE_TREE_DEPTH);
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| serde_json_to_owned(&over)));
        assert!(
            result.is_err(),
            "serde_json_to_owned should panic at MAX_VALUE_TREE_DEPTH"
        );
    }
}
