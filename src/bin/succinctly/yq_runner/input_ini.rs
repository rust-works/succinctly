//! yq's `-p ini` input format (#6925): `decoder_ini.go` over go-ini v1.67.0 (`ini.LoadSources`
//! with its default options and, under `--ini-preserve-quotes`, `PreserveSurroundedQuote`),
//! transcoded to the YAML text yq prints for the same nodes.
//!
//! go-ini's reader is ported from `parser.go`: a line is a comment (`#`, `;`), a section
//! (`[name]`, up to the last `]` of the line) or a key (`name = value`, `name : value`, the key
//! optionally in quotes), a value is a run of text with its inline comment cut at the first `#` or
//! `;`, a surrounding quote pair removed, a trailing `\` continuing it on the next line, and
//! `"""` or a backtick opening a value that runs over lines. A key written twice keeps its first
//! position and its last value; a section written twice is one section; the keys before the first
//! section are the `DEFAULT` section's. `Key.String()` then expands `%(name)s` from the key's
//! section, its parent sections (`a.b` has `a`) and `DEFAULT`.
//!
//! yq makes a mapping of the `DEFAULT` section's keys followed by one mapping per other section,
//! every key and value a string. The text is written as yq's YAML encoder writes it and the run
//! reads it as `-p yaml`.

use std::collections::HashMap;

use succinctly::jq::go_quote_bytes;
use succinctly::yaml::{go_yaml_explicit_key_scalar, go_yaml_key_is_simple};

use super::input_csv::string_scalar;

/// go-ini's `DefaultSection`.
const DEFAULT_SECTION: &str = "DEFAULT";

/// One level of indentation in the text written.
const STEP: &str = "  ";

/// The most bytes one value may grow to while its `%(name)s` references are expanded: 16 MiB.
const MAX_EXPANSION_BYTES: usize = 1 << 24;

/// go-ini's `depthValues`: how many `%(name)s` references one value expands.
const DEPTH_VALUES: usize = 99;

/// `raw` as the YAML text of the document yq decodes it to, or yq's message for an input that does
/// not load (printed after `bad file '<name>': `).
pub(super) fn transcode(raw: &[u8], preserve_quotes: bool) -> Result<Vec<u8>, String> {
    let raw = strip_bom(raw);
    let text = String::from_utf8_lossy(raw);
    let file =
        parse(&text, preserve_quotes).map_err(|e| format!("failed to parse INI content: {e}"))?;
    Ok(render(&file)?.into_bytes())
}

/// `Parser.BOM`: a UTF-16 byte-order mark or a UTF-8 one at the start is dropped.
fn strip_bom(raw: &[u8]) -> &[u8] {
    match raw {
        [0xFE, 0xFF, rest @ ..] | [0xFF, 0xFE, rest @ ..] | [0xEF, 0xBB, 0xBF, rest @ ..] => rest,
        _ => raw,
    }
}

// ---------------------------------------------------------------------------------------------
// ini.go: File, Section, Key
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Default)]
struct Section {
    name: String,
    /// The keys in the order they first appeared, with their last value.
    keys: Vec<(String, String)>,
    /// Where each key sits in `keys`.
    index: HashMap<String, usize>,
}

impl Section {
    fn position(&self, name: &str) -> Option<usize> {
        self.index.get(name).copied()
    }
}

#[derive(Debug, Default)]
struct File {
    /// The sections in the order they first appeared; the first is `DEFAULT`.
    sections: Vec<Section>,
    /// Where each section sits in `sections`.
    index: HashMap<String, usize>,
}

impl File {
    fn section_index(&self, name: &str) -> Option<usize> {
        let name = if name.is_empty() {
            DEFAULT_SECTION
        } else {
            name
        };
        self.sections
            .iter()
            .position(|section| section.name == name)
    }

    /// `File.NewSection`: the section of that name, made if there is none.
    fn new_section(&mut self, name: &str) -> Result<usize, String> {
        if name.is_empty() {
            return Err(String::from("empty section name"));
        }
        if let Some(at) = self.section_index(name) {
            return Ok(at);
        }
        self.sections.push(Section {
            name: name.to_string(),
            ..Section::default()
        });
        self.index.insert(name.to_string(), self.sections.len() - 1);
        Ok(self.sections.len() - 1)
    }

