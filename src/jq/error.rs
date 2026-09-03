//! Evaluation errors, and the jq-compatible wording of their messages.
//!
//! Since #158 bound the raised value as `catch`'s input, an evaluator-internal
//! error's message text is readable from a filter, so the wording is part of
//! the observable surface — `try f catch (if test("Cannot index") then … end)`
//! is a real jq idiom. #356 moved `EvalError` here and gave every message jq
//! defines a named constructor, so the vocabulary is enumerable in one place
//! instead of being inlined at ~300 raise sites across the two evaluators
//! (which is how they drifted from each other as well as from jq).
//!
//! Every constructor below reproduces jq-1.7.1 byte for byte; the expectations
//! are captured from the pinned binary into `tests/data/jq-error-messages.tsv`
//! and asserted by `tests/jq_error_message_tests.rs`. Errors with no jq
//! analogue (succinctly extensions, and builtins jq does not have) keep using
//! [`EvalError::new`] with their own wording.

#[cfg(not(test))]
use alloc::format;
#[cfg(not(test))]
use alloc::string::String;

use super::eval::EvalTag;
use super::stream::{stream_owned_value_json, stream_owned_value_json_jq};
use super::value::OwnedValue;

/// Error that occurs during evaluation.
#[derive(Debug, Clone, PartialEq)]
pub struct EvalError {
    pub message: String,

    /// Either the raw payload of `error(v)`, or a tag for one of the
    /// "never suppressed by `?`/`try`/`catch`" error classes that used to be
    /// recognized only by matching `message` against a fixed set of string
    /// literals/prefixes/suffixes (#1840). That string-matching classifier
    /// already produced two real bugs from the same root cause: #1660 (a
    /// user's own `error("invalid escape sequence")` was wrongly forced
    /// uncatchable) and #1813 (a dynamic message didn't match any literal in
    /// the list, so it was wrongly left catchable). [`ErrorKind`]'s variants
    /// are checked directly instead.
    ///
    /// `None` for errors raised internally by the evaluator with no special
    /// classification (ordinary type errors and friends). jq models those as
    /// string errors, so [`EvalError::payload`] falls back to `message`
    /// wrapped in [`OwnedValue::String`].
    ///
    /// The CLI reads [`EvalError::payload_is_not_a_string`] for a second
    /// purpose: jq appends `(not a string)` to an uncaught diagnostic when
    /// the raised value is not a string, which only the payload can decide —
    /// `message` has already lost the distinction (#355).
    pub value: EvalErrorPayload,
}

/// [`EvalError::value`]'s type: a value, a classification tag, or neither.
///
/// `error(v)`'s raw payload and a "this message always means X" tag are
/// mutually exclusive in practice — every constructor that sets one never
/// sets the other — so folding both into one field costs nothing extra
/// size-wise over the `Option<OwnedValue>` this replaces:
/// `size_of::<EvalErrorPayload>()` measures identical to
/// `size_of::<Option<OwnedValue>>()`, because `OwnedValue`'s discriminant
/// already has spare niche capacity beyond `Option`'s single "empty" state.
/// A plain extra tag *field* on `EvalError` was tried once instead and
/// reverted (see the `#[cfg(test)]` regression tests
/// `eval_error_payload_is_no_larger_than_the_option_it_replaces`/
/// `eval_error_size_is_pinned_for_the_1021_stack_overflow_fix` below for why
/// that mattered) — this enum avoids that cost entirely by riding along in
/// the space `Option<OwnedValue>` already used.
#[derive(Debug, Clone, PartialEq)]
pub enum EvalErrorPayload {
    /// No structured payload -- an ordinary internal error, unclassified.
    /// [`EvalError::payload`] falls back to the message string for this
    /// variant.
    None,
    /// The raw value `error(v)` raised, verbatim -- see
    /// [`EvalError::from_value_with`].
    Value(OwnedValue),
    /// A classification tag in place of a payload -- see [`ErrorKind`].
    Kind(ErrorKind),
}

/// A classification [`EvalError::value`] can carry instead of a real payload
/// — see [`EvalErrorPayload::Kind`].
///
/// Every variant here means "never
/// suppressed by `?`, never handed to a `catch` handler" at some scope; the
/// exact scope differs per [`EvalError::is_decode_failure`]/
/// [`EvalError::is_invalid_path_expression`]/
/// [`EvalError::is_untracked_navigation_error`]'s own doc comments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// See [`EvalError::is_decode_failure`] -- always uncatchable.
    DecodeFailure,
    /// See [`EvalError::is_invalid_path_expression`] -- always uncatchable.
    InvalidPathExpression,
    /// See [`EvalError::is_untracked_navigation_error`] -- ordinarily
    /// catchable, with one narrow bare-postfix-`?` exception.
    UntrackedNavigation,
}

/// A stream terminator: what ended a sequence of outputs when it wasn't
/// simple exhaustion.
///
/// Carried by `QueryResult::Partial`/`GenericResult::Partial` (#400, #494)
/// alongside whatever outputs the stream produced before terminating, so a
/// mid-stream `error(...)` or `break $label` no longer discards the prefix
/// that came before it.
#[derive(Debug, Clone)]
pub enum Control {
    Error(EvalError),
    Break(String),
    /// `halt`/`halt_error(n)`: exit the whole process with this code. Unlike
    /// `Error`/`Break`, this must NOT be caught by `try`/`catch` or
    /// `label`/`break` (confirmed against real jq: both bypass it entirely) —
    /// carried as its own variant rather than reusing `Error` so those two
    /// handlers' existing `other => other` fallthrough passes it through
    /// unchanged without needing to special-case it.
    Halt(i32),
}

/// How an owned-evaluation helper's expression stopped: a genuine error that
/// `try`/`catch` may handle and `?` may suppress, a `break $label` unwinding
/// to some enclosing `label` (#824), or a `halt` that nothing may catch
/// (#791).
///
/// This is the error type of `result_to_owned`, `eval_owned_multi` and the
/// other `eval.rs` helpers that evaluate a sub-expression to owned values. An
/// earlier design smuggled a halt through [`EvalError`] behind a marker field,
/// which made correctness opt-in at every call site: the natural
/// `Err(e) => QueryResult::Error(e)` silently turned a halt into a catchable
/// error, and review kept finding missed sites. Carrying the two cases as
/// distinct variants makes that mistake unrepresentable — an `EvalError` can
/// no longer *be* a halt, so only an explicit wildcard arm can misroute one.
/// `Break` was added later for the same reason: before #824, every consumer
/// of this type had no choice but to fold a `Control::Break` it received into
/// a synthetic `EvalError::new("break $label not in label")` — correct only
/// when nothing outside actually catches it, and wrong (loud, wrongly-worded,
/// wrongly-exit-coded) whenever a matching `label` sits further out, e.g.
/// across a `path(...)` call boundary. `eval_owned_multi`/
/// `eval_owned_multi_first` and `resolve_node`'s own arms (in `eval.rs`)
/// were the first to propagate a real `Break`, for `path()` and the
/// computed-key path this type also drives for `=`/`|=`/`del()`. `#833`
/// closed the far broader remaining gap: `result_to_owned` and
/// `eval_owned_expr` (used by dozens of builtins' argument evaluation, not
/// just path context) now propagate a *bare* unmatched `Break` too, matching
/// this type's `eval_owned_expr_ctrl`/`eval_owned_expr` split (#575's
/// precedent: a `_ctrl`-suffixed twin that preserves [`Control`] losslessly
/// exists at the few call sites that need it). One shape is still open,
/// though: when the argument generator produces one or more values *before*
/// breaking/erroring (`QueryResult::Partial`), `result_to_owned` still
/// silently takes the first value and drops the trailing escape — tracked as
/// #1164, since fixing it means every caller becoming `Partial`-aware itself,
/// not something this function can solve alone; see its own
/// `Partial(vs, _control)` arm in `eval.rs` for the full rationale.
/// `eval_owned_expr_ctrl` used to share this gap too, but #1559 fixed it to
/// propagate the trailing escape as its own `Err` instead.
///
/// Consumers should write `Err(EvalEscape::Error(e))` for the catchable case
/// and let everything else flow through the `From` conversions into
/// `QueryResult`/[`Control`], which preserve `Halt`/`Break` by construction.
/// Never write `Err(_) => …-that-discards` — that is the one remaining way to
/// lose a halt or a break addressed to an outer label.
#[derive(Debug, Clone, PartialEq)]
pub enum EvalEscape {
    /// A genuine evaluation error — catchable by `try`/`catch`, suppressible
    /// by `?`.
    Error(EvalError),
    /// `break $label`, still looking for a `label $label` to catch it (#824,
    /// #833). A consumer that has not been specifically updated to
    /// propagate this converts it to a synthetic "break $label not in
    /// label" `Error` instead — see this type's own doc comment for which
    /// consumers still do that.
    Break(String),
    /// `halt`/`halt_error(n)`: exit the whole process with this code. Must
    /// reach the CLI unconditionally; never catchable, never suppressible.
    Halt(i32),
}

impl From<EvalError> for EvalEscape {
    fn from(e: EvalError) -> Self {
        Self::Error(e)
    }
}

impl From<EvalError> for Control {
    fn from(e: EvalError) -> Self {
        Self::Error(e)
    }
}

impl From<Control> for EvalEscape {
    fn from(control: Control) -> Self {
        match control {
            Control::Error(e) => Self::Error(e),
            Control::Break(label) => Self::Break(label),
            Control::Halt(code) => Self::Halt(code),
        }
    }
}

impl From<EvalEscape> for Control {
    fn from(escape: EvalEscape) -> Self {
        match escape {
            EvalEscape::Error(e) => Self::Error(e),
            EvalEscape::Break(label) => Self::Break(label),
            EvalEscape::Halt(code) => Self::Halt(code),
        }
    }
}

/// jq truncates values embedded in error messages to a fixed-width buffer
/// (`jv_dump_string_trunc` with `char errbuf[15]`): a JSON dump of at most
/// `DUMP_BUDGET` bytes is used verbatim, anything longer keeps its first
/// `DUMP_KEEP` bytes and gains a `...` suffix.
const DUMP_BUDGET: usize = 14;
const DUMP_KEEP: usize = 11;

/// #2179: the `path()`-family "Invalid path expression..." messages don't
/// share `DUMP_BUDGET`/`DUMP_KEEP` with the `<type> (<dump>)`-shaped ones
/// above -- confirmed by reading jq 1.7.1's C source (`execute.c`), not
/// assumed: `PATH_END`'s `Invalid path expression with result %s` and the
/// `INDEX`/`EACH` "near attempt to..." messages all call
/// `jv_dump_string_trunc` with `char errbuf[30]` (`objbuf[30]` for
/// `near_access`'s *container* argument specifically -- its *element*/key
/// argument still uses `char keybuf[15]`, i.e. the narrow `DUMP_KEEP` above,
/// unchanged). `errbuf[30]` gives `DUMP_BUDGET_WIDE` = 29, `DUMP_KEEP_WIDE`
/// = 26 by the identical `bufsize - 1`/`bufsize - 4` arithmetic
/// `jv_dump_string_trunc` uses for every buffer size. Live-verified against
/// the pinned jq 1.7.1 oracle for all three message shapes (a 26-byte-kept
/// dump for the wide ones, an unchanged 11-byte-kept dump for
/// `near_access`'s own element argument).
const DUMP_BUDGET_WIDE: usize = 29;
const DUMP_KEEP_WIDE: usize = 26;

