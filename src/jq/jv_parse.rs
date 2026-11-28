//! jq 1.7.1's JSON text parser, the non-streaming path of `jv_parse.c`, driven a byte at a time:
//! the parser half of ADR-0033's reader model (#7987, step 3; decision 4).
//!
//! It is a port of `parser_init`, `parser_reset`, `jv_parser_set_buf` (the byte-order-mark
//! matcher), `scan`, `parse_token`, `check_literal`, `found_string`, `parse_check_done`,
//! `jv_parser_next` (the fault path and the end-of-input checks) and `jv_parse_sized`, from the
//! vendored `jq-reader-oracle/vendor/jq-1.7.1/src/jv_parse.c`, `--seq`'s record separator included
//! (`JV_PARSE_SEQ`). It builds no values. It hands back the span of each top-level value's text in
//! the stream it was fed, for the crate's own value reader to decode, and each fault as jq words and
//! places it.
//!
//! Where it departs from that source, each departure is a recorded rule of ADR-0018's:
//!
//! - **A NUL outside a string is `Invalid character` at its own column (E2, #7966).** `classify`
//!   answers `INVALID` for it, as jq master's does (commit fbee8f4, jqlang/jq#3606); jq 1.7.1 read it
//!   into the bare token. A NUL inside a string is still the control-character error.
//! - **Numbers are the crate's one number grammar** (`parse_message::is_number_token`), whose NaN
//!   rule is `json::validate::jq_special_number`'s (#7522, ADR-0033 decision 2): a NaN with a
//!   payload is refused, as in jq 1.8.
//! - **A bare `n` never reads a stale `tokenbuf[1]` (#7525).** jq 1.7.1's `check_literal` reads
//!   `tokenbuf[1]` whatever `tokenpos` is; the check here is jq 1.8.2's `tokenpos > 1`.
//! - **`last_ch_was_ws` is kept under `--seq` only**, the only mode that reads it
//!   (`seq_check_truncation` and the end-of-input check are both behind `JV_PARSE_SEQ`).
//!
//! It is `no_std` with `alloc`, so that `fromjson`, `--argjson` and the CLI's reader model can share
//! it (ADR-0033 decision 4). Since step 5 the CLI's reader model runs it on the materializing route
//! of `read_inputs`, and since step 6 on the default route, entered on each route's triggers.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::parse_message::{is_number_token, string_token_fault};

/// The UTF-8 byte-order mark `jv_parser_set_buf` skips at the front of the stream.
pub(crate) const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

/// `bom_strip_position` once the mark has been matched whole, or the first byte was no mark.
pub(crate) const BOM_DONE: u8 = 3;

/// `bom_strip_position` once a prefix of the mark was followed by another byte: every
/// `jv_parser_next` from then on is `Malformed BOM` (or, under `--seq`, a reset).
pub(crate) const BOM_MALFORMED: u8 = 0xff;

/// ASCII RS, the record separator of `--seq` (RFC 7464).
const ASCII_RS: u8 = 0x1e;

/// An entry of `p->stack`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Frame {
    /// `jv_array()`, and whether `jv_array_length` is non-zero.
    Array { has_element: bool },
    /// `jv_object()`, and whether `jv_object_length` is non-zero.
    Object { has_pair: bool },
    /// A key pushed by `:` and waiting for its value (a `JV_KIND_STRING` entry).
    Key,
}

/// The kind of the value in `p->next`, as far as the parser reads it: `:` asks for a string, and
/// `--seq`'s truncation checks for a number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    String,
    Number,
    Other,
}

/// `p->st`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum St {
    Normal,
    String,
    StringEscape,
    /// `JV_PARSER_WAITING_FOR_RS`: a `--seq` parser that waits for the next RS.
    WaitingForRs,
}

/// What `jv_parser_next` leaves of the parser at a sync point: between top-level values, with no
/// token, no container stack and no pending value (ADR-0033, "Sync points and state"). There a
/// parser is these few numbers, and [`JvParser::resume`] rebuilds it from them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ParserSync {
    /// `p->line`, 1-based.
    pub line: usize,
    /// `p->column`: bytes read on the line so far.
    pub column: usize,
    /// `p->bom_strip_position`: `0..=2` bytes of the mark matched, [`BOM_DONE`] or
    /// [`BOM_MALFORMED`].
    pub bom: u8,
    /// `p->st == JV_PARSER_WAITING_FOR_RS` (`--seq` only).
    pub waiting_for_rs: bool,
    /// `p->last_ch_was_ws` (`--seq` only).
    pub last_ch_was_ws: bool,
    /// `p->eof`: the parser has been given its last buffer and read it through.
    pub eof: bool,
}

