//! The message jq's JSON parser gives for text it refuses (#5309).
//!
//! `fromjson` and `tonumber` hand their string to `jv_parse`, which is the push parser in jq's
//! `jv_parse.c`, and the diagnostic it returns is a property of *that* parser: it reports the
//! first fault it meets, in the order it meets them, with the line and column it had reached.
//! `Invalid numeric literal at EOF at line 1, column 3` for `a,b` is wrong twice over: jq stops
//! at the `,` (`Invalid numeric literal at line 1, column 2`), and `[1,` is `Unfinished JSON term`
//! while `1 2` is `Unexpected extra JSON values`.
//!
//! This is a port of that machine's *error behaviour* only: it tracks the kinds jq tracks (an
//! array or object frame and whether it has an element, a pending key, whether a value is
//! waiting) and nothing about the values themselves. Whether a text parses at all is still the
//! validator's call; this runs on the refusal path to name the fault, and answers `None` for a
//! text it reads as valid so the caller can fall back to its older wording.
//!
//! Captured live against jq 1.7.1 and held to it by a differential run over ~430k strings:
//! every string of up to six characters over a structural alphabet, and 120k random strings
//! drawn from a pool of numbers, literals, escapes, newlines and truncated tokens.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// jq's `MAX_PARSING_DEPTH`: the stack of containers and pending keys.
///
/// An array holds one entry and an object two once its key is read, and a `[` or `{` is refused
/// when the stack already holds this many, so 256 nested arrays parse and 129 nested objects do
/// not. The one definition behind every route that applies jq's limit (#5497).
pub const JQ_MAX_PARSING_DEPTH: usize = 256;

/// The fault [`Parser::open`] stops on at [`JQ_MAX_PARSING_DEPTH`].
const DEPTH_FAULT: &str = "Exceeds depth limit for parsing";

/// The UTF-8 byte-order mark jq's parser skips at the front of its text.
const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// What stopped the parse: a diagnostic of the parser's own, or the second top-level value.
enum Stop {
    Fault(&'static str),
    ExtraValues,
}

/// A frame on jq's parse stack.
enum Frame {
    /// `[`, and whether an element has been appended.
    Array { has_element: bool },
    /// `{`, and whether a pair has been stored.
    Object { has_pair: bool },
    /// A key waiting for its value (`{"a":`).
    Key,
}

/// The kind of the value waiting in jq's `next` slot. Only a string matters, to `:`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pending {
    String,
    Other,
}

struct Parser {
    line: usize,
    column: usize,
    /// 0 = outside a string, 1 = inside, 2 = after a backslash.
    string_state: u8,
    token: Vec<u8>,
    stack: Vec<Frame>,
    next: Option<Pending>,
    /// Top-level values handed back so far; the second is `Unexpected extra JSON values`.
    emitted: usize,
    /// The stack size at which a `[` or `{` is refused: [`JQ_MAX_PARSING_DEPTH`], or one more
    /// under a wrapper jq never parsed ([`InputSource::Slurped`]).
    limit: usize,
}