/// A value's JSON dump, truncated the way jq truncates it.
///
/// Unlike jq — which builds the entire dump with `jv_dump_string` and then
/// discards all but `DUMP_BUDGET` bytes of it — this streams into a
/// [`PreviewSink`] and stops as soon as the answer is settled, so previewing a
/// mismatched 100 MB operand copies 14 bytes rather than the whole document
/// (#358).
///
/// It streams via [`stream_owned_value_json_jq`] — the jq-convention writer from
/// [`super::escape`] — rather than the `StreamableValue` impl, which escapes the
/// way `yq` does. The two differ at `\b`, `\f` and DEL, which a jq error message
/// has to render jq's way. (#358 used the `StreamableValue` impl here because it
/// was the only writer that left C1 raw; #385 fixed the C1 handling everywhere
/// and split the two conventions apart, so the correct writer is now available
/// by name.)
///
/// One deviation remains, about where the dump is cut rather than what gets
/// dumped: jq cuts at a byte offset and can therefore split a multi-byte
/// character, emitting invalid UTF-8; a Rust `String` cannot hold that, so we
/// snap back to the nearest character boundary. The two agree except when a
/// multi-byte character straddles byte `DUMP_KEEP` of the dump — see
/// `docs/compliance/jq/limitations.md`.
///
/// jq-pinned shim (#1055): delegates to [`dump_truncated_with`] fixed at
/// [`EvalTag::Jq`]. Migrate a call site to `dump_truncated_with(S::TAG, ..)`
/// once `S: EvalSemantics` is in scope there; this shim (and [`describe`]
/// below) can be deleted once every call site has moved.
fn dump_truncated(value: &OwnedValue) -> String {
    dump_truncated_with(EvalTag::Jq, value)
}

/// Mode-aware value preview (#1055): yq mode echoes a `NumberLiteral`
/// verbatim, the same as every other yq-mode output path (`tostring`,
/// `@json`, ...), instead of reformatting it via jq's rules --
/// `stream_owned_value_json` is #1008's own yq real-output convention,
/// reused here exactly as-is (not a bespoke error-message variant): no new
/// formatter is needed, only a dispatch. Both modes still share the same
/// truncation budget and boundary-snapping behavior below.
fn dump_truncated_with(tag: EvalTag, value: &OwnedValue) -> String {
    dump_truncated_at(tag, value, DUMP_BUDGET, DUMP_KEEP)
}

/// #2179's wide sibling of [`dump_truncated`], for the `path()`-family
/// "Invalid path expression..." messages -- jq-pinned shim (#1055), same
/// shape and same eventual `S::TAG` migration as [`dump_truncated`] itself.
fn dump_truncated_wide(value: &OwnedValue) -> String {
    dump_truncated_wide_with(EvalTag::Jq, value)
}

/// Mode-aware sibling of [`dump_truncated_wide`] (#1055): see
/// [`dump_truncated_with`]'s doc comment -- the same dispatch, just at
/// [`DUMP_BUDGET_WIDE`]/[`DUMP_KEEP_WIDE`] instead.
fn dump_truncated_wide_with(tag: EvalTag, value: &OwnedValue) -> String {
    dump_truncated_at(tag, value, DUMP_BUDGET_WIDE, DUMP_KEEP_WIDE)
}

/// The truncation logic [`dump_truncated_with`]/[`dump_truncated_wide_with`]
/// share, parameterized on the `(budget, keep)` pair jq's own
/// `jv_dump_string_trunc` derives from its caller's buffer size (`bufsize -
/// 1`/`bufsize - 4` respectively -- see [`DUMP_KEEP_WIDE`]'s own doc
/// comment for the arithmetic).
fn dump_truncated_at(tag: EvalTag, value: &OwnedValue, budget: usize, keep: usize) -> String {
    let mut sink = PreviewSink::new(budget);
    // The sink stops the writer once the dump is known to exceed the budget;
    // writing into a `String` cannot fail for any other reason, so the returned
    // `Result` carries nothing `sink.overflowed` has not already recorded.
    let _ = stream_value_preview(tag, value, &mut sink);
    if !sink.overflowed {
        return sink.buf;
    }
    sink.truncate_to(keep);
    sink.buf.push_str("...");
    sink.buf
}

/// The one dispatch every `EvalTag`-aware preview call site shares (#1055):
/// jq's error-message convention, or yq's real-output one reused as-is (see
/// [`dump_truncated_with`]'s doc comment for why no bespoke yq formatter is
/// needed). Factored out so [`dump_truncated_with`] and
/// [`EvalError::from_value_with`] can't drift apart on it.
fn stream_value_preview<W: core::fmt::Write>(
    tag: EvalTag,
    value: &OwnedValue,
    out: &mut W,
) -> core::fmt::Result {
    match tag {
        EvalTag::Jq => stream_owned_value_json_jq(value, out),
        EvalTag::Yq => stream_owned_value_json(value, out, 0, 0, ' ', false),
    }
}

/// A [`core::fmt::Write`] sink that keeps the first `cap` bytes written to it and
/// then stops the writer, so [`dump_truncated`] costs a constant amount of work
/// however large the offending value is.
///
/// Two details carry the bound and the safety:
///
/// * It clamps the copy even within a *single* oversized `write_str`. A long
///   JSON string arrives from the streaming writer as one span, so clamping the
///   copy — not just refusing later writes — is what actually bounds the work.
/// * `buf` stays valid UTF-8: an oversized span is cut back to a `char`
///   boundary. jq's `strncpy` emits the split bytes instead, which is the one
///   place the preview is allowed to come out shorter than jq's.
struct PreviewSink {
    /// The retained prefix; never longer than `cap`, always valid UTF-8.
    buf: String,
    /// Byte budget for `buf`.
    cap: usize,
    /// Whether anything was dropped — i.e. the dump was longer than `cap`.
    overflowed: bool,
}

impl PreviewSink {
    fn new(cap: usize) -> Self {
        Self {
            buf: String::with_capacity(cap),
            cap,
            overflowed: false,
        }
    }

    /// Cut `buf` back to at most `len` bytes, on a `char` boundary.
    fn truncate_to(&mut self, len: usize) {
        let mut cut = len.min(self.buf.len());
        while cut > 0 && !self.buf.is_char_boundary(cut) {
            cut -= 1;
        }
        self.buf.truncate(cut);
    }
}

impl core::fmt::Write for PreviewSink {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let room = self.cap - self.buf.len();
        if s.len() <= room {
            self.buf.push_str(s);
            return Ok(());
        }
        // More arrived than fits: take what we can, on a boundary, and stop the
        // traversal — nothing further can change the preview.
        let mut cut = room;
        while cut > 0 && !s.is_char_boundary(cut) {
            cut -= 1;
        }
        self.buf.push_str(&s[..cut]);
        self.overflowed = true;
        Err(core::fmt::Error)
    }
}

/// How jq names a value in an error message: `number (1)`, `string ("ab")`.
///
/// jq-pinned shim (#1055): see [`dump_truncated`]'s doc comment.
fn describe(value: &OwnedValue) -> String {
    describe_with(EvalTag::Jq, value)
}

/// Mode-aware sibling of [`describe`] (#1055): see [`dump_truncated_with`].
fn describe_with(tag: EvalTag, value: &OwnedValue) -> String {
    format!(
        "{} ({})",
        value.type_name(),
        dump_truncated_with(tag, value)
    )
}

/// Go-`%q`-style raw-byte quoting for [`EvalError::urid_invalid_escape`]
/// (#1216) -- unlike [`dump_truncated`]/[`describe`] above, this quotes
/// arbitrary bytes that may not even be valid UTF-8, so it can't reuse
/// `Debug` on a `&str` directly the way those do.
///
/// Walks `raw` left to right: a maximal run of complete, valid UTF-8
/// characters is quoted via Rust's own `Debug` escaping (stripping the
/// `Debug`-added surrounding quotes, since the caller wraps the whole
/// result once) -- already confirmed to match real yq's Go-style quoting
/// for embedded `"`/`\`/control characters exactly. Any byte that isn't
/// part of a complete valid character (a genuinely invalid byte, or a
/// multi-byte sequence truncated by running out of input) renders as
/// `\xHH`, one escape per raw byte -- matching real yq's own behavior for
/// a `%`-escape this close to the end of the input to leave a multi-byte
/// character split mid-sequence (verified live: a 3-byte character
/// truncated to its first two bytes by `@urid`'s 2-trailing-byte error
/// window renders as `\xe4\xb8`, not the whole character and not a lossy
/// replacement).
fn quote_bytes_go_style(raw: &[u8]) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    out.push('"');
    let mut i = 0;
    while i < raw.len() {
        match core::str::from_utf8(&raw[i..]) {
            // The rest of `raw` is entirely valid UTF-8 -- quote it all at
            // once and stop.
            Ok(s) => {
                push_debug_escaped(&mut out, s);
                break;
            }
            Err(e) => {
                let valid_len = e.valid_up_to();
                if valid_len > 0 {
                    // `valid_up_to()` always lands on a char boundary, so
                    // this slice is guaranteed valid UTF-8.
                    let s = core::str::from_utf8(&raw[i..i + valid_len])
                        .expect("valid_up_to() guarantees this prefix is valid UTF-8");
                    push_debug_escaped(&mut out, s);
                    i += valid_len;
                }
                // `error_len()` is `Some(n)` for n genuinely invalid bytes
                // at this position (escape exactly those and keep
                // scanning), or `None` when the remaining bytes are a
                // valid *prefix* of some multi-byte character that simply
                // ran out of input -- the truncated-character case this
                // function exists for -- in which case every remaining
                // byte is escaped and there's nothing left to scan.
                match e.error_len() {
                    Some(n) => {
                        for b in &raw[i..i + n] {
                            out.push_str(&format!("\\x{b:02x}"));
                        }
                        i += n;
                    }
                    None => {
                        for b in &raw[i..] {
                            out.push_str(&format!("\\x{b:02x}"));
                        }
                        i = raw.len();
                    }
                }
            }
        }
    }
    out.push('"');
    out
}

/// Append `s`'s `Debug`-escaped contents to `out`, without the surrounding
/// quotes `Debug` adds (the caller supplies its own, once, around the
/// whole message) -- shared by [`quote_bytes_go_style`]'s two call sites
/// (the all-valid fast path and the valid-prefix-before-a-truncation
/// case) so they can't independently drift on how the strip is done.
fn push_debug_escaped(out: &mut String, s: &str) {
    let debug = format!("{s:?}");
    // `Debug` on `&str` always wraps in a literal, single-byte `"` at each
    // end, so trimming exactly one byte off each side can't split a
    // multi-byte escape sequence it emitted in between.
    out.push_str(&debug[1..debug.len() - 1]);
}