impl ParserSync {
    /// `parser_init`: line 1, column 0, the mark not yet matched, and under `--seq` waiting for
    /// the first RS.
    pub(crate) const fn start(seq: bool) -> Self {
        Self {
            line: 1,
            column: 0,
            bom: 0,
            waiting_for_rs: seq,
            last_ch_was_ws: false,
            eof: false,
        }
    }
}

/// The bytes of one top-level value's text, as offsets in the stream the parser was fed (the
/// stripped byte-order mark and every thrown-away buffer counted).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    /// The value's first byte.
    pub start: usize,
    /// Just past its last byte.
    pub end: usize,
}

/// How a [`Fault`]'s message names its place, after `make_error`'s formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FaultPlace {
    /// `%s at line %d, column %d`.
    Byte,
    /// `--seq`: `%s at line %d, column %d (need RS to resync)`.
    Resync,
    /// `%s at EOF at line %d, column %d`.
    Eof,
    /// `Malformed BOM`, with no position.
    Bom,
}

/// A parse error, as `jv_parser_next` reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Fault {
    /// jq's words for it (`Unmatched ']'`), without the position.
    pub what: &'static str,
    /// How the message names the position.
    pub place: FaultPlace,
    /// `p->line` when it was raised.
    pub line: usize,
    /// `p->column` when it was raised.
    pub column: usize,
    /// The stream offset of the byte it was raised on; the end of the last buffer for a fault at
    /// the end of input, and the byte that broke the mark for `Malformed BOM`.
    pub offset: usize,
}

impl Fault {
    /// The message jq prints (`jq: parse error: <message>`) and `input` raises.
    pub(crate) fn message(&self) -> String {
        let Self {
            what, line, column, ..
        } = *self;
        match self.place {
            FaultPlace::Byte => format!("{what} at line {line}, column {column}"),
            FaultPlace::Resync => {
                format!("{what} at line {line}, column {column} (need RS to resync)")
            }
            FaultPlace::Eof => format!("{what} at EOF at line {line}, column {column}"),
            FaultPlace::Bom => String::from(what),
        }
    }
}

/// What one `jv_parser_next` call returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Next {
    /// A top-level value.
    Value(Span),
    /// A parse error (`jv_invalid_with_msg`).
    Fault(Fault),
    /// A plain `jv_invalid()`: the buffer is read and more is needed, the input is over, or
    /// (`--seq`) an RS ended an empty record with bytes still in the buffer.
    None,
}

/// What `scan` returns when it raises nothing: go on, or hand back (`OK`), with or without a
/// value.
enum Scanned {
    More,
    Done(Option<Span>),
}

/// `struct jv_parser`, without `JV_PARSE_STREAMING`.
pub(crate) struct JvParser<'a> {
    /// `JV_PARSE_SEQ`.
    seq: bool,
    /// `MAX_PARSING_DEPTH`, from the caller: the reader model reads it from its profile.
    max_depth: usize,
    /// `curr_buf` (`None` is the C `0`), `curr_buf_pos` and `curr_buf_is_partial`.
    buf: Option<&'a [u8]>,
    pos: usize,
    partial: bool,
    /// The stream offset of `buf[0]`.
    buf_base: usize,
    /// The stream offset just past the last buffer given to [`set_buf`](Self::set_buf).
    fed: usize,
    eof: bool,
    bom: u8,
    stack: Vec<Frame>,
    next: Option<Kind>,
    /// `tokenbuf[..tokenpos]`.
    token: Vec<u8>,
    line: usize,
    column: usize,
    st: St,
    last_ch_was_ws: bool,
    /// The stream offset of the first byte of the top-level value in progress.
    value_start: usize,
    /// The stream offset just past the last byte that belonged to a token.
    token_end: usize,
}

impl<'a> JvParser<'a> {
    /// `jv_parser_new`, at the front of a stream.
    pub(crate) fn new(seq: bool, max_depth: usize) -> Self {
        Self::resume(ParserSync::start(seq), seq, max_depth, 0)
    }

