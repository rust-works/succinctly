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
//! This module classifies *failures only*. It builds no values and is not
//! wired to what `--seq` emits on stdout -- that stays with
//! [`super::jq_runner::seq_record_scan`], whose deliberate conservatism is
//! documented there and in `docs/compliance/jq/limitations.md`.

use crate::front_matter::UTF8_BOM;

const ASCII_RS: u8 = 0x1e;

/// jq's `MAX_PARSING_DEPTH` (`jv_parse.c`), confirmed against the oracle:
/// 256 nested `[` parse and the 257th reports `Exceeds depth limit`.
const MAX_PARSING_DEPTH: usize = 256;

/// What jq's `jv_parser_set_buf` consumes as a leading BOM, over all
/// sources concatenated.
///
/// jq consumes the matching *prefix* byte by byte, so a partial BOM is
/// eaten even though it never completes one. Those bytes never reach the
/// scanner, so they never advance a column either -- getting this wrong
/// shifts every position in the stream.
#[derive(Clone, Copy, PartialEq, Eq)]
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

/// Hands `emit` every `jq: ignoring parse error: ...` line real jq would
/// write for this `--seq` stream, in order.
///
/// A sink rather than a `Vec`: an adversarial stream can warn once per
/// byte, and collecting those first cost ~140x the input in peak RSS
/// (2 MB of `}` measured at 286 MB, against jq's 2.4 MB). Streaming them
/// keeps the reader flat.
pub(crate) fn for_each_warning(raw_bytes: &[(Option<usize>, Vec<u8>)], emit: &mut dyn FnMut(&str)) {
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
    reader.run(stream_bytes(raw_bytes));
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
    /// See [`for_each_warning`]: a malformed BOM plus a newline-terminated
    /// stream means jq's own EOF report is wiped before it is written.
    suppress_eof_warning: bool,
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
            suppress_eof_warning: false,
            emit,
        }
    }

    fn warn(&mut self, body: &str) {
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
    }

    fn run(&mut self, bytes: impl Iterator<Item = u8>) {
        for ch in bytes {
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
            } else if self.resets_per_call && (self.produced_value || ch == b'\n') {
                self.reset();
            }
            self.produced_value = false;
        }
        self.finish();
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
                Cls::Quote => self.st = St::Str,
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
            self.produced_value = true;
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
            // #1525's template. Unreachable from the wired call site,
            // which routes a stream with no RS byte to
            // `seq_no_rs_byte_warning` instead, but kept so this model
            // stands on its own.
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
        if !self.last_ch_was_ws && self.next.take() == Some(Kind::Number) {
            self.warn(&format!(
                "Potentially truncated top-level numeric value at EOF at line {line}, column {column}"
            ));
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
    let body = match token.first() {
        Some(b'+' | b'-') => &token[1..],
        _ => token,
    };
    if body.is_empty() {
        return false;
    }

    // Longest first: stripping `inf` from `infinity` would leave `inity`.
    for form in [b"infinity".as_slice(), b"inf".as_slice()] {
        if let Some(rest) = strip_prefix_ignore_ascii_case(body, form) {
            return rest.is_empty();
        }
    }
    let nan_body = match body.first() {
        Some(b's' | b'S') => &body[1..],
        _ => body,
    };
    if let Some(payload) = strip_prefix_ignore_ascii_case(nan_body, b"nan") {
        return payload.iter().all(u8::is_ascii_digit);
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

fn strip_prefix_ignore_ascii_case<'a>(bytes: &'a [u8], prefix: &[u8]) -> Option<&'a [u8]> {
    let head = bytes.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &bytes[prefix.len()..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn warnings(sources: &[&[u8]]) -> Vec<String> {
        let owned: Vec<(Option<usize>, Vec<u8>)> =
            sources.iter().map(|s| (None, s.to_vec())).collect();
        let mut got = Vec::new();
        for_each_warning(&owned, &mut |w| {
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
