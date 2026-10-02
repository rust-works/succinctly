//! How go-yaml's encoder chooses a style for a string scalar.
//!
//! yq writes YAML through go-yaml, and a string it has just *decoded from JSON*
//! carries no source style of its own, so the encoder picks one: plain where it
//! can, otherwise single- or double-quoted. [`go_yaml_string_style`] reproduces
//! that choice for a single-line string, and [`write_go_yaml_double_quoted`] and
//! [`write_go_yaml_single_quoted`] write the quoted forms the way the emitter
//! does.
//!
//! This is the *encoding* rule, which is not the decoding rule in
//! [`super::scalar`]. go-yaml quotes a string that would *read back* as another
//! type, and its reader accepts YAML 1.1 leftovers (`1_000`, `0b11`, `0X1F`,
//! timestamps) that [`super::scalar::resolve_plain`] deliberately keeps as
//! strings, so the two answer differently for exactly those spellings.
//!
//! Every rule below was captured from yq v4.53.3 (`yq -p json -o yaml`), not
//! derived from go-yaml's source: `yes`, `no`, `on`, `off`, `y` and `n` stay
//! plain, and a base-60 spelling (`12:30:45`) does too.
//!
//! # What this does not cover
//!
//! A string with a line break (`\n`, `\r`, U+0085, U+2028, U+2029) is
//! [`None`]. yq writes a literal block for it as a value, `? ` explicit-key
//! syntax as a key, and folds a quoted one across lines, none of which a
//! single-line writer can produce. The same goes for a key longer than 128
//! bytes, which go-yaml also writes with `? `.

#[cfg(not(test))]
use alloc::string::String;
#[cfg(test)]
use std::string::String;

/// The style go-yaml's encoder gives a single-line string scalar in block
/// context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EncodedStringStyle {
    /// Written bare: `name: a`.
    Plain,
    /// `'a: b'`, with `'` doubled. Chosen when plain would be read back as
    /// something else, or not at all (an indicator first, `: ` or ` #` inside,
    /// a leading or trailing space).
    SingleQuoted,
    /// `"1"`. Chosen when the string would read back as another type, and when
    /// it holds a character the emitter will not write raw.
    DoubleQuoted,
}

/// The style go-yaml's encoder gives `s` in block context, or `None` for a
/// string with a line break (see the module docs).
#[must_use]
pub(crate) fn go_yaml_string_style(s: &str) -> Option<EncodedStringStyle> {
    if s.chars().any(is_break) {
        return None;
    }
    // yaml.v3 `stringv`: a string that would resolve to another tag when
    // written plain is double-quoted. The empty string resolves to null.
    if resolves_to_non_str(s) {
        return Some(EncodedStringStyle::DoubleQuoted);
    }
    // The emitter's `yaml_emitter_analyze_scalar`, for block context and a
    // string with no line break.
    let mut special = false;
    let mut block_indicators = s.starts_with("---") || s.starts_with("...");
    let mut preceded_by_blank = true;
    let mut chars = s.chars().peekable();
    let mut first = true;
    while let Some(c) = chars.next() {
        // `is_blankz` of the next character: a blank, or the end of the string.
        let followed_by_blank = chars.peek().map_or(true, |&n| is_blank(n));
        if first {
            match c {
                '#' | ',' | '[' | ']' | '{' | '}' | '&' | '*' | '!' | '|' | '>' | '\'' | '"'
                | '%' | '@' | '`' => block_indicators = true,
                '?' | ':' | '-' if followed_by_blank => block_indicators = true,
                _ => {}
            }
        } else {
            match c {
                ':' if followed_by_blank => block_indicators = true,
                '#' if preceded_by_blank => block_indicators = true,
                _ => {}
            }
        }
        special |= !is_printable(c);
        preceded_by_blank = is_blank(c);
        first = false;
    }
    if special {
        Some(EncodedStringStyle::DoubleQuoted)
    } else if block_indicators || s.starts_with(' ') || s.ends_with(' ') {
        Some(EncodedStringStyle::SingleQuoted)
    } else {
        Some(EncodedStringStyle::Plain)
    }
}

