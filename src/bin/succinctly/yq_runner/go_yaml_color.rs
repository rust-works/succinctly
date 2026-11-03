//! yq's `-C` colors for YAML output, as yq makes them (#5416).
//!
//! yq does not color by a scheme of its own. `colorizeAndPrint` hands the text it just encoded to
//! go-yaml's `lexer.Tokenize` and prints the tokens with `printer.Printer`, which wraps each
//! *line* of a token's `Origin` text in a prefix and a suffix chosen by the token's type, its
//! neighbours' types, and nothing else. Where an SGR code opens and closes -- on a leading space,
//! across an empty line, around the newline a token carries to the next line's indentation -- is
//! a consequence of how go-yaml's scanner cuts the text into tokens, so a byte-exact match needs
//! that scanner, not a table of colors. This is a port of the part of go-yaml v1.19.2's
//! `scanner` and `token` packages that yq v4.53.3 reaches, with the printer on top.
//!
//! # Streaming
//!
//! The Go scanner reads a whole string. [`YamlColorizer`] feeds the same scanner a line at a
//! time: it scans a line only once the next complete line has arrived, which gives every
//! look-ahead the scanner makes within a line (`is_eos`, `next_char`, a comment's end, a tag's
//! end) its text, and it declines to start a quoted scalar until the closing quote has arrived.
//! The consumed text is dropped as the scan moves on, so the working set is the lines in flight
//! and whatever token is being accumulated (a block scalar is one token), as the YAML colorizer
//! this replaces was (#808).
//!
//! # One deliberate difference
//!
//! go-yaml drops the `x`, `u`, `U` and hex digits of an escape sequence in a double-quoted scalar
//! from the token's `Origin`, so yq's `-C` prints `"x\u0001y"` as `"x\y"`. That changes the
//! string, so it is not reproduced: the escape is kept whole. It is recorded in
//! `docs/compliance/yq/limitations.md`.

use core::ops::{Index, Range, RangeFrom};

use crate::output::Terminator;

const RESET: &str = "\x1b[0m";
const CYAN: &str = "\x1b[36m";
const GREEN: &str = "\x1b[32m";
const MAGENTA: &str = "\x1b[95m";
const YELLOW: &str = "\x1b[93m";
const GREY: &str = "\x1b[90m";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ty {
    Null,
    Bool,
    Integer,
    Float,
    /// Binary, octal and hex integers: go-yaml gives them their own types, which the printer does
    /// not color.
    OtherInt,
    Inf,
    Nan,
    Str,
    SingleQuote,
    DoubleQuote,
    Anchor,
    Alias,
    Tag,
    Comment,
    MappingValue,
    MappingKey,
    SequenceEntry,
    SequenceStart,
    SequenceEnd,
    MappingStart,
    MappingEnd,
    CollectEntry,
    DocHeader,
    DocEnd,
    Directive,
    Literal,
    Folded,
    MergeKey,
    Invalid,
}

#[derive(Clone, Debug)]
struct Tok {
    ty: Ty,
    value: String,
    origin: String,
    column: i64,
    quoted: bool,
}

#[derive(Clone, Copy)]
struct Pos {
    column: i64,
}

fn tok(ty: Ty, value: &str, origin: String, pos: Pos) -> Tok {
    Tok {
        ty,
        // Only a tag's text is read back, by the value that follows it.
        value: if ty == Ty::Tag {
            value.to_string()
        } else {
            String::new()
        },
        origin,
        column: pos.column,
        quoted: false,
    }
}

fn s_of(r: &[char]) -> String {
    r.iter().collect()
}

const NULLS: &[&str] = &["null", "Null", "NULL", "~"];
const BOOLS: &[&str] = &["true", "True", "TRUE", "false", "False", "FALSE"];
const INFS: &[&str] = &[".inf", ".Inf", ".INF", "-.inf", "-.Inf", "-.INF"];
const NANS: &[&str] = &[".nan", ".NaN", ".NAN"];
/// go-yaml's `ReservedTagKeywordMap`: a token after one of these keeps its own type; after any
/// other tag it is a string.
const RESERVED_TAGS: &[&str] = &[
    "!!int",
    "!!float",
    "!!null",
    "!!seq",
    "!!map",
    "!!str",
    "!!binary",
    "!!omap",
    "!!set",
    "!!timestamp",
    "!!bool",
    "!!merge",
];

#[derive(PartialEq)]
enum NumKind {
    Decimal,
    Other,
    Float,
}

/// go-yaml's `token.ToNumber`, reduced to the kind of number `value` is, if it is one.
fn to_number(value: &str) -> Option<NumKind> {
    if value.is_empty() || value.starts_with('_') {
        return None;
    }
    let dot_count = value.matches('.').count();
    if dot_count > 1 {
        return None;
    }
    let is_negative = value.starts_with('-');
    let t = value.strip_prefix('+').unwrap_or(value);
    let t = t.strip_prefix('-').unwrap_or(t);
    let normalized: String = t.replace('_', "");
    let (kind, base, digits): (NumKind, u32, String) =
        if let Some(r) = normalized.strip_prefix("0x") {
            (NumKind::Other, 16, r.to_string())
        } else if let Some(r) = normalized.strip_prefix("0o") {
            (NumKind::Other, 8, r.to_string())
        } else if let Some(r) = normalized.strip_prefix("0b") {
            (NumKind::Other, 2, r.to_string())
        } else if normalized.starts_with('0') && normalized.len() > 1 && dot_count == 0 {
            (NumKind::Other, 8, normalized.clone())
        } else if dot_count == 1 {
            (NumKind::Float, 10, normalized.clone())
        } else {
            (NumKind::Decimal, 10, normalized.clone())
        };
    let text = if is_negative {
        format!("-{digits}")
    } else {
        digits.clone()
    };
    if kind == NumKind::Float {
        return go_parse_float_ok(&text).then_some(kind);
    }
    if digits.is_empty() || !digits.chars().all(|c| c.is_digit(base)) {
        return None;
    }
    if is_negative {
        match i128::from_str_radix(&text, base) {
            Ok(v) if v >= i128::from(i64::MIN) => Some(kind),
            _ => None,
        }
    } else {
        match u128::from_str_radix(&text, base) {
            Ok(v) if v <= u128::from(u64::MAX) => Some(kind),
            _ => None,
        }
    }
}

/// Go's `strconv.ParseFloat` for a string holding exactly one `.` and no underscore: a decimal
/// float. (`inf`/`nan` spellings and hex floats have no single dot.)
fn go_parse_float_ok(s: &str) -> bool {
    let b: Vec<char> = s.chars().collect();
    let mut i = 0;
    if i < b.len() && (b[i] == '+' || b[i] == '-') {
        i += 1;
    }
    let mut digits = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
        digits += 1;
    }
    if i < b.len() && b[i] == '.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return false;
    }
    if i < b.len() && (b[i] == 'e' || b[i] == 'E') {
        i += 1;
        if i < b.len() && (b[i] == '+' || b[i] == '-') {
            i += 1;
        }
        let mut exponent_digits = 0;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
            exponent_digits += 1;
        }
        if exponent_digits == 0 {
            return false;
        }
    }
    i == b.len()
}

/// go-yaml's `token.New`: a reserved keyword, a number, or a string.
fn token_new(value: &str, origin: String, pos: Pos) -> Tok {
    if NULLS.contains(&value) {
        return tok(Ty::Null, value, origin, pos);
    }
    if BOOLS.contains(&value) {
        return tok(Ty::Bool, value, origin, pos);
    }
    if INFS.contains(&value) {
        return tok(Ty::Inf, value, origin, pos);
    }
    if NANS.contains(&value) {
        return tok(Ty::Nan, value, origin, pos);
    }
    match to_number(value) {
        Some(NumKind::Float) => tok(Ty::Float, value, origin, pos),
        Some(NumKind::Decimal) => tok(Ty::Integer, value, origin, pos),
        Some(NumKind::Other) => tok(Ty::OtherInt, value, origin, pos),
        None => tok(Ty::Str, value, origin, pos),
    }
}

#[derive(Default, Clone)]
struct MState {
    opt: String,
    first_line_indent_column: i64,
    prev_line_indent_column: i64,
    line_indent_column: i64,
    last_not_space_only_line_indent_column: i64,
    space_only_indent_column: i64,
    folded_new_line: bool,
    is_raw_folded: bool,
    is_literal: bool,
}

fn first_line_indent_by_opt(opt: &str) -> i64 {
    let o = opt.strip_prefix('-').unwrap_or(opt);
    let o = o.strip_prefix('+').unwrap_or(o);
    let o = o.strip_suffix('-').unwrap_or(o);
    let o = o.strip_suffix('+').unwrap_or(o);
    o.parse::<i64>().unwrap_or(0)
}