/// jq's decNumber reader: an optionally signed decimal with an optional exponent, or
/// `inf`/`infinity`/`nan`/`snan` in any case (`nan` and `snan` with an optional payload).
fn is_number_token(token: &[u8]) -> bool {
    let body = match token.first() {
        Some(b'+' | b'-') => &token[1..],
        _ => token,
    };
    let lower: Vec<u8> = body.iter().map(u8::to_ascii_lowercase).collect();
    if lower == b"inf" || lower == b"infinity" {
        return true;
    }
    for word in [&b"snan"[..], b"nan"] {
        if let Some(payload) = lower.strip_prefix(word) {
            if payload.iter().all(u8::is_ascii_digit) {
                return true;
            }
        }
    }
    let digits = |from: usize| {
        lower[from..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count()
    };
    let mut at = 0;
    let whole = digits(at);
    at += whole;
    let mut fraction = 0;
    if lower.get(at) == Some(&b'.') {
        at += 1;
        fraction = digits(at);
        at += fraction;
    }
    if whole == 0 && fraction == 0 {
        return false;
    }
    if matches!(lower.get(at), Some(b'e')) {
        at += 1;
        if matches!(lower.get(at), Some(b'+' | b'-')) {
            at += 1;
        }
        let exponent = digits(at);
        if exponent == 0 {
            return false;
        }
        at += exponent;
    }
    at == lower.len()
}

/// `check_literal`'s refusal of a finished bare token, or the pending kind it yields.
fn literal(token: &[u8]) -> Result<Pending, Stop> {
    let expected: Option<&[u8]> = match token.first() {
        Some(b't') => Some(b"true"),
        Some(b'f') => Some(b"false"),
        Some(b'n') if token.get(1) == Some(&b'u') => Some(b"null"),
        _ => None,
    };
    match expected {
        Some(word) if token == word => Ok(Pending::Other),
        Some(_) => Err(Stop::Fault("Invalid literal")),
        // jq reads a number out of the token as a C string, so a NUL ends it (`1\0002` is 1);
        // the `true`/`false`/`null` comparison above is by length and sees the whole token.
        None if is_number_token(token.split(|&byte| byte == 0).next().unwrap_or(token)) => {
            Ok(Pending::Other)
        }
        None => Err(Stop::Fault("Invalid numeric literal")),
    }
}

fn hex4(digits: &[u8]) -> Option<u32> {
    if digits.len() < 4 || !digits[..4].iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    u32::from_str_radix(core::str::from_utf8(&digits[..4]).ok()?, 16).ok()
}

/// `found_string`'s checks on a finished string token (escapes, surrogate pairs, controls).
fn check_string(token: &[u8]) -> Result<(), Stop> {
    let mut at = 0;
    while at < token.len() {
        let byte = token[at];
        at += 1;
        if byte != b'\\' {
            if byte < 0x20 {
                return Err(Stop::Fault(
                    "Invalid string: control characters from U+0000 through U+001F must be escaped",
                ));
            }
            continue;
        }
        let Some(&escape) = token.get(at) else {
            return Err(Stop::Fault("Expected escape character at end of string"));
            // patchcov: coverage tolerate-line reason="unreachable: a backslash puts the scanner in its escape state, which cannot close the string, so a finished token never ends on one (jq keeps the check as a guard)"
        };
        at += 1;
        match escape {
            b'\\' | b'"' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => {}
            b'u' => {
                if at + 4 > token.len() {
                    return Err(Stop::Fault("Invalid \\uXXXX escape"));
                }
                let Some(code) = hex4(&token[at..]) else {
                    return Err(Stop::Fault("Invalid characters in \\uXXXX escape"));
                };
                at += 4;
                if (0xD800..=0xDBFF).contains(&code) {
                    const PAIR: Stop = Stop::Fault("Invalid \\uXXXX\\uXXXX surrogate pair escape");
                    if at + 6 > token.len() || token[at] != b'\\' || token[at + 1] != b'u' {
                        return Err(PAIR);
                    }
                    match hex4(&token[at + 2..]) {
                        Some(low) if (0xDC00..=0xDFFF).contains(&low) => at += 6,
                        _ => return Err(PAIR),
                    }
                }
            }
            _ => return Err(Stop::Fault("Invalid escape")),
        }
    }
    Ok(())
}

impl Parser {
    /// A parser whose first byte is at `line` (1-based) and just after `column` bytes of it, as
    /// jq's own counters stand there.
    fn new(line: usize, column: usize, limit: usize) -> Self {
        Self {
            limit,
            line,
            column,
            string_state: 0,
            token: Vec::new(),
            stack: Vec::new(),
            next: None,
            emitted: 0,
        }
    }

    /// `value`: a finished value goes in the `next` slot, which must be empty.
    fn value(&mut self, kind: Pending) -> Result<(), Stop> {
        if self.next.is_some() {
            return Err(Stop::Fault("Expected separator between values"));
        }
        self.next = Some(kind);
        Ok(())
    }

    fn check_literal(&mut self) -> Result<(), Stop> {
        if self.token.is_empty() {
            return Ok(());
        }
        let kind = literal(&self.token)?;
        self.value(kind)?;
        self.token.clear();
        Ok(())
    }

    /// A top-level value is complete once the stack is empty and `next` is full.
    fn complete(&self) -> bool {
        self.stack.is_empty() && self.next.is_some()
    }

    /// Hand the completed value back to the caller; the second one ends the parse.
    fn emit(&mut self) -> Result<(), Stop> {
        self.emitted += 1;
        if self.emitted == 2 {
            return Err(Stop::ExtraValues);
        }
        Ok(())
    }

    fn open(&mut self, frame: Frame) -> Result<(), Stop> {
        if self.stack.len() >= self.limit {
            return Err(Stop::Fault(DEPTH_FAULT));
        }
        if self.next.is_some() {
            return Err(Stop::Fault("Expected separator between values"));
        }
        self.stack.push(frame);
        Ok(())
    }

    fn token(&mut self, ch: u8) -> Result<(), Stop> {
        match ch {
            b'[' => self.open(Frame::Array { has_element: false })?,
            b'{' => self.open(Frame::Object { has_pair: false })?,
            b':' => {
                let Some(pending) = self.next else {
                    return Err(Stop::Fault("Expected string key before ':'"));
                };
                if !matches!(self.stack.last(), Some(Frame::Object { .. })) {
                    return Err(Stop::Fault("':' not as part of an object"));
                }
                if pending != Pending::String {
                    return Err(Stop::Fault("Object keys must be strings"));
                }
                self.stack.push(Frame::Key);
                self.next = None;
            }
            b',' => {
                if self.next.is_none() || self.stack.is_empty() {
                    return Err(Stop::Fault("Expected value before ','"));
                }
                match self.stack.last_mut() {
                    Some(Frame::Array { has_element }) => *has_element = true,
                    Some(Frame::Key) => {
                        self.stack.pop();
                        if let Some(Frame::Object { has_pair }) = self.stack.last_mut() {
                            *has_pair = true;
                        }
                    }
                    _ => return Err(Stop::Fault("Objects must consist of key:value pairs")),
                }
                self.next = None;
            }
            b']' => {
                let Some(Frame::Array { has_element }) = self.stack.last() else {
                    return Err(Stop::Fault("Unmatched ']'"));
                };
                if self.next.is_none() && *has_element {
                    return Err(Stop::Fault("Expected another array element"));
                }
                self.stack.pop();
                self.next = Some(Pending::Other);
            }
            _ => {
                // `}`
                let Some(top) = self.stack.last() else {
                    return Err(Stop::Fault("Unmatched '}'"));
                };
                if self.next.is_some() {
                    if !matches!(top, Frame::Key) {
                        return Err(Stop::Fault("Objects must consist of key:value pairs"));
                    }
                    self.stack.pop();
                } else {
                    match top {
                        Frame::Object { has_pair: true } => {
                            return Err(Stop::Fault("Expected another key-value pair"));
                        }
                        Frame::Object { has_pair: false } => {}
                        _ => return Err(Stop::Fault("Unmatched '}'")),
                    }
                }
                self.stack.pop();
                self.next = Some(Pending::Other);
            }
        }
        Ok(())
    }

    /// `scan`: one byte.
    fn feed(&mut self, ch: u8) -> Result<(), Stop> {
        self.column += 1;
        if ch == b'\n' {
            self.line += 1;
            self.column = 0;
        }
        if self.string_state == 0 {
            let structural = matches!(ch, b'[' | b'{' | b',' | b':' | b']' | b'}');
            let whitespace = matches!(ch, b' ' | b'\t' | b'\r' | b'\n');
            if structural || whitespace || ch == b'"' {
                self.check_literal()?;
                if structural {
                    // The finished top-level value is handed back before the token runs, but
                    // only counts if the token raises nothing: a token's fault wins over it.
                    let pending = self.complete();
                    if pending {
                        self.next = None;
                    }
                    self.token(ch)?;
                    if pending {
                        self.emit()?;
                    }
                }
            } else {
                self.token.push(ch);
            }
            if ch == b'"' {
                self.string_state = 1;
            }
            self.complete_and_emit()
        } else if ch == b'"' && self.string_state == 1 {
            check_string(&self.token)?;
            self.token.clear();
            self.value(Pending::String)?;
            self.string_state = 0;
            self.complete_and_emit()
        } else {
            self.token.push(ch);
            self.string_state = if ch == b'\\' && self.string_state == 1 {
                2
            } else {
                1
            };
            Ok(())
        }
    }

    fn complete_and_emit(&mut self) -> Result<(), Stop> {
        if self.complete() {
            self.next = None;
            self.emit()?;
        }
        Ok(())
    }

    /// End of text: an open string or container is unfinished; a bare token still waiting is
    /// checked now.
    fn finish(&mut self) -> Result<(), Stop> {
        if self.string_state != 0 {
            return Err(Stop::Fault("Unfinished string"));
        }
        self.check_literal()?;
        if !self.stack.is_empty() {
            return Err(Stop::Fault("Unfinished JSON term"));
        }
        self.complete_and_emit()
    }
}

/// The diagnostic `jv_parse` gives for `text`, without its trailing `(while parsing '...')`, or
/// `None` when jq's parser reads `text` as exactly one JSON value.
///
/// `text` is parsed whole, NULs included: jq's parser takes a length, not a C string.
pub(crate) fn jq_parse_message(text: &[u8]) -> Option<String> {
    jq_parse_fault(text, 1, 0, JQ_MAX_PARSING_DEPTH).map(|fault| fault.message)
}

/// What stopped jq's parser on a text it refuses (#5497).
struct ParseFault {
    /// Whether it stopped on [`JQ_MAX_PARSING_DEPTH`].
    depth: bool,
    /// The byte of `text` it stopped on, or `text`'s length when the fault is at its end.
    offset: usize,
    /// jq's diagnostic, worded as [`jq_parse_message`] words it.
    message: String,
}

/// [`jq_parse_message`] for a text that starts at `line` and `column` of a longer input, so the
/// position in the message is the one jq names for that input, refusing a bracket once `limit`
/// stack entries are open.
fn jq_parse_fault(text: &[u8], line: usize, column: usize, limit: usize) -> Option<ParseFault> {
    let full = text.len();
    let text = text.strip_prefix(UTF8_BOM).unwrap_or(text);
    let bom = full - text.len();
    let mut parser = Parser::new(line, column, limit);
    let mut offset = full;
    let outcome = text
        .iter()
        .enumerate()
        .try_for_each(|(at, &byte)| {
            offset = bom + at;
            parser.feed(byte)
        })
        .map_err(|stop| (stop, false))
        .and_then(|()| {
            offset = full;
            parser.finish().map_err(|stop| (stop, true))
        });
    let (depth, message) = match outcome {
        Err((Stop::ExtraValues, _)) => (false, String::from("Unexpected extra JSON values")),
        Err((Stop::Fault(fault), at_eof)) => (
            fault == DEPTH_FAULT,
            if at_eof {
                format!(
                    "{fault} at EOF at line {}, column {}",
                    parser.line, parser.column
                )
            } else {
                format!("{fault} at line {}, column {}", parser.line, parser.column)
            },
        ),
        Ok(()) if parser.emitted == 0 => (false, String::from("Expected JSON value")),
        Ok(()) => return None,
    };
    Some(ParseFault {
        depth,
        offset,
        message,
    })
}

/// The byte of `text` at which jq's parser, reading it from its start, refuses a bracket past
/// [`JQ_MAX_PARSING_DEPTH`], or `None` when it stops on anything else first (#5497).
///
/// For a caller that must place jq's depth refusal in its input, where a validator with a
/// limit of its own stops somewhere else.
#[doc(hidden)]
pub fn jq_depth_fault_offset(text: &[u8]) -> Option<usize> {
    jq_parse_fault(text, 1, 0, JQ_MAX_PARSING_DEPTH)
        .filter(|fault| fault.depth)
        .map(|fault| fault.offset)
}

/// Where a document checked against jq's parse-depth limit came from (#5497).
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputSource {
    /// A document of the input stream, as jq's parser reads it.
    Stream,
    /// The `[...]` `--slurp` joins every document into. jq parses the documents, never the
    /// array, so the limit applies to each member, and the files are gone by the time it is
    /// indexed, so the message has no position.
    Slurped,
}

/// jq's message when its parse-depth limit refuses the document `text`, whose index has the
/// balanced parentheses `bp`, or `None` when jq's parser gets past every bracket in it (#5497).
///
/// `position` places a [`InputSource::Stream`] document in its input as jq counts: the 1-based
/// line its first byte is on, and the bytes before it on that line. It is asked only when the
/// parser runs, so a caller can count lines there without charging every document for it.
///
/// Only the depth limit is reported. A document whose first fault is anything else is left to
/// the route's own handling, so this refuses nothing jq's limit does not.
///
/// The parser runs only on a document that could reach the limit. A `[` or `{` is refused once
/// [`JQ_MAX_PARSING_DEPTH`] entries are open, and no container adds more than two, so the
/// refused bracket is at least the 129th nested one. The BP has one open per container and
/// one per leaf, so its excess is at most the container depth plus one, and a BP that never
/// rises above 128 cannot hold a refused bracket. That costs one popcount per 64 parentheses
/// ([`BalancedParens::max_excess_exceeds`](crate::trees::BalancedParens::max_excess_exceeds)),
/// where the parser reads every byte.
#[doc(hidden)]
pub fn jq_input_depth_fault<W: AsRef<[u64]>>(
    text: &[u8],
    bp: &crate::trees::BalancedParens<W>,
    source: InputSource,
    position: impl FnOnce() -> (usize, usize),
) -> Option<String> {
    // The slurp wrapper adds one level to every member, so the bound moves with the limit.
    let wrapper = usize::from(source == InputSource::Slurped);
    if !bp.max_excess_exceeds(JQ_MAX_PARSING_DEPTH / 2 + wrapper) {
        return None;
    }
    let (line, column) = match source {
        InputSource::Stream => position(),
        InputSource::Slurped => (1, 0),
    };
    let fault = jq_parse_fault(text, line, column, JQ_MAX_PARSING_DEPTH + wrapper)?;
    fault.depth.then(|| match source {
        InputSource::Stream => fault.message,
        InputSource::Slurped => String::from(DEPTH_FAULT),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(text: &str) -> Option<String> {
        jq_parse_message(text.as_bytes())
    }

    /// Every row was captured live from `/usr/bin/jq` 1.7.1: `"<text>" | try fromjson catch .`,
    /// less the `(while parsing '<text>')` it appends.
    #[test]
    fn names_the_fault_jq_names_5309() {
        for (text, want) in [
            ("a,b", "Invalid numeric literal at line 1, column 2"),
            ("a b", "Invalid numeric literal at line 1, column 2"),
            ("a", "Invalid numeric literal at EOF at line 1, column 1"),
            ("nul", "Invalid literal at EOF at line 1, column 3"),
            ("truee", "Invalid literal at EOF at line 1, column 5"),
            ("1 2", "Unexpected extra JSON values"),
            ("true false", "Unexpected extra JSON values"),
            ("[1,", "Unfinished JSON term at EOF at line 1, column 3"),
            ("{", "Unfinished JSON term at EOF at line 1, column 1"),
            ("[1,]", "Expected another array element at line 1, column 4"),
            (
                "{\"a\":1,}",
                "Expected another key-value pair at line 1, column 8",
            ),
            (
                "{\"a\" 1}",
                "Expected separator between values at line 1, column 7",
            ),
            (
                "[1 2]",
                "Expected separator between values at line 1, column 5",
            ),
            ("{\"a\":}", "Unmatched '}' at line 1, column 6"),
            ("}", "Unmatched '}' at line 1, column 1"),
            ("{]", "Unmatched ']' at line 1, column 2"),
            (
                "[1}",
                "Objects must consist of key:value pairs at line 1, column 3",
            ),
            ("1,2", "Expected value before ',' at line 1, column 2"),
            ("1:2", "Expected string key before ':' at line 1, column 2"),
            ("\"abc", "Unfinished string at EOF at line 1, column 4"),
            ("\"a\\x\"", "Invalid escape at line 1, column 5"),
            ("a\n,b", "Invalid numeric literal at line 2, column 0"),
            (
                "\u{feff}\u{feff}1",
                "Invalid numeric literal at EOF at line 1, column 4",
            ),
            ("", "Expected JSON value"),
            (" ", "Expected JSON value"),
        ] {
            assert_eq!(message(text).as_deref(), Some(want), "{text:?}");
        }
    }

    #[test]
    fn reads_a_single_value_as_valid() {
        for text in [
            "1",
            " [1, {\"a\": \"b\"}] ",
            "null",
            "-Infinity",
            "nan",
            "\u{feff}[1]",
            "{}",
            ".5",
            "1.e5",
            "\"\\ud83d\\ude00\"",
        ] {
            assert_eq!(message(text), None, "{text:?}");
        }
    }

    /// The faults a container reports for a misplaced token, and the value that survives a
    /// following token (every row captured live from jq 1.7.1).
    #[test]
    fn misplaced_tokens_5309() {
        for (text, want) in [
            (
                "[1[",
                "Expected separator between values at line 1, column 3",
            ),
            ("[1:", "':' not as part of an object at line 1, column 3"),
            ("{1:", "Object keys must be strings at line 1, column 3"),
            (
                "{\"a\",",
                "Objects must consist of key:value pairs at line 1, column 5",
            ),
            // The second top-level value is complete when the `[` arrives, and that is the fault.
            ("1 2[", "Unexpected extra JSON values"),
            ("1e", "Invalid numeric literal at EOF at line 1, column 2"),
            ("1e+", "Invalid numeric literal at EOF at line 1, column 3"),
            (".", "Invalid numeric literal at EOF at line 1, column 1"),
            // Valid escapes, and a surrogate pair, inside a text that fails later.
            (
                "\"a\\n\\/\\u00e9\\ud83d\\ude00\" 1",
                "Unexpected extra JSON values",
            ),
        ] {
            assert_eq!(message(text).as_deref(), Some(want), "{text:?}");
        }
        assert_eq!(message("1e+5"), None);
        assert_eq!(message("-1E-5"), None);
    }

    #[test]
    fn string_escapes_and_depth() {
        assert_eq!(
            message("\"\\ud800\"").as_deref(),
            Some("Invalid \\uXXXX\\uXXXX surrogate pair escape at line 1, column 8")
        );
        assert_eq!(
            message("\"\\u12\"").as_deref(),
            Some("Invalid \\uXXXX escape at line 1, column 6")
        );
        assert_eq!(
            message("\"\\u12zz\"").as_deref(),
            Some("Invalid characters in \\uXXXX escape at line 1, column 8")
        );
        assert_eq!(
            message("\"a\u{1}\"").as_deref(),
            Some(
                "Invalid string: control characters from U+0000 through U+001F must be escaped \
                 at line 1, column 4"
            )
        );
        let deep = "[".repeat(257);
        assert_eq!(
            message(&deep).as_deref(),
            Some("Exceeds depth limit for parsing at line 1, column 257")
        );
        assert_eq!(
            message(&"[".repeat(256)).as_deref(),
            Some("Unfinished JSON term at EOF at line 1, column 256")
        );
    }

    fn depth_fault(text: &str, line: usize, column: usize) -> Option<String> {
        let index = crate::json::JsonIndex::build(text.as_bytes());
        jq_input_depth_fault(text.as_bytes(), index.bp(), InputSource::Stream, || {
            (line, column)
        })
    }

    fn nest(open: &str, inner: &str, close: &str, levels: usize) -> String {
        format!("{}{inner}{}", open.repeat(levels), close.repeat(levels))
    }

    /// Every boundary captured live from `/usr/bin/jq` 1.7.1 (`jq -c length`): an object holds two
    /// stack entries once its key is read, but the key itself is never refused (#5497).
    #[test]
    fn input_depth_fault_refuses_what_jq_refuses_5497() {
        for (text, want) in [
            (nest("[", "", "]", 256), None),
            (nest("[", "1", "]", 256), None),
            (
                nest("[", "", "]", 257),
                Some("Exceeds depth limit for parsing at line 1, column 257"),
            ),
            (nest("{\"a\":", "1", "}", 128), None),
            (
                nest("{\"a\":", "1", "}", 129),
                Some("Exceeds depth limit for parsing at line 1, column 641"),
            ),
            // The BP's excess peaks at exactly 129 here (the 128th key and the empty object),
            // so this is the shape that pins the pre-filter's bound.
            (
                nest("{\"a\":", "{}", "}", 128),
                Some("Exceeds depth limit for parsing at line 1, column 641"),
            ),
            (nest("[{\"a\":", "1", "}]", 85), None),
            (
                nest("[{\"a\":", "1", "}]", 86),
                Some("Exceeds depth limit for parsing at line 1, column 512"),
            ),
            (nest("[", "{\"a\":1}", "]", 255), None),
            (nest("[", "{}", "]", 255), None),
            (
                nest("[", "{\"a\":[1]}", "]", 254),
                Some("Exceeds depth limit for parsing at line 1, column 260"),
            ),
        ] {
            assert_eq!(depth_fault(&text, 1, 0).as_deref(), want, "{text}");
        }
    }

    /// The position continues from where the document sits in its input, as jq counts it: a
    /// newline inside the document moves the line, and the column restarts there.
    #[test]
    fn input_depth_fault_names_the_position_in_the_input_5497() {
        let deep = nest("[", "", "]", 300);
        assert_eq!(
            depth_fault(&deep, 4, 0).as_deref(),
            Some("Exceeds depth limit for parsing at line 4, column 257")
        );
        assert_eq!(
            depth_fault(&deep, 2, 10).as_deref(),
            Some("Exceeds depth limit for parsing at line 2, column 267")
        );
        let broken = format!("[\n{}", &deep[1..]);
        assert_eq!(
            depth_fault(&broken, 3, 5).as_deref(),
            Some("Exceeds depth limit for parsing at line 4, column 256")
        );
    }

    /// A document jq refuses for something else first is not a depth refusal, and a bracket
    /// inside a string is not a container.
    #[test]
    fn input_depth_fault_reports_only_the_depth_limit_5497() {
        let syntax_first = format!("[1,,{}]", nest("[", "", "]", 300));
        assert_eq!(depth_fault(&syntax_first, 1, 0), None);
        let strings = nest("[", "\"[[[[{{{{\"", "]", 200);
        assert_eq!(depth_fault(&strings, 1, 0), None);
    }

    /// Under `--slurp` the synthesized array is not one of jq's levels: its members get the
    /// whole limit, and the message has no position to name.
    #[test]
    fn input_depth_fault_does_not_count_the_slurp_wrapper_5497() {
        let slurped = |members: &[String]| {
            let text = format!("[{}]", members.join(","));
            let index = crate::json::JsonIndex::build(text.as_bytes());
            // A position the message would show if it were used.
            jq_input_depth_fault(text.as_bytes(), index.bp(), InputSource::Slurped, || (9, 9))
        };
        assert_eq!(slurped(&[nest("[", "1", "]", 256), "2".into()]), None);
        assert_eq!(slurped(&[nest("{\"a\":", "1", "}", 128)]), None);
        assert_eq!(
            slurped(&["1".into(), nest("[", "", "]", 257)]).as_deref(),
            Some("Exceeds depth limit for parsing")
        );
        assert_eq!(
            slurped(&[nest("{\"a\":", "1", "}", 129)]).as_deref(),
            Some("Exceeds depth limit for parsing")
        );
    }

    #[test]
    fn depth_fault_offset_is_the_refused_bracket_5497() {
        let deep = nest("[", "", "]", 300);
        assert_eq!(jq_depth_fault_offset(deep.as_bytes()), Some(256));
        // jq reads on into the next value, and meets the limit there.
        let after = format!("1 {deep} 2");
        assert_eq!(jq_depth_fault_offset(after.as_bytes()), Some(2 + 256));
        let syntax_first = format!("[1,,{deep}]");
        assert_eq!(jq_depth_fault_offset(syntax_first.as_bytes()), None);
        let objects = format!("\u{feff}{}", nest("{\"a\":", "1", "}", 129));
        assert_eq!(jq_depth_fault_offset(objects.as_bytes()), Some(3 + 640));
        assert_eq!(jq_depth_fault_offset(b"[1,,2]"), None);
    }
}
