//! yq's `-p toml` input format (#6925): `decoder_toml.go` over go-toml v2.3.1's `unstable` parser
//! (`KeepComments`), transcoded to the YAML text yq prints for the same nodes.
//!
//! The reader is a port of go-toml's `unstable/parser.go` and `scanner.go`: it checks TOML's
//! grammar and nothing else, so a key written twice, a table opened twice and a number such as
//! `1__0` are all accepted, and its errors are the parser's own words. The decoder walks the
//! expressions as yq's does: every value is a node of the kind and tag yq gives it (a number or a
//! boolean keeps its source text, a date-time has no tag, a local date, time or date-time is an
//! error), comments gather in a list that the next value, table or array element takes as its head
//! comment, and a table or a key is written into the document with yq's own assignment
//! (`DeeplyAssign`): a plain `=` for a scalar or an array, a deep merge that appends arrays for a
//! table, and a `+=` for an array of tables, each through yq's traversal (a key is a glob, a path
//! segment that names a missing key makes it, and `[[a]]` followed by `[a.b]` writes into the
//! last element of `a`).
//!
//! The nodes are then written as yq's YAML encoder writes them (block style, two-space indent,
//! a sequence indented inside its mapping), and the run reads that text as `-p yaml`.

use succinctly::jq::{go_quote_bytes, yq_match_key, yq_parse_int64};
use succinctly::yaml::{go_yaml_explicit_key_scalar, go_yaml_key_is_simple, go_yaml_resolved_tag};

use super::input_props::{go_parse_int_error, string_scalar};

/// The most sequence elements one document builds, padding included: 2^21. yq pads an index past
/// the end with `null` nodes of some 545 bytes each and dies on the allocation well past a
/// few million; a node here is about 150 bytes, so this is some 300 MB.
const MAX_SEQUENCE_ELEMENTS: usize = 1 << 21;

/// The deepest an array, an inline table or a key's dotted path may nest. yq has no bound (Go
/// grows its stack as far as memory allows); a document this deep is refused instead of
/// overflowing the native stack.
const MAX_NESTING: usize = 400;

/// One level of indentation in the text written.
const STEP: &str = "  ";

/// `raw` as the YAML text of the document yq decodes it to, or yq's message for an input that does
/// not load (printed after `bad file '<name>': `). An input with no key (empty, or only comments)
/// holds no document, which is the empty text.
pub(super) fn transcode(raw: &[u8]) -> Result<Vec<u8>, String> {
    let mut decoder = Decoder {
        parser: Parser {
            left: raw,
            err: None,
            first: true,
        },
        cur: None,
        pending: Vec::new(),
        first_content_seen: false,
        budget: Budget::default(),
    };
    let root = decoder.decode()?;
    let Some(root) = root else {
        return Ok(Vec::new());
    };
    let mut out = Out::default();
    render_root(&mut out, &root);
    Ok(out.text.into_bytes())
}

// ---------------------------------------------------------------------------------------------
// Go formatting helpers
// ---------------------------------------------------------------------------------------------

/// `%#U` of a byte, read as the rune of that value: `U+0062 'b'`, or `U+0001` for one that does
/// not print.
fn go_sharp_u(b: u8) -> String {
    let c = char::from(b);
    let printable = (0x20..0x7F).contains(&b) || (b >= 0xA1 && b != 0xAD);
    if printable {
        format!("U+{:04X} '{c}'", u32::from(b))
    } else {
        format!("U+{:04X}", u32::from(b))
    }
}

// ---------------------------------------------------------------------------------------------
// unstable/scanner.go and the characters tables
// ---------------------------------------------------------------------------------------------

/// `characters.InvalidASCII`: the control characters other than tab, line feed and carriage
/// return, and DEL.
fn invalid_ascii(b: u8) -> bool {
    matches!(b, 0x00..=0x08 | 0x0B | 0x0C | 0x0E..=0x1F | 0x7F)
}

/// `characters.Utf8ValidNext`: the size of the next rune, or 0 when it is not valid or is an
/// ASCII control character TOML does not allow.
fn utf8_valid_next(p: &[u8]) -> usize {
    let c = p[0];
    if c < 0x80 {
        return usize::from(!invalid_ascii(c));
    }
    let size = match c {
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => return 0,
    };
    if size > p.len() {
        return 0;
    }
    match std::str::from_utf8(&p[..size]) {
        Ok(_) => size,
        Err(_) => 0,
    }
}

/// `characters.Utf8TomlValidAlreadyEscaped`: whether every rune of `p` is one TOML allows
/// (a tab is; the other control characters, DEL and invalid UTF-8 are not).
fn utf8_toml_valid(p: &[u8]) -> bool {
    let mut i = 0;
    while i < p.len() {
        let size = utf8_valid_next(&p[i..]);
        if size == 0 {
            return false;
        }
        i += size;
    }
    true
}

fn scan_follows(b: &[u8], pattern: &str) -> bool {
    b.starts_with(pattern.as_bytes())
}

fn is_unquoted_key_char(r: u8) -> bool {
    r.is_ascii_alphanumeric() || r == b'-' || r == b'_'
}

fn scan_whitespace(b: &[u8]) -> &[u8] {
    let n = b.iter().take_while(|&&c| c == b' ' || c == b'\t').count();
    &b[n..]
}

/// `scanComment`: the comment (from its `#`) and what follows. go-toml keeps the `\r` of a `\r\n`
/// in the comment, and yq writes it back as a bare `\r` that ends the line; the YAML reader that
/// runs next reads that as a line break, so the comment is returned without it.
fn scan_comment(b: &[u8]) -> Result<(&[u8], &[u8]), String> {
    let mut i = 1;
    while i < b.len() {
        if b[i] == b'\n' {
            return Ok((&b[..i], &b[i..]));
        }
        if b[i] == b'\r' {
            if i + 1 < b.len() && b[i + 1] == b'\n' {
                return Ok((&b[..i], &b[i + 1..]));
            }
            return Err(String::from("invalid character in comment"));
        }
        let size = utf8_valid_next(&b[i..]);
        if size == 0 {
            return Err(String::from("invalid character in comment"));
        }
        i += size;
    }
    Ok((b, &b[b.len()..]))
}

/// `scanLiteralString`: the token with its quotes and what follows.
fn scan_literal_string(b: &[u8]) -> Result<(&[u8], &[u8]), String> {
    let mut i = 1;
    while i < b.len() {
        match b[i] {
            b'\'' => return Ok((&b[..=i], &b[i + 1..])),
            b'\n' | b'\r' => return Err(String::from("literal strings cannot have new lines")),
            _ => {}
        }
        let size = utf8_valid_next(&b[i..]);
        if size == 0 {
            return Err(String::from("invalid character"));
        }
        i += size;
    }
    Err(String::from("unterminated literal string"))
}

/// `scanMultilineLiteralString`.
fn scan_multiline_literal_string(b: &[u8]) -> Result<(&[u8], &[u8]), String> {
    let mut i = 3;
    while i < b.len() {
        match b[i] {
            b'\'' => {
                if scan_follows(&b[i..], "'''") {
                    i += 3;
                    if i >= b.len() || b[i] != b'\'' {
                        return Ok((&b[..i], &b[i..]));
                    }
                    i += 1;
                    if i >= b.len() || b[i] != b'\'' {
                        return Ok((&b[..i], &b[i..]));
                    }
                    i += 1;
                    if i < b.len() && b[i] == b'\'' {
                        return Err(String::from("''' not allowed in multiline literal string"));
                    }
                    return Ok((&b[..i], &b[i..]));
                }
            }
            b'\r' => {
                if b.len() < i + 2 || b[i + 1] != b'\n' {
                    return Err(String::from("need a \\n after \\r"));
                }
                i += 2;
                continue;
            }
            _ => {}
        }
        let size = utf8_valid_next(&b[i..]);
        if size == 0 {
            return Err(String::from("invalid character"));
        }
        i += size;
    }
    Err(String::from(
        "multiline literal string not terminated by '''",
    ))
}