    /// `Section.NewKey`: a key written again keeps its place and takes the new value.
    fn new_key(&mut self, section: usize, name: &str, value: String) -> Result<(), String> {
        if name.is_empty() {
            return Err(String::from("error creating new key: empty key name"));
        }
        let section = &mut self.sections[section];
        match section.position(name) {
            Some(at) => section.keys[at].1 = value,
            None => {
                section.index.insert(name.to_string(), section.keys.len());
                section.keys.push((name.to_string(), value));
            }
        }
        Ok(())
    }

    /// `Section.GetKey`: the key in the section, or else in the section named by the part of its
    /// name before the last `.`, and so on up.
    fn get_key(&self, section: usize, name: &str) -> Option<(usize, usize)> {
        if let Some(at) = self.sections[section].position(name) {
            return Some((section, at));
        }
        let mut name_of_section = self.sections[section].name.as_str();
        while let Some(dot) = name_of_section.rfind('.') {
            name_of_section = &name_of_section[..dot];
            let Some(parent) = self.section_index(name_of_section) else {
                continue;
            };
            return self.get_key(parent, name);
        }
        None
    }

    /// `Key.String()`: the value with its `%(name)s` references expanded. A value that grows past
    /// [`MAX_EXPANSION_BYTES`] is refused: go-ini expands a chain of values that each repeat the
    /// next twice until it runs out of memory.
    fn value_of(&self, section: usize, key: usize) -> Result<String, String> {
        let raw = &self.sections[section].keys[key].1;
        if !raw.contains('%') {
            return Ok(raw.clone());
        }
        let mut value = raw.clone();
        for _ in 0..DEPTH_VALUES {
            let Some((start, end)) = find_reference(&value) else {
                break;
            };
            let reference = value[start..end].to_string();
            let option = &reference[2..reference.len() - 2];
            let mut found = self.get_key(section, option);
            if found.is_none() || found == Some((section, key)) {
                found = self.get_key(0, option);
            }
            let Some((found_section, found_key)) = found else {
                break;
            };
            let replacement = &self.sections[found_section].keys[found_key].1;
            value = value.replace(&reference, replacement);
            if value.len() > MAX_EXPANSION_BYTES {
                return Err(format!(
                    "the value of key {} expands past {MAX_EXPANSION_BYTES} bytes",
                    go_quote(&self.sections[section].keys[key].0)
                ));
            }
        }
        Ok(value)
    }
}

/// The leftmost match of `%\(([^)]+)\)s` in `value`, as a byte range.
fn find_reference(value: &str) -> Option<(usize, usize)> {
    let bytes = value.as_bytes();
    let mut from = 0;
    while let Some(offset) = value[from..].find("%(") {
        let start = from + offset;
        let name = start + 2;
        let close = bytes[name..].iter().position(|&b| b == b')')?;
        if close > 0 && bytes.get(name + close + 1) == Some(&b's') {
            return Some((start, name + close + 2));
        }
        from = start + 1;
    }
    None
}

// ---------------------------------------------------------------------------------------------
// parser.go
// ---------------------------------------------------------------------------------------------

struct Parser<'a> {
    text: &'a str,
    pos: usize,
    is_eof: bool,
    preserve_quotes: bool,
}

