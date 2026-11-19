//! [`json_fast`](super::json_fast)'s reader over a byte stream read a chunk at a time (#7260).
//!
//! The reader of plain RFC 8259 text builds a document from a `&str` that holds all of it, so a
//! run that only ever wants the built tree (`--slurpfile`, a program whose first stage copies the
//! whole document) held the text and the tree together at the end of the parse: 6.9 MB of text
//! over a 45 MB tree for a 7 MB file of records. jq reads its input with `fgets` into a 4 KiB
//! buffer and never holds more of the text than that. This reads the same grammar from an
//! [`io::Read`](std::io::Read) through a window of [`CHUNK`] bytes that slides on as tokens are
//! consumed, so what the parse holds besides the tree is one window and the token being read.
//!
//! It accepts exactly what the slice reader accepts and builds what it builds (the differential
//! tests below hold the two to that over generated documents, at every window size down to one
//! byte), and it declines what that reader declines, after which the caller reads the text again
//! by its usual route: every leniency, error and message stays that route's.

use alloc::string::String;
use alloc::vec::Vec;
use std::io::{self, Read};

use super::eval::JqSemantics;
use super::eval_generic::{ObjectBuilder, SiblingShape};
use super::json_fast::MAX_DEPTH;
use super::value::OwnedValue;

/// How many bytes a read asks for: the window the parse holds, besides the token it is in.
const CHUNK: usize = 64 * 1024;

/// A sliding window over a [`Read`]: bytes are addressed by their offset in the window, which
/// grows as a token runs on past what it holds and drops what lies before the reader when the
/// reader is between tokens ([`StreamReader::compact`]), so no position the reader keeps moves
/// while a token is being read.
///
/// The window is text: each read is checked as UTF-8 as it arrives (a character split across two
/// reads waits in `pending` for the rest), so a token is cut from it as the slice reader cuts one
/// from a `&str`, with no check of its own, and a stream that is not UTF-8 declines.
struct Window<R> {
    reader: R,
    buf: String,
    /// The bytes of a character the last read split, until the next read completes it.
    pending: Vec<u8>,
    /// Where each read lands before it is checked.
    scratch: Vec<u8>,
    /// The stream offset of `buf`'s first byte.
    base: usize,
    chunk: usize,
    eof: bool,
    /// A read failed or the stream is not UTF-8: the parse declines, and the caller's route reads
    /// the file and reports it.
    failed: bool,
}

impl<R: Read> Window<R> {
    fn new(reader: R, chunk: usize) -> Self {
        Self {
            reader,
            buf: String::new(),
            pending: Vec::new(),
            scratch: alloc::vec![0; chunk],
            base: 0,
            chunk,
            eof: false,
            failed: false,
        }
    }

    /// What the window holds from offset `at` on.
    #[inline]
    fn held_from(&self, at: usize) -> &[u8] {
        self.buf.as_bytes().get(at..).unwrap_or(&[])
    }

    /// The byte at offset `at`, reading on as far as it; `None` past the end.
    #[inline]
    fn byte(&mut self, at: usize) -> Option<u8> {
        match self.buf.as_bytes().get(at) {
            Some(&b) => Some(b),
            None => self.byte_after_fill(at),
        }
    }

    #[cold]
    #[inline(never)]
    fn byte_after_fill(&mut self, at: usize) -> Option<u8> {
        self.fill_to(at + 1);
        self.buf.as_bytes().get(at).copied()
    }

    /// The text at offsets `from..to`, or `None` if the stream ends first or the range splits a
    /// character.
    #[inline]
    fn text(&mut self, from: usize, to: usize) -> Option<&str> {
        if to > self.buf.len() {
            self.fill_to(to);
        }
        self.buf.get(from..to)
    }

    /// Drops what lies before offset `at`, once it is a chunk or more, and says by how much the
    /// offsets moved (0 when nothing was dropped).
    #[inline]
    fn drop_before(&mut self, at: usize) -> usize {
        if at < self.chunk || !self.buf.is_char_boundary(at) {
            return 0;
        }
        self.buf.drain(..at);
        self.base += at;
        at
    }