impl MState {
    fn last_delim_column(&self) -> i64 {
        if self.first_line_indent_column == 0 {
            0
        } else {
            self.first_line_indent_column - 1
        }
    }
    fn update_indent_column(&mut self, column: i64) {
        if self.first_line_indent_column == 0 {
            self.first_line_indent_column = column;
        }
        if self.line_indent_column == 0 {
            self.line_indent_column = column;
        }
    }
    fn update_space_only_indent_column(&mut self, column: i64) {
        if self.first_line_indent_column == 0 {
            self.space_only_indent_column = column;
        }
    }
    fn update_new_line_state(&mut self) {
        self.prev_line_indent_column = self.line_indent_column;
        if self.line_indent_column != 0 {
            self.last_not_space_only_line_indent_column = self.line_indent_column;
        }
        self.folded_new_line = true;
        self.line_indent_column = 0;
    }
    fn is_indent_column(&self, column: i64) -> bool {
        if self.first_line_indent_column == 0 {
            return column == 1;
        }
        self.first_line_indent_column > column
    }
    fn has_trim_all_end_newline_opt(&self) -> bool {
        self.opt.starts_with('-') || self.opt.ends_with('-') || self.is_raw_folded
    }
    fn has_keep_all_end_newline_opt(&self) -> bool {
        self.opt.starts_with('+') || self.opt.ends_with('+')
    }
}

/// The text being scanned, addressed by position from the start of the current scan (`origin`),
/// of which only a trailing window is kept: everything before `base` has been dropped.
#[derive(Default)]
struct Src {
    window: Vec<char>,
    /// The absolute position of `window[0]`.
    base: usize,
    /// The absolute position the scan's own `0` is.
    origin: usize,
    /// The absolute positions of the last two `\n` pushed, found as the text arrives so that
    /// finding where the scan may stop does not read the window again.
    last_newline: Option<usize>,
    prev_newline: Option<usize>,
}

impl Src {
    fn len(&self) -> usize {
        self.base + self.window.len() - self.origin
    }
    fn push_str(&mut self, s: &str) {
        for c in s.chars() {
            if c == '\n' {
                self.prev_newline = self.last_newline;
                self.last_newline = Some(self.base + self.window.len());
            }
            self.window.push(c);
        }
    }

    /// Whether a line break has been pushed at or after relative position `from`.
    fn has_newline_from(&self, from: usize) -> bool {
        self.last_newline
            .and_then(|abs| abs.checked_sub(self.origin))
            .is_some_and(|rel| rel >= from)
    }

    /// The relative position of the second-to-last line break, if it is at or after `from`.
    fn prev_newline_from(&self, from: usize) -> Option<usize> {
        self.prev_newline
            .and_then(|abs| abs.checked_sub(self.origin))
            .filter(|rel| *rel >= from)
    }
    /// Empties the text, keeping the room it had.
    fn reset(&mut self) {
        self.window.clear();
        self.base = 0;
        self.origin = 0;
        self.last_newline = None;
        self.prev_newline = None;
    }

    /// Drops the text before relative position `keep_from`, less a little history the scanner
    /// reads behind itself.
    fn compact(&mut self, keep_from: usize) {
        const HISTORY: usize = 8;
        let abs = (self.origin + keep_from).saturating_sub(HISTORY);
        if abs > self.base + 4096 {
            self.window.drain(..abs - self.base);
            self.base = abs;
        }
    }
}

impl Index<usize> for Src {
    type Output = char;
    fn index(&self, i: usize) -> &char {
        &self.window[self.origin + i - self.base]
    }
}
impl Index<Range<usize>> for Src {
    type Output = [char];
    fn index(&self, r: Range<usize>) -> &[char] {
        &self.window[self.origin + r.start - self.base..self.origin + r.end - self.base]
    }
}
impl Index<RangeFrom<usize>> for Src {
    type Output = [char];
    fn index(&self, r: RangeFrom<usize>) -> &[char] {
        &self.window[self.origin + r.start - self.base..]
    }
}

/// What a scan remembers of the last token it produced: the tokens themselves are handed on to
/// the printer as they are made, but go-yaml's scanner looks back at the last one (the tag a value
/// follows, the quote a key ended in).
struct LastTok {
    ty: Ty,
    /// Kept only for a tag; the value of any other token can be as long as the document.
    value: String,
    column: i64,
    quoted: bool,
}

struct Ctx {
    idx: usize,
    not_space_char_pos: usize,
    not_space_org_char_pos: usize,
    src: Src,
    buf: Vec<char>,
    obuf: Vec<char>,
    tokens: Vec<Tok>,
    last: Option<LastTok>,
    mstate: Option<MState>,
}

impl Ctx {
    fn new() -> Self {
        Self {
            idx: 0,
            not_space_char_pos: 0,
            not_space_org_char_pos: 0,
            src: Src::default(),
            buf: vec![],
            obuf: vec![],
            tokens: vec![],
            last: None,
            mstate: None,
        }
    }
    fn size(&self) -> usize {
        self.src.len()
    }
    /// Back to a context with nothing scanned, keeping the room its buffers had.
    fn reset(&mut self) {
        self.idx = 0;
        self.src.reset();
        self.reset_buffer();
        self.tokens.clear();
        self.last = None;
        self.mstate = None;
    }
    fn clear(&mut self) {
        self.reset_buffer();
        self.mstate = None;
    }
    fn reset_buffer(&mut self) {
        self.buf.clear();
        self.obuf.clear();
        self.not_space_char_pos = 0;
        self.not_space_org_char_pos = 0;
    }
    fn add_token(&mut self, tk: Option<Tok>) {
        if let Some(t) = tk {
            self.last = Some(LastTok {
                ty: t.ty,
                value: if t.ty == Ty::Tag {
                    t.value.clone()
                } else {
                    String::new()
                },
                column: t.column,
                quoted: t.quoted,
            });
            self.tokens.push(t);
        }
    }
    fn add_buf(&mut self, r: char) {
        if self.buf.is_empty() && (r == ' ' || r == '\t') {
            return;
        }
        self.buf.push(r);
        if r != ' ' && r != '\t' {
            self.not_space_char_pos = self.buf.len();
        }
    }
    fn add_buf_with_tab(&mut self, r: char) {
        if self.buf.is_empty() && r == ' ' {
            return;
        }
        self.buf.push(r);
        if r != ' ' {
            self.not_space_char_pos = self.buf.len();
        }
    }
    fn add_origin_buf(&mut self, r: char) {
        self.obuf.push(r);
        if r != ' ' && r != '\t' {
            self.not_space_org_char_pos = self.obuf.len();
        }
    }
    fn remove_right_space_from_buf(&mut self) {
        let buflen = self.not_space_org_char_pos;
        if self.obuf.len() > buflen {
            self.obuf.truncate(buflen);
            self.buf = self.buffered_src();
        }
    }
    fn is_eos(&self) -> bool {
        self.size() <= self.idx + 1
    }
    fn is_next_eos(&self) -> bool {
        self.size() <= self.idx + 1
    }
    fn next(&self) -> bool {
        self.idx < self.size()
    }
    fn previous_char(&self) -> char {
        if self.idx > 0 {
            self.src[self.idx - 1]
        } else {
            '\0'
        }
    }
    fn current_char(&self) -> char {
        if self.size() > self.idx {
            self.src[self.idx]
        } else {
            '\0'
        }
    }
    fn next_char(&self) -> char {
        if self.size() > self.idx + 1 {
            self.src[self.idx + 1]
        } else {
            '\0'
        }
    }
    fn repeat_num(&self, r: char) -> usize {
        let mut cnt = 0;
        for i in self.idx..self.size() {
            if self.src[i] == r {
                cnt += 1;
            } else {
                break;
            }
        }
        cnt
    }
    fn exists_buffer(&self) -> bool {
        if self.mstate.is_none() {
            // The common case, without building the text `buffered_src` would.
            return self.not_space_char_pos.min(self.buf.len()) != 0;
        }
        !self.buffered_src().is_empty()
    }

    /// Whether the origin text so far, less one leading space, begins with a tab.
    fn origin_starts_with_tab(&self) -> bool {
        let rest = match self.obuf.first() {
            Some(' ') => &self.obuf[1..],
            _ => &self.obuf[..],
        };
        rest.first() == Some(&'\t')
    }
    fn is_multi_line(&self) -> bool {
        self.mstate.is_some()
    }
    fn last_token(&self) -> Option<&LastTok> {
        self.last.as_ref()
    }

    fn buffered_src(&self) -> Vec<char> {
        let mut src: Vec<char> = self.buf[..self.not_space_char_pos.min(self.buf.len())].to_vec();
        if let Some(ms) = &self.mstate {
            if ms.has_trim_all_end_newline_opt() {
                while src.last() == Some(&'\n') {
                    src.pop();
                }
            } else if !ms.has_keep_all_end_newline_opt() {
                let newlines = src.iter().rev().take_while(|c| **c == '\n').count();
                for _ in 1..newlines {
                    src.pop();
                }
            }
            if ms.has_trim_all_end_newline_opt() {
                while src.last() == Some(&' ') {
                    src.pop();
                }
            }
            if src == ['\n'] {
                src.clear();
            }
            if ms.has_keep_all_end_newline_opt() && src.is_empty() {
                src = vec!['\n'];
            }
        }
        src
    }

    fn buffered_token(&mut self, pos: Pos) -> Option<Tok> {
        if self.idx == 0 {
            return None;
        }
        let source = self.buffered_src();
        if source.is_empty() {
            self.buf.clear();
            return None;
        }
        let value = s_of(&source);
        let origin = s_of(&self.obuf);
        let mut tk = if self.is_multi_line() {
            tok(Ty::Str, &value, origin, pos)
        } else {
            token_new(&value, origin, pos)
        };
        if let Some(last) = self.last_token() {
            if last.ty == Ty::Tag && !RESERVED_TAGS.contains(&last.value.as_str()) {
                tk.ty = Ty::Str;
            }
        }
        self.reset_buffer();
        Some(tk)
    }
}

