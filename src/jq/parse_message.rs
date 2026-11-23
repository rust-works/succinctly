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
    /// Whether a second top-level value is fine: jq's input stream, not `fromjson`'s one text.
    multi: bool,
}

/// jq's decNumber reader: an optionally signed decimal with an optional exponent, or
/// `inf`/`infinity`/`nan`/`snan` in any case (`nan` and `snan` with a zero payload only: jq 1.8
/// refuses a payload, #7522).
fn is_number_token(token: &[u8]) -> bool {
    // The word grammar is one definition (#2877, #7522): the readers and this message must agree.
    if crate::json::validate::jq_special_number(token).is_some() {
        return true;
    }
    let body = match token.first() {
        Some(b'+' | b'-') => &token[1..],
        _ => token,
    };
    let lower: Vec<u8> = body.iter().map(u8::to_ascii_lowercase).collect();
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

/// The offset of the first NUL byte in `hay` (#7437).
///
/// The one search for a NUL that [`blank_nul_number_tails`] and the CLI's reading routes make. A
/// text with no NUL costs one unsigned `min` per vector of bytes (`util::simd::nul`).
#[inline]
pub fn find_nul(hay: &[u8]) -> Option<usize> {
    crate::util::simd::nul::find_nul(hay)
}

/// What [`scan_utf8_and_nul`] finds of a text: whether it is UTF-8 (as
/// [`validate_utf8`](crate::text::utf8::validate_utf8) answers) and whether it holds a NUL byte (as
/// [`find_nul`] answers).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextScan {
    /// The text is UTF-8.
    pub utf8: bool,
    /// The text holds a NUL byte.
    pub nul: bool,
}

/// The bytes [`scan_utf8_and_nul`] reads for their smallest and largest byte before it validates
/// them: few enough to still be in the core's own cache when the validation reads them again.
const SCAN_PIECE: usize = 128 * 1024;

/// Validates `text` as UTF-8 and looks for a NUL byte in it, in one read of it from memory (#7437).
///
/// Every route that reads JSON input validates each text before anything else reads it, and the
/// search for the NUL a number token may hold was a second pass over the same bytes: on a text
/// bigger than the cache it read them from memory again (+1.8-3.1% wall clock on whole-file 100 MB
/// rows on a Ryzen 9 7950X), and on an Apple M4 Pro even a search of each piece the validation had
/// just read cost +1.5-2.6%. So each piece of the text is read once for its smallest and largest
/// byte (`util::simd::nul::min_max`): the smallest is 0 where it holds a NUL, and
/// a largest under 0x80 is ASCII, which is UTF-8 with no further check. Only a piece with a byte
/// past 0x7F is validated, while it is still in the cache.
///
/// The pieces are cut before a byte that is not a continuation byte, so no character is cut: a
/// UTF-8 text cut there is pieces that are each UTF-8, and a text that is not leaves a piece that is
/// not (a sequence cut short ends the piece before it, and a stray continuation byte stays in one).
/// So [`TextScan::utf8`] is the whole text's verdict.
#[must_use]
pub fn scan_utf8_and_nul(text: &[u8]) -> TextScan {
    scan_utf8_and_nul_in(text, SCAN_PIECE)
}

/// [`scan_utf8_and_nul`] with pieces of about `piece` bytes.
fn scan_utf8_and_nul_in(text: &[u8], piece: usize) -> TextScan {
    let mut scan = TextScan {
        utf8: true,
        nul: false,
    };
    let mut at = 0;
    while at < text.len() {
        let mut end = (at + piece).min(text.len());
        // On to the end of the character `end` is inside of, three bytes at most: a fourth
        // continuation byte in a row is no character's, and the piece after it is not UTF-8.
        let ceiling = (end + 3).min(text.len());
        while end < ceiling && text[end] & 0xC0 == 0x80 {
            end += 1;
        }
        let bytes = &text[at..end];
        let (lo, hi) = crate::util::simd::nul::min_max(bytes);
        // A piece of ASCII bytes is UTF-8: only one with a byte past 0x7F is validated.
        if scan.utf8 && hi >= 0x80 {
            scan.utf8 = crate::text::utf8::validate_utf8(bytes).is_ok();
        }
        scan.nul |= lo == 0;
        if !scan.utf8 && scan.nul {
            break;
        }
        at = end;
    }
    scan
}

/// Whether `byte` ends a bare token for jq's tokenizer (`classify` in `jv_parse.c`): whitespace,
/// structure or a quote. Every other byte, a NUL included, is part of the token.
fn ends_token(byte: u8) -> bool {
    matches!(
        byte,
        b' ' | b'\t' | b'\r' | b'\n' | b'[' | b']' | b'{' | b'}' | b',' | b':' | b'"'
    )
}

/// Where jq's tokenizer stands after `bytes` of one line, from `in_string` and `escaped` before
/// them: inside a string (and right after a backslash there), and where in `bytes` the bare token
/// being read starts, `0` when it started before them.
fn token_state(bytes: &[u8], mut in_string: bool, mut escaped: bool) -> (bool, bool, usize) {
    let mut token_start = 0;
    for (at, &byte) in bytes.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
                token_start = at + 1;
            }
        } else if ends_token(byte) {
            in_string = byte == b'"';
            token_start = at + 1;
        }
    }
    (in_string, escaped, token_start)
}

