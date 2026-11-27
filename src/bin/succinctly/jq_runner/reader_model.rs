//! MEASUREMENT PROTOTYPE (never merged): one port of jq's input reader, entered at sync points.
//!
//! jq reads every input file through one `jq_util_input_state` (`src/util.c`) feeding one
//! `struct jv_parser` (`src/jv_parse.c`), and every edge decision the CLI makes today with a
//! byte-rewriting pre-pass (a NUL inside a number token, a BOM cut across files, a token open at a
//! file seam, where a fault resumes, which line a value is read at, how the input ends) is a plain
//! consequence of that pair's state. This module is that pair, ported as written:
//!
//! - [`Reader::read_more`] is `jq_util_input_read_more`: `fgets` chunking over in-memory sources,
//!   the per-file line counter, the file switch, `is_last`, and 1.7.1's `strlen` cut.
//! - [`Parser`] is the non-streaming, non-`--seq` half of `jv_parse.c`: `jv_parser_set_buf`'s BOM
//!   matcher, `scan`, `token` (`parse_token`), `check_literal`, `found_string`, the error path and
//!   the end-of-input checks.
//! - [`Reader::next_input`] is `jq_util_input_next_input` (parser mode, slurp or not).
//!
//! The parser builds no value: it writes the JSON text of the value it accepted (a number token as
//! the C string jq's number reader saw, so cut at its first NUL; a string as its token with jq's
//! U+FFFD substitution), which the caller hands to the crate's own value reader.
//!
//! `--seq` and `--stream` are not ported here.

/// A jq release whose reader a [`ReaderProfile`] reproduces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Release {
    /// jq 1.7.1 (`71c2ab5`), the pinned reference.
    Jq171,
    /// jq 1.8.2 (`34f7186`), the latest release.
    Jq182,
}

/// The release-dependent switches of jq's reader. Every field names the jq source line it models.
///
/// A profile is one whole release ([`ReaderProfile::release`]), never a switch-by-switch mixture:
/// the prototype's first profile paired 1.7.1's 4,095-byte read with 1.8.2's "no NUL cut", and gave
/// an answer neither release gives on inputs both agree on (a NUL at bytes 4,090-4,093). What the
/// CLI ships ([`ReaderProfile::shipped_for`]) is the release less the switches in [`EXCEPTIONS`],
/// each with its reason; a test holds the two lists to each other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ReaderProfile {
    /// The release this profile reads as.
    pub release: Release,
    /// The most bytes one `fgets` call returns: `sizeof(buf) - 1` = 4095 in jq 1.7.1,
    /// `max_gets_len - 1` = 4091 in jq 1.8.2 (`util.c` line 302 at jq-1.8.2).
    pub read_len: usize,
    /// jq 1.8.2's completion of a full read that ends inside a UTF-8 sequence (`util.c` lines
    /// 335-341): `jvp_utf8_backtrack` finds how many bytes the last sequence lacks and `fread`
    /// appends that many raw bytes, a newline included, which `current_line` never counts.
    pub utf8_completion: bool,
    /// `jq_util_input_read_more`'s return value. `false` is 1.7.1's (`curr_file == nfiles &&
    /// (!current_input || feof || ferror)`: the last data chunk can itself be the last buffer);
    /// `true` is 1.8.2's (`curr_file == nfiles && !current_input`: the last buffer is always an
    /// empty one, read by the call that closes the last file).
    pub closing_refill: bool,
    /// jq 1.7.1's `if (p == NULL && state->parser != NULL) buf_valid_len = strlen(buf)`: a chunk
    /// with no newline is cut at its first NUL and the rest of that chunk is lost. Removed in
    /// 1.8.2 (CVE-2026-33948), where the NUL reaches the parser as a literal byte.
    pub nul_cut: bool,
    /// `MAX_PARSING_DEPTH`: 256 in 1.7.1, 10000 in 1.8.2. A `[` or `{` is refused when the stack
    /// (one entry per array, one per object and one per pending key) already holds this many.
    pub max_depth: usize,
    /// jq 1.7.1 clears `current_filename` and `current_line` when it closes a file, so the
    /// position after the last close is `<unknown>`; 1.8.2 clears them only when the next file
    /// opens, so the last file's name and line stay.
    pub position_after_end_171: bool,
    /// jq 1.7.1's slurp loop reads on while `jv_parser_remaining` is non-zero (`has_more`);
    /// 1.8.2's loop is `while (!is_last)` alone.
    pub slurp_reads_remaining: bool,
    /// jq 1.8.2's `check_literal` case `'\''`: `Invalid string literal; expected ", but got '`.
    /// 1.7.1 reads the token as a number (`Invalid numeric literal`).
    pub single_quote_message: bool,
    /// jq 1.7.1's `jv_number_with_literal` takes whatever decNumber reads, a NaN with a payload
    /// included (`NaN5`); 1.8.2 refuses a NaN with a payload (`Invalid numeric literal`).
    pub nan_payload: bool,
}