#[derive(PartialEq, Clone, Copy)]
enum IndentState {
    Equal,
    Up,
    Down,
    Keep,
}

type Res<T> = Result<T, Tok>;

fn invalid(msg: &str, origin: String, pos: Pos) -> Tok {
    tok(Ty::Invalid, msg, origin, pos)
}

struct Scanner {
    line: i64,
    column: i64,
    last_delim_column: i64,
    indent_num: i64,
    prev_line_indent_num: i64,
    indent_level: i64,
    is_first_char_at_line: bool,
    is_anchor: bool,
    is_alias: bool,
    is_directive: bool,
    started_flow_sequence_num: i64,
    started_flow_map_num: i64,
    indent_state: IndentState,
    saved_pos: Option<Pos>,
    /// Whether the end of the text has been fed in. Until it has, the scan stops short of the
    /// last complete line, and a quoted scalar waits for its closing quote.
    eof: bool,
    /// How far the search for the closing quote of the scalar waiting at `.0` has read, so that
    /// each piece of text that arrives is read once, not the scalar again from its opening.
    quote_probe: Option<(usize, usize)>,
    /// How much text with no line break in it a scan waits for before it starts on the line, and
    /// how much of its end it then leaves unscanned for the look-ahead the scanner makes.
    /// A line shorter than the first is read whole, as the Go scanner reads it; a longer one is
    /// read as it arrives, so that one very long line does not have to be held entire.
    long_line: usize,
    tail: usize,
}

/// The defaults of [`Scanner::long_line`] and [`Scanner::tail`].
const LONG_LINE: usize = 64 * 1024;
const LINE_TAIL: usize = 4 * 1024;
/// The tokens a scan hands to the printer at a time.
const TOKEN_BATCH: usize = 512;

impl Scanner {
    fn new() -> Self {
        Self {
            line: 1,
            column: 1,
            last_delim_column: 0,
            indent_num: 0,
            prev_line_indent_num: 0,
            indent_level: 0,
            is_first_char_at_line: true,
            is_anchor: false,
            is_alias: false,
            is_directive: false,
            started_flow_sequence_num: 0,
            started_flow_map_num: 0,
            indent_state: IndentState::Equal,
            saved_pos: None,
            eof: false,
            quote_probe: None,
            long_line: LONG_LINE,
            tail: LINE_TAIL,
        }
    }

    fn pos(&self) -> Pos {
        Pos {
            column: self.column,
        }
    }

    fn clear_state(&mut self) {
        self.prev_line_indent_num = 0;
        self.last_delim_column = 0;
        self.indent_level = 0;
        self.indent_num = 0;
    }

    fn buffered_token(&mut self, ctx: &mut Ctx) -> Option<Tok> {
        if let Some(p) = self.saved_pos.take() {
            return ctx.buffered_token(p);
        }
        let column = if ctx.is_multi_line() {
            let obuf = s_of(&ctx.obuf);
            let buf = s_of(&ctx.buf);
            obuf.find(&buf).map_or(-1, |i| i as i64) + 1
        } else {
            self.column - ctx.buf.len() as i64
        };
        ctx.buffered_token(Pos { column })
    }

    fn progress_column(&mut self, ctx: &mut Ctx, num: usize) {
        self.column += num as i64;
        ctx.idx += num;
    }
    fn progress_only(ctx: &mut Ctx, num: usize) {
        ctx.idx += num;
    }
    fn progress_line(&mut self, ctx: &mut Ctx) {
        self.prev_line_indent_num = self.indent_num;
        self.column = 1;
        self.line += 1;
        self.indent_num = 0;
        self.is_first_char_at_line = true;
        self.is_anchor = false;
        self.is_alias = false;
        self.is_directive = false;
        ctx.idx += 1;
    }
    fn progress(ctx: &mut Ctx, num: usize) {
        ctx.idx += num;
    }
    fn is_new_line_char(c: char) -> bool {
        c == '\n' || c == '\r'
    }

    fn update_indent_level(&mut self) {
        if self.prev_line_indent_num < self.indent_num {
            self.indent_level += 1;
        } else if self.prev_line_indent_num > self.indent_num && self.indent_level > 0 {
            self.indent_level -= 1;
        }
    }
    fn update_indent_state(&mut self) {
        if self.last_delim_column == 0 {
            return;
        }
        self.indent_state = if self.last_delim_column < self.column {
            IndentState::Up
        } else {
            IndentState::Down
        };
    }
    fn update_indent(&mut self, c: char) {
        if self.is_first_char_at_line && Self::is_new_line_char(c) {
            return;
        }
        if self.is_first_char_at_line && c == ' ' {
            self.indent_num += 1;
            return;
        }
        if self.is_first_char_at_line && c == '\t' {
            return;
        }
        if !self.is_first_char_at_line {
            self.indent_state = IndentState::Keep;
            return;
        }
        self.update_indent_level();
        self.update_indent_state();
        self.is_first_char_at_line = false;
    }

    fn add_buffered_token_if_exists(&mut self, ctx: &mut Ctx) {
        let t = self.buffered_token(ctx);
        ctx.add_token(t);
    }

    fn found_document_separator_marker(src: &[char]) -> bool {
        if src.len() < 3 {
            return false;
        }
        let marker: String = if src.len() == 3 {
            s_of(src)
        } else {
            s_of(&src[..4])
                .trim_end_matches([' ', '\t', '\n', '\r'])
                .to_string()
        };
        marker == "---" || marker == "..."
    }

    /// Whether the closing quote of the scalar opened at `ctx.idx` is in the text so far.
    fn quote_is_closed(&mut self, ctx: &Ctx, quote: char) -> bool {
        let size = ctx.size();
        let mut i = match self.quote_probe {
            Some((start, read_to)) if start == ctx.idx => read_to,
            _ => ctx.idx + 1,
        };
        while i < size {
            let c = ctx.src[i];
            if quote == '"' && c == '\\' {
                if i + 1 >= size {
                    break; // The escaped character has not arrived.
                }
                i += 2;
                continue;
            }
            if c == quote {
                if quote == '\'' {
                    if i + 1 >= size {
                        break; // A doubled quote or the end: the next character says.
                    }
                    if ctx.src[i + 1] == '\'' {
                        i += 2;
                        continue;
                    }
                }
                self.quote_probe = None;
                return true;
            }
            i += 1;
        }
        self.quote_probe = Some((ctx.idx, i));
        false
    }

    fn scan_single_quote(&mut self, ctx: &mut Ctx) -> Res<Tok> {
        ctx.add_origin_buf('\'');
        let srcpos = self.pos();
        let size = ctx.size();
        let mut is_first_line_char = false;
        let mut is_new_line = false;
        let mut idx = ctx.idx + 1;
        while idx < size {
            if !is_new_line {
                self.progress_column(ctx, 1);
            } else {
                is_new_line = false;
            }
            let c = ctx.src[idx];
            ctx.add_origin_buf(c);
            if Self::is_new_line_char(c) {
                is_first_line_char = true;
                is_new_line = true;
                self.progress_line(ctx);
                if idx + 1 < size && Self::found_document_separator_marker(&ctx.src[idx + 1..]) {
                    return Err(invalid(
                        "found unexpected document separator",
                        s_of(&ctx.obuf),
                        self.pos(),
                    ));
                }
                idx += 1;
                continue;
            } else if is_first_line_char && c == ' ' {
                idx += 1;
                continue;
            } else if is_first_line_char && c == '\t' {
                if self.last_delim_column >= self.column {
                    return Err(invalid(
                        "tab character cannot be used for indentation in single-quoted text",
                        s_of(&ctx.obuf),
                        self.pos(),
                    ));
                }
                idx += 1;
                continue;
            } else if c != '\'' {
                is_first_line_char = false;
                idx += 1;
                continue;
            } else if idx + 1 < size && ctx.src[idx + 1] == '\'' {
                ctx.add_origin_buf(c);
                idx += 2;
                self.progress_column(ctx, 1);
                continue;
            }
            self.progress_column(ctx, 1);
            let mut t = tok(Ty::SingleQuote, "", s_of(&ctx.obuf), srcpos);
            t.quoted = true;
            return Ok(t);
        }
        self.progress_column(ctx, 1);
        Err(invalid(
            "could not find end character of single-quoted text",
            s_of(&ctx.obuf),
            srcpos,
        ))
    }

