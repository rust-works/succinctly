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
//! [`go_yaml_string_style`] is [`None`] for a string with a line break (`\n`, `\r`,
//! U+0085, U+2028, U+2029): the single-line choice has nothing to say about it.
//! [`go_yaml_multiline_scalar`] writes those (#3587): a literal block, double quotes,
//! or single quotes across lines, and [`go_yaml_explicit_key_scalar`] the `? ` explicit-key
//! syntax for a key with a break or over 128 bytes.

use super::scalar::{resolve_plain, ResolvedScalar};

#[cfg(not(test))]
use alloc::{
    borrow::Cow,
    format,
    string::{String, ToString},
};
#[cfg(test)]
use std::{
    borrow::Cow,
    format,
    string::{String, ToString},
};

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

/// The style go-yaml's encoder gives `s`, in flow context (inside `[...]` or
/// `{...}`) when `flow` is set and in block context otherwise, or `None` for a
/// string with a line break (see the module docs).
///
/// Flow context quotes more: a `,`, `[`, `]`, `{`, `}` or `?` anywhere, and a `:`
/// anywhere (not only before a blank), because each can end or restructure a flow
/// collection (`{v: ['a:b', 'x,y']}`), where block context leaves them bare.
#[must_use]
pub(crate) fn go_yaml_string_style(s: &str, flow: bool) -> Option<EncodedStringStyle> {
    go_yaml_string_style_typed(s, flow, true)
}

/// Whether go-yaml's emitter single-quotes `s` where it sits in flow context as a *plain*
/// scalar read from the source, whatever type it resolves to.
///
/// The emitter decides on the characters alone (`flow_indicators`), so a plain `x:y`,
/// `http://a`, `12:30:45`, `2001-12-14T21:59:43Z` or `a?b` inside `[...]` or `{...}` is
/// written `'x:y'` and the type check [`go_yaml_string_style`] adds does not apply: a plain
/// scalar of another type is still quoted when it holds a `:` or `?` (#5467).
#[must_use]
pub(crate) fn flow_plain_needs_single_quotes(s: &str) -> bool {
    matches!(
        go_yaml_string_style_typed(s, true, false),
        Some(EncodedStringStyle::SingleQuoted)
    )
}

/// [`go_yaml_string_style`], optionally without the type check: with an explicit `!!str`
/// tag in front (yq's `style = "tagged"`, #4066) a string that would read back as
/// another type is written plain, since the tag already says what it is (`!!str 1`).
fn go_yaml_string_style_typed(s: &str, flow: bool, typed: bool) -> Option<EncodedStringStyle> {
    if has_break(s) {
        return None;
    }
    // yaml.v3 `stringv`: a string that would resolve to another tag when
    // written plain is double-quoted. The empty string resolves to null.
    if typed && resolves_to_non_str(s) {
        return Some(EncodedStringStyle::DoubleQuoted);
    }
    // The emitter's `yaml_emitter_analyze_scalar`, for block context and a
    // string with no line break.
    let mut special = false;
    let document_marker = s.starts_with("---") || s.starts_with("...");
    let mut block_indicators = document_marker;
    let mut flow_indicators = document_marker;
    let mut preceded_by_blank = true;
    let mut chars = s.chars().peekable();
    let mut first = true;
    while let Some(c) = chars.next() {
        // `is_blankz` of the next character: a blank, or the end of the string.
        let followed_by_blank = chars.peek().map_or(true, |&n| is_blankz(n));
        if first {
            match c {
                '#' | ',' | '[' | ']' | '{' | '}' | '&' | '*' | '!' | '|' | '>' | '\'' | '"'
                | '%' | '@' | '`' => {
                    block_indicators = true;
                    flow_indicators = true;
                }
                '?' | ':' => {
                    flow_indicators = true;
                    block_indicators |= followed_by_blank;
                }
                '-' if followed_by_blank => {
                    block_indicators = true;
                    flow_indicators = true;
                }
                _ => {}
            }
        } else {
            match c {
                ',' | '?' | '[' | ']' | '{' | '}' => flow_indicators = true,
                ':' => {
                    flow_indicators = true;
                    block_indicators |= followed_by_blank;
                }
                '#' if preceded_by_blank => {
                    block_indicators = true;
                    flow_indicators = true;
                }
                _ => {}
            }
        }
        special |= !is_printable(c);
        preceded_by_blank = is_blankz(c);
        first = false;
    }
    if special {
        Some(EncodedStringStyle::DoubleQuoted)
    } else if (if flow {
        flow_indicators
    } else {
        block_indicators
    }) || s.starts_with(' ')
        || s.ends_with(' ')
    {
        Some(EncodedStringStyle::SingleQuoted)
    } else {
        Some(EncodedStringStyle::Plain)
    }
}