/// One switch a shipped profile takes from somewhere other than its release.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct Exception {
    pub release: Release,
    /// The [`ReaderProfile`] field, by name.
    pub field: &'static str,
    pub reason: &'static str,
}

/// Every switch in which [`ReaderProfile::shipped_for`] departs from [`ReaderProfile::release`].
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const EXCEPTIONS: &[Exception] = &[
    Exception {
        release: Release::Jq171,
        field: "nan_payload",
        reason: "the crate's one number grammar refuses a NaN with a payload as jq 1.8 does \
                 (CVE-2024-53427, recorded in limitations.md, #7522); the model's text goes to that \
                 grammar, which would refuse a `NaN5` the 1.7.1 parser accepted",
    },
    Exception {
        release: Release::Jq182,
        field: "max_depth",
        reason: "every document route caps a document at 1.7.1's 256 levels (limitations.md, \
                 \"Nesting depth\", #5497); a 1.8.2 depth here would hand the value reader text \
                 it refuses",
    },
];

impl ReaderProfile {
    /// `release`'s reader as released.
    pub(crate) const fn release(release: Release) -> Self {
        match release {
            Release::Jq171 => Self {
                release,
                read_len: 4095,
                utf8_completion: false,
                closing_refill: false,
                nul_cut: true,
                max_depth: 256,
                position_after_end_171: true,
                slurp_reads_remaining: true,
                single_quote_message: false,
                nan_payload: true,
            },
            Release::Jq182 => Self {
                release,
                read_len: 4091,
                utf8_completion: true,
                closing_refill: true,
                nul_cut: false,
                max_depth: 10000,
                position_after_end_171: false,
                slurp_reads_remaining: false,
                single_quote_message: true,
                nan_payload: false,
            },
        }
    }

    /// What the CLI ships for `release`: the release, less [`EXCEPTIONS`].
    pub(crate) const fn shipped_for(release: Release) -> Self {
        let base = Self::release(release);
        match release {
            Release::Jq171 => Self {
                nan_payload: false,
                ..base
            },
            Release::Jq182 => Self {
                max_depth: 256,
                ..base
            },
        }
    }

    /// The profile `SUCCINCTLY_READER_PROFILE` names (`jq171` or `jq182`; jq 1.8.2 when unset).
    pub(crate) fn shipped() -> Self {
        static PROFILE: std::sync::OnceLock<ReaderProfile> = std::sync::OnceLock::new();
        *PROFILE.get_or_init(|| {
            let value = std::env::var("SUCCINCTLY_READER_PROFILE").ok();
            Self::shipped_for(release_named(value.as_deref()))
        })
    }

    /// Every switch by name, as a number, for the exception test.
    #[cfg(test)]
    pub(crate) fn fields(&self) -> [(&'static str, usize); 9] {
        [
            ("read_len", self.read_len),
            ("utf8_completion", usize::from(self.utf8_completion)),
            ("closing_refill", usize::from(self.closing_refill)),
            ("nul_cut", usize::from(self.nul_cut)),
            ("max_depth", self.max_depth),
            (
                "position_after_end_171",
                usize::from(self.position_after_end_171),
            ),
            (
                "slurp_reads_remaining",
                usize::from(self.slurp_reads_remaining),
            ),
            (
                "single_quote_message",
                usize::from(self.single_quote_message),
            ),
            ("nan_payload", usize::from(self.nan_payload)),
        ]
    }
}

/// The release a `SUCCINCTLY_READER_PROFILE` value names; jq 1.8.2 for anything else.
pub(crate) fn release_named(value: Option<&str>) -> Release {
    match value {
        Some("jq171") => Release::Jq171,
        _ => Release::Jq182,
    }
}

/// The source index of a position with no file (`current_filename` is not a string): jq's
/// `<unknown>`, and `null` from `input_filename`.
pub(crate) const NO_SOURCE: u32 = u32::MAX;

/// One thing a read of jq's reader hands back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Item {
    /// A value: its JSON text as jq understood it, and what `input_filename` (as a source index, or
    /// [`NO_SOURCE`]) and `input_line_number` answer right after the read that completed it.
    Value {
        text: String,
        source: u32,
        line: u64,
    },
    /// A parse error, worded as jq words it, and the same position.
    Fault {
        message: String,
        source: u32,
        line: u64,
    },
}

/// Where jq's reader stands (`jq_util_input_get_position`): a source and its line count, or
/// [`NO_SOURCE`] for `<unknown>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Position {
    pub source: u32,
    pub line: u64,
}

