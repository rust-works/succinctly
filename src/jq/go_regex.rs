//! Go `regexp` (RE2) pattern syntax, translated to the `regex` crate's (#5374, #5388).
//!
//! yq compiles a pattern with Go's `regexp`, whose syntax is close to the `regex` crate's but not
//! the same: its shorthand classes and `\b` are ASCII, `\Q..\E` is a literal span, `\<` is a plain
//! `<`, a `{` that does not start a repeat is a literal, `\0`/`\012` are octal escapes, and a long
//! list of things the crate accepts (`a++`, `(?x)`, `[a--b]`, `\p{Script=Greek}`) is an error.
//! Handing the pattern to the crate unread therefore both accepted programs yq rejects and
//! answered `true` for `"é" | test("\\w")`.
//!
//! [`translate`] reads the pattern the way `regexp/syntax`'s parser does and writes an equivalent
//! pattern for the crate, with every literal escaped so that none of the crate's own syntax
//! (`--`, `&&`, `~~`, `\<`, `\>`, `\b{..}`) can leak through. A pattern Go refuses is refused with
//! Go's own wording (`error parsing regexp: invalid escape sequence: `\Z``). It is a transcription
//! of `parse.go`'s `parse`, `parsePerlFlags`, `parseEscape`, `parseClass`, `parseRepeat` and
//! `repeat`, checked against yq v4.53.3 over the patterns in `tests/yq_cli/i5388.rs`.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Go's cap on a counted repeat (`{n,m}`): `maxRepeat` in `parse.go`.
const MAX_REPEAT: u64 = 1000;

/// Translate a Go `regexp` pattern to the `regex` crate's, or Go's own error message for it.
pub(crate) fn translate(pattern: &str) -> Result<String, String> {
    let mut t = Translator {
        c: pattern.chars().collect(),
        i: 0,
        out: String::with_capacity(pattern.len() + 16),
        groups: Vec::new(),
        can_repeat: false,
        atom_start: 0,
        atom_is_assertion: false,
        atom_repeated: false,
        pending_flags: String::new(),
        atom_need: 1,
        levels: alloc::vec![1],
    };
    t.run()?;
    t.flush_flags();
    Ok(t.out)
}

fn parse_error(code: &str, expr: &str) -> String {
    format!("error parsing regexp: {code}: `{expr}`")
}

struct Translator {
    c: Vec<char>,
    i: usize,
    out: String,
    /// `out` offsets of the open groups, innermost last.
    groups: Vec<usize>,
    /// Whether a repeat operator here has something to repeat (Go: the top of its stack is a
    /// real expression, not the start, a `(` or a `|`).
    can_repeat: bool,
    /// Where the last atom starts in `out`, and whether it matches no text (`^`, `\b`).
    atom_start: usize,
    atom_is_assertion: bool,
    /// Whether the last atom already carries a repeat (only a flag setting between two repeat
    /// operators gets here: `a*(?i)+`, which Go reads as `(a*)+`).
    atom_repeated: bool,
    /// A flag-only group `(?i)` is written out just before the next thing that is not a repeat
    /// operator, so that `a(?i)*` and `a*(?i)+` can still repeat what came before it.
    pending_flags: String,
    /// The product of the counted repeats nested in the last atom (`(a{10}){20}` needs 200),
    /// and the largest such product of the atoms before it at each open group's level, outermost
    /// first: Go's `repeatIsValid(re, 1000)`, which refuses a counted repeat that takes a nested
    /// product past 1000.
    atom_need: u64,
    levels: Vec<u64>,
}

impl Translator {
    fn text(&self, from: usize, to: usize) -> String {
        self.c[from..to.min(self.c.len())].iter().collect()
    }

    /// Fold the last atom's need into its group level's.
    fn fold_atom(&mut self) {
        if let Some(level) = self.levels.last_mut() {
            *level = (*level).max(self.atom_need);
        }
    }

    fn flush_flags(&mut self) {
        if !self.pending_flags.is_empty() {
            let flags = core::mem::take(&mut self.pending_flags);
            self.out.push_str(&flags);
        }
    }

    fn whole(&self) -> String {
        self.c.iter().collect()
    }

    fn peek(&self, at: usize) -> Option<char> {
        self.c.get(at).copied()
    }

