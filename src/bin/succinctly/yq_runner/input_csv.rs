//! yq's `-p csv` and `-p tsv` input formats (#6925): `decoder_csv_object.go` over Go's
//! `encoding/csv`, transcoded to the YAML text yq prints for the same nodes.
//!
//! The first record is the header and every later record is a mapping from the header's names to
//! its cells, so a document is a sequence of mappings: `[]` when only a header is present, `null`
//! when nothing is. Records come from [`read_records`], a port of `csv.Reader` with its defaults:
//! blank lines are skipped, a quote is only allowed in a quoted field, every record must have the
//! header's field count, and a `\r\n` is a `\n` even inside a quoted field.
//!
//! A cell is read as a snippet of YAML (yq's `parseSnippet`, the default `--csv-auto-parse`): a
//! snippet that is a string, or does not parse, stays the cell's own text, and any other node is
//! the node the snippet makes (`1` an integer, `[1,2]` a sequence, `a: b` a mapping, `&a 5` an
//! anchored integer). With `--csv-auto-parse=false` a cell is a scalar, typed as a plain scalar of
//! its text is. The text is then written the way yq writes the nodes and read as `-p yaml`, so
//! every route after the read is the YAML route's.

use succinctly::yaml::{
    go_yaml_dom_scalar, go_yaml_explicit_key_scalar, go_yaml_key_is_simple,
    go_yaml_multiline_scalar, go_yaml_resolved_tag, YamlIndex, YamlValue,
};

/// One level of indentation in the text written.
const STEP: &str = "  ";

/// `raw` as the YAML text of the document yq decodes it to, or yq's message for an input that does
/// not load (printed after `bad file '<name>': `).
///
/// `separator` is the field separator (`--csv-separator` for CSV, a tab for TSV) and `auto_parse`
/// is `--csv-auto-parse` / `--tsv-auto-parse`.
pub(super) fn transcode(raw: &[u8], separator: char, auto_parse: bool) -> Result<Vec<u8>, String> {
    let raw = raw.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(raw);
    let records = read_records(raw, separator)?;
    let mut records = records.into_iter();
    let Some(header) = records.next() else {
        return Ok(b"null\n".to_vec());
    };
    let mut out = String::new();
    let mut rows = 0usize;
    for record in records {
        rows += 1;
        write_row(&mut out, &header, &record, auto_parse);
    }
    if rows == 0 {
        return Ok(b"[]\n".to_vec());
    }
    Ok(out.into_bytes())
}

// ---------------------------------------------------------------------------------------------
// encoding/csv: reader.go
// ---------------------------------------------------------------------------------------------

/// A failure of Go's `csv.Reader`, which formats as its `ParseError` does.
fn parse_error(start_line: usize, line: usize, column: usize, message: &str) -> String {
    if start_line != line {
        format!(
            "record on line {start_line}; parse error on line {line}, column {column}: {message}"
        )
    } else {
        format!("parse error on line {line}, column {column}: {message}")
    }
}

const ERR_BARE_QUOTE: &str = "bare \" in non-quoted-field";
const ERR_QUOTE: &str = "extraneous or missing \" in quoted-field";

/// The lines of `raw` as `csv.Reader.readLine` hands them out: each ends with `\n` (a `\r\n` is
/// read as `\n`), except a last line with no line break, which also loses a trailing `\r`.
struct Lines<'a> {
    rest: &'a [u8],
    /// The number of lines handed out (`Reader.numLine`).
    number: usize,
}

impl Lines<'_> {
    fn next_line(&mut self) -> Option<Vec<u8>> {
        if self.rest.is_empty() {
            return None;
        }
        let (mut line, rest) = match self.rest.iter().position(|&b| b == b'\n') {
            Some(at) => (self.rest[..=at].to_vec(), &self.rest[at + 1..]),
            None => (self.rest.to_vec(), &self.rest[self.rest.len()..]),
        };
        self.rest = rest;
        self.number += 1;
        let n = line.len();
        if n >= 2 && line[n - 2] == b'\r' && line[n - 1] == b'\n' {
            line[n - 2] = b'\n';
            line.truncate(n - 1);
        } else if line.last() == Some(&b'\r') {
            // "For backwards compatibility, drop trailing \r before EOF."
            line.pop();
        }
        Some(line)
    }
}

fn length_nl(line: &[u8]) -> usize {
    usize::from(line.last() == Some(&b'\n'))
}