// ---------------------------------------------------------------------------------------------
// jv_parse.c
// ---------------------------------------------------------------------------------------------

/// An entry of `p->stack`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Frame {
    /// `jv_array()`, and whether `jv_array_length` is non-zero.
    Array { has_element: bool },
    /// `jv_object()`, and whether `jv_object_length` is non-zero.
    Object { has_pair: bool },
    /// A key pushed by `:`, waiting for its value (a `JV_KIND_STRING` entry).
    Key,
}

/// The kind of the value in `p->next`; only a string matters (to `:`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    String,
    Other,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum St {
    Normal,
    String,
    StringEscape,
}

/// What `jv_parser_next` returns.
enum Next {
    Value(String),
    Fault(String),
    /// A plain `jv_invalid()`: need more input, or the end.
    None,
}

/// `struct jv_parser`, non-streaming, without `JV_PARSE_SEQ`.
struct Parser<'a> {
    /// `curr_buf` (`None` is the C `0`), `curr_buf_pos`, `curr_buf_is_partial`.
    buf: Option<&'a [u8]>,
    pos: usize,
    partial: bool,
    /// Sticky end of input.
    eof: bool,
    /// `bom_strip_position`: 0..=2 matched bytes, 3 done, 0xff malformed.
    bom: u8,
    stack: Vec<Frame>,
    next: Option<Kind>,
    /// `tokenbuf[..tokenpos]`.
    token: Vec<u8>,
    /// The text of the top-level value in progress: every accepted token, in order.
    out: Vec<u8>,
    line: u64,
    column: u64,
    st: St,
    max_depth: usize,
    single_quote_message: bool,
    nan_payload: bool,
}

const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];
const BOM_MALFORMED: u8 = 0xff;

type Fault = &'static str;

impl<'a> Parser<'a> {
    /// `parser_init`.
    fn new(profile: &ReaderProfile) -> Self {
        Self {
            buf: None,
            pos: 0,
            partial: false,
            eof: false,
            bom: 0,
            stack: Vec::new(),
            next: None,
            token: Vec::new(),
            out: Vec::new(),
            line: 1,
            column: 0,
            st: St::Normal,
            max_depth: profile.max_depth,
            single_quote_message: profile.single_quote_message,
            nan_payload: profile.nan_payload,
        }
    }

    /// `parser_reset`: `line`, `column`, the buffer, `eof` and the BOM state are untouched.
    fn reset(&mut self) {
        self.next = None;
        self.stack.clear();
        self.token.clear();
        self.out.clear();
        self.st = St::Normal;
    }

    /// `jv_parser_remaining`.
    fn remaining(&self) -> usize {
        self.buf.map_or(0, |buf| buf.len() - self.pos)
    }

    /// `jv_parser_set_buf`: the BOM is matched a byte at a time across buffers, and a matched byte
    /// is skipped without being counted in `column`.
    fn set_buf(&mut self, mut buf: &'a [u8], is_partial: bool) {
        debug_assert!(self.remaining() == 0, "previous buffer not exhausted");
        while !buf.is_empty() && (self.bom as usize) < UTF8_BOM.len() {
            if buf[0] == UTF8_BOM[self.bom as usize] {
                buf = &buf[1..];
                self.bom += 1;
            } else if self.bom == 0 {
                // no BOM in this document
                self.bom = UTF8_BOM.len() as u8;
            } else {
                // malformed BOM (prefix present, rest missing)
                self.bom = BOM_MALFORMED;
            }
        }
        self.buf = Some(buf);
        self.pos = 0;
        self.partial = is_partial;
    }

    /// `value`.
    fn value(&mut self, kind: Kind) -> Result<(), Fault> {
        if self.next.is_some() {
            return Err("Expected separator between values");
        }
        self.next = Some(kind);
        Ok(())
    }

