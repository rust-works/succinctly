//! Real jq's `--stream-errors` reader (#5474).
//!
//! `--stream-errors` implies `--stream` and changes one thing about it: a parse error is not
//! raised, it becomes the next input, an array `[message, path]` whose `path` is where the
//! streaming parser stood (`null` for an object key it has not read yet). The reader goes on
//! with the next line, since jq throws away only the rest of the line the error was in.
//!
//! That cannot be had from the whole-document parse `--stream` builds its events from: the
//! events a malformed document produced before the fault, and the path at the fault, are
//! properties of jq's own incremental parser. So this is a port of the streaming half of jq
//! 1.7.1's `jv_parse.c` (`stream_token`, `stream_check_done`, `scan`, `check_literal`,
//! `found_string` and the EOF branch of `jv_parser_next`), checked against the pinned
//! `/usr/bin/jq` rather than written from the documentation. It reuses
//! [`crate::jq_seq_reader`]'s port of the tokenizer pieces both modes share: the `fgets` chunking,
//! the number grammar, `\uXXXX`.
//!
//! What it keeps from jq on purpose:
//!
//! - **The input is read a line at a time.** jq feeds its parser one `fgets` chunk (a line, or
//!   4095 bytes) per call and, on an error, drops what is left of the chunk
//!   (`p->curr_buf = 0`). Parser state, including the line and column counters, carries over
//!   from one chunk and one file to the next, but the dropped bytes are never counted.
//! - **A value that completes on the byte that then fails is lost.** `1}` reports
//!   `Unmatched '}' at the top-level` without the `[[],1]` event `1` had just produced, because
//!   `scan` returns the error and not the value.
//! - **There is no depth limit.** `parse_token` checks `MAX_PARSING_DEPTH`, `stream_token`
//!   does not.
//! - **A token that ends a file continues in the next one.** The parser is never reset between
//!   files.
//!
//! The scalars and strings are materialized by the checked document parser from the token's
//! bytes, so a number keeps its spelling and a string its decoding exactly as a document's would;
//! this module decides only where each one begins and ends and which error fires.

use crate::jq_seq_reader::{fgets_chunk_lens, number_is_valid, unhex4, JQ_FGETS_CHUNK};
use succinctly::jq::OwnedValue;

/// What jq's `p->last_seen` can hold.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LastSeen {
    None,
    OpenArray,
    OpenObject,
    Colon,
    Comma,
    Value,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum St {
    Normal,
    Str,
    StrEscape,
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

/// The inputs `--stream-errors` hands the program, with where jq's reader stood for each.
pub(crate) struct StreamErrorsRead {
    pub(crate) values: Vec<OwnedValue>,
    /// Parallel to `values`: the index into the source list and the number of lines jq had read
    /// from that source when it yielded the value, or [`CLOSED`] for a value jq yielded after
    /// it had closed its input (`input_filename` is `null` and `input_line_number` `0` there).
    pub(crate) locations: Vec<(usize, usize)>,
    /// How many of `values` jq's `--slurp` loop puts in its array. The loop ends at the first call
    /// that consumes the last byte of the last read, so what that call leaves pending (the close
    /// of a container) and the EOF branch's report come out after the array, as inputs of their
    /// own.
    pub(crate) inside: usize,
    /// Where jq stood when it closed the array: a source index and a line count.
    pub(crate) slurp_at: (usize, usize),
    /// The line count of the last source if an `input` that runs past the end of the stream is the
    /// call that reaches the end of the file, so that jq's input is still open to name a line: it
    /// is when no value came out of the last refill. `None` when an earlier call already
    /// reached the end and jq has closed the file (`<unknown>`).
    pub(crate) open_at_end: Option<usize>,
}

/// The line count of a value yielded once jq's input is closed.
pub(crate) const CLOSED: usize = usize::MAX;

/// One refill of jq's input buffer (`jq_util_input_read_more`).
struct Refill<'a> {
    source: usize,
    bytes: &'a [u8],
    /// Whether the chunk holds a newline, which `read_more` counts as a line.
    newline: bool,
    /// Whether this refill is the last one, `read_more`'s return value.
    is_last: bool,
}