    fn scan_double_quote(&mut self, ctx: &mut Ctx) -> Res<Tok> {
        ctx.add_origin_buf('"');
        let srcpos = self.pos();
        let size = ctx.size();
        let mut is_first_line_char = false;
        let mut is_new_line = false;
        let mut idx = ctx.idx + 1;
        while idx < size {
            if !is_new_line {
                self.progress_column(ctx, 1);
            } else {
                is_new_line = false;
            }
            let c = ctx.src[idx];
            ctx.add_origin_buf(c);
            if Self::is_new_line_char(c) {
                is_first_line_char = true;
                is_new_line = true;
                self.progress_line(ctx);
                if idx + 1 < size && Self::found_document_separator_marker(&ctx.src[idx + 1..]) {
                    return Err(invalid(
                        "found unexpected document separator",
                        s_of(&ctx.obuf),
                        self.pos(),
                    ));
                }
                idx += 1;
                continue;
            } else if is_first_line_char && c == ' ' {
                idx += 1;
                continue;
            } else if is_first_line_char && c == '\t' {
                if self.last_delim_column >= self.column {
                    return Err(invalid(
                        "tab character cannot be used for indentation in double-quoted text",
                        s_of(&ctx.obuf),
                        self.pos(),
                    ));
                }
                idx += 1;
                continue;
            } else if c == '\\' {
                is_first_line_char = false;
                if idx + 1 >= size {
                    idx += 1;
                    continue;
                }
                let next_char = ctx.src[idx + 1];
                let mut progress: usize;
                match next_char {
                    '0' | 'a' | 'b' | 't' | 'n' | 'v' | 'f' | 'r' | 'e' | ' ' | '"' | '/'
                    | '\\' | 'N' | '_' | 'L' | 'P' | '\t' => progress = 1,
                    'x' => progress = if idx + 3 >= size { 1 } else { 3 },
                    'u' => {
                        if idx + 5 >= size {
                            return Err(invalid(
                                "not enough length for escaped UTF-16 character",
                                s_of(&ctx.obuf),
                                self.pos(),
                            ));
                        }
                        progress = 5;
                        let code = hex_runes(&ctx.src[idx + 2..idx + 6]);
                        if (0xD800..=0xDBFF).contains(&code) {
                            if idx + 11 >= size {
                                return Err(invalid(
                                    "not enough length for escaped UTF-16 surrogate pair",
                                    s_of(&ctx.obuf),
                                    self.pos(),
                                ));
                            }
                            if ctx.src[idx + 6] != '\\' || ctx.src[idx + 7] != 'u' {
                                return Err(invalid(
                                    "found unexpected character after high surrogate",
                                    s_of(&ctx.obuf),
                                    self.pos(),
                                ));
                            }
                            let low = hex_runes(&ctx.src[idx + 8..idx + 12]);
                            if !(0xDC00..=0xDFFF).contains(&low) {
                                return Err(invalid(
                                    "found unexpected low surrogate",
                                    s_of(&ctx.obuf),
                                    self.pos(),
                                ));
                            }
                            progress += 6;
                        }
                    }
                    'U' => {
                        if idx + 9 >= size {
                            return Err(invalid(
                                "not enough length for escaped UTF-32 character",
                                s_of(&ctx.obuf),
                                self.pos(),
                            ));
                        }
                        progress = 9;
                    }
                    '\n' => {
                        is_first_line_char = true;
                        is_new_line = true;
                        ctx.add_origin_buf(next_char);
                        self.progress_column(ctx, 1);
                        self.progress_line(ctx);
                        idx += 2;
                        continue;
                    }
                    '\r' => {
                        is_first_line_char = true;
                        is_new_line = true;
                        ctx.add_origin_buf(next_char);
                        self.progress_line(ctx);
                        progress = 1;
                        if idx + 2 < size && ctx.src[idx + 2] == '\n' {
                            ctx.add_origin_buf('\n');
                            progress = 2;
                        }
                        idx += progress;
                        self.progress_column(ctx, progress);
                        idx += 1;
                        continue;
                    }
                    _ => {
                        self.progress_column(ctx, 1);
                        return Err(invalid(
                            &format!("found unknown escape character {next_char:?}"),
                            s_of(&ctx.obuf),
                            self.pos(),
                        ));
                    }
                }
                // go-yaml leaves the digits of `\x`, `\u` and `\U` out of the origin text, which
                // changes the string yq prints. Every character the escape consumes is kept.
                for k in 1..=progress {
                    if idx + k < size {
                        ctx.add_origin_buf(ctx.src[idx + k]);
                    }
                }
                idx += progress;
                self.progress_column(ctx, progress);
                idx += 1;
                continue;
            } else if c == '\t' {
                let mut found_not_space_char = false;
                let mut progress = 0usize;
                let mut i = idx + 1;
                while i < size {
                    if ctx.src[i] == ' ' || ctx.src[i] == '\t' {
                        progress += 1;
                        i += 1;
                        continue;
                    }
                    if Self::is_new_line_char(ctx.src[i]) {
                        break;
                    }
                    found_not_space_char = true;
                    i += 1;
                }
                if found_not_space_char {
                    if ctx.src[idx + 1] != '"' {
                        self.progress_column(ctx, 1);
                    }
                } else {
                    // The whitespace before a line break is skipped by the scan; it is kept in
                    // the origin so that the line prints as it was written.
                    for k in 1..=progress {
                        ctx.add_origin_buf(ctx.src[idx + k]);
                    }
                    idx += progress;
                    self.progress_column(ctx, progress);
                }
                idx += 1;
                continue;
            } else if c != '"' {
                is_first_line_char = false;
                idx += 1;
                continue;
            }
            self.progress_column(ctx, 1);
            let mut t = tok(Ty::DoubleQuote, "", s_of(&ctx.obuf), srcpos);
            t.quoted = true;
            return Ok(t);
        }
        self.progress_column(ctx, 1);
        Err(invalid(
            "could not find end character of double-quoted text",
            s_of(&ctx.obuf),
            srcpos,
        ))
    }

    fn scan_quote(&mut self, ctx: &mut Ctx, ch: char) -> Res<bool> {
        if ctx.exists_buffer() {
            return Ok(false);
        }
        let tk = if ch == '\'' {
            self.scan_single_quote(ctx)?
        } else {
            self.scan_double_quote(ctx)?
        };
        ctx.add_token(Some(tk));
        ctx.clear();
        Ok(true)
    }

    fn scan_white_space(&mut self, ctx: &mut Ctx) -> bool {
        if ctx.is_multi_line() {
            return false;
        }
        if !self.is_anchor && !self.is_directive && !self.is_alias && !self.is_first_char_at_line {
            return false;
        }
        if self.is_first_char_at_line {
            self.progress_column(ctx, 1);
            ctx.add_origin_buf(' ');
            return true;
        }
        if self.is_directive {
            self.add_buffered_token_if_exists(ctx);
            self.progress_column(ctx, 1);
            ctx.add_origin_buf(' ');
            return true;
        }
        self.add_buffered_token_if_exists(ctx);
        self.is_anchor = false;
        self.is_alias = false;
        true
    }

    fn is_merge_key(ctx: &Ctx) -> bool {
        if ctx.repeat_num('<') != 2 {
            return false;
        }
        let size = ctx.size();
        let mut idx = ctx.idx + 2;
        while idx < size {
            let c = ctx.src[idx];
            if c == ' ' {
                idx += 1;
                continue;
            }
            if c != ':' {
                return false;
            }
            if idx + 1 < size {
                let nc = ctx.src[idx + 1];
                if nc == ' ' || Self::is_new_line_char(nc) {
                    return true;
                }
            }
            idx += 1;
        }
        false
    }

    fn scan_tag(&mut self, ctx: &mut Ctx) -> Res<bool> {
        if ctx.exists_buffer() || self.is_directive {
            return Ok(false);
        }
        ctx.add_origin_buf('!');
        Self::progress(ctx, 1);
        let base = ctx.idx;
        let size = ctx.size();
        let mut idx = 0usize;
        let mut progress = 0usize;
        while base + idx < size {
            let c = ctx.src[base + idx];
            progress = idx + 1;
            match c {
                ' ' => {
                    ctx.add_origin_buf(c);
                    let value = s_of(&ctx.src[ctx.idx - 1..ctx.idx + idx]);
                    let origin = s_of(&ctx.obuf);
                    ctx.add_token(Some(tok(Ty::Tag, &value, origin, self.pos())));
                    self.progress_column(ctx, value.chars().count());
                    ctx.clear();
                    return Ok(true);
                }
                ',' => {
                    if self.started_flow_sequence_num > 0 || self.started_flow_map_num > 0 {
                        let value = s_of(&ctx.src[ctx.idx - 1..ctx.idx + idx]);
                        let origin = s_of(&ctx.obuf);
                        ctx.add_token(Some(tok(Ty::Tag, &value, origin, self.pos())));
                        self.progress_column(ctx, value.chars().count() - 1);
                        ctx.clear();
                        return Ok(true);
                    }
                    ctx.add_origin_buf(c);
                }
                '\n' | '\r' => {
                    ctx.add_origin_buf(c);
                    let value = s_of(&ctx.src[ctx.idx - 1..ctx.idx + idx]);
                    let origin = s_of(&ctx.obuf);
                    ctx.add_token(Some(tok(Ty::Tag, &value, origin, self.pos())));
                    self.progress_column(ctx, value.chars().count() - 1);
                    ctx.clear();
                    return Ok(true);
                }
                '{' | '}' => {
                    ctx.add_origin_buf(c);
                    self.progress_column(ctx, progress);
                    return Err(invalid(
                        &format!("found invalid tag character {c:?}"),
                        s_of(&ctx.obuf),
                        self.pos(),
                    ));
                }
                _ => ctx.add_origin_buf(c),
            }
            idx += 1;
        }
        self.progress_column(ctx, progress);
        ctx.clear();
        Ok(true)
    }