/// A character go-yaml's emitter treats as a line break, other than `\r`, which it
/// will not write raw.
fn is_yaml_line_break(c: char) -> bool {
    matches!(c, '\n' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

/// Whether go-yaml writes a string with a line break double-quoted.
///
/// Whatever the requested style: a character its emitter will not write raw (a control,
/// `\r`, a character outside the BMP) or a space before a line break (libyaml's
/// `special_characters` and `space_break`).
pub fn go_yaml_forces_double_quotes(s: &str) -> bool {
    let printable = |c: char| {
        matches!(
            c,
            '\n' | '\t' | '\u{20}'..='\u{7E}' | '\u{A0}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}'
        ) && c != '\u{FEFF}'
    };
    let mut previous = '\0';
    for c in s.chars() {
        if !printable(c) || (previous == ' ' && is_yaml_line_break(c)) {
            return true;
        }
        previous = c;
    }
    false
}

/// The block scalar (`|` literal or `>` folded) go-yaml writes for `s`.
///
/// Header line included and without the final line break (the entry separator
/// supplies it), or `None` when go-yaml would not write one (#2707, #3587).
///
/// `indent` is the indent of the content lines and `step` the width of one
/// indent unit, which is the indentation indicator go-yaml writes when the first
/// line starts with a space (`|2-`). Ported from libyaml's
/// `yaml_emitter_analyze_scalar` (what disqualifies a block) and
/// `yaml_emitter_write_{literal,folded}_scalar` (how it is written), then
/// checked against yq v4.53.3 on random strings:
///
/// - the chomping indicator comes from the trailing breaks: none is `-`, one
///   is clip (no indicator), two or more (or a lone break) is `+`;
/// - a string go-yaml cannot write as a block (empty, a character it will not
///   write raw, a space before a break, a trailing space, any break other
///   than `\n`) is `None`, and its caller falls back to quoting;
/// - folded writes an empty line for every `\n` -- except that go-yaml tests the
///   first character of the *string*, not the one after the break, so a string
///   starting with a space, tab or break gets none.
pub fn go_yaml_block_scalar(s: &str, folded: bool, indent: &str, step: usize) -> Option<String> {
    if s.is_empty() || step == 0 || step > 9 {
        return None;
    }
    let last = s.chars().next_back()?;
    if go_yaml_forces_double_quotes(s) || last == ' ' || (last != '\n' && is_yaml_line_break(last))
    {
        return None;
    }
    // go-yaml tests the string's first character only, so `"\n a"` gets no indicator
    // and reads back as `"\na"` (yq cannot read its own output). A literal block looks
    // past the leading breaks instead, as `stream_yaml_block_scalar` does for a source
    // block scalar (#836): output that corrupts the value is the one place the reference
    // is not followed (ADR-0018). Folded keeps go-yaml's rule, which its own quirk below
    // is tied to.
    let first_is_space = if folded {
        s.starts_with(' ')
    } else {
        s.chars().find(|&c| !is_yaml_line_break(c)) == Some(' ')
    };
    let hint = if first_is_space {
        char::from_digit(step as u32, 10)?.to_string()
    } else {
        String::new()
    };
    let breaks_at_end = s
        .chars()
        .rev()
        .take_while(|&c| is_yaml_line_break(c))
        .count();
    let chomp = match breaks_at_end {
        0 => "-",
        1 if s.len() > 1 => "",
        _ => "+",
    };
    let mut out = format!("{}{hint}{chomp}\n", if folded { '>' } else { '|' });
    // The final break is the entry separator, not content.
    let text: &str = s.strip_suffix(is_yaml_line_break).unwrap_or(s);
    if folded {
        // go-yaml's quirk, see the doc comment: the test reads the string's start.
        let extra_break = s
            .chars()
            .find(|&c| !is_yaml_line_break(c))
            .is_some_and(|c| !matches!(c, ' ' | '\t'));
        let mut breaks = true;
        let mut leading_spaces = true;
        for c in text.chars() {
            if is_yaml_line_break(c) {
                if !breaks && !leading_spaces && c == '\n' && extra_break {
                    out.push('\n');
                }
                out.push(c);
                breaks = true;
            } else {
                if breaks {
                    out.push_str(indent);
                    leading_spaces = matches!(c, ' ' | '\t');
                }
                out.push(c);
                breaks = false;
            }
        }
        // The final break, when the string ended in one, still goes through the
        // loop above in go-yaml and earns its extra empty line.
        if s.ends_with('\n') && !breaks && !leading_spaces && extra_break {
            out.push('\n');
        }
    } else {
        let mut breaks = true;
        for c in text.chars() {
            if is_yaml_line_break(c) {
                out.push(c);
                breaks = true;
            } else {
                if breaks {
                    out.push_str(indent);
                }
                out.push(c);
                breaks = false;
            }
        }
    }
    Some(out)
}

/// A string holding a line break, as go-yaml's encoder writes it in block context (#3587).
///
/// `None` for a string with none. `indent` is the indent of a literal block's
/// lines (and of what follows a break inside single quotes) and `step` the width of one
/// indent unit, as for [`go_yaml_block_scalar`]; the text has no final line break.
///
/// - A `\n` asks for the literal style, which the emitter allows or turns into double
///   quotes ([`go_yaml_block_scalar`]).
/// - Any other break (`\r`, U+0085, U+2028, U+2029) asks for plain, which a break
///   refuses; single quotes take it unless the string holds a character the emitter
///   will not write raw (`\r`, U+0085, a tab) or a space next to a break, and double
///   quotes otherwise. Inside single quotes a break is written as itself and the
///   next character is indented.
#[must_use]
pub fn go_yaml_multiline_scalar(s: &str, indent: &str, step: usize) -> Option<String> {
    if !has_break(s) {
        return None;
    }
    if s.contains('\n') {
        return Some(
            go_yaml_block_scalar(s, false, indent, step)
                .unwrap_or_else(|| go_yaml_double_quoted_scalar(s)),
        );
    }
    let mut previous = '\0';
    let refused = s.chars().any(|c| {
        let adjacent = (previous == ' ' && is_break(c)) || (is_break(previous) && c == ' ');
        previous = c;
        !is_printable(c) || adjacent
    });
    if refused {
        return Some(go_yaml_double_quoted_scalar(s));
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    let mut breaks = false;
    for c in s.chars() {
        if is_break(c) {
            out.push(c);
            breaks = true;
        } else {
            if breaks {
                out.push_str(indent);
            }
            if c == '\'' {
                out.push('\'');
            }
            out.push(c);
            breaks = false;
        }
    }
    out.push('\'');
    Some(out)
}

/// Whether `s` is written as an ordinary `key:` (#3587).
///
/// One line and at most 128 bytes (`yaml_emitter_check_simple_key`). Cheap, for a caller
/// that would otherwise build the indent [`go_yaml_explicit_key_scalar`] takes for every key.
#[must_use]
pub fn go_yaml_key_is_simple(s: &str) -> bool {
    s.len() <= 128 && !has_break(s)
}

/// The scalar go-yaml writes after the `? ` of a mapping key that needs explicit-key
/// syntax, or `None` for a key that is written the ordinary way (#3587).
///
/// `yaml_emitter_check_simple_key`: a key is simple only if its scalar is one line and
/// at most 128 bytes. The scalar after the `? ` is chosen as a value's is (the
/// restriction to a simple key does not apply to it): [`go_yaml_multiline_scalar`] for a
/// line break, the single-line choice for a long key.
#[must_use]
pub fn go_yaml_explicit_key_scalar(s: &str, indent: &str, step: usize) -> Option<String> {
    if go_yaml_key_is_simple(s) {
        return None;
    }
    go_yaml_multiline_scalar(s, indent, step).or_else(|| go_yaml_string_scalar(s, false))
}

/// Writes `s` in the style go-yaml's encoder picks, or returns `None` for a line break.
///
/// The one place a [`EncodedStringStyle`] becomes text, so the streaming route, the
/// computed-value streamer and [`go_yaml_string_scalar`] cannot drift apart. `None` is
/// [`go_yaml_string_style`]'s own (see the module docs) and writes nothing.
pub(crate) fn write_go_yaml_string<Out: core::fmt::Write>(
    out: &mut Out,
    s: &str,
    flow: bool,
) -> Option<core::fmt::Result> {
    Some(match go_yaml_string_style(s, flow)? {
        EncodedStringStyle::Plain => out.write_str(s),
        EncodedStringStyle::SingleQuoted => write_go_yaml_single_quoted(out, s),
        EncodedStringStyle::DoubleQuoted => write_go_yaml_double_quoted(out, s),
    })
}

/// `s` as the YAML scalar go-yaml's encoder writes, or `None` for a line break.
#[must_use]
pub(crate) fn go_yaml_string_scalar(s: &str, flow: bool) -> Option<String> {
    let mut out = String::new();
    write_go_yaml_string(&mut out, s, flow)?.ok()?;
    Some(out)
}

/// The scalar the yq command line's DOM writer should print for a string that has no
/// style of its own to preserve, or `None` to leave it to that writer's older rule.
///
/// A string with no recorded style is either decoded from JSON, computed by a filter,
/// or a *plain* scalar of a YAML document, and the DOM holds only its text, so which
/// it was is the `json_sourced` flag and a guess:
///
/// - **JSON** (`json_sourced`): always a real string, so it takes go-yaml's choice
///   (`"1"`, `'- x'`, `a b`).
/// - **A YAML document**: a spelling yq reads as another type is most likely a plain
///   scalar of that type, which must be echoed or its type would change on any write.
///   A *key* (`200: ok`, `~: v`, `2001-12-14T01:02:03Z: a`) is echoed as it is, except
///   the empty one, which is left to the caller. A *value* is left to the caller when
///   the YAML 1.2 core schema calls it a string and yq does not (a timestamp, `1_000`,
///   `0b11`, `<<`); one the core schema also types (`true`, `1`, `null`) can only be a
///   string that was quoted or computed, and is quoted.
///
/// `flow` is whether the scalar sits inside `[...]` or `{...}`. A string with a line
/// break is `None`.
#[must_use]
pub fn go_yaml_dom_scalar(s: &str, flow: bool, json_sourced: bool, is_key: bool) -> Option<String> {
    if !json_sourced && resolves_to_non_str(s) {
        if is_key {
            return (!s.is_empty()).then(|| String::from(s));
        }
        if matches!(resolve_plain(s), ResolvedScalar::Str) {
            // Inside `[...]` or `{...}` yq quotes by characters alone, so a plain timestamp or
            // `12:30:45` is single-quoted there even though it is left bare in block context
            // (#5469).
            if flow && !s.is_empty() && flow_plain_needs_single_quotes(s) {
                let mut out = String::new();
                if write_go_yaml_single_quoted(&mut out, s).is_ok() {
                    return Some(out);
                }
            }
            return None;
        }
    }
    go_yaml_string_scalar(s, flow)
}

/// The scalar go-yaml writes after an explicit `!!str` tag, or `None` for a line break.
///
/// For yq's `style = "tagged"` (#4066): the usual choice of style without the rule that
/// quotes a string reading back as another type. Empty for the empty string, which is
/// the tag alone (`a: !!str`).
#[must_use]
pub fn go_yaml_tagged_str_scalar(s: &str, flow: bool) -> Option<String> {
    let mut out = String::new();
    match go_yaml_string_style_typed(s, flow, false)? {
        EncodedStringStyle::Plain => out.push_str(s),
        EncodedStringStyle::SingleQuoted => write_go_yaml_single_quoted(&mut out, s).ok()?,
        EncodedStringStyle::DoubleQuoted => write_go_yaml_double_quoted(&mut out, s).ok()?,
    }
    Some(out)
}

/// `s` as go-yaml's emitter writes a double-quoted scalar, as a string.
///
/// For the yq command line's DOM writer (#2707): the fallback for a string it was
/// asked to write as a block scalar (`style = "literal"`) that go-yaml cannot.
#[must_use]
pub fn go_yaml_double_quoted_scalar(s: &str) -> String {
    let mut out = String::new();
    // Writing into a `String` cannot fail.
    let _ = write_go_yaml_double_quoted(&mut out, s);
    out
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
    // The common case, printable ASCII with nothing to escape, is written whole.
    if s.bytes()
        .all(|b| (0x20..0x7F).contains(&b) && b != b'"' && b != b'\\')
    {
        out.write_char('"')?;
        out.write_str(s)?;
        return out.write_char('"');
    }
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

/// Whether `s` holds U+2028 or U+2029 (#4865).
///
/// Of go-yaml's line breaks these are the two its scanner keeps as characters in a folded scalar,
/// and its emitter writes them followed by the indent, inside single quotes.
fn has_unicode_line_separator(s: &str) -> bool {
    !s.is_ascii() && s.chars().any(|c| matches!(c, '\u{2028}' | '\u{2029}'))
}

/// A string whose only line breaks are U+2028/U+2029, as yq writes a YAML source scalar (#4865).
///
/// `None` for any other string, which keeps the source's own spelling. The one definition the
/// streaming route, the write route and a flow item share; [`go_yaml_multiline_scalar`] has the
/// layout rules, and `indent` is the indent of the scalar itself.
#[must_use]
pub fn go_yaml_unicode_separator_scalar(s: &str, indent: &str, step: usize) -> Option<String> {
    if s.contains('\n') || !has_unicode_line_separator(s) {
        return None;
    }
    go_yaml_multiline_scalar(s, indent, step)
}

/// Whether `s` holds a character go-yaml's emitter will not write raw.
///
/// A tab, DEL, a C1 control, U+FEFF or a character outside the BMP keeps a string out of the plain
/// and single-quoted styles whatever style its source had (#4858). A line break (`\n`, U+0085,
/// U+2028, U+2029) is left to the line-break rules, which differ from this one.
#[must_use]
pub fn has_unwritable_char(s: &str) -> bool {
    unwritable_char(s, false)
}

/// [`has_unwritable_char`] for the content of a block scalar, where a tab is written as it is
/// (`|-` over `x<TAB>y` stays a literal block in yq).
#[must_use]
pub(crate) fn has_unwritable_char_in_block(s: &str) -> bool {
    unwritable_char(s, true)
}

fn unwritable_char(s: &str, tab_ok: bool) -> bool {
    !s.bytes().all(|b| (0x20..0x7F).contains(&b) || b == b'\n')
        && s.chars()
            .any(|c| !(is_break(c) || is_printable(c) || (tab_ok && c == '\t')))
}

/// `s` as go-yaml's emitter writes a single-quoted scalar, quotes included: `'`
/// is doubled and everything else is raw. A string with a character the emitter will not write
/// raw ([`has_unwritable_char`]) is double-quoted instead, as yq re-emits a single-quoted source
/// scalar that holds one (#4858).
pub(crate) fn write_go_yaml_single_quoted<Out: core::fmt::Write>(
    out: &mut Out,
    s: &str,
) -> core::fmt::Result {
    if has_unwritable_char(s) {
        return write_go_yaml_double_quoted(out, s);
    }
    out.write_char('\'')?;
    for c in s.chars() {
        if c == '\'' {
            out.write_char('\'')?;
        }
        out.write_char(c)?;
    }
    out.write_char('\'')
}

/// Whether `s` holds a line break. The ASCII breaks are a byte scan, and the
/// three that are not ASCII (U+0085, U+2028, U+2029) are only looked for in a
/// string that has a non-ASCII character at all, so the common string costs one
/// pass of byte compares.
fn has_break(s: &str) -> bool {
    s.bytes().any(|b| matches!(b, b'\n' | b'\r')) || (!s.is_ascii() && s.chars().any(is_break))
}

/// go-yaml's `is_break`: the characters it treats as a line break.
fn is_break(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

/// go-yaml's `is_blankz` for a character: a space, a tab or a line break. (The
/// `z` is the end of the string, which the one caller that can see it, in
/// [`go_yaml_string_style`], counts as blank itself.)
fn is_blankz(c: char) -> bool {
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
    go_yaml_resolved_tag(s) != "!!str"
}

/// The tag yaml.v3's `resolve("", s)` gives a plain scalar spelled `s` (#2796).
///
/// What go-yaml's encoder compares an assigned tag against: a tag equal to this one is
/// implicit and not written. `-0` is a float (captured from yq: `tag = "!!float"` leaves
/// `-0` bare and `tag = "!!int"` writes `!!int -0`).
#[must_use]
pub fn go_yaml_resolved_tag(s: &str) -> &'static str {
    // `resolveMap`: the keywords, including the empty string (null) and `<<`.
    match s {
        "" | "~" | "null" | "Null" | "NULL" => return "!!null",
        "true" | "True" | "TRUE" | "false" | "False" | "FALSE" => return "!!bool",
        ".nan" | ".NaN" | ".NAN" | ".inf" | ".Inf" | ".INF" | "+.inf" | "+.Inf" | "+.INF"
        | "-.inf" | "-.Inf" | "-.INF" => return "!!float",
        "<<" => return "!!merge",
        _ => {}
    }
    match s.as_bytes()[0] {
        b'.' if is_go_float(s) => "!!float",
        b'0'..=b'9' | b'+' | b'-' => {
            if is_timestamp(s) {
                return "!!timestamp";
            }
            // yaml.v3 strips every underscore before it tries an integer or a
            // float, so `1_000` and `0x1_f` are numbers.
            let plain = without_underscores(s);
            if plain == "-0" {
                "!!float"
            } else if is_go_int(&plain) {
                "!!int"
            } else if matches_yaml_float(&plain) && is_finite_float(&plain) {
                "!!float"
            } else {
                "!!str"
            }
        }
        _ => "!!str",
    }
}

/// `s` without its underscores, borrowed when it has none (the common case, a
/// number, a date or a version, so a string that starts like one allocates
/// nothing).
fn without_underscores(s: &str) -> Cow<'_, str> {
    if s.contains('_') {
        Cow::Owned(s.chars().filter(|&c| c != '_').collect::<String>())
    } else {
        Cow::Borrowed(s)
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
    let plain = without_underscores(s);
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
            // Underscores are only a digit separator between two digits.
            "._5",
            ".5_",
            ".5__0",
            // Shaped like a date and not one.
            "2001-12",
            "2001-1x-1",
            "2001-1/2",
            "1900-02-29",
            "2100-02-29",
            "2001-02-29",
            "2004-02-30",
            "2001-04-31",
        ] {
            assert_eq!(go_yaml_string_style(s, false), Some(Plain), "{s:?}");
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
            // A real date: 2004 and 2000 are leap years, April has 30 days.
            "2004-02-29",
            "2000-02-29",
            "2001-04-30",
            "2001-12-14T1:2:3+24:60",
            ".5_0",
            "1_0.5",
        ] {
            assert_eq!(go_yaml_string_style(s, false), Some(DoubleQuoted), "{s:?}");
        }
    }

    #[test]
    fn plain_where_a_number_shaped_string_does_not_resolve() {
        // Out of range, so go-yaml leaves them strings.
        assert_eq!(
            go_yaml_string_style("0xFFFFFFFFFFFFFFFFF", false),
            Some(Plain),
            "a hex literal past u64 is not an int"
        );
        assert_eq!(
            go_yaml_string_style("-0x8000000000000001", false),
            Some(Plain)
        );
        assert_eq!(
            go_yaml_string_style("+9223372036854775808", false),
            Some(DoubleQuoted)
        );
        assert_eq!(
            go_yaml_string_style(&format!("1{}", "0".repeat(400)), false),
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
            assert_ne!(go_yaml_string_style(s, false), Some(DoubleQuoted), "{s:?}");
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
            assert_eq!(go_yaml_string_style(s, false), Some(SingleQuoted), "{s:?}");
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
            assert_eq!(go_yaml_string_style(s, false), Some(DoubleQuoted), "{s:?}");
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
            assert_eq!(go_yaml_string_style(s, false), None, "{s:?}");
        }
    }

    #[test]
    fn single_quoted_writer_falls_back_to_double_quotes_for_an_unwritable_character_4858() {
        let single = |s: &str| {
            let mut out = String::new();
            write_go_yaml_single_quoted(&mut out, s).unwrap();
            out
        };
        assert_eq!(single("it's"), "'it''s'");
        assert_eq!(single("x\u{e9}y"), "'x\u{e9}y'");
        assert_eq!(single("x\u{7f}y"), "\"x\\x7Fy\"");
        assert_eq!(single("x\ty"), "\"x\\ty\"");
        assert_eq!(single("x\u{90}y"), "\"x\\x90y\"");
        assert_eq!(single("\u{feff}x"), "\"\\uFEFF\\x78\"");
        assert_eq!(single("x\u{1F600}"), "\"x\\U0001F600\"");
        // A line break is the line-break rules' business, not this predicate's.
        assert_eq!(single("x\u{2028}y"), "'x\u{2028}y'");
        assert!(has_unicode_line_separator("x\u{2028}y") && has_unicode_line_separator("\u{2029}"));
        assert!(!has_unicode_line_separator("plain") && !has_unicode_line_separator("a\nb\u{85}c"));
        assert!(!has_unicode_line_separator("caf\u{e9}"));
        // Only a string whose breaks are all U+2028/U+2029 is respelled; a `\n` keeps the source style.
        assert_eq!(
            go_yaml_unicode_separator_scalar("x\u{2028}y", "  ", 2).as_deref(),
            Some("'x\u{2028}  y'")
        );
        assert_eq!(
            go_yaml_unicode_separator_scalar("x\u{2028}y\nz", "  ", 2),
            None
        );
        assert_eq!(go_yaml_unicode_separator_scalar("plain", "  ", 2), None);
        assert!(has_unwritable_char("a\tb") && !has_unwritable_char_in_block("a\tb"));
        assert!(has_unwritable_char_in_block("a\u{7f}b"));
        assert!(!has_unwritable_char("plain text\nsecond line"));
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
    /// Flow context (inside `[...]` or `{...}`) quotes what could end or restructure
    /// the collection: `,`, `[`, `]`, `{`, `}` and `?` anywhere, and a `:` anywhere,
    /// not only before a blank. Each row is a string yq v4.53.3 writes bare in a
    /// block sequence and single-quoted in `{v: ['...']}` (#3588).
    #[test]
    fn flow_context_quotes_what_block_context_leaves_bare() {
        for s in [
            "a,b",
            "a[b",
            "a]b",
            "a{b",
            "a}b",
            "a?b",
            "12:30:45",
            "1:2",
            "1,000",
            "?x",
            ":x",
            "a:b",
            "a :b",
            "a, b",
            "http://x.y/z?q=1#f",
            "C:\\path",
            "${x}",
        ] {
            assert_eq!(
                go_yaml_string_style(s, false),
                Some(Plain),
                "{s:?} in block"
            );
            assert_eq!(
                go_yaml_string_style(s, true),
                Some(SingleQuoted),
                "{s:?} in flow"
            );
        }
        // Everything else answers the same in both contexts.
        for s in [
            "a", "a b", "a-b", "a.b", "a#b", "é", "yes", "-x", "a'b", "a\"b",
        ] {
            assert_eq!(go_yaml_string_style(s, true), Some(Plain), "{s:?}");
        }
        for s in ["1", "true", "", "a\tb", "2001-12-14", "\u{1F600}"] {
            assert_eq!(go_yaml_string_style(s, true), Some(DoubleQuoted), "{s:?}");
        }
        for s in ["- x", "a: b", "x ", " x", "#x", "---", "!x", "&x"] {
            assert_eq!(go_yaml_string_style(s, true), Some(SingleQuoted), "{s:?}");
            assert_eq!(go_yaml_string_style(s, false), Some(SingleQuoted), "{s:?}");
        }
        assert_eq!(go_yaml_string_style("a\nb", true), None);
    }

    #[test]
    fn the_scalar_function_renders_each_style_and_declines_a_line_break() {
        assert_eq!(go_yaml_string_scalar("abc", false).as_deref(), Some("abc"));
        assert_eq!(go_yaml_string_scalar("1", false).as_deref(), Some("\"1\""));
        assert_eq!(go_yaml_string_scalar("", false).as_deref(), Some("\"\""));
        assert_eq!(
            go_yaml_string_scalar("- x", false).as_deref(),
            Some("'- x'")
        );
        assert_eq!(
            go_yaml_string_scalar("it's: x", false).as_deref(),
            Some("'it''s: x'")
        );
        assert_eq!(
            go_yaml_string_scalar("a\tb", false).as_deref(),
            Some("\"a\\tb\"")
        );
        assert_eq!(
            go_yaml_string_scalar("\u{1F600}", true).as_deref(),
            Some("\"\\U0001F600\"")
        );
        assert_eq!(go_yaml_string_scalar("a,b", false).as_deref(), Some("a,b"));
        assert_eq!(go_yaml_string_scalar("a,b", true).as_deref(), Some("'a,b'"));
        assert_eq!(go_yaml_string_scalar("a\nb", false), None);
        assert_eq!(go_yaml_string_scalar("a\u{2028}b", true), None);
        let mut out = String::from("k: ");
        assert!(write_go_yaml_string(&mut out, "1", false).unwrap().is_ok());
        assert_eq!(out, "k: \"1\"");
        assert!(write_go_yaml_string(&mut out, "a\nb", false).is_none());
        assert_eq!(out, "k: \"1\"", "a declined string writes nothing");
    }

    /// A JSON string is always a string, so it takes go-yaml's choice whatever it is
    /// spelled like: `"1"` is `"1"`, a date is `"2001-12-14"`, a key `~` is `"~"`.
    #[test]
    fn dom_scalar_for_json_is_the_encoders_choice_for_keys_and_values() {
        for (s, expected) in [
            ("abc", "abc"),
            ("1", "\"1\""),
            ("true", "\"true\""),
            ("~", "\"~\""),
            ("2001-12-14", "\"2001-12-14\""),
            ("1_000", "\"1_000\""),
            ("<<", "\"<<\""),
            ("", "\"\""),
            ("- x", "'- x'"),
        ] {
            for is_key in [false, true] {
                assert_eq!(
                    go_yaml_dom_scalar(s, false, true, is_key).as_deref(),
                    Some(expected),
                    "{s:?} key={is_key}"
                );
            }
        }
        assert_eq!(go_yaml_dom_scalar("a\nb", false, true, false), None);
    }

    /// A YAML document's key spelled like another type is a typed key: echoed as
    /// it is (the empty one is left to the caller), never quoted into a string key.
    #[test]
    fn dom_scalar_for_a_yaml_key_echoes_a_typed_spelling() {
        for s in [
            "200",
            "-1",
            "~",
            "null",
            "true",
            "1.5",
            "0x1f",
            "1_000",
            "2001-12-14",
            "2001-12-14T01:02:03Z",
            "2001-12-14 21:59:43",
        ] {
            assert_eq!(
                go_yaml_dom_scalar(s, false, false, true).as_deref(),
                Some(s),
                "{s:?}"
            );
            assert_eq!(
                go_yaml_dom_scalar(s, true, false, true).as_deref(),
                Some(s),
                "{s:?} flow"
            );
        }
        assert_eq!(go_yaml_dom_scalar("", false, false, true), None);
        // Anything that is a string takes the encoder's choice.
        assert_eq!(
            go_yaml_dom_scalar("a b", false, false, true).as_deref(),
            Some("a b")
        );
        assert_eq!(
            go_yaml_dom_scalar("x: y", false, false, true).as_deref(),
            Some("'x: y'")
        );
        assert_eq!(
            go_yaml_dom_scalar("- x", false, false, true).as_deref(),
            Some("'- x'")
        );
        assert_eq!(
            go_yaml_dom_scalar("a,b", true, false, true).as_deref(),
            Some("'a,b'")
        );
    }

    /// A YAML document's string value with no style: a spelling the core schema calls a
    /// string but yq types (a timestamp, `1_000`, `0b11`, `<<`) is a plain scalar of that
    /// type and is left to the caller to echo; one the core schema types too can only be a
    /// string that was quoted or computed, and is quoted.
    #[test]
    fn dom_scalar_for_a_yaml_value_leaves_a_yq_typed_plain_scalar_alone() {
        for s in [
            "2001-12-14",
            "2001-12-14T01:02:03Z",
            "1_000",
            "0b11",
            "0X1F",
            "<<",
        ] {
            assert_eq!(go_yaml_dom_scalar(s, false, false, false), None, "{s:?}");
        }
        for (s, expected) in [
            ("true", "\"true\""),
            ("1", "\"1\""),
            ("null", "\"null\""),
            ("~", "\"~\""),
            (".inf", "\".inf\""),
            ("+.inf", "\"+.inf\""),
            ("", "\"\""),
            ("abc", "abc"),
            ("a b", "a b"),
            ("- x", "'- x'"),
            ("yes", "yes"),
        ] {
            assert_eq!(
                go_yaml_dom_scalar(s, false, false, false).as_deref(),
                Some(expected),
                "{s:?}"
            );
        }
        assert_eq!(go_yaml_dom_scalar("a\nb", false, false, false), None);
    }

    /// #3587: every row is yq v4.53.3's output for the string as a mapping value
    /// (`yq -p json -o yaml`, indent step 2), the literal block where go-yaml's emitter
    /// allows one and double quotes where not.
    #[test]
    fn a_line_break_gives_a_literal_block_or_double_quotes_3587() {
        let value = |s: &str| go_yaml_multiline_scalar(s, "  ", 2);
        for (s, expected) in [
            ("x\ny", "|-\n  x\n  y"),
            ("p\n", "|\n  p"),
            ("\n", "|+\n"),
            ("a\n\nb\n\n", "|+\n  a\n\n  b\n"),
            ("\n\nc", "|-\n\n\n  c"),
            ("a\tb\nc", "|-\n  a\tb\n  c"),
            // A leading space asks for the indentation indicator.
            (" x\ny", "|2-\n   x\n  y"),
            ("x\n y", "|-\n  x\n   y"),
            // Refused by the emitter: a space before a break, a trailing space, a
            // character it will not write raw.
            ("x \ny", "\"x \\ny\""),
            ("x\ny ", "\"x\\ny \""),
            ("x\r\ny", "\"x\\r\\ny\""),
            ("x\u{85}\ny", "\"x\\N\\ny\""),
            ("\n\u{1F600}", "\"\\n\\U0001F600\""),
            ("a\nb\u{7F}", "\"a\\nb\\x7F\""),
            // Only U+2028/U+2029: single quotes, a break written raw and the next
            // character indented.
            ("a\u{2028}b", "'a\u{2028}  b'"),
            ("\u{2028}", "'\u{2028}'"),
            ("a\u{2028}", "'a\u{2028}'"),
            ("it's\u{2029}x", "'it''s\u{2029}  x'"),
            // ... unless a space touches the break, or a tab is present.
            (" \u{2028}", "\" \\L\""),
            ("\u{2028} ", "\"\\L \""),
            ("\u{2028}\t", "\"\\L\\t\""),
            ("a\rb", "\"a\\rb\""),
        ] {
            assert_eq!(value(s).as_deref(), Some(expected), "{s:?}");
        }
        assert_eq!(value("no break"), None);
    }

    /// #3587: the one deliberate divergence. go-yaml tests only the first character of
    /// the string for the indentation indicator, so `"\n a"` is written `|-` and reads
    /// back as `"\na"` (yq cannot read its own output); a literal block looks past the
    /// leading breaks instead, as `stream_yaml_block_scalar` does for a source block.
    #[test]
    fn a_literal_block_looks_past_leading_breaks_for_its_indicator_3587() {
        assert_eq!(
            go_yaml_multiline_scalar("\n a", "  ", 2).as_deref(),
            Some("|2-\n\n   a")
        );
        // Folded keeps go-yaml's own rule.
        assert_eq!(
            go_yaml_block_scalar("\n a", true, "  ", 2).as_deref(),
            Some(">-\n\n   a")
        );
    }

    /// #3587: `yaml_emitter_check_simple_key`: a key over 128 bytes or with a break is
    /// written `? key`, the scalar chosen as a value's is.
    #[test]
    fn explicit_keys_follow_the_emitters_simple_key_rule_3587() {
        let key = |s: &str| go_yaml_explicit_key_scalar(s, "  ", 2);
        assert_eq!(key("plain"), None);
        assert_eq!(key(&"k".repeat(128)), None);
        assert_eq!(key(&"k".repeat(129)), Some("k".repeat(129)));
        assert_eq!(
            key(&format!("{} x", "k".repeat(129))),
            Some(format!("{} x", "k".repeat(129)))
        );
        assert_eq!(
            key(&format!("- {}", "k".repeat(129))),
            Some(format!("'- {}'", "k".repeat(129)))
        );
        assert_eq!(key("a\nb").as_deref(), Some("|-\n  a\n  b"));
        assert_eq!(key("a\r").as_deref(), Some("\"a\\r\""));
        assert_eq!(key("a\u{2028}").as_deref(), Some("'a\u{2028}'"));
        // The long-key path takes the single-line quoting rule: a tab forces double quotes.
        assert_eq!(
            key(&format!("a\t{}", "k".repeat(129))).as_deref(),
            Some(&*format!("\"a\\t{}\"", "k".repeat(129)))
        );
    }

    /// #2796: the tag yaml.v3's `resolve` gives a plain scalar, which is what an assigned tag
    /// is compared with. Every row is captured from yq v4.53.3 (`tag = "<tag>"` printing
    /// nothing exactly when `<tag>` is the row's).
    #[test]
    fn plain_scalars_resolve_to_the_tag_go_yaml_compares_2796() {
        for (text, tag) in [
            ("", "!!null"),
            ("~", "!!null"),
            ("null", "!!null"),
            ("true", "!!bool"),
            ("False", "!!bool"),
            ("yes", "!!str"),
            ("1", "!!int"),
            ("0x1F", "!!int"),
            ("0o17", "!!int"),
            ("1_000", "!!int"),
            ("-5", "!!int"),
            ("1.5", "!!float"),
            ("1e3", "!!float"),
            (".5", "!!float"),
            (".inf", "!!float"),
            ("-.INF", "!!float"),
            (".nan", "!!float"),
            ("-0", "!!float"),
            ("2001-12-14", "!!timestamp"),
            ("<<", "!!merge"),
            ("x", "!!str"),
            ("1.2.3", "!!str"),
            ("0x", "!!str"),
        ] {
            assert_eq!(go_yaml_resolved_tag(text), tag, "{text:?}");
        }
    }

    /// #5467: a plain flow scalar is quoted by its characters alone, whatever type it reads as.
    #[test]
    fn flow_plain_scalar_quoting_follows_the_flow_indicators() {
        for s in [
            "x:y",
            "http://a",
            "12:30:45",
            "2001-12-14T21:59:43Z",
            "a?b",
            "?x",
            ":x",
            "x:",
            "a b:c d",
        ] {
            assert!(flow_plain_needs_single_quotes(s), "{s:?}");
        }
        for s in ["abc", "a b", "1", "true", "null", "1.5", "a#b", "-x", "a-b"] {
            assert!(!flow_plain_needs_single_quotes(s), "{s:?}");
        }
    }
}