    fn run(&mut self) -> Result<(), String> {
        // `lastRepeat`: where the repeat operator that was the previous token starts.
        let mut last_repeat: Option<usize> = None;
        while self.i < self.c.len() {
            let mut repeat = None;
            let ch = self.c[self.i];
            match ch {
                '(' => self.open_group()?,
                '|' => {
                    self.flush_flags();
                    self.fold_atom();
                    self.atom_need = 1;
                    self.out.push('|');
                    self.can_repeat = false;
                    self.i += 1;
                }
                ')' => {
                    let Some(open) = self.groups.pop() else {
                        return Err(parse_error("unexpected )", &self.whole()));
                    };
                    self.flush_flags();
                    self.fold_atom();
                    self.atom_need = self.levels.pop().unwrap_or(1);
                    self.out.push(')');
                    self.atom_start = open;
                    self.atom_is_assertion = false;
                    self.atom_repeated = false;
                    self.can_repeat = true;
                    self.i += 1;
                }
                '^' | '$' => {
                    self.assertion(&ch.to_string());
                    self.i += 1;
                }
                '.' => {
                    self.atom(".");
                    self.i += 1;
                }
                '[' => self.class()?,
                '*' | '+' | '?' => {
                    let start = self.i;
                    self.i += 1;
                    self.repeat(start, self.i, last_repeat, None)?;
                    repeat = Some(start);
                }
                '{' => match self.parse_repeat() {
                    Some((min, max, end)) => {
                        let start = self.i;
                        if min > MAX_REPEAT || max.is_some_and(|m| m > MAX_REPEAT || min > m) {
                            return Err(parse_error(
                                "invalid repeat count",
                                &self.text(start, end),
                            ));
                        }
                        self.i = end;
                        self.repeat(start, end, last_repeat, Some((min, max)))?;
                        repeat = Some(start);
                    }
                    None => {
                        self.literal('{');
                        self.i += 1;
                    }
                },
                '\\' => self.escape()?,
                other => {
                    self.literal(other);
                    self.i += 1;
                }
            }
            last_repeat = repeat;
        }
        if !self.groups.is_empty() {
            return Err(parse_error("missing closing )", &self.whole()));
        }
        Ok(())
    }

    /// Start an atom that matches text.
    fn atom(&mut self, text: &str) {
        self.flush_flags();
        self.fold_atom();
        self.atom_need = 1;
        self.atom_start = self.out.len();
        self.atom_is_assertion = false;
        self.atom_repeated = false;
        self.out.push_str(text);
        self.can_repeat = true;
    }

    /// Start an atom that matches no text (`^`, `$`, `\b`): a repeat of it is wrapped in a group.
    fn assertion(&mut self, text: &str) {
        self.atom(text);
        self.atom_is_assertion = true;
    }

    fn literal(&mut self, ch: char) {
        let mut text = String::new();
        push_literal(&mut text, ch);
        self.atom(&text);
    }

    /// `repeat` in `parse.go`: the operator spans `start..end`; a `?` right after it makes it
    /// non-greedy; a repeat operator directly after another is an error before anything else.
    fn repeat(
        &mut self,
        start: usize,
        mut end: usize,
        last_repeat: Option<usize>,
        counted: Option<(u64, Option<u64>)>,
    ) -> Result<(), String> {
        let lazy = self.peek(end) == Some('?');
        if lazy {
            end += 1;
        }
        if let Some(prev) = last_repeat {
            return Err(parse_error(
                "invalid nested repetition operator",
                &self.text(prev, end),
            ));
        }
        if !self.can_repeat {
            return Err(parse_error(
                "missing argument to repetition operator",
                &self.text(start, end),
            ));
        }
        if let Some((min, max)) = counted {
            let count = max.unwrap_or(min);
            let need = if count == 0 {
                1
            } else {
                count.saturating_mul(self.atom_need.max(1))
            };
            if (min >= 2 || max.is_some_and(|m| m >= 2)) && need > MAX_REPEAT {
                return Err(parse_error("invalid repeat count", &self.text(start, end)));
            }
            self.atom_need = need;
        }
        if self.atom_is_assertion || self.atom_repeated {
            self.out.insert_str(self.atom_start, "(?:");
            self.out.push(')');
        }
        let op = self.text(start, end);
        self.out.push_str(&op);
        self.atom_repeated = true;
        self.i = end;
        Ok(())
    }

    /// `parseRepeat`: `{n}`, `{n,}` or `{n,m}` at `self.i`, as `(min, max, end)`; anything else
    /// is not a repeat and the `{` is a literal. A count too large for a `u64` is past the cap.
    fn parse_repeat(&self) -> Option<(u64, Option<u64>, usize)> {
        let mut j = self.i + 1;
        let (min, after_min) = self.number(j)?;
        j = after_min;
        let max = if self.peek(j) == Some(',') {
            j += 1;
            if self.peek(j) == Some('}') {
                None
            } else {
                let (m, after_max) = self.number(j)?;
                j = after_max;
                Some(m)
            }
        } else {
            Some(min)
        };
        if self.peek(j) != Some('}') {
            return None;
        }
        Some((min, max, j + 1))
    }

