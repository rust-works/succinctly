//! Go's `encoding/xml` decoder (`xml.go`, Go 1.27.1), the part yq's XML decoder drives: `RawToken`
//! (the default, `--xml-raw-token`) and `Token`, in strict or lenient mode, with the
//! `golang.org/x/net/html/charset` reader yq installs for a declared `encoding`.
//!
//! The port keeps Go's byte-at-a-time shape on purpose: every message and every quirk (an end tag
//! is never matched under `RawToken`, an unquoted attribute value is read to the first character
//! that is not `[A-Za-z0-9_:-]`, an unknown entity is left alone when not strict) comes from the
//! order in which `getc`, `ungetc` and `mustgetc` are called.

use std::collections::HashMap;

use super::super::input_xml_tables::{
    GRAPHIC_NOT_SPACE, NAME_FIRST, NAME_SECOND, SINGLE_BYTE_LABELS, SINGLE_BYTE_TABLES, UTF8_LABELS,
};

/// `xml.Name`: the prefix (`Space`) and local part of a name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Name {
    pub space: String,
    pub local: String,
}

/// `xml.Attr`.
#[derive(Clone, Debug)]
pub(super) struct Attr {
    pub name: Name,
    pub value: String,
}

/// `xml.Token`.
#[derive(Clone, Debug)]
pub(super) enum Token {
    Start { name: Name, attr: Vec<Attr> },
    End(Name),
    CharData(Vec<u8>),
    Comment(Vec<u8>),
    ProcInst { target: String, inst: Vec<u8> },
    Directive(Vec<u8>),
}

/// An error the decoder holds in `d.err`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum XmlError {
    /// `io.EOF`.
    Eof,
    /// Any other error, as its message.
    Message(String),
}

impl XmlError {
    pub(super) fn message(&self) -> String {
        match self {
            XmlError::Eof => "EOF".to_string(),
            XmlError::Message(message) => message.clone(),
        }
    }
}

/// One entry of Go's name-space and element stack.
#[derive(Clone, Debug)]
enum Stack {
    Start(Name),
    /// `stkNs`: the prefix, and the value it had before (`None` when it was unset).
    Ns(String, Option<String>),
}

/// The decoder's options: `Strict` and whether `Token` is used, as yq sets them.
#[derive(Clone, Copy, Debug)]
pub(super) struct LexerOptions {
    pub strict: bool,
    pub raw_token: bool,
}

pub(super) struct Decoder<'a> {
    input: std::borrow::Cow<'a, [u8]>,
    pos: usize,
    strict: bool,
    raw: bool,
    buf: Vec<u8>,
    stk: Vec<Stack>,
    need_close: bool,
    to_close: Name,
    next_token: Option<Token>,
    next_byte: Option<u8>,
    ns: HashMap<String, String>,
    err: Option<XmlError>,
    line: usize,
}

const XML_URL: &str = "http://www.w3.org/XML/1998/namespace";
const XMLNS_PREFIX: &str = "xmlns";
const XML_PREFIX: &str = "xml";

fn in_table(table: &[(u32, u32, u32)], c: u32) -> bool {
    table
        .iter()
        .any(|&(lo, hi, stride)| c >= lo && c <= hi && (c - lo) % stride == 0)
}

fn is_name_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b':' || c == b'.' || c == b'-'
}

/// `unicode.Is(first, c)`, with the ASCII answers (the common ones) without the table walk.
fn is_name_start(c: u32) -> bool {
    if c < 0x80 {
        return c == 0x3A || c == 0x5F || (0x41..=0x5A).contains(&c) || (0x61..=0x7A).contains(&c);
    }
    in_table(NAME_FIRST, c)
}

/// `unicode.Is(second, c)`.
fn is_name_char(c: u32) -> bool {
    if c < 0x80 {
        return c == 0x2D || c == 0x2E || (0x30..=0x39).contains(&c);
    }
    in_table(NAME_SECOND, c)
}

/// `isName`: the bytes are valid UTF-8 and every character is an XML name character.
fn is_name(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !is_name_start(first as u32) {
        return false;
    }
    chars.all(|c| is_name_start(c as u32) || is_name_char(c as u32))
}

/// `isInCharacterRange`.
fn is_in_character_range(c: u32) -> bool {
    c == 0x09
        || c == 0x0A
        || c == 0x0D
        || (0x20..=0xD7FF).contains(&c)
        || (0xE000..=0xFFFD).contains(&c)
        || (0x10000..=0x10FFFF).contains(&c)
}