/// The binary operators jq names in arithmetic error messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
}

impl BinOp {
    /// jq's past participle for this operator. Note `%` reports itself as
    /// division: `1 % "a"` is "cannot be divided (remainder)".
    fn participle(self) -> &'static str {
        match self {
            Self::Add => "added",
            Self::Subtract => "subtracted",
            Self::Multiply => "multiplied",
            Self::Divide => "divided",
            Self::Modulo => "divided (remainder)",
        }
    }
}

impl EvalError {
    /// Create a new evaluation error with a message.
    ///
    /// The error carries no structured payload, so `catch` sees the message as
    /// a string. Use [`EvalError::from_value`] for `error(v)`, where jq
    /// preserves `v` verbatim.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            value: EvalErrorPayload::None,
        }
    }

    /// Create an error tagged with a classification [`ErrorKind`] instead of
    /// a real payload (#1840) -- the base constructor for
    /// [`Self::decode_failure`]/[`Self::colliding_display_key`]/
    /// [`Self::invalid_path_expression`]/
    /// [`Self::invalid_path_expression_near_access`]/
    /// [`Self::invalid_path_expression_near_iterate`], the same way
    /// [`Self::new`] is the base for every plain, unclassified error.
    fn with_kind(message: impl Into<String>, kind: ErrorKind) -> Self {
        Self {
            message: message.into(),
            value: EvalErrorPayload::Kind(kind),
        }
    }

    /// Create an error that raises `value` as its payload, as `error(v)` does.
    ///
    /// The message renders `value` the way jq reports it: a string payload is
    /// used as-is (`error("boom")` reports `boom`, not `"boom"`), anything else
    /// is serialized via the jq-error-message convention (`stream_owned_value_json_jq`,
    /// the same one `describe`/`dump_truncated` use — not [`OwnedValue::to_json`],
    /// whose RFC-8259-safe `null` for a non-finite float is wrong here: `error(infinite)`
    /// reports jq's real `DBL_MAX` text, not `null` (#930)). Unlike `dump_truncated`,
    /// this has no length budget — `error(v)`'s message is the whole value, verbatim,
    /// same as real jq.
    ///
    /// jq-pinned shim (#1055): delegates to [`EvalError::from_value_with`]
    /// fixed at `EvalTag::Jq` — use that constructor directly once
    /// `S: EvalSemantics` is in scope at the call site.
    pub fn from_value(value: OwnedValue) -> Self {
        Self::from_value_with(EvalTag::Jq, value)
    }

    /// Mode-aware sibling of [`EvalError::from_value`] (#1055): yq mode
    /// echoes a `NumberLiteral` verbatim here too, matching `.a | tostring`
    /// on the same value instead of reformatting it via jq's rules.
    pub fn from_value_with(tag: EvalTag, value: OwnedValue) -> Self {
        let message = match &value {
            OwnedValue::String(s) => s.clone(),
            other => {
                let mut message = String::new();
                let _ = stream_value_preview(tag, other, &mut message);
                message
            }
        };
        Self {
            message,
            value: EvalErrorPayload::Value(value),
        }
    }

    /// The value this error raises, as `catch` should see it.
    ///
    /// Errors from `error(v)` return `v` unchanged; internal errors (no
    /// payload, or a [`ErrorKind`] classification tag) return their message
    /// as a string, matching how jq raises them.
    pub fn payload(self) -> OwnedValue {
        match self.value {
            EvalErrorPayload::Value(v) => v,
            EvalErrorPayload::None | EvalErrorPayload::Kind(_) => OwnedValue::String(self.message),
        }
    }

    /// Whether the raised payload was something other than a string.
    ///
    /// Drives jq's `(not a string)` marker on an uncaught error. Internal
    /// errors (no payload, or a classification tag) are message-shaped and
    /// therefore never flagged.
    pub fn payload_is_not_a_string(&self) -> bool {
        matches!(&self.value, EvalErrorPayload::Value(v) if !matches!(v, OwnedValue::String(_)))
    }

    /// Create a type error.
    ///
    /// This is succinctly's own wording, kept for the error sites that have no
    /// jq counterpart. Anything jq also reports should use one of the named
    /// constructors below instead, so it matches byte for byte.
    pub fn type_error(expected: &str, got: &str) -> Self {
        Self::new(format!("expected {expected}, got {got}"))
    }

    // ===== jq message shapes ============================================
    //
    // Grouped by sentence shape rather than by call site, because jq reuses
    // one sentence across many operations: `.a`, `.a = 1`, `del(.a)`,
    // `getpath(["a"])` and `. as {a:$a}` all report "Cannot index … with
    // string \"a\"".

    /// `Cannot index <container> with string "<key>"`, or, for a non-string
    /// key, `Cannot index <container> with <key type>`.
    ///
    /// jq embeds the key's text only when it is a string, and does not
    /// truncate it. A slice reports its key as `object`, because jq models
    /// `.[a:b]` as indexing with `{"start":a,"end":b}`.
    pub fn cannot_index(container_type: &str, key: &OwnedValue) -> Self {
        match key {
            OwnedValue::String(k) => {
                Self::new(format!("Cannot index {container_type} with string \"{k}\""))
            }
            other => Self::cannot_index_with_type(container_type, other.type_name()),
        }
    }

    /// `Cannot index <container> with <key type>`, for call sites that know
    /// the key's kind but do not have the value to hand.
    pub fn cannot_index_with_type(container_type: &str, key_type: &str) -> Self {
        Self::new(format!("Cannot index {container_type} with {key_type}"))
    }

    /// `Cannot index <container> with string "<key>"`, for the common case of
    /// a field access whose key is already a `&str`.
    pub fn cannot_index_with_field(container_type: &str, key: &str) -> Self {
        Self::new(format!(
            "Cannot index {container_type} with string \"{key}\""
        ))
    }

    /// `Cannot use <type> (<v>) as object key`.
    ///
    /// jq builds an object key from an arbitrary expression and refuses a
    /// non-string one where it would be inserted, so `{(0):1}` and
    /// `[{"key":0}] | from_entries` report the same sentence — jq *defines*
    /// `from_entries` as `map({(.key // .Key // .name // .Name): …})`, so the
    /// two are one raise site in jq and one constructor here (#391).
    pub fn cannot_use_as_object_key(key: &OwnedValue) -> Self {
        Self::new(format!("Cannot use {} as object key", describe(key)))
    }

    /// `invalid UTF-8 in comment`.
    ///
    /// Raised by the `line_comment` builtin (issue #797) when a node's
    /// trailing comment bytes aren't valid UTF-8. Mirrors
    /// `YamlValue::Error("invalid UTF-8 in anchor name")`'s handling of the
    /// identical situation for anchor names, so "invalid" surfaces as an
    /// error here too, rather than silently reading back as "no comment".
    pub fn invalid_utf8_in_comment() -> Self {
        Self::new("invalid UTF-8 in comment")
    }

    /// `Path must be specified as an array`.
    ///
    /// Raised by the path builtins when their path argument is not an array at
    /// all — `1 | setpath("a"; 1)`.
    pub fn path_must_be_array() -> Self {
        Self::new("Path must be specified as an array")
    }

    /// `Paths must be specified as an array`.
    ///
    /// `delpaths`'s own whole-argument shape refusal — `delpaths(1)`,
    /// `delpaths(null)`. Spelled with the plural noun, distinct from
    /// [`Self::path_must_be_array`], which is `setpath`/`getpath`'s sentence
    /// for the same shape of mistake.
    pub fn paths_must_be_array() -> Self {
        Self::new("Paths must be specified as an array")
    }

    /// `expected a single result but found <n>`.
    ///
    /// yq mode's refusal of a multi-output `setpath`/`delpaths` argument
    /// (#1279). Real yq refuses these too, unlike its `has`/`test`/`sub`,
    /// which quietly take the argument's first output — live-verified against
    /// the pinned v4.53.3, which answers `SETPATH: expected single path but
    /// found 2 results instead`. succinctly's wording is its own; matching
    /// yq's exactly needs the per-slot spelling yq uses (`single path` vs
    /// `single value on RHS`) and is tracked separately. What matters here is
    /// that yq mode keeps *erroring* rather than starting to fan out or
    /// silently truncate.
    pub fn single_argument_result_required(found: usize) -> Self {
        Self::new(format!("expected a single result but found {found}"))
    }

    /// `Path must be specified as array, not <type>`.
    ///
    /// `delpaths(paths)` raises this for one entry of `paths` that is not
    /// itself an array — `delpaths([0])`, `delpaths(["a"])`. Checked over the
    /// whole list before any deletion runs, so a bad entry anywhere refuses
    /// the call rather than deleting the entries that sort ahead of it.
    pub fn path_must_be_array_not(type_name: &str) -> Self {
        Self::new(format!("Path must be specified as array, not {type_name}"))
    }

    /// `DELPATHS: expected either a !!str or !!int in the path, found <tag> instead`.
    ///
    /// yq-mode-only. Real yq's `delpaths()` accepts only `!!str`/`!!int`
    /// path components — every other YAML type errors here, `tag` naming
    /// whichever one was actually found. Originally #1162 (a slice-
    /// descriptor path component, `{"start":s,"end":e}`, `path(.[a:b])`'s
    /// own output shape — unlike real jq, which accepts one at any position
    /// and splices through the named sub-range) scoped this to just
    /// `!!map`; #1220 found the real rule is this much broader one —
    /// `!!null`/`!!bool`/`!!float`/`!!seq` all hit the identical message,
    /// substituting their own tag. Verified live against yq v4.53.3 for
    /// every variant, at both a top-level and a nested position, including
    /// that a plain `!!int` is accepted but a whole-number-valued `!!float`
    /// (`1.0`) is not.
    pub fn delpaths_rejects_type(tag: &str) -> Self {
        Self::new(format!(
            "DELPATHS: expected either a !!str or !!int in the path, found {tag} instead"
        ))
    }

    /// `Cannot delete fields from <type>`.
    ///
    /// `delpaths`/`del` reached a scalar (other than `null`) as the container
    /// to delete a key from — `1 | delpaths([[0]])`.
    pub fn cannot_delete_fields_from(type_name: &str) -> Self {
        Self::new(format!("Cannot delete fields from {type_name}"))
    }

    /// `Cannot delete <type> field of object`.
    ///
    /// `delpaths`/`del` named an object field to delete with a non-string key
    /// — `{"a":1} | delpaths([[0]])`.
    pub fn cannot_delete_field_of_object(key_type: &str) -> Self {
        Self::new(format!("Cannot delete {key_type} field of object"))
    }

    /// `Cannot delete <type> element of array`.
    ///
    /// `delpaths`/`del` named an array element to delete with a non-number
    /// key — `[1,2] | delpaths([["a"]])`.
    pub fn cannot_delete_element_of_array(key_type: &str) -> Self {
        Self::new(format!("Cannot delete {key_type} element of array"))
    }

    /// `Out of bounds negative array index`.
    ///
    /// jq raises this for a negative index that is still negative after being
    /// resolved against the array's length — `[1,2] | setpath([-5]; 9)`. The
    /// sentence carries neither the index nor the length.
    pub fn out_of_bounds_negative_index() -> Self {
        Self::new(Self::OUT_OF_BOUNDS_NEGATIVE_INDEX)
    }

    const OUT_OF_BOUNDS_NEGATIVE_INDEX: &'static str = "Out of bounds negative array index";
    const SLICE_ASSIGN_NON_ARRAY: &'static str =
        "A slice of an array can only be assigned another array";
    const CANNOT_UPDATE_STRING_SLICES: &'static str = "Cannot update string slices";

    /// Whether this is one of the write-time *application* checks jq's `?`
    /// does not suppress, unlike every other indexing error it raises (`?`
    /// only suppresses a failure to *reach* a target while collecting a
    /// path; once a write is confirmed to land somewhere, a mismatch in what
    /// gets written there survives even an inline `?`). Three messages
    /// qualify, each confirmed live against jq 1.7.1: [`Self::out_of_bounds_negative_index`]
    /// (`.a[-5]? = 9` still raises), [`Self::slice_assign_non_array`]
    /// (`.a[0:1]? = 9`, a non-array RHS, still raises), and
    /// [`Self::cannot_update_string_slices`] (`"str"[0:1]? = "x"` still
    /// raises) — #498, #1303.
    ///
    /// Contrast a genuinely *non-sliceable* target (`true[0:1]? = 9`) — that
    /// one **is** suppressed, since it's a navigation failure (the slice
    /// never applies to a boolean at all), not a write-time application one;
    /// `through_slice`'s own `_ if optional => Ok(())` arm already handles
    /// that case correctly and is unaffected by this predicate.
    ///
    /// The lone call site that needs to tell these apart from an ordinary
    /// (suppressible) navigation failure is `set_path`'s `Expr::Optional`
    /// arm in `eval.rs`.
    pub fn is_write_time_application_error(&self) -> bool {
        self.message == Self::OUT_OF_BOUNDS_NEGATIVE_INDEX
            || self.message == Self::SLICE_ASSIGN_NON_ARRAY
            || self.message == Self::CANNOT_UPDATE_STRING_SLICES
    }

    /// `Array/string slice indices must be integers`.
    ///
    /// The slice path component `{"start":s,"end":e}` was malformed. jq wants
    /// *both* keys present — an explicit `null` counts, a missing one does not
    /// — and each holding a number or `null`, so `[1,2] | setpath([{"foo":1}];
    /// 9)` and `[1,2,3] | delpaths([[{}]])` both land here. Extra keys are
    /// ignored.
    pub fn slice_indices_not_integers() -> Self {
        Self::new("Array/string slice indices must be integers")
    }

    /// `A slice of an array can only be assigned another array`.
    ///
    /// Writing through a slice splices the replacement in element by element,
    /// so it has to be an array: `[1,2,3] | .[1:2] = "x"`. Raised on whatever
    /// reaches the slice, which for a deeper path is the *result* of the rest
    /// of the walk — `null | setpath([{"start":0,"end":1},"a"]; 9)` builds
    /// `{"a":9}` and refuses it here.
    pub fn slice_assign_non_array() -> Self {
        Self::new(Self::SLICE_ASSIGN_NON_ARRAY)
    }

    /// `Cannot update string slices`.
    ///
    /// jq reads a string slice but will not write one back, whatever the
    /// replacement: `"abcdef" | .[1:2] = "x"`, `|= "x"`, and the `setpath`
    /// spelling all report this.
    pub fn cannot_update_string_slices() -> Self {
        Self::new(Self::CANNOT_UPDATE_STRING_SLICES)
    }

    /// `Cannot iterate over <type> (<value>)`.
    ///
    /// The jq-pinned shim this once forwarded from (#1055) is gone: #1494
    /// finished migrating every production call site to pass its own real
    /// `S::TAG` instead of a hardcoded `EvalTag::Jq`.
    ///
    /// jq-mode only, despite taking a `tag` parameter for the value-preview
    /// convention (#1494/#1900): #1901 found real yq doesn't use this
    /// template at all for `any`/`all`/`flatten`/`group_by`/`unique`/
    /// `unique_by`/`from_entries` -- see [`Self::yq_only_supports_arrays`],
    /// [`Self::yq_only_arrays_supported_for`], and
    /// [`Self::yq_from_entries_requires_array`] for real yq's own wording,
    /// confirmed live against v4.53.3.
    pub fn cannot_iterate_with(tag: EvalTag, value: &OwnedValue) -> Self {
        Self::new(format!("Cannot iterate over {}", describe_with(tag, value)))
    }

    /// `<builtin> only supports arrays, was <tag>` (#1901).
    ///
    /// Real yq's own wording for `any`/`all` on a non-array (object or
    /// scalar) -- confirmed live against yq v4.53.3: `5 | any` is `"any only
    /// supports arrays, was !!int"`, `{"a":1} | any` is `"any only supports
    /// arrays, was !!map"`. Unlike jq (whose `any`/`all` iterate an object's
    /// *values*), real yq rejects an object the same as a scalar -- so this
    /// covers both, not just the scalar case `cannot_iterate_with` was
    /// reached for.
    pub fn yq_only_supports_arrays(builtin: &str, tag: &str) -> Self {
        Self::new(format!("{builtin} only supports arrays, was {tag}"))
    }

    /// `only arrays are supported for <op>` (#1901).
    ///
    /// Real yq's own wording for `flatten`/`group_by`/`unique`/`unique_by`
    /// on a non-array -- confirmed live against yq v4.53.3: `5 | flatten` is
    /// `"only arrays are supported for flatten"`, `5 | group_by(.)` is
    /// `"...for group by"`, and both `5 | unique` and `5 | unique_by(.)` are
    /// `"...for unique"` (`unique_by` does **not** get its own `"unique
    /// by"` wording -- confirmed live, not a typo here). No YAML tag in this
    /// one, unlike [`Self::yq_only_supports_arrays`] -- real yq's own
    /// message doesn't name the offending type for these four.
    pub fn yq_only_arrays_supported_for(op: &str) -> Self {
        Self::new(format!("only arrays are supported for {op}"))
    }

    /// `from entries only runs against arrays` (#1901).
    ///
    /// Real yq's own wording for `from_entries` on a non-array -- confirmed
    /// live against yq v4.53.3, for both an object and a scalar input.
    pub fn yq_from_entries_requires_array() -> Self {
        Self::new("from entries only runs against arrays")
    }

    /// `index [N] out of range, array size is M` (#2254).
    ///
    /// Real yq's own wording for a negative array index whose magnitude
    /// still exceeds the array length after resolving against it -- unlike
    /// jq, which treats this the same as a positive out-of-range index
    /// (`null`, not an error). Confirmed live against yq v4.53.3:
    /// `[1,2] | .[-3]` raises this, `[1,2] | .[-1]`/`.[-2]` (in-bounds
    /// wraparound) don't, and `[1,2] | .[5]` (positive OOB) is `null`, not
    /// an error -- the asymmetry is specific to the negative-magnitude
    /// case. `N` here is the original, unresolved index (`-3`, not the
    /// still-negative sum) -- real yq's own message reports the argument as
    /// written, not the arithmetic. Suppressible by `optional` like any
    /// other read-time indexing error -- real yq's own lexer doesn't even
    /// accept `?` after a bracket index in this position, so there's no
    /// oracle to match either way, and matching this codebase's own
    /// established default for ordinary indexing errors is simpler than
    /// carving out a special unsuppressible case with no oracle behind it.
    pub fn yq_negative_index_out_of_range(index: i64, len: usize) -> Self {
        Self::new(format!("index [{index}] out of range, array size is {len}"))
    }

    /// `strptime/1 requires string inputs and arguments` (#929).
    ///
    /// jq's C implementation validates `strptime`'s input *and* format
    /// argument with a single combined check, raising this exact message
    /// regardless of which one is the non-string offender — confirmed live
    /// against jq 1.7.1 for all three: a non-string format on a valid
    /// string input, a non-string input with a valid format string, and
    /// `fromdate`/`fromdateiso8601` (defined in terms of `strptime`) on a
    /// non-string input. (A non-string format *and* non-string input
    /// together crashes real jq with a C assertion failure rather than
    /// raising this message — not reproduced here; this message is
    /// strictly better than a crash for that combination.)
    pub fn strptime_requires_string() -> Self {
        Self::new("strptime/1 requires string inputs and arguments")
    }

    /// `<type> (<value>) cannot be <format>-formatted, only array` (#929).
    ///
    /// Raised by `@csv`/`@tsv` for a non-array top-level value — confirmed
    /// live against jq 1.7.1: `5 | @csv` is `"number (5) cannot be
    /// csv-formatted, only array"`, `5 | @tsv` the same with `tsv`.
    pub fn cannot_be_dsv_formatted(value: &OwnedValue, format: &str) -> Self {
        Self::new(format!(
            "{} cannot be {format}-formatted, only array",
            describe(value)
        ))
    }

    /// `<type> (<value>) can not be escaped for shell` (#929).
    ///
    /// Raised by `@sh` for a value that isn't a string, number, boolean,
    /// `null`, or array of those (jq's own shell-quoting rules) — confirmed
    /// live against jq 1.7.1: `{"a":1} | @sh` is `"object ({\"a\":1}) can
    /// not be escaped for shell"`.
    pub fn cannot_be_shell_escaped(value: &OwnedValue) -> Self {
        Self::new(format!("{} can not be escaped for shell", describe(value)))
    }

    /// `<type> (<value>) is not valid in a csv row` (#991).
    ///
    /// Raised by `@csv`/`@tsv`/`@dsv` for a row containing a nested array or
    /// object element, instead of silently stringifying it -- confirmed live
    /// against jq 1.7.1: `[[1,2]] | @csv` is `"array ([1,2]) is not valid in
    /// a csv row"`. `@tsv` reports the identical "csv row" wording rather
    /// than "tsv row" -- a real jq wording quirk (also confirmed live), not
    /// a succinctly bug reproduced here.
    pub fn not_valid_in_csv_row(value: &OwnedValue) -> Self {
        Self::new(format!("{} is not valid in a csv row", describe(value)))
    }

    /// `<type> (<value>) trailing base64 byte found` (#1120).
    ///
    /// Raised by `@base64d` when, after truncating at the first `=`
    /// (real jq discards it and everything after, not just within its own
    /// 4-character group), the remaining data's length has exactly a
    /// 1-character remainder past a multiple of 4 -- one base64 character
    /// (6 bits) can't carry even a single byte (8 bits). Confirmed live
    /// against jq 1.7.1: `"false" | @base64d` is `"string (\"false\")
    /// trailing base64 byte found"`.
    pub fn base64_trailing_byte(value: &OwnedValue) -> Self {
        Self::new(format!("{} trailing base64 byte found", describe(value)))
    }

    /// `<type> (<value>) is not valid base64 data` (#1146).
    ///
    /// Raised by `@base64d` (jq mode) when a byte in the input isn't a
    /// member of the base64 alphabet. Confirmed live against jq 1.7.1:
    /// `"ab!d" | @base64d` is `"string (\"ab!d\") is not valid base64
    /// data"`. Distinct from [`Self::base64_trailing_byte`] above, which
    /// covers a *different* jq wording for a too-short trailing group of
    /// otherwise-valid characters -- this one is for an outright invalid
    /// byte anywhere in the input.
    pub fn base64_invalid_data(value: &OwnedValue) -> Self {
        Self::new(format!("{} is not valid base64 data", describe(value)))
    }

    /// `illegal base64 data at input byte N` (#1146).
    ///
    /// Raised by `@base64d` (yq mode only) for *any* decode failure --
    /// invalid character or a too-short trailing group alike. Unlike jq's
    /// two-message split ([`Self::base64_invalid_data`]/
    /// [`Self::base64_trailing_byte`]), real yq (v4.53.3, live-verified)
    /// uses one uniform, byte-position-based message for every base64
    /// decode error, with no jq analogue -- hence [`Self::new`] directly,
    /// following this module's own convention for jq-less wording. `pos`
    /// is a 0-indexed byte offset into the string actually fed to the
    /// decoder (post leading/trailing-whitespace trim, since yq trims
    /// before decoding) -- confirmed live: an input with 2 leading spaces
    /// reports the identical position as the same input with none, so the
    /// position is relative to the *trimmed* string, not the original.
    pub fn base64_illegal_data(pos: usize) -> Self {
        Self::new(format!("illegal base64 data at input byte {pos}"))
    }

    /// `invalid URL escape "<escape>"` (#1138).
    ///
    /// Raised by `@urid` when a `%` isn't immediately followed by two
    /// valid hex digits. Unlike [`Self::base64_illegal_data`] above --
    /// genuinely yq-only, since jq's own real `@base64d` has independently
    /// verified wording to diverge against -- `@urid` is a succinctly
    /// extension with **no jq analogue at all**, so there's no competing
    /// jq-mode wording to preserve: this fires identically in both jq and
    /// yq mode (`format_urid`'s malformed-escape check has no `S::TAG`
    /// branch).
    ///
    /// `raw` is `%` plus whatever 0, 1, or 2 bytes actually follow it in
    /// the input (not validated -- confirmed live against yq v4.53.3 that
    /// both bytes are echoed verbatim even when only one, or neither, is
    /// a valid hex digit, and that a literal `%` immediately after the
    /// first one is included unchanged rather than treated as a new
    /// escape's start: `"x%y%zz" | @urid` -> `invalid URL escape "%y%"`,
    /// not `"%y"`). Takes raw bytes, not a `&str` (#1216): a `%` this
    /// close to the end of the input can truncate a multi-byte UTF-8
    /// character mid-sequence, which no `&str` can represent at all.
    /// `quote_bytes_go_style` (this module's own private helper) handles
    /// both a complete, valid character
    /// (Rust's own `Debug` escaping, confirmed live to match real yq's
    /// Go-style quoting for embedded `"`/`\`/control characters exactly:
    /// `%"y` -> `%\"y`, `%\y` -> `%\\y`, a literal tab -> `%\ty`) and a
    /// truncated one (raw `\xHH` per byte, matching real yq's own
    /// Go-`%q`-style escaping there too -- previously a documented,
    /// deliberate divergence, this is what #1216 closes).
    pub fn urid_invalid_escape(raw: &[u8]) -> Self {
        Self::new(format!("invalid URL escape {}", quote_bytes_go_style(raw)))
    }

    /// `Invalid path expression with result <value>` (#530).
    ///
    /// Raised by `path()` when the filter it was given is not a path
    /// expression at all — `path(1)`, `path(length)`, `path({a:1})` — rather
    /// than one jq recognises but leaves unresolved. Unlike the `describe`-
    /// shaped messages above, jq embeds the bare dump here, not
    /// `<type> (<dump>)`: `path(1)` reports `result 1`, not
    /// `result number (1)`. #2179: this message's dump does *not* share
    /// `DUMP_KEEP`/`DUMP_BUDGET` with the `describe`-shaped ones -- jq's own
    /// `PATH_END` case uses a wider `char errbuf[30]`, not the `errbuf[15]`
    /// those use, giving `DUMP_KEEP_WIDE` (26 bytes) here instead (see that
    /// constant's own doc comment).
    pub fn invalid_path_expression(value: &OwnedValue) -> Self {
        Self::with_kind(
            format!(
                "{}{}",
                Self::INVALID_PATH_EXPRESSION_PREFIX,
                dump_truncated_wide(value)
            ),
            ErrorKind::InvalidPathExpression,
        )
    }

    const INVALID_PATH_EXPRESSION_PREFIX: &'static str = "Invalid path expression with result ";

    /// `Invalid path expression near attempt to access element <k> of <v>`
    /// (#843).
    ///
    /// Raised by `path()` when a genuine navigation step (a field, a
    /// literal/computed index, or a slice) is attempted against a value that
    /// was not itself reached by real navigation from the expression's
    /// original input — today, only the payload `catch` binds to `.` inside
    /// its handler (`resolve_catch` in `eval.rs`). Unlike
    /// [`Self::invalid_path_expression`] above (`?`/`try` never suppress
    /// it), this is mostly an *ordinary*, catchable error — confirmed live
    /// against jq 1.7.1: `path(try (.a, error({b:1})) catch (.b)?)` prints
    /// only `["a"]`, no error, and a *nested*
    /// `try (.a, error({b:1})) catch (try .b catch "caught")` actually runs
    /// `"caught"`. The one exception is a *bare* postfix `?` directly on the
    /// plain field/index/iterate/slice access that raised it (`.b?`, not
    /// `(.b)?`) — jq does not suppress it there either (confirmed live:
    /// `path(try (.a, error(5)) catch .b?)` still raises), which is what
    /// [`Self::is_untracked_navigation_error`] exists for. So this
    /// constructor deliberately does *not* participate in
    /// [`Self::is_invalid_path_expression`] — only in that narrower,
    /// position-sensitive check.
    ///
    /// #2179: `element`/`container` are truncated at *different* widths, not
    /// the same one -- jq's own `INDEX`/`INDEX_OPT` case truncates the key
    /// (`element`) with `char keybuf[15]` (the narrow, shared `DUMP_KEEP`,
    /// unchanged) but the container with its own `char objbuf[30]`
    /// (`DUMP_KEEP_WIDE`, the same wide constant [`Self::invalid_path_expression`]
    /// uses), confirmed against jq 1.7.1's C source and live-verified.
    pub fn invalid_path_expression_near_access(
        element: &OwnedValue,
        container: &OwnedValue,
    ) -> Self {
        Self::with_kind(
            format!(
                "{}{} of {}",
                Self::UNTRACKED_NAVIGATION_ACCESS_PREFIX,
                dump_truncated(element),
                dump_truncated_wide(container)
            ),
            ErrorKind::UntrackedNavigation,
        )
    }

    const UNTRACKED_NAVIGATION_ACCESS_PREFIX: &'static str =
        "Invalid path expression near attempt to access element ";

    /// `Invalid path expression near attempt to iterate through <v>` (#843).
    ///
    /// The `.[]`/`Expr::Iterate` sibling of
    /// [`Self::invalid_path_expression_near_access`] — same trigger (a
    /// genuine navigation attempt against an untracked value), same
    /// mostly-catchable status (see that constructor's doc comment for the
    /// bare-`?` exception both share), just jq's distinct wording for
    /// iteration rather than a keyed access (confirmed live:
    /// `path(try (.a, error(5)) catch .[])` on a caught scalar `5` reports
    /// "near attempt to iterate through 5", never "Cannot iterate over
    /// number").
    ///
    /// #2179: `container` uses the same wide truncation as
    /// [`Self::invalid_path_expression_near_access`]'s own container -- jq's
    /// `EACH`/`EACH_OPT` case truncates with `char errbuf[30]`, confirmed
    /// against jq 1.7.1's C source and live-verified.
    pub fn invalid_path_expression_near_iterate(container: &OwnedValue) -> Self {
        Self::with_kind(
            format!(
                "{}{}",
                Self::UNTRACKED_NAVIGATION_ITERATE_PREFIX,
                dump_truncated_wide(container)
            ),
            ErrorKind::UntrackedNavigation,
        )
    }

    const UNTRACKED_NAVIGATION_ITERATE_PREFIX: &'static str =
        "Invalid path expression near attempt to iterate through ";

    /// Whether this is one of [`Self::invalid_path_expression_near_access`]/
    /// [`Self::invalid_path_expression_near_iterate`] (#843) — an untracked
    /// navigation attempt inside a `catch` handler. Unlike
    /// [`Self::is_invalid_path_expression`], this error *is* ordinarily
    /// suppressed by `?`/bare `try` — except in one narrow position, a bare
    /// postfix `?` directly wrapping the plain `Field`/`Index`/`Iterate`/
    /// `Slice` access that raised it, which is exactly why this needs its
    /// own predicate rather than reusing `is_invalid_path_expression`'s
    /// blanket "never suppressed" rule: `resolve_node`'s `Expr::Optional`
    /// arm consults this, but only for that one bare-primitive shape, to
    /// decide *not* to prune it there — see that arm's doc comment for the
    /// jq-1.7.1-confirmed `.b?` vs `(.b)?` distinction this exists for.
    pub fn is_untracked_navigation_error(&self) -> bool {
        matches!(
            self.value,
            EvalErrorPayload::Kind(ErrorKind::UntrackedNavigation)
        )
    }

    /// Whether this is an [`Self::invalid_path_expression`] — a statement
    /// that the *filter* is not a path expression, not a runtime value
    /// error. `?` only suppresses failures raised while collecting a path
    /// (a missing key, an out-of-range index, ...); this survives it, the
    /// same way [`Self::is_write_time_application_error`] does for the write
    /// side (#530: confirmed live — `path(("a")?)` still raises in jq). The
    /// lone call site that needs to tell the two apart is `resolve_node`'s
    /// bare-`?` arm in `eval.rs`.
    pub fn is_invalid_path_expression(&self) -> bool {
        matches!(
            self.value,
            EvalErrorPayload::Kind(ErrorKind::InvalidPathExpression)
        )
    }

    /// A decode failure while materializing a lazily-indexed document value
    /// (#1247): invalid UTF-8, an unrecoverable escape, or a malformed
    /// number literal. Unlike an ordinary type-mismatch error, this must
    /// never be suppressed by `?` or caught by `try`/`catch` (#1620) — jq's
    /// own equivalent is a parse-time rejection no program could ever catch
    /// either.
    pub fn decode_failure(reason: impl Into<String>) -> Self {
        Self::with_kind(reason, ErrorKind::DecodeFailure)
    }

    /// `object key "<key>" is ambiguous: ...` (#1642) — a #1620-class
    /// decode failure in its own right: two distinct undecodable keys
    /// whose display fallback collides, so keeping the second silently
    /// overwrites the first (#1385 forbids treating them as the same key,
    /// but a display-keyed map has no way to hold both). This is what
    /// `to_owned`/`materialize` raise instead, via
    /// [`super::document::resolve_display_key`]/[`super::document::DisplayKeyGuard`].
    /// [`Self::is_decode_failure`] checks the [`ErrorKind::DecodeFailure`]
    /// tag this constructor sets directly (#1840), not `message` text — this
    /// used to share a fixed-suffix convention with
    /// [`Self::invalid_path_expression`]'s own prefix constant so the two
    /// couldn't independently drift out of sync, but drifting was exactly
    /// what happened once anyway (confirmed live before that fix, #1813:
    /// this previously built its message ad hoc via `format!` at its one
    /// call site in `document.rs`, and `is_decode_failure`'s fixed literal
    /// list didn't include it — `try sort catch .` and `sort?` both wrongly
    /// treated this as an ordinary catchable error). A tag can't drift out
    /// of sync with itself the way a duplicated string constant can.
    ///
    /// Called directly (not via a `document.rs`-local wrapper, #1813
    /// review) from three sites across two crates: `document.rs`'s own
    /// `resolve_display_key`, `eval.rs`'s `yaml_value_to_owned_checked`
    /// (`load()`'s YAML path), and `succinctly-cli`'s `yq_runner.rs`
    /// (`yaml_to_owned_value`, #1749) — `EvalError` is already `pub` from
    /// `jq::mod`, so a re-exporting wrapper added an extra hop without
    /// adding any encapsulation.
    pub fn colliding_display_key(key: &str) -> Self {
        // Delegates to `Self::decode_failure` (not a second `with_kind`
        // call) so the two constructors can't independently drift out of
        // sync on which `ErrorKind` a decode failure gets tagged with --
        // exactly the class of drift #1840 introduced tags to prevent.
        Self::decode_failure(format!(
            "object key \"{key}\" is ambiguous{}",
            Self::COLLIDING_DISPLAY_KEY_SUFFIX
        ))
    }

    const COLLIDING_DISPLAY_KEY_SUFFIX: &'static str = ": an undecodable key's display form \
         collides with another key of the same name and cannot be represented";

    /// Whether this is a [`Self::decode_failure`] — see #1620/#1660. Every
    /// `?`/`try`/`catch` boundary consults this so a decode failure passes
    /// through unmatched instead of being suppressed, the same way
    /// [`Self::is_invalid_path_expression`] exempts its own narrow error
    /// class without leaving the ordinary catchable `Error` channel.
    ///
    /// #1840: this used to classify purely by matching `message` against a
    /// fixed set of string literals/prefixes/suffixes, which produced two
    /// real bugs from the same root cause -- #1660 (a user's own
    /// `error("invalid escape sequence")` collided with the literal list and
    /// was wrongly forced uncatchable; live-verified against jq 1.7.1, real
    /// jq retries/catches/suppresses it as an ordinary error) and #1813 (a
    /// dynamic message, built ad hoc via `format!`, didn't match any literal
    /// in the list, so it was wrongly left catchable). Checking
    /// [`ErrorKind::DecodeFailure`] directly instead makes both classes of
    /// mistake structurally impossible: a constructor either tags its error
    /// with this `Kind` or it doesn't, there is no message text for a future
    /// literal list to miss or a legitimate user error to collide with.
    ///
    /// This costs nothing extra over the plain `Option<OwnedValue>`
    /// [`EvalErrorPayload`] replaces -- see that type's own doc comment. A
    /// dedicated boolean *field* on `EvalError` was tried first instead, but
    /// `EvalError` had no spare padding to absorb it: the extra 8 bytes
    /// propagated into `QueryResult`/`Control`'s recursive-materializer call
    /// frames and turned a previously-controlled "nesting depth exceeds
    /// limit" panic into a genuine stack overflow during unwind
    /// (`jq::lazy::tests::into_owned_panics_past_nesting_depth_limit_1021`)
    /// -- reverted in favor of `EvalErrorPayload`'s niche-sharing enum
    /// instead, which avoids that cost entirely.
    pub fn is_decode_failure(&self) -> bool {
        matches!(self.value, EvalErrorPayload::Kind(ErrorKind::DecodeFailure))
    }

    /// Whether this error class is *always* uncatchable, with no positional
    /// nuance — [`Self::is_invalid_path_expression`] or
    /// [`Self::is_decode_failure`]. Unlike
    /// [`Self::is_untracked_navigation_error`], which genuinely depends on
    /// *where* the error was caught (a bare postfix `?` on the primitive
    /// that raised it vs. anything else), both of these mean the same thing
    /// at every call site that checks them: never suppressed by `?`, never
    /// handed to a `catch` handler. `resolve_node`'s `Expr::Try` and
    /// `Expr::Optional` arms (`?`'s two path-context dispatch sites, kept in
    /// agreement since #1746 — `expr?` is documented sugar for `try expr`)
    /// both check this today, but a future always-uncatchable error class
    /// only needs to be added here, not hand-copied into whichever `?`/`try`
    /// dispatch sites grow the same check next.
    pub fn is_uncatchable(&self) -> bool {
        self.is_invalid_path_expression() || self.is_decode_failure()
    }

    /// `Cannot check whether <container> has a <key type> key`.
    pub fn cannot_check_has(container_type: &str, key_type: &str) -> Self {
        Self::new(format!(
            "Cannot check whether {container_type} has a {key_type} key"
        ))
    }

    /// `<a> and <b> cannot be <added|subtracted|multiplied|divided>`.
    pub fn binary_op(left: &OwnedValue, right: &OwnedValue, op: BinOp) -> Self {
        Self::pair(left, right, &format!("cannot be {}", op.participle()))
    }

    /// `<a> and <b> cannot be divided because the divisor is zero` (and the
    /// `(remainder)` variant for `%`).
    pub fn divisor_is_zero(left: &OwnedValue, right: &OwnedValue, op: BinOp) -> Self {
        Self::pair(
            left,
            right,
            &format!("cannot be {} because the divisor is zero", op.participle()),
        )
    }

    /// `cannot modulo by 0` — real yq's own wording for an integer modulo
    /// by zero (#1231), confirmed live against yq v4.53.3. Unlike jq's
    /// [`Self::divisor_is_zero`], this is a fixed sentence with no embedded
    /// operand values, and applies only to the integer case: a
    /// float-involving modulo by zero returns NaN in yq, not an error.
    pub fn yq_modulo_by_zero() -> Self {
        Self::new("cannot modulo by 0")
    }

    /// `<a> and <b> cannot have their containment checked`.
    pub fn containment_check(left: &OwnedValue, right: &OwnedValue) -> Self {
        Self::pair(left, right, "cannot have their containment checked")
    }

    /// `<a> and <b> cannot be iterated over` — jq's wording for `min`/`max`
    /// on a non-array, where both operands are the same value.
    pub fn pair_cannot_be_iterated(left: &OwnedValue, right: &OwnedValue) -> Self {
        Self::pair(left, right, "cannot be iterated over")
    }

    /// `<a> and <b> cannot be sorted, as they are not both arrays` — jq's
    /// wording for `unique_by`/`sort_by`/`group_by` on a non-array (#929).
    pub fn pair_cannot_be_sorted(left: &OwnedValue, right: &OwnedValue) -> Self {
        Self::pair(left, right, "cannot be sorted, as they are not both arrays")
    }

    /// `<v> has no keys`.
    pub fn has_no_keys(value: &OwnedValue) -> Self {
        Self::subject(value, "has no keys")
    }

    /// `<v> cannot be negated` — unary minus (`-expr`) on a non-numeric
    /// operand (#1056). Matches real jq's own dedicated wording, confirmed
    /// live against jq 1.7.1 (`-"abc"` -> `string ("abc") cannot be
    /// negated`).
    pub fn cannot_be_negated(value: &OwnedValue) -> Self {
        Self::subject(value, "cannot be negated")
    }

    /// `<v> has no length`.
    pub fn has_no_length(value: &OwnedValue) -> Self {
        Self::subject(value, "has no length")
    }

    /// `<v> only strings have UTF-8 byte length`.
    pub fn no_utf8_byte_length(value: &OwnedValue) -> Self {
        Self::subject(value, "only strings have UTF-8 byte length")
    }

    /// `<v> cannot be sorted, as it is not an array`.
    pub fn cannot_be_sorted(value: &OwnedValue) -> Self {
        Self::subject(value, "cannot be sorted, as it is not an array")
    }

    /// `<v> cannot be matched, as it is not a string`.
    pub fn cannot_be_matched(value: &OwnedValue) -> Self {
        Self::subject(value, "cannot be matched, as it is not a string")
    }

    /// `<v> can't be imploded, unicode codepoint needs to be numeric`.
    pub fn cannot_be_imploded(value: &OwnedValue) -> Self {
        Self::subject(
            value,
            "can't be imploded, unicode codepoint needs to be numeric",
        )
    }

    /// `<type> not a string or array`.
    ///
    /// jq's wording when test/match/capture's *pattern* argument is neither a
    /// string nor an array: no value preview, no parens — just the bare type
    /// name. Confirmed live against jq-1.7.1: `test(12345)` → `number not a
    /// string or array`, `test({"a":1})` → `object not a string or array`.
    pub fn not_string_or_array(type_name: &str) -> Self {
        Self::new(format!("{type_name} not a string or array"))
    }

    /// `<v> is not a string`.
    ///
    /// jq's wording when scan/gsub/sub/splits's *pattern* argument isn't a
    /// string — a genuinely different sentence from `not_string_or_array`
    /// above (test/match/capture's own pattern-type error), not just a
    /// stylistic variant: confirmed live against jq-1.7.1, `scan(1)` and
    /// `sub(1; "y")` both raise `number (1) is not a string` (#926), where
    /// `test(1)` raises `number not a string or array`. jq itself uses two
    /// different sentences for the same conceptual failure depending on
    /// which builtin family raises it, so both constructors stay separate
    /// rather than merging into one.
    pub fn is_not_a_string(value: &OwnedValue) -> Self {
        Self::subject(value, "is not a string")
    }

    /// `<v> cannot be parsed as a number`.
    pub fn cannot_parse_as_number(value: &OwnedValue) -> Self {
        Self::subject(value, "cannot be parsed as a number")
    }

    /// `<v> only strings can be parsed` — `fromjson` on a non-string.
    pub fn only_strings_can_be_parsed(value: &OwnedValue) -> Self {
        Self::subject(value, "only strings can be parsed")
    }

    /// `Invalid numeric literal at EOF at line 1, column <n> (while parsing
    /// '<s>')`, or `Expected JSON value (while parsing '')` for empty input.
    ///
    /// jq reaches these by handing the string to its JSON parser, so the
    /// column is the parser's position — for a single malformed token that is
    /// the end of input, i.e. the string's length in bytes. Inputs that fail
    /// *after* a complete token (`"1 2"`, `"1,2"`) get a different diagnostic
    /// from jq that succinctly does not reproduce; see
    /// `docs/compliance/jq/limitations.md`.
    pub fn invalid_numeric_literal(text: &str) -> Self {
        if text.is_empty() {
            return Self::new("Expected JSON value (while parsing '')");
        }
        Self::new(format!(
            "Invalid numeric literal at EOF at line 1, column {} (while parsing '{text}')",
            text.len()
        ))
    }

    /// `Invalid JSON text: <cause>` — a document the semi-index accepted that
    /// is not, in fact, valid JSON (#1194).
    ///
    /// The semi-index recovers an object's members by pairing the container's
    /// BP children two at a time, checking neither that a key is a string nor
    /// that the count is even — `json::standard::is_delim` maps `:` and `,` to
    /// the same nothing, so `{invalid}` and `{invalid: 1}` index exactly as
    /// `{"a":1}` does. Once a materializer has actually *found* such a member,
    /// this re-runs the strict validator over the same document to name the
    /// real syntax error, which is far more specific than anything
    /// reconstructible from the cursor alone.
    ///
    /// Reached only after a swallow point has already fired, so a well-formed
    /// document never pays for the pass. Same shape as the `tonumber` path in
    /// `eval.rs`, which re-parses on the error path purely to pick a better
    /// message.
    ///
    /// Positions are deliberately left out of the message: they would be
    /// relative to this document's own slice, while the caller reports a
    /// location counted in the whole file. Carrying both would print two
    /// numbers that disagree.
    pub fn malformed_json_text(text: &[u8]) -> Self {
        match crate::json::validate::validate(text) {
            Err(err) => Self::new(format!("Invalid JSON text: {}", err.kind)),
            // The validator disagreeing with the indexer means the two have
            // drifted apart. Report the generic form rather than claim the
            // document is fine when a swallow point has already fired.
            Ok(()) => Self::new("Invalid JSON text"),
        }
    }

    /// `<builtin>() requires numeric inputs` — `gmtime`/`localtime` on a
    /// non-number input.
    pub fn datetime_requires_number(builtin: &str) -> Self {
        Self::new(format!("{builtin}() requires numeric inputs"))
    }

    /// `error converting number of seconds since epoch to datetime` — a
    /// `gmtime`/`localtime`/`strftime`/`todate`/`tz` timestamp too extreme
    /// to convert (matches real jq's own message and, approximately, its
    /// error boundary: confirmed empirically that jq itself starts
    /// erroring once the resulting year would overflow a 32-bit int,
    /// e.g. `(7e16) | strftime(...)` errors while `(6e16) | strftime(...)`
    /// succeeds).
    pub fn datetime_out_of_range() -> Self {
        Self::new("error converting number of seconds since epoch to datetime")
    }

    /// `mktime/strftime: broken-down time value out of representable
    /// range` — a `mktime`/`strftime` broken-down-time array element
    /// (year, month, day, hour, minute, second, weekday, or yearday) too
    /// extreme for this codebase's checked civil-date arithmetic to
    /// convert without overflow (#893, #911). Deliberately a distinct
    /// message from [`Self::datetime_out_of_range`] rather than reusing
    /// it: that message is anchored to real jq's own text for the
    /// *opposite* (seconds -> broken-down-time) direction, and real jq has
    /// no equivalent error for *this* direction at all — confirmed
    /// empirically it silently computes a wrapped/nonsensical result
    /// instead (`[9223372036854775807,0,1,0,0,0,0,0] | mktime` succeeds in
    /// real jq with a bogus timestamp) — so there is no oracle text to
    /// match here, and reusing the other message would misdescribe which
    /// conversion actually failed.
    pub fn broken_down_time_out_of_range() -> Self {
        Self::new("mktime/strftime: broken-down time value out of representable range")
    }

    /// `mktime requires array inputs`.
    pub fn mktime_requires_array() -> Self {
        Self::new("mktime requires array inputs")
    }

    /// `strftime/1 requires parsed datetime inputs` — `strftime` on an input
    /// that's neither a broken-down-time array nor a raw number.
    pub fn strftime_requires_parsed_datetime_inputs() -> Self {
        Self::new("strftime/1 requires parsed datetime inputs")
    }

    /// `date "<input>" does not match format "<fmt>"` — any `strptime`
    /// parse failure. jq reports this one wording regardless of which
    /// format specifier the input failed to satisfy.
    pub fn strptime_no_match(input: &str, fmt: &str) -> Self {
        Self::new(format!(r#"date "{input}" does not match format "{fmt}""#))
    }

    /// `<a> and <b> <phrase>`.
    fn pair(left: &OwnedValue, right: &OwnedValue, phrase: &str) -> Self {
        Self::new(format!(
            "{} and {} {phrase}",
            describe(left),
            describe(right)
        ))
    }

    /// `<v> <phrase>`.
    fn subject(value: &OwnedValue, phrase: &str) -> Self {
        Self::new(format!("{} {phrase}", describe(value)))
    }
}

impl core::fmt::Display for EvalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> OwnedValue {
        OwnedValue::String(v.to_string())
    }

    #[test]
    fn dump_under_the_budget_is_verbatim() {
        // 14 bytes including the quotes — jq's errbuf holds exactly this.
        assert_eq!(dump_truncated(&s("abcdefghijkl")), "\"abcdefghijkl\"");
    }

    #[test]
    fn dump_over_the_budget_keeps_eleven_bytes() {
        assert_eq!(dump_truncated(&s("abcdefghijklm")), "\"abcdefghij...");
        assert_eq!(dump_truncated(&s("abcdefghijklmn")), "\"abcdefghij...");
    }

    #[test]
    fn dump_truncation_counts_bytes_not_characters() {
        // Five 2-byte characters plus the opening quote is exactly 11 bytes,
        // so this cut lands on a character boundary just as jq's does.
        assert_eq!(dump_truncated(&s("ααααααααα")), "\"ααααα...");
    }

    #[test]
    fn dump_truncation_snaps_back_off_a_character_boundary() {
        // jq would cut mid-character here and emit invalid UTF-8; we keep one
        // byte fewer so the message stays a valid Rust string.
        assert_eq!(dump_truncated(&s("あああああ")), "\"あああ...");
    }

    /// The preview must cost a bounded amount of work, not a copy of the value.
    ///
    /// [`dump_truncated`] is the reason: jq builds the whole dump and throws all
    /// but 14 bytes away, which for succinctly would mean serialising a 100 MB
    /// operand to produce a 14-byte message. The sink is what prevents that, so
    /// pin the sink directly — a `dump_truncated` result is short either way and
    /// would not catch a regression here (#358).
    #[test]
    fn preview_sink_copies_at_most_its_cap() {
        use core::fmt::Write;

        // A single oversized `write_str` is the case that matters: the streaming
        // writer hands a long JSON string over as one span.
        let mut sink = PreviewSink::new(14);
        let huge = "x".repeat(1_000_000);
        assert!(sink.write_str(&huge).is_err(), "should stop the writer");
        assert!(sink.overflowed);
        assert_eq!(sink.buf.len(), 14, "must not copy beyond the cap");

        // An exactly-fitting dump completes without reporting overflow, which is
        // what lets a 14-byte dump through verbatim.
        let mut sink = PreviewSink::new(14);
        assert!(sink.write_str("12345678901234").is_ok());
        assert!(!sink.overflowed);
        assert_eq!(sink.buf, "12345678901234");
        // One more byte tips it over.
        assert!(sink.write_str("5").is_err());
        assert!(sink.overflowed);

        // Multi-byte characters are cut back to a boundary, never split.
        let mut sink = PreviewSink::new(14);
        let _ = sink.write_str(&"あ".repeat(20)); // 3 bytes each: 12 < 14 < 15
        assert_eq!(sink.buf, "ああああ", "cut back to a char boundary");
        assert!(sink.buf.len() <= 14);
    }

    /// #930: an infinite `NumberLiteral` reaching `dump_truncated` now
    /// renders its own source text (rather than the unconditional `"null"`
    /// this whole area used to take) via `format_number_jq_compat` - which,
    /// same spirit as `preview_sink_copies_at_most_its_cap` above, must stay
    /// bounded even if the document's mantissa is enormous, since almost
    /// none of it will ever be visible past `dump_truncated`'s own budget.
    #[test]
    fn dump_truncated_bounds_an_overflowed_literal_with_a_huge_mantissa() {
        let mantissa = "9".repeat(2_000_000);
        let literal: Box<str> = format!("{mantissa}.5e400").into();
        let value = OwnedValue::NumberLiteral(
            super::super::value::NumberRepr::Float(f64::INFINITY),
            literal,
        );
        let result = dump_truncated(&value);
        assert!(
            result.len() < 100,
            "must not render anywhere near the full 2,000,000-digit mantissa: got {} bytes",
            result.len()
        );
        assert!(result.starts_with("9.999999999"), "got: {result}");
        assert!(result.ends_with("..."), "must be truncated: {result}");
    }

    /// #1304: unlike the mantissa above, `assemble_scientific_from_raw_
    /// exponent` (reached via `format_near_zero_literal` for a saturated
    /// exponent) deliberately does *not* cap the raw exponent digit string
    /// it echoes -- see that function's own doc comment for the measured
    /// reasoning. Pins that `dump_truncated`'s own output still stays
    /// bounded regardless: the echo itself is uncapped, but
    /// `dump_truncated`'s `PreviewSink` still clamps what actually gets
    /// kept, the same as it already does for every other value shape.
    #[test]
    fn dump_truncated_bounds_a_near_zero_literal_with_a_huge_saturated_exponent() {
        let exponent = "9".repeat(100_000);
        let literal: Box<str> = format!("0.005e-{exponent}").into();
        let value = OwnedValue::NumberLiteral(super::super::value::NumberRepr::Float(0.0), literal);
        let result = dump_truncated(&value);
        assert!(
            result.len() < 100,
            "must not render anywhere near the full 100,000-digit exponent: got {} bytes",
            result.len()
        );
        assert!(result.starts_with("5E-99999999"), "got: {result}");
        assert!(result.ends_with("..."), "must be truncated: {result}");
    }

    /// #930 review: `from_value`'s non-string branch used `OwnedValue::to_json`,
    /// which is the *real-output* convention (RFC-8259-safe `null` for a
    /// non-finite float) - wrong for `error(v)`'s message, which (like
    /// `describe`/`dump_truncated` above) isn't JSON and should show jq's
    /// real preview text instead. Oracle-verified: `error(infinite)`
    /// uncaught reports `1.7976931348623157e+308` in real jq, not `null`.
    #[test]
    fn from_value_renders_infinite_float_like_jq_not_as_null() {
        assert_eq!(
            EvalError::from_value(OwnedValue::Float(f64::INFINITY)).message,
            "1.7976931348623157e+308"
        );
        assert_eq!(
            EvalError::from_value(OwnedValue::Float(f64::NEG_INFINITY)).message,
            "-1.7976931348623157e+308"
        );
        // NaN still has no literal to fall back to either way - unchanged.
        assert_eq!(
            EvalError::from_value(OwnedValue::Float(f64::NAN)).message,
            "null"
        );
        // A container holding a non-finite field renders it the same way,
        // not just a bare top-level value.
        let mut obj = indexmap::IndexMap::new();
        obj.insert("a".to_string(), OwnedValue::Float(f64::INFINITY));
        obj.insert("b".to_string(), OwnedValue::String("x".to_string()));
        assert_eq!(
            EvalError::from_value(OwnedValue::Object(obj)).message,
            r#"{"a":1.7976931348623157e+308,"b":"x"}"#
        );
        // `from_value`'s message is the whole value, not a truncated
        // preview - unlike `dump_truncated`'s budget above.
        let long = "x".repeat(100);
        assert_eq!(
            EvalError::from_value(s(&long)).message,
            long,
            "a string payload is used as-is, unmodified and untruncated"
        );
    }

    #[test]
    fn index_message_quotes_string_keys_and_names_other_kinds() {
        assert_eq!(
            EvalError::cannot_index("number", &s("foo")).message,
            "Cannot index number with string \"foo\""
        );
        assert_eq!(
            EvalError::cannot_index("number", &OwnedValue::Int(0)).message,
            "Cannot index number with number"
        );
        assert_eq!(
            EvalError::cannot_index("object", &OwnedValue::Null).message,
            "Cannot index object with null"
        );
    }

    /// `path(1)` reports `result 1`, not `result number (1)` — the bare
    /// dump, unlike `describe`'s `<type> (<dump>)` shape used elsewhere.
    #[test]
    fn invalid_path_expression_embeds_the_bare_dump() {
        assert_eq!(
            EvalError::invalid_path_expression(&OwnedValue::Int(1)).message,
            "Invalid path expression with result 1"
        );
        assert_eq!(
            EvalError::invalid_path_expression(&s("ab")).message,
            "Invalid path expression with result \"ab\""
        );
    }

    /// #2179: long results are truncated wider than every other embedded
    /// value (`DUMP_KEEP_WIDE` = 26 bytes, not the shared `DUMP_KEEP` = 11) —
    /// jq's own `PATH_END` case uses `char errbuf[30]`, not `errbuf[15]`,
    /// confirmed against jq 1.7.1's C source and live-verified: 26 bytes of
    /// the dump (opening quote plus 25 characters here) then `...`.
    #[test]
    fn invalid_path_expression_truncates_a_long_result() {
        let long = "a".repeat(40);
        let kept: String = "a".repeat(25);
        assert_eq!(
            EvalError::invalid_path_expression(&s(&long)).message,
            format!("Invalid path expression with result \"{kept}...")
        );
    }

    /// #2179: `invalid_path_expression_near_access`'s `container` argument
    /// uses the same wide (26-byte) truncation as `invalid_path_expression`
    /// above -- jq's `INDEX`/`INDEX_OPT` case truncates it with
    /// `char objbuf[30]`, not `errbuf[15]`.
    #[test]
    fn invalid_path_expression_near_access_truncates_a_long_container() {
        let long = "a".repeat(40);
        let kept: String = "a".repeat(25);
        assert_eq!(
            EvalError::invalid_path_expression_near_access(&OwnedValue::Int(1), &s(&long)).message,
            format!("Invalid path expression near attempt to access element 1 of \"{kept}...")
        );
    }

    /// #2179: unlike its own sibling `container` argument, `element` keeps
    /// the narrow (11-byte) truncation -- jq's `INDEX`/`INDEX_OPT` case
    /// truncates it with `char keybuf[15]`, the same width every
    /// `describe`-shaped message uses.
    #[test]
    fn invalid_path_expression_near_access_element_stays_narrow() {
        let long = "a".repeat(20);
        let kept: String = "a".repeat(10);
        assert_eq!(
            EvalError::invalid_path_expression_near_access(&s(&long), &OwnedValue::Int(1)).message,
            format!("Invalid path expression near attempt to access element \"{kept}... of 1")
        );
    }

    /// #2179: `invalid_path_expression_near_iterate`'s `container` argument
    /// uses the same wide (26-byte) truncation as the two constructors
    /// above -- jq's `EACH`/`EACH_OPT` case truncates it with
    /// `char errbuf[30]`, not `errbuf[15]`.
    #[test]
    fn invalid_path_expression_near_iterate_truncates_a_long_container() {
        let long = "a".repeat(40);
        let kept: String = "a".repeat(25);
        assert_eq!(
            EvalError::invalid_path_expression_near_iterate(&s(&long)).message,
            format!("Invalid path expression near attempt to iterate through \"{kept}...")
        );
    }

    #[test]
    fn index_keys_are_not_truncated() {
        let key = "aaaaaaaaaaaaaaaaaaaa";
        assert_eq!(
            EvalError::cannot_index("number", &s(key)).message,
            format!("Cannot index number with string \"{key}\"")
        );
    }

    /// The object-key refusal names the kind and previews the value, and the
    /// preview is truncated like every other — a long key is what jq reports
    /// as `number (12345678901...)`, not in full.
    #[test]
    fn object_key_message_names_the_kind_and_truncates_the_preview() {
        assert_eq!(
            EvalError::cannot_use_as_object_key(&OwnedValue::Int(0)).message,
            "Cannot use number (0) as object key"
        );
        assert_eq!(
            EvalError::cannot_use_as_object_key(&OwnedValue::Null).message,
            "Cannot use null (null) as object key"
        );
        assert_eq!(
            EvalError::cannot_use_as_object_key(&OwnedValue::Int(12345678901234567)).message,
            "Cannot use number (12345678901...) as object key"
        );
    }

    #[test]
    fn modulo_reports_itself_as_remainder_division() {
        assert_eq!(
            EvalError::binary_op(&OwnedValue::Int(1), &s("a"), BinOp::Modulo).message,
            "number (1) and string (\"a\") cannot be divided (remainder)"
        );
        assert_eq!(
            EvalError::divisor_is_zero(&OwnedValue::Int(1), &OwnedValue::Int(0), BinOp::Divide)
                .message,
            "number (1) and number (0) cannot be divided because the divisor is zero"
        );
    }

    #[test]
    fn numeric_literal_column_is_the_byte_length() {
        assert_eq!(
            EvalError::invalid_numeric_literal("0x10").message,
            "Invalid numeric literal at EOF at line 1, column 4 (while parsing '0x10')"
        );
        assert_eq!(
            EvalError::invalid_numeric_literal("").message,
            "Expected JSON value (while parsing '')"
        );
    }

    #[test]
    fn internal_errors_carry_no_payload_but_catch_sees_the_message() {
        let err = EvalError::cannot_iterate_with(EvalTag::Jq, &OwnedValue::Int(1));
        assert_eq!(err.message, "Cannot iterate over number (1)");
        assert_eq!(err.value, EvalErrorPayload::None);
        assert_eq!(
            err.payload(),
            OwnedValue::String("Cannot iterate over number (1)".to_string())
        );
    }

    /// #1840: `EvalErrorPayload`'s whole reason for existing is riding along
    /// in the same space `Option<OwnedValue>` already used -- verify that
    /// empirically rather than trusting the doc comment's claim, and pin
    /// `EvalError`'s own size so a future change to either type can't
    /// silently regress it back into the #1021 stack-overflow class of bug
    /// (see `jq::lazy::tests::into_owned_panics_past_nesting_depth_limit_1021`,
    /// which exercises the actual regression this guards).
    #[test]
    fn eval_error_payload_is_no_larger_than_the_option_it_replaces() {
        assert_eq!(
            core::mem::size_of::<EvalErrorPayload>(),
            core::mem::size_of::<Option<OwnedValue>>(),
            "EvalErrorPayload must not be larger than the Option<OwnedValue> it replaces"
        );
    }

    // 32-bit targets shrink String/OwnedValue's pointer-sized fields, so this
    // exact byte count doesn't hold there -- matches this crate's existing
    // convention for exact-size assertions (e.g. `trees::bp`'s own
    // `#[cfg(target_pointer_width = "64")]`-gated tests).
    #[test]
    #[cfg(target_pointer_width = "64")]
    fn eval_error_size_is_pinned_for_the_1021_stack_overflow_fix() {
        assert_eq!(
            core::mem::size_of::<EvalError>(),
            96,
            "EvalError's size regressed -- see #1021's doc comment on EvalErrorPayload \
             for why an 8-byte increase here turns a controlled panic into a stack overflow"
        );
    }

    /// #1216: a truncated multi-byte character hex-escapes each raw byte,
    /// matching real yq's own Go-`%q`-style escaping (live-verified,
    /// tests/yq_cli_tests.rs's own `_1216`-suffixed CLI tests carry the
    /// oracle comparison) -- this is the unit-level equivalent, pinning
    /// `quote_bytes_go_style` directly rather than only through `@urid`.
    #[test]
    fn quote_bytes_go_style_hex_escapes_a_truncated_multibyte_character() {
        // 中 = E4 B8 AD, truncated to its first two bytes.
        assert_eq!(quote_bytes_go_style(b"%\xe4\xb8"), r#""%\xe4\xb8""#);
        // 4-byte character (outside the BMP), truncated the same way.
        assert_eq!(quote_bytes_go_style(b"%\xf0\x9f"), r#""%\xf0\x9f""#);
    }

    #[test]
    fn quote_bytes_go_style_leaves_a_complete_character_unescaped() {
        // é = C3 A9, not truncated at all.
        assert_eq!(quote_bytes_go_style("%é".as_bytes()), "\"%é\"");
    }

    #[test]
    fn quote_bytes_go_style_escapes_special_ascii_like_rust_debug() {
        assert_eq!(quote_bytes_go_style(br#"%"y"#), r#""%\"y""#);
        assert_eq!(quote_bytes_go_style(br"%\y"), r#""%\\y""#);
        assert_eq!(quote_bytes_go_style(b"%\ty"), r#""%\ty""#);
    }

    #[test]
    fn quote_bytes_go_style_handles_empty_and_ascii_only() {
        assert_eq!(quote_bytes_go_style(b""), "\"\"");
        assert_eq!(quote_bytes_go_style(b"%"), "\"%\"");
        assert_eq!(quote_bytes_go_style(b"%y"), "\"%y\"");
    }

    /// A run of genuinely invalid bytes (not just an incomplete valid
    /// prefix) also hex-escapes, one `\xHH` per byte -- `error_len()`'s
    /// `Some(n)` branch, not just its `None` (ran-out-of-input) branch
    /// `quote_bytes_go_style`'s other tests exercise.
    #[test]
    fn quote_bytes_go_style_escapes_genuinely_invalid_bytes() {
        // 0xFF is never a valid UTF-8 lead byte at all.
        assert_eq!(quote_bytes_go_style(b"%\xff"), r#""%\xff""#);
        assert_eq!(quote_bytes_go_style(b"%\xff\xfe"), r#""%\xff\xfe""#);
    }

    #[test]
    fn urid_invalid_escape_message_matches_1216_wording() {
        assert_eq!(
            EvalError::urid_invalid_escape(b"%\xe4\xb8").message,
            r#"invalid URL escape "%\xe4\xb8""#
        );
    }
}