impl<'a> Parser<'a> {
    /// `readUntil('\n')`: the text through the next line break, or what is left of the input,
    /// which sets `isEOF`.
    fn read_line(&mut self) -> &'a str {
        let rest = &self.text[self.pos..];
        match rest.find('\n') {
            Some(at) => {
                self.pos += at + 1;
                &rest[..=at]
            }
            None => {
                self.pos = self.text.len();
                self.is_eof = true;
                rest
            }
        }
    }

    /// `readMultilines`: a value opened by `quote` that runs over lines.
    fn read_multilines(&mut self, line: &str, first: &str, quote: &str) -> Result<String, String> {
        let mut value = first.to_string();
        loop {
            let next = self.read_line();
            if let Some(at) = next.rfind(quote) {
                value.push_str(&next[..at]);
                return Ok(value);
            }
            value.push_str(next);
            if self.is_eof {
                return Err(format!(
                    "missing closing key quote from {} to {}",
                    go_quote(line),
                    go_quote(next)
                ));
            }
        }
    }

    /// `readContinuationLines`: a value whose line ended in `\`.
    fn read_continuation_lines(&mut self, first: &str) -> String {
        let mut value = first.to_string();
        loop {
            let next = self.read_line().trim();
            if next.is_empty() {
                break;
            }
            value.push_str(next);
            if !value.ends_with('\\') {
                break;
            }
            value.pop();
        }
        value
    }

    /// `readValue`: the value of the text after a key's delimiter.
    fn read_value(&mut self, input: &str) -> Result<String, String> {
        let line = input.trim_start_matches(char::is_whitespace);
        if line.is_empty() {
            return Ok(String::new());
        }
        let quote = if line.len() > 3 && line.starts_with("\"\"\"") {
            "\"\"\""
        } else if line.starts_with('`') {
            "`"
        } else {
            ""
        };
        if !quote.is_empty() {
            let start = quote.len();
            return match line[start..].rfind(quote) {
                None => self.read_multilines(line, &line[start..], quote),
                Some(at) => Ok(line[start..at + start].to_string()),
            };
        }
        let line = line.trim();
        if let Some(before) = line.strip_suffix('\\') {
            return Ok(self.read_continuation_lines(before));
        }
        let mut line = line;
        if let Some(at) = line.find(['#', ';']) {
            line = line[..at].trim();
        }
        if !self.preserve_quotes
            && (has_surrounded_quote(line, b'\'') || has_surrounded_quote(line, b'"'))
        {
            line = &line[1..line.len() - 1];
        }
        Ok(line.to_string())
    }
}

/// `s` as Go's `%q` writes it.
fn go_quote(s: &str) -> String {
    go_quote_bytes(s.as_bytes())
}

/// `hasSurroundedQuote`: `in` starts and ends with `quote` and holds no other.
fn has_surrounded_quote(text: &str, quote: u8) -> bool {
    let bytes = text.as_bytes();
    bytes.len() >= 2
        && bytes[0] == quote
        && bytes[bytes.len() - 1] == quote
        && bytes[1..].iter().position(|&b| b == quote) == Some(bytes.len() - 2)
}

/// `readKeyName`: the key's name and the offset of its value, with `=` and `:` for delimiters.
fn read_key_name(line: &str) -> Result<(String, usize), String> {
    let delimiters = ['=', ':'];
    let quote = if line.starts_with('"') {
        if line.len() > 6 && line.starts_with("\"\"\"") {
            "\"\"\""
        } else {
            "\""
        }
    } else if line.starts_with('`') {
        "`"
    } else {
        ""
    };
    if !quote.is_empty() {
        let start = quote.len();
        let Some(found) = line[start..].find(quote) else {
            return Err(format!("missing closing key quote: {line}"));
        };
        let pos = found + start;
        let Some(i) = line[pos + start..].find(delimiters) else {
            return Err(format!("key-value delimiter not found: {line}"));
        };
        let end = pos + i;
        return Ok((line[start..pos].trim().to_string(), end + start + 1));
    }
    let Some(end) = line.find(delimiters) else {
        return Err(format!("key-value delimiter not found: {line}"));
    };
    if end == 0 {
        return Err(format!("empty key name: {line}"));
    }
    Ok((line[..end].trim().to_string(), end + 1))
}

fn parse(text: &str, preserve_quotes: bool) -> Result<File, String> {
    let mut parser = Parser {
        text,
        pos: 0,
        is_eof: false,
        preserve_quotes,
    };
    let mut file = File::default();
    let mut section = file.new_section(DEFAULT_SECTION)?;
    let mut count = 1usize;
    while !parser.is_eof {
        let line = parser.read_line();
        let line = line.trim_start_matches(char::is_whitespace);
        if line.is_empty() || line.starts_with(['#', ';']) {
            continue;
        }
        if line.starts_with('[') {
            let Some(close) = line.rfind(']') else {
                return Err(format!("unclosed section: {line}"));
            };
            section = file.new_section(&line[1..close])?;
            count = 1;
            continue;
        }
        let (mut name, offset) = read_key_name(line)?;
        if name == "-" {
            name = format!("#{count}");
            count += 1;
        }
        let value = parser.read_value(&line[offset..])?;
        file.new_key(section, &name, value)?;
    }
    Ok(file)
}

// ---------------------------------------------------------------------------------------------
// The YAML text
// ---------------------------------------------------------------------------------------------