    /// Reads until the window holds every offset below `end` or the stream ends.
    #[cold]
    fn fill_to(&mut self, end: usize) {
        while self.buf.len() < end && !self.eof {
            let n = loop {
                match self.reader.read(&mut self.scratch) {
                    Ok(n) => break n,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => {
                        self.failed = true;
                        break 0;
                    }
                }
            };
            if n == 0 {
                self.eof = true;
                // A character the stream ends inside of is not UTF-8.
                self.failed |= !self.pending.is_empty();
                break;
            }
            self.push_checked(n);
        }
    }

    /// Appends the `n` bytes just read to the window, checked as UTF-8.
    fn push_checked(&mut self, n: usize) {
        let mut read = &self.scratch[..n];
        // A character the last read split: complete it from the front of this one.
        while !self.pending.is_empty() && !read.is_empty() {
            self.pending.push(read[0]);
            read = &read[1..];
            match core::str::from_utf8(&self.pending) {
                Ok(c) => {
                    self.buf.push_str(c);
                    self.pending.clear();
                }
                Err(e) if e.error_len().is_some() => {
                    self.fail();
                    return;
                }
                Err(_) => {}
            }
        }
        match core::str::from_utf8(read) {
            Ok(text) => self.buf.push_str(text),
            Err(e) => {
                let (valid, rest) = read.split_at(e.valid_up_to());
                if e.error_len().is_some() {
                    self.fail();
                    return;
                }
                self.buf
                    .push_str(core::str::from_utf8(valid).expect("checked up to here"));
                self.pending.extend_from_slice(rest);
            }
        }
    }

    fn fail(&mut self) {
        self.failed = true;
        self.eof = true;
    }
}

/// Where a document read by [`parse_plain_document_from`] ends, in the terms jq's reader counts
/// lines in: what the caller needs to name the line of a diagnostic about it, once its text is
/// gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DocumentEnd {
    /// The offset of the byte on which the value completes: its closing byte for a string or a
    /// container, the byte after it for a number or a literal.
    pub(crate) completes_at: usize,
    /// The line breaks before `completes_at`.
    pub(crate) newlines_before: usize,
    /// The offset just after the last line break before `completes_at` (0 when there is none).
    pub(crate) line_start: usize,
    /// The offset of the first line break at or after `completes_at`, if any.
    pub(crate) next_newline: Option<usize>,
    /// The length of the stream.
    pub(crate) len: usize,
}

struct StreamReader<R> {
    window: Window<R>,
    at: usize,
    /// Line breaks consumed so far (they are only ever white space: a raw one in a string is
    /// declined).
    newlines: usize,
    /// The offset just after the last line break consumed.
    line_start: usize,
}

impl<R: Read> StreamReader<R> {
    fn new(reader: R, chunk: usize) -> Self {
        Self {
            window: Window::new(reader, chunk),
            at: 0,
            newlines: 0,
            line_start: 0,
        }
    }

    #[inline]
    fn peek(&mut self) -> Option<u8> {
        self.window.byte(self.at)
    }

    #[inline]
    fn get(&mut self, at: usize) -> Option<u8> {
        self.window.byte(at)
    }

    /// `from..to` as text, cut on the window's checked boundaries.
    #[inline]
    fn slice(&mut self, from: usize, to: usize) -> Option<&str> {
        self.window.text(from, to)
    }

    /// Between tokens: what lies before the reader is dropped once it is a chunk or more. Only
    /// called where no frame holds an offset but the reader's own (a value's start, the white
    /// space between top-level values); the line start is kept as a stream offset.
    #[inline]
    fn compact(&mut self) {
        self.at -= self.window.drop_before(self.at);
    }

    /// The stream offset of window offset `at`.
    #[inline]
    fn offset(&self, at: usize) -> usize {
        self.window.base + at
    }

