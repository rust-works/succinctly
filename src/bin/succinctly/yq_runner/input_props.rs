//! yq's `-p props` input format (#6925): `decoder_properties.go` over magiconair/properties
//! v1.8.10, transcoded to the YAML text yq prints for the same nodes.
//!
//! yq loads the file with `properties.LoadString`, then for each key in file order splits it on
//! `.` into a path (a segment `strconv.ParseInt` reads as an `int32` is an array index, and under
//! `--properties-array-brackets` a `name[0][1]` segment is a name and two indexes), assigns the
//! key's comment lines to the node the path names, and assigns the value there as a `!!str`. Both
//! assignments go through yq's own traversal, so its rules apply: a `null` is made a mapping or a
//! sequence by the segment that first indexes it, a segment is a key glob (`a*` matches every
//! existing key it fits), a string or other scalar is never indexed into (the assignment matches
//! nothing), and an array index is read by yq's `parseInt` (`0x1`, `1_0`).
//!
//! The nodes are then written as yq's YAML encoder writes them: block style, two-space indent,
//! a sequence indented inside its mapping, each string in the style go-yaml picks for a string
//! with no style of its own ([`go_yaml_dom_scalar`]), and each comment as a head comment. The run
//! reads that text as `-p yaml`, so every route after the read is the YAML route's.

use std::collections::HashMap;

use succinctly::jq::{go_quote_bytes, yq_match_key, yq_parse_int64};
use succinctly::yaml::{
    go_yaml_dom_scalar, go_yaml_explicit_key_scalar, go_yaml_key_is_simple,
    go_yaml_multiline_scalar,
};

/// The most sequence elements one document builds, padding included: 2^24. yq pads an index
/// past the end with `null` nodes of about 545 bytes each (`e.4000000=x` peaks at 2.2 GB), so it
/// needs some 9 GB at this bound and dies on the allocation well past it; this refuses instead,
/// with the message a yq-mode write past its own bound prints.
const MAX_SEQUENCE_ELEMENTS: usize = 1 << 24;

/// The most bytes the `${key}` expansion check may produce. magiconair expands every value once
/// at load time only to find a malformed or circular reference (yq never reads the expansions),
/// and a chain of values that each repeat the next one twice doubles the work per link, so this
/// bounds the check rather than let a small file run without end.
const MAX_EXPANSION_BYTES: usize = 1 << 28;

/// `raw` as the YAML text of the document yq decodes it to, or yq's message for a file that does
/// not load (printed after `bad file '<name>': `). An empty input holds no document, which is the
/// empty text.
pub(super) fn transcode(raw: &[u8], array_brackets: bool) -> Result<Vec<u8>, String> {
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    let input = go_runes(raw);
    let properties = parse(&input)?;
    properties.check()?;
    let mut root = Node::Map(Vec::new());
    let mut budget = Budget::default();
    for key in &properties.keys {
        apply_property(&mut root, &properties, key, array_brackets, &mut budget)?;
    }
    let mut out = String::new();
    match &root {
        Node::Map(entries) if !entries.is_empty() => render_map(&mut out, entries, ""),
        _ => out.push_str("{}\n"),
    }
    Ok(out.into_bytes())
}