    fn number(&self, from: usize) -> Option<(u64, usize)> {
        let mut j = from;
        let mut value: u64 = 0;
        while let Some(d) = self.peek(j).and_then(|c| c.to_digit(10)) {
            value = value.saturating_mul(10).saturating_add(u64::from(d));
            j += 1;
        }
        (j > from).then_some((value, j))
    }

    /// `\` outside a class.
    fn escape(&mut self) -> Result<(), String> {
        let start = self.i;
        match self.peek(start + 1) {
            Some('A') => {
                self.assertion("\\A");
                self.i += 2;
            }
            Some('z') => {
                self.assertion("\\z");
                self.i += 2;
            }
            Some('b') => {
                self.assertion("(?-u:\\b)");
                self.i += 2;
            }
            Some('B') => {
                self.assertion("(?-u:\\B)");
                self.i += 2;
            }
            Some('C') => return Err(parse_error("invalid escape sequence", "\\C")),
            Some('Q') => {
                let body = start + 2;
                let end = (body..self.c.len().saturating_sub(1))
                    .find(|&k| self.c[k] == '\\' && self.c[k + 1] == 'E');
                let stop = end.unwrap_or(self.c.len());
                for k in body..stop {
                    let ch = self.c[k];
                    self.literal(ch);
                }
                self.i = end.map_or(self.c.len(), |e| e + 2);
            }
            Some('p' | 'P') => {
                let (text, next) = self.unicode_class(start)?;
                self.atom(&text);
                self.i = next;
            }
            Some(k @ ('d' | 'D' | 's' | 'S' | 'w' | 'W')) => {
                self.atom(&perl_class(k));
                self.i += 2;
            }
            _ => {
                let (ch, next) = self.escape_char(start)?;
                self.literal(ch);
                self.i = next;
            }
        }
        Ok(())
    }

    /// `parseEscape`: the single character the escape at `start` (a `\`) names, and where it
    /// ends.
    fn escape_char(&self, start: usize) -> Result<(char, usize), String> {
        let mut i = start + 1;
        let Some(c) = self.peek(i) else {
            return Err(parse_error("trailing backslash at end of expression", ""));
        };
        i += 1;
        let invalid = |end: usize| {
            Err(parse_error(
                "invalid escape sequence",
                &self.text(start, end),
            ))
        };
        let octal = |first: u32, mut j: usize| {
            let mut r = first;
            for _ in 0..2 {
                match self.peek(j).and_then(|d| d.to_digit(8)) {
                    Some(d) => {
                        r = r * 8 + d;
                        j += 1;
                    }
                    None => break,
                }
            }
            (char::from_u32(r).unwrap_or('\0'), j)
        };
        match c {
            '1'..='7' => {
                // A lone non-zero digit is a backreference, which RE2 does not support.
                if self.peek(i).is_some_and(|d| ('0'..='7').contains(&d)) {
                    Ok(octal(c.to_digit(8).unwrap_or(0), i))
                } else {
                    invalid(i)
                }
            }
            '0' => Ok(octal(0, i)),
            'x' => {
                let Some(first) = self.peek(i) else {
                    return invalid(i);
                };
                i += 1;
                if first == '{' {
                    let mut value: u32 = 0;
                    let mut digits = 0;
                    loop {
                        let Some(d) = self.peek(i) else {
                            return invalid(i);
                        };
                        i += 1;
                        if d == '}' {
                            break;
                        }
                        let Some(v) = d.to_digit(16) else {
                            return invalid(i);
                        };
                        value = value.saturating_mul(16).saturating_add(v);
                        if value > 0x10FFFF {
                            return invalid(i);
                        }
                        digits += 1;
                    }
                    if digits == 0 {
                        return invalid(i);
                    }
                    // A surrogate is a valid Go rune and no `char`; it matches nothing real.
                    return Ok((char::from_u32(value).unwrap_or('\u{FFFD}'), i));
                }
                let Some(second) = self.peek(i) else {
                    return invalid(i);
                };
                i += 1;
                match (first.to_digit(16), second.to_digit(16)) {
                    (Some(x), Some(y)) => Ok((char::from_u32(x * 16 + y).unwrap_or('\0'), i)),
                    _ => invalid(i),
                }
            }
            'a' => Ok(('\u{7}', i)),
            'f' => Ok(('\u{c}', i)),
            'n' => Ok(('\n', i)),
            'r' => Ok(('\r', i)),
            't' => Ok(('\t', i)),
            'v' => Ok(('\u{b}', i)),
            // Escaped punctuation is itself (`\<` is a `<`, `\_` a `_`).
            c if c.is_ascii() && !c.is_ascii_alphanumeric() => Ok((c, i)),
            _ => invalid(i),
        }
    }