/// The refills jq makes over `sources`, in order.
///
/// A file is read a line at a time (`fgets` into 4096 bytes), and the refill is the last one when
/// `fgets` hit the end of the last file: a chunk that ends in a newline, or fills the buffer, has
/// not hit it yet and is followed by a refill that returns nothing. The parser sees a chunk with no
/// newline up to its first NUL (`strlen`).
fn refills(sources: &[(Option<usize>, Vec<u8>)]) -> Vec<Refill<'_>> {
    let mut refills: Vec<Refill<'_>> = Vec::new();
    for (index, (_, raw)) in sources.iter().enumerate() {
        let last_source = index + 1 == sources.len();
        let lens: Vec<usize> = fgets_chunk_lens(raw).collect();
        if lens.is_empty() {
            refills.push(Refill {
                source: index,
                bytes: &[],
                newline: false,
                is_last: last_source,
            });
            continue;
        }
        let mut start = 0;
        for (k, &len) in lens.iter().enumerate() {
            let chunk = &raw[start..start + len];
            start += len;
            let newline = chunk.ends_with(b"\n");
            let hit_eof = !newline && len < JQ_FGETS_CHUNK;
            let bytes = if newline {
                chunk
            } else {
                &chunk[..chunk.iter().position(|&b| b == 0).unwrap_or(len)]
            };
            refills.push(Refill {
                source: index,
                bytes,
                newline,
                is_last: last_source && k + 1 == lens.len() && hit_eof,
            });
        }
    }
    if let Some(last) = refills.last() {
        if !last.is_last {
            refills.push(Refill {
                source: last.source,
                bytes: &[],
                newline: false,
                is_last: true,
            });
        }
    }
    refills
}

/// Read `sources` (each source's bytes as read, a leading BOM already removed) the way
/// `jq --stream-errors` does.
pub(crate) fn read(sources: &[(Option<usize>, Vec<u8>)]) -> StreamErrorsRead {
    let mut parser = Parser::new();
    // What the last byte of a refill left for the next call to hand out.
    let mut deferred: Vec<OwnedValue> = Vec::new();
    let mut inside = None;
    let mut slurp_at = (0, 0);
    let mut finished = false;
    let mut values_before_last_refill = 0;
    for refill in refills(sources) {
        if refill.is_last {
            values_before_last_refill = parser.values.len();
        }
        if parser.source != refill.source {
            parser.source = refill.source;
            parser.lines_read = 0;
        }
        if refill.newline {
            parser.lines_read += 1;
        }
        // The call that follows a refill starts with `stream_check_done`.
        let drained = !deferred.is_empty();
        for value in std::mem::take(&mut deferred) {
            parser.yield_value(value);
        }
        let ended_with_value = parser.feed(refill.bytes, &mut deferred);
        if refill.is_last {
            // The slurp loop ends with the first call that leaves nothing in the buffer.
            let first_call_empties = if refill.bytes.is_empty() {
                drained
            } else {
                ended_with_value
            };
            if !first_call_empties {
                parser.finish();
                finished = true;
            }
            inside = Some(parser.values.len());
            slurp_at = (parser.source, parser.lines_read);
        }
    }
    // Whatever is left comes from calls made after jq has closed its input.
    parser.closed = true;
    for value in std::mem::take(&mut deferred) {
        parser.yield_value(value);
    }
    if !finished {
        parser.finish();
    }
    StreamErrorsRead {
        inside: inside.unwrap_or(parser.values.len()),
        slurp_at,
        open_at_end: (parser.values.len() == values_before_last_refill)
            .then_some(parser.lines_read),
        values: parser.values,
        locations: parser.locations,
    }
}

struct Parser {
    line: usize,
    column: usize,
    st: St,
    /// jq's `p->path`; its length is jq's `p->stacklen`. An element is a number (inside an array),
    /// a string (inside an object, once its key is known) or `null` (inside an object, key not yet
    /// read).
    path: Vec<OwnedValue>,
    last_seen: LastSeen,
    /// jq's `p->output`: zero to three elements. `[path]` closes a container, `[path, value]` is a
    /// leaf, and `[path, value, true]` is a leaf whose container closes right after it, which
    /// [`Parser::check_done`] hands out as the leaf and then `[path]`.
    output: Vec<OwnedValue>,
    next: Option<OwnedValue>,
    /// jq's `tokenbuf`/`tokenpos`, as in [`crate::jq_seq_reader`]: a buffer that is not cleared
    /// with the token, because `check_literal` reads `tokenbuf[1]` without checking `tokenpos`.
    token_buf: Vec<u8>,
    token_len: usize,
    source: usize,
    lines_read: usize,
    /// Set once jq has closed its input, for the calls made after the last refill.
    closed: bool,
    values: Vec<OwnedValue>,
    locations: Vec<(usize, usize)>,
}

