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

use super::stream::stream_owned_value_json_jq;
use super::value::OwnedValue;

/// Error that occurs during evaluation.
#[derive(Debug, Clone, PartialEq)]
pub struct EvalError {
    pub message: String,

    /// The raw payload of `error(v)`, kept so that `catch` can inspect a
    /// non-string value: jq binds the *raised value* as the catch handler's
    /// input, so `try error({a:1}) catch .` must yield `{"a":1}` rather than
    /// the rendered string `"{\"a\":1}"`.
    ///
    /// `None` for errors raised internally by the evaluator (type errors and
    /// friends). jq models those as string errors, so [`EvalError::payload`]
    /// falls back to `message` wrapped in [`OwnedValue::String`].
    pub value: Option<OwnedValue>,
}

/// jq truncates values embedded in error messages to a fixed-width buffer
/// (`jv_dump_string_trunc` with `char errbuf[15]`): a JSON dump of at most
/// `DUMP_BUDGET` bytes is used verbatim, anything longer keeps its first
/// `DUMP_KEEP` bytes and gains a `...` suffix.
const DUMP_BUDGET: usize = 14;
const DUMP_KEEP: usize = 11;

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
fn dump_truncated(value: &OwnedValue) -> String {
    let mut sink = PreviewSink::new(DUMP_BUDGET);
    // The sink stops the writer once the dump is known to exceed the budget;
    // writing into a `String` cannot fail for any other reason, so the returned
    // `Result` carries nothing `sink.overflowed` has not already recorded.
    let _ = stream_owned_value_json_jq(value, &mut sink);
    if !sink.overflowed {
        return sink.buf;
    }
    sink.truncate_to(DUMP_KEEP);
    sink.buf.push_str("...");
    sink.buf
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
fn describe(value: &OwnedValue) -> String {
    format!("{} ({})", value.type_name(), dump_truncated(value))
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
            value: None,
        }
    }

    /// Create an error that raises `value` as its payload, as `error(v)` does.
    ///
    /// The message renders `value` the way jq reports it: a string payload is
    /// used as-is (`error("boom")` reports `boom`, not `"boom"`), anything else
    /// is serialized as JSON.
    pub fn from_value(value: OwnedValue) -> Self {
        let message = match &value {
            OwnedValue::String(s) => s.clone(),
            other => other.to_json(),
        };
        Self {
            message,
            value: Some(value),
        }
    }

    /// The value this error raises, as `catch` should see it.
    ///
    /// Errors from `error(v)` return `v` unchanged; internal errors return
    /// their message as a string, matching how jq raises them.
    pub fn payload(self) -> OwnedValue {
        self.value.unwrap_or(OwnedValue::String(self.message))
    }

    /// Create a type error.
    ///
    /// This is succinctly's own wording, kept for the error sites that have no
    /// jq counterpart. Anything jq also reports should use one of the named
    /// constructors below instead, so it matches byte for byte.
    pub fn type_error(expected: &str, got: &str) -> Self {
        Self::new(format!("expected {expected}, got {got}"))
    }

    /// Create a field not found error.
    pub fn field_not_found(name: &str) -> Self {
        Self::new(format!("field '{name}' not found"))
    }

    /// Create an index out of bounds error.
    pub fn index_out_of_bounds(index: i64, len: usize) -> Self {
        Self::new(format!("index {index} out of bounds (length {len})"))
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

    /// `Path must be specified as an array`.
    ///
    /// Raised by the path builtins when their path argument is not an array at
    /// all — `1 | setpath("a"; 1)`.
    pub fn path_must_be_array() -> Self {
        Self::new("Path must be specified as an array")
    }

    /// `Out of bounds negative array index`.
    ///
    /// jq raises this for a negative index that is still negative after being
    /// resolved against the array's length — `[1,2] | setpath([-5]; 9)`. The
    /// sentence carries neither the index nor the length.
    pub fn out_of_bounds_negative_index() -> Self {
        Self::new("Out of bounds negative array index")
    }

    /// `Cannot iterate over <type> (<value>)`.
    pub fn cannot_iterate(value: &OwnedValue) -> Self {
        Self::new(format!("Cannot iterate over {}", describe(value)))
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

    /// `<a> and <b> cannot have their containment checked`.
    pub fn containment_check(left: &OwnedValue, right: &OwnedValue) -> Self {
        Self::pair(left, right, "cannot have their containment checked")
    }

    /// `<a> and <b> cannot be iterated over` — jq's wording for `min`/`max`
    /// on a non-array, where both operands are the same value.
    pub fn pair_cannot_be_iterated(left: &OwnedValue, right: &OwnedValue) -> Self {
        Self::pair(left, right, "cannot be iterated over")
    }

    /// `<v> has no keys`.
    pub fn has_no_keys(value: &OwnedValue) -> Self {
        Self::subject(value, "has no keys")
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
        let err = EvalError::cannot_iterate(&OwnedValue::Int(1));
        assert_eq!(err.message, "Cannot iterate over number (1)");
        assert_eq!(err.value, None);
        assert_eq!(
            err.payload(),
            OwnedValue::String("Cannot iterate over number (1)".to_string())
        );
    }
}