/// yq's `trimNonGraphic`: the text between the first and the last character that is graphic and
/// not a space, or the empty string when there is none.
pub(super) fn trim_non_graphic(s: &str) -> &str {
    let graphic = |c: char| {
        let c = c as u32;
        GRAPHIC_NOT_SPACE
            .binary_search_by(|&(lo, hi)| {
                if c < lo {
                    std::cmp::Ordering::Greater
                } else if c > hi {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .is_ok()
    };
    let Some(start) = s.char_indices().find(|&(_, c)| graphic(c)).map(|(i, _)| i) else {
        return "";
    };
    let end = s
        .char_indices()
        .rev()
        .find(|&(_, c)| graphic(c))
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(s.len());
    &s[start..end]
}

/// Go's `procInst`: the value of `param="..."` or `param='...'` in a processing instruction. The
/// search runs over bytes, as Go's does, so a multi-byte character cannot be split.
fn proc_inst(param: &str, s: &str) -> String {
    let param = format!("{param}=");
    let lenp = param.len();
    let bytes = s.as_bytes();
    let mut i = 0usize;
    let mut sep = 0u8;
    while i < bytes.len() {
        let sub = &bytes[i..];
        let Some(k) = sub
            .windows(lenp)
            .position(|window| window == param.as_bytes())
        else {
            return String::new();
        };
        if lenp + k >= sub.len() {
            return String::new();
        }
        i += lenp + k + 1;
        let c = sub[lenp + k];
        if c == b'\'' || c == b'"' {
            sep = c;
            break;
        }
    }
    if sep == 0 {
        return String::new();
    }
    match bytes[i..].iter().position(|&b| b == sep) {
        Some(j) => String::from_utf8_lossy(&bytes[i..i + j]).into_owned(),
        None => String::new(),
    }
}

/// The reader `charset.NewReaderLabel` makes for a label, as the bytes it yields for `rest`, or
/// `None` for a label this port does not read.
///
/// `golang.org/x/text/encoding/htmlindex` trims and lowercases the label and maps it to an
/// encoding: the UTF-8 labels to `unicode.UTF8`, whose decoder writes U+FFFD for each byte that is
/// not valid UTF-8, and the single-byte labels (Latin-1 and ASCII are Windows-1252) to a table.
/// The multi-byte encodings (Shift_JIS, GBK, Big5, EUC-JP, EUC-KR, GB18030, ISO-2022-JP and
/// UTF-16) are not read.
fn charset_reader(label: &str, rest: &[u8]) -> Option<Vec<u8>> {
    let key = label
        .trim_matches(|c: char| c.is_whitespace())
        .to_ascii_lowercase();
    if UTF8_LABELS.contains(&key.as_str()) {
        let mut out = Vec::with_capacity(rest.len());
        let mut at = 0usize;
        while at < rest.len() {
            match std::str::from_utf8(&rest[at..]) {
                Ok(_) => {
                    out.extend_from_slice(&rest[at..]);
                    break;
                }
                Err(e) => {
                    let valid = e.valid_up_to();
                    out.extend_from_slice(&rest[at..at + valid]);
                    out.extend_from_slice("\u{FFFD}".as_bytes());
                    at += valid + 1;
                }
            }
        }
        return Some(out);
    }
    let &(_, table) = SINGLE_BYTE_LABELS.iter().find(|(name, _)| *name == key)?;
    let table = &SINGLE_BYTE_TABLES[table];
    let mut out = Vec::with_capacity(rest.len() + rest.len() / 4);
    for &b in rest {
        if b < 0x80 {
            out.push(b);
        } else {
            let c = char::from_u32(u32::from(table[usize::from(b - 0x80)])).unwrap_or('\u{FFFD}');
            let mut tmp = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut tmp).as_bytes());
        }
    }
    Some(out)
}

/// Go's `%q` of a string.
fn quote(s: &str) -> String {
    succinctly::jq::go_quote_bytes(s.as_bytes())
}

impl<'a> Decoder<'a> {
    pub(super) fn new(input: &'a [u8], options: LexerOptions) -> Self {
        Decoder {
            input: std::borrow::Cow::Borrowed(input),
            pos: 0,
            strict: options.strict,
            raw: options.raw_token,
            buf: Vec::new(),
            stk: Vec::new(),
            need_close: false,
            to_close: Name::default(),
            next_token: None,
            next_byte: None,
            ns: HashMap::new(),
            err: None,
            line: 1,
        }
    }

    /// The next token, by `RawToken` or `Token` as the options say; `Ok(None)` at the end.
    pub(super) fn next(&mut self) -> Result<Option<Token>, XmlError> {
        let result = if self.raw {
            self.raw_token()
        } else {
            self.token()
        };
        match result {
            Ok(token) => Ok(Some(token)),
            Err(XmlError::Eof) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn syntax_error(&self, msg: &str) -> XmlError {
        XmlError::Message(format!("XML syntax error on line {}: {msg}", self.line))
    }

    // ---- Token (name spaces, matched end elements) -------------------------------------------

    fn token(&mut self) -> Result<Token, XmlError> {
        let t = if let Some(t) = self.next_token.take() {
            t
        } else {
            match self.raw_token() {
                Ok(t) => t,
                Err(e) => {
                    if e == XmlError::Eof && !self.stk.is_empty() {
                        return Err(self.syntax_error("unexpected EOF"));
                    }
                    return Err(e);
                }
            }
        };
        // `AutoClose` is never set by yq, so `autoClose` does nothing.
        match t {
            Token::Start { mut name, mut attr } => {
                for a in &attr {
                    if a.name.space == XMLNS_PREFIX {
                        let old = self.ns.get(&a.name.local).cloned();
                        self.stk.push(Stack::Ns(a.name.local.clone(), old));
                        self.ns.insert(a.name.local.clone(), a.value.clone());
                    }
                    if a.name.space.is_empty() && a.name.local == XMLNS_PREFIX {
                        let old = self.ns.get("").cloned();
                        self.stk.push(Stack::Ns(String::new(), old));
                        self.ns.insert(String::new(), a.value.clone());
                    }
                }
                self.stk.push(Stack::Start(name.clone()));
                self.translate(&mut name, true);
                for a in &mut attr {
                    self.translate(&mut a.name, false);
                }
                Ok(Token::Start { name, attr })
            }
            Token::End(mut name) => {
                if !self.pop_element(&mut name) {
                    return Err(self.err.clone().unwrap_or(XmlError::Eof));
                }
                Ok(Token::End(name))
            }
            other => Ok(other),
        }
    }

    fn translate(&self, n: &mut Name, is_element_name: bool) {
        if n.space == XMLNS_PREFIX {
            return;
        }
        if n.space.is_empty() && !is_element_name {
            return;
        }
        if n.space == XML_PREFIX {
            n.space = XML_URL.to_string();
        } else if n.space.is_empty() && n.local == XMLNS_PREFIX {
            return;
        }
        if let Some(v) = self.ns.get(&n.space) {
            n.space = v.clone();
        }
        // `DefaultSpace` is empty, so an unset prefix stays as it is.
    }

    fn pop_element(&mut self, t: &mut Name) -> bool {
        let s = self.stk.pop();
        let name = t.clone();
        match s {
            Some(Stack::Start(open)) => {
                if open.local != name.local {
                    if !self.strict {
                        self.need_close = true;
                        self.to_close = name;
                        *t = open;
                        return true;
                    }
                    self.err = Some(self.syntax_error(&format!(
                        "element <{}> closed by </{}>",
                        open.local, name.local
                    )));
                    return false;
                }
                if open.space != name.space {
                    let ns = if name.space.is_empty() {
                        "\"\"".to_string()
                    } else {
                        name.space.clone()
                    };
                    self.err = Some(self.syntax_error(&format!(
                        "element <{}> in space {} closed by </{}> in space {}",
                        open.local, open.space, name.local, ns
                    )));
                    return false;
                }
            }
            _ => {
                self.err =
                    Some(self.syntax_error(&format!("unexpected end element </{}>", name.local)));
                return false;
            }
        }
        self.translate(t, true);
        while let Some(Stack::Ns(..)) = self.stk.last() {
            if let Some(Stack::Ns(local, old)) = self.stk.pop() {
                match old {
                    Some(v) => {
                        self.ns.insert(local, v);
                    }
                    None => {
                        self.ns.remove(&local);
                    }
                }
            }
        }
        true
    }

    // ---- rawToken ----------------------------------------------------------------------------

    fn raw_token(&mut self) -> Result<Token, XmlError> {
        if let Some(e) = &self.err {
            return Err(e.clone());
        }
        if self.need_close {
            self.need_close = false;
            return Ok(Token::End(self.to_close.clone()));
        }
        let Some(b) = self.getc() else {
            return Err(self.err.clone().unwrap_or(XmlError::Eof));
        };
        if b != b'<' {
            self.ungetc(b);
            return match self.text(None, false) {
                Some(data) => Ok(Token::CharData(data)),
                None => Err(self.take_err()),
            };
        }
        let Some(b) = self.mustgetc() else {
            return Err(self.take_err());
        };
        match b {
            b'/' => {
                let Some(name) = self.nsname() else {
                    if self.err.is_none() {
                        self.err = Some(self.syntax_error("expected element name after </"));
                    }
                    return Err(self.take_err());
                };
                self.space();
                let Some(b) = self.mustgetc() else {
                    return Err(self.take_err());
                };
                if b != b'>' {
                    self.err = Some(self.syntax_error(&format!(
                        "invalid characters between </{} and >",
                        name.local
                    )));
                    return Err(self.take_err());
                }
                Ok(Token::End(name))
            }
            b'?' => self.proc_inst_token(),
            b'!' => self.bang_token(),
            _ => {
                self.ungetc(b);
                self.start_element()
            }
        }
    }

    fn take_err(&self) -> XmlError {
        self.err.clone().unwrap_or(XmlError::Eof)
    }

    fn proc_inst_token(&mut self) -> Result<Token, XmlError> {
        let Some(target) = self.name() else {
            if self.err.is_none() {
                self.err = Some(self.syntax_error("expected target name after <?"));
            }
            return Err(self.take_err());
        };
        self.space();
        self.buf.clear();
        let mut b0 = 0u8;
        loop {
            let Some(b) = self.mustgetc() else {
                return Err(self.take_err());
            };
            self.buf.push(b);
            if b0 == b'?' && b == b'>' {
                break;
            }
            b0 = b;
        }
        let data = self.buf[..self.buf.len() - 2].to_vec();
        if target == "xml" {
            let content = String::from_utf8_lossy(&data).into_owned();
            let ver = proc_inst("version", &content);
            if !ver.is_empty() && ver != "1.0" {
                self.err = Some(XmlError::Message(format!(
                    "xml: unsupported version {}; only version 1.0 is supported",
                    quote(&ver)
                )));
                return Err(self.take_err());
            }
            let enc = proc_inst("encoding", &content);
            if !enc.is_empty()
                && enc != "utf-8"
                && enc != "UTF-8"
                && !enc.eq_ignore_ascii_case("utf-8")
            {
                let rest = &self.input[self.pos..];
                match charset_reader(&enc, rest) {
                    Some(converted) => {
                        let mut bytes = self.input[..self.pos].to_vec();
                        bytes.extend_from_slice(&converted);
                        self.input = std::borrow::Cow::Owned(bytes);
                    }
                    None => {
                        self.err = Some(XmlError::Message(format!(
                            "xml: opening charset {}: unsupported charset: {}",
                            quote(&enc),
                            quote(&enc)
                        )));
                        return Err(self.take_err());
                    }
                }
            }
        }
        Ok(Token::ProcInst { target, inst: data })
    }

    fn bang_token(&mut self) -> Result<Token, XmlError> {
        let Some(b) = self.mustgetc() else {
            return Err(self.take_err());
        };
        match b {
            b'-' => {
                let Some(b) = self.mustgetc() else {
                    return Err(self.take_err());
                };
                if b != b'-' {
                    self.err = Some(self.syntax_error("invalid sequence <!- not part of <!--"));
                    return Err(self.take_err());
                }
                self.buf.clear();
                let (mut b0, mut b1) = (0u8, 0u8);
                loop {
                    let Some(b) = self.mustgetc() else {
                        return Err(self.take_err());
                    };
                    self.buf.push(b);
                    if b0 == b'-' && b1 == b'-' {
                        if b != b'>' {
                            self.err =
                                Some(self.syntax_error(
                                    "invalid sequence \"--\" not allowed in comments",
                                ));
                            return Err(self.take_err());
                        }
                        break;
                    }
                    b0 = b1;
                    b1 = b;
                }
                let data = self.buf[..self.buf.len() - 3].to_vec();
                Ok(Token::Comment(data))
            }
            b'[' => {
                for expected in b"CDATA[" {
                    let Some(b) = self.mustgetc() else {
                        return Err(self.take_err());
                    };
                    if b != *expected {
                        self.err = Some(self.syntax_error("invalid <![ sequence"));
                        return Err(self.take_err());
                    }
                }
                match self.text(None, true) {
                    Some(data) => Ok(Token::CharData(data)),
                    None => Err(self.take_err()),
                }
            }
            _ => self.directive(b),
        }
    }

    /// A directive: `<!DOCTYPE ...>`, `<!ENTITY ...>`; `first` is the byte after `<!`.
    fn directive(&mut self, first: u8) -> Result<Token, XmlError> {
        self.buf.clear();
        self.buf.push(first);
        let mut inquote = 0u8;
        let mut depth = 0i64;
        loop {
            let Some(mut b) = self.mustgetc() else {
                return Err(self.take_err());
            };
            if inquote == 0 && b == b'>' && depth == 0 {
                break;
            }
            // `HandleB:`
            loop {
                self.buf.push(b);
                if b == inquote {
                    inquote = 0;
                } else if inquote != 0 {
                    // in quotes, no special action
                } else if b == b'\'' || b == b'"' {
                    inquote = b;
                } else if b == b'>' && inquote == 0 {
                    depth -= 1;
                } else if b == b'<' && inquote == 0 {
                    let s = b"!--";
                    let mut restart = false;
                    for (i, expected) in s.iter().enumerate() {
                        let Some(next) = self.mustgetc() else {
                            return Err(self.take_err());
                        };
                        if next != *expected {
                            for item in s.iter().take(i) {
                                self.buf.push(*item);
                            }
                            depth += 1;
                            b = next;
                            restart = true;
                            break;
                        }
                    }
                    if restart {
                        continue;
                    }
                    // Remove the `<` written above.
                    self.buf.pop();
                    let (mut b0, mut b1) = (0u8, 0u8);
                    loop {
                        let Some(next) = self.mustgetc() else {
                            return Err(self.take_err());
                        };
                        if b0 == b'-' && b1 == b'-' && next == b'>' {
                            break;
                        }
                        b0 = b1;
                        b1 = next;
                    }
                    self.buf.push(b' ');
                }
                break;
            }
        }
        Ok(Token::Directive(self.buf.clone()))
    }

    fn start_element(&mut self) -> Result<Token, XmlError> {
        let Some(name) = self.nsname() else {
            if self.err.is_none() {
                self.err = Some(self.syntax_error("expected element name after <"));
            }
            return Err(self.take_err());
        };
        let mut empty = false;
        let mut attr = Vec::new();
        loop {
            self.space();
            let Some(b) = self.mustgetc() else {
                return Err(self.take_err());
            };
            if b == b'/' {
                empty = true;
                let Some(b) = self.mustgetc() else {
                    return Err(self.take_err());
                };
                if b != b'>' {
                    self.err = Some(self.syntax_error("expected /> in element"));
                    return Err(self.take_err());
                }
                break;
            }
            if b == b'>' {
                break;
            }
            self.ungetc(b);

            let Some(attr_name) = self.nsname() else {
                if self.err.is_none() {
                    self.err = Some(self.syntax_error("expected attribute name in element"));
                }
                return Err(self.take_err());
            };
            self.space();
            let Some(b) = self.mustgetc() else {
                return Err(self.take_err());
            };
            let value;
            if b != b'=' {
                if self.strict {
                    self.err = Some(self.syntax_error("attribute name without = in element"));
                    return Err(self.take_err());
                }
                self.ungetc(b);
                value = attr_name.local.clone();
            } else {
                self.space();
                let Some(data) = self.attrval() else {
                    return Err(self.take_err());
                };
                value = String::from_utf8_lossy(&data).into_owned();
            }
            attr.push(Attr {
                name: attr_name,
                value,
            });
        }
        if empty {
            self.need_close = true;
            self.to_close = name.clone();
        }
        Ok(Token::Start { name, attr })
    }

    fn attrval(&mut self) -> Option<Vec<u8>> {
        let b = self.mustgetc()?;
        if b == b'"' || b == b'\'' {
            return self.text(Some(b), false);
        }
        if self.strict {
            self.err = Some(self.syntax_error("unquoted or missing attribute value in element"));
            return None;
        }
        self.ungetc(b);
        self.buf.clear();
        loop {
            let b = self.mustgetc()?;
            if b.is_ascii_alphanumeric() || b == b'_' || b == b':' || b == b'-' {
                self.buf.push(b);
            } else {
                self.ungetc(b);
                break;
            }
        }
        Some(self.buf.clone())
    }

    // ---- bytes -------------------------------------------------------------------------------

    fn space(&mut self) {
        while let Some(b) = self.getc() {
            match b {
                b' ' | b'\r' | b'\n' | b'\t' => {}
                _ => {
                    self.ungetc(b);
                    return;
                }
            }
        }
    }

    fn getc(&mut self) -> Option<u8> {
        if self.err.is_some() {
            return None;
        }
        let b = if let Some(b) = self.next_byte.take() {
            b
        } else if self.pos < self.input.len() {
            let b = self.input[self.pos];
            self.pos += 1;
            b
        } else {
            self.err = Some(XmlError::Eof);
            return None;
        };
        if b == b'\n' {
            self.line += 1;
        }
        Some(b)
    }

    fn mustgetc(&mut self) -> Option<u8> {
        let b = self.getc();
        if b.is_none() && self.err == Some(XmlError::Eof) {
            self.err = Some(self.syntax_error("unexpected EOF"));
        }
        b
    }

    fn ungetc(&mut self, b: u8) {
        if b == b'\n' {
            self.line -= 1;
        }
        self.next_byte = Some(b);
    }

    /// Plain text, up to a `<`; with `quote`, up to the matching quote; with `cdata`, up to `]]>`.
    fn text(&mut self, quote: Option<u8>, cdata: bool) -> Option<Vec<u8>> {
        let (mut b0, mut b1) = (0u8, 0u8);
        let mut trunc = 0usize;
        self.buf.clear();
        loop {
            let Some(b) = self.getc() else {
                if cdata {
                    if self.err == Some(XmlError::Eof) {
                        self.err = Some(self.syntax_error("unexpected EOF in CDATA section"));
                    }
                    return None;
                }
                break;
            };
            if quote.is_none() && b0 == b']' && b1 == b']' && b == b'>' {
                if cdata {
                    trunc = 2;
                    break;
                }
                self.err = Some(self.syntax_error("unescaped ]]> not in CDATA section"));
                return None;
            }
            if b == b'<' && !cdata {
                if quote.is_some() {
                    self.err = Some(self.syntax_error("unescaped < inside quoted string"));
                    return None;
                }
                self.ungetc(b'<');
                break;
            }
            if quote == Some(b) {
                break;
            }
            if b == b'&' && !cdata {
                let before = self.buf.len();
                self.buf.push(b'&');
                let mut text: Option<String> = None;
                let mut b = self.mustgetc()?;
                if b == b'#' {
                    self.buf.push(b);
                    b = self.mustgetc()?;
                    let mut base = 10u32;
                    if b == b'x' {
                        base = 16;
                        self.buf.push(b);
                        b = self.mustgetc()?;
                    }
                    let start = self.buf.len();
                    while b.is_ascii_digit()
                        || (base == 16 && (b'a'..=b'f').contains(&b))
                        || (base == 16 && (b'A'..=b'F').contains(&b))
                    {
                        self.buf.push(b);
                        b = self.mustgetc()?;
                    }
                    if b != b';' {
                        self.ungetc(b);
                    } else {
                        let digits = String::from_utf8_lossy(&self.buf[start..]).into_owned();
                        self.buf.push(b';');
                        if let Ok(n) = u64::from_str_radix(&digits, base) {
                            if n <= 0x10FFFF {
                                // `string(rune(n))`: a surrogate becomes U+FFFD.
                                text = Some(
                                    char::from_u32(n as u32).unwrap_or('\u{FFFD}').to_string(),
                                );
                            }
                        }
                    }
                } else {
                    self.ungetc(b);
                    if !self.read_name() && self.err.is_some() {
                        return None;
                    }
                    b = self.mustgetc()?;
                    if b != b';' {
                        self.ungetc(b);
                    } else {
                        let name = self.buf[before + 1..].to_vec();
                        self.buf.push(b';');
                        if is_name(&name) {
                            text = match name.as_slice() {
                                b"lt" => Some("<".to_string()),
                                b"gt" => Some(">".to_string()),
                                b"amp" => Some("&".to_string()),
                                b"apos" => Some("'".to_string()),
                                b"quot" => Some("\"".to_string()),
                                _ => None,
                            };
                        }
                    }
                }
                if let Some(text) = text {
                    self.buf.truncate(before);
                    self.buf.extend_from_slice(text.as_bytes());
                    b0 = 0;
                    b1 = 0;
                    continue;
                }
                if !self.strict {
                    b0 = 0;
                    b1 = 0;
                    continue;
                }
                let mut ent = String::from_utf8_lossy(&self.buf[before..]).into_owned();
                if !ent.ends_with(';') {
                    ent.push_str(" (no semicolon)");
                }
                self.err = Some(self.syntax_error(&format!("invalid character entity {ent}")));
                return None;
            }

            // An unescaped `\r` and `\r\n` are a `\n`.
            if b == b'\r' {
                self.buf.push(b'\n');
            } else if b1 == b'\r' && b == b'\n' {
                // Skip `\r\n`: the `\n` is already written.
            } else {
                self.buf.push(b);
            }
            b0 = b1;
            b1 = b;
        }
        let data = &self.buf[..self.buf.len() - trunc];
        // Inspect each rune for being a disallowed character.
        let mut rest = data;
        while !rest.is_empty() {
            match decode_rune(rest) {
                Some((c, size)) => {
                    if !is_in_character_range(c) {
                        self.err =
                            Some(self.syntax_error(&format!("illegal character code U+{c:04X}")));
                        return None;
                    }
                    rest = &rest[size..];
                }
                None => {
                    self.err = Some(self.syntax_error("invalid UTF-8"));
                    return None;
                }
            }
        }
        Some(data.to_vec())
    }

    fn nsname(&mut self) -> Option<Name> {
        let s = self.name()?;
        if s.matches(':').count() > 1 {
            return None;
        }
        match s.split_once(':') {
            Some((space, local)) if !space.is_empty() && !local.is_empty() => Some(Name {
                space: space.to_string(),
                local: local.to_string(),
            }),
            _ => Some(Name {
                space: String::new(),
                local: s,
            }),
        }
    }

    fn name(&mut self) -> Option<String> {
        self.buf.clear();
        if !self.read_name() {
            return None;
        }
        let b = self.buf.clone();
        if !is_name(&b) {
            self.err = Some(self.syntax_error(&format!(
                "invalid XML name: {}",
                String::from_utf8_lossy(&b)
            )));
            return None;
        }
        Some(String::from_utf8_lossy(&b).into_owned())
    }

    fn read_name(&mut self) -> bool {
        let Some(b) = self.mustgetc() else {
            return false;
        };
        if b < 0x80 && !is_name_byte(b) {
            self.ungetc(b);
            return false;
        }
        self.buf.push(b);
        loop {
            let Some(b) = self.mustgetc() else {
                return false;
            };
            if b < 0x80 && !is_name_byte(b) {
                self.ungetc(b);
                break;
            }
            self.buf.push(b);
        }
        true
    }
}

/// `utf8.DecodeRune`: the first character and its width, or `None` for an invalid sequence.
fn decode_rune(bytes: &[u8]) -> Option<(u32, usize)> {
    let width = match bytes[0] {
        0x00..=0x7F => 1,
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => return None,
    };
    let chunk = bytes.get(..width)?;
    let text = std::str::from_utf8(chunk).ok()?;
    text.chars().next().map(|c| (c as u32, width))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(input: &str, strict: bool, raw: bool) -> Result<Vec<Token>, String> {
        let mut decoder = Decoder::new(
            input.as_bytes(),
            LexerOptions {
                strict,
                raw_token: raw,
            },
        );
        let mut out = Vec::new();
        loop {
            match decoder.next() {
                Ok(Some(token)) => out.push(token),
                Ok(None) => return Ok(out),
                Err(e) => return Err(e.message()),
            }
        }
    }

    fn lenient(input: &str) -> Result<Vec<Token>, String> {
        tokens(input, false, true)
    }

    fn texts(input: &str) -> Vec<String> {
        lenient(input)
            .unwrap()
            .into_iter()
            .filter_map(|t| match t {
                Token::CharData(d) => Some(String::from_utf8(d).unwrap()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_ascii_fast_path_is_the_table() {
        for c in 0u32..0x80 {
            assert_eq!(is_name_start(c), in_table(NAME_FIRST, c), "start {c:#x}");
            assert_eq!(is_name_char(c), in_table(NAME_SECOND, c), "char {c:#x}");
        }
    }

    #[test]
    fn names_follow_the_xml_tables() {
        assert!(is_name(b"a-b.c:d"));
        assert!(is_name("é".as_bytes()));
        assert!(!is_name(b""));
        assert!(!is_name(b"1a"));
        assert!(!is_name(b"-a"));
        assert!(!is_name(b"a\xa9"));
        assert!(!is_name("a\u{200b}".as_bytes()));
    }

    #[test]
    fn text_is_trimmed_to_its_graphic_ends() {
        assert_eq!(trim_non_graphic("  hi  "), "hi");
        assert_eq!(
            trim_non_graphic("\u{a0}\u{200b}zw\u{200b}\u{a0}nbsp\u{a0}"),
            "zw\u{200b}\u{a0}nbsp"
        );
        assert_eq!(trim_non_graphic(" \n\t"), "");
        assert_eq!(trim_non_graphic(""), "");
        assert_eq!(trim_non_graphic("é"), "é");
    }

    #[test]
    fn a_processing_instruction_value_is_found_as_go_finds_it() {
        assert_eq!(proc_inst("version", "version=\"1.0\" encoding='x'"), "1.0");
        assert_eq!(proc_inst("encoding", "version=\"1.0\" encoding='x'"), "x");
        assert_eq!(proc_inst("encoding", "encoding=x encoding=\"y\""), "y");
        assert_eq!(proc_inst("encoding", "encoding="), "");
        assert_eq!(proc_inst("encoding", "encoding=\"é"), "");
        assert_eq!(proc_inst("encoding", "é encoding=\"é\""), "é");
        assert_eq!(proc_inst("encoding", ""), "");
    }

    #[test]
    fn character_references_and_entities() {
        assert_eq!(
            texts("a&lt;&gt;&amp;&apos;&quot;&#65;&#x42;z"),
            ["a<>&'\"ABz"]
        );
        assert_eq!(
            texts("&bogus;&amp&#;&#xZZ;&#99999999999;&#xD800;"),
            ["&bogus;&amp&#;&#xZZ;&#99999999999;\u{fffd}"]
        );
        assert_eq!(texts("a\r\nb\rc"), ["a\nb\nc"]);
        assert_eq!(
            tokens("&bogus;", true, true).unwrap_err(),
            "XML syntax error on line 1: invalid character entity &bogus;"
        );
        assert_eq!(
            tokens("&amp", true, true).unwrap_err(),
            "XML syntax error on line 1: unexpected EOF"
        );
    }

    #[test]
    fn markup_errors_name_their_line() {
        let error = |input: &str| lenient(input).unwrap_err();
        assert_eq!(error("<a\n"), "XML syntax error on line 2: unexpected EOF");
        assert_eq!(
            error("<a b=\"1"),
            "XML syntax error on line 1: unexpected EOF"
        );
        assert_eq!(
            error("<!- x>"),
            "XML syntax error on line 1: invalid sequence <!- not part of <!--"
        );
        assert_eq!(
            error("<!-- a -- b -->"),
            "XML syntax error on line 1: invalid sequence \"--\" not allowed in comments"
        );
        assert_eq!(
            error("<![XDATA[x]]>"),
            "XML syntax error on line 1: invalid <![ sequence"
        );
        assert_eq!(
            error("<![CDATA[x"),
            "XML syntax error on line 1: unexpected EOF in CDATA section"
        );
        assert_eq!(
            error("x ]]> y"),
            "XML syntax error on line 1: unescaped ]]> not in CDATA section"
        );
        assert_eq!(
            error("<a b=\"<\">"),
            "XML syntax error on line 1: unescaped < inside quoted string"
        );
        assert_eq!(
            error("\u{1}"),
            "XML syntax error on line 1: illegal character code U+0001"
        );
        assert_eq!(
            error("< a/>"),
            "XML syntax error on line 1: expected element name after <"
        );
        assert_eq!(
            error("</>"),
            "XML syntax error on line 1: expected element name after </"
        );
        assert_eq!(
            error("</a b>"),
            "XML syntax error on line 1: invalid characters between </a and >"
        );
        assert_eq!(
            error("<a/ >"),
            "XML syntax error on line 1: expected /> in element"
        );
        assert_eq!(
            error("<a b='1' ='2'>"),
            "XML syntax error on line 1: expected attribute name in element"
        );
        assert_eq!(
            error("<?>"),
            "XML syntax error on line 1: expected target name after <?"
        );
        assert_eq!(
            error("<a:b:c/>"),
            "XML syntax error on line 1: expected element name after <"
        );
        assert_eq!(
            error("<1a/>"),
            "XML syntax error on line 1: invalid XML name: 1a"
        );
        assert_eq!(
            error("<?xml version=\"1.1\"?>"),
            "xml: unsupported version \"1.1\"; only version 1.0 is supported"
        );
        assert_eq!(
            error("<?xml version=\"1.0\" encoding=\"x-nope\"?>"),
            "xml: opening charset \"x-nope\": unsupported charset: \"x-nope\""
        );
    }

    #[test]
    fn lenient_mode_accepts_what_strict_mode_refuses() {
        assert_eq!(lenient("<a b=c d/>").unwrap().len(), 2);
        assert_eq!(
            tokens("<a b=c/>", true, true).unwrap_err(),
            "XML syntax error on line 1: unquoted or missing attribute value in element"
        );
        assert_eq!(
            tokens("<a b/>", true, true).unwrap_err(),
            "XML syntax error on line 1: attribute name without = in element"
        );
        assert!(tokens("<a></b>", false, true).is_ok());
    }

    #[test]
    fn directives_keep_nested_brackets_quotes_and_comments() {
        let tokens = lenient("<!DOCTYPE a [<!ENTITY e 'x>y'><!-- c -->]>").unwrap();
        let Token::Directive(data) = &tokens[0] else {
            panic!("not a directive: {tokens:?}");
        };
        assert_eq!(
            String::from_utf8_lossy(data),
            "DOCTYPE a [<!ENTITY e 'x>y'> ]"
        );
        assert!(lenient("<!DOCTYPE a [<x").is_err());
        assert!(lenient("<!DOCTYPE a <!- b>").is_err());
        assert!(lenient("<!DOCTYPE a <!- b>>").is_ok());
    }

    #[test]
    fn token_mode_matches_ends_and_keeps_namespaces_apart() {
        let strict = |input: &str| tokens(input, true, false);
        assert_eq!(
            strict("<a></b>").unwrap_err(),
            "XML syntax error on line 1: element <a> closed by </b>"
        );
        assert_eq!(
            strict("</a>").unwrap_err(),
            "XML syntax error on line 1: unexpected end element </a>"
        );
        assert_eq!(
            strict("<a>").unwrap_err(),
            "XML syntax error on line 1: unexpected EOF"
        );
        assert_eq!(strict("<x:a xmlns:x='u'></x:a>").map(|t| t.len()), Ok(2));
        let translated = strict("<x:a xmlns:x='u' x:b='1'></x:a>").unwrap();
        let Token::Start { name, attr } = &translated[0] else {
            panic!("not a start: {translated:?}");
        };
        assert_eq!((name.space.as_str(), name.local.as_str()), ("u", "a"));
        assert_eq!(attr[1].name.space, "u");
        assert_eq!(strict("<x:a xmlns:x='u'></x:a xmlns:x='v'>").is_err(), true);
        assert_eq!(
            strict("<a xmlns:x='u'><x:b></y:b></a>").unwrap_err(),
            "XML syntax error on line 1: element <b> in space x closed by </b> in space y"
        );
        // Lenient `Token` closes the element the end tag does not name.
        assert_eq!(tokens("<a><b></a>", false, false).map(|t| t.len()), Ok(4));
        let xml_space = tokens("<a xml:lang='en'/>", false, false).unwrap();
        let Token::Start { attr, .. } = &xml_space[0] else {
            panic!("not a start");
        };
        assert_eq!(attr[0].name.space, XML_URL);
    }

    #[test]
    fn declared_charsets_are_opened_by_label() {
        assert_eq!(
            charset_reader(" Latin1 ", b"caf\xe9").unwrap(),
            "café".as_bytes()
        );
        assert_eq!(
            charset_reader("windows-1251", b"\xcf").unwrap(),
            "П".as_bytes()
        );
        assert_eq!(
            charset_reader("utf8", b"a\xffb\xc3").unwrap(),
            "a\u{fffd}b\u{fffd}".as_bytes()
        );
        assert!(charset_reader("shift_jis", b"x").is_none());
        let converted =
            lenient("<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><a>\u{e9}</a>").unwrap();
        assert!(converted
            .iter()
            .any(|t| matches!(t, Token::CharData(d) if d == "Ã©".as_bytes())));
    }

    #[test]
    fn go_quote_wraps_the_message_text() {
        assert_eq!(quote("a\"b"), "\"a\\\"b\"");
    }
}