    #[inline]
    fn space(&mut self) {
        while let Some(b) = self.peek() {
            match b {
                b' ' | b'\t' | b'\r' => self.at += 1,
                b'\n' => {
                    self.at += 1;
                    self.newlines += 1;
                    self.line_start = self.offset(self.at);
                }
                _ => break,
            }
        }
    }

    #[inline]
    fn eat(&mut self, byte: u8) -> Option<()> {
        (self.peek()? == byte).then(|| self.at += 1)
    }

    fn value(&mut self, depth: usize) -> Option<OwnedValue> {
        self.value_sized(depth, 0)
    }

    fn value_sized(&mut self, depth: usize, capacity: usize) -> Option<OwnedValue> {
        if depth >= MAX_DEPTH {
            return None;
        }
        self.compact();
        match self.peek()? {
            b'{' => self.object(depth, capacity),
            b'[' => self.array(depth),
            b'"' => self.string().map(|s| OwnedValue::String(s.into())),
            b't' => self.literal(b"true", OwnedValue::Bool(true)),
            b'f' => self.literal(b"false", OwnedValue::Bool(false)),
            b'n' => self.literal(b"null", OwnedValue::Null),
            b'-' | b'0'..=b'9' => self.number(),
            _ => None,
        }
    }

    fn literal(&mut self, word: &[u8], value: OwnedValue) -> Option<OwnedValue> {
        let end = self.at + word.len();
        if self.window.text(self.at, end)?.as_bytes() != word {
            return None;
        }
        if self
            .get(end)
            .is_some_and(|b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b',' | b']' | b'}'))
        {
            return None;
        }
        self.at = end;
        Some(value)
    }

    fn number(&mut self) -> Option<OwnedValue> {
        let start = self.at;
        let mut end = start;
        while matches!(
            self.get(end),
            Some(b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E')
        ) {
            end += 1;
        }
        let token = self.window.text(start, end)?;
        if !crate::json::validate::is_valid_number(token.as_bytes()) {
            return None;
        }
        self.at = end;
        Some(OwnedValue::from_number_literal::<JqSemantics>(
            self.slice(start, end)?,
        ))
    }

    /// [`json_fast`](super::json_fast)'s string grammar: the single-character escapes and `\u` of
    /// a non-surrogate or a surrogate pair; a raw control byte or anything else declines.
    fn string(&mut self) -> Option<String> {
        self.at += 1;
        let start = self.at;
        // The common string has no escape: one scan for the closing quote over what the window
        // holds, reading on when it runs out.
        let mut at = start;
        loop {
            let held = self.window.held_from(at);
            match held
                .iter()
                .position(|&b| b == b'"' || b == b'\\' || b < 0x20)
            {
                Some(offset) => {
                    at += offset;
                    break;
                }
                None => {
                    at += held.len();
                    self.window.fill_to(at + 1);
                    if self.window.buf.len() <= at {
                        return None;
                    }
                }
            }
        }
        match self.get(at)? {
            b'"' => {
                self.at = at + 1;
                return self.slice(start, at).map(String::from);
            }
            b'\\' => {}
            _ => return None,
        }
        let mut out = String::from(self.slice(start, at)?);
        loop {
            match self.get(at)? {
                b'"' => {
                    self.at = at + 1;
                    return Some(out);
                }
                b'\\' => {
                    let escaped = match self.get(at + 1)? {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let unit = self.hex4(at + 2)?;
                            if (0xd800..0xdc00).contains(&unit) {
                                if self.window.text(at + 6, at + 8)? != "\\u" {
                                    return None;
                                }
                                let low = self.hex4(at + 8)?;
                                if !(0xdc00..0xe000).contains(&low) {
                                    return None;
                                }
                                let c = char::from_u32(
                                    0x1_0000 + ((unit - 0xd800) << 10) + (low - 0xdc00),
                                )?;
                                out.push(c);
                                at += 12;
                                continue;
                            }
                            let c = char::from_u32(unit)?;
                            out.push(c);
                            at += 6;
                            continue;
                        }
                        _ => return None,
                    };
                    out.push(escaped);
                    at += 2;
                }
                0..=0x1f => return None,
                _ => {
                    let run = at;
                    while matches!(self.get(at), Some(b) if b != b'"' && b != b'\\' && b >= 0x20) {
                        at += 1;
                    }
                    out.push_str(self.slice(run, at)?);
                }
            }
        }
    }

    /// The four hex digits at `at` as a code unit.
    fn hex4(&mut self, at: usize) -> Option<u32> {
        let hex = self.slice(at, at + 4)?;
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        u32::from_str_radix(hex, 16).ok()
    }

    fn array(&mut self, depth: usize) -> Option<OwnedValue> {
        self.at += 1;
        self.space();
        let mut items = Vec::new();
        if self.peek() == Some(b']') {
            self.at += 1;
            return Some(OwnedValue::array_from(items));
        }
        let mut siblings = SiblingShape::default();
        loop {
            self.space();
            let capacity = siblings.capacity();
            let item = self.value_sized(depth + 1, capacity)?;
            siblings.push(
                match &item {
                    OwnedValue::Object(map) => Some(map.len()),
                    _ => None,
                },
                capacity,
            );
            items.push(item);
            self.space();
            match self.peek()? {
                b',' => self.at += 1,
                b']' => {
                    self.at += 1;
                    return Some(OwnedValue::array_from(items));
                }
                _ => return None,
            }
        }
    }

    fn object(&mut self, depth: usize, capacity: usize) -> Option<OwnedValue> {
        self.at += 1;
        self.space();
        let mut members = ObjectBuilder::<OwnedValue>::with_capacity(capacity);
        if self.peek() == Some(b'}') {
            self.at += 1;
            return Some(OwnedValue::Object(members.finish()));
        }
        let mut count = 0usize;
        loop {
            self.space();
            if self.peek()? != b'"' {
                return None;
            }
            let key = self.string()?;
            self.space();
            self.eat(b':')?;
            self.space();
            let value = self.value(depth + 1)?;
            members.push::<JqSemantics>(key, value);
            count += 1;
            self.space();
            match self.peek()? {
                b',' => self.at += 1,
                b'}' => {
                    self.at += 1;
                    return (members.map().len() == count)
                        .then(|| OwnedValue::Object(members.finish()));
                }
                _ => return None,
            }
        }
    }
}

