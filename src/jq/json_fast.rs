//! A one-pass reader for the JSON that `fromjson` is almost always handed (#5675).
//!
//! The general route (`validate_jq_lenient`, a semi-index, then `to_owned`) is three passes and
//! half a dozen allocations for a 90-byte record, 1.75 us a call against jq's 0.9. This reads text
//! that is plain RFC 8259 straight into an [`OwnedValue`] and answers `None` for everything else,
//! so the general route still decides every case it did: the leniencies jq has (`nan`,
//! leading zeros, `Infinity`), a repeated key, a surrogate escape, a depth near the limit, and
//! every error and its message.
//!
//! What it accepts it builds exactly as `to_owned` does: a number goes through
//! [`OwnedValue::from_number_literal`] from its own spelling, an object through
//! [`ObjectBuilder`](super::eval_generic::ObjectBuilder), a string as the decoded text. The
//! differential tests at the foot of this file hold the two to that over generated documents. The
//! grammar is written twice (here, and in `json::validate` with `to_owned`), so a change to what
//! the general route accepts or builds has to be mirrored here or declined here.
//!
//! `tonumber` reaches this only through its `is_ok()` probe on the error path.
//!
//! The stream loaders read through it too (#6110): `--slurpfile` and the lazy `inputs` producer
//! hand it a document or a whole stream and fall back to their own routes when it declines.

use alloc::string::String;
use alloc::vec::Vec;

use super::eval::JqSemantics;
use super::eval_generic::{ObjectBuilder, SiblingShape};
use super::value::{ObjectMap, OwnedValue, SharableString};

/// Deeper than this the general route decides (it enforces the document nesting ceilings and
/// words the refusal).
///
/// This is the validator's own ceiling ([`crate::json::validate::MAX_NESTING_DEPTH`], which the
/// assertion below holds it to), not a smaller one (#6614). The walk tries this reader on every
/// child of a container (`plain_owned_jq`), and a subtree it declines for depth has cost the whole
/// descent to the limit before the walk tries that node's own children, one level shallower each
/// time. At 64 that made a tree of 40 nodes nested in a chain (80 levels) read 2x slower than one
/// of 30 (60 levels): `[.[] | recurse(empty)]` over 200,000 nodes, 0.09 s at 30 and 0.19 s from 40
/// (Apple M5). The recursion costs 128 frames of native stack, which every caller has: the
/// evaluation thread has 256 MB and a library caller's 2 MiB holds it with room to spare.
pub(crate) const MAX_DEPTH: usize = 128;

// The reader must never accept what a ceiling of the general route refuses.
const _: () = assert!(MAX_DEPTH <= crate::json::validate::MAX_NESTING_DEPTH);
const _: () = assert!(MAX_DEPTH <= super::eval_generic::MAX_NESTING_DEPTH);

/// `text` as an [`OwnedValue`] if it is one plain JSON value, optionally surrounded by JSON
/// whitespace; `None` when it is anything else, for the caller's general route to decide.
pub fn parse_plain(text: &str) -> Option<OwnedValue> {
    parse_plain_sized(text, 0)
}

/// [`parse_plain`], with room for `capacity` members if the value is an object (#7260): a stream
/// of records sizes each document's map from the ones before it, as an array sizes its elements'
/// ([`SiblingShape`], #5594), instead of growing every record's map from empty.
pub(crate) fn parse_plain_sized(text: &str, capacity: usize) -> Option<OwnedValue> {
    let mut reader = Reader {
        text: Some(text),
        bytes: text.as_bytes(),
        at: 0,
        nodes: 0,
    };
    reader.space();
    let value = reader.value_sized(0, capacity, None)?;
    reader.space();
    (reader.at == reader.bytes.len()).then_some(value)
}

/// Every value of a stream of plain JSON values, or `None` if any one is anything else (#6110).
///
/// `text` is the stream as `--slurpfile` and `inputs` read it, its values separated and surrounded
/// by JSON white space. A `None` leaves the whole text to the caller's general route.
///
/// A value is taken only when white space or the end of the text follows it, so `[1][2]`,
/// `1[2]` and `"a""b"` (which a stream parser may split) are the general route's too.
pub fn parse_plain_stream(text: &str) -> Option<Vec<OwnedValue>> {
    let mut reader = Reader {
        text: Some(text),
        bytes: text.as_bytes(),
        at: 0,
        nodes: 0,
    };
    let mut values: Vec<OwnedValue> = Vec::new();
    // A stream of records is an array's elements to the sizing and the key sharing (#7260).
    let mut siblings = SiblingShape::default();
    loop {
        reader.space();
        if reader.at == reader.bytes.len() {
            return Some(values);
        }
        let capacity = siblings.capacity();
        let value = reader.value_sized(0, capacity, values.last_mut())?;
        siblings.push(object_len(&value), capacity);
        values.push(value);
        if let Some(next) = reader.bytes.get(reader.at) {
            if !matches!(next, b' ' | b'\t' | b'\n' | b'\r') {
                return None;
            }
        }
    }
}