    /// `parse_token`.
    fn structure(&mut self, ch: u8) -> Result<(), Fault> {
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
                match self.stack.last().copied() {
                    None => return Err("',' not as part of an object or array"),
                    Some(Frame::Array { .. }) => {
                        *self.stack.last_mut().expect("checked") =
                            Frame::Array { has_element: true };
                    }
                    Some(Frame::Key) => {
                        self.stack.pop();
                        if let Some(Frame::Object { has_pair }) = self.stack.last_mut() {
                            *has_pair = true;
                        }
                    }
                    // this case hits on input like {"a", "b"}
                    Some(Frame::Object { .. }) => {
                        return Err("Objects must consist of key:value pairs")
                    }
                }
                self.next = None;
            }
            b']' => {
                let Some(Frame::Array { has_element }) = self.stack.last().copied() else {
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
                let Some(top) = self.stack.last().copied() else {
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
                            return Err("Expected another key-value pair")
                        }
                        Frame::Object { has_pair: false } => {}
                        _ => return Err("Unmatched '}'"),
                    }
                }
                self.stack.pop();
                self.next = Some(Kind::Other);
            }
        }
        self.out.push(ch);
        Ok(())
    }

    /// `found_string`: the escapes and control bytes of the finished string token, in the order
    /// jq meets them, decoded into the bytes jq's `out` pointer writes; then `value` of
    /// `jv_string_sized` over them, which replaces what is not UTF-8 by jq's own rule
    /// ([`jq_string_sized`]). The string goes into the value's text re-escaped, so the text the
    /// crate's value reader gets is valid UTF-8 and holds the string jq holds.
    fn found_string(&mut self) -> Result<(), Fault> {
        let token = &self.token;
        let end = token.len();
        let mut body: Vec<u8> = Vec::with_capacity(end);
        let mut at = 0;
        while at < end {
            let c = token[at];
            at += 1;
            if c == b'\\' {
                if at >= end {
                    return Err("Expected escape character at end of string");
                }
                let c = token[at];
                at += 1;
                match c {
                    b'\\' | b'"' | b'/' => body.push(c),
                    b'b' => body.push(0x08),
                    b'f' => body.push(0x0c),
                    b't' => body.push(b'\t'),
                    b'n' => body.push(b'\n'),
                    b'r' => body.push(b'\r'),
                    b'u' => {
                        if at + 4 > end {
                            return Err("Invalid \\uXXXX escape");
                        }
                        let Some(mut code) = unhex4(&token[at..at + 4]) else {
                            return Err("Invalid characters in \\uXXXX escape");
                        };
                        at += 4;
                        if (0xD800..=0xDBFF).contains(&code) {
                            const PAIR: Fault = "Invalid \\uXXXX\\uXXXX surrogate pair escape";
                            if at + 6 > end || token[at] != b'\\' || token[at + 1] != b'u' {
                                return Err(PAIR);
                            }
                            match unhex4(&token[at + 2..at + 6]) {
                                Some(low) if (0xDC00..=0xDFFF).contains(&low) => {
                                    at += 6;
                                    code = 0x10000 + (((code - 0xD800) << 10) | (low - 0xDC00));
                                }
                                _ => return Err(PAIR),
                            }
                        }
                        // `if (codepoint > 0x10FFFF) codepoint = 0xFFFD` cannot fire: four hex
                        // digits or a pair never exceed it.
                        jvp_utf8_encode(code, &mut body);
                    }
                    _ => return Err("Invalid escape"),
                }
            } else if c <= 0x1f {
                return Err(
                    "Invalid string: control characters from U+0000 through U+001F must be escaped",
                );
            } else {
                body.push(c);
            }
        }
        self.value(Kind::String)?;
        let string = jq_string_sized(&body);
        push_json_string(&mut self.out, &string);
        self.token.clear();
        Ok(())
    }

    /// `check_literal`: `true`/`false`/`null` are compared by the token's length, NULs included; a
    /// number is read out of the token as a C string, so its first NUL ends it.
    fn check_literal(&mut self) -> Result<(), Fault> {
        if self.token.is_empty() {
            return Ok(());
        }
        let pattern: Option<&[u8]> = match self.token[0] {
            b't' => Some(b"true"),
            b'f' => Some(b"false"),
            b'\'' if self.single_quote_message => {
                return Err("Invalid string literal; expected \", but got '");
            }
            // jq 1.7.1 reads `tokenbuf[1]` without checking `tokenpos > 1` (a stale byte for the
            // token `n`); 1.8.2's check is the one modelled.
            b'n' if self.token.get(1) == Some(&b'u') => Some(b"null"),
            _ => None,
        };
        if let Some(pattern) = pattern {
            if self.token != pattern {
                return Err("Invalid literal");
            }
            self.value(Kind::Other)?;
            self.out.extend_from_slice(pattern);
        } else {
            let c_string = self
                .token
                .split(|&byte| byte == 0)
                .next()
                .unwrap_or_default();
            // `jv_number_with_literal`: decNumber's grammar. The crate's one definition of it
            // refuses a NaN payload as jq 1.8 does; 1.7.1 accepts `nan5`.
            if !(crate::cli::jq_seq_reader::number_is_valid(c_string)
                || (self.nan_payload && is_nan_with_payload(c_string)))
            {
                return Err("Invalid numeric literal");
            }
            let len = c_string.len();
            self.value(Kind::Other)?;
            self.out.extend_from_slice(&self.token[..len]);
        }
        self.token.clear();
        Ok(())
    }

    /// `parse_check_done`: the finished top-level value, moved out.
    fn check_done(&mut self) -> Option<String> {
        if !self.stack.is_empty() || self.next.is_none() {
            return None;
        }
        self.next = None;
        let text = std::mem::take(&mut self.out);
        // Strings were replaced and re-escaped by `found_string`; a number is checked against
        // decNumber's grammar and a literal is ASCII, so nothing else can hold such a byte.
        Some(String::from_utf8(text).unwrap_or_else(|e| jq_string_sized(e.as_bytes())))
    }

    /// `scan`, without the RS branch.
    fn scan(&mut self, ch: u8) -> Result<Option<String>, Fault> {
        self.column += 1;
        if ch == b'\n' {
            self.line += 1;
            self.column = 0;
        }
        let mut answer = None;
        if self.st == St::Normal {
            let whitespace = matches!(ch, b' ' | b'\t' | b'\r' | b'\n');
            let quote = ch == b'"';
            let structure = matches!(ch, b'[' | b',' | b']' | b'{' | b':' | b'}');
            if whitespace || quote || structure {
                self.check_literal()?;
                // A value moved out here is lost if the token below raises: `1}` is only the
                // error, `1 }` is 1 and then the error.
                if let Some(done) = self.check_done() {
                    answer = Some(done);
                }
            }
            if structure {
                self.structure(ch)?;
            } else if quote {
                self.st = St::String;
            } else if !whitespace {
                self.token.push(ch);
            }
            if let Some(done) = self.check_done() {
                answer = Some(done);
            }
        } else if ch == b'"' && self.st == St::String {
            self.found_string()?;
            self.st = St::Normal;
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
        Ok(answer)
    }

    /// `jv_parser_next`.
    fn next(&mut self) -> Next {
        if self.eof {
            return Next::None;
        }
        let Some(buf) = self.buf else {
            return Next::None; // Need a buffer
        };
        if self.bom == BOM_MALFORMED {
            // Consumes nothing, so every later call answers this again.
            return Next::Fault(String::from("Malformed BOM"));
        }
        while self.pos < buf.len() {
            let ch = buf[self.pos];
            self.pos += 1;
            match self.scan(ch) {
                Ok(None) => {}
                Ok(Some(value)) => return Next::Value(value),
                Err(msg) => {
                    let message = format!("{msg} at line {}, column {}", self.line, self.column);
                    self.reset();
                    // We're not parsing a JSON text sequence; throw this buffer away.
                    self.buf = None;
                    self.pos = 0;
                    return Next::Fault(message);
                }
            }
        }
        if self.partial {
            // need another buffer
            return Next::None;
        }
        // at EOF
        self.eof = true;
        if self.st != St::Normal {
            let message = format!(
                "Unfinished string at EOF at line {}, column {}",
                self.line, self.column
            );
            self.reset();
            return Next::Fault(message);
        }
        if let Err(msg) = self.check_literal() {
            let message = format!("{msg} at EOF at line {}, column {}", self.line, self.column);
            self.reset();
            return Next::Fault(message);
        }
        if !self.stack.is_empty() {
            let message = format!(
                "Unfinished JSON term at EOF at line {}, column {}",
                self.line, self.column
            );
            self.reset();
            return Next::Fault(message);
        }
        // `p->next` is either invalid (nothing here, but no syntax error) or the value.
        match self.check_done() {
            Some(value) => Next::Value(value),
            None => Next::None,
        }
    }
}