/// `scanBasicString`: the token with its quotes, whether it holds an escape, and what follows.
fn scan_basic_string(b: &[u8]) -> Result<(&[u8], bool, &[u8]), String> {
    let mut escaped = false;
    let mut i = 1;
    while i < b.len() {
        match b[i] {
            b'"' => return Ok((&b[..=i], escaped, &b[i + 1..])),
            b'\n' | b'\r' => return Err(String::from("basic strings cannot have new lines")),
            b'\\' => {
                if b.len() < i + 2 {
                    return Err(String::from("need a character after \\"));
                }
                escaped = true;
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    Err(String::from("basic string not terminated by \""))
}

/// `scanMultilineBasicString`.
fn scan_multiline_basic_string(b: &[u8]) -> Result<(&[u8], bool, &[u8]), String> {
    let mut escaped = false;
    let mut i = 3;
    while i < b.len() {
        match b[i] {
            b'"' => {
                if scan_follows(&b[i..], "\"\"\"") {
                    i += 3;
                    if i >= b.len() || b[i] != b'"' {
                        return Ok((&b[..i], escaped, &b[i..]));
                    }
                    i += 1;
                    if i >= b.len() || b[i] != b'"' {
                        return Ok((&b[..i], escaped, &b[i..]));
                    }
                    i += 1;
                    if i < b.len() && b[i] == b'"' {
                        return Err(String::from("\"\"\" not allowed in multiline basic string"));
                    }
                    return Ok((&b[..i], escaped, &b[i..]));
                }
            }
            b'\\' => {
                if b.len() < i + 2 {
                    return Err(String::from("need a character after \\"));
                }
                escaped = true;
                i += 1;
            }
            b'\r' => {
                if b.len() < i + 2 || b[i + 1] != b'\n' {
                    return Err(String::from("need a \\n after \\r"));
                }
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    Err(String::from(
        "multiline basic string not terminated by \"\"\"",
    ))
}

// ---------------------------------------------------------------------------------------------
// unstable/parser.go
// ---------------------------------------------------------------------------------------------

/// A value of the document.
#[derive(Clone, Debug)]
enum Value {
    Str(String),
    Bool(bool),
    Int(String),
    Float(String),
    DateTime(String),
    /// A local date, time or date-time, which yq refuses: the node kind's name.
    Local(&'static str),
    Array(Vec<ArrayItem>),
    InlineTable(Vec<KeyVal>),
}

#[derive(Clone, Debug)]
enum ArrayItem {
    /// The first comment of a run of comments between the elements.
    Comment(String),
    Value(Value),
}

#[derive(Clone, Debug)]
struct KeyVal {
    path: Vec<String>,
    value: Value,
    /// The comment on the same line, after the value.
    comment: Option<String>,
}

#[derive(Clone, Debug)]
enum Expr {
    Comment(String),
    KeyValue(KeyVal),
    Table(Vec<String>),
    ArrayTable(Vec<String>),
}

type Res<T> = Result<T, String>;

struct Parser<'a> {
    left: &'a [u8],
    err: Option<String>,
    first: bool,
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn expect(x: u8, b: &[u8]) -> Res<&[u8]> {
    match b.first() {
        None => Err(format!(
            "expected character {} but the document ended here",
            char::from(x)
        )),
        Some(&c) if c != x => Err(format!("expected character {}", char::from(x))),
        Some(_) => Ok(&b[1..]),
    }
}

fn parse_newline(b: &[u8]) -> Res<&[u8]> {
    match b[0] {
        b'\n' => Ok(&b[1..]),
        b'\r' => {
            if b.len() < 2 {
                return Err(String::from("windows new line expected"));
            }
            if b[1] != b'\n' {
                return Err(String::from("windows new line should be \\r\\n"));
            }
            Ok(&b[2..])
        }
        c => Err(format!("expected newline but got {}", go_sharp_u(c))),
    }
}

impl<'a> Parser<'a> {
    /// `NextExpression`: the next expression, or `None` at the end of the document or after an
    /// error (kept in `err`).
    fn next_expression(&mut self) -> Option<Expr> {
        if self.left.is_empty() || self.err.is_some() {
            return None;
        }
        loop {
            if self.left.is_empty() || self.err.is_some() {
                return None;
            }
            if !self.first {
                match parse_newline(self.left) {
                    Ok(rest) => self.left = rest,
                    Err(e) => return self.fail(e),
                }
            }
            if self.left.is_empty() {
                return None;
            }
            match Self::parse_expression(self.left) {
                Ok((expr, rest)) => {
                    self.left = rest;
                    self.first = false;
                    if expr.is_some() {
                        return expr;
                    }
                }
                Err(e) => return self.fail(e),
            }
        }
    }

    fn fail(&mut self, e: String) -> Option<Expr> {
        self.err = Some(e);
        self.left = &[];
        None
    }

    fn parse_expression(b: &'a [u8]) -> Res<(Option<Expr>, &'a [u8])> {
        let b = scan_whitespace(b);
        if b.is_empty() {
            return Ok((None, b));
        }
        if b[0] == b'#' {
            let (data, rest) = scan_comment(b)?;
            return Ok((Some(Expr::Comment(text(data))), rest));
        }
        if b[0] == b'\n' || b[0] == b'\r' {
            return Ok((None, b));
        }
        let (mut expr, b) = if b[0] == b'[' {
            Self::parse_table(b)?
        } else {
            let (kv, rest) = Self::parse_keyval(b, 0)?;
            (Expr::KeyValue(kv), rest)
        };
        let b = scan_whitespace(b);
        if !b.is_empty() && b[0] == b'#' {
            let (data, rest) = scan_comment(b)?;
            if let Expr::KeyValue(kv) = &mut expr {
                kv.comment = Some(text(data));
            }
            return Ok((Some(expr), rest));
        }
        Ok((Some(expr), b))
    }

    fn parse_table(b: &'a [u8]) -> Res<(Expr, &'a [u8])> {
        let array = b.len() > 1 && b[1] == b'[';
        let b = scan_whitespace(&b[if array { 2 } else { 1 }..]);
        let (key, b) = Self::parse_key(b)?;
        let b = scan_whitespace(b);
        let b = expect(b']', b)?;
        if array {
            let b = expect(b']', b)?;
            Ok((Expr::ArrayTable(key), b))
        } else {
            Ok((Expr::Table(key), b))
        }
    }

    fn parse_keyval(b: &'a [u8], depth: usize) -> Res<(KeyVal, &'a [u8])> {
        let (key, b) = Self::parse_key(b)?;
        let b = scan_whitespace(b);
        if b.is_empty() {
            return Err(String::from(
                "expected = after a key, but the document ends there",
            ));
        }
        let b = expect(b'=', b)?;
        let b = scan_whitespace(b);
        let (value, b) = Self::parse_val(b, depth)?;
        Ok((
            KeyVal {
                path: key,
                value,
                comment: None,
            },
            b,
        ))
    }

    fn parse_key(b: &'a [u8]) -> Res<(Vec<String>, &'a [u8])> {
        let (first, mut b) = Self::parse_simple_key(b)?;
        let mut key = vec![first];
        loop {
            b = scan_whitespace(b);
            if !b.is_empty() && b[0] == b'.' {
                b = scan_whitespace(&b[1..]);
                let (part, rest) = Self::parse_simple_key(b)?;
                b = rest;
                key.push(part);
                if key.len() > MAX_NESTING {
                    return Err(nesting_error());
                }
            } else {
                break;
            }
        }
        Ok((key, b))
    }

    fn parse_simple_key(b: &'a [u8]) -> Res<(String, &'a [u8])> {
        if b.is_empty() {
            return Err(String::from("expected key but found none"));
        }
        match b[0] {
            b'\'' => {
                let (_, v, rest) = parse_literal_string(b)?;
                Ok((v, rest))
            }
            b'"' => {
                let (_, v, rest) = parse_basic_string(b)?;
                Ok((v, rest))
            }
            c if is_unquoted_key_char(c) => {
                let n = b.iter().take_while(|&&c| is_unquoted_key_char(c)).count();
                Ok((text(&b[..n]), &b[n..]))
            }
            c => Err(format!(
                "invalid character at start of key: {}",
                char::from(c)
            )),
        }
    }

    fn parse_val(b: &'a [u8], depth: usize) -> Res<(Value, &'a [u8])> {
        if b.is_empty() {
            return Err(String::from("expected value, not eof"));
        }
        match b[0] {
            b'"' => {
                let (v, rest) = if scan_follows(b, "\"\"\"") {
                    let (_, v, rest) = parse_multiline_basic_string(b)?;
                    (v, rest)
                } else {
                    let (_, v, rest) = parse_basic_string(b)?;
                    (v, rest)
                };
                Ok((Value::Str(v), rest))
            }
            b'\'' => {
                let (v, rest) = if scan_follows(b, "'''") {
                    let (_, v, rest) = parse_multiline_literal_string(b)?;
                    (v, rest)
                } else {
                    let (_, v, rest) = parse_literal_string(b)?;
                    (v, rest)
                };
                Ok((Value::Str(v), rest))
            }
            b't' => {
                if !scan_follows(b, "true") {
                    return Err(String::from("expected 'true'"));
                }
                Ok((Value::Bool(true), &b[4..]))
            }
            b'f' => {
                if !scan_follows(b, "false") {
                    return Err(String::from("expected 'false'"));
                }
                Ok((Value::Bool(false), &b[5..]))
            }
            b'[' => Self::parse_val_array(b, depth),
            b'{' => Self::parse_inline_table(b, depth),
            _ => parse_int_or_float_or_date_time(b),
        }
    }

    fn parse_inline_table(b: &'a [u8], depth: usize) -> Res<(Value, &'a [u8])> {
        if depth >= MAX_NESTING {
            return Err(nesting_error());
        }
        let mut children = Vec::new();
        let mut first = true;
        let mut b = &b[1..];
        while !b.is_empty() {
            b = scan_whitespace(b);
            if b.is_empty() {
                return Err(String::from("inline table is incomplete"));
            }
            if b[0] == b'}' {
                break;
            }
            if !first {
                b = expect(b',', b)?;
                b = scan_whitespace(b);
            }
            let (kv, rest) = Self::parse_keyval(b, depth + 1)?;
            b = rest;
            children.push(kv);
            first = false;
        }
        let rest = expect(b'}', b)?;
        Ok((Value::InlineTable(children), rest))
    }

    fn parse_val_array(b: &'a [u8], depth: usize) -> Res<(Value, &'a [u8])> {
        if depth >= MAX_NESTING {
            return Err(nesting_error());
        }
        let mut b = &b[1..];
        let mut items = Vec::new();
        let mut first = true;
        while !b.is_empty() {
            let (comment, rest) = parse_optional_whitespace_comment_newline(b)?;
            b = rest;
            if let Some(c) = comment {
                items.push(ArrayItem::Comment(c));
            }
            if b.is_empty() {
                return Err(String::from("array is incomplete"));
            }
            if b[0] == b']' {
                break;
            }
            if b[0] == b',' {
                if first {
                    return Err(String::from("array cannot start with comma"));
                }
                b = &b[1..];
                let (comment, rest) = parse_optional_whitespace_comment_newline(b)?;
                b = rest;
                if let Some(c) = comment {
                    items.push(ArrayItem::Comment(c));
                }
            } else if !first {
                return Err(String::from("array elements must be separated by commas"));
            }
            if !b.is_empty() && b[0] == b']' {
                break;
            }
            let (value, rest) = Self::parse_val(b, depth + 1)?;
            b = rest;
            items.push(ArrayItem::Value(value));
            let (comment, rest) = parse_optional_whitespace_comment_newline(b)?;
            b = rest;
            if let Some(c) = comment {
                items.push(ArrayItem::Comment(c));
            }
            first = false;
        }
        let rest = expect(b']', b)?;
        Ok((Value::Array(items), rest))
    }
}

fn nesting_error() -> String {
    format!("the document nests arrays, inline tables or dotted keys more than {MAX_NESTING} levels deep")
}

/// `parseOptionalWhitespaceCommentNewline`: the first comment of the run and what follows it.
fn parse_optional_whitespace_comment_newline(mut b: &[u8]) -> Res<(Option<String>, &[u8])> {
    let mut root = None;
    while !b.is_empty() {
        b = scan_whitespace(b);
        if !b.is_empty() && b[0] == b'#' {
            let (data, rest) = scan_comment(b)?;
            b = rest;
            if root.is_none() {
                root = Some(text(data));
            }
        }
        if b.is_empty() {
            break;
        }
        if b[0] == b'\n' || b[0] == b'\r' {
            b = parse_newline(b)?;
        } else {
            break;
        }
    }
    Ok((root, b))
}

/// `parseLiteralString`: the token, the string and what follows.
fn parse_literal_string(b: &[u8]) -> Res<(&[u8], String, &[u8])> {
    let (token, rest) = scan_literal_string(b)?;
    Ok((token, text(&token[1..token.len() - 1]), rest))
}

/// `parseMultilineLiteralString`.
fn parse_multiline_literal_string(b: &[u8]) -> Res<(&[u8], String, &[u8])> {
    let (token, rest) = scan_multiline_literal_string(b)?;
    let mut i = 3;
    if token[i] == b'\n' {
        i += 1;
    } else if token[i] == b'\r' && token.get(i + 1) == Some(&b'\n') {
        i += 2;
    }
    Ok((token, text(&token[i..token.len() - 3]), rest))
}

/// `hexToRune`.
fn hex_to_rune(b: &[u8], length: usize) -> Res<char> {
    if b.len() < length {
        return Err(format!(
            "unicode point needs {length} character, not {}",
            b.len()
        ));
    }
    let mut r: u32 = 0;
    for &c in &b[..length] {
        let d = match c {
            b'0'..=b'9' => u32::from(c - b'0'),
            b'a'..=b'f' => u32::from(c - b'a' + 10),
            b'A'..=b'F' => u32::from(c - b'A' + 10),
            _ => return Err(String::from("non-hex character")),
        };
        r = r * 16 + d;
    }
    char::from_u32(r).ok_or_else(|| String::from("escape sequence is invalid Unicode code point"))
}

/// The byte a simple escape (`\"` `\\` `\b` `\f` `\n` `\r` `\t` `\e`) writes.
fn simple_escape(c: u8) -> Option<u8> {
    match c {
        b'"' | b'\\' => Some(c),
        b'b' => Some(0x08),
        b'f' => Some(0x0C),
        b'n' => Some(b'\n'),
        b'r' => Some(b'\r'),
        b't' => Some(b'\t'),
        b'e' => Some(0x1B),
        _ => None,
    }
}

/// `parseBasicString`.
fn parse_basic_string(b: &[u8]) -> Res<(&[u8], String, &[u8])> {
    let (token, escaped, rest) = scan_basic_string(b)?;
    if !escaped {
        let body = &token[1..token.len() - 1];
        if !utf8_toml_valid(body) {
            return Err(String::from("invalid UTF-8"));
        }
        return Ok((token, text(body), rest));
    }
    let mut out: Vec<u8> = Vec::new();
    let mut i = 1;
    while i < token.len() - 1 {
        let c = token[i];
        if c == b'\\' {
            i += 1;
            let c = token[i];
            if let Some(byte) = simple_escape(c) {
                out.push(byte);
            } else if c == b'u' {
                let x = hex_to_rune(&token[i + 1..token.len() - 1], 4)?;
                push_char(&mut out, x);
                i += 4;
            } else if c == b'U' {
                let x = hex_to_rune(&token[i + 1..token.len() - 1], 8)?;
                push_char(&mut out, x);
                i += 8;
            } else {
                return Err(format!("invalid escaped character {}", go_sharp_u(c)));
            }
            i += 1;
        } else {
            let size = utf8_valid_next(&token[i..]);
            if size == 0 {
                return Err(format!("invalid character {}", go_sharp_u(c)));
            }
            out.extend_from_slice(&token[i..i + size]);
            i += size;
        }
    }
    Ok((token, text(&out), rest))
}

fn push_char(out: &mut Vec<u8>, c: char) {
    let mut buf = [0u8; 4];
    out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
}

/// `parseMultilineBasicString`.
fn parse_multiline_basic_string(b: &[u8]) -> Res<(&[u8], String, &[u8])> {
    let (token, escaped, rest) = scan_multiline_basic_string(b)?;
    let mut i = 3;
    if token[i] == b'\n' {
        i += 1;
    } else if token[i] == b'\r' && token.get(i + 1) == Some(&b'\n') {
        i += 2;
    }
    let start = i;
    let end = token.len() - 3;
    if !escaped {
        let body = &token[start..end];
        if !utf8_toml_valid(body) {
            return Err(String::from("invalid UTF-8"));
        }
        return Ok((token, text(body), rest));
    }
    let mut out: Vec<u8> = Vec::new();
    while i < end {
        let c = token[i];
        if c == b'\\' {
            // A `\` that ends its line trims itself and the white space after it.
            let mut last_non_whitespace_on_line = false;
            let mut probe = 1;
            while probe < end - i {
                match token[i + probe] {
                    b' ' | b'\t' => {
                        probe += 1;
                        continue;
                    }
                    b'\r' => {
                        if token[i + probe + 1] == b'\n' {
                            probe += 1;
                            continue;
                        }
                    }
                    b'\n' => last_non_whitespace_on_line = true,
                    _ => {}
                }
                break;
            }
            if last_non_whitespace_on_line {
                i += probe;
                while i < end {
                    let c = token[i];
                    if c != b'\n' && c != b'\r' && c != b' ' && c != b'\t' {
                        break;
                    }
                    i += 1;
                }
                continue;
            }
            i += 1;
            let c = token[i];
            if let Some(byte) = simple_escape(c) {
                out.push(byte);
            } else if c == b'u' {
                let after = &token[i + 1..];
                let rune = hex_to_rune(&after[..after.len().min(4)], 4)?;
                push_char(&mut out, rune);
                i += 4;
            } else if c == b'U' {
                let after = &token[i + 1..];
                let rune = hex_to_rune(&after[..after.len().min(8)], 8)?;
                push_char(&mut out, rune);
                i += 8;
            } else {
                return Err(format!("invalid escaped character {}", go_sharp_u(c)));
            }
            i += 1;
        } else {
            let size = utf8_valid_next(&token[i..]);
            if size == 0 {
                return Err(format!("invalid character {}", go_sharp_u(c)));
            }
            out.extend_from_slice(&token[i..i + size]);
            i += size;
        }
    }
    Ok((token, text(&out), rest))
}

/// `parseIntOrFloatOrDateTime`.
fn parse_int_or_float_or_date_time(b: &[u8]) -> Res<(Value, &[u8])> {
    match b[0] {
        b'i' => {
            if !scan_follows(b, "inf") {
                return Err(String::from("expected 'inf'"));
            }
            return Ok((Value::Float(text(&b[..3])), &b[3..]));
        }
        b'n' => {
            if !scan_follows(b, "nan") {
                return Err(String::from("expected 'nan'"));
            }
            return Ok((Value::Float(text(&b[..3])), &b[3..]));
        }
        b'+' | b'-' => return scan_int_or_float(b),
        _ => {}
    }
    if b.len() < 3 {
        return scan_int_or_float(b);
    }
    let s = b.len().min(5);
    for (idx, &c) in b[..s].iter().enumerate() {
        if c.is_ascii_digit() {
            continue;
        }
        if (idx == 2 && c == b':') || (idx == 4 && c == b'-') {
            return Ok(scan_date_time(b));
        }
        break;
    }
    scan_int_or_float(b)
}

/// `scanDateTime`: the run of date-time characters, typed by what it holds.
fn scan_date_time(b: &[u8]) -> (Value, &[u8]) {
    let mut has_date = false;
    let mut has_time = false;
    let mut has_tz = false;
    let mut seen_space = false;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match c {
            b'0'..=b'9' => {}
            b'-' => {
                has_date = true;
                if i >= 8 {
                    has_tz = true;
                }
            }
            b'T' | b't' | b':' | b'.' => has_time = true,
            b'+' | b'Z' | b'z' => has_tz = true,
            b' ' => {
                if !seen_space && i + 1 < b.len() && b[i + 1].is_ascii_digit() {
                    i += 2;
                    if i >= b.len() {
                        i -= 1;
                    }
                    seen_space = true;
                    has_time = true;
                } else {
                    break;
                }
            }
            _ => break,
        }
        i += 1;
    }
    let data = text(&b[..i]);
    let value = match (has_time, has_date, has_tz) {
        (true, true, true) => Value::DateTime(data),
        (true, true, false) => Value::Local("LocalDateTime"),
        (true, false, _) => Value::Local("LocalTime"),
        (false, _, _) => Value::Local("LocalDate"),
    };
    (value, &b[i..])
}

/// `scanIntOrFloat`.
fn scan_int_or_float(b: &[u8]) -> Res<(Value, &[u8])> {
    let mut i = 0;
    if b.len() > 2 && b[0] == b'0' && b[1] != b'.' && b[1] != b'e' && b[1] != b'E' {
        let valid: Option<fn(u8) -> bool> = match b[1] {
            b'x' => Some(|r| r.is_ascii_hexdigit() || r == b'_'),
            b'o' => Some(|r| (b'0'..=b'7').contains(&r) || r == b'_'),
            b'b' => Some(|r| r == b'0' || r == b'1' || r == b'_'),
            _ => {
                i += 1;
                None
            }
        };
        if let Some(valid) = valid {
            i += 2;
            while i < b.len() && valid(b[i]) {
                i += 1;
            }
        }
        return Ok((Value::Int(text(&b[..i])), &b[i..]));
    }
    let mut is_float = false;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_digit() || c == b'+' || c == b'-' || c == b'_' {
            i += 1;
            continue;
        }
        if c == b'.' || c == b'e' || c == b'E' {
            is_float = true;
            i += 1;
            continue;
        }
        if c == b'i' {
            if scan_follows(&b[i..], "inf") {
                return Ok((Value::Float(text(&b[..i + 3])), &b[i + 3..]));
            }
            return Err(String::from(
                "unexpected character 'i' while scanning for a number",
            ));
        }
        if c == b'n' {
            if scan_follows(&b[i..], "nan") {
                return Ok((Value::Float(text(&b[..i + 3])), &b[i + 3..]));
            }
            return Err(String::from(
                "unexpected character 'n' while scanning for a number",
            ));
        }
        break;
    }
    if i == 0 {
        return Err(String::from("incomplete number"));
    }
    let data = text(&b[..i]);
    let value = if is_float {
        Value::Float(data)
    } else {
        Value::Int(data)
    };
    Ok((value, &b[i..]))
}

// ---------------------------------------------------------------------------------------------
// The numbers and dates yq checks while it decodes
// ---------------------------------------------------------------------------------------------

/// `strconv.ParseFloat`'s verdict on `s`: `Ok` or the message of its error.
fn go_parse_float(s: &str) -> Result<(), String> {
    let err = |reason: &str| {
        format!(
            "strconv.ParseFloat: parsing {}: {reason}",
            go_quote_bytes(s.as_bytes())
        )
    };
    let bytes = s.as_bytes();
    // `special`: an optional sign and `inf` or `infinity`, or an unsigned `nan`.
    let (signed, unsigned) = match bytes.first() {
        Some(b'+' | b'-') => (true, &s[1..]),
        _ => (false, s),
    };
    let lower = unsigned.to_ascii_lowercase();
    if lower == "inf" || lower == "infinity" || (!signed && lower == "nan") {
        return Ok(());
    }
    // `readFloat`, for a decimal number.
    let mut i = usize::from(signed);
    let mut underscores = false;
    let mut saw_dot = false;
    let mut saw_digits = false;
    while i < bytes.len() {
        match bytes[i] {
            b'_' => underscores = true,
            b'.' => {
                if saw_dot {
                    break;
                }
                saw_dot = true;
            }
            b'0'..=b'9' => saw_digits = true,
            _ => break,
        }
        i += 1;
    }
    if !saw_digits {
        return Err(err("invalid syntax"));
    }
    if i < bytes.len() && bytes[i].eq_ignore_ascii_case(&b'e') {
        i += 1;
        if i >= bytes.len() {
            return Err(err("invalid syntax"));
        }
        if bytes[i] == b'+' || bytes[i] == b'-' {
            i += 1;
        }
        if i >= bytes.len() || !bytes[i].is_ascii_digit() {
            return Err(err("invalid syntax"));
        }
        while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
            underscores |= bytes[i] == b'_';
            i += 1;
        }
    }
    if underscores && !underscore_ok(&bytes[..i]) {
        return Err(err("invalid syntax"));
    }
    if i != bytes.len() {
        return Err(err("invalid syntax"));
    }
    let cleaned: String = s.chars().filter(|&c| c != '_').collect();
    match cleaned.parse::<f64>() {
        Ok(v) if v.is_infinite() => Err(err("value out of range")),
        Ok(_) => Ok(()),
        Err(_) => Err(err("invalid syntax")),
    }
}

/// `underscoreOK`: underscores only between digits.
fn underscore_ok(mut s: &[u8]) -> bool {
    if matches!(s.first(), Some(b'-' | b'+')) {
        s = &s[1..];
    }
    let mut saw = b'^';
    let mut hex = false;
    let mut i = 0;
    if s.len() >= 2 && s[0] == b'0' && matches!(s[1].to_ascii_lowercase(), b'b' | b'o' | b'x') {
        i = 2;
        saw = b'0';
        hex = s[1].eq_ignore_ascii_case(&b'x');
    }
    while i < s.len() {
        let c = s[i];
        if c.is_ascii_digit() || (hex && (b'a'..=b'f').contains(&c.to_ascii_lowercase())) {
            saw = b'0';
        } else if c == b'_' {
            if saw != b'0' {
                return false;
            }
            saw = b'_';
        } else {
            if saw == b'_' {
                return false;
            }
            saw = b'!';
        }
        i += 1;
    }
    saw != b'_'
}

/// yq's `parseDateTime(time.RFC3339, s)`: `Ok`, or the message of the error of `time.Parse`
/// reading `s` as `2006-01-02` (the second attempt, whose error is the one returned).
fn parse_date_time(s: &str) -> Result<(), String> {
    if parse_rfc3339(s) {
        return Ok(());
    }
    parse_date_layout(s)
}

fn go_time_quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `getnum`: a number of one or two digits (two when `fixed`) and the rest, or `None`.
fn getnum(s: &[u8], fixed: bool) -> Option<(i64, &[u8])> {
    if !s.first()?.is_ascii_digit() {
        return None;
    }
    if !s.get(1).is_some_and(u8::is_ascii_digit) {
        if fixed {
            return None;
        }
        return Some((i64::from(s[0] - b'0'), &s[1..]));
    }
    Some((
        i64::from(s[0] - b'0') * 10 + i64::from(s[1] - b'0'),
        &s[2..],
    ))
}

fn days_in(month: i64, year: i64) -> i64 {
    match month {
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// `time.Parse("2006-01-02", s)`: `Ok` for a date, or Go's error.
fn parse_date_layout(s: &str) -> Result<(), String> {
    const LAYOUT: &str = "\"2006-01-02\"";
    let cannot = |value: &[u8], elem: &str| {
        format!(
            "parsing time {} as {LAYOUT}: cannot parse {} as {}",
            go_time_quote(s),
            go_time_quote(&text(value)),
            go_time_quote(elem)
        )
    };
    let range = |what: &str| format!("parsing time {}: {what} out of range", go_time_quote(s));
    let mut value = s.as_bytes();
    // 2006
    if value.len() < 4 || !value[..4].iter().all(u8::is_ascii_digit) {
        return Err(cannot(value, "2006"));
    }
    let year: i64 = text(&value[..4]).parse().unwrap_or(0);
    value = &value[4..];
    // -01
    if value.first() != Some(&b'-') {
        return Err(cannot(value, "-"));
    }
    value = &value[1..];
    let Some((month, rest)) = getnum(value, true) else {
        return Err(cannot(value, "01"));
    };
    if !(1..=12).contains(&month) {
        return Err(range("month"));
    }
    value = rest;
    // -02
    if value.first() != Some(&b'-') {
        return Err(cannot(value, "-"));
    }
    value = &value[1..];
    let Some((day, rest)) = getnum(value, true) else {
        return Err(cannot(value, "02"));
    };
    value = rest;
    if !value.is_empty() {
        return Err(format!(
            "parsing time {}: extra text: {}",
            go_time_quote(s),
            go_time_quote(&text(value))
        ));
    }
    if day < 1 || day > days_in(month, year) {
        return Err(range("day"));
    }
    Ok(())
}

/// Whether `time.Parse(time.RFC3339, s)` reads `s`.
fn parse_rfc3339(s: &str) -> bool {
    let mut value = s.as_bytes();
    if value.len() < 4 || !value[..4].iter().all(u8::is_ascii_digit) {
        return false;
    }
    let year: i64 = text(&value[..4]).parse().unwrap_or(0);
    value = &value[4..];
    let Some(rest) = value.strip_prefix(b"-") else {
        return false;
    };
    let Some((month, rest)) = getnum(rest, true) else {
        return false;
    };
    if !(1..=12).contains(&month) {
        return false;
    }
    let Some(rest) = rest.strip_prefix(b"-") else {
        return false;
    };
    let Some((day, rest)) = getnum(rest, true) else {
        return false;
    };
    let Some(rest) = rest.strip_prefix(b"T") else {
        return false;
    };
    let Some((hour, rest)) = getnum(rest, false) else {
        return false;
    };
    if hour >= 24 {
        return false;
    }
    let Some(rest) = rest.strip_prefix(b":") else {
        return false;
    };
    let Some((minute, rest)) = getnum(rest, true) else {
        return false;
    };
    if minute >= 60 {
        return false;
    }
    let Some(rest) = rest.strip_prefix(b":") else {
        return false;
    };
    let Some((second, mut rest)) = getnum(rest, true) else {
        return false;
    };
    if second >= 60 {
        return false;
    }
    // A fractional second, though the layout has none.
    if rest.len() >= 2 && (rest[0] == b'.' || rest[0] == b',') && rest[1].is_ascii_digit() {
        let mut n = 2;
        while n < rest.len() && rest[n].is_ascii_digit() {
            n += 1;
        }
        rest = &rest[n..];
    }
    // Z07:00
    if rest.first() == Some(&b'Z') {
        rest = &rest[1..];
    } else {
        if rest.len() < 6 || rest[3] != b':' {
            return false;
        }
        let (sign, hh, mm) = (rest[0], &rest[1..3], &rest[4..6]);
        let (Some((hr, _)), Some((mi, _))) = (getnum(hh, true), getnum(mm, true)) else {
            return false;
        };
        if hr > 24 || mi > 60 {
            return false;
        }
        if sign != b'+' && sign != b'-' {
            return false;
        }
        rest = &rest[6..];
    }
    if !rest.is_empty() {
        return false;
    }
    day >= 1 && day <= days_in(month, year)
}

// ---------------------------------------------------------------------------------------------
// The nodes yq builds
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Scalar,
    Map,
    Seq,
}

/// A `CandidateNode`: a mapping holds its keys beside its values, in `keys` and `items`.
#[derive(Clone, Debug)]
struct Node {
    kind: Kind,
    tag: String,
    value: String,
    keys: Vec<String>,
    items: Vec<Self>,
    head: String,
    line: String,
}

impl Node {
    fn scalar(tag: &str, value: &str) -> Self {
        Self {
            kind: Kind::Scalar,
            tag: tag.to_string(),
            value: value.to_string(),
            keys: Vec::new(),
            items: Vec::new(),
            head: String::new(),
            line: String::new(),
        }
    }

    fn null() -> Self {
        Self::scalar("!!null", "null")
    }

    fn map() -> Self {
        let mut node = Self::scalar("!!map", "");
        node.kind = Kind::Map;
        node
    }

    fn seq() -> Self {
        let mut node = Self::scalar("!!seq", "");
        node.kind = Kind::Seq;
        node
    }

    /// `UpdateFrom`: this node becomes a copy of `other` (keeping the comments it has when
    /// `other` has none).
    fn update_from(&mut self, other: &Self) {
        self.items.clone_from(&other.items);
        self.keys.clone_from(&other.keys);
        self.kind = other.kind;
        self.value.clone_from(&other.value);
        self.update_attributes_from(other);
    }

    /// `UpdateAttributesFrom`: kind, tag and comments, but not the content.
    fn update_attributes_from(&mut self, other: &Self) {
        if self.kind != other.kind {
            self.items.clear();
            self.keys.clear();
            self.value.clear();
        }
        self.kind = other.kind;
        if self.tag.starts_with("!!") || self.tag.is_empty() {
            self.tag.clone_from(&other.tag);
        }
        if !other.head.is_empty() {
            self.head.clone_from(&other.head);
        }
        if !other.line.is_empty() {
            self.line.clone_from(&other.line);
        }
    }
}

/// One segment of a path: a key, or an index `getPathToUse` put in.
#[derive(Clone, Debug)]
enum Seg {
    Index(i64),
    Name(String),
}

impl Seg {
    fn text(&self) -> String {
        match self {
            Self::Index(index) => index.to_string(),
            Self::Name(name) => name.clone(),
        }
    }
}

fn names(path: &[String]) -> Vec<Seg> {
    path.iter().cloned().map(Seg::Name).collect()
}

/// `GetNicePath` of the path.
fn nice_path(path: &[Seg]) -> String {
    let mut out = String::new();
    for (i, seg) in path.iter().enumerate() {
        match seg {
            Seg::Index(index) => out.push_str(&format!("[{index}]")),
            Seg::Name(name) => {
                if i == 0 {
                    out.push_str(name);
                } else if name.contains('.') {
                    out.push_str(&format!("[{name}]"));
                } else {
                    out.push('.');
                    out.push_str(name);
                }
            }
        }
    }
    out
}

#[derive(Default)]
struct Budget {
    sequence_elements: usize,
}

/// A step from a node to one of its children, by position (an assignment only ever appends).
#[derive(Clone, Copy)]
enum Step {
    Entry(usize),
    Item(usize),
}

fn node_at<'a>(root: &'a mut Node, path: &[Step]) -> &'a mut Node {
    let mut node = root;
    for step in path {
        node = match *step {
            Step::Entry(i) | Step::Item(i) => &mut node.items[i],
        };
    }
    node
}

/// yq's `traverse` of one segment from `node`: the children it matches, making what it names.
fn traverse(
    node: &mut Node,
    seg: &Seg,
    exact: bool,
    create: bool,
    budget: &mut Budget,
) -> Result<Vec<Step>, String> {
    let wanted = seg.text();
    if node.tag == "!!null" && !matches!(seg, Seg::Name(name) if name == "[]") {
        node.kind = match seg {
            Seg::Index(_) => Kind::Seq,
            Seg::Name(_) => Kind::Map,
        };
        node.tag = String::new();
    }
    match node.kind {
        Kind::Map => {
            // yq collects the matches in an ordered map keyed by the key's text, so of two keys
            // spelled alike only the last is matched, in the place of the first.
            let mut matched: Vec<Step> = Vec::new();
            let mut spellings: Vec<&str> = Vec::new();
            for (i, key) in node.keys.iter().enumerate() {
                let hit = if exact {
                    *key == wanted
                } else {
                    yq_match_key(key, &wanted)
                };
                if !hit {
                    continue;
                }
                match spellings.iter().position(|&k| k == key) {
                    Some(seen) => matched[seen] = Step::Entry(i),
                    None => {
                        spellings.push(key);
                        matched.push(Step::Entry(i));
                    }
                }
            }
            if !matched.is_empty() || !create {
                return Ok(matched);
            }
            node.keys.push(wanted);
            node.items.push(Node::null());
            Ok(vec![Step::Entry(node.items.len() - 1)])
        }
        Kind::Seq => {
            let index = yq_parse_int64(&wanted).ok_or_else(|| {
                format!(
                    "cannot index array with '{wanted}' ({})",
                    go_parse_int_error(&wanted)
                )
            })?;
            let len = node.items.len();
            if let Ok(target) = usize::try_from(index) {
                if target >= len {
                    let total = budget
                        .sequence_elements
                        .checked_add(target - len + 1)
                        .filter(|&total| total <= MAX_SEQUENCE_ELEMENTS);
                    let Some(total) = total else {
                        return Err(format!(
                            "Cannot grow array to {} elements",
                            target as u64 + 1
                        ));
                    };
                    budget.sequence_elements = total;
                    node.items.resize_with(target + 1, Node::null);
                }
            }
            let len = node.items.len();
            let position = if index < 0 { len as i64 + index } else { index };
            let Ok(position) = usize::try_from(position) else {
                return Err(format!("index [{index}] out of range, array size is {len}"));
            };
            Ok(vec![Step::Item(position)])
        }
        Kind::Scalar => Ok(Vec::new()),
    }
}

/// The nodes a path matches below `root`, segment by segment (each runs over every match of the
/// one before it), as positions.
fn traverse_path(
    root: &mut Node,
    path: &[Seg],
    exact: bool,
    create: bool,
    budget: &mut Budget,
) -> Result<Vec<Vec<Step>>, String> {
    let mut frontier: Vec<Vec<Step>> = vec![Vec::new()];
    for seg in path {
        let mut next = Vec::new();
        for at in &frontier {
            let node = node_at(root, at);
            for step in traverse(node, seg, exact, create, budget)? {
                let mut child = at.clone();
                child.push(step);
                next.push(child);
            }
        }
        frontier = next;
    }
    Ok(frontier)
}

/// `add` of the sequence `rhs` to `lhs`, as `+=` does.
fn add(lhs: &Node, rhs: &Node, lhs_path: &str, rhs_path: &str) -> Result<Node, String> {
    if lhs.tag == "!!null" {
        return Ok(rhs.clone());
    }
    match lhs.kind {
        Kind::Seq => {
            let mut target = lhs.clone();
            if rhs.tag != "!!null" {
                target.items.extend(rhs.items.iter().cloned());
            }
            Ok(target)
        }
        Kind::Map | Kind::Scalar => Err(format!(
            "{} ({rhs_path}) cannot be added to a {} ({lhs_path})",
            rhs.tag, lhs.tag
        )),
    }
}

/// `multiply` of the table `rhs` into `lhs`, appending arrays.
fn multiply(lhs: Node, rhs: &Node, base: &[Seg], budget: &mut Budget) -> Result<Node, String> {
    if rhs.tag == "!!null" {
        return Ok(lhs);
    }
    if rhs.kind == Kind::Map && (lhs.kind == Kind::Map || lhs.tag == "!!null") {
        let mut blank = lhs;
        if blank.head.is_empty() {
            blank.head.clone_from(&rhs.head);
        }
        return merge_objects(blank, rhs, base, budget);
    }
    Err(format!("cannot multiply {} with {}", lhs.tag, rhs.tag))
}

/// One node `mergeObjects` visits: the root, a key, or a value.
enum Visit<'a> {
    Root,
    Key(Vec<Seg>),
    Value(Vec<Seg>, &'a Node),
}

fn descend<'a>(node: &'a Node, path: &mut Vec<Seg>, out: &mut Vec<Visit<'a>>) {
    if node.kind != Kind::Map {
        return;
    }
    // yq lists a mapping's children in an ordered map keyed by the key's text, so of two keys
    // spelled alike (an inline table can hold them) only the last is visited, in the first's place.
    let mut entries: Vec<(&String, &Node)> = Vec::with_capacity(node.keys.len());
    let mut position: std::collections::HashMap<&str, usize> =
        std::collections::HashMap::with_capacity(node.keys.len());
    for (key, value) in node.keys.iter().zip(&node.items) {
        match position.get(key.as_str()) {
            Some(&at) => entries[at].1 = value,
            None => {
                position.insert(key, entries.len());
                entries.push((key, value));
            }
        }
    }
    for (key, value) in entries {
        path.push(Seg::Name(key.clone()));
        out.push(Visit::Key(path.clone()));
        out.push(Visit::Value(path.clone(), value));
        descend(value, path, out);
        path.pop();
    }
}

/// The keys and indexes of the nodes a match walks through.
fn steps_path(root: &mut Node, at: &[Step]) -> Vec<Seg> {
    let mut path = Vec::with_capacity(at.len());
    let mut node = root;
    for step in at {
        match *step {
            Step::Entry(i) => path.push(Seg::Name(node.keys[i].clone())),
            Step::Item(i) => path.push(Seg::Index(i as i64)),
        }
        node = match *step {
            Step::Entry(i) | Step::Item(i) => &mut node.items[i],
        };
    }
    path
}

/// `mergeObjects`: every key and value of `rhs`, in order, written into `lhs` by its path.
fn merge_objects(
    mut lhs: Node,
    rhs: &Node,
    base: &[Seg],
    budget: &mut Budget,
) -> Result<Node, String> {
    let mut visits = vec![Visit::Root];
    descend(rhs, &mut Vec::new(), &mut visits);
    for visit in visits {
        match visit {
            Visit::Root => lhs.update_attributes_from(rhs),
            Visit::Key(path) => {
                // The key nodes the path matches become the key written (a key that is a glob
                // matches, and so renames, every key it fits).
                let matches = traverse_path(&mut lhs, &path, false, true, budget)?;
                if let Some(Seg::Name(wanted)) = path.last() {
                    for at in matches {
                        if let Some((Step::Entry(i), parent)) = at.split_last() {
                            node_at(&mut lhs, parent).keys[*i].clone_from(wanted);
                        }
                    }
                }
            }
            Visit::Value(path, value) => {
                let matches = traverse_path(&mut lhs, &path, false, true, budget)?;
                for at in matches {
                    let mut actual = base.to_vec();
                    actual.extend(steps_path(&mut lhs, &at));
                    let node = node_at(&mut lhs, &at);
                    match value.kind {
                        Kind::Seq => {
                            let result =
                                add(&node.clone(), value, &nice_path(&actual), &nice_path(&path))?;
                            node.update_from(&result);
                        }
                        Kind::Scalar => node.update_from(value),
                        Kind::Map => node.update_attributes_from(value),
                    }
                }
            }
        }
    }
    Ok(lhs)
}

/// `DeeplyAssign`: a table is merged in, anything else replaces what the path names.
fn deeply_assign(
    root: &mut Node,
    path: &[Seg],
    rhs: &Node,
    budget: &mut Budget,
) -> Result<(), String> {
    let matches = traverse_path(root, path, false, true, budget)?;
    for at in matches {
        let actual = steps_path(root, &at);
        let node = node_at(root, &at);
        if rhs.kind == Kind::Map {
            let result = multiply(node.clone(), rhs, &actual, budget)?;
            node.update_from(&result);
        } else {
            node.update_from(rhs);
        }
    }
    Ok(())
}

/// `arrayAppend`: `path += [table]`.
fn array_append(
    root: &mut Node,
    path: &[Seg],
    table: Node,
    budget: &mut Budget,
) -> Result<(), String> {
    let mut rhs = Node::seq();
    rhs.items.push(table);
    let matches = traverse_path(root, path, false, true, budget)?;
    for at in matches {
        let node = node_at(root, &at);
        // This is `add`'s rule for a sequence on the left (kept in step with it); written in place, so a long run of
        // `[[array]]` tables does not copy the array once per table (yq does, and is cubic).
        if node.kind == Kind::Seq && node.tag != "!!null" {
            node.items.push(rhs.items[0].clone());
            continue;
        }
        let actual = nice_path(&steps_path(root, &at));
        let node = node_at(root, &at);
        let result = add(&node.clone(), &rhs, &actual, "")?;
        node.update_from(&result);
    }
    Ok(())
}

/// `getPathToUse`: a segment that names an array (with an element) is followed by that array's
/// last index, so a table writes into the last `[[element]]`.
fn get_path_to_use(
    root: &mut Node,
    full: &[String],
    budget: &mut Budget,
) -> Result<Vec<Seg>, String> {
    let mut result: Vec<Seg> = Vec::new();
    let Some((last, init)) = full.split_last() else {
        return Ok(result);
    };
    for name in init {
        result.push(Seg::Name(name.clone()));
        let matches = traverse_path(root, &result, false, false, budget)?;
        if let Some(first) = matches.first() {
            let node = node_at(root, first);
            if node.kind == Kind::Seq && !node.items.is_empty() {
                result.push(Seg::Index(node.items.len() as i64 - 1));
            }
        }
    }
    result.push(Seg::Name(last.clone()));
    Ok(result)
}

// ---------------------------------------------------------------------------------------------
// decoder_toml.go
// ---------------------------------------------------------------------------------------------

struct Decoder<'a> {
    parser: Parser<'a>,
    /// The expression the parser last returned, which a loop may ask to run again.
    cur: Option<Expr>,
    /// The standalone comments that the next value, table or element takes as its head comment.
    pending: Vec<String>,
    first_content_seen: bool,
    budget: Budget,
}

impl Decoder<'_> {
    fn advance(&mut self) -> bool {
        match self.parser.next_expression() {
            Some(expr) => {
                self.cur = Some(expr);
                true
            }
            None => false,
        }
    }

    fn take_pending(&mut self) -> Option<String> {
        if self.pending.is_empty() {
            return None;
        }
        let joined = self.pending.join("\n");
        self.pending.clear();
        Some(joined)
    }

    /// `Decode`: the document, or `None` when it has no key.
    fn decode(&mut self) -> Result<Option<Node>, String> {
        let mut root = Node::map();
        let mut run_again = false;
        while run_again || self.advance() {
            let Some(expr) = self.cur.clone() else { break };
            run_again = self.process_top_level_node(&mut root, expr)?;
        }
        if let Some(err) = self.parser.err.take() {
            return Err(err);
        }
        if root.items.is_empty() {
            return Ok(None);
        }
        Ok(Some(root))
    }

    fn process_top_level_node(&mut self, root: &mut Node, expr: Expr) -> Result<bool, String> {
        match expr {
            Expr::Comment(comment) => {
                if self.first_content_seen {
                    self.pending.push(comment);
                } else if root.head.is_empty() {
                    root.head = comment;
                } else {
                    root.head.push('\n');
                    root.head.push_str(&comment);
                }
                Ok(false)
            }
            Expr::Table(path) => {
                self.first_content_seen = true;
                self.process_table(root, &path)
            }
            Expr::ArrayTable(path) => {
                self.first_content_seen = true;
                self.process_array_table(root, &path)
            }
            Expr::KeyValue(kv) => {
                self.first_content_seen = true;
                self.decode_key_values_into_map(root, kv)
            }
        }
    }

    fn process_key_value_into_map(&mut self, target: &mut Node, kv: &KeyVal) -> Result<(), String> {
        let mut value = self.decode_node(&kv.value)?;
        if let Some(head) = self.take_pending() {
            value.head = head;
        }
        if let Some(comment) = &kv.comment {
            value.line.clone_from(comment);
        }
        deeply_assign(target, &names(&kv.path), &value, &mut self.budget)
    }

    fn decode_key_values_into_map(
        &mut self,
        target: &mut Node,
        first: KeyVal,
    ) -> Result<bool, String> {
        self.process_key_value_into_map(target, &first)?;
        while self.advance() {
            match self.cur.clone() {
                Some(Expr::KeyValue(kv)) => self.process_key_value_into_map(target, &kv)?,
                Some(Expr::Comment(comment)) => self.pending.push(comment),
                _ => return Ok(true),
            }
        }
        Ok(false)
    }

    fn decode_node(&mut self, value: &Value) -> Result<Node, String> {
        match value {
            Value::Str(s) => Ok(Node::scalar("!!str", s)),
            Value::Bool(b) => Ok(Node::scalar("!!bool", if *b { "true" } else { "false" })),
            Value::Int(s) => {
                if yq_parse_int64(s).is_none() {
                    return Err(go_parse_int_error(s));
                }
                Ok(Node::scalar("!!int", s))
            }
            Value::DateTime(s) => {
                parse_date_time(s)?;
                Ok(Node::scalar("", s))
            }
            Value::Float(s) => {
                go_parse_float(s)?;
                Ok(Node::scalar("!!float", s))
            }
            Value::Local(kind) => Err(format!("unsupported type {kind}")),
            Value::Array(items) => {
                let mut node = Node::seq();
                let mut pending_array_comments: Vec<&str> = Vec::new();
                for item in items {
                    match item {
                        ArrayItem::Comment(c) => pending_array_comments.push(c),
                        ArrayItem::Value(v) => {
                            let mut element = self.decode_node(v)?;
                            if !pending_array_comments.is_empty() {
                                element.head = pending_array_comments.join("\n");
                                pending_array_comments.clear();
                            }
                            node.items.push(element);
                        }
                    }
                }
                Ok(node)
            }
            Value::InlineTable(children) => {
                let mut node = Node::map();
                for child in children {
                    let mut key_values = Node::map();
                    self.process_key_value_into_map(&mut key_values, child)?;
                    node.keys.append(&mut key_values.keys);
                    node.items.append(&mut key_values.items);
                }
                Ok(node)
            }
        }
    }

    fn attach_orphaned_comments(&mut self, table: &mut Node) {
        if let Some(comments) = self.take_pending() {
            if table.head.is_empty() {
                table.head = comments;
            } else {
                table.head.push('\n');
                table.head.push_str(&comments);
            }
        }
    }

    fn process_table(&mut self, root: &mut Node, path: &[String]) -> Result<bool, String> {
        let full = get_path_to_use(root, path, &mut self.budget)?;
        let mut table = Node::map();
        if let Some(head) = self.take_pending() {
            table.head = head;
        }
        let mut run_again = false;
        let mut saw_key_value = false;
        while self.advance() {
            let Some(expr) = self.cur.clone() else { break };
            match expr {
                Expr::Comment(comment) => {
                    self.pending.push(comment);
                    continue;
                }
                Expr::KeyValue(kv) => {
                    saw_key_value = true;
                    run_again = self.decode_key_values_into_map(&mut table, kv)?;
                }
                _ => {
                    if !saw_key_value {
                        self.attach_orphaned_comments(&mut table);
                    }
                    run_again = true;
                }
            }
            break;
        }
        if !saw_key_value {
            self.attach_orphaned_comments(&mut table);
        }
        deeply_assign(root, &full, &table, &mut self.budget)?;
        Ok(run_again)
    }

    fn process_array_table(&mut self, root: &mut Node, path: &[String]) -> Result<bool, String> {
        let full = get_path_to_use(root, path, &mut self.budget)?;
        let has_value = self.advance();
        let mut table = Node::map();
        if let Some(head) = self.take_pending() {
            table.head = head;
        }
        let mut run_again = false;
        let mut saw_key_value = false;
        if has_value {
            while let Some(expr) = self.cur.clone() {
                match expr {
                    Expr::Comment(comment) => {
                        self.pending.push(comment);
                        if !self.advance() {
                            break;
                        }
                    }
                    Expr::KeyValue(kv) => {
                        saw_key_value = true;
                        run_again = self.decode_key_values_into_map(&mut table, kv)?;
                        break;
                    }
                    Expr::Table(_) | Expr::ArrayTable(_) => {
                        if !saw_key_value {
                            self.attach_orphaned_comments(&mut table);
                        }
                        run_again = true;
                        break;
                    }
                }
            }
        }
        if !saw_key_value {
            self.attach_orphaned_comments(&mut table);
        }
        array_append(root, &full, table, &mut self.budget)?;
        Ok(run_again)
    }
}