/// [`parse_plain_stream`](super::json_fast::parse_plain_stream) over a stream read a chunk at a
/// time: every value of a stream of plain JSON values, or `None` if any one is anything else or a
/// read fails, for the caller to read the text again by its general route. The values are the
/// ones that function builds; only the room each object's map is given differs.
pub(crate) fn parse_plain_stream_from<R: Read>(reader: R) -> Option<Vec<OwnedValue>> {
    parse_plain_stream_chunked(reader, CHUNK)
}

fn parse_plain_stream_chunked<R: Read>(reader: R, chunk: usize) -> Option<Vec<OwnedValue>> {
    let mut reader = StreamReader::new(reader, chunk);
    let mut values = Vec::new();
    // The values of a stream are siblings as an array's elements are: a stream of records sizes
    // each record's map from the ones before it (#5594), instead of growing it from empty.
    let mut siblings = SiblingShape::default();
    loop {
        reader.space();
        reader.compact();
        let Some(_) = reader.peek() else {
            return (!reader.window.failed).then_some(values);
        };
        let capacity = siblings.capacity();
        let value = reader.value_sized(0, capacity)?;
        siblings.push(
            match &value {
                OwnedValue::Object(map) => Some(map.len()),
                _ => None,
            },
            capacity,
        );
        values.push(value);
        if let Some(next) = reader.peek() {
            if !matches!(next, b' ' | b'\t' | b'\n' | b'\r') {
                return None;
            }
        }
    }
}