/// decNumber's NaN with a payload: an optional sign, `NaN` or `sNaN` in any case, then digits.
fn is_nan_with_payload(token: &[u8]) -> bool {
    let body = match token.first() {
        Some(b'+' | b'-') => &token[1..],
        _ => token,
    };
    let body = match body.first() {
        Some(b's' | b'S') => &body[1..],
        _ => body,
    };
    body.len() > 3
        && body[..3].eq_ignore_ascii_case(b"nan")
        && body[3..].iter().all(u8::is_ascii_digit)
}

/// `utf8_coding_length` of `jv_utf8_tables.h` (the same table in 1.7.1 and 1.8.2):
/// [`UTF8_CONTINUATION_BYTE`] for 80-BF, 0 for a byte no sequence starts with (C0, C1, F5-FF).
const fn utf8_coding_length(byte: u8) -> u8 {
    match byte {
        0x00..=0x7f => 1,
        0x80..=0xbf => UTF8_CONTINUATION_BYTE,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => 0,
    }
}

/// `utf8_coding_bits`: the payload bits of a lead byte.
const fn utf8_coding_bits(byte: u8) -> u32 {
    match byte {
        0x00..=0x7f => 0x7f,
        0x80..=0xbf => 0x3f,
        0xc2..=0xdf => 0x1f,
        0xe0..=0xef => 0x0f,
        0xf0..=0xf4 => 0x07,
        _ => 0,
    }
}

const UTF8_CONTINUATION_BYTE: u8 = 255;

/// `utf8_first_codepoint`: the least code point a sequence of each length may encode.
const UTF8_FIRST_CODEPOINT: [i64; 5] = [0, 0, 0x80, 0x800, 0x10000];