impl Parser {
    fn new() -> Self {
        Self {
            line: 1,
            column: 0,
            st: St::Normal,
            path: Vec::new(),
            last_seen: LastSeen::None,
            output: Vec::new(),
            next: None,
            token_buf: Vec::new(),
            token_len: 0,
            source: 0,
            lines_read: 0,
            closed: false,
            values: Vec::new(),
            locations: Vec::new(),
        }
    }

    /// jq's `parser_reset` for a streaming parser.
    fn reset(&mut self) {
        self.path.clear();
        self.last_seen = LastSeen::None;
        self.output.clear();
        self.next = None;
        self.token_len = 0;
        self.st = St::Normal;
    }

    fn yield_value(&mut self, value: OwnedValue) {
        self.values.push(value);
        let line = if self.closed { CLOSED } else { self.lines_read };
        self.locations.push((self.source, line));
    }

    fn path_value(&self) -> OwnedValue {
        OwnedValue::array_from(self.path.clone())
    }

    /// jq's `make_error` with `JV_PARSE_STREAM_ERRORS`: the message and the path as it stands.
    fn error_value(&self, message: String) -> OwnedValue {
        OwnedValue::array_from(vec![OwnedValue::String(message.into()), self.path_value()])
    }

    /// One refill's bytes, which are one call to `jv_parser_next` or more: the values it completes
    /// are yielded as they are found, and an error ends the refill. What the refill's last byte
    /// leaves pending goes to `deferred`, since the call that hands it out comes after the next
    /// refill. Whether the last call yielded a value, which an error does wherever it fell, is
    /// the result.
    fn feed(&mut self, chunk: &[u8], deferred: &mut Vec<OwnedValue>) -> bool {
        for (index, &ch) in chunk.iter().enumerate() {
            let last = index + 1 == chunk.len();
            match self.scan(ch) {
                Ok(Some(value)) => {
                    self.yield_value(value);
                    // The next call starts with `stream_check_done`, which hands out whatever the
                    // byte left pending (the close of a container) before it scans another.
                    while let Some(value) = self.check_done() {
                        if last {
                            deferred.push(value);
                        } else {
                            self.yield_value(value);
                        }
                    }
                    if last {
                        return true;
                    }
                }
                Ok(None) => {}
                Err(message) => {
                    let error = self.error_value(format!(
                        "{message} at line {}, column {}",
                        self.line, self.column
                    ));
                    self.reset();
                    self.yield_value(error);
                    // `p->curr_buf = 0`: the rest of this refill is thrown away.
                    return true;
                }
            }
        }
        false
    }

    /// The EOF branch of `jv_parser_next`.
    fn finish(&mut self) {
        let (line, column) = (self.line, self.column);
        if self.st != St::Normal {
            let error = self.error_value(format!(
                "Unfinished string at EOF at line {line}, column {column}"
            ));
            self.yield_value(error);
            return;
        }
        if let Err(message) = self.check_literal() {
            let error =
                self.error_value(format!("{message} at EOF at line {line}, column {column}"));
            self.yield_value(error);
            return;
        }
        if !self.path.is_empty() {
            let error = self.error_value(format!(
                "Unfinished JSON term at EOF at line {line}, column {column}"
            ));
            self.yield_value(error);
            return;
        }
        if let Some(value) = self.next.take() {
            let event = OwnedValue::array_from(vec![self.path_value(), value]);
            self.yield_value(event);
        }
    }