/// `s` as go-yaml's emitter writes a double-quoted scalar, quotes included.
///
/// Escapes `"` and `\`, and every character the emitter will not write raw
/// (anything outside its printable set), with go-yaml's own escape letters and
/// upper-case hex: `\t`, `\x01`, `\x7F`, `\N` for U+0085, `\uFEFF`, and
/// `\U0001F600` for a character outside the BMP.
///
/// One quirk, captured from yq: a string that *starts* with U+FEFF has every
/// character after it escaped too, ASCII included (`\uFEFF\x78` for `\u{FEFF}x`,
/// a space as `\x20`, U+00A0 as `\_`). A BOM anywhere else changes nothing for
/// the characters around it.
pub(crate) fn write_go_yaml_double_quoted<Out: core::fmt::Write>(
    out: &mut Out,
    s: &str,
) -> core::fmt::Result {
    out.write_char('"')?;
    let escape_all = s.starts_with('\u{FEFF}');
    for c in s.chars() {
        match c {
            '"' => out.write_str("\\\"")?,
            '\\' => out.write_str("\\\\")?,
            c if !escape_all && is_printable(c) && !is_break(c) => out.write_char(c)?,
            '\0' => out.write_str("\\0")?,
            '\u{07}' => out.write_str("\\a")?,
            '\u{08}' => out.write_str("\\b")?,
            '\t' => out.write_str("\\t")?,
            '\n' => out.write_str("\\n")?,
            '\u{0B}' => out.write_str("\\v")?,
            '\u{0C}' => out.write_str("\\f")?,
            '\r' => out.write_str("\\r")?,
            '\u{1B}' => out.write_str("\\e")?,
            '\u{85}' => out.write_str("\\N")?,
            '\u{A0}' => out.write_str("\\_")?,
            '\u{2028}' => out.write_str("\\L")?,
            '\u{2029}' => out.write_str("\\P")?,
            c => {
                let v = c as u32;
                if v <= 0xFF {
                    write!(out, "\\x{v:02X}")?;
                } else if v <= 0xFFFF {
                    write!(out, "\\u{v:04X}")?;
                } else {
                    write!(out, "\\U{v:08X}")?;
                }
            }
        }
    }
    out.write_char('"')
}

/// `s` as go-yaml's emitter writes a single-quoted scalar, quotes included: `'`
/// is doubled and everything else is raw.
pub(crate) fn write_go_yaml_single_quoted<Out: core::fmt::Write>(
    out: &mut Out,
    s: &str,
) -> core::fmt::Result {
    out.write_char('\'')?;
    for c in s.chars() {
        if c == '\'' {
            out.write_char('\'')?;
        }
        out.write_char(c)?;
    }
    out.write_char('\'')
}