    fn scan_comment(&mut self, ctx: &mut Ctx) -> bool {
        if ctx.exists_buffer() {
            let c = ctx.previous_char();
            if c != ' ' && c != '\t' && !Self::is_new_line_char(c) {
                return false;
            }
        }
        self.add_buffered_token_if_exists(ctx);
        ctx.add_origin_buf('#');
        Self::progress(ctx, 1);
        let base = ctx.idx;
        let n = ctx.size() - base;
        for idx in 0..n {
            let c = ctx.src[base + idx];
            ctx.add_origin_buf(c);
            if !Self::is_new_line_char(c) {
                continue;
            }
            // go-yaml tests `ctx.previousChar() == '\\'` here, which reads the character before
            // the `#`, not before the break; it cannot be a backslash that continues the line.
            if ctx.previous_char() == '\\' {
                continue;
            }
            let value = s_of(&ctx.src[ctx.idx..ctx.idx + idx]);
            let progress = value.chars().count();
            let origin = s_of(&ctx.obuf);
            ctx.add_token(Some(tok(Ty::Comment, &value, origin, self.pos())));
            self.progress_column(ctx, progress);
            self.progress_line(ctx);
            ctx.clear();
            return true;
        }
        let value = s_of(&ctx.src[ctx.idx..]);
        let origin = s_of(&ctx.obuf);
        let progress = value.chars().count();
        ctx.add_token(Some(tok(Ty::Comment, &value, origin, self.pos())));
        self.progress_column(ctx, progress);
        self.progress_line(ctx);
        ctx.clear();
        true
    }

    fn add_indent(ctx: &mut Ctx, column: i64) {
        let ms = ctx.mstate.as_mut().expect("multi-line state");
        if ms.first_line_indent_column == 0 || column < ms.first_line_indent_column {
            return;
        }
        if !ms.is_literal && ms.folded_new_line {
            ms.folded_new_line = false;
        }
        ctx.buf.push(' ');
        ctx.not_space_char_pos = ctx.buf.len();
    }

    fn update_new_line_in_folded(ctx: &mut Ctx, column: i64) {
        let ms = ctx.mstate.as_ref().expect("multi-line state");
        let (is_literal, folded_new_line, line_indent, prev_line_indent, last_nso) = (
            ms.is_literal,
            ms.folded_new_line,
            ms.line_indent_column,
            ms.prev_line_indent_column,
            ms.last_not_space_only_line_indent_column,
        );
        if is_literal || !folded_new_line {
            return;
        }
        let last_char = ctx.buf.last().copied().unwrap_or('\0');
        let prev_last_char = if ctx.buf.len() > 1 {
            ctx.buf[ctx.buf.len() - 2]
        } else {
            '\0'
        };
        if line_indent == prev_line_indent {
            if last_char == '\n' {
                let l = ctx.buf.len();
                ctx.buf[l - 1] = ' ';
            }
        } else if prev_line_indent == 0
            && last_nso == column
            && last_char == '\n'
            && prev_last_char == '\n'
        {
            ctx.buf.pop();
            ctx.not_space_char_pos = ctx.buf.len();
        }
        ctx.mstate
            .as_mut()
            .expect("multi-line state")
            .folded_new_line = false;
    }

    fn scan_multi_line(&mut self, ctx: &mut Ctx, c0: char) -> Res<()> {
        let mut c = c0;
        ctx.add_origin_buf(c);
        if c == '\r' {
            if ctx.next_char() == '\n' {
                ctx.add_origin_buf('\n');
                Self::progress(ctx, 1);
            }
            c = '\n';
        }
        let column = self.column;
        if ctx.is_eos() {
            if self.is_first_char_at_line && c == ' ' {
                Self::add_indent(ctx, column);
            } else {
                ctx.add_buf(c);
            }
            ctx.mstate
                .as_mut()
                .expect("multi-line state")
                .update_indent_column(column);
            let value = ctx.buffered_src();
            let origin = s_of(&ctx.obuf);
            ctx.add_token(Some(tok(Ty::Str, &s_of(&value), origin, self.pos())));
            ctx.clear();
            self.progress_column(ctx, 1);
        } else if Self::is_new_line_char(c) {
            ctx.add_buf(c);
            {
                let ms = ctx.mstate.as_mut().expect("multi-line state");
                ms.update_space_only_indent_column(column - 1);
                ms.update_new_line_state();
            }
            self.progress_line(ctx);
            if ctx.next() && Self::found_document_separator_marker(&ctx.src[ctx.idx..]) {
                let value = ctx.buffered_src();
                let origin = s_of(&ctx.obuf);
                ctx.add_token(Some(tok(Ty::Str, &s_of(&value), origin, self.pos())));
                ctx.clear();
                ctx.mstate = None;
            }
        } else if self.is_first_char_at_line && c == ' ' {
            Self::add_indent(ctx, column);
            self.progress_column(ctx, 1);
        } else if self.is_first_char_at_line
            && c == '\t'
            && ctx
                .mstate
                .as_ref()
                .expect("multi-line state")
                .is_indent_column(column)
        {
            self.progress_column(ctx, 1);
            return Err(invalid(
                "found a tab character where an indentation space is expected",
                s_of(&ctx.obuf),
                self.pos(),
            ));
        } else if c == '\t'
            && !ctx
                .mstate
                .as_ref()
                .expect("multi-line state")
                .is_indent_column(column)
        {
            ctx.add_buf_with_tab(c);
            self.progress_column(ctx, 1);
        } else {
            {
                let ms = ctx.mstate.as_mut().expect("multi-line state");
                if ms.first_line_indent_column == 0 && ms.space_only_indent_column > column {
                    self.progress_column(ctx, 1);
                    return Err(invalid(
                        "invalid number of indent is specified after space only",
                        s_of(&ctx.obuf),
                        self.pos(),
                    ));
                }
                ms.update_indent_column(column);
                if first_line_indent_by_opt(&ms.opt) != 0
                    && ms.first_line_indent_column > ms.line_indent_column
                {
                    self.progress_column(ctx, 1);
                    return Err(invalid(
                        "invalid number of indent is specified in the multi-line header",
                        s_of(&ctx.obuf),
                        self.pos(),
                    ));
                }
                let col = ms.last_delim_column();
                if col > 0 {
                    self.last_delim_column = col;
                }
            }
            Self::update_new_line_in_folded(ctx, column);
            ctx.add_buf_with_tab(c);
            self.progress_column(ctx, 1);
        }
        Ok(())
    }

    fn scan_new_line(&mut self, ctx: &mut Ctx, c0: char) {
        let mut c = c0;
        if !ctx.buf.is_empty() && self.saved_pos.is_none() {
            let buf_len = ctx.buffered_src().len() as i64;
            let mut p = self.pos();
            p.column -= buf_len;
            self.saved_pos = Some(p);
        }
        ctx.remove_right_space_from_buf();
        if c == '\r' && ctx.next_char() == '\n' {
            ctx.add_origin_buf('\r');
            Self::progress(ctx, 1);
            c = '\n';
        }
        if ctx.is_eos() || self.is_anchor || self.is_alias || self.is_directive {
            self.add_buffered_token_if_exists(ctx);
        }
        if ctx.exists_buffer() && self.is_first_char_at_line {
            if ctx.buf.last() == Some(&' ') {
                let l = ctx.buf.len();
                ctx.buf[l - 1] = '\n';
            } else {
                ctx.buf.push('\n');
            }
        } else {
            ctx.add_buf(' ');
        }
        ctx.add_origin_buf(c);
        self.progress_line(ctx);
    }

    fn is_flow_mode(&self) -> bool {
        self.started_flow_sequence_num > 0 || self.started_flow_map_num > 0
    }

    fn scan_flow_map_start(&mut self, ctx: &mut Ctx) -> bool {
        if ctx.exists_buffer() && !self.is_flow_mode() {
            return false;
        }
        self.add_buffered_token_if_exists(ctx);
        ctx.add_origin_buf('{');
        let o = s_of(&ctx.obuf);
        ctx.add_token(Some(tok(Ty::MappingStart, "{", o, self.pos())));
        self.started_flow_map_num += 1;
        self.progress_column(ctx, 1);
        ctx.clear();
        true
    }
    fn scan_flow_map_end(&mut self, ctx: &mut Ctx) -> bool {
        if self.started_flow_map_num <= 0 {
            return false;
        }
        self.add_buffered_token_if_exists(ctx);
        ctx.add_origin_buf('}');
        let o = s_of(&ctx.obuf);
        ctx.add_token(Some(tok(Ty::MappingEnd, "}", o, self.pos())));
        self.started_flow_map_num -= 1;
        self.progress_column(ctx, 1);
        ctx.clear();
        true
    }
    fn scan_flow_array_start(&mut self, ctx: &mut Ctx) -> bool {
        if ctx.exists_buffer() && !self.is_flow_mode() {
            return false;
        }
        self.add_buffered_token_if_exists(ctx);
        ctx.add_origin_buf('[');
        let o = s_of(&ctx.obuf);
        ctx.add_token(Some(tok(Ty::SequenceStart, "[", o, self.pos())));
        self.started_flow_sequence_num += 1;
        self.progress_column(ctx, 1);
        ctx.clear();
        true
    }
    fn scan_flow_array_end(&mut self, ctx: &mut Ctx) -> bool {
        if ctx.exists_buffer() && self.started_flow_sequence_num <= 0 {
            return false;
        }
        self.add_buffered_token_if_exists(ctx);
        ctx.add_origin_buf(']');
        let o = s_of(&ctx.obuf);
        ctx.add_token(Some(tok(Ty::SequenceEnd, "]", o, self.pos())));
        self.started_flow_sequence_num -= 1;
        self.progress_column(ctx, 1);
        ctx.clear();
        true
    }
    fn scan_flow_entry(&mut self, ctx: &mut Ctx, c: char) -> bool {
        if self.started_flow_sequence_num <= 0 && self.started_flow_map_num <= 0 {
            return false;
        }
        self.add_buffered_token_if_exists(ctx);
        ctx.add_origin_buf(c);
        let o = s_of(&ctx.obuf);
        ctx.add_token(Some(tok(Ty::CollectEntry, ",", o, self.pos())));
        self.progress_column(ctx, 1);
        ctx.clear();
        true
    }