// ---------------------------------------------------------------------------------------------
// The YAML text yq's encoder writes for the nodes
// ---------------------------------------------------------------------------------------------

#[derive(Default)]
struct Out {
    text: String,
    /// Whether the next line's indent is already written (a collection after a `- `).
    skip_indent: bool,
    /// The line comments of collections, which go-yaml's emitter writes one at a time on the next
    /// lines that have none of their own (after the next scalar, or after the next key whose
    /// value is a collection), and loses when there are no more lines.
    carry: std::collections::VecDeque<String>,
}

impl Out {
    fn indent(&mut self, indent: &str) {
        if self.skip_indent {
            self.skip_indent = false;
        } else {
            self.text.push_str(indent);
        }
    }

    /// The comment a scalar's line ends with: its own, or else the first carried one.
    fn line_comment_for(&mut self, own: &str) -> Option<String> {
        if own.is_empty() {
            self.carry.pop_front()
        } else {
            Some(own.to_string())
        }
    }

    /// The line comment of a collection that has been written, for a later line.
    fn carry_from(&mut self, collection: &Node) {
        if !collection.line.is_empty() {
            self.carry.push_back(collection.line.clone());
        }
    }
}

fn render_root(out: &mut Out, root: &Node) {
    render_comment(out, &root.head, "");
    render_map(out, root, 0);
}