/// go-yaml's `is_break`: the characters it treats as a line break.
fn is_break(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

/// go-yaml's `is_blank`: a space or a tab.
fn is_blank(c: char) -> bool {
    matches!(c, ' ' | '\t') || is_break(c)
}

/// go-yaml's `is_printable`: the characters the emitter writes raw. Excludes
/// controls (tab included), DEL, the C1 block, U+FEFF, U+FFFE/U+FFFF and every
/// character outside the BMP.
fn is_printable(c: char) -> bool {
    matches!(
        c,
        '\n' | '\u{20}'..='\u{7E}' | '\u{A0}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}'
    ) && c != '\u{FEFF}'
}

/// Whether yaml.v3's `resolve("", s)` gives `s` a tag other than `!!str`, which
/// is when the encoder quotes it.
fn resolves_to_non_str(s: &str) -> bool {
    // `resolveMap`: the keywords, including the empty string (null) and `<<`.
    if matches!(
        s,
        "" | "~"
            | "null"
            | "Null"
            | "NULL"
            | "true"
            | "True"
            | "TRUE"
            | "false"
            | "False"
            | "FALSE"
            | ".nan"
            | ".NaN"
            | ".NAN"
            | ".inf"
            | ".Inf"
            | ".INF"
            | "+.inf"
            | "+.Inf"
            | "+.INF"
            | "-.inf"
            | "-.Inf"
            | "-.INF"
            | "<<"
    ) {
        return true;
    }
    match s.as_bytes()[0] {
        b'.' => is_go_float(s),
        b'0'..=b'9' | b'+' | b'-' => {
            if is_timestamp(s) {
                return true;
            }
            // yaml.v3 strips every underscore before it tries an integer or a
            // float, so `1_000` and `0x1_f` are numbers.
            let plain: String = s.chars().filter(|&c| c != '_').collect();
            is_go_int(&plain) || (matches_yaml_float(&plain) && is_finite_float(&plain))
        }
        _ => false,
    }
}

/// Go's `strconv.ParseInt(s, 0, 64)` or `ParseUint(s, 0, 64)` succeeds: an
/// optional sign, then a `0b`/`0o`/`0x` prefix or a leading `0` for octal, and
/// a value that fits (a sign is not accepted by `ParseUint`, so only an
/// unsigned value may use the full `u64` range).
fn is_go_int(s: &str) -> bool {
    let (negative, signed, rest) = match s.as_bytes().first() {
        Some(b'-') => (true, true, &s[1..]),
        Some(b'+') => (false, true, &s[1..]),
        _ => (false, false, s),
    };
    let bytes = rest.as_bytes();
    let (radix, digits) = match bytes {
        [] => return false,
        [b'0', p, ..] if bytes.len() >= 3 && matches!(*p | 0x20, b'b' | b'o' | b'x') => {
            match *p | 0x20 {
                b'b' => (2, &rest[2..]),
                b'o' => (8, &rest[2..]),
                _ => (16, &rest[2..]),
            }
        }
        [b'0', ..] => (8, &rest[1..]),
        _ => (10, rest),
    };
    // `0` alone is octal with no digits left, which Go accepts as zero.
    if digits.is_empty() {
        return radix == 8 && rest == "0";
    }
    let mut value: u128 = 0;
    for d in digits.bytes() {
        let Some(digit) = (d as char).to_digit(radix) else {
            return false;
        };
        value = value * u128::from(radix) + u128::from(digit);
        if value > u128::from(u64::MAX) {
            return false;
        }
    }
    let limit = match (signed, negative) {
        (false, _) => u128::from(u64::MAX),
        (true, true) => 1u128 << 63,
        (true, false) => (1u128 << 63) - 1,
    };
    value <= limit
}

/// yaml.v3's `yamlStyleFloat`:
/// `^[-+]?(\.[0-9]+|[0-9]+(\.[0-9]*)?)([eE][-+]?[0-9]+)?$`.
fn matches_yaml_float(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = usize::from(matches!(b.first(), Some(b'-' | b'+')));
    let digits = |i: &mut usize| {
        let start = *i;
        while b.get(*i).is_some_and(u8::is_ascii_digit) {
            *i += 1;
        }
        *i - start
    };
    if b.get(i) == Some(&b'.') {
        i += 1;
        if digits(&mut i) == 0 {
            return false;
        }
    } else {
        if digits(&mut i) == 0 {
            return false;
        }
        if b.get(i) == Some(&b'.') {
            i += 1;
            digits(&mut i);
        }
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'-' | b'+')) {
            i += 1;
        }
        if digits(&mut i) == 0 {
            return false;
        }
    }
    i == b.len()
}

/// Go's `ParseFloat` accepts `s` without a range error: a float that overflows
/// to infinity (`1e999`) is an error there, so it stays a string.
fn is_finite_float(s: &str) -> bool {
    s.parse::<f64>().is_ok_and(f64::is_finite)
}

/// yaml.v3's `strconv.ParseFloat(in, 64)` for a string that starts with `.`:
/// `.` digits, an optional exponent, with single underscores allowed between
/// digits as in a Go literal.
fn is_go_float(s: &str) -> bool {
    let b = s.as_bytes();
    // `_` only between two digits.
    let underscores_ok = b.iter().enumerate().all(|(i, &c)| {
        c != b'_'
            || (i > 0 && i + 1 < b.len() && b[i - 1].is_ascii_digit() && b[i + 1].is_ascii_digit())
    });
    if !underscores_ok {
        return false;
    }
    let plain: String = s.chars().filter(|&c| c != '_').collect();
    matches_yaml_float(&plain) && is_finite_float(&plain)
}