/// Rewrites a number token that holds a NUL byte into the number jq's parser reads of it, padded
/// with spaces in front to the token's length (#7437, #7749), and says whether it rewrote anything.
///
/// jq's tokenizer reads a NUL into a bare token like any other byte, so `1`, a NUL and `abc` are
/// one token, and `check_literal` hands the token to its number parser as a C string, which stops
/// at the NUL: the token is the number `1` when what comes before the NUL is one jq accepts (the
/// port of `check_literal` above). Anything else is left as it is: a token jq refuses (`-`, `1e`,
/// `true`, a NUL at a token's start), and a NUL inside a string, which is a control character jq
/// refuses. The rewrite keeps every offset, so a line or a later fault is placed as jq places it.
///
/// The number goes at the token's END, the spaces before it (`[[1]1`, NUL, `]` becomes `[[1] 1]`):
/// jq ends the token at the next byte that ends one, not at the NUL, and acts on it there, so an
/// error it raises then (`Expected separator between values`, an unmatched closer) is at that
/// byte's column, and a top-level number jq never finishes (`1`, NUL, `}`) is not a document ended
/// by a space. Blanking the tail instead (#7437's first form) ended the token at the NUL.
///
/// `fgets_chunk` is jq's read of its input stream: a line is handed to the parser that many bytes
/// at a time, and a read that ends without a newline only up to its first NUL (#5654), so a NUL in
/// such a read and what follows it are never seen and nothing is blanked for it. `None` for a text
/// jq parses whole by its length (`fromjson`). `lead` is how many bytes of the first line were read
/// before `text` starts (a byte-order mark taken off it), so that line's reads fall where jq's do.
/// `line_head` is the part of `text`'s first line that came before it, in an earlier file of the
/// same stream: the string or token open at its end goes on in `text`. jq reads each file from its
/// start, so the reads of `text` start at `text`, whatever `line_head` holds. A number token that
/// starts in `line_head` is rewritten over both: its bytes in `line_head` become spaces, or part of
/// the number where the token's part in `text` is too short to hold it, and the caller writes
/// `line_head` back to the files it came from.
///
/// A JSON string holds no line break, so the scan for strings starts afresh at each line: a text
/// where one does is refused by jq at that string whatever this does. Costs one search for a NUL
/// on a text with none.
pub fn blank_nul_number_tails(
    text: &mut [u8],
    fgets_chunk: Option<usize>,
    lead: usize,
    line_head: &mut [u8],
) -> bool {
    let mut changed = false;
    let Some(first) = find_nul(text) else {
        return false;
    };
    // The scan state, at `scanned`: inside a string (and after a backslash there), and where the
    // bare token being read starts, which is in `line_head` (from `head_token`) while `head` is set.
    let (mut in_string, mut escaped, head_token) = token_state(line_head, false, false);
    let mut head = head_token < line_head.len();
    let (mut scanned, mut token_start, mut line_start) = (0, 0, 0);
    let mut nul = first;
    loop {
        // A line break since the scan stopped starts a new line, read from its start.
        if let Some(last) = text[scanned..nul].iter().rposition(|&b| b == b'\n') {
            line_start = scanned + last + 1;
            (scanned, in_string, escaped, token_start, head) =
                (line_start, false, false, line_start, false);
        }
        let (string, escape, start) = token_state(&text[scanned..nul], in_string, escaped);
        if start > 0 || string != in_string {
            token_start = scanned + start;
            head = false;
        }
        (in_string, escaped, scanned) = (string, escape, nul);
        // A NUL in a string is a byte of the string: the search goes on after it.
        let search = if in_string {
            nul + 1
        } else {
            let token_end = text[nul..]
                .iter()
                .position(|&b| ends_token(b))
                .map_or(text.len(), |at| nul + at);
            let seen = fgets_chunk.map_or(true, |chunk| {
                let lead = if line_start == 0 { lead } else { 0 };
                let read_end = line_start + ((nul - line_start + lead) / chunk + 1) * chunk - lead;
                text[nul..]
                    .iter()
                    .position(|&b| b == b'\n')
                    .is_some_and(|at| nul + at < read_end)
            });
            // The first NUL of the token decides it: one later in it is part of the same tail.
            let number = |token: &[u8]| !token.is_empty() && literal(token).is_ok();
            // A NUL jq never reads (`seen` false) rewrites nothing.
            if seen && !head {
                if number(&text[token_start..token_end]) {
                    right_align_number(&mut text[token_start..token_end], nul - token_start);
                    changed = true;
                }
            } else if seen {
                let mut token = [&line_head[head_token..], &text[..token_end]].concat();
                if number(&token) {
                    let digits = line_head.len() - head_token + nul;
                    right_align_number(&mut token, digits);
                    let (in_head, in_text) = token.split_at(line_head.len() - head_token);
                    line_head[head_token..].copy_from_slice(in_head);
                    text[..token_end].copy_from_slice(in_text);
                    changed = true;
                }
            }
            // The token's bytes up to its end are bare bytes of the same token.
            (scanned, token_start, head) = (token_end, token_end, false);
            token_end.max(nul + 1)
        };
        match find_nul(&text[search..]) {
            Some(found) => nul = search + found,
            None => return changed,
        }
    }
}

/// A bare token whose first `digits` bytes are the number jq reads of it, rewritten as that number at
/// the token's end with spaces before it ([`blank_nul_number_tails`]).
fn right_align_number(token: &mut [u8], digits: usize) {
    let pad = token.len() - digits;
    token.copy_within(..digits, pad);
    token[..pad].fill(b' ');
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
        at = check_escape(token, at)?;
    }
    Ok(())
}

/// [`check_string`]'s check of the escape whose backslash is just before `token[at]`: where the
/// escape ends, or jq's fault. The one definition of which escapes jq's parser accepts, for the
/// parser and for [`jq_refused_escape_offset`] (#7758).
fn check_escape(token: &[u8], mut at: usize) -> Result<usize, Stop> {
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
    Ok(at)
}

/// The byte of `text` at which the first escape jq's parser refuses starts (its backslash), or
/// `None` when it refuses none (#7758): an unknown escape, a short or non-hex `\u`, and a high
/// surrogate with no low one after it. A lone low surrogate is accepted, as jq reads it as U+FFFD.
///
/// For a route whose own check accepts a document jq refuses for an escape alone: serde's
/// `IgnoredAny` reads a `\u` escape without looking at the code it names, and a key that does
/// not decode falls back to its source span (#1642), so neither raises. The text is the documents
/// of a stream as a whole; a backslash is only ever in a string there, and the escapes are read
/// from each string's first one on, so an escaped backslash is never read as the start of one.
/// Escapes at the very end of the text are left to the route's own handling of an unfinished
/// string.
///
/// An escape jq accepts is passed over without the parser model: a one-character escape, and a
/// `\u` of four hex digits that is not a high surrogate; only the rest ([`check_escape`])
/// decide. The backslashes are found eight bytes at a time.
#[doc(hidden)]
pub fn jq_refused_escape_offset(text: &[u8]) -> Option<usize> {
    if !crate::util::simd::escape::contains_byte(text, b'\\') {
        return None;
    }
    let mut from = 0;
    while let Some(found) = find_backslash(&text[from..]) {
        let backslash = from + found;
        let escape = *text.get(backslash + 1)?;
        from = match escape {
            b'\\' | b'"' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => backslash + 2,
            b'u' if text.get(backslash + 2..backslash + 6).is_some_and(|hex| {
                hex.iter().all(u8::is_ascii_hexdigit) && !is_high_surrogate(hex)
            }) =>
            {
                backslash + 6
            }
            _ => match check_escape(text, backslash + 1) {
                Ok(end) => end,
                Err(_) => return Some(backslash),
            },
        };
    }
    None
}