fn indent_of(depth: usize) -> String {
    STEP.repeat(depth)
}

fn is_empty_map(node: &Node) -> bool {
    node.kind == Kind::Map && node.items.is_empty()
}

fn render_map(out: &mut Out, node: &Node, depth: usize) {
    let indent = indent_of(depth);
    let child = indent_of(depth + 1);
    for (key, value) in node.keys.iter().zip(&node.items) {
        // yq writes the head comment of an empty table inside braces (`key: {` / `# c` / `}`),
        // which the YAML reader drops; it is written before the key instead.
        if is_empty_map(value) && !out.skip_indent {
            render_comment(out, &value.head, &indent);
        }
        out.indent(&indent);
        if go_yaml_key_is_simple(key) {
            out.text.push_str(&string_scalar(key, &child, true));
            out.text.push(':');
            render_value_after_key(out, value, depth);
        } else {
            out.text.push_str("? ");
            out.text.push_str(
                &go_yaml_explicit_key_scalar(key, &child, STEP.len())
                    .unwrap_or_else(|| string_scalar(key, &child, true)),
            );
            out.text.push('\n');
            out.text.push_str(&indent);
            out.text.push_str(": ");
            render_compact(out, value, depth);
        }
    }
}

/// `text` (a scalar as written in place) and the comment that ends its line, then the line break;
/// a block scalar's comment follows its header (`|- # c`).
fn push_line(out: &mut Out, text: &str, comment: Option<String>) {
    match comment {
        None => out.text.push_str(text),
        Some(comment) => match text.split_once('\n') {
            Some((header, rest)) if text.starts_with(['|', '>']) => {
                out.text.push_str(header);
                out.text.push(' ');
                out.text.push_str(&comment);
                out.text.push('\n');
                out.text.push_str(rest);
            }
            _ => {
                out.text.push_str(text);
                out.text.push(' ');
                out.text.push_str(&comment);
            }
        },
    }
    out.text.push('\n');
}