/// yaml.v3's `parseTimestamp`: four digits and a `-`, then Go's `time.Parse`
/// against `2006-1-2T15:4:5.999999999Z07:00` (also with a lower-case `t`),
/// `2006-1-2 15:4:5.999999999` and `2006-1-2`.
fn is_timestamp(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() < 5 || !b[..4].iter().all(u8::is_ascii_digit) || b[4] != b'-' {
        return false;
    }
    let year: u32 = s[..4].parse().unwrap_or(0);
    let mut cur = Cursor { b, i: 5 };
    let Some(month) = cur.number(1, 12) else {
        return false;
    };
    if !cur.eat(b'-') {
        return false;
    }
    let Some(_day) = cur.number(1, days_in_month(year, month)) else {
        return false;
    };
    if cur.done() {
        return true;
    }
    let zone = match cur.next() {
        Some(b'T' | b't') => true,
        Some(b' ') => false,
        _ => return false,
    };
    if cur.number(0, 23).is_none()
        || !cur.eat(b':')
        || cur.number(0, 59).is_none()
        || !cur.eat(b':')
        || cur.number(0, 59).is_none()
    {
        return false;
    }
    // Any number of fractional digits, after `.` or `,`.
    if matches!(cur.peek(), Some(b'.' | b','))
        && cur.b.get(cur.i + 1).is_some_and(u8::is_ascii_digit)
    {
        cur.i += 1;
        while cur.peek().is_some_and(|c| c.is_ascii_digit()) {
            cur.i += 1;
        }
    }
    if !zone {
        return cur.done();
    }
    // `Z07:00`: `Z`, or a sign and `hh:mm` (a range check that, as in Go, allows
    // 24 hours and 60 minutes).
    if cur.eat(b'Z') {
        return cur.done();
    }
    if !matches!(cur.next(), Some(b'+' | b'-')) {
        return false;
    }
    let (Some(h), Some(m)) = (cur.fixed2(), {
        if cur.eat(b':') {
            cur.fixed2()
        } else {
            None
        }
    }) else {
        return false;
    };
    h <= 24 && m <= 60 && cur.done()
}

/// The number of days in `month` (1-12) of `year`, as Go's `daysIn`.
fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 31,
    }
}

/// A byte cursor for [`is_timestamp`].
struct Cursor<'a> {
    b: &'a [u8],
    i: usize,
}