/// `name` and `:` at `depth`, the `? ` explicit-key form for a key with a line break or over
/// 128 bytes.
fn key_line(out: &mut String, name: &str, depth: usize) {
    let indent = STEP.repeat(depth);
    let child = STEP.repeat(depth + 1);
    out.push_str(&indent);
    if go_yaml_key_is_simple(name) {
        out.push_str(&string_scalar(name, &child));
        out.push(':');
    } else {
        out.push_str("? ");
        out.push_str(
            &go_yaml_explicit_key_scalar(name, &child, STEP.len())
                .unwrap_or_else(|| string_scalar(name, &child)),
        );
        out.push('\n');
        out.push_str(&indent);
        out.push(':');
    }
}

fn render_entries(
    out: &mut String,
    file: &File,
    section: usize,
    depth: usize,
) -> Result<(), String> {
    let child = STEP.repeat(depth + 1);
    for (key, (name, _)) in file.sections[section].keys.iter().enumerate() {
        key_line(out, name, depth);
        out.push(' ');
        out.push_str(&string_scalar(&file.value_of(section, key)?, &child));
        out.push('\n');
    }
    Ok(())
}

fn render(file: &File) -> Result<String, String> {
    let mut out = String::new();
    render_entries(&mut out, file, 0, 0)?;
    for (at, section) in file.sections.iter().enumerate().skip(1) {
        key_line(&mut out, &section.name, 0);
        if section.keys.is_empty() {
            out.push_str(" {}\n");
        } else {
            out.push('\n');
            render_entries(&mut out, file, at, 1)?;
        }
    }
    if out.is_empty() {
        out.push_str("{}\n");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yaml(input: &str) -> String {
        String::from_utf8(transcode(input.as_bytes(), false).unwrap()).unwrap()
    }

    fn err(input: &str) -> String {
        transcode(input.as_bytes(), false).unwrap_err()
    }

    #[test]
    fn sections_are_mappings_and_the_keys_before_the_first_are_the_roots() {
        assert_eq!(
            yaml("top=1\n[s]\na=b\n[t]\n"),
            "top: \"1\"\ns:\n  a: b\nt: {}\n"
        );
        assert_eq!(yaml(""), "{}\n");
    }

    #[test]
    fn comments_quotes_and_continuations_are_go_inis() {
        assert_eq!(yaml("# c\n; c\na = b # x\nc: d\n"), "a: b\nc: d\n");
        assert_eq!(
            yaml("a = \"q w\"\nb = `raw`\nc = 's'\n"),
            "a: q w\nb: raw\nc: s\n"
        );
        assert_eq!(yaml("a = 1\\\n  2\n"), "a: \"12\"\n");
        assert_eq!(yaml("a=b=c\n"), "a: b=c\n");
    }

    #[test]
    fn a_key_or_section_written_twice_is_one() {
        assert_eq!(
            yaml("[s]\nk=1\nk=2\n[s]\nj=3\n"),
            "s:\n  k: \"2\"\n  j: \"3\"\n"
        );
    }

    #[test]
    fn references_expand_from_the_section_its_parents_and_default() {
        assert_eq!(
            yaml("d=D\n[a]\nx=1\n[a.b]\ny=%(x)s-%(d)s-%(zz)s\n"),
            "d: D\na:\n  x: \"1\"\na.b:\n  y: 1-D-%(zz)s\n"
        );
    }

    #[test]
    fn the_readers_errors_are_go_inis() {
        assert_eq!(
            err("k\n"),
            "failed to parse INI content: key-value delimiter not found: k\n"
        );
        assert_eq!(
            err("[s\n"),
            "failed to parse INI content: unclosed section: [s\n"
        );
        assert_eq!(
            err("=v\n"),
            "failed to parse INI content: empty key name: =v\n"
        );
        assert_eq!(
            err("[]\n"),
            "failed to parse INI content: empty section name"
        );
        assert_eq!(
            err("a=\"\"\"\nx"),
            "failed to parse INI content: missing closing key quote from \"\\\"\\\"\\\"\\n\" to \"x\""
        );
    }

    #[test]
    fn an_expansion_that_doubles_per_pass_is_refused() {
        let message = err("a=%(b)s%(b)s\nb=%(a)s%(a)s\n[s]\nc=%(a)s\n");
        assert!(message.contains("expands past 16777216 bytes"), "{message}");
    }
}