/// The plain JSON value that starts at `bytes[start]`, with whatever follows it left alone, and
/// how many nodes the semi-index holds under it (values and object keys, not the value itself);
/// `None` when it is anything else, for the caller's general route to decide (#6125). `bytes` is a
/// document's text, so a string is checked for UTF-8 as it is cut.
///
/// The count lets the caller hold the read to the index: a text the index nests differently (a
/// stray quote earlier in a malformed document flips what is a string) is the walk's to judge.
pub(crate) fn parse_plain_at(bytes: &[u8], start: usize) -> Option<(OwnedValue, usize)> {
    let mut reader = Reader {
        text: None,
        bytes,
        at: start,
        nodes: 0,
    };
    let value = reader.value(0)?;
    Some((value, reader.nodes - 1))
}

/// How deep [`plain_value_end`] follows a value; deeper is the caller's general route's.
///
/// Below every ceiling a parser the stream loaders cross-check against has (serde's 128 levels,
/// [`MAX_DEPTH`]), so a value it accepts is one each of them reads whole.
pub(crate) const PLAIN_END_MAX_DEPTH: usize = 64;

/// Where the plain RFC 8259 value at `bytes[start]` ends, if it is one: checked as the reader
/// checks it, and built into nothing (#7260). `None` for anything else, a value nested deeper
/// than [`PLAIN_END_MAX_DEPTH`] and a string holding a lone surrogate escape included, for the
/// caller's general route to decide; a high half followed by a low one is read as the reader reads
/// it (#7758).
///
/// What follows the value is not looked at. The lazy `inputs` scan finds a document and checks it
/// with this one pass, where it ran the splitter and then serde over the same text.
///
/// `text` is a `&str` because the walk checks no UTF-8: a string's run of ordinary bytes is skipped
/// whole, so bytes that are not UTF-8 would pass as a plain value (#7406). The caller's text has
/// been checked already, and the type keeps a byte-slice caller from handing it unchecked bytes.
pub(crate) fn plain_value_end(text: &str, start: usize) -> Option<usize> {
    let mut skip = Skip {
        bytes: text.as_bytes(),
        at: start,
    };
    skip.value(0)?;
    Some(skip.at)
}

/// Whether the four hex digits of a `\u` escape name a surrogate half: `Some(true)` for a high
/// one (U+D800..U+DBFF), `Some(false)` for a low one (U+DC00..U+DFFF), `None` for any other code.
fn surrogate_half(hex: &[u8]) -> Option<bool> {
    if !matches!(hex[0], b'd' | b'D') {
        return None;
    }
    match hex[1] {
        b'8' | b'9' | b'a' | b'b' | b'A' | b'B' => Some(true),
        b'c'..=b'f' | b'C'..=b'F' => Some(false),
        _ => None,
    }
}