    fn scan_map_delim(&mut self, ctx: &mut Ctx) -> Res<bool> {
        let nc = ctx.next_char();
        if self.is_directive || self.is_anchor || self.is_alias {
            return Ok(false);
        }
        if self.started_flow_map_num <= 0
            && nc != ' '
            && nc != '\t'
            && !Self::is_new_line_char(nc)
            && !ctx.is_next_eos()
        {
            return Ok(false);
        }
        if self.started_flow_map_num > 0 && nc == '/' {
            return Ok(false);
        }
        if self.started_flow_map_num > 0 {
            if let Some(tk) = ctx.last_token() {
                if tk.ty == Ty::MappingValue {
                    return Ok(false);
                }
            }
        }
        if ctx.origin_starts_with_tab() && ctx.buf.first() != Some(&'\t') {
            self.progress_column(ctx, 1);
            return Err(invalid(
                "tab character cannot use as a map key directly",
                s_of(&ctx.obuf),
                self.pos(),
            ));
        }
        if let Some(t) = self.buffered_token(ctx) {
            self.last_delim_column = t.column;
            ctx.add_token(Some(t));
        } else if let Some(t) = ctx.last_token() {
            if t.quoted {
                self.last_delim_column = t.column;
            }
        }
        ctx.add_token(Some(tok(
            Ty::MappingValue,
            ":",
            ":".to_string(),
            self.pos(),
        )));
        self.progress_column(ctx, 1);
        ctx.clear();
        Ok(true)
    }

    fn scan_document_start(&mut self, ctx: &mut Ctx) -> bool {
        if self.indent_num != 0 || self.column != 1 || ctx.repeat_num('-') != 3 {
            return false;
        }
        if ctx.size() > ctx.idx + 3 {
            let c = ctx.src[ctx.idx + 3];
            if c != ' ' && c != '\t' && c != '\n' && c != '\r' {
                return false;
            }
        }
        self.add_buffered_token_if_exists(ctx);
        let o = format!("{}---", s_of(&ctx.obuf));
        ctx.add_token(Some(tok(Ty::DocHeader, "---", o, self.pos())));
        self.progress_column(ctx, 3);
        ctx.clear();
        self.clear_state();
        true
    }
    fn scan_document_end(&mut self, ctx: &mut Ctx) -> bool {
        if self.indent_num != 0 || self.column != 1 || ctx.repeat_num('.') != 3 {
            return false;
        }
        self.add_buffered_token_if_exists(ctx);
        let o = format!("{}...", s_of(&ctx.obuf));
        ctx.add_token(Some(tok(Ty::DocEnd, "...", o, self.pos())));
        self.progress_column(ctx, 3);
        ctx.clear();
        true
    }
    fn scan_merge_key(&mut self, ctx: &mut Ctx) -> bool {
        if !Self::is_merge_key(ctx) {
            return false;
        }
        self.last_delim_column = self.column;
        let o = format!("{}<<", s_of(&ctx.obuf));
        ctx.add_token(Some(tok(Ty::MergeKey, "<<", o, self.pos())));
        self.progress_column(ctx, 2);
        ctx.clear();
        true
    }
    fn scan_raw_folded_char(&mut self, ctx: &mut Ctx) -> bool {
        if !ctx.exists_buffer() || self.indent_state != IndentState::Up {
            return false;
        }
        let mut ms = MState {
            is_raw_folded: true,
            ..MState::default()
        };
        ms.update_indent_column(self.column);
        ctx.mstate = Some(ms);
        ctx.add_buf('-');
        ctx.add_origin_buf('-');
        self.progress_column(ctx, 1);
        true
    }
    fn scan_sequence(&mut self, ctx: &mut Ctx) -> Res<bool> {
        if ctx.exists_buffer() {
            return Ok(false);
        }
        let nc = ctx.next_char();
        if nc != '\0' && nc != ' ' && nc != '\t' && !Self::is_new_line_char(nc) {
            return Ok(false);
        }
        if ctx.origin_starts_with_tab() {
            self.progress_column(ctx, 1);
            return Err(invalid(
                "tab character cannot use as a sequence delimiter",
                s_of(&ctx.obuf),
                self.pos(),
            ));
        }
        self.add_buffered_token_if_exists(ctx);
        ctx.add_origin_buf('-');
        let o = s_of(&ctx.obuf);
        let tk = tok(Ty::SequenceEntry, "-", o, self.pos());
        self.last_delim_column = tk.column;
        ctx.add_token(Some(tk));
        self.progress_column(ctx, 1);
        ctx.clear();
        Ok(true)
    }
    fn scan_multi_line_header(&mut self, ctx: &mut Ctx) -> Res<bool> {
        if ctx.exists_buffer() {
            return Ok(false);
        }
        self.scan_multi_line_header_option(ctx);
        self.progress_line(ctx);
        Ok(true)
    }
    fn scan_multi_line_header_option(&mut self, ctx: &mut Ctx) {
        let header = ctx.current_char();
        ctx.add_origin_buf(header);
        Self::progress(ctx, 1);
        let mut progress = 0usize;
        let mut crlf = false;
        let base = ctx.idx;
        let n = ctx.size() - base;
        for idx in 0..n {
            let c = ctx.src[base + idx];
            progress = idx;
            ctx.add_origin_buf(c);
            if Self::is_new_line_char(c) {
                let next_idx = ctx.idx + idx + 1;
                if c == '\r' && next_idx < ctx.size() && ctx.src[next_idx] == '\n' {
                    crlf = true;
                    continue;
                }
                break;
            }
        }
        let mut end_pos = ctx.idx + progress;
        if crlf {
            end_pos -= 1;
        }
        let value = s_of(&ctx.src[ctx.idx..end_pos])
            .trim_end_matches(' ')
            .to_string();
        let comment_value_index = value.find('#');
        let mut opt = value.clone();
        if let Some(ci) = comment_value_index {
            if ci > 0 {
                opt = value[..ci].to_string();
            }
        }
        let opt = opt.trim_end_matches([' ', '\t']).to_string();
        if self.column == 1 {
            self.last_delim_column = 1;
        }
        let ob = s_of(&ctx.obuf);
        let comment_index = ob.find('#');
        let mut header_buf = ob.clone();
        if let Some(ci) = comment_index {
            if ci > 0 {
                header_buf = ob[..ci].to_string();
            }
        }
        let pos = self.pos();
        let mut ms = MState {
            opt: opt.clone(),
            is_literal: header == '|',
            ..MState::default()
        };
        let indent = first_line_indent_by_opt(&opt);
        if indent > 0 {
            ms.first_line_indent_column = self.last_delim_column + indent;
        }
        let (ty, prefix) = if header == '|' {
            (Ty::Literal, '|')
        } else {
            (Ty::Folded, '>')
        };
        ctx.add_token(Some(tok(
            ty,
            &format!("{prefix}{opt}"),
            header_buf.clone(),
            pos,
        )));
        ctx.mstate = Some(ms);
        if let (Some(ci), Some(cvi)) = (comment_index, comment_value_index) {
            if ci > 0 {
                self.column += header_buf.len() as i64;
                let o: String = ctx.obuf[header_buf.chars().count()..].iter().collect();
                let cv = value[cvi + 1..].to_string();
                ctx.add_token(Some(tok(Ty::Comment, &cv, o, self.pos())));
            }
        }
        self.indent_state = IndentState::Keep;
        ctx.reset_buffer();
        self.progress_column(ctx, progress);
    }
    fn scan_map_key(&mut self, ctx: &mut Ctx) -> bool {
        if ctx.exists_buffer() {
            return false;
        }
        let nc = ctx.next_char();
        if nc != ' ' && nc != '\t' {
            return false;
        }
        let tk = tok(Ty::MappingKey, "?", "?".to_string(), self.pos());
        self.last_delim_column = tk.column;
        ctx.add_token(Some(tk));
        self.progress_column(ctx, 1);
        ctx.clear();
        true
    }
    fn scan_directive(&mut self, ctx: &mut Ctx) -> bool {
        if ctx.exists_buffer() || self.indent_num != 0 {
            return false;
        }
        self.add_buffered_token_if_exists(ctx);
        ctx.add_origin_buf('%');
        let o = s_of(&ctx.obuf);
        ctx.add_token(Some(tok(Ty::Directive, "%", o, self.pos())));
        self.progress_column(ctx, 1);
        ctx.clear();
        self.is_directive = true;
        true
    }
    fn scan_anchor(&mut self, ctx: &mut Ctx) -> bool {
        if ctx.exists_buffer() {
            return false;
        }
        self.add_buffered_token_if_exists(ctx);
        ctx.add_origin_buf('&');
        let o = s_of(&ctx.obuf);
        ctx.add_token(Some(tok(Ty::Anchor, "&", o, self.pos())));
        self.progress_column(ctx, 1);
        self.is_anchor = true;
        ctx.clear();
        true
    }
    fn scan_alias(&mut self, ctx: &mut Ctx) -> bool {
        if ctx.exists_buffer() {
            return false;
        }
        self.add_buffered_token_if_exists(ctx);
        ctx.add_origin_buf('*');
        let o = s_of(&ctx.obuf);
        ctx.add_token(Some(tok(Ty::Alias, "*", o, self.pos())));
        self.progress_column(ctx, 1);
        self.is_alias = true;
        ctx.clear();
        true
    }
    fn scan_reserved_char(&mut self, ctx: &mut Ctx, c: char) -> Res<()> {
        if ctx.exists_buffer() {
            return Ok(());
        }
        ctx.add_buf(c);
        ctx.add_origin_buf(c);
        let e = invalid(
            &format!("{c:?} is a reserved character"),
            s_of(&ctx.obuf),
            self.pos(),
        );
        self.progress_column(ctx, 1);
        ctx.clear();
        Err(e)
    }
    fn scan_tab(&mut self, ctx: &mut Ctx, c: char) -> Res<()> {
        if self.is_flow_mode() || !self.is_first_char_at_line {
            return Ok(());
        }
        ctx.add_buf(c);
        ctx.add_origin_buf(c);
        let e = invalid(
            "found character '\t' that cannot start any token",
            s_of(&ctx.obuf),
            self.pos(),
        );
        self.progress_column(ctx, 1);
        ctx.clear();
        Err(e)
    }