    /// `\p{Name}`, `\pN`, `\P..`, `\p{^Name}` at `start`, as the crate's class and where it ends.
    /// A name with anything but letters, digits and `_` (`Script=Greek`, `Age=1.1`) is an error in
    /// Go. Which names it knows is its own table (it refuses `Alphabetic` and `Grek`, accepts
    /// `greek`), which the crate's loose matching does not follow, so that is left to the crate.
    fn unicode_class(&self, start: usize) -> Result<(String, usize), String> {
        let mut negated = self.c[start + 1] == 'P';
        let mut j = start + 2;
        let Some(first) = self.peek(j) else {
            return Err(parse_error(
                "invalid character class range",
                &self.text(start, self.c.len()),
            ));
        };
        let mut name;
        if first == '{' {
            let Some(close) = (j + 1..self.c.len()).find(|&k| self.c[k] == '}') else {
                return Err(parse_error(
                    "invalid character class range",
                    &self.text(start, self.c.len()),
                ));
            };
            name = self.text(j + 1, close);
            j = close + 1;
        } else {
            name = first.to_string();
            j += 1;
        }
        if let Some(rest) = name.strip_prefix('^') {
            negated = !negated;
            name = rest.to_string();
        }
        if name.is_empty()
            || !name
                .chars()
                .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        {
            return Err(parse_error(
                "invalid character class range",
                &self.text(start, j),
            ));
        }
        let text = format!("\\{}{{{}}}", if negated { 'P' } else { 'p' }, name);
        Ok((text, j))
    }

    /// `parseClass`: the bracketed class at `self.i`, re-written with every member escaped.
    fn class(&mut self) -> Result<(), String> {
        let whole = self.i;
        let mut i = whole + 1;
        let mut out = String::from("[");
        if self.peek(i) == Some('^') {
            out.push('^');
            i += 1;
        }
        let missing = |t: &Self| Err(parse_error("missing closing ]", &t.text(whole, t.c.len())));
        let mut first = true;
        while i < self.c.len() && (self.c[i] != ']' || first) {
            first = false;
            // A POSIX group: `[:alpha:]`, `[:^alpha:]`.
            if self.c.len() - i > 2 && self.c[i] == '[' && self.c[i + 1] == ':' {
                if let Some(k) =
                    (i + 1..self.c.len() - 1).find(|&k| self.c[k] == ':' && self.c[k + 1] == ']')
                {
                    let name = self.text(i, k + 2);
                    let inner = &name[2..name.len() - 2];
                    let known = [
                        "alnum", "alpha", "ascii", "blank", "cntrl", "digit", "graph",
                    ]
                    .iter()
                    .chain(
                        [
                            "lower", "print", "punct", "space", "upper", "word", "xdigit",
                        ]
                        .iter(),
                    )
                    .any(|g| inner.strip_prefix('^').unwrap_or(inner) == *g);
                    if !known {
                        return Err(parse_error("invalid character class range", &name));
                    }
                    out.push_str(&name);
                    i = k + 2;
                    continue;
                }
            }
            // `\p{..}`, `\P{..}`.
            if self.c.len() - i > 2 && self.c[i] == '\\' && matches!(self.c[i + 1], 'p' | 'P') {
                let (text, next) = self.unicode_class(i)?;
                out.push_str(&text);
                i = next;
                continue;
            }
            // `\d`, `\S`, `\w`, ...
            if self.c[i] == '\\' {
                if let Some(&k) = self.c.get(i + 1) {
                    if matches!(k, 'd' | 'D' | 's' | 'S' | 'w' | 'W') {
                        out.push_str(&perl_class(k));
                        i += 2;
                        continue;
                    }
                }
            }
            // A character or a range.
            let range_start = i;
            let (lo, next) = self.class_char(i, whole)?;
            i = next;
            let mut hi = lo;
            if self.c.len() - i >= 2 && self.c[i] == '-' && self.c[i + 1] != ']' {
                let (h, next) = self.class_char(i + 1, whole)?;
                hi = h;
                i = next;
                if hi < lo {
                    return Err(parse_error(
                        "invalid character class range",
                        &self.text(range_start, i),
                    ));
                }
            }
            push_literal(&mut out, lo);
            if hi != lo {
                out.push('-');
                push_literal(&mut out, hi);
            }
        }
        if i >= self.c.len() {
            return missing(self);
        }
        out.push(']');
        self.atom(&out);
        self.i = i + 1;
        Ok(())
    }