/// [`plain_value_end`]'s walk: the grammar [`Reader`] reads, without building the value.
struct Skip<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Skip<'_> {
    #[inline]
    fn space(&mut self) {
        while matches!(self.bytes.get(self.at), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn value(&mut self, depth: usize) -> Option<()> {
        match *self.bytes.get(self.at)? {
            b'{' => self.object(depth),
            b'[' => self.array(depth),
            b'"' => self.string(),
            b't' => self.word(b"true"),
            b'f' => self.word(b"false"),
            b'n' => self.word(b"null"),
            b'-' | b'0'..=b'9' => self.number(),
            _ => None,
        }
    }

    fn word(&mut self, word: &[u8]) -> Option<()> {
        let end = self.at + word.len();
        (self.bytes.get(self.at..end)? == word).then(|| self.at = end)
    }

    fn digits(&mut self) -> usize {
        let start = self.at;
        while matches!(self.bytes.get(self.at), Some(b'0'..=b'9')) {
            self.at += 1;
        }
        self.at - start
    }

    /// `-? (0 | [1-9][0-9]*) (\.[0-9]+)? ([eE][+-]?[0-9]+)?`
    fn number(&mut self) -> Option<()> {
        if self.bytes.get(self.at) == Some(&b'-') {
            self.at += 1;
        }
        match *self.bytes.get(self.at)? {
            b'0' => self.at += 1,
            b'1'..=b'9' => {
                self.digits();
            }
            _ => return None,
        }
        if self.bytes.get(self.at) == Some(&b'.') {
            self.at += 1;
            if self.digits() == 0 {
                return None;
            }
        }
        if matches!(self.bytes.get(self.at), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.bytes.get(self.at), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if self.digits() == 0 {
                return None;
            }
        }
        Some(())
    }

    fn string(&mut self) -> Option<()> {
        self.at += 1;
        loop {
            // The run of ordinary bytes up to the next quote, backslash or control byte.
            let run = self.bytes.get(self.at..)?;
            self.at += run
                .iter()
                .position(|&b| b == b'"' || b == b'\\' || b < 0x20)
                .unwrap_or(run.len());
            match *self.bytes.get(self.at)? {
                b'"' => {
                    self.at += 1;
                    return Some(());
                }
                b'\\' => match *self.bytes.get(self.at + 1)? {
                    b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => self.at += 2,
                    b'u' => {
                        let hex = self.bytes.get(self.at + 2..self.at + 6)?;
                        if !hex.iter().all(u8::is_ascii_hexdigit) {
                            return None;
                        }
                        if matches!(hex[0], b'd' | b'D') {
                            self.d_escape()?;
                        } else {
                            self.at += 6;
                        }
                    }
                    _ => return None,
                },
                // The run stopped at a control byte.
                _ => return None,
            }
        }
    }

    /// The `\u` escape at `self.at` whose code starts with `d`, its hex digits checked: a surrogate
    /// half is read as the reader reads it (#7758), a high half followed by a low one as one
    /// character and a lone half of either kind left to the general route. Out of line, so the
    /// string walk of text with no such escape keeps its shape.
    #[cold]
    #[inline(never)]
    fn d_escape(&mut self) -> Option<()> {
        let hex = self.bytes.get(self.at + 2..self.at + 6)?;
        match surrogate_half(hex) {
            None => self.at += 6,
            Some(true) => {
                let low = self.bytes.get(self.at + 6..self.at + 12)?;
                if low[..2] != *b"\\u"
                    || !low[2..].iter().all(u8::is_ascii_hexdigit)
                    || surrogate_half(&low[2..]) != Some(false)
                {
                    return None;
                }
                self.at += 12;
            }
            Some(false) => return None,
        }
        Some(())
    }

    fn array(&mut self, depth: usize) -> Option<()> {
        if depth >= PLAIN_END_MAX_DEPTH {
            return None;
        }
        self.at += 1;
        self.space();
        if self.bytes.get(self.at) == Some(&b']') {
            self.at += 1;
            return Some(());
        }
        loop {
            self.space();
            self.value(depth + 1)?;
            self.space();
            match *self.bytes.get(self.at)? {
                b',' => self.at += 1,
                b']' => {
                    self.at += 1;
                    return Some(());
                }
                _ => return None,
            }
        }
    }

    fn object(&mut self, depth: usize) -> Option<()> {
        if depth >= PLAIN_END_MAX_DEPTH {
            return None;
        }
        self.at += 1;
        self.space();
        if self.bytes.get(self.at) == Some(&b'}') {
            self.at += 1;
            return Some(());
        }
        loop {
            self.space();
            if self.bytes.get(self.at) != Some(&b'"') {
                return None;
            }
            self.string()?;
            self.space();
            if self.bytes.get(self.at) != Some(&b':') {
                return None;
            }
            self.at += 1;
            self.space();
            self.value(depth + 1)?;
            self.space();
            match *self.bytes.get(self.at)? {
                b',' => self.at += 1,
                b'}' => {
                    self.at += 1;
                    return Some(());
                }
                _ => return None,
            }
        }
    }
}

/// How many members `value` has if it is an object, for [`SiblingShape`].
#[inline]
fn object_len(value: &OwnedValue) -> Option<usize> {
    match value {
        OwnedValue::Object(map) => Some(map.len()),
        _ => None,
    }
}

struct Reader<'a> {
    /// The text as a `&str` when the caller has one (slices are then cut on the checked
    /// boundaries); `None` for bytes of a document, whose slices are validated as they are cut.
    text: Option<&'a str>,
    bytes: &'a [u8],
    at: usize,
    /// The values and keys read so far: the nodes a semi-index holds for them.
    nodes: usize,
}