    /// A parser at a sync point: what `sync` saved, with the stream offset of the next byte it
    /// will be fed.
    pub(crate) fn resume(sync: ParserSync, seq: bool, max_depth: usize, offset: usize) -> Self {
        let ParserSync {
            line,
            column,
            bom,
            waiting_for_rs,
            last_ch_was_ws,
            eof,
        } = sync;
        Self {
            seq,
            max_depth,
            buf: None,
            pos: 0,
            partial: false,
            buf_base: offset,
            fed: offset,
            eof,
            bom,
            stack: Vec::new(),
            next: None,
            token: Vec::new(),
            line,
            column,
            st: if waiting_for_rs {
                St::WaitingForRs
            } else {
                St::Normal
            },
            last_ch_was_ws,
            value_start: offset,
            token_end: offset,
        }
    }

    /// The parser as [`ParserSync`], when it is between top-level values (no token, no stack, no
    /// pending value, outside a string); `None` otherwise. Whether its buffer is read through is
    /// the caller's to check ([`remaining`](Self::remaining)).
    pub(crate) fn sync(&self) -> Option<ParserSync> {
        let between = self.stack.is_empty()
            && self.token.is_empty()
            && self.next.is_none()
            && matches!(self.st, St::Normal | St::WaitingForRs);
        between.then_some(ParserSync {
            line: self.line,
            column: self.column,
            bom: self.bom,
            waiting_for_rs: self.st == St::WaitingForRs,
            last_ch_was_ws: self.last_ch_was_ws,
            eof: self.eof,
        })
    }

    /// `jv_parser_remaining`.
    pub(crate) fn remaining(&self) -> usize {
        self.buf.map_or(0, |buf| buf.len() - self.pos)
    }

    /// Whether the mark matcher is in its `Malformed BOM` state, which it never leaves.
    pub(crate) fn malformed_bom(&self) -> bool {
        self.bom == BOM_MALFORMED
    }

    /// The parser with its buffer let go, once it is read through: between two buffers it holds
    /// none of them, so the next can come from a source the last did not (#7987, step 7: a reader
    /// fed the windows a route holds, one read at a time, where its whole input is not in memory).
    /// Everything else, a value in progress included, goes on as it was. In C the buffer is the
    /// caller's too, and `jv_parser_set_buf` only ever replaces it once it is read through. A
    /// parser stuck on `Malformed BOM` reads nothing more of its buffer, so it lets go of it too.
    pub(crate) fn rebind<'b>(self) -> JvParser<'b> {
        debug_assert!(
            self.remaining() == 0 || self.malformed_bom(),
            "the buffer is read through"
        );
        JvParser {
            seq: self.seq,
            max_depth: self.max_depth,
            buf: None,
            pos: 0,
            partial: self.partial,
            buf_base: self.buf_base,
            fed: self.fed,
            eof: self.eof,
            bom: self.bom,
            stack: self.stack,
            next: self.next,
            token: self.token,
            line: self.line,
            column: self.column,
            st: self.st,
            last_ch_was_ws: self.last_ch_was_ws,
            value_start: self.value_start,
            token_end: self.token_end,
        }
    }

    /// `jv_parser_set_buf`: the next buffer of the stream. While the mark is not decided its bytes
    /// are matched one at a time across buffers, and a matched byte is skipped without being
    /// counted in `column`.
    pub(crate) fn set_buf(&mut self, buf: &'a [u8], is_partial: bool) {
        debug_assert!(self.remaining() == 0, "previous buffer not exhausted");
        let base = self.fed;
        self.fed += buf.len();
        let mut buf = buf;
        while !buf.is_empty() && usize::from(self.bom) < UTF8_BOM.len() {
            if buf[0] == UTF8_BOM[usize::from(self.bom)] {
                // matched a BOM character
                buf = &buf[1..];
                self.bom += 1;
            } else if self.bom == 0 {
                // no BOM in this document
                self.bom = BOM_DONE;
            } else {
                // malformed BOM (prefix present, rest missing)
                self.bom = BOM_MALFORMED;
            }
        }
        self.buf_base = self.fed - buf.len();
        debug_assert!(self.buf_base >= base);
        self.buf = Some(buf);
        self.pos = 0;
        self.partial = is_partial;
    }

    /// `parser_reset`: `line`, `column`, the buffer, `eof`, the mark and `last_ch_was_ws` are
    /// untouched.
    fn reset(&mut self) {
        self.next = None;
        self.stack.clear();
        self.token.clear();
        self.st = St::Normal;
    }

