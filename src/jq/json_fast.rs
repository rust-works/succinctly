//! A one-pass reader for the JSON that `fromjson` and `tonumber` are almost always handed (#5675).
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
//! differential tests at the foot of this file hold the two to that over generated documents.

use alloc::string::String;
use alloc::vec::Vec;

use super::eval::JqSemantics;
use super::eval_generic::ObjectBuilder;
use super::value::OwnedValue;

/// Deeper than this the general route decides (it enforces the document nesting ceiling and
/// words the refusal).
const MAX_DEPTH: usize = 64;

/// `text` as an [`OwnedValue`] if it is one plain JSON value, optionally surrounded by JSON
/// whitespace; `None` when it is anything else, for the caller's general route to decide.
pub(crate) fn parse_plain(text: &str) -> Option<OwnedValue> {
    let mut reader = Reader {
        text,
        bytes: text.as_bytes(),
        at: 0,
    };
    reader.space();
    let value = reader.value(0)?;
    reader.space();
    (reader.at == reader.bytes.len()).then_some(value)
}

struct Reader<'a> {
    text: &'a str,
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
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
        if depth >= MAX_DEPTH {
            return None;
        }
        match *self.bytes.get(self.at)? {
            b'{' => self.object(depth),
            b'[' => self.array(depth),
            b'"' => self.string().map(|s| OwnedValue::String(s.into())),
            b't' => self.literal(b"true", OwnedValue::Bool(true)),
            b'f' => self.literal(b"false", OwnedValue::Bool(false)),
            b'n' => self.literal(b"null", OwnedValue::Null),
            b'-' | b'0'..=b'9' => self.number(),
            _ => None,
        }
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
            self.text.get(start..end)?,
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
                    return self.text.get(start..at).map(String::from);
                }
                b'\\' => break,
                0..=0x1f => return None,
                _ => at += 1,
            }
        }
        let mut out = String::from(self.text.get(start..at)?);
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
                            let hex = self.text.get(at + 2..at + 6)?;
                            if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                                return None;
                            }
                            let unit = u32::from_str_radix(hex, 16).ok()?;
                            // A surrogate half pairs with another or is refused: not decided here.
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
                    out.push_str(self.text.get(run..at)?);
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
        loop {
            self.space();
            items.push(self.value(depth + 1)?);
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

    fn object(&mut self, depth: usize) -> Option<OwnedValue> {
        self.at += 1;
        self.space();
        let mut members = ObjectBuilder::<OwnedValue>::new();
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
            let key = self.string()?;
            self.space();
            self.eat(b':')?;
            self.space();
            let value = self.value(depth + 1)?;
            members.push::<JqSemantics>(key, value);
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
            "\"\\ud83d\\ude00\"",
            "\"\\ud800\"",
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
        }
        assert!(
            accepted > 3_000,
            "the sweep must reach the reader: {accepted}"
        );
    }
}