/// `jvp_utf8_next` (`jv_unicode.c`, the same rule in 1.7.1 and 1.8.2): the length of the step at
/// the front of `bytes` (not empty) and its code point, `-1` for a bad one. A lead byte whose
/// sequence is longer than what is left takes all that is left, ASCII included (the "tail
/// swallow"); a whole-length overlong, surrogate or out-of-range sequence is one bad step.
fn jvp_utf8_next(bytes: &[u8]) -> (usize, i64) {
    let first = bytes[0];
    let mut length = usize::from(utf8_coding_length(first));
    let mut codepoint: i64 = -1;
    if first & 0x80 == 0 {
        // Fast-path for ASCII
        codepoint = i64::from(first);
        length = 1;
    } else if length == 0 || length == usize::from(UTF8_CONTINUATION_BYTE) {
        // Bad single byte - either an invalid byte or an out-of-place continuation byte
        length = 1;
    } else if length > bytes.len() {
        // String ends before UTF8 sequence ends
        length = bytes.len();
    } else {
        codepoint = i64::from(u32::from(first) & utf8_coding_bits(first));
        for (i, &ch) in bytes.iter().enumerate().take(length).skip(1) {
            if utf8_coding_length(ch) != UTF8_CONTINUATION_BYTE {
                // Invalid UTF8 sequence - not followed by the right number of continuation bytes
                codepoint = -1;
                length = i;
                break;
            }
            codepoint = (codepoint << 6) | i64::from(ch & 0x3f);
        }
        if codepoint < UTF8_FIRST_CODEPOINT[length] {
            // Overlong UTF8 sequence
            codepoint = -1;
        }
        if (0xD800..=0xDFFF).contains(&codepoint) {
            // Surrogate codepoints can't be encoded in UTF8
            codepoint = -1;
        }
        if codepoint > 0x10FFFF {
            // Outside Unicode range
            codepoint = -1;
        }
    }
    (length, codepoint)
}

/// `jv_string_sized`: `bytes` as they are when they are UTF-8 (`jvp_utf8_is_valid` refuses what
/// Rust's validation refuses), else `jvp_string_copy_replace_bad`: one U+FFFD per bad step of
/// [`jvp_utf8_next`].
pub(crate) fn jq_string_sized(bytes: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_owned();
    }
    let mut out = String::with_capacity(bytes.len() + 8);
    let mut at = 0;
    while at < bytes.len() {
        let (length, codepoint) = jvp_utf8_next(&bytes[at..]);
        at += length;
        let ch = u32::try_from(codepoint)
            .ok()
            .and_then(char::from_u32)
            .unwrap_or('\u{fffd}');
        out.push(ch);
    }
    out
}

/// `jvp_utf8_encode`: a code point as UTF-8 bytes, a surrogate included (as `ED A0 80`, which
/// [`jq_string_sized`] then replaces, as jq's does after a lone `\uDC00`).
fn jvp_utf8_encode(codepoint: u32, out: &mut Vec<u8>) {
    if codepoint <= 0x7f {
        out.push(codepoint as u8);
    } else if codepoint <= 0x7ff {
        out.push(0xc0 | (codepoint >> 6) as u8);
        out.push(0x80 | (codepoint & 0x3f) as u8);
    } else if codepoint <= 0xffff {
        out.push(0xe0 | (codepoint >> 12) as u8);
        out.push(0x80 | ((codepoint >> 6) & 0x3f) as u8);
        out.push(0x80 | (codepoint & 0x3f) as u8);
    } else {
        out.push(0xf0 | (codepoint >> 18) as u8);
        out.push(0x80 | ((codepoint >> 12) & 0x3f) as u8);
        out.push(0x80 | ((codepoint >> 6) & 0x3f) as u8);
        out.push(0x80 | (codepoint & 0x3f) as u8);
    }
}