/// The value of a simple key, after its `:`.
fn render_value_after_key(out: &mut Out, value: &Node, depth: usize) {
    let indent = indent_of(depth);
    let child = indent_of(depth + 1);
    match value.kind {
        Kind::Map if !value.items.is_empty() => {
            if let Some(carried) = out.carry.pop_front() {
                out.text.push(' ');
                out.text.push_str(&carried);
            }
            out.text.push('\n');
            render_comment(out, &value.head, &child);
            render_map(out, value, depth + 1);
            out.carry_from(value);
        }
        Kind::Seq if !value.items.is_empty() => {
            if let Some(carried) = out.carry.pop_front() {
                out.text.push(' ');
                out.text.push_str(&carried);
            }
            out.text.push('\n');
            render_seq(out, value, depth + 1, &value.head);
            out.carry_from(value);
        }
        _ => {
            out.text.push(' ');
            let comment = out.line_comment_for(&value.line);
            push_line(out, &inline_text(value, &child), comment);
            if !is_empty_map(value) {
                render_comment(out, &value.head, &indent);
            }
        }
    }
}

/// The elements of a sequence; its own head comment is the first element's when that has none.
fn render_seq(out: &mut Out, node: &Node, depth: usize, head: &str) {
    let indent = indent_of(depth);
    for (i, item) in node.items.iter().enumerate() {
        let item_head = if i == 0 && item.head.is_empty() {
            head
        } else {
            &item.head
        };
        render_comment(out, item_head, &indent);
        out.indent(&indent);
        out.text.push_str("- ");
        render_compact(out, item, depth);
    }
}