/// One plain JSON value with nothing but JSON white space around it to the end of the stream,
/// read a chunk at a time, and where it ends ([`DocumentEnd`]); `None` for anything else (a second
/// value, a byte-order mark, a leniency, a read that fails) for the caller's general route.
pub(crate) fn parse_plain_document_from<R: Read>(reader: R) -> Option<(OwnedValue, DocumentEnd)> {
    parse_plain_document_chunked(reader, CHUNK)
}

fn parse_plain_document_chunked<R: Read>(
    reader: R,
    chunk: usize,
) -> Option<(OwnedValue, DocumentEnd)> {
    let mut reader = StreamReader::new(reader, chunk);
    reader.space();
    let value = reader.value(0)?;
    let end = reader.offset(reader.at);
    let completes_at = match value {
        OwnedValue::String(_) | OwnedValue::Array(_) | OwnedValue::Object(_) => end - 1,
        _ => end,
    };
    let (newlines_before, line_start) = (reader.newlines, reader.line_start);
    // What follows is white space to the end, and the first line break in it is where jq's read
    // that completes the value ends.
    let mut next_newline = None;
    while let Some(b) = reader.peek() {
        match b {
            b' ' | b'\t' | b'\r' => {}
            b'\n' => {
                next_newline.get_or_insert_with(|| reader.offset(reader.at));
            }
            _ => return None,
        }
        reader.at += 1;
        reader.compact();
    }
    if reader.window.failed {
        return None;
    }
    Some((
        value,
        DocumentEnd {
            completes_at,
            newlines_before,
            line_start,
            next_newline,
            len: reader.offset(reader.at),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jq::json_fast::{parse_plain, parse_plain_stream};

    /// A reader that hands out at most `step` bytes a call, so a token straddles every boundary.
    struct Trickle<'a> {
        bytes: &'a [u8],
        step: usize,
    }

    impl Read for Trickle<'_> {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            let n = self.step.min(out.len()).min(self.bytes.len());
            out[..n].copy_from_slice(&self.bytes[..n]);
            self.bytes = &self.bytes[n..];
            Ok(n)
        }
    }

    fn same(a: &Option<Vec<OwnedValue>>, b: &Option<Vec<OwnedValue>>) -> bool {
        match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(b)
                        .all(|(x, y)| x == y && x.to_json() == y.to_json())
            }
            _ => false,
        }
    }

    fn check(text: &str) {
        let expected_stream = parse_plain_stream(text);
        let expected_doc = parse_plain(text);
        for (chunk, step) in [(1, 1), (2, 1), (3, 2), (7, 5), (64, 64), (CHUNK, CHUNK)] {
            let reader = Trickle {
                bytes: text.as_bytes(),
                step,
            };
            let got = parse_plain_stream_chunked(reader, chunk);
            assert!(
                same(&got, &expected_stream),
                "stream {text:?} at {chunk}/{step}"
            );
            let reader = Trickle {
                bytes: text.as_bytes(),
                step,
            };
            let got = parse_plain_document_chunked(reader, chunk).map(|(v, _)| vec![v]);
            assert!(
                same(&got, &expected_doc.clone().map(|v| vec![v])),
                "document {text:?} at {chunk}/{step}"
            );
        }
    }

    #[test]
    fn a_stream_read_in_chunks_reads_as_the_text_reads_7260() {
        for text in [
            "",
            "   \n",
            "1",
            "-0.5e+3",
            "1 2",
            "[1,2] {\"a\":3}",
            "{\"a\":1,\"a\":2}",
            "\"caf\u{e9} \\u00e9 \\ud83d\\ude00 \\n\\t\\\"\"",
            "\"\\ud800\"",
            "\"\\udc00\"",
            "[true,false,null]",
            "nullx",
            "true1",
            "[1,]",
            "{\"a\" 1}",
            "007",
            "nan",
            "\u{feff}1",
            "[1][2]",
            "\"a\"\"b\"",
            "\"raw\ttab\"",
            "{\"k\":[{\"x\":\"y\"},{\"x\":\"z\",\"w\":[]}]}\n{}\n",
            "[\"\u{10348}\u{20ac}\"]",
        ] {
            check(text);
        }
    }

    #[test]
    fn invalid_utf8_is_declined_7260() {
        for bytes in [&b"\"\xff\""[..], b"[\"a\xc3\"]", b"\xc3\xa9"] {
            for step in [1, 2, 64] {
                let reader = Trickle { bytes, step };
                assert!(parse_plain_stream_chunked(reader, step).is_none());
                let reader = Trickle { bytes, step };
                assert!(parse_plain_document_chunked(reader, step).is_none());
            }
        }
    }

    #[test]
    fn a_read_that_fails_declines_7260() {
        struct Failing(usize);
        impl Read for Failing {
            fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
                if self.0 == 0 {
                    return Err(io::Error::other("gone"));
                }
                self.0 -= 1;
                out[0] = b' ';
                Ok(1)
            }
        }
        assert!(parse_plain_stream_chunked(Failing(3), 4).is_none());
        assert!(parse_plain_document_chunked(Failing(3), 4).is_none());
    }

    #[test]
    fn nesting_past_the_limit_is_declined_in_chunks_7260() {
        let deep = |n: usize| "[".repeat(n) + &"]".repeat(n);
        check(&deep(MAX_DEPTH));
        check(&deep(MAX_DEPTH + 1));
    }

    #[test]
    fn a_document_end_names_where_the_value_completes_7260() {
        let end = |text: &str| parse_plain_document_chunked(text.as_bytes(), 3).map(|(_, e)| e);
        let e = end("\n {\"a\":\n1}\n \n").unwrap();
        assert_eq!(
            (
                e.completes_at,
                e.newlines_before,
                e.line_start,
                e.next_newline,
                e.len
            ),
            (9, 2, 8, Some(10), 13)
        );
        let e = end("12 \n").unwrap();
        assert_eq!(
            (e.completes_at, e.newlines_before, e.next_newline),
            (2, 0, Some(3))
        );
        let e = end("12\n").unwrap();
        assert_eq!((e.completes_at, e.next_newline), (2, Some(2)));
        assert!(end("1 2").is_none());
        assert!(end("[1] x").is_none());
    }

    /// Generated documents, as `json_fast`'s own differential test generates them.
    #[test]
    fn generated_streams_read_in_chunks_as_the_text_reads_7260() {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        fn gen(next: &mut impl FnMut() -> u64, depth: usize, out: &mut String) {
            let pick = next() % if depth > 4 { 6 } else { 9 };
            match pick {
                0 => out.push_str("null"),
                1 => out.push_str(if next() % 2 == 0 { "true" } else { "false" }),
                2 => out.push_str(
                    ["0", "-1", "3.25", "1e5", "-0.0", "12345678901234567890"]
                        [(next() % 6) as usize],
                ),
                3 => out.push_str(
                    [
                        "\"\"",
                        "\"ab\"",
                        "\"\\u00e9\\n\"",
                        "\"x\\\"y\"",
                        "\"\u{e9}\"",
                    ][(next() % 5) as usize],
                ),
                4 | 5 => {
                    out.push_str([" ", "\n", "\t1", "01", "\"\\ud800\""][(next() % 5) as usize]);
                }
                6 => {
                    out.push('[');
                    for i in 0..next() % 4 {
                        if i > 0 {
                            out.push(',');
                        }
                        gen(next, depth + 1, out);
                    }
                    out.push(']');
                }
                _ => {
                    out.push('{');
                    for i in 0..next() % 4 {
                        if i > 0 {
                            out.push_str(", ");
                        }
                        out.push_str(["\"a\"", "\"b\"", "\"c\""][(next() % 3) as usize]);
                        out.push(':');
                        gen(next, depth + 1, out);
                    }
                    out.push('}');
                }
            }
        }
        for _ in 0..3000 {
            let mut text = String::new();
            for _ in 0..=next() % 3 {
                gen(&mut next, 0, &mut text);
                text.push_str([" ", "\n", ""][(next() % 3) as usize]);
            }
            check(&text);
        }
    }
}