    /// jq's `scan`, for a streaming parser that is not reading a `--seq`.
    fn scan(&mut self, ch: u8) -> Result<Option<OwnedValue>, &'static str> {
        self.column += 1;
        if ch == b'\n' {
            self.line += 1;
            self.column = 0;
        }
        // A value found before the byte fails is dropped with the failure: jq returns the error
        // and not `*out`.
        let mut answer = None;
        if self.st == St::Normal {
            let cls = classify(ch);
            if cls != Cls::Literal {
                self.check_literal()?;
                if let Some(value) = self.check_done() {
                    answer = Some(value);
                }
            }
            match cls {
                Cls::Literal => self.token_add(ch),
                Cls::Whitespace => {}
                Cls::Quote => self.st = St::Str,
                Cls::Structure => self.stream_token(ch)?,
            }
            if let Some(value) = self.check_done() {
                answer = Some(value);
            }
        } else if ch == b'"' && self.st == St::Str {
            self.found_string()?;
            self.st = St::Normal;
            if let Some(value) = self.check_done() {
                answer = Some(value);
            }
        } else {
            self.token_add(ch);
            self.st = if ch == b'\\' && self.st == St::Str {
                St::StrEscape
            } else {
                St::Str
            };
        }
        Ok(answer)
    }

    /// jq's `tokenadd`, growth policy included.
    fn token_add(&mut self, ch: u8) {
        if self.token_len == self.token_buf.len() {
            self.token_buf.resize(self.token_buf.len() * 2 + 256, 0);
        }
        self.token_buf[self.token_len] = ch;
        self.token_len += 1;
    }

    /// jq's `stream_check_done`: the next value to hand out, if any. A top-level scalar goes out as
    /// `[[], scalar]`; otherwise the pending `output`, a leaf-and-close pair as the leaf first.
    fn check_done(&mut self) -> Option<OwnedValue> {
        if self.path.is_empty() {
            if let Some(value) = self.next.take() {
                return Some(OwnedValue::array_from(vec![self.path_value(), value]));
            }
        }
        if self.output.is_empty() {
            return None;
        }
        if self.output.len() > 2 {
            let leaf = OwnedValue::array_from(vec![self.output[0].clone(), self.output[1].clone()]);
            self.output.truncate(1);
            return Some(leaf);
        }
        Some(OwnedValue::array_from(std::mem::take(&mut self.output)))
    }

    /// jq's `value`, for a streaming parser.
    fn value(&mut self, value: OwnedValue) -> Result<(), &'static str> {
        if self.next.is_some() || self.last_seen == LastSeen::Value {
            return Err("Expected separator between values");
        }
        self.last_seen = if self.path.is_empty() {
            LastSeen::None
        } else {
            LastSeen::Value
        };
        self.next = Some(value);
        Ok(())
    }

    fn last_is_number(&self) -> bool {
        matches!(
            self.path.last(),
            Some(OwnedValue::Int(_) | OwnedValue::Float(_))
        )
    }

    fn last_is_string(&self) -> bool {
        matches!(self.path.last(), Some(OwnedValue::String(_)))
    }

    /// jq's `stream_token`.
    fn stream_token(&mut self, ch: u8) -> Result<(), &'static str> {
        match ch {
            b'[' => {
                if self.next.is_some() {
                    return Err("Expected a separator between values");
                }
                if self.last_seen == LastSeen::OpenObject {
                    return Err("Expected string key after '{', not '['");
                }
                if self.last_seen == LastSeen::Comma && !self.last_is_number() {
                    return Err("Expected string key after ',' in object, not '['");
                }
                self.path.push(OwnedValue::Int(0));
                self.last_seen = LastSeen::OpenArray;
            }
            b'{' => {
                if self.last_seen == LastSeen::Value {
                    return Err("Expected a separator between values");
                }
                if self.last_seen == LastSeen::OpenObject {
                    return Err("Expected string key after '{', not '{'");
                }
                if self.last_seen == LastSeen::Comma && !self.last_is_number() {
                    return Err("Expected string key after ',' in object, not '{'");
                }
                self.path.push(OwnedValue::Null);
                self.last_seen = LastSeen::OpenObject;
            }
            b':' => {
                if self.path.is_empty() || self.last_is_number() {
                    return Err("':' not as part of an object");
                }
                if self.next.is_none() || self.last_seen == LastSeen::None {
                    return Err("Expected string key before ':'");
                }
                if !matches!(self.next, Some(OwnedValue::String(_))) {
                    return Err("Object keys must be strings");
                }
                if self.last_seen != LastSeen::Value {
                    return Err("':' should follow a key");
                }
                self.last_seen = LastSeen::Colon;
                if let (Some(key), Some(slot)) = (self.next.take(), self.path.last_mut()) {
                    *slot = key;
                }
            }
            b',' => {
                if self.last_seen != LastSeen::Value {
                    return Err("Expected value before ','");
                }
                if self.path.is_empty() {
                    return Err("',' not as part of an object or array");
                }
                let after = match self.path.last() {
                    Some(OwnedValue::Int(index)) => OwnedValue::Int(index + 1),
                    Some(OwnedValue::String(_)) => OwnedValue::Null,
                    // this case hits on input like `{,}`, and on `{"a", "b"}` and `{"a":, ...}`
                    _ => return Err("Objects must consist of key:value pairs"),
                };
                if let Some(value) = self.next.take() {
                    self.output = vec![self.path_value(), value];
                }
                if let Some(slot) = self.path.last_mut() {
                    *slot = after;
                }
                self.last_seen = LastSeen::Comma;
            }
            b']' => {
                if self.path.is_empty() {
                    return Err("Unmatched ']' at the top-level");
                }
                if self.last_seen == LastSeen::Comma {
                    return Err("Expected another array element");
                }
                if !self.last_is_number() {
                    return Err("Unmatched ']' in the middle of an object");
                }
                if let Some(value) = self.next.take() {
                    self.output = vec![self.path_value(), value, OwnedValue::Bool(true)];
                } else if self.last_seen != LastSeen::OpenArray {
                    self.output = vec![self.path_value()];
                }
                self.path.pop();
                self.next = None;
                if self.last_seen == LastSeen::OpenArray {
                    // Empty arrays are leaves.
                    self.output = vec![self.path_value(), OwnedValue::array_from(Vec::new())];
                }
                self.last_seen = if self.path.is_empty() {
                    LastSeen::None
                } else {
                    LastSeen::Value
                };
            }
            b'}' => {
                if self.path.is_empty() {
                    return Err("Unmatched '}' at the top-level");
                }
                if self.last_seen == LastSeen::Comma {
                    return Err("Expected another key:value pair");
                }
                if self.last_is_number() {
                    return Err("Unmatched '}' in the middle of an array");
                }
                if let Some(value) = self.next.take() {
                    if !self.last_is_string() {
                        return Err("Objects must consist of key:value pairs");
                    }
                    self.output = vec![self.path_value(), value, OwnedValue::Bool(true)];
                } else {
                    // Perhaps `{"a":[]}`.
                    match self.last_seen {
                        // Looks like `{"a":}`.
                        LastSeen::Colon => return Err("Missing value in key:value pair"),
                        // Looks like `{"a":0,}`.
                        LastSeen::Comma => return Err("Expected another key-value pair"),
                        LastSeen::OpenArray => {
                            return Err("Unmatched '}' in the middle of an array")
                        }
                        LastSeen::Value | LastSeen::OpenObject => {}
                        LastSeen::None => return Err("Unmatched '}'"),
                    }
                    if self.last_seen != LastSeen::OpenObject {
                        self.output = vec![self.path_value()];
                    }
                }
                self.path.pop();
                self.next = None;
                if self.last_seen == LastSeen::OpenObject {
                    // Empty objects are leaves.
                    self.output = vec![self.path_value(), OwnedValue::object()];
                }
                self.last_seen = if self.path.is_empty() {
                    LastSeen::None
                } else {
                    LastSeen::Value
                };
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
        let pattern: Option<&[u8]> = match self.token_buf[0] {
            b't' => Some(b"true"),
            b'f' => Some(b"false"),
            // Deliberately the whole buffer, not just the token: see the field's docs.
            b'n' if self.token_buf.get(1) == Some(&b'u') => Some(b"null"),
            _ => None,
        };
        let value = match pattern {
            Some(pattern) => {
                if &self.token_buf[..self.token_len] != pattern {
                    return Err("Invalid literal");
                }
                match pattern[0] {
                    b't' => OwnedValue::Bool(true),
                    b'f' => OwnedValue::Bool(false),
                    _ => OwnedValue::Null,
                }
            }
            None => {
                // jq NUL-terminates the token in place before its number parser reads it, which
                // scrubs the stale byte read above.
                if self.token_len == self.token_buf.len() {
                    self.token_buf.resize(self.token_buf.len() * 2 + 256, 0);
                }
                self.token_buf[self.token_len] = 0;
                // ... which also means the number parser reads the token as a C string: a NUL in
                // it ends the number (`2.5` then NUL then `{}` is the number `2.5`).
                let token = &self.token_buf[..self.token_len];
                let token = &token[..token.iter().position(|&b| b == 0).unwrap_or(token.len())];
                if !number_is_valid(token) {
                    return Err("Invalid numeric literal");
                }
                materialize(token)
            }
        };
        self.value(value)?;
        self.token_len = 0;
        Ok(())
    }

    /// jq's `found_string`.
    fn found_string(&mut self) -> Result<(), &'static str> {
        let buf = &self.token_buf[..self.token_len];
        let mut i = 0;
        while i < buf.len() {
            let c = buf[i];
            i += 1;
            if c != b'\\' {
                // jq compares a *signed* char, so only U+0000..=U+001F qualifies.
                if c <= 0x1f {
                    return Err(
                        "Invalid string: control characters from U+0000 through U+001F must be escaped",
                    );
                }
                continue;
            }
            // Unreachable: a trailing backslash leaves the scanner in `StrEscape`, so the next byte
            // is consumed as the escaped one. jq carries the arm anyway.
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
        let mut quoted = Vec::with_capacity(self.token_len + 2);
        quoted.push(b'"');
        quoted.extend_from_slice(&self.token_buf[..self.token_len]);
        quoted.push(b'"');
        self.value(materialize(&quoted))?;
        self.token_len = 0;
        Ok(())
    }
}