    /// `value`: a finished value goes in `p->next`, which must be empty.
    fn value(&mut self, kind: Kind) -> Result<(), &'static str> {
        if self.next.is_some() {
            return Err("Expected separator between values");
        }
        self.next = Some(kind);
        Ok(())
    }

    /// `parse_token`.
    fn parse_token(&mut self, ch: u8) -> Result<(), &'static str> {
        match ch {
            b'[' | b'{' => {
                if self.stack.len() >= self.max_depth {
                    return Err("Exceeds depth limit for parsing");
                }
                if self.next.is_some() {
                    return Err("Expected separator between values");
                }
                self.stack.push(if ch == b'[' {
                    Frame::Array { has_element: false }
                } else {
                    Frame::Object { has_pair: false }
                });
            }
            b':' => {
                let Some(next) = self.next else {
                    return Err("Expected string key before ':'");
                };
                if !matches!(self.stack.last(), Some(Frame::Object { .. })) {
                    return Err("':' not as part of an object");
                }
                if next != Kind::String {
                    return Err("Object keys must be strings");
                }
                self.stack.push(Frame::Key);
                self.next = None;
            }
            b',' => {
                if self.next.is_none() {
                    return Err("Expected value before ','");
                }
                match self.stack.last_mut() {
                    // `check_done` empties `p->next` at the top level, so this is jq's guard only.
                    None => return Err("',' not as part of an object or array"),
                    Some(Frame::Array { has_element }) => *has_element = true,
                    Some(Frame::Key) => {
                        self.stack.pop();
                        if let Some(Frame::Object { has_pair }) = self.stack.last_mut() {
                            *has_pair = true;
                        }
                    }
                    // this case hits on input like {"a", "b"}
                    Some(Frame::Object { .. }) => {
                        return Err("Objects must consist of key:value pairs");
                    }
                }
                self.next = None;
            }
            b']' => {
                let Some(&Frame::Array { has_element }) = self.stack.last() else {
                    return Err("Unmatched ']'");
                };
                if self.next.is_none() && has_element {
                    // this case hits on input like [1,2,3,]
                    return Err("Expected another array element");
                }
                self.stack.pop();
                self.next = Some(Kind::Other);
            }
            _ => {
                // `}`
                let Some(&top) = self.stack.last() else {
                    return Err("Unmatched '}'");
                };
                if self.next.is_some() {
                    if top != Frame::Key {
                        return Err("Objects must consist of key:value pairs");
                    }
                    self.stack.pop();
                } else {
                    match top {
                        Frame::Object { has_pair: true } => {
                            return Err("Expected another key-value pair");
                        }
                        Frame::Object { has_pair: false } => {}
                        _ => return Err("Unmatched '}'"),
                    }
                }
                self.stack.pop();
                self.next = Some(Kind::Other);
            }
        }
        Ok(())
    }

    /// `found_string`: the finished string token's escapes and control bytes, by the crate's one
    /// definition of them ([`string_token_fault`]), then `value`.
    fn found_string(&mut self) -> Result<(), &'static str> {
        if let Some(fault) = string_token_fault(&self.token) {
            return Err(fault);
        }
        self.value(Kind::String)?;
        self.token.clear();
        Ok(())
    }

    /// `check_literal`: `true`, `false` and `null` are compared whole; any other token is a
    /// number to decNumber's grammar ([`is_number_token`]).
    fn check_literal(&mut self) -> Result<(), &'static str> {
        if self.token.is_empty() {
            return Ok(());
        }
        let pattern: Option<&[u8]> = match self.token[0] {
            b't' => Some(b"true"),
            b'f' => Some(b"false"),
            // if it starts with 'n', it could be a literal "nan". jq 1.7.1 reads `tokenbuf[1]`
            // without checking `tokenpos`; 1.8.2's check is kept (#7525).
            b'n' if self.token.len() > 1 && self.token[1] == b'u' => Some(b"null"),
            _ => None,
        };
        let kind = match pattern {
            Some(pattern) if self.token != pattern => return Err("Invalid literal"),
            Some(_) => Kind::Other,
            None if is_number_token(&self.token) => Kind::Number,
            None => return Err("Invalid numeric literal"),
        };
        self.value(kind)?;
        self.token.clear();
        Ok(())
    }

    /// `parse_check_done`: the finished top-level value, moved out of `p->next`.
    fn check_done(&mut self) -> Option<Span> {
        if !self.stack.is_empty() || self.next.is_none() {
            return None;
        }
        self.next = None;
        Some(Span {
            start: self.value_start,
            end: self.token_end,
        })
    }

    /// `seq_check_truncation`.
    fn check_truncation(&self) -> bool {
        !self.last_ch_was_ws
            && (!self.stack.is_empty() || !self.token.is_empty() || self.next == Some(Kind::Number))
    }

    /// Marks the first byte of a top-level value.
    fn begin_value(&mut self, at: usize) {
        if self.stack.is_empty() && self.token.is_empty() {
            self.value_start = at;
        }
    }

    /// `scan`: one byte, at stream offset `at`.
    fn scan(&mut self, ch: u8, at: usize) -> Result<Scanned, &'static str> {
        self.column += 1;
        if ch == b'\n' {
            self.line += 1;
            self.column = 0;
        }
        if self.seq && ch == ASCII_RS {
            if self.check_truncation() {
                if self.check_literal().is_ok()
                    && self.stack.is_empty()
                    && self.next == Some(Kind::Number)
                {
                    return Err("Potentially truncated top-level numeric value");
                }
                return Err("Truncated value");
            }
            self.check_literal()?;
            if self.st == St::Normal {
                if let Some(done) = self.check_done() {
                    return Ok(Scanned::Done(Some(done)));
                }
            }
            // shouldn't happen?
            self.reset();
            return Ok(Scanned::Done(None));
        }
        let mut answer = None;
        if self.seq {
            self.last_ch_was_ws = false;
        }
        if self.st == St::Normal {
            let whitespace = matches!(ch, b' ' | b'\t' | b'\r' | b'\n');
            let structure = matches!(ch, b'[' | b',' | b']' | b'{' | b':' | b'}');
            let quote = ch == b'"';
            // E2 (#7966): `classify` answers `INVALID` for a NUL, as jq master's does.
            let invalid = ch == 0;
            if whitespace && self.seq {
                self.last_ch_was_ws = true;
            }
            if whitespace || structure || quote || invalid {
                self.check_literal()?;
                // A value moved out here is lost if the token below raises: `1}` is only the
                // fault, `1 }` is `1` and then the fault.
                if let Some(done) = self.check_done() {
                    answer = Some(done);
                }
            }
            if invalid {
                return Err(super::parse_message::INVALID_CHARACTER);
            }
            if structure {
                if matches!(ch, b'[' | b'{') {
                    self.begin_value(at);
                }
                self.parse_token(ch)?;
                self.token_end = at + 1;
            } else if quote {
                self.begin_value(at);
                self.st = St::String;
                self.token_end = at + 1;
            } else if !whitespace {
                // `LITERAL`
                self.begin_value(at);
                self.token.push(ch);
                self.token_end = at + 1;
            }
            if let Some(done) = self.check_done() {
                answer = Some(done);
            }
        } else if ch == b'"' && self.st == St::String {
            self.found_string()?;
            self.st = St::Normal;
            self.token_end = at + 1;
            if let Some(done) = self.check_done() {
                answer = Some(done);
            }
        } else {
            self.token.push(ch);
            self.st = if ch == b'\\' && self.st == St::String {
                St::StringEscape
            } else {
                St::String
            };
        }
        Ok(answer.map_or(Scanned::More, |done| Scanned::Done(Some(done))))
    }

    /// A fault raised at the parser's position now.
    fn fault(&self, what: &'static str, place: FaultPlace, offset: usize) -> Fault {
        Fault {
            what,
            place,
            line: self.line,
            column: self.column,
            offset,
        }
    }

    /// `jv_parser_next`.
    pub(crate) fn next(&mut self) -> Next {
        if self.eof {
            return Next::None;
        }
        let Some(buf) = self.buf else {
            return Next::None; // Need a buffer
        };
        if self.bom == BOM_MALFORMED {
            if !self.seq {
                // Consumes nothing, so every later call answers this again.
                let offset = self.buf_base + self.pos;
                return Next::Fault(self.fault("Malformed BOM", FaultPlace::Bom, offset));
            }
            self.st = St::WaitingForRs;
            // `parser_reset` writes `JV_PARSER_NORMAL` straight back over the wait.
            self.reset();
        }
        let mut ch = 0u8;
        let mut at = 0usize;
        let mut msg: Option<Result<Option<Span>, &'static str>> = None;
        while msg.is_none() && self.pos < buf.len() {
            ch = buf[self.pos];
            at = self.buf_base + self.pos;
            self.pos += 1;
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
                continue; // need to resync, wait for RS
            }
            match self.scan(ch, at) {
                Ok(Scanned::More) => {}
                Ok(Scanned::Done(value)) => msg = Some(Ok(value)),
                Err(what) => msg = Some(Err(what)),
            }
        }
        match msg {
            Some(Ok(value)) => value.map_or(Next::None, Next::Value),
            Some(Err(what)) => {
                if ch != ASCII_RS && self.seq {
                    // Skip to the next RS
                    self.st = St::WaitingForRs;
                    let fault = self.fault(what, FaultPlace::Resync, at);
                    // `parser_reset` undoes the wait, as in both releases.
                    self.reset();
                    return Next::Fault(fault);
                }
                let fault = self.fault(what, FaultPlace::Byte, at);
                self.reset();
                if !self.seq {
                    // We're not parsing a JSON text sequence; throw this buffer away.
                    self.buf = None;
                    self.pos = 0;
                } // Else ch must be RS; don't clear buf so we can start parsing again after this ch
                Next::Fault(fault)
            }
            None if self.partial => Next::None, // need another buffer
            None => self.at_eof(),
        }
    }

    /// `jv_parser_next`'s end-of-input branch.
    fn at_eof(&mut self) -> Next {
        self.eof = true;
        let end = self.fed;
        let fault = |parser: &Self, what| Next::Fault(parser.fault(what, FaultPlace::Eof, end));
        if self.st == St::WaitingForRs {
            return fault(self, "Unfinished abandoned text");
        }
        if self.st != St::Normal {
            let answer = fault(self, "Unfinished string");
            self.reset();
            self.st = St::WaitingForRs;
            return answer;
        }
        if let Err(what) = self.check_literal() {
            let answer = fault(self, what);
            self.reset();
            self.st = St::WaitingForRs;
            return answer;
        }
        if !self.stack.is_empty() {
            let answer = fault(self, "Unfinished JSON term");
            self.reset();
            self.st = St::WaitingForRs;
            return answer;
        }
        // p->next is either invalid (nothing here, but no syntax error)
        // or valid (this is the value). either way it's the thing to return
        let kind = self.next.take();
        if self.seq && !self.last_ch_was_ws && kind == Some(Kind::Number) {
            return fault(self, "Potentially truncated top-level numeric value");
        }
        kind.map_or(Next::None, |_| {
            Next::Value(Span {
                start: self.value_start,
                end: self.token_end,
            })
        })
    }
}