/// Whether four hex digits name a high surrogate (U+D800..U+DBFF).
fn is_high_surrogate(hex: &[u8]) -> bool {
    matches!(hex[0], b'd' | b'D') && matches!(hex[1], b'8' | b'9' | b'a' | b'b' | b'A' | b'B')
}

/// The offset of the first backslash of `hay`, read a word of eight bytes at a time: a byte that is
/// a backslash is a zero byte of the word XORed with backslashes, and the lowest byte the zero-byte
/// test flags is always a true zero.
fn find_backslash(hay: &[u8]) -> Option<usize> {
    const ONES: u64 = u64::from_le_bytes([1; 8]);
    const HIGHS: u64 = u64::from_le_bytes([0x80; 8]);
    const BACKSLASHES: u64 = u64::from_le_bytes([b'\\'; 8]);
    let mut at = 0;
    while let Some(word) = hay.get(at..at + 8) {
        let word = u64::from_le_bytes([
            word[0], word[1], word[2], word[3], word[4], word[5], word[6], word[7],
        ]) ^ BACKSLASHES;
        let zeros = word.wrapping_sub(ONES) & !word & HIGHS;
        if zeros != 0 {
            return Some(at + (zeros.trailing_zeros() / 8) as usize);
        }
        at += 8;
    }
    hay[at..]
        .iter()
        .position(|&byte| byte == b'\\')
        .map(|found| at + found)
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
            multi: false,
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
        if self.emitted == 2 && !self.multi {
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
/// A text that opens with part of the BOM and departs from it is `Malformed BOM` before the
/// parser runs, as in `jv_parser_set_buf` (#5804); the stream routes ([`jq_stream_fault`]) do not
/// make that check.
///
/// `text` is parsed whole, NULs included: jq's parser takes a length, not a C string.
pub(crate) fn jq_parse_message(text: &[u8]) -> Option<String> {
    if malformed_bom(text) {
        return Some(String::from("Malformed BOM"));
    }
    jq_parse_fault(text, 1, 0, JQ_MAX_PARSING_DEPTH).map(|fault| fault.message)
}

/// Whether `text` opens with part of the UTF-8 BOM and then departs from it (#5804).
///
/// `jv_parser_set_buf` matches the first bytes against `EF BB BF` one at a time; a byte that
/// matches advances, and the first mismatch after at least one match is `Malformed BOM`. A whole
/// BOM is skipped, and a text that does not start with `EF` is not a BOM at all. The mismatching
/// byte has to be there: a text that ends inside the prefix is left to the parser.
fn malformed_bom(text: &[u8]) -> bool {
    let matched = text
        .iter()
        .zip(UTF8_BOM)
        .take_while(|(byte, bom)| byte == bom)
        .count();
    0 < matched && matched < UTF8_BOM.len() && matched < text.len()
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

/// Whether a `[` or `{` of `text` opens with `limit` entries already on jq's parse stack, an
/// array holding one and an object two (#5497): the count [`Parser::open`] refuses at, kept
/// for the brackets alone. A container can only open as a value, so every object above it has
/// its key read, and jq never refuses the key itself.
///
/// It reads strings with [`string_literal_end`](crate::json::light::string_literal_end), the
/// splitter's own scanner, and gives up (`false`) on one that scanner rejects: the document is
/// malformed there, and its first fault in jq's order is for the route's own handling, not for
/// this check.
fn stack_reaches(text: &[u8], limit: usize) -> bool {
    let mut weights: Vec<u8> = Vec::new();
    let mut entries = 0usize;
    let mut at = 0;
    while let Some(&byte) = text.get(at) {
        match byte {
            b'"' => match crate::json::light::string_literal_end(text, at) {
                Some(end) => {
                    at = end;
                    continue;
                }
                None => return false,
            },
            b'[' | b'{' => {
                if entries >= limit {
                    return true;
                }
                let weight = if byte == b'[' { 1 } else { 2 };
                entries += usize::from(weight);
                weights.push(weight);
            }
            b']' | b'}' => entries -= weights.pop().map_or(0, usize::from),
            _ => {}
        }
        at += 1;
    }
    false
}

/// jq's message when its parser, reading `chunks` in order from `line` and `column` of its input
/// as one stream of top-level values, stops on a fault, or `None` when it reads every value
/// (#5555).
///
/// For a route that has already found its own reason to refuse a document and now has to word it
/// the way jq's `main` prints it: `jq: parse error: <message>`. The fault is the first in the
/// stream, so a caller passes the text from where its own check stopped, or all of it.
///
/// A byte-order mark is a fault like any other byte: jq skips the one at the front of its input,
/// so the caller has stripped it (`strip_leading_json_bom`) before the first chunk.
#[doc(hidden)]
pub fn jq_stream_fault<B: AsRef<[u8]>>(
    chunks: impl IntoIterator<Item = B>,
    line: usize,
    column: usize,
) -> Option<String> {
    jq_stream_fault_end(chunks, line, column).map(|(message, _)| message)
}

/// [`jq_stream_fault`], and the `(line, column)` jq's parser stands on when it stops (#5923).
///
/// jq keeps those counters across a fault: it drops the rest of the read that held the fault, and
/// the bytes it drops are never scanned, so a second fault is placed from where the first left the
/// parser, not from where the second sits in the text. The same position is the one the message
/// names, because the message is built when the parser stops. `None` when the parser reads every
/// value.
#[doc(hidden)]
pub fn jq_stream_fault_end<B: AsRef<[u8]>>(
    chunks: impl IntoIterator<Item = B>,
    line: usize,
    column: usize,
) -> Option<(String, (usize, usize))> {
    let mut parser = Parser::new(line, column, JQ_MAX_PARSING_DEPTH);
    parser.multi = true;
    for chunk in chunks {
        for &byte in chunk.as_ref() {
            if let Err(Stop::Fault(fault)) = parser.feed(byte) {
                return Some((
                    format!("{fault} at line {}, column {}", parser.line, parser.column),
                    (parser.line, parser.column),
                ));
            }
        }
    }
    match parser.finish() {
        Err(Stop::Fault(fault)) => Some((
            format!(
                "{fault} at EOF at line {}, column {}",
                parser.line, parser.column
            ),
            (parser.line, parser.column),
        )),
        _ => None,
    }
}

/// jq's parser over its input stream, fed a byte at a time, for a reader that only needs to know at
/// which bytes it refuses the stream (#7575). After a refusal it starts afresh, as jq's parser is
/// reset after one; the caller drops what jq's reader drops with it.
#[doc(hidden)]
pub struct JqStreamFaults(Parser);

impl JqStreamFaults {
    /// A parser at the front of the stream.
    pub fn new() -> Self {
        let mut parser = Parser::new(1, 0, JQ_MAX_PARSING_DEPTH);
        parser.multi = true;
        Self(parser)
    }

    /// Whether jq's parser refuses the stream at `byte`. A BOM jq skips is never fed.
    pub fn feed(&mut self, byte: u8) -> bool {
        match self.0.feed(byte) {
            Err(Stop::Fault(_)) => {
                *self = Self::new();
                true
            }
            _ => false,
        }
    }
}

impl Default for JqStreamFaults {
    fn default() -> Self {
        Self::new()
    }
}

/// jq's parser reading `chunks` from `(line, column)` without reaching the end of the text (#7024): the
/// first fault it meets in them (worded as [`jq_stream_fault`] words it), or whether it stands at a
/// boundary between top-level values once they are read (no container, string, token or value
/// pending), where a parser started afresh would go on exactly as this one does.
///
/// For a reader that words a stream's first fault without holding the stream: what it has read up to
/// such a boundary leaves nothing in jq's parser but its position.
#[doc(hidden)]
pub fn jq_stream_prefix_fault<B: AsRef<[u8]>>(
    chunks: impl IntoIterator<Item = B>,
    line: usize,
    column: usize,
) -> Result<bool, String> {
    let mut parser = Parser::new(line, column, JQ_MAX_PARSING_DEPTH);
    parser.multi = true;
    for chunk in chunks {
        for &byte in chunk.as_ref() {
            if let Err(Stop::Fault(fault)) = parser.feed(byte) {
                return Err(format!(
                    "{fault} at line {}, column {}",
                    parser.line, parser.column
                ));
            }
        }
    }
    Ok(parser.string_state == 0
        && parser.token.is_empty()
        && parser.stack.is_empty()
        && parser.next.is_none())
}

/// The index of the chunk jq's parser is reading when it meets the first fault of the stream the
/// `chunks` make, and whether that fault is at the end of the text (`... at EOF`), or `None` when
/// it reads every value (#6485). A fault at the end is the last chunk's, since the parser is
/// waiting for more where they end.
///
/// For a caller that must say which input file a fault is in: jq's reader has opened every file up
/// to that one, and none after it unless the fault is at the end of the text.
#[doc(hidden)]
pub fn jq_stream_fault_chunk<B: AsRef<[u8]>>(
    chunks: impl IntoIterator<Item = B>,
) -> Option<(usize, bool)> {
    let mut parser = Parser::new(1, 0, JQ_MAX_PARSING_DEPTH);
    parser.multi = true;
    let mut count = 0;
    for chunk in chunks {
        count += 1;
        for &byte in chunk.as_ref() {
            if let Err(Stop::Fault(_)) = parser.feed(byte) {
                return Some((count - 1, false));
            }
        }
    }
    match parser.finish() {
        Err(Stop::Fault(_)) => count.checked_sub(1).map(|last| (last, true)),
        _ => None,
    }
}

/// Whether `message` is jq's parse-depth refusal, which jq's `main` prints as a parse error.
#[doc(hidden)]
pub fn is_jq_depth_fault(message: &str) -> bool {
    message.starts_with(DEPTH_FAULT)
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

/// The byte of `text` at which jq's parser, reading it as [`jq_stream_fault_end`] reads it, meets
/// the first fault in the stream of top-level values `text` holds, whatever the fault (#7470,
/// #7611): the bracket it refuses for its depth limit, the delimiter after a bad token, a string's
/// closing quote, a stray byte. `None` when it reads to the end without one, which leaves only a
/// fault at the end of the text (`... at EOF`).
///
/// It reads only as far as the first fault, so a caller can learn where jq stops in a long text
/// for the cost of the bytes before that point. The parser reads a byte at a time and never looks
/// ahead, so the bytes after the one returned change neither the fault nor anything before it.
#[doc(hidden)]
pub fn jq_stream_fault_offset(text: &[u8]) -> Option<usize> {
    let mut parser = Parser::new(1, 0, JQ_MAX_PARSING_DEPTH);
    parser.multi = true;
    for (at, &byte) in text.iter().enumerate() {
        if let Err(Stop::Fault(_)) = parser.feed(byte) {
            return Some(at);
        }
    }
    None
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
/// Two cheaper reads come first, so the parser runs only on a document it will refuse. A `[` or
/// `{` is refused once [`JQ_MAX_PARSING_DEPTH`] entries are open, and no container adds more
/// than two, so the refused bracket is at least the 129th nested one. The BP has one open per
/// container and one per leaf, so its excess is at most the container depth plus one, and a BP
/// that never rises above 128 cannot hold a refused bracket. That costs one popcount per 64
/// parentheses ([`BalancedParens::max_excess_exceeds`](crate::trees::BalancedParens::max_excess_exceeds)).
/// A document that does rise above 128 is walked for jq's stack alone ([`stack_reaches`]), its
/// strings skipped whole, which is several times faster than the parser's byte-at-a-time
/// tokenizing, and only a document that walk refuses is parsed, for jq's message.
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
    if !stack_reaches(text, JQ_MAX_PARSING_DEPTH + wrapper) {
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

    /// #7024: the prefix reader stops at the first fault, worded as the whole-stream reader words it,
    /// and otherwise says whether it ends between top-level values.
    #[test]
    fn the_prefix_reader_words_the_fault_and_knows_a_boundary_7024() {
        let at = |text: &str| jq_stream_prefix_fault([text.as_bytes()], 1, 0);
        assert_eq!(at("{\"a\":1} [2]"), Ok(true));
        assert_eq!(at("{\"a\":1}\n"), Ok(true));
        assert_eq!(at("\"s\""), Ok(true));
        // A bare token waits for the byte that ends it, and an open container or string for the rest.
        assert_eq!(at("12"), Ok(false));
        assert_eq!(at("12 "), Ok(true));
        assert_eq!(at("[1,"), Ok(false));
        assert_eq!(at("\"ab"), Ok(false));
        // A fault is the whole-stream reader's, at the same place; the end of the text is not one.
        for text in ["[1 2]", "{\"a\" 1}\n", "1\n2]", "tru "] {
            assert_eq!(
                at(text).err(),
                jq_stream_fault([text.as_bytes()], 1, 0),
                "{text:?}"
            );
        }
        assert_eq!(at("{\"a\":"), Ok(false));
        assert!(jq_stream_fault([b"{\"a\":"], 1, 0).is_some());
        // The position it starts from is the one the message names from.
        assert_eq!(
            jq_stream_prefix_fault([b"]"], 7, 3),
            Err(String::from("Unmatched ']' at line 7, column 4"))
        );
    }

    /// #7024: a document the strict validator accepts (its depth capped at 127, since an object costs
    /// jq's parser two slots and it refuses at 256) is one jq's parser reads without a fault. The
    /// windowed reader of a pipe certifies the documents it drops by this, and words the stream's
    /// first fault from the bytes as read only where the validator turns one down.
    #[test]
    fn what_the_strict_validator_accepts_jq_reads_7024() {
        use crate::json::validate::Validator;
        let seeds: &[&[u8]] = &[
            br#"{"a":[1,2.5e-3,-0,true,false,null],"b":{"c":"d\u00e9\ud83d\ude00\n\"\\\/"}}"#,
            br#"[[],{},"",0,1E+2,0.0]"#,
            br#""\ud800\udc00""#,
            b"123",
            b"-1.5",
            "\"\u{fffd}\u{e9}\"".as_bytes(),
        ];
        let alphabet: &[u8] =
            b"{}[]:,\"\\/ubnrtfe+-.0123456789adEtrulsx \t\r\n\x00\x01\x1f\x7f\xc3\xa9\xef\xbf\xbd\xff";
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let accepted = |doc: &[u8]| Validator::new(doc).with_max_depth(127).validate().is_ok();
        let check = |doc: &[u8]| {
            if accepted(doc) {
                let text = [doc, b"\n"].concat();
                assert_eq!(
                    jq_stream_fault([text.as_slice()], 1, 0),
                    None,
                    "{:?}",
                    String::from_utf8_lossy(doc)
                );
            }
        };
        let mut kept = 0;
        for round in 0..60_000 {
            let mut doc = seeds[round % seeds.len()].to_vec();
            for _ in 0..=(next() % 4) {
                let at = (next() as usize) % (doc.len() + 1);
                let byte = alphabet[(next() as usize) % alphabet.len()];
                match next() % 3 {
                    0 => doc.insert(at, byte),
                    1 if at < doc.len() => {
                        doc.remove(at);
                    }
                    _ if at < doc.len() => doc[at] = byte,
                    _ => doc.push(byte),
                }
            }
            kept += usize::from(accepted(&doc));
            check(&doc);
        }
        // The mutations keep enough documents valid for the check to mean something.
        assert!(kept > 1_000, "{kept}");
        // Depth: 127 is the cap, at which 127 nested objects (254 slots) still parse.
        for depth in [126, 127, 128, 200] {
            let objects = format!("{}1{}", "{\"k\":".repeat(depth), "}".repeat(depth));
            let arrays = format!("{}{}", "[".repeat(depth), "]".repeat(depth));
            check(objects.as_bytes());
            check(arrays.as_bytes());
            assert_eq!(accepted(objects.as_bytes()), depth <= 127, "{depth}");
        }
    }

    /// #5804: a text that opens with part of the BOM and then departs from it is `Malformed BOM`
    /// (every U+F000..=U+FFFF character starts `EF`, and U+FEC0 is `EF BB 80`); a whole BOM is
    /// skipped and any other first byte is not a BOM. Rows captured from `/usr/bin/jq` 1.7.1.
    #[test]
    fn a_partial_bom_is_malformed_5804() {
        for text in ["\u{fffd}", "\u{fffd}e", "\u{f000}", "\u{ffff}1", "\u{fec0}"] {
            assert_eq!(message(text).as_deref(), Some("Malformed BOM"), "{text:?}");
        }
        // Not a BOM prefix: `EE 80 80`, a digit, and a whole BOM (skipped by the caller, so the
        // second one is an ordinary fault).
        for text in ["\u{e000}1", "1", "\u{feff}\u{feff}"] {
            assert_ne!(message(text).as_deref(), Some("Malformed BOM"), "{text:?}");
        }
        // A text that ends inside the prefix is the parser's.
        assert!(!malformed_bom(b"\xEF"));
        assert!(!malformed_bom(b"\xEF\xBB"));
        assert!(!malformed_bom(b""));
        assert!(!malformed_bom(b"\xEF\xBB\xBF1"));
        assert!(malformed_bom(b"\xEF\xBB1"));
    }

    fn message(text: &str) -> Option<String> {
        jq_parse_message(text.as_bytes())
    }

    /// #5555: a stream of values is clean until a fault, which is worded and placed as jq's
    /// `main` places it across files read as one stream. Rows captured from `/usr/bin/jq` 1.7.1.
    #[test]
    fn names_the_first_fault_of_a_stream_5555() {
        let fault = |chunks: &[&str], line, column| {
            jq_stream_fault(chunks.iter().map(|c| c.as_bytes()), line, column)
        };
        assert_eq!(fault(&[], 1, 0), None);
        assert_eq!(fault(&[" \n"], 1, 0), None);
        assert_eq!(fault(&["1 2 [3] {\"a\":4}\n"], 1, 0), None);
        assert_eq!(
            fault(&["1 [2,"], 1, 0).as_deref(),
            Some("Unfinished JSON term at EOF at line 1, column 5")
        );
        // Files are one stream: lines run on, and a value can span the seam.
        assert_eq!(
            fault(&["1\n2\n", "3\n{\"a\":1,}\n"], 1, 0).as_deref(),
            Some("Expected another key-value pair at line 4, column 8")
        );
        assert_eq!(fault(&["[1,", "2]"], 1, 0), None);
        // A text that starts part-way in is placed by the position it is given.
        assert_eq!(
            fault(&["}"], 3, 4).as_deref(),
            Some("Unmatched '}' at line 3, column 5")
        );
        // The newline after a bare token is read: the fault is on the next line.
        assert_eq!(
            fault(&["ul\n"], 1, 0).as_deref(),
            Some("Invalid numeric literal at line 2, column 0")
        );
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

    /// #7758: the first escape jq's parser refuses, at its backslash, by the rules the parser itself
    /// applies to a string ([`check_escape`]): every shape the hunt captured from jq 1.7.1 and 1.8.2.
    #[test]
    fn refused_escape_offset_is_where_the_parser_refuses_an_escape_7758() {
        for (text, want) in [
            (r#""\ud83d""#, Some(1)),
            (r#""\uD83D""#, Some(1)),
            (r#""\udc00""#, None),
            (r#""\udc00\udc01""#, None),
            (r#""\ud83d\ude00""#, None),
            (r#""\uD83D\uDE00""#, None),
            (r#""\ud83dx""#, Some(1)),
            (r#""\uD83D\u0041""#, Some(1)),
            (r#""\ud83d\n""#, Some(1)),
            (r#""\udc00\ud83d""#, Some(7)),
            (r#""\ud83d\ud83d\ude00""#, Some(1)),
            (r#""ab\ud83d""#, Some(3)),
            (r#"{"\ud800":1}"#, Some(2)),
            (r#"[{"a":["\ud83d"]}]"#, Some(8)),
            (r#"1 "ok" ["\x"]"#, Some(9)),
            (r#"{"\u12":1}"#, Some(2)),
            (r#""\u12zz""#, Some(1)),
            // An escaped backslash is no start of an escape; one at the very end is left alone.
            (r#""\\ud83d" "\\x""#, None),
            (r#""a\"b\/\b\f\n\r\t""#, None),
            ("\"\\", None),
            ("[1, 2, \"no escapes\"]", None),
        ] {
            assert_eq!(jq_refused_escape_offset(text.as_bytes()), want, "{text}");
            // The parser refuses the same texts, at the string the escape is in.
            if want.is_some() {
                let fault = jq_stream_fault_offset(text.as_bytes());
                assert!(
                    fault.is_some_and(|at| at >= want.unwrap()),
                    "{text}: {fault:?}"
                );
            } else if !text.ends_with('\\') {
                assert_eq!(jq_stream_fault([text], 1, 0), None, "{text}");
            }
        }
    }

    /// #7758: the escape search with its fast paths and word-at-a-time scan answers as the plain
    /// walk it replaced (every backslash through [`check_escape`]) on texts dense with backslashes,
    /// escaped backslashes before a `u`, surrogate halves in both cases at every position, and
    /// escapes cut short at the end of the text.
    #[test]
    fn refused_escape_offset_agrees_with_the_walk_it_replaced_7758() {
        fn walk(text: &[u8]) -> Option<usize> {
            let mut from = 0;
            while let Some(found) = text[from..].iter().position(|&byte| byte == b'\\') {
                let backslash = from + found;
                if backslash + 1 == text.len() {
                    return None;
                }
                match check_escape(text, backslash + 1) {
                    Ok(end) => from = end,
                    Err(_) => return Some(backslash),
                }
            }
            None
        }
        const PIECES: &[&[u8]] = &[
            b"\\",
            b"\\\\",
            b"\\u",
            b"\\ud83d",
            b"\\uD83D",
            b"\\ude00",
            b"\\uDE00",
            b"\\udbff",
            b"\\udc00",
            b"\\u0041",
            b"\\u00e9",
            b"\\ud8",
            b"\\uzz12",
            b"\\n",
            b"\\\"",
            b"\\/",
            b"\\x",
            b"u",
            b"d83d",
            b"\"",
            b"a",
            "\u{e9}".as_bytes(),
            b" ",
            b"0",
        ];
        let mut state: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = |bound: usize| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) as usize % bound
        };
        let mut text = Vec::new();
        for _ in 0..200_000 {
            text.clear();
            for _ in 0..next(24) {
                text.extend_from_slice(PIECES[next(PIECES.len())]);
            }
            // Every cut of the text, so an escape is cut short at the end of the text and the
            // word-at-a-time scan meets its last backslash at every offset in a word.
            let cut = next(text.len() + 1);
            assert_eq!(
                jq_refused_escape_offset(&text[..cut]),
                walk(&text[..cut]),
                "{:?}",
                String::from_utf8_lossy(&text[..cut])
            );
        }
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

    /// The stream form reads every top-level value, places a depth refusal where the single-text
    /// form does, and stops there: what follows the refused bracket is never read (#7470). A
    /// fault of any other kind is placed on the byte the parser meets it at (#7611).
    #[test]
    fn stream_fault_offset_is_the_first_fault_of_the_stream_7470() {
        let deep = nest("[", "", "]", 300);
        assert_eq!(jq_stream_fault_offset(deep.as_bytes()), Some(256));
        let after = format!("1 2 {{\"a\":3}} {deep} 4");
        assert_eq!(jq_stream_fault_offset(after.as_bytes()), Some(12 + 256));
        // A fault of another kind first is that fault's byte.
        let syntax_first = format!("1 ] {deep}");
        assert_eq!(jq_stream_fault_offset(syntax_first.as_bytes()), Some(2));
        assert_eq!(jq_stream_fault_offset(b"[1,,2]"), Some(3));
        // A bad token at the delimiter that ends it, a bad string at its closing quote.
        assert_eq!(jq_stream_fault_offset(b"[x[x[x"), Some(2));
        assert_eq!(jq_stream_fault_offset(b"1 truex 2"), Some(7));
        assert_eq!(jq_stream_fault_offset(b"[\"a\x01b\"]"), Some(5));
        // None before the end: a clean stream, or one only the end leaves unfinished.
        assert_eq!(jq_stream_fault_offset(b"1 2 [3]"), None);
        assert_eq!(jq_stream_fault_offset(b"1 [2"), None);
        // What follows the refused bracket is never read, malformed or not.
        let open = format!("{}\u{1}]]", "{\"a\":".repeat(129));
        assert_eq!(jq_stream_fault_offset(open.as_bytes()), Some(128 * 5));
        // It agrees with the message `jq_stream_fault_end` gives for the same text.
        for text in [deep.as_str(), after.as_str(), open.as_str()] {
            let (message, _) = jq_stream_fault_end([text], 1, 0).unwrap();
            assert!(is_jq_depth_fault(&message), "{message}");
        }
    }

    /// The bracket walk refuses exactly where the parser does: on every boundary shape above and
    /// on random mixed nestings, with brackets inside strings and escaped quotes (#5497).
    #[test]
    fn stack_reaches_agrees_with_the_parser_5497() {
        let parser_refuses = |text: &str| {
            jq_parse_fault(text.as_bytes(), 1, 0, JQ_MAX_PARSING_DEPTH).is_some_and(|f| f.depth)
        };
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..400 {
            let depth = 100 + (next() % 200) as usize;
            let (mut open, mut close) = (String::new(), Vec::new());
            for _ in 0..depth {
                if next() % 2 == 0 {
                    open.push_str(["[", "[1,\"]\",", "[\"[{\\\"\","][(next() % 3) as usize]);
                    close.push("]");
                } else {
                    open.push_str(["{\"a\":", "{\"k\":{},\"a\":"][(next() % 2) as usize]);
                    close.push("}");
                }
            }
            close.reverse();
            let text = format!("{open}1{}", close.concat());
            let head = &text[..text.len().min(120)];
            assert_eq!(
                stack_reaches(text.as_bytes(), JQ_MAX_PARSING_DEPTH),
                parser_refuses(&text),
                "{head}"
            );
        }
        assert!(!stack_reaches(b"[\"unterminated", JQ_MAX_PARSING_DEPTH));
    }

    /// #6485: the chunk a fault is read in, and whether it is the end of the text. A value that
    /// spans two chunks is one value, and a fault in the second is the second's.
    #[test]
    fn a_fault_is_placed_in_the_chunk_the_parser_was_reading_6485() {
        let place = |chunks: &[&str]| jq_stream_fault_chunk(chunks.iter().map(|c| c.as_bytes()));
        assert_eq!(place(&[]), None);
        assert_eq!(place(&["1\n", "2\n"]), None);
        assert_eq!(place(&["[1,}", "2"]), Some((0, false)));
        assert_eq!(place(&["1", "[1,}", "2"]), Some((1, false)));
        // Text that ends inside a value is the last chunk's, whichever chunk opened it.
        assert_eq!(place(&["[1,", "2"]), Some((1, true)));
        assert_eq!(place(&["1 [", ""]), Some((1, true)));
        // A value that spans the chunks: the fault is where the bytes are.
        assert_eq!(place(&["[1,", "}"]), Some((1, false)));
        // The parse-depth limit is a fault like any other.
        let deep = "[".repeat(JQ_MAX_PARSING_DEPTH + 1);
        assert_eq!(place(&["1", &deep]), Some((1, false)));
    }

    /// #7437: what [`blank_nul_number_tails`] makes of a text, `None` for one it leaves as it is.
    fn blanked(text: &[u8], chunk: Option<usize>, lead: usize, head: &[u8]) -> Option<Vec<u8>> {
        blanked_with_head(text, chunk, lead, head).map(|(text, _)| text)
    }

    /// [`blanked`], with what the rewrite made of the line head too (#7749).
    fn blanked_with_head(
        text: &[u8],
        chunk: Option<usize>,
        lead: usize,
        head: &[u8],
    ) -> Option<(Vec<u8>, Vec<u8>)> {
        let (mut text, mut head) = (text.to_vec(), head.to_vec());
        blank_nul_number_tails(&mut text, chunk, lead, &mut head).then_some((text, head))
    }

    /// What the scan must answer: the whole text's UTF-8 verdict and whether [`find_nul`] finds one.
    fn scan_reference(text: &[u8]) -> TextScan {
        TextScan {
            utf8: core::str::from_utf8(text).is_ok(),
            nul: find_nul(text).is_some(),
        }
    }

    #[test]
    fn the_scan_answers_as_a_validation_and_a_nul_search_of_the_whole_text_do_7437() {
        // Characters of one to four bytes, so that pieces end inside each.
        let chars: [&[u8]; 6] = [
            b"a",
            b"{\"k\":1}",
            "\u{e9}".as_bytes(),
            "\u{20ac}".as_bytes(),
            "\u{1f600}".as_bytes(),
            b" \n",
        ];
        let mut seed: u32 = 7437;
        let mut next = move |n: usize| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (seed >> 8) as usize % n
        };
        for round in 0..400 {
            let mut text = Vec::new();
            while text.len() < 40 + round % 60 {
                text.extend_from_slice(chars[next(chars.len())]);
            }
            for piece in [1, 2, 3, 4, 5, 7, 16, 32, 64] {
                assert_eq!(
                    scan_utf8_and_nul_in(&text, piece),
                    scan_reference(&text),
                    "{text:?} piece {piece}"
                );
                // A NUL, and a byte that breaks the UTF-8, at every offset.
                for at in 0..text.len() {
                    for bad in [0x00, 0x80, 0xC3, 0xF0, 0xFF] {
                        let mut one = text.clone();
                        one[at] = bad;
                        assert_eq!(
                            scan_utf8_and_nul_in(&one, piece),
                            scan_reference(&one),
                            "{one:?} piece {piece}"
                        );
                    }
                }
            }
        }
        // The real piece size: a NUL either side of each boundary, and a four-byte character across
        // it, both alone and with a fault after it.
        let len = 3 * SCAN_PIECE + 64;
        let plain: Vec<u8> = (0..len).map(|i| b"0123456789,[]{} \n"[i % 17]).collect();
        assert_eq!(
            scan_utf8_and_nul(&plain),
            TextScan {
                utf8: true,
                nul: false
            }
        );
        for boundary in [SCAN_PIECE, 2 * SCAN_PIECE, 3 * SCAN_PIECE] {
            for at in boundary - 4..boundary + 4 {
                let mut nul = plain.clone();
                nul[at] = 0;
                assert_eq!(
                    scan_utf8_and_nul(&nul),
                    TextScan {
                        utf8: true,
                        nul: true
                    },
                    "NUL at {at}"
                );
                let mut wide = plain.clone();
                wide[at..at + 4].copy_from_slice("\u{1f600}".as_bytes());
                assert_eq!(
                    scan_utf8_and_nul(&wide),
                    scan_reference(&wide),
                    "character at {at}"
                );
                wide[len - 1] = 0xFF;
                assert_eq!(
                    scan_utf8_and_nul(&wide),
                    TextScan {
                        utf8: false,
                        nul: false
                    },
                    "fault after {at}"
                );
            }
        }
        assert_eq!(
            scan_utf8_and_nul(b""),
            TextScan {
                utf8: true,
                nul: false
            }
        );
    }

    #[test]
    fn a_nul_tail_of_a_number_token_is_blanked_and_nothing_else_7437() {
        // #7749: the number goes at the token's end, where jq's tokenizer ends the token.
        let cases: &[(&[u8], Option<&[u8]>)] = &[
            (b"[1\0x,2]\n", Some(b"[  1,2]\n")),
            (b"1\0\0\0\n", Some(b"   1\n")),
            (b"{\"a\":12\0\"b\"}\n", Some(b"{\"a\": 12\"b\"}\n")),
            (b"nan\0 1.5\0e3\n", Some(b" nan    1.5\n")),
            // Every NUL of a token after the first is part of the same tail.
            (b"1\0a\0b 2\0\n", Some(b"    1  2\n")),
            // Before a closer: jq ends the token at the closer (#7749).
            (b"[[1]1\0]\n", Some(b"[[1] 1]\n")),
            (b"1\0}\n", Some(b" 1}\n")),
            // Refused tokens, a NUL at a token's start and one in a string stay.
            (b"[true\0,-\0,1e\0,\0,nu\0]\n", None),
            (b"[\"1\0\",\"a\\\"1\0\"]\n", None),
            // A string closed on the line before the NUL does not hide it.
            (b"[\"a\\\\\",1\0]\n", Some(b"[\"a\\\\\", 1]\n")),
            // A line break ends a string jq would refuse, and the next line is read afresh.
            (b"\"a\n1\0\n", Some(b"\"a\n 1\n")),
            // No newline after the NUL in its read: jq never sees it.
            (b"[1\0,2]", None),
        ];
        for (text, want) in cases {
            assert_eq!(
                blanked(text, Some(4095), 0, &[]).as_deref(),
                *want,
                "{text:?}"
            );
        }
        // `fromjson` reads its text whole, newline or not.
        assert_eq!(
            blanked(b"[1\0,2]", None, 0, &[]).as_deref(),
            Some(&b"[ 1,2]"[..])
        );
    }

    #[test]
    fn the_reads_of_a_line_decide_whether_a_nul_is_seen_7437() {
        let line = |spaces: usize| [vec![b' '; spaces], b"1\0x\n".to_vec()].concat();
        // The NUL at 4094 is in the first read, which ends without a newline; at 4095 it is in
        // the second, which holds it.
        assert_eq!(blanked(&line(4093), Some(4095), 0, &[]), None);
        assert!(blanked(&line(4094), Some(4095), 0, &[]).is_some());
        // A lead of three bytes read before the text moves the read boundary three bytes in.
        assert!(blanked(&line(4091), Some(4095), 3, &[]).is_some());
        assert_eq!(blanked(&line(4090), Some(4095), 3, &[]), None);
        // The lead is the first line's only: a later line's reads start at its own start.
        let second = [b"\n".to_vec(), line(4094)].concat();
        assert!(blanked(&second, Some(4095), 3, &[]).is_some());
    }

    #[test]
    fn a_token_or_string_open_in_the_line_head_goes_on_7437() {
        // The token started in an earlier file: `1` there and `2`, a NUL, `x` here are `12`, which
        // goes at the token's end, over both (#7749).
        assert_eq!(
            blanked_with_head(b"2\0x]\n", Some(4095), 0, b"[1"),
            Some((b" 12]\n".to_vec(), b"[ ".to_vec()))
        );
        // A part here too short for the number: some of it stays in the head.
        assert_eq!(
            blanked_with_head(b"\0]\n", Some(4095), 0, b"[123"),
            Some((b"3]\n".to_vec(), b"[ 12".to_vec()))
        );
        // `e` there and `5` here are `e5`, which jq refuses.
        assert_eq!(blanked(b"5\0x]\n", Some(4095), 0, b"[e"), None);
        // A string open there holds the NUL.
        assert_eq!(blanked(b"1\0\"]\n", Some(4095), 0, b"[\"a"), None);
        // One closed in the head does not.
        assert!(blanked(b"1\0]\n", Some(4095), 0, b"[\"a\",").is_some());
    }
}