/// A node after a `- ` or an explicit key's `: ` at `depth`.
fn render_compact(out: &mut Out, value: &Node, depth: usize) {
    match value.kind {
        Kind::Map if !value.items.is_empty() => {
            // A carried comment ends the `- ` line, and the mapping starts on the next one.
            match out.carry.pop_front() {
                Some(carried) => {
                    out.text.push_str(&carried);
                    out.text.push('\n');
                }
                None => out.skip_indent = true,
            }
            render_map(out, value, depth + 1);
            out.carry_from(value);
        }
        Kind::Seq if !value.items.is_empty() => {
            out.skip_indent = true;
            render_seq(out, value, depth + 1, "");
            out.carry_from(value);
        }
        _ => {
            let comment = out.line_comment_for(&value.line);
            push_line(out, &inline_text(value, &indent_of(depth + 1)), comment);
        }
    }
}

/// A scalar, or an empty collection, as it is written in place.
fn inline_text(value: &Node, block_indent: &str) -> String {
    match value.kind {
        Kind::Map => String::from("{}"),
        Kind::Seq => String::from("[]"),
        Kind::Scalar => match value.tag.as_str() {
            "!!str" => string_scalar(&value.value, block_indent, false),
            "" => value.value.clone(),
            tag => {
                // An underscored number is plain for yq, but the YAML reader here reads it as a
                // string (the recorded input-spec divergence), so it carries its tag.
                if go_yaml_resolved_tag(&value.value) == tag && !value.value.contains('_') {
                    value.value.clone()
                } else {
                    format!("{tag} {}", value.value)
                }
            }
        },
    }
}