    /// The first position of the last complete line of the text so far: the scan stops there
    /// until more arrives, so that every line it scans has a whole line after it.
    fn stop(&self, ctx: &Ctx) -> usize {
        if self.eof {
            return ctx.size();
        }
        if let Some(newline) = ctx.src.prev_newline_from(ctx.idx) {
            return newline + 1;
        }
        // No complete line follows the one being scanned, but this one has already outgrown
        // the rest of the pieces in the text: scan it up to the tail it still needs.
        let size = ctx.size();
        if size.saturating_sub(ctx.idx) > self.long_line {
            return size.saturating_sub(self.tail).max(ctx.idx);
        }
        ctx.idx
    }

    /// Whether the scanner may look ahead from `ctx.idx` for the end of the line: either the end
    /// of the line has arrived, or the text is over.
    fn line_end_is_known(&self, ctx: &Ctx) -> bool {
        self.eof || ctx.src.has_newline_from(ctx.idx)
    }

    /// Whether the character at `ctx.idx` would make the scanner read on to the end of its line,
    /// and that line has not been fully received (only a line long enough to be scanned before its
    /// end arrives can be in that state).
    fn must_wait_for_line_end(&self, ctx: &Ctx, c: char) -> bool {
        if self.line_end_is_known(ctx) {
            return false;
        }
        match c {
            '#' => !ctx.exists_buffer() || matches!(ctx.previous_char(), ' ' | '\t' | '\n' | '\r'),
            '|' | '>' => !ctx.exists_buffer(),
            '!' => !ctx.exists_buffer() && !self.is_directive,
            '<' => ctx.repeat_num('<') == 2,
            _ => false,
        }
    }

    /// Scans the text so far, as far as `stop` allows. Returns an error token to print and
    /// carry on after, like `lexer.Tokenize` does, and whether it paused to have its tokens
    /// taken, to be called again.
    fn scan(&mut self, ctx: &mut Ctx) -> Res<bool> {
        let stop = self.stop(ctx);
        while ctx.idx < stop {
            if ctx.tokens.len() >= TOKEN_BATCH {
                return Ok(true);
            }
            let c = ctx.current_char();
            if !self.eof
                && (c == '"' || c == '\'')
                && !ctx.is_multi_line()
                && !ctx.exists_buffer()
                && !self.quote_is_closed(ctx, c)
            {
                return Ok(false);
            }
            if !ctx.is_multi_line() && self.must_wait_for_line_end(ctx, c) {
                return Ok(false);
            }
            self.update_indent(c);
            if self.indent_state == IndentState::Down {
                self.add_buffered_token_if_exists(ctx);
            }
            if ctx.is_multi_line() {
                if self.indent_state == IndentState::Down {
                    if let Some(tk) = ctx.last_token() {
                        if tk.column == 1 {
                            return Err(invalid(
                                "could not find multi-line content",
                                s_of(&ctx.obuf),
                                self.pos(),
                            ));
                        }
                        if tk.ty != Ty::Str {
                            let p = self.pos();
                            ctx.add_token(Some(tok(Ty::Str, "", String::new(), p)));
                        }
                    }
                    ctx.mstate = None;
                } else {
                    self.scan_multi_line(ctx, c)?;
                    continue;
                }
            }
            let handled = match c {
                '{' => self.scan_flow_map_start(ctx),
                '}' => self.scan_flow_map_end(ctx),
                '.' => self.scan_document_end(ctx),
                '<' => self.scan_merge_key(ctx),
                '-' => {
                    self.scan_document_start(ctx)
                        || self.scan_raw_folded_char(ctx)
                        || self.scan_sequence(ctx)?
                }
                '[' => self.scan_flow_array_start(ctx),
                ']' => self.scan_flow_array_end(ctx),
                ',' => self.scan_flow_entry(ctx, c),
                ':' => self.scan_map_delim(ctx)?,
                '|' | '>' => self.scan_multi_line_header(ctx)?,
                '!' => self.scan_tag(ctx)?,
                '%' => self.scan_directive(ctx),
                '?' => self.scan_map_key(ctx),
                '&' => self.scan_anchor(ctx),
                '*' => self.scan_alias(ctx),
                '#' => self.scan_comment(ctx),
                '\'' | '"' => self.scan_quote(ctx, c)?,
                '\r' | '\n' => {
                    self.scan_new_line(ctx, c);
                    true
                }
                ' ' => self.scan_white_space(ctx),
                '@' | '`' => {
                    self.scan_reserved_char(ctx, c)?;
                    false
                }
                '\t' => {
                    if (ctx.exists_buffer() && self.last_delim_column == 0)
                        || self.last_delim_column < self.column
                    {
                        self.indent_num += 1;
                        ctx.add_origin_buf(c);
                        Self::progress_only(ctx, 1);
                        true
                    } else {
                        self.scan_tab(ctx, c)?;
                        false
                    }
                }
                _ => false,
            };
            if handled {
                continue;
            }
            ctx.add_buf(c);
            ctx.add_origin_buf(c);
            self.progress_column(ctx, 1);
        }
        if self.eof {
            self.add_buffered_token_if_exists(ctx);
        }
        Ok(false)
    }
}

fn hex_runes(b: &[char]) -> i64 {
    let mut sum = 0i64;
    for (i, c) in b.iter().enumerate() {
        let v = i64::from(c.to_digit(16).unwrap_or(0));
        sum += v << ((b.len() - i - 1) * 4);
    }
    sum
}

/// The printer: go-yaml's `Printer.PrintTokens` with yq's properties, one token behind so that a
/// token's color can depend on the one after it.
#[derive(Default)]
struct Printer {
    prev_ty: Option<Ty>,
    held: Option<Tok>,
}

impl Printer {
    fn property(prev: Option<Ty>, tk: &Tok, next: Option<Ty>) -> Option<&'static str> {
        if let Some(Ty::Anchor | Ty::Alias) = prev {
            return Some(YELLOW);
        }
        if next == Some(Ty::MappingValue) {
            return Some(CYAN);
        }
        match tk.ty {
            Ty::Bool | Ty::Integer | Ty::Float => Some(MAGENTA),
            Ty::Anchor | Ty::Alias => Some(YELLOW),
            Ty::Str | Ty::SingleQuote | Ty::DoubleQuote => Some(GREEN),
            Ty::Comment => Some(GREY),
            _ => None,
        }
    }

    /// Writes the held token, whose successor is of type `next`.
    fn flush_held(&mut self, next: Option<Ty>, out: &mut String) {
        let Some(tk) = self.held.take() else { return };
        let prop = Self::property(self.prev_ty, &tk, next);
        for (i, line) in tk.origin.split('\n').enumerate() {
            if i > 0 {
                out.push('\n');
            }
            match prop {
                Some(p) => {
                    out.push_str(p);
                    out.push_str(line);
                    out.push_str(RESET);
                }
                None => out.push_str(line),
            }
        }
        self.prev_ty = Some(tk.ty);
    }

    fn push(&mut self, tk: Tok, out: &mut String) {
        self.flush_held(Some(tk.ty), out);
        self.held = Some(tk);
    }

    fn finish(&mut self, out: &mut String) {
        self.flush_held(None, out);
        self.prev_ty = None;
    }
}