/// The value a token spells, built by the checked document parser so that it reads exactly as the
/// same text would in a document: a number keeps its literal, a string its escapes decoded, and an
/// invalid byte becomes U+FFFD the way jq's `jv_string_sized` makes it.
fn materialize(token: &[u8]) -> OwnedValue {
    let substituted;
    let bytes = if succinctly::text::utf8::validate_utf8(token).is_ok() {
        token
    } else {
        substituted = succinctly::jq::utf8_document::substitute_invalid_utf8_jq_document(token);
        substituted.as_bytes()
    };
    crate::jq_runner::json_bytes_to_owned_value_checked(bytes).unwrap_or(OwnedValue::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_one(input: &[u8]) -> StreamErrorsRead {
        read(&[(None, input.to_vec())])
    }

    fn run(input: &str) -> Vec<String> {
        read_one(input.as_bytes())
            .values
            .iter()
            .map(OwnedValue::to_json)
            .collect()
    }

    #[test]
    fn a_truncated_array_reports_the_events_before_it_and_the_path() {
        assert_eq!(
            run("[1,"),
            [
                "[[0],1]",
                r#"["Unfinished JSON term at EOF at line 1, column 3",[1]]"#
            ]
        );
    }

    #[test]
    fn an_object_key_not_yet_read_is_null_in_the_path() {
        assert_eq!(
            run("{"),
            [r#"["Unfinished JSON term at EOF at line 1, column 1",[null]]"#]
        );
    }

    #[test]
    fn a_nul_ends_a_number_token_but_not_a_keyword_5474() {
        // The number parser reads the token as a C string.
        assert_eq!(run("2.5\u{0}{}\n"), ["[[],2.5]", "[[],{}]"]);
        // `tru` + NUL is five bytes where `true` is four.
        assert_eq!(
            run("tru\u{0}\n"),
            [r#"["Invalid literal at line 2, column 0",[]]"#]
        );
    }

    #[test]
    fn a_chunk_with_no_newline_stops_at_its_first_nul() {
        assert_eq!(run("1 \u{0}2 3"), ["[[],1]"]);
        // ... but a chunk that ends in a newline is scanned whole, the NUL included.
        assert_eq!(
            run("1 \u{0}2 3\n"),
            [
                "[[],1]",
                r#"["Invalid numeric literal at line 1, column 5",[]]"#
            ]
        );
    }

    #[test]
    fn the_slurp_array_closes_at_the_call_that_empties_the_buffer() {
        // No final newline: the read ends on the `]` that yields the leaf, so the close is outside.
        let read = read_one(b"[1,2]");
        assert_eq!((read.values.len(), read.inside), (3, 2));
        // A final newline: the close is handed out by a call before the read that hits the end.
        let read = read_one(b"[1,2]\n");
        assert_eq!((read.values.len(), read.inside), (3, 3));
    }

    #[test]
    fn a_value_yielded_after_the_input_is_closed_has_no_location() {
        let read = read_one(b"[1,2]");
        let lines: Vec<usize> = read.locations.iter().map(|&(_, line)| line).collect();
        assert_eq!(lines, [0, 0, CLOSED]);
        assert_eq!(read.open_at_end, None);
        // A newline-terminated end leaves the input open for the call that finds the end.
        assert_eq!(read_one(b"[1,2]\n").open_at_end, Some(1));
    }

    #[test]
    fn a_token_continues_into_the_next_source() {
        let read = read(&[(Some(0), b"[1".to_vec()), (Some(1), b"2]\n".to_vec())]);
        let events: Vec<String> = read.values.iter().map(OwnedValue::to_json).collect();
        assert_eq!(events, ["[[0],12]", "[[0]]"]);
    }

    /// Each row's output was captured from `jq --stream-errors -c .` (jq 1.7.1) and covers one of
    /// the parser's error arms or an event shape.
    #[test]
    #[allow(clippy::literal_string_with_formatting_args)]
    fn every_error_arm_and_event_shape_matches_jq_5474() {
        let rows: &[(&str, &str)] = &[
        ("[1 2]", "[\"Expected separator between values at line 1, column 5\",[0]]\n"),
        ("{[\"a\"]}", "[\"Expected string key after '{', not '[' at line 1, column 2\",[null]]\n"),
        ("{\"x\":\"y\",[\"foo\"]}", "[[\"x\"],\"y\"]\n[\"Expected string key after ',' in object, not '[' at line 1, column 10\",[null]]\n"),
        ("{{\"a\":1}}", "[\"Expected string key after '{', not '{' at line 1, column 2\",[null]]\n"),
        ("{\"x\":\"y\",{\"foo\":\"bar\"}}", "[[\"x\"],\"y\"]\n[\"Expected string key after ',' in object, not '{' at line 1, column 10\",[null]]\n"),
        (":", "[\"':' not as part of an object at line 1, column 1\",[]]\n"),
        ("[1:2]", "[\"':' not as part of an object at line 1, column 3\",[0]]\n"),
        ("{\"a\":1:2}", "[\"Object keys must be strings at line 1, column 7\",[\"a\"]]\n"),
        ("{1:2}", "[\"Object keys must be strings at line 1, column 3\",[null]]\n"),
        ("{\"a\" \"b\"}", "[\"Expected separator between values at line 1, column 8\",[null]]\n"),
        ("1:2", "[\"':' not as part of an object at line 1, column 2\",[]]\n"),
        ("{,}", "[\"Expected value before ',' at line 1, column 2\",[null]]\n"),
        ("{\"a\",1}", "[\"Objects must consist of key:value pairs at line 1, column 5\",[null]]\n"),
        ("[,]", "[\"Expected value before ',' at line 1, column 2\",[0]]\n"),
        (",", "[\"Expected value before ',' at line 1, column 1\",[]]\n"),
        ("[1,,2]", "[[0],1]\n[\"Expected value before ',' at line 1, column 4\",[1]]\n"),
        ("]", "[\"Unmatched ']' at the top-level at line 1, column 1\",[]]\n"),
        ("[1,2,]", "[[0],1]\n[[1],2]\n[\"Expected another array element at line 1, column 6\",[2]]\n"),
        ("{\"a\":[1]]}", "[[\"a\",0],1]\n[[\"a\",0]]\n[\"Unmatched ']' in the middle of an object at line 1, column 9\",[\"a\"]]\n"),
        ("}", "[\"Unmatched '}' at the top-level at line 1, column 1\",[]]\n"),
        ("{\"a\":1,}", "[[\"a\"],1]\n[\"Expected another key:value pair at line 1, column 8\",[null]]\n"),
        ("{\"a\":}", "[\"Missing value in key:value pair at line 1, column 6\",[\"a\"]]\n"),
        ("{\"a\":1]", "[\"Unmatched ']' in the middle of an object at line 1, column 7\",[\"a\"]]\n"),
        ("[}", "[\"Unmatched '}' in the middle of an array at line 1, column 2\",[0]]\n"),
        ("[1}", "[\"Unmatched '}' in the middle of an array at line 1, column 3\",[0]]\n"),
        ("{1}", "[\"Objects must consist of key:value pairs at line 1, column 3\",[null]]\n"),
        ("{\"a\"}", "[\"Objects must consist of key:value pairs at line 1, column 5\",[null]]\n"),
        ("[1,2] }", "[[0],1]\n[[1],2]\n[[1]]\n[\"Unmatched '}' at the top-level at line 1, column 7\",[]]\n"),
        ("\"\\x\"", "[\"Invalid escape at line 1, column 4\",[]]\n"),
        ("\"\\u12\"", "[\"Invalid \\\\uXXXX escape at line 1, column 6\",[]]\n"),
        ("\"\\u12zz\"", "[\"Invalid characters in \\\\uXXXX escape at line 1, column 8\",[]]\n"),
        ("\"\\ud800x\"", "[\"Invalid \\\\uXXXX\\\\uXXXX surrogate pair escape at line 1, column 9\",[]]\n"),
        ("\"\\ud800\\u0041\"", "[\"Invalid \\\\uXXXX\\\\uXXXX surrogate pair escape at line 1, column 14\",[]]\n"),
        ("\"a\tb\"", "[\"Invalid string: control characters from U+0000 through U+001F must be escaped at line 1, column 5\",[]]\n"),
        ("tru", "[\"Invalid literal at EOF at line 1, column 3\",[]]\n"),
        ("nul", "[\"Invalid literal at EOF at line 1, column 3\",[]]\n"),
        ("1e", "[\"Invalid numeric literal at EOF at line 1, column 2\",[]]\n"),
        ("-", "[\"Invalid numeric literal at EOF at line 1, column 1\",[]]\n"),
        ("\"abc", "[\"Unfinished string at EOF at line 1, column 4\",[]]\n"),
        ("[1,2", "[[0],1]\n[\"Unfinished JSON term at EOF at line 1, column 4\",[1]]\n"),
        ("[[]]", "[[0],[]]\n[[0]]\n"),
        ("{\"a\":{}}", "[[\"a\"],{}]\n[[\"a\"]]\n"),
        ("[[1],[2]]", "[[0,0],1]\n[[0,0]]\n[[1,0],2]\n[[1,0]]\n[[1]]\n"),
        ("{\"a\":[]}", "[[\"a\"],[]]\n[[\"a\"]]\n"),
        ("{\"a\":{\"b\":[]}}", "[[\"a\",\"b\"],[]]\n[[\"a\",\"b\"]]\n[[\"a\"]]\n"),
        ("[{}]", "[[0],{}]\n[[0]]\n"),
        ("{\"a\":1,\"b\":[2,3]}", "[[\"a\"],1]\n[[\"b\",0],2]\n[[\"b\",1],3]\n[[\"b\",1]]\n[[\"b\"]]\n"),
        ("\"\\ud83d\\ude00\"", "[[],\"😀\"]\n"),
        ];
        for (input, want) in rows {
            let got: String = run(input).iter().map(|line| format!("{line}\n")).collect();
            assert_eq!(got, *want, "{input:?}");
        }
    }

    #[test]
    fn an_empty_source_is_a_refill_that_hands_out_what_the_last_byte_left() {
        // `[1]` leaves its close pending after the last byte; the empty source after it is the
        // call that hands it out, at that source's own position.
        let read = read(&[(Some(0), b"[1]".to_vec()), (Some(1), Vec::new())]);
        let events: Vec<String> = read.values.iter().map(OwnedValue::to_json).collect();
        assert_eq!(events, ["[[0],1]", "[[0]]"]);
        assert_eq!(read.locations, [(0, 0), (1, 0)]);
        assert_eq!(read.inside, 2);
        // An empty input has no value, and nothing to slurp.
        assert_eq!(read_one(b"").values.len(), 0);
    }
}