impl Cursor<'_> {
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn next(&mut self) -> Option<u8> {
        let c = self.peek()?;
        self.i += 1;
        Some(c)
    }

    fn eat(&mut self, c: u8) -> bool {
        let hit = self.peek() == Some(c);
        self.i += usize::from(hit);
        hit
    }

    fn done(&self) -> bool {
        self.i == self.b.len()
    }

    /// Go's `getnum(value, false)`: one or two digits, in `lo..=hi`.
    fn number(&mut self, lo: u32, hi: u32) -> Option<u32> {
        let first = self.peek().filter(u8::is_ascii_digit)?;
        self.i += 1;
        let mut n = u32::from(first - b'0');
        if let Some(second) = self.peek().filter(u8::is_ascii_digit) {
            self.i += 1;
            n = n * 10 + u32::from(second - b'0');
        }
        (lo..=hi).contains(&n).then_some(n)
    }

    /// Go's `getnum(value, true)`: exactly two digits.
    fn fixed2(&mut self) -> Option<u32> {
        let a = self.peek().filter(u8::is_ascii_digit)?;
        let c = self.b.get(self.i + 1).copied().filter(u8::is_ascii_digit)?;
        self.i += 2;
        Some(u32::from(a - b'0') * 10 + u32::from(c - b'0'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use EncodedStringStyle::{DoubleQuoted, Plain, SingleQuoted};

    fn double_quoted(s: &str) -> String {
        let mut out = String::new();
        write_go_yaml_double_quoted(&mut out, s).unwrap();
        out
    }

    #[test]
    fn plain_where_go_yaml_writes_plain() {
        for s in [
            "a",
            "abc",
            "a b",
            "a-b",
            "a.b",
            "a/b",
            "a=b",
            "a<b",
            "a>b",
            "a+b",
            "a$b",
            "a?b",
            "a,b",
            "a[b",
            "a]b",
            "a{b",
            "a}b",
            "a:b",
            "a#b",
            "a'b",
            "a\"b",
            "-x",
            "--",
            "?x",
            ":x",
            "::x",
            "é",
            "日本語",
            "a\u{a0}b",
            "yes",
            "No",
            "ON",
            "off",
            "y",
            "N",
            "inf",
            "Inf",
            "nan",
            "NaN",
            "12:30:45",
            "1:2",
            "1,000",
            "1.2.3",
            "1e",
            "1e+",
            "12abc",
            "abc12",
            "0x",
            "0x1p-2",
            ".",
            "..",
            ".a",
            ".iNf",
            "+inf",
            "+",
            "-x1",
            "_1",
            "_",
        ] {
            assert_eq!(go_yaml_string_style(s), Some(Plain), "{s:?}");
        }
    }

    #[test]
    fn double_quoted_where_the_string_would_read_back_as_another_type() {
        for s in [
            "",
            "~",
            "null",
            "Null",
            "NULL",
            "true",
            "True",
            "FALSE",
            "<<",
            "0",
            "1",
            "-1",
            "+1",
            "12",
            "1.5",
            ".5",
            "5.",
            "1e3",
            "1E3",
            "1e+3",
            "1.5e-3",
            "0x1f",
            "0X1F",
            "0o17",
            "0O17",
            "0b11",
            "1_000",
            "00",
            "07",
            "08",
            "0.0",
            "-0",
            "+0",
            "-0x1f",
            "+0b11",
            ".inf",
            "-.inf",
            "+.inf",
            ".Inf",
            ".INF",
            ".nan",
            ".NaN",
            ".NAN",
            "+.5",
            "-.5",
            "18446744073709551615",
            "99999999999999999999999",
            "2001-12-14",
            "2001-1-2",
            "2001-12-14T21:59:43.10-05:00",
            "2001-12-14t21:59:43Z",
            "2001-12-14 21:59:43",
            "2001-12-14 21:59:43.123456789012",
            "1_",
            "1__0",
            "0_",
            "2001-12-14T1:2:3+24:60",
            ".5_0",
            "1_0.5",
        ] {
            assert_eq!(go_yaml_string_style(s), Some(DoubleQuoted), "{s:?}");
        }
    }

    #[test]
    fn plain_where_a_number_shaped_string_does_not_resolve() {
        // Out of range, so go-yaml leaves them strings.
        assert_eq!(
            go_yaml_string_style("0xFFFFFFFFFFFFFFFFF"),
            Some(Plain),
            "a hex literal past u64 is not an int"
        );
        assert_eq!(go_yaml_string_style("-0x8000000000000001"), Some(Plain));
        assert_eq!(
            go_yaml_string_style("+9223372036854775808"),
            Some(DoubleQuoted)
        );
        assert_eq!(
            go_yaml_string_style(&format!("1{}", "0".repeat(400))),
            Some(Plain),
            "a decimal that overflows to infinity is a string"
        );
        // Not timestamps.
        for s in [
            "2001-13-14",
            "2001-12-32",
            "2001-02-30",
            "2001-12-14T24:00:00Z",
            "2001-12-14T21:60:00Z",
            "2001-12-14T21:59:60Z",
            "2001-12-14T21:59:43",
            "2001-12-14T21:59:43+0500",
            "2001-12-14 21:59:43Z",
            "2001-12-14 ",
            "2001-12-14x",
            "01-12-14",
            "20011-12-14",
        ] {
            assert_ne!(go_yaml_string_style(s), Some(DoubleQuoted), "{s:?}");
        }
    }

    #[test]
    fn single_quoted_where_plain_would_not_read_back() {
        for s in [
            "-",
            "- x",
            "---",
            "...",
            "?",
            "? x",
            ":",
            ": x",
            "::",
            ",",
            ", x",
            "[",
            "]",
            "[x]",
            "{",
            "}",
            "{x}",
            "#",
            "# x",
            "#x",
            "&",
            "&x",
            "& x",
            "*",
            "*x",
            "!",
            "!x",
            "!!str",
            "|",
            "| x",
            ">",
            "> x",
            "'",
            "'x'",
            "\"",
            "\"x\"",
            "%",
            "%x",
            "@",
            "@x",
            "`",
            "`x",
            "x ",
            " x",
            "  ",
            "x  ",
            " x ",
            "a: b",
            "a:",
            "a :",
            "a: ",
            "a #b",
            "a #",
            "key: value",
            "- item",
            "[a, b]",
            "{a: b}",
            "&amp;",
            "null ",
            "true ",
            "1 ",
            "0x1f ",
            " 0.5",
            "---x",
            "...x",
        ] {
            assert_eq!(go_yaml_string_style(s), Some(SingleQuoted), "{s:?}");
        }
    }

    #[test]
    fn double_quoted_where_the_emitter_will_not_write_a_character_raw() {
        for s in [
            "a\tb",
            "\t",
            "a\u{1}b",
            "\u{1}",
            "a\u{7f}b",
            "\u{9f}",
            "a\u{feff}b",
            "\u{feff}",
            "a\u{fffe}b",
            "😀",
            "a😀b",
            "\u{10000}",
        ] {
            assert_eq!(go_yaml_string_style(s), Some(DoubleQuoted), "{s:?}");
        }
    }

    #[test]
    fn a_string_with_a_line_break_is_not_decided_here() {
        for s in [
            "a\nb",
            "\n",
            "a\n",
            "\na",
            "a\r\nb",
            "a\u{85}b",
            "a\u{2028}b",
            "a\u{2029}b",
        ] {
            assert_eq!(go_yaml_string_style(s), None, "{s:?}");
        }
    }

    #[test]
    fn double_quoted_writer_uses_go_yaml_escapes() {
        assert_eq!(double_quoted("1"), "\"1\"");
        assert_eq!(double_quoted(""), "\"\"");
        assert_eq!(double_quoted("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(double_quoted("a\tb"), "\"a\\tb\"");
        assert_eq!(
            double_quoted("\u{0}\u{7}\u{8}\u{b}\u{c}\u{1b}"),
            "\"\\0\\a\\b\\v\\f\\e\""
        );
        assert_eq!(double_quoted("\u{1}\u{1f}"), "\"\\x01\\x1F\"");
        assert_eq!(double_quoted("\u{7f}"), "\"\\x7F\"");
        assert_eq!(double_quoted("\u{85}"), "\"\\N\"");
        assert_eq!(double_quoted("\u{9f}"), "\"\\x9F\"");
        assert_eq!(double_quoted("\u{feff}"), "\"\\uFEFF\"");
        assert_eq!(double_quoted("\u{ffff}"), "\"\\uFFFF\"");
        assert_eq!(double_quoted("😀"), "\"\\U0001F600\"");
        assert_eq!(double_quoted("é日\u{a0}"), "\"é日\u{a0}\"");
        // A leading BOM escapes everything after it, and only a leading one.
        assert_eq!(double_quoted("\u{feff}x"), "\"\\uFEFF\\x78\"");
        assert_eq!(
            double_quoted("\u{feff}\u{feff}x"),
            "\"\\uFEFF\\uFEFF\\x78\""
        );
        assert_eq!(
            double_quoted("\u{feff}xy\u{feff}z"),
            "\"\\uFEFF\\x78\\x79\\uFEFF\\x7A\""
        );
        assert_eq!(double_quoted("\u{feff}\u{a0}"), "\"\\uFEFF\\_\"");
        assert_eq!(
            double_quoted("\u{feff}é日😀"),
            "\"\\uFEFF\\xE9\\u65E5\\U0001F600\""
        );
        assert_eq!(
            double_quoted("\u{feff}\"\\ 5"),
            "\"\\uFEFF\\\"\\\\\\x20\\x35\""
        );
        assert_eq!(double_quoted("\u{feff}\u{7}\t"), "\"\\uFEFF\\a\\t\"");
        assert_eq!(double_quoted("x\u{feff}y"), "\"x\\uFEFFy\"");
        assert_eq!(double_quoted("\u{a0}\u{feff}x"), "\"\u{a0}\\uFEFFx\"");
        // A break the style function never routes here still has an escape.
        assert_eq!(double_quoted("a\nb\r"), "\"a\\nb\\r\"");
        assert_eq!(double_quoted("\u{2028}\u{2029}"), "\"\\L\\P\"");
    }

    #[test]
    fn single_quoted_writer_doubles_the_quote() {
        let mut out = String::new();
        write_go_yaml_single_quoted(&mut out, "it's 'x'").unwrap();
        assert_eq!(out, "'it''s ''x'''");
        let mut out = String::new();
        write_go_yaml_single_quoted(&mut out, "- x").unwrap();
        assert_eq!(out, "'- x'");
    }

    #[test]
    fn go_int_follows_parse_int_base_zero() {
        for s in [
            "0", "00", "07", "0x1f", "0X1F", "0b11", "0B11", "0o17", "-0x1f", "+5",
        ] {
            assert!(is_go_int(s), "{s:?}");
        }
        for s in [
            "", "-", "+", "08", "0x", "0b", "0o", "0b2", "0o8", "0xg", "1x", "--1",
        ] {
            assert!(!is_go_int(s), "{s:?}");
        }
        assert!(is_go_int("9223372036854775807"));
        assert!(is_go_int("-9223372036854775808"));
        assert!(is_go_int("18446744073709551615"));
        assert!(
            !is_go_int("+9223372036854775808"),
            "ParseUint takes no sign"
        );
        assert!(!is_go_int("-9223372036854775809"));
        assert!(!is_go_int("18446744073709551616"));
    }

    #[test]
    fn yaml_style_float_is_the_yaml_v3_regex() {
        for s in [
            "1", "1.", "1.5", ".5", "+.5", "-1.5e3", "1e3", "1E+3", "5.e1", "0",
        ] {
            assert!(matches_yaml_float(s), "{s:?}");
        }
        for s in [
            "", ".", "+", "e3", "1e", "1e+", ".e3", "1.2.3", "1_0", "0x1", "inf", "1 ",
        ] {
            assert!(!matches_yaml_float(s), "{s:?}");
        }
    }
}