/// A head comment as go-yaml writes it: each line `# ` and the text, or the text as it is when it
/// already starts with `#`.
fn render_comment(out: &mut Out, comment: &str, indent: &str) {
    if comment.is_empty() {
        return;
    }
    let comment = comment.strip_suffix('\n').unwrap_or(comment);
    for line in comment.split('\n') {
        if line.is_empty() {
            out.text.push('\n');
            continue;
        }
        out.indent(indent);
        if !line.starts_with('#') {
            out.text.push_str("# ");
        }
        out.text.push_str(line);
        out.text.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yaml(input: &str) -> Result<String, String> {
        transcode(input.as_bytes()).map(|text| String::from_utf8(text).unwrap())
    }

    #[test]
    fn tables_arrays_and_scalars_render_as_yq_prints_them() {
        assert_eq!(
            yaml("a = 1\nb = \"x\"\n[t]\nc = [1, 2]\n[[p]]\nn = true\n[[p]]\nn = false\n").unwrap(),
            "a: 1\nb: x\nt:\n  c:\n    - 1\n    - 2\np:\n  - n: true\n  - n: false\n"
        );
        assert_eq!(yaml("").unwrap(), "");
        assert_eq!(yaml("# only a comment\n").unwrap(), "");
    }

    #[test]
    fn numbers_keep_their_text_and_a_float_word_is_tagged() {
        assert_eq!(
            yaml("a = 0xDEAD_beef\nb = 1_000\nc = inf\nd = -nan\n").unwrap_err(),
            "strconv.ParseFloat: parsing \"-nan\": invalid syntax"
        );
        assert_eq!(
            yaml("a = 0o17\nb = +inf\n").unwrap(),
            "a: 0o17\nb: !!float +inf\n"
        );
    }

    #[test]
    fn local_dates_and_binary_integers_are_errors_in_yq() {
        assert_eq!(
            yaml("a = 1979-05-27\n").unwrap_err(),
            "unsupported type LocalDate"
        );
        assert_eq!(
            yaml("a = 0b101\n").unwrap_err(),
            "strconv.ParseInt: parsing \"0b101\": invalid syntax"
        );
        assert_eq!(
            yaml("a = 1979-05-27 07:32:00Z\n").unwrap_err(),
            "parsing time \"1979-05-27 07:32:00Z\": extra text: \" 07:32:00Z\""
        );
    }

    #[test]
    fn the_parser_names_its_errors_as_go_toml_does() {
        assert_eq!(yaml("a = ").unwrap_err(), "expected value, not eof");
        assert_eq!(yaml("a = \n").unwrap_err(), "incomplete number");
        assert_eq!(
            yaml("a").unwrap_err(),
            "expected = after a key, but the document ends there"
        );
        assert_eq!(yaml("a\n").unwrap_err(), "expected character =");
        assert_eq!(
            yaml("a = 1 b = 2\n").unwrap_err(),
            "expected newline but got U+0062 'b'"
        );
        assert_eq!(yaml("[a\n").unwrap_err(), "expected character ]");
    }

    #[test]
    fn a_key_written_twice_keeps_the_last_value_and_a_table_opened_twice_merges() {
        assert_eq!(yaml("a = 1\na = 2\n").unwrap(), "a: 2\n");
        assert_eq!(
            yaml("[a]\nx = 1\n[a]\ny = 2\n").unwrap(),
            "a:\n  x: 1\n  y: 2\n"
        );
        assert_eq!(yaml("a.b = 1\na.c = 2\n").unwrap(), "a:\n  b: 1\n  c: 2\n");
    }

    #[test]
    fn comments_land_where_yq_prints_them() {
        assert_eq!(
            yaml("# top\n\na = 1 # line\n# before b\nb = 2\n[t]\n# in t\nc = 3\n").unwrap(),
            "# top\na: 1 # line\nb: 2\n# before b\nt:\n  c: 3\n  # in t\n"
        );
    }

    #[test]
    fn a_table_array_element_is_the_last_one_a_deeper_table_writes_into() {
        assert_eq!(
            yaml("[[a]]\nx = 1\n[a.b]\ny = 2\n[[a]]\nx = 3\n").unwrap(),
            "a:\n  - x: 1\n    b:\n      y: 2\n  - x: 3\n"
        );
    }
}