/// Colors YAML text as yq's `-C` does, a result at a time, as the text streams in (#5416).
pub(super) struct YamlColorizer {
    scanner: Scanner,
    ctx: Ctx,
    printer: Printer,
    /// Whether any text has reached the current result: a result with none prints nothing.
    fed: bool,
}

impl Default for YamlColorizer {
    fn default() -> Self {
        Self {
            scanner: Scanner::new(),
            ctx: Ctx::new(),
            printer: Printer::default(),
            fed: false,
        }
    }
}

impl YamlColorizer {
    /// Runs the scanner over what it can, handing every finished token to the printer, and
    /// carries on after an invalid token as `lexer.Tokenize` does.
    fn run(&mut self, out: &mut String) {
        loop {
            let result = self.scanner.scan(&mut self.ctx);
            for tk in self.ctx.tokens.drain(..) {
                self.printer.push(tk, out);
            }
            match result {
                Ok(true) => self.ctx.src.compact(self.ctx.idx),
                Ok(false) => break,
                Err(invalid_token) => {
                    self.printer.push(invalid_token, out);
                    // Tokenize starts a new Scan() after the invalid token: a context of its own
                    // that begins where the last one stopped.
                    self.ctx.reset_buffer();
                    self.ctx.mstate = None;
                    self.ctx.last = None;
                    self.scanner.quote_probe = None;
                    // An invalid token can leave the scan past the end of the text.
                    let consumed = self.ctx.idx.min(self.ctx.size());
                    self.ctx.src.origin += consumed;
                    self.ctx.idx = 0;
                    if self.ctx.size() == 0 {
                        break;
                    }
                }
            }
        }
        self.ctx.src.compact(self.ctx.idx);
    }

    /// A colorizer that scans a line before it has arrived in full once it passes `long_line`
    /// characters, leaving `tail` of them unscanned: the defaults are far longer than a test's text.
    #[cfg(test)]
    fn with_line_limits(long_line: usize, tail: usize) -> Self {
        let mut colorizer = Self::default();
        colorizer.scanner.long_line = long_line;
        colorizer.scanner.tail = tail;
        colorizer
    }

    /// Colors `text`, a piece of the current result's text.
    pub(super) fn push_str(&mut self, text: &str, out: &mut String) {
        if text.is_empty() {
            return;
        }
        self.fed = true;
        self.ctx.src.push_str(text);
        self.run(out);
    }

    /// Ends the current result: yq encodes it with a final line break the printer leaves out, and
    /// prints `terminator` in its place. A new result is scanned afresh.
    pub(super) fn boundary(&mut self, terminator: Terminator, out: &mut String) {
        self.close(out);
        out.push_str(terminator.as_str());
    }

    /// Ends the last result, which has no terminator of its own to write.
    pub(super) fn finish(&mut self, out: &mut String) {
        self.close(out);
    }

    fn close(&mut self, out: &mut String) {
        if self.fed {
            self.ctx.src.push_str("\n");
            self.scanner.eof = true;
            self.run(out);
            self.printer.finish(out);
        }
        // A new result is scanned afresh, in the buffers this one used.
        self.scanner = Scanner {
            long_line: self.scanner.long_line,
            tail: self.scanner.tail,
            ..Scanner::new()
        };
        self.ctx.reset();
        self.printer = Printer::default();
        self.fed = false;
    }
}

/// yq's `PrintYAMLLeadingContent` with colors on: the lines of a document's leading comments
/// (and blank lines, and `---` markers) before its text. A line with something on it is written
/// in grey with its line break inside the span; the rest are written as they are.
pub(super) fn color_leading_content(content: &str, out: &mut String) {
    for line in content.split_inclusive('\n') {
        let bare = line.trim_end_matches(['\n', '\r']);
        if bare == "---" || bare.trim().is_empty() {
            out.push_str(line);
        } else {
            out.push_str(GREY);
            out.push_str(line);
            out.push_str(RESET);
        }
    }
}

/// What yq's `PrintYAMLLeadingContent` adds to the text it tokenizes when a document has trailing
/// comments: each line with something on it in grey, line break inside, with the escape codes in
/// the text itself, so that the scanner reads them as part of a plain scalar. The line break yq
/// adds after a last line without one is the one [`YamlColorizer::close`] adds to every result.
pub(super) fn color_trailing_content(content: &str, out: &mut String) {
    for line in content.split_inclusive('\n') {
        if line.trim().is_empty() {
            out.push_str(line);
        } else {
            out.push_str(GREY);
            out.push_str(line);
            out.push_str(RESET);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn color(text: &str) -> String {
        let mut colorizer = YamlColorizer::default();
        let mut out = String::new();
        colorizer.push_str(text, &mut out);
        colorizer.finish(&mut out);
        out
    }

    /// The text cut into pieces of `size` characters, which must color the same as the whole.
    fn color_in_pieces(text: &str, size: usize) -> String {
        let mut colorizer = YamlColorizer::default();
        let mut out = String::new();
        let chars: Vec<char> = text.chars().collect();
        for piece in chars.chunks(size) {
            let piece: String = piece.iter().collect();
            colorizer.push_str(&piece, &mut out);
        }
        colorizer.finish(&mut out);
        out
    }

    #[test]
    fn a_mapping_colors_keys_cyan_and_numbers_and_booleans_magenta() {
        assert_eq!(
            color("e: true\nb: 2"),
            "\x1b[36me\x1b[0m:\x1b[95m true\x1b[0m\n\x1b[95m\x1b[0m\x1b[36mb\x1b[0m:\x1b[95m 2\x1b[0m"
        );
    }

    #[test]
    fn nothing_fed_prints_nothing() {
        assert_eq!(color(""), "");
    }

    #[test]
    fn an_apostrophe_in_a_plain_scalar_does_not_open_a_span() {
        assert_eq!(
            color("a: it's ok\nb: 2"),
            "\x1b[36ma\x1b[0m:\x1b[32m it's ok\x1b[0m\n\x1b[32m\x1b[0m\x1b[36mb\x1b[0m:\x1b[95m 2\x1b[0m"
        );
    }

    #[test]
    fn pieces_color_as_the_whole_does() {
        let text = "a: &x 1 # c\nb: *x\nc: |\n  lit\n  eral\nd: 'q''s'\ne: \"a\\nb\"\nf: [1, \"b\"]\n- x\n";
        let whole = color(text);
        for size in [1, 2, 3, 5, 8, 13, 100] {
            assert_eq!(color_in_pieces(text, size), whole, "pieces of {size}");
        }
    }

    /// A line scanned before it has arrived in full colors as it does when it is read whole.
    #[test]
    fn a_long_line_scanned_as_it_arrives_colors_as_when_read_whole_5416() {
        let texts = [
            "a: &x 1 # c\nb: *x\nc: |\n  lit\n  eral\nd: 'q''s'\ne: \"a\\nb\"\nf: [1, \"b\", c]\n",
            "- x # c\n- !!str 1\n- !custom 5\n- {a: [1, 2], b: 'x'}\n- ? k\n  : v\n",
            "list: [aaa, bbb # not a comment\n  , ccc]\nurl: http://x/#y\nm:\n  <<: *x\n  n: ~\n",
            "---\na: 1\n---\nb: |-\n  text\n  more\n...\n",
            "k: this is a fairly long plain scalar # with a comment after it\nj: \"and a long double quoted one\"\n",
        ];
        for text in texts {
            let whole = color(text);
            for (long_line, tail) in [(1, 8), (6, 8), (20, 16)] {
                for size in [1, 2, 5, 13, 64] {
                    let mut colorizer = YamlColorizer::with_line_limits(long_line, tail);
                    let mut out = String::new();
                    let chars: Vec<char> = text.chars().collect();
                    for piece in chars.chunks(size) {
                        colorizer.push_str(&piece.iter().collect::<String>(), &mut out);
                    }
                    colorizer.finish(&mut out);
                    assert_eq!(
                        out, whole,
                        "{text:?} limits {long_line}/{tail} pieces of {size}"
                    );
                }
            }
        }
    }

    /// A very long line, and a quoted scalar that takes many pieces to close, are each read once
    /// as they arrive, not again from their start with every piece.
    #[test]
    fn a_long_line_and_a_long_quoted_scalar_cost_linear_time_5416() {
        let started = std::time::Instant::now();
        for text in [
            format!("a: [{}]\n", "1, ".repeat(400_000)),
            format!("a: \"{}\"\nb: 1\n", "x ".repeat(400_000)),
            format!("a: '{}'\nb: 1\n", "y''".repeat(300_000)),
        ] {
            let mut colorizer = YamlColorizer::default();
            let mut out = String::new();
            let chars: Vec<char> = text.chars().collect();
            for piece in chars.chunks(4096) {
                let piece: String = piece.iter().collect();
                colorizer.push_str(&piece, &mut out);
            }
            colorizer.finish(&mut out);
            assert!(out.len() > text.len());
        }
        // Quadratic reading is minutes at this size; linear is well under a second in a debug build.
        assert!(
            started.elapsed() < std::time::Duration::from_secs(20),
            "took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn leading_content_is_grey_with_its_line_break_inside() {
        let mut out = String::new();
        color_leading_content("# head\n\n---\n# more\n", &mut out);
        assert_eq!(out, "\x1b[90m# head\n\x1b[0m\n---\n\x1b[90m# more\n\x1b[0m");
    }
}