    /// `parseClassChar`: one member character, a plain one or an escape.
    fn class_char(&self, at: usize, whole: usize) -> Result<(char, usize), String> {
        match self.peek(at) {
            None => Err(parse_error(
                "missing closing ]",
                &self.text(whole, self.c.len()),
            )),
            Some('\\') => self.escape_char(at),
            Some(ch) => Ok((ch, at + 1)),
        }
    }

    /// `(` : a capture, a named capture, a non-capturing group or a flag setting.
    fn open_group(&mut self) -> Result<(), String> {
        let start = self.i;
        if self.peek(start + 1) != Some('?') {
            self.flush_flags();
            self.groups.push(self.out.len());
            self.fold_atom();
            self.levels.push(1);
            self.atom_need = 1;
            self.out.push('(');
            self.can_repeat = false;
            self.i += 1;
            return Ok(());
        }
        let rest = self.c.len() - start;
        let named_p = rest > 4 && self.c[start + 2] == 'P' && self.c[start + 3] == '<';
        let named = named_p || (rest > 3 && self.c[start + 2] == '<');
        if named {
            let Some(end) = (start..self.c.len()).find(|&k| self.c[k] == '>') else {
                return Err(parse_error(
                    "invalid named capture",
                    &self.text(start, self.c.len()),
                ));
            };
            let name = self.text(start + if named_p { 4 } else { 3 }, end);
            if name.is_empty()
                || !name
                    .chars()
                    .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
            {
                return Err(parse_error(
                    "invalid named capture",
                    &self.text(start, end + 1),
                ));
            }
            self.flush_flags();
            self.groups.push(self.out.len());
            self.fold_atom();
            self.levels.push(1);
            self.atom_need = 1;
            self.out.push_str(&format!("(?P<{name}>"));
            self.can_repeat = false;
            self.i = end + 1;
            return Ok(());
        }
        // Flags: `(?i)`, `(?i:`, `(?-s)`, `(?i-s:`; Go knows `i`, `m`, `s` and `U` only.
        let mut j = start + 2;
        let mut negative = false;
        let mut saw_flag = false;
        let mut flags = String::new();
        while let Some(ch) = self.peek(j) {
            j += 1;
            match ch {
                'i' | 'm' | 's' | 'U' => {
                    flags.push(ch);
                    saw_flag = true;
                }
                '-' => {
                    if negative {
                        break;
                    }
                    negative = true;
                    saw_flag = false;
                    flags.push('-');
                }
                ':' | ')' => {
                    if negative && !saw_flag {
                        break;
                    }
                    if ch == ':' {
                        self.flush_flags();
                        self.groups.push(self.out.len());
                        self.fold_atom();
                        self.levels.push(1);
                        self.atom_need = 1;
                        self.out.push_str(&format!("(?{flags}:"));
                        self.can_repeat = false;
                    } else if !flags.is_empty() {
                        self.pending_flags.push_str(&format!("(?{flags})"));
                    }
                    self.i = j;
                    return Ok(());
                }
                _ => break,
            }
        }
        Err(parse_error(
            "invalid or unsupported Perl syntax",
            &self.text(start, j),
        ))
    }
}

/// Go's `\d`, `\s`, `\w` and their negations, as a bracketed class of the crate's: ASCII, and
/// `\s` without `\v`. The space is `\x20`, which a verbose `(?x)` would not ignore.
fn perl_class(shorthand: char) -> String {
    let (body, negated) = match shorthand {
        'w' => ("0-9A-Za-z_", false),
        'W' => ("0-9A-Za-z_", true),
        'd' => ("0-9", false),
        'D' => ("0-9", true),
        's' => (r"\t\n\f\r\x20", false),
        _ => (r"\t\n\f\r\x20", true),
    };
    format!("[{}{body}]", if negated { "^" } else { "" })
}

/// A literal character outside a class, escaped unless it is a letter, a digit or not ASCII.
fn push_literal(out: &mut String, ch: char) {
    if ch.is_ascii_alphanumeric() || (!ch.is_ascii() && !ch.is_whitespace() && !ch.is_control()) {
        out.push(ch);
    } else {
        out.push_str(&format!("\\x{{{:X}}}", ch as u32));
    }
}