/// The input as Go's `utf8.DecodeRuneInString` steps through it: every byte that does not start
/// a valid sequence is one U+FFFD.
fn go_runes(raw: &[u8]) -> Vec<char> {
    let mut out = Vec::with_capacity(raw.len());
    let mut rest = raw;
    while !rest.is_empty() {
        match std::str::from_utf8(rest) {
            Ok(valid) => {
                out.extend(valid.chars());
                break;
            }
            Err(error) => {
                let (valid, after) = rest.split_at(error.valid_up_to());
                // `valid_up_to` bytes are UTF-8 by the error's own account.
                out.extend(std::str::from_utf8(valid).unwrap_or_default().chars());
                let invalid = error.error_len().unwrap_or(after.len());
                out.extend(std::iter::repeat('\u{FFFD}').take(invalid));
                rest = &after[invalid..];
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// magiconair/properties: lex.go and parser.go
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ItemType {
    Error,
    Eof,
    Key,
    Value,
    Comment,
}

#[derive(Debug)]
struct Item {
    typ: ItemType,
    /// The item's start, in runes (magiconair counts bytes, but only to count the `\n`s before it).
    pos: usize,
    val: String,
}

impl Item {
    /// `item.String()`: a value over ten bytes is quoted after its first ten runes, with `...`.
    fn describe(&self) -> String {
        match self.typ {
            ItemType::Eof => String::from("EOF"),
            ItemType::Error => self.val.clone(),
            _ if self.val.len() > 10 => {
                let head: String = self.val.chars().take(10).collect();
                format!("{}...", go_quote_bytes(head.as_bytes()))
            }
            _ => go_quote_bytes(self.val.as_bytes()),
        }
    }
}

#[derive(Clone, Copy)]
enum State {
    BeforeKey,
    Comment,
    Key,
    BeforeValue,
    Value,
    Done,
}

const WHITESPACE: &[char] = &[' ', '\x0c', '\t'];

struct Lexer<'a> {
    input: &'a [char],
    pos: usize,
    start: usize,
    /// Whether the last `next` read a rune (`width` in lex.go: `backup` after the end is a no-op).
    width: usize,
    runes: String,
    items: Vec<Item>,
}

impl Lexer<'_> {
    fn next(&mut self) -> Option<char> {
        let Some(&c) = self.input.get(self.pos) else {
            self.width = 0;
            return None;
        };
        self.width = 1;
        self.pos += 1;
        Some(c)
    }

    fn backup(&mut self) {
        self.pos -= self.width;
    }

    fn peek(&mut self) -> Option<char> {
        let c = self.next();
        self.backup();
        c
    }

    fn emit(&mut self, typ: ItemType) {
        let val = std::mem::take(&mut self.runes);
        self.items.push(Item {
            typ,
            pos: self.start,
            val,
        });
        self.start = self.pos;
    }

    fn ignore(&mut self) {
        self.start = self.pos;
    }

    fn accept(&mut self, valid: &[char]) -> bool {
        if self.next().is_some_and(|c| valid.contains(&c)) {
            return true;
        }
        self.backup();
        false
    }

    fn accept_run(&mut self, valid: &[char]) {
        while self.next().is_some_and(|c| valid.contains(&c)) {}
        self.backup();
    }

    fn error(&mut self, message: &str) -> State {
        self.items.push(Item {
            typ: ItemType::Error,
            pos: self.start,
            val: String::from(message),
        });
        State::Done
    }

    fn before_key(&mut self) -> State {
        match self.next() {
            None => {
                self.emit(ItemType::Eof);
                State::Done
            }
            Some('\n' | '\r') => {
                self.ignore();
                State::BeforeKey
            }
            Some('#' | '!') => State::Comment,
            Some(c) if WHITESPACE.contains(&c) => {
                self.ignore();
                State::BeforeKey
            }
            Some(_) => {
                self.backup();
                State::Key
            }
        }
    }

    fn comment(&mut self) -> State {
        self.accept_run(WHITESPACE);
        self.ignore();
        loop {
            match self.next() {
                // A comment that ends the input with no line break is dropped.
                None => {
                    self.ignore();
                    self.runes.clear();
                    self.emit(ItemType::Eof);
                    return State::Done;
                }
                Some('\n' | '\r') => {
                    self.emit(ItemType::Comment);
                    return State::BeforeKey;
                }
                Some(c) => self.runes.push(c),
            }
        }
    }

    fn key(&mut self) -> State {
        let last = loop {
            let r = self.next();
            match r {
                Some('\\') => {
                    if let Err(message) = self.escape() {
                        return self.error(message);
                    }
                }
                Some(' ' | '\x0c' | '\t' | '\r' | '\n' | ':' | '=') => {
                    self.backup();
                    break r;
                }
                None => break r,
                Some(c) => self.runes.push(c),
            }
        };
        if !self.runes.is_empty() {
            self.emit(ItemType::Key);
        }
        if last.is_none() {
            self.emit(ItemType::Eof);
            return State::Done;
        }
        State::BeforeValue
    }

    fn before_value(&mut self) -> State {
        self.accept_run(WHITESPACE);
        self.accept(&[':', '=']);
        self.accept_run(WHITESPACE);
        self.ignore();
        State::Value
    }

    fn value(&mut self) -> State {
        loop {
            match self.next() {
                Some('\\') => {
                    if matches!(self.peek(), Some('\n' | '\r')) {
                        // A line continuation: one break, then the next line's indent.
                        self.next();
                        self.accept_run(WHITESPACE);
                    } else if let Err(message) = self.escape() {
                        return self.error(message);
                    }
                }
                Some('\n' | '\r') => {
                    self.emit(ItemType::Value);
                    self.ignore();
                    return State::BeforeKey;
                }
                None => {
                    self.emit(ItemType::Value);
                    self.emit(ItemType::Eof);
                    return State::Done;
                }
                Some(c) => self.runes.push(c),
            }
        }
    }

    /// `scanEscapeSequence`, after the `\`.
    fn escape(&mut self) -> Result<(), &'static str> {
        match self.next() {
            Some(c @ (' ' | ':' | '=')) => self.runes.push(c),
            Some('f') => self.runes.push('\x0c'),
            Some('n') => self.runes.push('\n'),
            Some('r') => self.runes.push('\r'),
            Some('t') => self.runes.push('\t'),
            Some('u') => {
                let mut code = 0u32;
                for _ in 0..4 {
                    let digit = self.next().and_then(|c| c.to_digit(16));
                    let Some(digit) = digit else {
                        return Err("invalid unicode literal");
                    };
                    code = code * 16 + digit;
                }
                // A surrogate is no rune; Go's `string(runes)` writes it as U+FFFD.
                self.runes.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
            }
            None => return Err("premature EOF"),
            // Any other escaped rune is itself, the `\` dropped.
            Some(c) => self.runes.push(c),
        }
        Ok(())
    }
}

/// Every item of `input`, through the first `EOF` or error.
fn lex(input: &[char]) -> Vec<Item> {
    let mut lexer = Lexer {
        input,
        pos: 0,
        start: 0,
        width: 0,
        runes: String::new(),
        items: Vec::new(),
    };
    let mut state = State::BeforeKey;
    loop {
        state = match state {
            State::BeforeKey => lexer.before_key(),
            State::Comment => lexer.comment(),
            State::Key => lexer.key(),
            State::BeforeValue => lexer.before_value(),
            State::Value => lexer.value(),
            State::Done => return lexer.items,
        };
    }
}

/// A loaded file: its keys in the order each first appears, the last value of each, and the
/// comment lines before the last occurrence that had any.
struct Properties {
    keys: Vec<String>,
    values: HashMap<String, String>,
    comments: HashMap<String, Vec<String>>,
}

/// parser.go's `parse`.
fn parse(input: &[char]) -> Result<Properties, String> {
    let items = lex(input);
    let mut items = items.iter();
    let mut properties = Properties {
        keys: Vec::new(),
        values: HashMap::new(),
        comments: HashMap::new(),
    };
    let mut comments = Vec::new();
    let unexpected = |item: &Item| {
        // `p.errorf`: the line is the one the offending item starts on, and the whole message is
        // then a `fmt.Errorf` format with no arguments.
        let line = 1 + input[..item.pos].iter().filter(|&&c| c == '\n').count();
        go_format_without_arguments(&format!("properties: Line {line}: {}", item.describe()))
    };
    loop {
        // The lexer ends every run with an `EOF` or an error, which ends this loop first.
        let Some(item) = items.next() else {
            return Ok(properties);
        };
        let key = match item.typ {
            ItemType::Eof => return Ok(properties),
            ItemType::Comment => {
                comments.push(item.val.clone());
                continue;
            }
            ItemType::Key => item.val.clone(),
            _ => return Err(unexpected(item)),
        };
        if !properties.values.contains_key(&key) {
            properties.keys.push(key.clone());
        }
        let Some(item) = items.next() else {
            properties.values.insert(key, String::new());
            return Ok(properties);
        };
        if !comments.is_empty() {
            properties
                .comments
                .insert(key.clone(), std::mem::take(&mut comments));
        }
        match item.typ {
            ItemType::Eof => {
                properties.values.insert(key, String::new());
                return Ok(properties);
            }
            ItemType::Value => {
                properties.values.insert(key, item.val.clone());
            }
            _ => return Err(unexpected(item)),
        }
    }
}

impl Properties {
    /// `p.check()`: every value expanded once, for its error. Go walks its map in a random order,
    /// so of several bad values yq names any one; this names the first in file order.
    fn check(&self) -> Result<(), String> {
        let mut budget = MAX_EXPANSION_BYTES;
        for key in &self.keys {
            let value = self.values.get(key).map_or("", String::as_str);
            let mut stack = vec![key.clone()];
            self.expand(value, &mut stack, &mut budget)?;
        }
        Ok(())
    }

    /// properties.go's `expand`, with `${` and `}` around a key, which names another value or,
    /// failing that, an environment variable.
    fn expand(
        &self,
        s: &str,
        keys: &mut Vec<String>,
        budget: &mut usize,
    ) -> Result<String, String> {
        if keys.len() > 64 {
            return Err(String::from("expansion too deep"));
        }
        let mut s = String::from(s);
        loop {
            let Some(start) = s.find("${") else {
                return Ok(s);
            };
            let key_start = start + 2;
            let Some(key_len) = s[key_start..].find('}') else {
                return Err(String::from("malformed expression"));
            };
            let key = &s[key_start..key_start + key_len];
            if keys.iter().any(|k| k == key) {
                let mut message = String::from("circular reference in:\n");
                for k in keys.iter() {
                    let value = self.values.get(k).map_or("", String::as_str);
                    message.push_str(&format!("{k}={value}\n"));
                }
                return Err(go_format_without_arguments(&message));
            }
            let value = match self.values.get(key) {
                Some(value) => value.clone(),
                None => go_getenv(key),
            };
            keys.push(String::from(key));
            let expanded = self.expand(&value, keys, budget);
            keys.pop();
            let expanded = expanded?;
            let end = key_start + key_len + 1;
            s = format!("{}{expanded}{}", &s[..start], &s[end..]);
            *budget = budget
                .checked_sub(s.len())
                .ok_or_else(|| String::from("properties: ${...} expansion too large to check"))?;
        }
    }
}

/// `os.Getenv`: the empty string for a name that is not set, or that no variable can have.
fn go_getenv(name: &str) -> String {
    if name.is_empty() || name.contains(['=', '\0']) {
        return String::new();
    }
    std::env::var_os(name).map_or_else(String::new, |value| value.to_string_lossy().into_owned())
}

/// `fmt.Errorf(format)` with no arguments: `%%` is `%`, and any other verb (its flags, width and
/// precision dropped) is `%!v(MISSING)`; a `%` with no verb after it is `%!(NOVERB)`.
fn go_format_without_arguments(format: &str) -> String {
    let mut out = String::with_capacity(format.len());
    let mut chars = format.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        while chars.next_if(|c| "+-# 0".contains(*c)).is_some() {}
        if chars.next_if_eq(&'*').is_some() {
            out.push_str("%!(BADWIDTH)");
        } else {
            while chars.next_if(char::is_ascii_digit).is_some() {}
        }
        if chars.next_if_eq(&'.').is_some() {
            if chars.next_if_eq(&'*').is_some() {
                out.push_str("%!(BADPREC)");
            } else {
                while chars.next_if(char::is_ascii_digit).is_some() {}
            }
        }
        match chars.next() {
            None => out.push_str("%!(NOVERB)"),
            Some('%') => out.push('%'),
            Some(verb) => {
                out.push_str("%!");
                out.push(verb);
                out.push_str("(MISSING)");
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// decoder_properties.go and the traversal it assigns through
// ---------------------------------------------------------------------------------------------

/// A node of the decoded document.
enum Node {
    Null,
    Str(Box<str>),
    Map(Vec<Entry>),
    Seq(Vec<SeqItem>),
}

/// A mapping entry. Its comment is the key node's head comment, where yq assigns a property's
/// comment lines when the path ends at a mapping key.
struct Entry {
    key: String,
    comment: Box<str>,
    value: Node,
}

/// A sequence element, which carries the comment of a path that ends at its index.
struct SeqItem {
    comment: Box<str>,
    value: Node,
}

/// One segment of a property's path: `fmt.Sprintf("%v", ...)` of it is what the traversal matches.
enum Segment {
    Index(i64),
    Name(String),
}

impl Segment {
    fn text(&self) -> String {
        match self {
            Self::Index(index) => index.to_string(),
            Self::Name(name) => name.clone(),
        }
    }
}

/// What an assignment writes at the end of its path.
enum Write<'a> {
    /// `DeeplyAssign`: the value, as a `!!str`.
    Value(&'a str),
    /// `applyPropertyComments`: the key node (or the element) the path ends at becomes the
    /// path's last segment, carrying the comment when it is not empty.
    Comment { text: String, comment: &'a str },
}

#[derive(Default)]
struct Budget {
    sequence_elements: usize,
}

/// `parsePropKey`.
fn property_path(key: &str, array_brackets: bool) -> Vec<Segment> {
    let mut path = Vec::new();
    for segment in key.split('.') {
        if array_brackets && segment.contains('[') {
            if let Some(bracketed) = bracketed_segment(segment) {
                path.extend(bracketed);
                continue;
            }
        }
        match segment.parse::<i32>() {
            Ok(index) => path.push(Segment::Index(i64::from(index))),
            Err(_) => path.push(Segment::Name(String::from(segment))),
        }
    }
    path
}

/// `parsePropKeyArrayBracketSegment`: `name[0][1]` is the name (unparsed, when there is one) and
/// each bracketed `int32`; anything else is `None`, and the segment is read as one.
fn bracketed_segment(segment: &str) -> Option<Vec<Segment>> {
    let bracket = segment.find('[')?;
    let mut path = Vec::new();
    if bracket > 0 {
        path.push(Segment::Name(String::from(&segment[..bracket])));
    }
    let mut remaining = &segment[bracket..];
    while !remaining.is_empty() {
        let inner = remaining.strip_prefix('[')?;
        let closing = remaining.find(']')?;
        let index = inner[..closing - 1].parse::<i32>().ok()?;
        path.push(Segment::Index(i64::from(index)));
        remaining = &remaining[closing + 1..];
    }
    Some(path)
}

/// `applyProperty`: the comment, then the value. An error assigning the comment skips the
/// property without one, as yq's `return nil` there does.
fn apply_property(
    root: &mut Node,
    properties: &Properties,
    key: &str,
    array_brackets: bool,
    budget: &mut Budget,
) -> Result<(), String> {
    let path = property_path(key, array_brackets);
    if let Some(lines) = properties.comments.get(key) {
        let joined = lines.join("\n");
        let comment = if joined.is_empty() {
            String::new()
        } else {
            format!("# {joined}")
        };
        // `path` is never empty: splitting a key gives at least one segment.
        let text = path.last().map(Segment::text).unwrap_or_default();
        let write = Write::Comment {
            text,
            comment: &comment,
        };
        if assign(root, &path, &write, budget).is_err() {
            return Ok(());
        }
    }
    let value = properties.values.get(key).map_or("", String::as_str);
    assign(root, &path, &Write::Value(value), budget)
}

/// The assignment of `write` at `path` below `node`, through yq's `traverse` for each segment.
fn assign(
    node: &mut Node,
    path: &[Segment],
    write: &Write<'_>,
    budget: &mut Budget,
) -> Result<(), String> {
    let Some((segment, rest)) = path.split_first() else {
        return Ok(());
    };
    let wanted = segment.text();
    // An auto-created `null` becomes what its first index asks for, except for `[]`, which
    // leaves it a scalar that matches nothing.
    if matches!(node, Node::Null) && !matches!(segment, Segment::Name(name) if name == "[]") {
        *node = match segment {
            Segment::Index(_) => Node::Seq(Vec::new()),
            Segment::Name(_) => Node::Map(Vec::new()),
        };
    }
    match node {
        Node::Map(entries) => {
            let mut matched: Vec<usize> = (0..entries.len())
                .filter(|&i| yq_match_key(&entries[i].key, &wanted))
                .collect();
            if matched.is_empty() {
                entries.push(Entry {
                    key: wanted,
                    comment: Box::default(),
                    value: Node::Null,
                });
                matched.push(entries.len() - 1);
            }
            for i in matched {
                let entry = &mut entries[i];
                if !rest.is_empty() {
                    assign(&mut entry.value, rest, write, budget)?;
                    continue;
                }
                match write {
                    Write::Value(value) => entry.value = Node::Str(Box::from(*value)),
                    Write::Comment { text, comment } => {
                        entry.key.clone_from(text);
                        if !comment.is_empty() {
                            entry.comment = Box::from(*comment);
                        }
                    }
                }
            }
            Ok(())
        }
        Node::Seq(items) => {
            let index = seq_index(&wanted)?;
            let len = items.len();
            if let Ok(target) = usize::try_from(index) {
                if target >= len {
                    let grown = target - len + 1;
                    budget.sequence_elements += grown;
                    if budget.sequence_elements > MAX_SEQUENCE_ELEMENTS {
                        return Err(format!(
                            "Cannot grow array to {} elements",
                            target as u64 + 1
                        ));
                    }
                    items.resize_with(target + 1, || SeqItem {
                        comment: Box::default(),
                        value: Node::Null,
                    });
                }
            }
            let len = items.len();
            let position = if index < 0 { len as i64 + index } else { index };
            let Ok(position) = usize::try_from(position) else {
                return Err(format!("index [{index}] out of range, array size is {len}"));
            };
            let item = &mut items[position];
            if !rest.is_empty() {
                return assign(&mut item.value, rest, write, budget);
            }
            match write {
                Write::Value(value) => item.value = Node::Str(Box::from(*value)),
                Write::Comment { text, comment } => {
                    item.value = Node::Str(Box::from(text.as_str()));
                    if !comment.is_empty() {
                        item.comment = Box::from(*comment);
                    }
                }
            }
            Ok(())
        }
        // A string, or a `null` indexed by `[]`: nothing to traverse into.
        Node::Null | Node::Str(_) => Ok(()),
    }
}

/// `traverseArrayWithIndices`' index: yq's `parseInt`, or its error naming the segment.
fn seq_index(wanted: &str) -> Result<i64, String> {
    yq_parse_int64(wanted).ok_or_else(|| {
        format!(
            "cannot index array with '{wanted}' ({})",
            go_parse_int_error(wanted)
        )
    })
}

/// The `strconv.ParseInt` error yq's `parseInt64` gets for `s`, which it does not read: every `_`
/// is removed first, then a `0x`/`0X` (hex) or `0o` (octal) prefix picks the base and is dropped.
fn go_parse_int_error(s: &str) -> String {
    let stripped: String = s.chars().filter(|&c| c != '_').collect();
    let (digits, base) = if let Some(rest) = stripped
        .strip_prefix("0x")
        .or_else(|| stripped.strip_prefix("0X"))
    {
        (rest, 16)
    } else if let Some(rest) = stripped.strip_prefix("0o") {
        (rest, 8)
    } else {
        (stripped.as_str(), 10)
    };
    // `ParseUint` reads left to right and reports the first problem it meets: a digit outside
    // the base is a syntax error, and a value past `u64` before that is a range error. A value
    // that fits `u64` but not `i64` is a range error too.
    let unsigned = digits.strip_prefix(['+', '-']).unwrap_or(digits);
    let mut value: u64 = 0;
    let mut out_of_range = false;
    let mut syntax = unsigned.is_empty();
    for c in unsigned.chars() {
        match c.to_digit(base) {
            None => {
                syntax = true;
                break;
            }
            Some(digit) => {
                match value
                    .checked_mul(u64::from(base))
                    .and_then(|v| v.checked_add(u64::from(digit)))
                {
                    Some(next) => value = next,
                    None => {
                        out_of_range = true;
                        break;
                    }
                }
            }
        }
    }
    let reason = if syntax && !out_of_range {
        "invalid syntax"
    } else {
        "value out of range"
    };
    format!(
        "strconv.ParseInt: parsing {}: {reason}",
        go_quote_bytes(digits.as_bytes())
    )
}

// ---------------------------------------------------------------------------------------------
// The YAML text yq's encoder writes for the nodes
// ---------------------------------------------------------------------------------------------

const STEP: &str = "  ";

fn render_map(out: &mut String, entries: &[Entry], indent: &str) {
    for entry in entries {
        render_comment(out, &entry.comment, indent);
        let child = format!("{indent}{STEP}");
        if go_yaml_key_is_simple(&entry.key) {
            out.push_str(indent);
            out.push_str(&string_scalar(&entry.key, &child, true));
            out.push(':');
            match &entry.value {
                Node::Map(children) if !children.is_empty() => {
                    out.push('\n');
                    render_map(out, children, &child);
                }
                Node::Seq(children) if !children.is_empty() => {
                    out.push('\n');
                    render_seq(out, children, &child);
                }
                value => {
                    out.push(' ');
                    render_inline(out, value, indent);
                }
            }
        } else {
            // go-yaml's `? ` explicit key, for a key with a line break or over 128 bytes.
            out.push_str(indent);
            out.push_str("? ");
            out.push_str(
                &go_yaml_explicit_key_scalar(&entry.key, &child, STEP.len())
                    .unwrap_or_else(|| string_scalar(&entry.key, &child, true)),
            );
            out.push('\n');
            out.push_str(indent);
            out.push_str(": ");
            render_compact(out, &entry.value, indent);
        }
    }
}

fn render_seq(out: &mut String, items: &[SeqItem], indent: &str) {
    for item in items {
        render_comment(out, &item.comment, indent);
        out.push_str(indent);
        out.push_str("- ");
        render_compact(out, &item.value, indent);
    }
}

/// A node written after a `- ` or an explicit key's `: ` at `indent`: a non-empty collection
/// starts on the same line, its first line's indent replaced by the indicator.
fn render_compact(out: &mut String, value: &Node, indent: &str) {
    let child = format!("{indent}{STEP}");
    let mut nested = String::new();
    match value {
        Node::Map(children) if !children.is_empty() => render_map(&mut nested, children, &child),
        Node::Seq(children) if !children.is_empty() => render_seq(&mut nested, children, &child),
        value => {
            render_inline(out, value, indent);
            return;
        }
    }
    out.push_str(&nested[child.len()..]);
}

/// A scalar, or an empty collection, written in place, with its line break.
fn render_inline(out: &mut String, value: &Node, indent: &str) {
    match value {
        Node::Null => out.push_str("null"),
        Node::Str(s) => out.push_str(&string_scalar(s, &format!("{indent}{STEP}"), false)),
        Node::Map(_) => out.push_str("{}"),
        Node::Seq(_) => out.push_str("[]"),
    }
    out.push('\n');
}

/// `s` as go-yaml writes a string with no style of its own; `block_indent` is the indent of a
/// literal block's lines.
fn string_scalar(s: &str, block_indent: &str, is_key: bool) -> String {
    go_yaml_dom_scalar(s, false, true, is_key)
        .or_else(|| go_yaml_multiline_scalar(s, block_indent, STEP.len()))
        .unwrap_or_else(|| succinctly::yaml::go_yaml_double_quoted_scalar(s))
}

/// A head comment as go-yaml writes it: each line `# ` and the text, or the text as it is when it
/// already starts with `#`, and an empty line as a blank one; a final line break ends the last
/// line rather than adding one.
fn render_comment(out: &mut String, comment: &str, indent: &str) {
    if comment.is_empty() {
        return;
    }
    let comment = comment.strip_suffix('\n').unwrap_or(comment);
    for line in comment.split('\n') {
        if line.is_empty() {
            out.push('\n');
            continue;
        }
        out.push_str(indent);
        if !line.starts_with('#') {
            out.push_str("# ");
        }
        out.push_str(line);
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yaml(input: &str) -> Result<String, String> {
        transcode(input.as_bytes(), false).map(|text| String::from_utf8(text).unwrap())
    }

    #[test]
    fn values_paths_and_comments_render_as_yq_prints_them() {
        assert_eq!(
            yaml("# top\na=1\nb.c=true\n# about e\ne.0=x\ne.1=\n").unwrap(),
            "# top\na: \"1\"\nb:\n  c: \"true\"\ne:\n  # about e\n  - x\n  - \"\"\n"
        );
        assert_eq!(yaml("\n\n").unwrap(), "{}\n");
        assert_eq!(yaml("").unwrap(), "");
    }

    #[test]
    fn traversal_follows_yq() {
        assert_eq!(yaml("a=1\na.b=2").unwrap(), "a: \"1\"\n");
        assert_eq!(
            yaml("e.3=x").unwrap(),
            "e:\n  - null\n  - null\n  - null\n  - x\n"
        );
        assert_eq!(
            yaml("abc=1\nabd=2\nab*=9").unwrap(),
            "abc: \"9\"\nabd: \"9\"\n"
        );
        assert_eq!(yaml("x.[]=1").unwrap(), "x: null\n");
        assert_eq!(
            yaml("a.0=x\na.b=y").unwrap_err(),
            "cannot index array with 'b' (strconv.ParseInt: parsing \"b\": invalid syntax)"
        );
        assert_eq!(
            yaml("e.-1=x").unwrap_err(),
            "index [-1] out of range, array size is 0"
        );
        // A comment that cannot be assigned skips its property.
        assert_eq!(
            yaml("a.0=x\n# c\na.b=y\nz=1").unwrap(),
            "a:\n  - x\nz: \"1\"\n"
        );
    }

    #[test]
    fn errors_are_magiconairs() {
        assert_eq!(
            yaml("=50%").unwrap_err(),
            "properties: Line 1: \"50%!\"(MISSING)"
        );
        assert_eq!(
            yaml("a=1\nb=2\n\n=3").unwrap_err(),
            "properties: Line 4: \"3\""
        );
        assert_eq!(
            yaml("k=\\u00zz").unwrap_err(),
            "properties: Line 1: invalid unicode literal"
        );
        assert_eq!(yaml("a=${b").unwrap_err(), "malformed expression");
        assert_eq!(
            yaml("a=${b}\nb=${a}%d").unwrap_err(),
            "circular reference in:\na=${b}\nb=${a}%!d(MISSING)\n"
        );
    }

    #[test]
    fn parse_int_errors_name_what_go_parsed() {
        assert_eq!(
            go_parse_int_error("a_b"),
            "strconv.ParseInt: parsing \"ab\": invalid syntax"
        );
        assert_eq!(
            go_parse_int_error("0xzz"),
            "strconv.ParseInt: parsing \"zz\": invalid syntax"
        );
        assert_eq!(
            go_parse_int_error("99999999999999999999"),
            "strconv.ParseInt: parsing \"99999999999999999999\": value out of range"
        );
    }
}