/// `string` as a JSON string literal.
fn push_json_string(out: &mut Vec<u8>, string: &str) {
    out.push(b'"');
    for ch in string.chars() {
        match ch {
            '"' => out.extend_from_slice(b"\\\""),
            '\\' => out.extend_from_slice(b"\\\\"),
            c if u32::from(c) < 0x20 => {
                out.extend_from_slice(format!("\\u{:04x}", u32::from(c)).as_bytes());
            }
            c => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
    out.push(b'"');
}

/// `jvp_utf8_backtrack(end - 1, buf, &len)` as `jq_util_input_read_more` calls it in 1.8.2: how
/// many bytes the last sequence of `buf` lacks, or `None` (the C `NULL`) when the bytes before the
/// end hold no lead byte or a bad one.
fn utf8_backtrack(buf: &[u8]) -> Option<usize> {
    if buf.len() <= 1 {
        // `if (min == start) return min;` without writing `*missing_bytes`.
        return Some(0);
    }
    let mut start = buf.len() - 1;
    let mut seen = 1usize;
    let mut length;
    loop {
        length = utf8_coding_length(buf[start]);
        if length != UTF8_CONTINUATION_BYTE {
            break;
        }
        if start == 0 {
            break;
        }
        start -= 1;
        seen += 1;
    }
    if length == 0 || length == UTF8_CONTINUATION_BYTE || usize::from(length) < seen {
        return None;
    }
    Some(usize::from(length) - seen)
}

/// `unhex4`.
fn unhex4(hex: &[u8]) -> Option<u32> {
    let mut r = 0u32;
    for &c in &hex[..4] {
        let n = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => return None,
        };
        r = (r << 4) | u32::from(n);
    }
    Some(r)
}

// ---------------------------------------------------------------------------------------------
// util.c
// ---------------------------------------------------------------------------------------------

/// What one `jq_util_input_next_input` call returns.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Value(String),
    Fault(String),
    /// The array of a slurp, as the texts of its members. Returned once: `state->slurped` is
    /// invalid afterwards, so every later call reads as a run that does not slurp.
    Slurped(Vec<String>),
    /// No more input (a plain invalid).
    End,
}

/// `jq_util_input_state` over in-memory sources, with its parser.
pub(crate) struct Reader<'a> {
    profile: ReaderProfile,
    sources: &'a [&'a [u8]],
    /// `curr_file`: the next source to open.
    curr_file: usize,
    /// `current_input`: the open source, how far it is read, and its `feof`.
    open: Option<usize>,
    offset: usize,
    feof: bool,
    /// `current_filename` (as a source index) and `current_line`.
    current_source: Option<usize>,
    current_line: u64,
    parser: Parser<'a>,
    /// `state->slurped`, as the texts of its members; `None` is the C `jv_invalid()`.
    slurped: Option<Vec<String>>,
}

impl<'a> Reader<'a> {
    /// A reader of `sources`, in command-line order, one per file; standard input is one source.
    /// `slurp` is `jq_util_input_set_parser`'s flag.
    pub(crate) fn new(sources: &'a [&'a [u8]], profile: ReaderProfile, slurp: bool) -> Self {
        Self {
            profile,
            sources,
            curr_file: 0,
            open: None,
            offset: 0,
            feof: false,
            current_source: None,
            current_line: 0,
            parser: Parser::new(&profile),
            slurped: slurp.then(Vec::new),
        }
    }

    /// `jq_util_input_read_more`: the next chunk, and whether it is the last buffer.
    fn read_more(&mut self) -> (&'a [u8], bool) {
        if self.open.is_none() || self.feof {
            if self.open.take().is_some() && self.profile.position_after_end_171 {
                self.current_source = None;
                self.current_line = 0;
            }
            if self.curr_file < self.sources.len() {
                let file = self.curr_file;
                self.curr_file += 1;
                self.open = Some(file);
                self.offset = 0;
                self.feof = false;
                self.current_source = Some(file);
                self.current_line = 0;
            }
        }
        let mut chunk: &'a [u8] = &[];
        if let Some(file) = self.open {
            let rest: &'a [u8] = &self.sources[file][self.offset..];
            if rest.is_empty() {
                // `fgets` returns NULL at the end of the file.
                self.feof = true;
            } else {
                let cap = self.profile.read_len;
                let window = &rest[..cap.min(rest.len())];
                let newline = window.iter().position(|&byte| byte == b'\n');
                let read = newline.map_or(window.len(), |at| at + 1);
                self.offset += read;
                // `fgets` met the end of the file before a newline or a full buffer.
                if newline.is_none() && read < cap {
                    self.feof = true;
                }
                let valid = if newline.is_some() {
                    self.current_line += 1;
                    read
                } else if self.profile.nul_cut {
                    // `strlen(buf)`
                    window.iter().position(|&byte| byte == 0).unwrap_or(read)
                } else {
                    read
                };
                chunk = &window[..valid];
                // jq 1.8.2: a full read with no newline (`p == NULL`, not at EOF) that ends inside
                // a UTF-8 sequence takes the bytes it lacks with `fread`, whatever they are.
                if self.profile.utf8_completion && newline.is_none() && read == cap {
                    if let Some(missing) = utf8_backtrack(chunk).filter(|&missing| missing > 0) {
                        let more = missing.min(rest.len() - read);
                        self.offset += more;
                        if more < missing {
                            self.feof = true;
                        }
                        chunk = &rest[..read + more];
                    }
                }
            }
        }
        let is_last = self.curr_file == self.sources.len()
            && (self.open.is_none() || (!self.profile.closing_refill && self.feof));
        (chunk, is_last)
    }

    /// `jq_util_input_next_input`, parser mode.
    pub(crate) fn next_input(&mut self) -> Step {
        let mut is_last = false;
        let mut has_more = false;
        loop {
            if self.parser.remaining() == 0 {
                let (chunk, last) = self.read_more();
                is_last = last;
                self.parser.set_buf(chunk, !is_last);
            }
            let value = self.parser.next();
            if let Some(slurped) = &mut self.slurped {
                if self.profile.slurp_reads_remaining {
                    has_more = self.parser.remaining() != 0;
                }
                match value {
                    Next::Value(text) => slurped.push(text),
                    Next::Fault(message) => return Step::Fault(message),
                    Next::None => {}
                }
            } else {
                match value {
                    Next::Value(text) => return Step::Value(text),
                    Next::Fault(message) => return Step::Fault(message),
                    Next::None => {}
                }
            }
            if is_last && !has_more {
                break;
            }
        }
        match self.slurped.take() {
            Some(members) => Step::Slurped(members),
            None => Step::End,
        }
    }

    /// `jq_util_input_get_position`, as it stands now.
    pub(crate) fn position(&self) -> Position {
        Position {
            source: self.current_source.map_or(NO_SOURCE, |source| {
                u32::try_from(source).unwrap_or(NO_SOURCE)
            }),
            line: self.current_line,
        }
    }

    /// Whether the parser is in its `Malformed BOM` state, which it never leaves: every read from
    /// here on is that fault again.
    pub(crate) fn malformed_bom(&self) -> bool {
        self.parser.bom == BOM_MALFORMED
    }

    /// The next item of a run that does not slurp, with the position its read left; `None` at the
    /// end of the input, whose position is then [`position`](Self::position).
    pub(crate) fn next_item(&mut self) -> Option<Item> {
        let step = self.next_input();
        let Position { source, line } = self.position();
        match step {
            Step::Value(text) => Some(Item::Value { text, source, line }),
            Step::Fault(message) => Some(Item::Fault {
                message,
                source,
                line,
            }),
            Step::Slurped(_) | Step::End => None,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The CLI's side: the trigger, the switch, the trace.
// ---------------------------------------------------------------------------------------------

/// `SUCCINCTLY_READER_MODEL=0` turns the model off: every route reads as it did before.
pub(crate) fn enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var_os("SUCCINCTLY_READER_MODEL").map_or(true, |value| value != "0")
    })
}

