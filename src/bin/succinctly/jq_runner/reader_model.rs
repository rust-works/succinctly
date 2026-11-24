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

/// The release-dependent switches of jq's reader. Every field names the jq source line it models.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ReaderProfile {
    /// The most bytes one `fgets` call returns: `sizeof(buf) - 1` = 4095 in jq 1.7.1,
    /// `max_gets_len - 1` = 4091 in jq 1.8.2 (which also completes a cut UTF-8 sequence with
    /// `fread`; that backtrack is NOT modelled).
    pub read_len: usize,
    /// `jq_util_input_read_more`'s return value. `false` is 1.7.1's (`curr_file == nfiles &&
    /// (!current_input || feof || ferror)`: the last data chunk can itself be the last buffer);
    /// `true` is 1.8.2's (`curr_file == nfiles && !current_input`: the last buffer is always an
    /// empty one, read by the call that closes the last file).
    pub closing_refill: bool,
    /// jq 1.7.1's `if (p == NULL && state->parser != NULL) buf_valid_len = strlen(buf)`: a chunk
    /// with no newline is cut at its first NUL and the rest of that chunk is lost. Removed in
    /// 1.8.2, where the NUL reaches the parser as a literal byte.
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
}

impl ReaderProfile {
    /// What `succinctly jq` follows, switch by switch:
    ///
    /// - `read_len: 4095`: jq 1.7.1 (the pinned reference; every chunk emulation in the CLI is 4095).
    /// - `closing_refill: false`: jq 1.7.1's `is_last`, for the non-slurp plain route.
    /// - `nul_cut: false`: jq 1.8.2. This is the recorded #5654 divergence from the pinned 1.7.1:
    ///   the bytes after a NUL in a newline-less read are not dropped, the NUL reaches the parser.
    /// - `max_depth: 256`: jq 1.7.1.
    /// - `position_after_end_171: true`: jq 1.7.1.
    /// - `slurp_reads_remaining: true`: jq 1.7.1.
    /// - `single_quote_message: false`: jq 1.7.1.
    pub(crate) const fn succinctly() -> Self {
        Self {
            read_len: 4095,
            closing_refill: false,
            nul_cut: false,
            max_depth: 256,
            position_after_end_171: true,
            slurp_reads_remaining: true,
            single_quote_message: false,
        }
    }

    /// jq 1.7.1 as released (tests only).
    #[cfg(test)]
    const fn jq171() -> Self {
        Self {
            nul_cut: true,
            ..Self::succinctly()
        }
    }

    /// jq 1.8.2 as released, less its UTF-8 backtrack of a full read (tests only).
    #[cfg(test)]
    const fn jq182() -> Self {
        Self {
            read_len: 4091,
            closing_refill: true,
            nul_cut: false,
            max_depth: 10000,
            position_after_end_171: false,
            slurp_reads_remaining: false,
            single_quote_message: true,
        }
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
    Value { text: String, source: u32, line: u64 },
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
    /// jq meets them, then `value`.
    fn found_string(&mut self) -> Result<(), Fault> {
        let token = &self.token;
        let end = token.len();
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
                    b'\\' | b'"' | b'/' | b'b' | b'f' | b't' | b'n' | b'r' => {}
                    b'u' => {
                        if at + 4 > end {
                            return Err("Invalid \\uXXXX escape");
                        }
                        let Some(code) = unhex4(&token[at..at + 4]) else {
                            return Err("Invalid characters in \\uXXXX escape");
                        };
                        at += 4;
                        if (0xD800..=0xDBFF).contains(&code) {
                            const PAIR: Fault = "Invalid \\uXXXX\\uXXXX surrogate pair escape";
                            if at + 6 > end || token[at] != b'\\' || token[at + 1] != b'u' {
                                return Err(PAIR);
                            }
                            match unhex4(&token[at + 2..at + 6]) {
                                Some(low) if (0xDC00..=0xDFFF).contains(&low) => at += 6,
                                _ => return Err(PAIR),
                            }
                        }
                    }
                    _ => return Err("Invalid escape"),
                }
            } else if c <= 0x1f {
                return Err(
                    "Invalid string: control characters from U+0000 through U+001F must be escaped",
                );
            }
        }
        self.value(Kind::String)?;
        self.out.push(b'"');
        self.out.append(&mut self.token);
        self.out.push(b'"');
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
            if !crate::cli::jq_seq_reader::number_is_valid(c_string) {
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
        // `jv_string_sized` replaces what is not UTF-8 in a string with U+FFFD; nothing else of an
        // accepted value can hold such a byte (a number is checked against decNumber's grammar).
        Some(match String::from_utf8(text) {
            Ok(text) => text,
            Err(e) => succinctly::jq::utf8_document::substitute_invalid_utf8_jq_document(
                e.as_bytes(),
            ),
        })
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
                    let message =
                        format!("{msg} at line {}, column {}", self.line, self.column);
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
            let message = format!(
                "{msg} at EOF at line {}, column {}",
                self.line, self.column
            );
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
            source: self
                .current_source
                .map_or(NO_SOURCE, |source| u32::try_from(source).unwrap_or(NO_SOURCE)),
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
/// - of two or more sources with bytes, one before the last does not end in a newline.
///
/// Reads the first three bytes of one source and the last byte of each: nothing per byte.
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
                if end != b'\n' {
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