/// `csv.Reader.ReadAll` over `raw` with `Comma` set to `separator` and every other field of the
/// reader at its zero value.
fn read_records(raw: &[u8], separator: char) -> Result<Vec<Vec<Vec<u8>>>, String> {
    let mut comma = [0u8; 4];
    let comma: &[u8] = separator.encode_utf8(&mut comma).as_bytes();
    if matches!(separator, '"' | '\r' | '\n' | '\u{FFFD}') || separator == '\0' {
        return Err(String::from("csv: invalid field or comment delimiter"));
    }
    let mut lines = Lines {
        rest: raw,
        number: 0,
    };
    let mut records: Vec<Vec<Vec<u8>>> = Vec::new();
    let mut fields_per_record = 0usize;
    loop {
        // Skip empty lines.
        let mut line = loop {
            match lines.next_line() {
                None => return Ok(records),
                Some(line) if line.len() == length_nl(&line) => {}
                Some(line) => break line,
            }
        };
        let rec_line = lines.number;
        let (mut pos_line, mut pos_col) = (lines.number, 1usize);
        let mut record: Vec<Vec<u8>> = Vec::new();
        let mut field: Vec<u8> = Vec::new();
        // The unread part of `line` is `line[at..]`.
        let mut at = 0usize;
        let mut error: Option<String> = None;
        'fields: loop {
            if at >= line.len() || line[at] != b'"' {
                // A non-quoted field.
                let rest = &line[at..];
                let found = find(rest, comma);
                let text = match found {
                    Some(i) => &rest[..i],
                    None => &rest[..rest.len() - length_nl(rest)],
                };
                if let Some(j) = text.iter().position(|&b| b == b'"') {
                    error = Some(parse_error(
                        rec_line,
                        lines.number,
                        pos_col + j,
                        ERR_BARE_QUOTE,
                    ));
                    break 'fields;
                }
                field.extend_from_slice(text);
                record.push(std::mem::take(&mut field));
                match found {
                    Some(i) => {
                        at += i + comma.len();
                        pos_col += i + comma.len();
                        continue 'fields;
                    }
                    None => break 'fields,
                }
            }
            // A quoted field.
            at += 1;
            pos_col += 1;
            loop {
                let rest = &line[at..];
                if let Some(i) = rest.iter().position(|&b| b == b'"') {
                    // Hit the next quote.
                    field.extend_from_slice(&rest[..i]);
                    at += i + 1;
                    pos_col += i + 1;
                    let after = &line[at..];
                    if after.first() == Some(&b'"') {
                        // `""`: a quote.
                        field.push(b'"');
                        at += 1;
                        pos_col += 1;
                    } else if after.starts_with(comma) {
                        // `",`: the end of the field.
                        at += comma.len();
                        pos_col += comma.len();
                        record.push(std::mem::take(&mut field));
                        continue 'fields;
                    } else if length_nl(after) == after.len() {
                        // `"\n`: the end of the line.
                        record.push(std::mem::take(&mut field));
                        break 'fields;
                    } else {
                        // `"*`: a quote that is not escaped.
                        error = Some(parse_error(rec_line, lines.number, pos_col - 1, ERR_QUOTE));
                        break 'fields;
                    }
                } else if !rest.is_empty() {
                    // The end of the line: keep all of it and read the next.
                    field.extend_from_slice(rest);
                    pos_col += rest.len();
                    match lines.next_line() {
                        Some(next) => {
                            // A line that was only a `\r` at the end of the input is empty, and
                            // go does not move its position onto it.
                            if !next.is_empty() {
                                pos_line += 1;
                                pos_col = 1;
                            }
                            line = next;
                            at = 0;
                        }
                        None => {
                            line = Vec::new();
                            at = 0;
                        }
                    }
                } else {
                    // The end of the input in a quoted field.
                    error = Some(parse_error(rec_line, pos_line, pos_col, ERR_QUOTE));
                    break 'fields;
                }
            }
        }
        if let Some(message) = error {
            return Err(message);
        }
        if fields_per_record == 0 {
            fields_per_record = record.len();
        } else if record.len() != fields_per_record {
            return Err(format!("record on line {rec_line}: wrong number of fields"));
        }
        records.push(record);
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

// ---------------------------------------------------------------------------------------------
// The cells: decoder_csv_object.go and parseSnippet
// ---------------------------------------------------------------------------------------------

/// What a cell is once yq has read it.
#[derive(Debug, PartialEq)]
enum Cell {
    /// A `!!null` with no text.
    Empty,
    /// A string node with no style of its own.
    Str(String),
    /// A scalar of another type, written as its own text (`5`, `0x1f`, `&a 5`, `true`).
    Scalar(String),
    /// A collection, as the YAML text of the snippet.
    Collection(String),
}

fn classify(text: &str, auto_parse: bool) -> Cell {
    if text.is_empty() {
        return Cell::Empty;
    }
    if !auto_parse {
        return if go_yaml_resolved_tag(text) == "!!str" {
            Cell::Str(text.to_string())
        } else {
            Cell::Scalar(text.to_string())
        };
    }
    // A tag with nothing to tag: a bare `!` is a null, any other (`!!`, `!!str`, `!x`) a string.
    // go-yaml reads a lone `\r` as a line break, as the snippet is read here; a string keeps it.
    let norm = text.replace('\r', "\n");
    let bare = norm.trim();
    if bare.starts_with('!') && !bare.contains(char::is_whitespace) {
        return if bare == "!" {
            Cell::Scalar(bare.to_string())
        } else {
            Cell::Str(text.to_string())
        };
    }
    let Ok(index) = YamlIndex::build_yq(norm.as_bytes()) else {
        return Cell::Str(text.to_string());
    };
    // The documents sit in a virtual root sequence; a snippet is one of them, and one that holds
    // several is a string.
    let YamlValue::Sequence(documents) = index.root(norm.as_bytes()).value() else {
        return Cell::Str(text.to_string());
    };
    let Some((root, rest)) = documents.uncons_cursor() else {
        // Nothing but comments, or nothing at all: a null, which keeps the text of the comment.
        return Cell::Scalar(norm.trim().to_string());
    };
    if norm.trim().is_empty() {
        // Blanks only: a null whose comment is the blanks, which yq writes behind its `# `.
        return Cell::Scalar(format!("# {norm}"));
    }
    if rest.uncons_cursor().is_some() {
        return Cell::Str(text.to_string());
    }
    match root.value() {
        YamlValue::Mapping(_) | YamlValue::Sequence(_) => Cell::Collection(norm),
        YamlValue::Null => Cell::Scalar(norm.trim().to_string()),
        YamlValue::String(s) => {
            let explicit = root.explicit_tag();
            let unquoted = s.is_unquoted();
            let value = s.as_str().ok();
            let is_str = match (explicit, &value) {
                (Some(tag), _) => tag == "!!str",
                // A plain scalar is typed as go-yaml's `resolve` types it (a date is a timestamp
                // and `1_000` an integer); a quoted one is a string.
                (None, Some(value)) if unquoted => go_yaml_resolved_tag(value.trim()) == "!!str",
                (None, _) => true,
            };
            if is_str {
                return Cell::Str(text.to_string());
            }
            let mut scalar = String::new();
            if let Some(anchor) = root.anchor() {
                scalar.push('&');
                scalar.push_str(anchor);
                scalar.push(' ');
            }
            if let Some(tag) = explicit {
                scalar.push_str(tag);
                scalar.push(' ');
            }
            match value {
                Some(value) if unquoted => scalar.push_str(value.trim()),
                _ => scalar.push_str(&String::from_utf8_lossy(s.raw_bytes())),
            }
            Cell::Scalar(scalar)
        }
        YamlValue::Alias { .. } | YamlValue::Error(_) => Cell::Str(text.to_string()),
    }
}

// ---------------------------------------------------------------------------------------------
// The YAML text
// ---------------------------------------------------------------------------------------------

/// `s` as go-yaml writes a string with no style of its own; `block_indent` is the indent of a
/// literal block's lines. A header's name is a node of its own and is written like a value: `1`
/// is `"1"`, as it is in yq's output.
fn string_scalar(s: &str, block_indent: &str) -> String {
    go_yaml_dom_scalar(s, false, false, false)
        .or_else(|| go_yaml_multiline_scalar(s, block_indent, STEP.len()))
        .unwrap_or_else(|| succinctly::yaml::go_yaml_double_quoted_scalar(s))
}

/// `text` with every line after the first indented by `indent`.
fn hang(text: &str, indent: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (n, line) in text.split('\n').enumerate() {
        if n > 0 {
            out.push('\n');
            if !line.is_empty() {
                out.push_str(indent);
            }
        }
        out.push_str(line);
    }
    out
}

/// One record as an item of the document's sequence.
fn write_row(out: &mut String, header: &[Vec<u8>], record: &[Vec<u8>], auto_parse: bool) {
    let child = STEP.repeat(2);
    for (n, (name, cell)) in header.iter().zip(record).enumerate() {
        out.push_str(if n == 0 { "- " } else { STEP });
        let name = String::from_utf8_lossy(name);
        let cell = String::from_utf8_lossy(cell);
        if go_yaml_key_is_simple(&name) {
            out.push_str(&string_scalar(&name, &child));
            out.push(':');
        } else {
            // go-yaml's `? ` explicit key, for a key with a line break or over 128 bytes.
            out.push_str("? ");
            out.push_str(
                &go_yaml_explicit_key_scalar(&name, &child, STEP.len())
                    .unwrap_or_else(|| string_scalar(&name, &child)),
            );
            out.push('\n');
            out.push_str(STEP);
            out.push(':');
        }
        match classify(&cell, auto_parse) {
            Cell::Empty => {}
            Cell::Str(s) => {
                out.push(' ');
                out.push_str(&string_scalar(&s, &child));
            }
            Cell::Scalar(s) => {
                out.push(' ');
                out.push_str(&hang(&s, &child));
            }
            Cell::Collection(text) => {
                let text = text.trim_end_matches('\n');
                if text.trim_start().starts_with(['[', '{']) {
                    out.push(' ');
                    out.push_str(&hang(text.trim_start(), &child));
                } else {
                    out.push('\n');
                    out.push_str(&child);
                    out.push_str(&hang(text.trim_start_matches('\n'), &child));
                }
            }
        }
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yaml(input: &str) -> String {
        String::from_utf8(transcode(input.as_bytes(), ',', true).unwrap()).unwrap()
    }

    fn err(input: &str) -> String {
        transcode(input.as_bytes(), ',', true).unwrap_err()
    }

    #[test]
    fn records_become_a_sequence_of_mappings() {
        assert_eq!(
            yaml("name,age\nAlice,30\nBob,0x1f\n"),
            "- name: Alice\n  age: 30\n- name: Bob\n  age: 0x1f\n"
        );
    }

    #[test]
    fn a_header_alone_is_an_empty_sequence_and_nothing_is_null() {
        assert_eq!(yaml("a,b\n"), "[]\n");
        assert_eq!(yaml(""), "null\n");
        assert_eq!(yaml("\n\n"), "null\n");
    }

    #[test]
    fn the_reader_skips_blank_lines_and_reads_crlf_as_lf() {
        assert_eq!(yaml("a\n\n\n1\n\n2"), "- a: 1\n- a: 2\n");
        assert_eq!(
            yaml("a,b\r\n\"x\r\ny\",2\r\n"),
            "- a: |-\n    x\n    y\n  b: 2\n"
        );
    }

    #[test]
    fn the_reader_names_its_errors_as_go_does() {
        assert_eq!(err("a,b\n1\n"), "record on line 2: wrong number of fields");
        assert_eq!(
            err("a,b\n1,\"x\n"),
            "parse error on line 2, column 6: extraneous or missing \" in quoted-field"
        );
        assert_eq!(
            err("a,b\n1,x\"y\n"),
            "parse error on line 2, column 4: bare \" in non-quoted-field"
        );
        assert_eq!(
            err("a,b\n\"1\"x,2\n"),
            "parse error on line 2, column 3: extraneous or missing \" in quoted-field"
        );
    }

    #[test]
    fn a_cell_that_is_not_a_string_is_its_own_node() {
        let cell = |text: &str| classify(text, true);
        assert_eq!(cell("5 # c"), Cell::Scalar("5".into()));
        assert_eq!(cell(" 1"), Cell::Scalar("1".into()));
        assert_eq!(cell("0x1f"), Cell::Scalar("0x1f".into()));
        assert_eq!(cell("true"), Cell::Scalar("true".into()));
        assert_eq!(cell("&a 5"), Cell::Scalar("&a 5".into()));
        assert_eq!(cell("[1,2]"), Cell::Collection("[1,2]".into()));
        assert_eq!(cell("a: b"), Cell::Collection("a: b".into()));
        assert_eq!(cell(""), Cell::Empty);
    }

    #[test]
    fn a_cell_that_is_a_string_keeps_its_text() {
        let cell = |text: &str| classify(text, true);
        for text in [
            "'x'", "\"x\"", "!!str 5", "a # c", "yes", " x ", "*a", "a: b: c",
        ] {
            assert_eq!(cell(text), Cell::Str(text.into()), "{text}");
        }
    }

    #[test]
    fn without_auto_parse_a_cell_is_a_plain_scalar() {
        assert_eq!(classify("1", false), Cell::Scalar("1".into()));
        assert_eq!(classify("[1]", false), Cell::Str("[1]".into()));
        assert_eq!(classify("yes", false), Cell::Str("yes".into()));
    }
}