impl<'a> Reader<'a> {
    /// `bytes[from..to]` as text; `None` where it is not (a split character, or invalid UTF-8
    /// in a document read from bytes).
    #[inline]
    fn slice(&self, from: usize, to: usize) -> Option<&'a str> {
        match self.text {
            Some(text) => text.get(from..to),
            None => core::str::from_utf8(self.bytes.get(from..to)?).ok(),
        }
    }

    #[inline]
    fn space(&mut self) {
        while matches!(self.bytes.get(self.at), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    #[inline]
    fn eat(&mut self, byte: u8) -> Option<()> {
        (*self.bytes.get(self.at)? == byte).then(|| self.at += 1)
    }

    fn value(&mut self, depth: usize) -> Option<OwnedValue> {
        self.value_sized(depth, 0, None)
    }

    /// [`value`](Self::value), with room for `capacity` members if it is an object (the
    /// walk's sibling sizing, #5594), and sharing the keys it repeats of `prev`, the value read
    /// just before it in the same place (the previous element, or the same member of the
    /// previous record), when both are objects (#7260).
    fn value_sized(
        &mut self,
        depth: usize,
        capacity: usize,
        prev: Option<&mut OwnedValue>,
    ) -> Option<OwnedValue> {
        self.nodes += 1;
        if depth >= MAX_DEPTH {
            return None;
        }
        match *self.bytes.get(self.at)? {
            b'{' => {
                let prev = match prev {
                    Some(OwnedValue::Object(map)) => Some(map),
                    _ => None,
                };
                self.object(depth, capacity, prev)
            }
            b'[' => self.array(depth),
            b'"' => self.string().map(|s| OwnedValue::String(s.into())),
            b't' => self.literal(b"true", OwnedValue::Bool(true)),
            b'f' => self.literal(b"false", OwnedValue::Bool(false)),
            b'n' => self.literal(b"null", OwnedValue::Null),
            b'-' | b'0'..=b'9' => self.number(),
            _ => None,
        }
    }

    /// The key at `self.at` when it is the text of `prev`'s key at `pos`, as a handle sharing
    /// that key's storage, with the reader moved past it; `None` (the reader where it was)
    /// otherwise, for [`string`](Self::string) to read (#7260).
    ///
    /// Byte equality with the decoded key is the decoded equality only for a key with no quote,
    /// backslash or control byte, whose text is the key itself between quotes; any other key
    /// takes the plain read. A record keeps its keys' order far more often than not, so the
    /// previous record's key at the same position is the one to try.
    fn sibling_key(&mut self, prev: &mut ObjectMap, pos: usize) -> Option<SharableString> {
        let close = {
            let key = prev.key_at(pos)?.as_bytes();
            if key.iter().any(|&b| b == b'"' || b == b'\\' || b < 0x20) {
                return None;
            }
            let open = self.at + 1;
            let close = open + key.len();
            if self.bytes.get(open..close)? != key || self.bytes.get(close) != Some(&b'"') {
                return None;
            }
            close
        };
        let key = prev.share_key(pos)?;
        self.at = close + 1;
        Some(key)
    }

    /// The keyword, and not a longer token that starts with it (`nullx`, `true1`).
    fn literal(&mut self, word: &[u8], value: OwnedValue) -> Option<OwnedValue> {
        let end = self.at + word.len();
        if self.bytes.get(self.at..end)? != word {
            return None;
        }
        if self
            .bytes
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
            self.bytes.get(end),
            Some(b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E')
        ) {
            end += 1;
        }
        let token = &self.bytes[start..end];
        // Strict RFC 8259 only: jq's leniencies (`007`, `+1`, `.5`) are the general route's.
        if !crate::json::validate::is_valid_number(token) {
            return None;
        }
        self.at = end;
        Some(OwnedValue::from_number_literal::<JqSemantics>(
            self.slice(start, end)?,
        ))
    }

    /// A string with no escape but the single-character ones and `\u` of a non-surrogate; a raw
    /// control byte or anything else is the general route's.
    fn string(&mut self) -> Option<String> {
        self.at += 1;
        let start = self.at;
        // The common string has no escape: one scan for the closing quote.
        let mut at = start;
        loop {
            match *self.bytes.get(at)? {
                b'"' => {
                    self.at = at + 1;
                    return self.slice(start, at).map(String::from);
                }
                b'\\' => break,
                0..=0x1f => return None,
                _ => at += 1,
            }
        }
        let mut out = String::from(self.slice(start, at)?);
        loop {
            match *self.bytes.get(at)? {
                b'"' => {
                    self.at = at + 1;
                    return Some(out);
                }
                b'\\' => {
                    let escaped = match *self.bytes.get(at + 1)? {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let hex = self.slice(at + 2, at + 6)?;
                            if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                                return None;
                            }
                            let unit = u32::from_str_radix(hex, 16).ok()?;
                            if (0xd800..0xdc00).contains(&unit) {
                                // A high half must be followed by a low half; a lone one is the
                                // general route's (it words the refusal).
                                if self.bytes.get(at + 6..at + 8)? != b"\\u" {
                                    return None;
                                }
                                let low_hex = self.slice(at + 8, at + 12)?;
                                if !low_hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                                    return None;
                                }
                                let low = u32::from_str_radix(low_hex, 16).ok()?;
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
                            // A lone low half is not a character: char::from_u32 declines it.
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
                    // A run of ordinary bytes, copied whole (the text is UTF-8, and a quote or a
                    // backslash is never a continuation byte).
                    let run = at;
                    while matches!(self.bytes.get(at), Some(b) if *b != b'"' && *b != b'\\' && *b >= 0x20)
                    {
                        at += 1;
                    }
                    out.push_str(self.slice(run, at)?);
                }
            }
        }
    }

    fn array(&mut self, depth: usize) -> Option<OwnedValue> {
        self.at += 1;
        self.space();
        let mut items = Vec::new();
        if self.bytes.get(self.at) == Some(&b']') {
            self.at += 1;
            return Some(OwnedValue::array_from(items));
        }
        let mut siblings = SiblingShape::default();
        loop {
            self.space();
            let capacity = siblings.capacity();
            let item = self.value_sized(depth + 1, capacity, items.last_mut())?;
            siblings.push(object_len(&item), capacity);
            items.push(item);
            self.space();
            match *self.bytes.get(self.at)? {
                b',' => self.at += 1,
                b']' => {
                    self.at += 1;
                    return Some(OwnedValue::array_from(items));
                }
                _ => return None,
            }
        }
    }

    fn object(
        &mut self,
        depth: usize,
        capacity: usize,
        mut prev: Option<&mut ObjectMap>,
    ) -> Option<OwnedValue> {
        self.at += 1;
        self.space();
        let mut members = ObjectBuilder::<OwnedValue>::with_capacity(capacity);
        if self.bytes.get(self.at) == Some(&b'}') {
            self.at += 1;
            return Some(OwnedValue::Object(members.finish()));
        }
        let mut count = 0usize;
        loop {
            self.space();
            if *self.bytes.get(self.at)? != b'"' {
                return None;
            }
            // The same key at the same position of the previous record: share its storage.
            let shared = match prev.as_deref_mut() {
                Some(prev) => self.sibling_key(prev, count),
                None => None,
            };
            let repeats = shared.is_some();
            let key = match shared {
                Some(key) => key,
                None => SharableString::from(self.string()?),
            };
            self.nodes += 1;
            self.space();
            self.eat(b':')?;
            self.space();
            // Its value's own sibling is the previous record's value for the same key.
            let value = match prev.as_deref_mut() {
                Some(prev) if repeats => {
                    let sibling = prev.get_index_mut(count).map(|(_, v)| v);
                    self.value_sized(depth + 1, 0, sibling)?
                }
                _ => self.value(depth + 1)?,
            };
            members.push_shared::<JqSemantics>(key, value);
            count += 1;
            self.space();
            match *self.bytes.get(self.at)? {
                b',' => self.at += 1,
                b'}' => {
                    self.at += 1;
                    // A repeated key collapsed: jq's rule for it is the general route's.
                    return (members.map().len() == count)
                        .then(|| OwnedValue::Object(members.finish()));
                }
                _ => return None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jq::eval::parse_complete_json_indexed;

    /// The two routes agree: the reader either declines, or builds what the indexed route builds.
    fn agrees(text: &str) -> bool {
        match parse_plain(text) {
            None => true,
            Some(fast) => match parse_complete_json_indexed(text.as_bytes()) {
                Ok(slow) => fast.to_json() == slow.to_json() && fast == slow,
                Err(_) => false,
            },
        }
    }

    /// The lazy scan checks a document against serde's reader of nothing ([`IgnoredAny`]), and so
    /// does this.
    ///
    /// [`IgnoredAny`]: serde::de::IgnoredAny
    ///
    /// For a container, the values the lazy scan asks about: [`plain_value_end`] either declines,
    /// or ends it where serde's stream reader ends it, and it accepts every one the reader builds
    /// (#7260; surrogate pairs included since #7758). (A scalar's end is where serde would look at the
    /// byte after it, which the caller decides.)
    #[cfg(feature = "serde")]
    fn end_agrees(text: &str) -> bool {
        let bytes = text.as_bytes();
        let start = bytes
            .iter()
            .take_while(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
            .count();
        if !matches!(bytes.get(start), Some(b'{' | b'[')) {
            return true;
        }
        let end = plain_value_end(text, start);
        let mut stream =
            serde_json::Deserializer::from_str(&text[start..]).into_iter::<serde::de::IgnoredAny>();
        let serde_end = matches!(stream.next(), Some(Ok(_))).then(|| start + stream.byte_offset());
        if end.is_some() && end != serde_end {
            return false;
        }
        if parse_plain(text).is_some() {
            return end == Some(text.trim_end_matches([' ', '\t', '\n', '\r']).len());
        }
        true
    }

    /// The keys of a record, for the sharing tests below.
    fn keys_of(value: &OwnedValue) -> Vec<&SharableString> {
        let map = value.as_object().expect("an object");
        (0..map.len())
            .map(|i| map.key_at(i).expect("a key"))
            .collect()
    }

    /// #7260: a record repeating the previous record's key at the same position shares that key's
    /// storage (one string for the key of every record), and so does a nested record's key; a
    /// key the previous record has elsewhere, or one whose text holds an escape, is its own. The
    /// values are what the indexed route builds either way.
    #[test]
    fn records_share_the_keys_they_repeat_7260() {
        let text = r#"[{"id":1,"name":"a","at":{"x":1}},{"id":2,"name":"b","at":{"x":2}},
            {"name":"c","id":3},{"id":4,"n\u0061me":"d"},{"id":5,"name":"e"}]"#;
        assert!(agrees(text));
        let value = parse_plain(text).expect("plain");
        let items = value.as_array().expect("an array");
        let (first, second) = (keys_of(&items[0]), keys_of(&items[1]));
        assert!(first[0].ptr_eq(second[0]) && first[1].ptr_eq(second[1]));
        let nested = |i: usize| keys_of(&items[i].as_object().unwrap()["at"])[0].clone();
        assert!(nested(0).ptr_eq(&nested(1)), "a nested record's key");
        // Reordered: neither key is at the position it had.
        let third = keys_of(&items[2]);
        assert!(!third[0].is_shared() && !third[1].is_shared());
        // An escaped key reads through the plain path; the next record's plain `"name"` is its
        // decoded text, so that one shares it.
        let (fourth, fifth) = (keys_of(&items[3]), keys_of(&items[4]));
        assert_eq!(fourth[1].as_str(), "name");
        assert!(fifth[0].ptr_eq(fourth[0]) && fifth[1].ptr_eq(fourth[1]));
        assert!(
            !fourth[1].ptr_eq(second[1]),
            "the escaped key was read, not shared"
        );
        // A stream of records shares across its documents.
        let stream = parse_plain_stream("{\"k\":1}\n{\"k\":2}\n").expect("plain");
        assert!(keys_of(&stream[0])[0].ptr_eq(keys_of(&stream[1])[0]));
        assert_eq!(stream[1].to_json(), r#"{"k":2}"#);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn plain_value_end_is_where_serde_ends_the_value_7260() {
        for text in [
            "{}",
            "[]",
            "{\"a\":1,\"b\":[true,null,{\"c\":\"d\"}]} x",
            "[1, 2.5e-3, -0, \"\\n\\u0041\"]]",
            "{\"a\":1,\"a\":2}",
            "[1,]",
            "[01]",
            "[1.]",
            "[\"\\ud83d\\ude00\"]",
            "[\"a\nb\"]",
            "[nan]",
            "{\"a\" 1}",
        ] {
            assert!(end_agrees(text), "ends differ on {text:?}");
        }
        assert_eq!(plain_value_end("[1] [2]", 0), Some(3));
        assert_eq!(plain_value_end("[1] [2]", 4), Some(7));
        assert_eq!(plain_value_end(r#"{"a":{"b":[]}}"#, 0), Some(14));
        // Deeper than the limit and a lone surrogate half of either kind are the caller's; a high
        // half followed by a low one is read, as the reader reads it (#7758), and so is a repeated
        // key the reader declines.
        let deep = |n: usize| alloc::format!("{}{}", "[".repeat(n), "]".repeat(n));
        assert!(plain_value_end(&deep(PLAIN_END_MAX_DEPTH), 0).is_some());
        assert!(plain_value_end(&deep(PLAIN_END_MAX_DEPTH + 1), 0).is_none());
        assert_eq!(plain_value_end(r#"["\ud83d\ude00"]"#, 0), Some(16));
        assert_eq!(plain_value_end(r#"["\uD83D\uDE00", "x"]"#, 0), Some(21));
        for lone in [
            r#"["\ud83d"]"#,
            r#"["\udc00"]"#,
            r#"["\ud83dx"]"#,
            r#"["\ud83d\u0041"]"#,
            r#"["\ud83d\ud83d\ude00"]"#,
            r#"["\udc00\ud83d"]"#,
            r#"{"\ud800":1}"#,
            r#"["\ud83d\ude0"]"#,
            r#"["\ud83d\"]"#,
        ] {
            assert!(plain_value_end(lone, 0).is_none(), "{lone}");
        }
        assert!(plain_value_end(r#"{"a":1,"a":2}"#, 0).is_some());
    }

    #[test]
    fn plain_documents_are_read_as_the_indexed_route_reads_them() {
        for text in [
            "null",
            "true",
            "false",
            "0",
            "-0",
            "1.5",
            "-1.50",
            "1e5",
            "1E-5",
            "100000000000000000000",
            "1.7976931348623157e308",
            "1e1000",
            "9007199254740993",
            "3.14159265358979323846",
            "\"\"",
            "\"abc\"",
            "\"a\\nb\\t\\\"\\\\\\/\"",
            "\"\\u00e9\\u0041\\u0000\"",
            "\"\\ud83d\\ude00 x \\ud83d\\ude00\"",
            "\"caf\u{e9} \u{1f600}\"",
            "[]",
            "[ ]",
            "{}",
            "{ }",
            "[1,2,3]",
            " [ 1 , 2 ] ",
            "{\"a\":1,\"b\":[true,null,{\"c\":\"d\"}]}",
            "{\"id\":7,\"name\":\"User7\",\"email\":\"user7@example.com\",\"age\":25,\"active\":true,\"score\":70.5}",
            "[[[[1]]]]",
            "\t\n\r 1 \t\n\r",
        ] {
            assert!(parse_plain(text).is_some(), "declined {text:?}");
            assert!(agrees(text), "differs on {text:?}");
        }
    }

    #[test]
    fn what_is_not_plain_is_declined() {
        for text in [
            "",
            " ",
            "nan",
            "NaN",
            "Infinity",
            "-Infinity",
            "nullx",
            "true1",
            "tru",
            "007",
            "+1",
            ".5",
            "1.",
            "1e",
            "0x10",
            "1 2",
            "[1,]",
            "[,1]",
            "[1 2]",
            "{\"a\":1,}",
            "{\"a\" 1}",
            concat!("{", "a:1}"),
            "{\"a\":1,\"a\":2}",
            "\"\\ud800\"",
            "\"\\ud83d\"",
            "\"\\ud83dx\"",
            "\"\\ud83d\\u0041\"",
            "\"\\ude00\"",
            "\"\\x\"",
            "\"\\u12\"",
            "\"a\nb\"",
            "\"unterminated",
            "'single'",
            "[1]]",
            "\u{feff}1",
            "1\u{c}",
        ] {
            assert!(parse_plain(text).is_none(), "accepted {text:?}");
        }
    }

    /// The stream form (#6110): every value, or `None` if any one is declined.
    #[test]
    fn a_stream_of_plain_values_is_read_value_by_value() {
        let one = |text: &str| parse_plain_stream(text).map(|v| v.len());
        assert_eq!(one(""), Some(0));
        assert_eq!(one(" \t\r\n "), Some(0));
        assert_eq!(one("1"), Some(1));
        assert_eq!(one("1 2\n3"), Some(3));
        assert_eq!(one("{\"a\":1}\n{\"a\":2}\n[3]\n"), Some(3));
        assert_eq!(one("\"a\" \"b\" null true []"), Some(5));
        let values = parse_plain_stream("{\"a\":1} [2] \"c\" 4.50").unwrap();
        let each: Vec<OwnedValue> = ["{\"a\":1}", "[2]", "\"c\"", "4.50"]
            .iter()
            .map(|t| parse_plain(t).unwrap())
            .collect();
        assert_eq!(values, each);
        assert_eq!(
            values.iter().map(OwnedValue::to_json).collect::<Vec<_>>(),
            each.iter().map(OwnedValue::to_json).collect::<Vec<_>>()
        );
    }

    /// A value the text does not set apart, or any one the reader declines, declines the stream:
    /// the general route reads the whole text.
    #[test]
    fn a_stream_with_one_declined_value_is_declined() {
        for text in [
            "[1][2]",
            "1[2]",
            "\"a\"\"b\"",
            "{}{}",
            "1,2",
            "[1] ]",
            "[1] nan",
            "[1] 007",
            "[1] {\"a\":1,\"a\":2}",
            "[1] \"\\ud800\"",
            "[1] [1,]",
            "[1] \"unterminated",
            "\u{feff}[1]",
            "[1]\u{c}",
        ] {
            assert!(parse_plain_stream(text).is_none(), "accepted {text:?}");
        }
        let deep = alloc::format!("{}{}", "[".repeat(MAX_DEPTH + 1), "]".repeat(MAX_DEPTH + 1));
        assert!(parse_plain_stream(&alloc::format!("1 {deep}")).is_none());
    }

    /// The bytes form (#6125): the value at an offset, whatever follows it, and a string checked
    /// for UTF-8 as it is cut.
    #[test]
    fn a_value_at_an_offset_of_bytes_is_read_and_what_follows_is_left_alone() {
        let doc = br#"  {"a":[1,"x\u00e9"],"b":null} trailing {"#;
        let (value, nodes) = parse_plain_at(doc, 2).expect("the object is plain");
        assert_eq!(nodes, 6, "two keys, `[1,\"x\"]`, its two items and `null`");
        assert_eq!(
            value,
            parse_plain(r#"{"a":[1,"x\u00e9"],"b":null}"#).unwrap()
        );
        assert_eq!(value.to_json(), r#"{"a":[1,"xé"],"b":null}"#);
        // An offset inside the text, a container among others.
        assert!(parse_plain_at(br"[[1],[2,3]]", 1).is_some());
        // What is not plain is declined, as `parse_plain` declines it.
        for bad in [
            &br#"{"a":1,"a":2}"#[..],
            br"[1,]",
            br"[007]",
            br#"["\ud800"]"#,
            b"[\"\xff\"]",
            b"[\"caf\xc3\"]",
            br"[nan]",
            br#"{"a":"#,
        ] {
            assert!(parse_plain_at(bad, 0).is_none(), "accepted {bad:?}");
        }
        // Past the end, and on a byte that starts no value.
        assert!(parse_plain_at(b"[1]", 3).is_none());
        assert!(parse_plain_at(b"[1]", 2).is_none());
    }

    /// #6614: a record tree of 40 nodes nested in a chain is 80 levels (an object and its array
    /// per node), and the walk tries this reader on each such subtree: a reader that stops at 64
    /// declines it after the whole descent and the walk pays that at every level above.
    #[test]
    fn a_chain_of_forty_nodes_is_read_in_one_pass_6614() {
        let chain = |n: usize| {
            let mut text = alloc::format!(r#"{{"id":{n},"children":[]}}"#);
            for id in (1..n).rev() {
                text = alloc::format!(r#"{{"id":{id},"children":[{text}]}}"#);
            }
            text
        };
        for n in [30, 40, 50, 63] {
            let text = chain(n);
            let (value, nodes) = parse_plain_at(text.as_bytes(), 0)
                .unwrap_or_else(|| panic!("{n} nodes ({} levels) must be read", 2 * n));
            // Per node: two keys, the `id` value, the `children` array and the child object that
            // sits in it; the root itself is not counted.
            assert_eq!(nodes, 5 * n - 1, "{n} nodes");
            assert_eq!(value.to_json(), text, "{n} nodes");
        }
        // 64 nodes is exactly the limit; 65 is one level past it.
        assert!(parse_plain_at(chain(64).as_bytes(), 0).is_some());
        assert!(parse_plain_at(chain(65).as_bytes(), 0).is_none());
    }

    /// #6614: the reader recurses once per level, and the limit is now 128. A debug build has the
    /// largest frames, so a 256 KiB thread (an eighth of the 2 MiB a library caller's spawned thread
    /// gets) reading the deepest nesting it accepts, as arrays and as objects, shows the room that
    /// is left.
    #[cfg(feature = "std")]
    #[test]
    fn the_deepest_nesting_is_read_on_a_small_stack_6614() {
        let handle = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(|| {
                // MAX_DEPTH containers, the innermost empty: a scalar inside the last one would
                // be a value at depth MAX_DEPTH, which the reader declines.
                let arrays = alloc::format!("{}{}", "[".repeat(MAX_DEPTH), "]".repeat(MAX_DEPTH));
                let objects = alloc::format!(
                    "{}{{}}{}",
                    r#"{"a":"#.repeat(MAX_DEPTH - 1),
                    "}".repeat(MAX_DEPTH - 1)
                );
                for text in [arrays, objects] {
                    let value = parse_plain(&text).expect("the deepest nesting is read");
                    assert_eq!(value.to_json(), text);
                }
            })
            .expect("spawns");
        handle.join().expect("no stack overflow");
    }

    #[test]
    fn nesting_past_the_limit_is_declined() {
        let deep = |n: usize| alloc::format!("{}{}", "[".repeat(n), "]".repeat(n));
        assert!(parse_plain(&deep(MAX_DEPTH)).is_some());
        assert!(parse_plain(&deep(MAX_DEPTH + 1)).is_none());
        assert!(parse_plain(&deep(100_000)).is_none());
    }

    /// A deterministic sweep of generated texts, valid and mangled: wherever the reader answers,
    /// the indexed route answers the same.
    #[test]
    fn generated_documents_agree_with_the_indexed_route() {
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        fn gen(next: &mut impl FnMut() -> u64, depth: usize, out: &mut String) {
            let pick = next() % if depth > 4 { 7 } else { 10 };
            match pick {
                0 => out.push_str("null"),
                1 => out.push_str(if next() % 2 == 0 { "true" } else { "false" }),
                2 => {
                    let n = [
                        "0",
                        "-0",
                        "7",
                        "-12",
                        "3.5",
                        "-0.25",
                        "1e3",
                        "2E-2",
                        "123456789012345678",
                        "1.0",
                        "10.50",
                        "9223372036854775807",
                        "9223372036854775808",
                        "1e999",
                        "007",
                        "+1",
                        ".5",
                        "1.",
                        "0x1",
                        "-",
                        "1e",
                    ];
                    out.push_str(n[(next() % n.len() as u64) as usize]);
                }
                3..=5 => {
                    let words = [
                        "",
                        "a",
                        "User7",
                        "caf\u{e9}",
                        "\u{1f600}",
                        "a\\nb",
                        "q\\\"q",
                        "\\u0041",
                        "\\u00e9",
                        "\\ud83d\\ude00",
                        "\\ud800",
                        "\\/",
                        "tab\\t",
                        "bad\\x",
                        "a b",
                    ];
                    out.push('"');
                    out.push_str(words[(next() % words.len() as u64) as usize]);
                    out.push('"');
                }
                6 => out.push_str(
                    ["nan", "NaN", "Infinity", "-Infinity", "nul"][(next() % 5) as usize],
                ),
                7 | 8 => {
                    out.push('[');
                    for i in 0..next() % 4 {
                        if i > 0 {
                            out.push(if next() % 12 == 0 { ' ' } else { ',' });
                        }
                        gen(next, depth + 1, out);
                    }
                    if next() % 15 == 0 {
                        out.push(',');
                    }
                    out.push(']');
                }
                _ => {
                    out.push('{');
                    for i in 0..next() % 4 {
                        if i > 0 {
                            out.push(',');
                        }
                        let keys = ["a", "b", "id", "name", "a", "k\\u00e9", "x y"];
                        out.push('"');
                        out.push_str(keys[(next() % keys.len() as u64) as usize]);
                        out.push('"');
                        out.push(if next() % 20 == 0 { ' ' } else { ':' });
                        gen(next, depth + 1, out);
                    }
                    out.push('}');
                }
            }
        }
        let mut accepted = 0;
        for _ in 0..20_000 {
            let mut text = String::new();
            if next() % 6 == 0 {
                text.push(' ');
            }
            gen(&mut next, 0, &mut text);
            if next() % 9 == 0 {
                text.push_str([" ", "x", ",", " 1", "\u{c}"][(next() % 5) as usize]);
            }
            if parse_plain(&text).is_some() {
                accepted += 1;
            }
            assert!(agrees(&text), "differs on {text:?}");
            #[cfg(feature = "serde")]
            assert!(end_agrees(&text), "ends differ on {text:?}");
        }
        assert!(
            accepted > 3_000,
            "the sweep must reach the reader: {accepted}"
        );
    }
}