/// `SUCCINCTLY_READER_MODEL_TRACE=1`: what the model read, on standard error.
pub(crate) fn trace(route: &str, values: usize, faults: usize, refused: usize) {
    if std::env::var_os("SUCCINCTLY_READER_MODEL_TRACE").is_some_and(|value| value == "1") {
        eprintln!("reader_model: route={route} values={values} faults={faults} refused={refused}");
    }
}

/// Whether the sources may hold an edge the fast readers do not decide as jq's reader does:
///
/// - `any_nul`: some source holds a NUL byte (a fact each route's own NUL search already has);
/// - the first source with a byte starts with `EF` but not with the whole mark `EF BB BF`;
/// - of two or more sources with bytes, one before the last ends in a byte other than white
///   space, `}`, `]` or `"`: a bare token may be open there.
///
/// Reads the first three bytes of one source and the last byte of each: nothing per byte.
fn seam_trigger_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("SUCCINCTLY_READER_MODEL_SEAM").is_some_and(|v| v == "1"))
}

pub(crate) fn trigger<'s>(sources: impl Iterator<Item = &'s [u8]>, any_nul: bool) -> bool {
    if any_nul {
        return true;
    }
    // The last byte of the source with bytes before the one in hand.
    let mut previous: Option<u8> = None;
    for source in sources {
        let Some(&last) = source.last() else {
            continue;
        };
        match previous {
            None => {
                if source[0] == UTF8_BOM[0] && !source.starts_with(&UTF8_BOM) {
                    return true;
                }
            }
            Some(end) => {
                // A source that ends on white space or on the closing byte of a string or
                // container ends between tokens as far as one byte can say (`[1,{"a":1}` then
                // `]` is certified wrongly: the splitter's own verdict would not be).
                // Round 3: a seam alone is no trigger (the perf guard's `split*` rows are a
                // well-formed document cut across files; a seam is an edge only with a NUL, a
                // broken mark or a fault, and those have their own triggers). Set
                // `SUCCINCTLY_READER_MODEL_SEAM=1` for round 2's token-byte seam trigger.
                if seam_trigger_enabled()
                    && !matches!(end, b'\n' | b' ' | b'\t' | b'\r' | b'}' | b']' | b'"')
                {
                    return true;
                }
            }
        }
        previous = Some(last);
    }
    false
}

#[cfg(test)]
mod tests;