/// `jv_parse_sized`'s verdict on `text` as one JSON text (`fromjson`'s): `None` when it is exactly
/// one value, else jq's message without the `(while parsing '...')` it appends.
pub(crate) fn jv_parse_sized_message(text: &[u8], max_depth: usize) -> Option<String> {
    let mut parser = JvParser::new(false, max_depth);
    parser.set_buf(text, false);
    match parser.next() {
        Next::Value(_) => match parser.next() {
            // multiple JSON values, we only wanted one
            Next::Value(_) => Some(String::from("Unexpected extra JSON values")),
            // parser error after the first JSON value
            Next::Fault(fault) => Some(fault.message()),
            // a single valid JSON value
            Next::None => None,
        },
        Next::Fault(fault) => Some(fault.message()),
        Next::None => Some(String::from("Expected JSON value")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jq::parse_message::jq_parse_message;
    use crate::jq::{jq_stream_fault, jq_stream_fault_offset, JQ_MAX_PARSING_DEPTH};

    /// Every value and the first fault of `text` read as one stream, from `(line, column)` with
    /// the mark already decided, in `chunk`-byte buffers.
    fn stream(text: &[u8], line: usize, column: usize, chunk: usize) -> (Vec<Span>, Option<Fault>) {
        let sync = ParserSync {
            line,
            column,
            bom: BOM_DONE,
            ..ParserSync::start(false)
        };
        let mut parser = JvParser::resume(sync, false, JQ_MAX_PARSING_DEPTH, 0);
        let pieces: Vec<&[u8]> = if text.is_empty() {
            vec![text]
        } else {
            text.chunks(chunk).collect()
        };
        let mut values = Vec::new();
        for (i, piece) in pieces.iter().enumerate() {
            parser.set_buf(piece, i + 1 < pieces.len());
            loop {
                match parser.next() {
                    Next::Value(span) => values.push(span),
                    Next::Fault(fault) => return (values, Some(fault)),
                    Next::None => break,
                }
            }
        }
        (values, None)
    }

    /// Every text [`jq_parse_message`]'s tests pin (`parse_message.rs`, #5309, #5804, #7966), read
    /// by both: the port and the imitation it is to replace word every fault alike.
    const ONE_TEXT_ROWS: &[&str] = &[
        "\u{fffd}",
        "\u{fffd}e",
        "\u{f000}",
        "\u{ffff}1",
        "\u{fec0}",
        "\u{e000}1",
        "1",
        "\u{feff}\u{feff}",
        "\0",
        "1\0",
        "12\u{0}3",
        "[1\0x,2]\n",
        "[1,\0 2]",
        "true\0",
        "{\"a\":1\0}",
        "{\"a\"\0:1}",
        "[1,\n2\0]",
        "1.5\0",
        "nan\0",
        "tr\0ue",
        "-\0",
        "[[1]1\0]",
        "1 2\0",
        "\"a\0b\"",
        "a,b",
        "a b",
        "a",
        "nul",
        "truee",
        "1 2",
        "true false",
        "[1,",
        "{",
        "[1,]",
        "{\"a\":1,}",
        "{\"a\" 1}",
        "[1 2]",
        "{\"a\":}",
        "}",
        "{]",
        "[1}",
        "1,2",
        "1:2",
        "\"abc",
        "\"a\\x\"",
        "a\n,b",
        "\u{feff}\u{feff}1",
        "",
        " ",
        " [1, {\"a\": \"b\"}] ",
        "null",
        "-Infinity",
        "nan",
        "\u{feff}[1]",
        "{}",
        ".5",
        "1.e5",
        "\"\\ud83d\\ude00\"",
        "[1[",
        "[1:",
        "{1:",
        "{\"a\",",
        "1 2[",
        "1e",
        "1e+",
        ".",
        "\"a\\n\\/\\u00e9\\ud83d\\ude00\" 1",
        "1e+5",
        "-1E-5",
        "\"\\ud800\"",
        "\"\\u12\"",
        "\"\\u12zz\"",
        "\"a\u{1}\"",
    ];

    /// The streams [`jq_stream_fault`]'s tests pin (#5555, #7966), as `(chunks, line, column)`.
    const STREAM_ROWS: &[(&[&str], usize, usize)] = &[
        (&[], 1, 0),
        (&[" \n"], 1, 0),
        (&["1 2 [3] {\"a\":4}\n"], 1, 0),
        (&["1 [2,"], 1, 0),
        (&["1\n2\n", "3\n{\"a\":1,}\n"], 1, 0),
        (&["[1,", "2]"], 1, 0),
        (&["}"], 3, 4),
        (&["ul\n"], 1, 0),
        (&["1 \0 2"], 1, 0),
        (&["1\n  \0"], 1, 0),
        (&["{\"a\":1} [2]"], 1, 0),
        (&["[1 2]"], 1, 0),
        (&["{\"a\" 1}\n"], 1, 0),
        (&["1\n2]"], 1, 0),
        (&["tru "], 1, 0),
        (&["]"], 7, 3),
    ];

    /// ADR-0033 step 3: the port agrees with `parse_message.rs`'s fault messages on every text its
    /// tests pin, one text at a time (`fromjson`) and as a stream.
    #[test]
    fn the_port_words_every_pinned_fault_as_parse_message_does_7987() {
        for text in ONE_TEXT_ROWS {
            assert_eq!(
                jv_parse_sized_message(text.as_bytes(), JQ_MAX_PARSING_DEPTH),
                jq_parse_message(text.as_bytes()),
                "{text:?}"
            );
        }
        for depth in [255, 256, 257] {
            for text in ["[".repeat(depth), "{\"a\":".repeat(depth.div_ceil(2))] {
                assert_eq!(
                    jv_parse_sized_message(text.as_bytes(), JQ_MAX_PARSING_DEPTH),
                    jq_parse_message(text.as_bytes()),
                    "{text:?}"
                );
            }
        }
        for &(chunks, line, column) in STREAM_ROWS {
            let text = chunks.concat();
            let want = jq_stream_fault(chunks.iter().map(|c| c.as_bytes()), line, column);
            for size in [1, 2, 7, 4096] {
                let (_, fault) = stream(text.as_bytes(), line, column, size);
                assert_eq!(fault.map(|f| f.message()), want, "{chunks:?} in {size}s");
            }
        }
        assert_eq!(jq_stream_fault_offset(b"[] \0"), Some(3));
        assert_eq!(stream(b"[] \0", 1, 0, 2).1.map(|f| f.offset), Some(3));
    }

    /// The span of each value is its text in the stream, a bare token ending before its
    /// delimiter, whatever the buffers it was fed in.
    #[test]
    fn each_value_is_handed_back_as_its_span_in_the_stream_7987() {
        let text = b" 12 \"a\\\"b\"[1,{\"k\":[]}]\ntrue{}\t-1.5e3 null";
        let want: Vec<&[u8]> = vec![
            b"12",
            b"\"a\\\"b\"",
            b"[1,{\"k\":[]}]",
            b"true",
            b"{}",
            b"-1.5e3",
            b"null",
        ];
        for size in 1..=text.len() {
            let (values, fault) = stream(text, 1, 0, size);
            assert_eq!(fault, None);
            let texts: Vec<&[u8]> = values.iter().map(|s| &text[s.start..s.end]).collect();
            assert_eq!(texts, want, "{size}");
        }
        // A whole mark is skipped and counted in the offsets, not in the column.
        let mut parser = JvParser::new(false, JQ_MAX_PARSING_DEPTH);
        parser.set_buf(b"\xEF\xBB\xBF[7] x", false);
        assert_eq!(parser.next(), Next::Value(Span { start: 3, end: 6 }));
        let Next::Fault(fault) = parser.next() else {
            panic!("a fault");
        };
        assert_eq!(
            (fault.message(), fault.offset),
            (
                String::from("Invalid numeric literal at EOF at line 1, column 5"),
                8
            )
        );
    }

    /// The mark is matched a byte at a time across buffers; a prefix followed by another byte is
    /// `Malformed BOM` on every call, and under `--seq` a reset on every call instead.
    #[test]
    fn the_mark_is_matched_across_buffers_7987() {
        let mut parser = JvParser::new(false, JQ_MAX_PARSING_DEPTH);
        for piece in [&b"\xEF"[..], b"\xBB", b"\xBF1 "] {
            parser.set_buf(piece, true);
            if piece.len() == 1 {
                assert_eq!(parser.next(), Next::None);
            }
        }
        assert_eq!(parser.next(), Next::Value(Span { start: 3, end: 4 }));
        let mut parser = JvParser::new(false, JQ_MAX_PARSING_DEPTH);
        parser.set_buf(b"\xEF\xBB", true);
        assert_eq!(parser.next(), Next::None);
        parser.set_buf(b"1\n", false);
        for _ in 0..3 {
            let Next::Fault(fault) = parser.next() else {
                panic!("a fault");
            };
            assert_eq!(
                (fault.message(), fault.offset),
                (String::from("Malformed BOM"), 2)
            );
        }
        assert_eq!(parser.remaining(), 2);
        assert!(parser.malformed_bom());
        // `--seq`: the malformed mark resets the parser on every call and reading goes on; the RS
        // after the reset ends an empty record.
        let mut parser = JvParser::new(true, JQ_MAX_PARSING_DEPTH);
        parser.set_buf(b"\xEF\xBB\x1e1 ", false);
        assert_eq!(parser.next(), Next::None);
        assert_eq!(parser.remaining(), 2);
        assert_eq!(parser.next(), Next::Value(Span { start: 3, end: 4 }));
    }

    /// `--seq`: the parser waits for the first RS, a fault not on an RS is `(need RS to resync)`
    /// and the reset after it undoes the wait, and an RS checks what it cuts.
    #[test]
    fn seq_reads_records_by_jv_parse_c_7987() {
        let read = |text: &[u8]| {
            let mut parser = JvParser::new(true, JQ_MAX_PARSING_DEPTH);
            parser.set_buf(text, false);
            let mut out = Vec::new();
            loop {
                match parser.next() {
                    Next::Value(span) => {
                        out.push(String::from_utf8_lossy(&text[span.start..span.end]).into_owned());
                    }
                    Next::Fault(fault) => out.push(fault.message()),
                    Next::None if parser.remaining() > 0 => out.push(String::from("<empty>")),
                    Next::None => break,
                }
            }
            out
        };
        // An RS after a value and white space ends an empty record (`OK` with no value).
        assert_eq!(read(b"x\x1e1\n\x1e[2]\n"), ["1", "<empty>", "[2]"]);
        assert_eq!(
            read(b"\x1e} 5\n"),
            ["Unmatched '}' at line 1, column 2 (need RS to resync)", "5"]
        );
        assert_eq!(
            read(b"\x1e12\x1e3 "),
            [
                "Potentially truncated top-level numeric value at line 1, column 4",
                "3"
            ]
        );
        assert_eq!(
            read(b"\x1e[1\x1e2 "),
            ["Truncated value at line 1, column 4", "2"]
        );
        assert_eq!(read(b"\x1e1 \x1e\x1e2 "), ["1", "<empty>", "<empty>", "2"]);
        assert_eq!(
            read(b"\x1e12"),
            ["Potentially truncated top-level numeric value at EOF at line 1, column 3"]
        );
        assert_eq!(
            read(b"\x1e[1\0"),
            ["Invalid character at line 1, column 4 (need RS to resync)"]
        );
        assert_eq!(
            read(b"ab"),
            ["Unfinished abandoned text at EOF at line 1, column 2"]
        );
    }
}
