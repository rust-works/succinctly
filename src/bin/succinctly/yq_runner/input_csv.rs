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
    let keys: Vec<String> = header.iter().map(|name| key_text(name)).collect();
    let mut out = String::new();
    let mut rows = 0usize;
    for record in records {
        rows += 1;
        write_row(&mut out, &keys, &record, auto_parse);
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

/// `s` without the blanks YAML trims from a plain scalar: spaces and tabs only (`str::trim` also
/// strips U+00A0 and the other Unicode white space, which go-yaml keeps).
fn yaml_trim(s: &str) -> &str {
    s.trim_matches([' ', '\t'])
}

/// Whether `text` is a plain scalar the YAML parser need not see: ASCII letters, digits, `_` and
/// single spaces between words, so no indicator, comment, tag, anchor or line break can be in it.
fn is_simple_plain(text: &str) -> bool {
    text.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b' ')
        && !text.starts_with(' ')
        && !text.ends_with(' ')
}

/// The text of the first YAML document of `text` (#7136): yq's `parseSnippet` decodes one document and
/// never reads what follows it, so a `---` or `...` line after the document's content ends the cell.
/// A marker before any content opens the document and is kept.
fn first_document(text: &str) -> &str {
    let mut seen_content = false;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let bare = line.trim_end_matches(['\n', ' ', '\t']);
        let is_marker = |m: &str| {
            bare == m
                || (bare.len() > m.len()
                    && bare.starts_with(m)
                    && matches!(bare.as_bytes()[m.len()], b' ' | b'\t'))
        };
        if is_marker("---") || is_marker("...") {
            if seen_content {
                return &text[..offset];
            }
            // `--- text` puts content on the marker line.
            let after = bare[3..].trim_start_matches([' ', '\t']);
            if bare.starts_with("---") && !after.is_empty() && !after.starts_with('#') {
                seen_content = true;
            }
        } else if !bare.trim_start().is_empty()
            && !bare.trim_start().starts_with('#')
            && !bare.starts_with('%')
        {
            seen_content = true;
        }
        offset += line.len();
    }
    text
}

/// Whether `text` opens with a flow collection (#7136), after comment lines and node properties
/// (`&anchor`, `!tag`).
fn opens_with_flow_collection(text: &str) -> bool {
    let mut rest = text;
    loop {
        rest = rest.trim_start_matches([' ', '\t', '\n']);
        if rest.starts_with('#') {
            rest = rest.find('\n').map_or("", |at| &rest[at..]);
        } else if rest.starts_with(['&', '!']) {
            rest = rest.find([' ', '\t', '\n']).map_or("", |at| &rest[at..]);
        } else {
            return rest.starts_with(['[', '{']);
        }
    }
}

/// Whether go-yaml's scanner can read what follows a complete first node (#7136). The parser peeks at
/// the token after the node to see whether the document ended, and the scanner is a token ahead of
/// it (the node's own candidate simple key is only settled once the next token is read), so the first
/// two tokens after the node are scanned: a flow collection counts as one, with everything inside it.
/// A token that fails to scan is yq's error, and the cell stays its text. A plain scalar, a bracket,
/// a comma, a comment and a document marker scan; a `-` or `?` indicator that does not start a line,
/// an unterminated quote, `@`, a backtick, `%`, a block scalar header followed by text, and an anchor
/// or alias name followed by a character that cannot end it do not.
fn token_after_the_first_node_scans(rest: &str, starts_a_line: bool) -> bool {
    let bytes = rest.as_bytes();
    let mut at = 0;
    // After a flow collection the text starts on the node's own line; after a document it starts a line.
    let mut line_start = starts_a_line;
    for _ in 0..2 {
        loop {
            match bytes.get(at) {
                Some(b' ' | b'\t' | b'\r') => at += 1,
                Some(b'\n') => {
                    line_start = true;
                    at += 1;
                }
                Some(b'#') => {
                    while bytes.get(at).is_some_and(|&b| b != b'\n') {
                        at += 1;
                    }
                }
                _ => break,
            }
        }
        let Some(&c) = bytes.get(at) else {
            return true;
        };
        let at_marker = line_start
            && (bytes[at..].starts_with(b"---") || bytes[at..].starts_with(b"..."))
            && blank_or_end(bytes.get(at + 3));
        let token_start_is_line_start = line_start;
        line_start = false;
        if at_marker {
            at += 3;
            continue;
        }
        match c {
            b'-' | b'?' if blank_or_end(bytes.get(at + 1)) => {
                if !token_start_is_line_start {
                    return false;
                }
                at += 1;
            }
            b'@' | b'`' => return false,
            b'%' => return false,
            b'\'' | b'"' => match skip_quoted(bytes, at) {
                Some(end) => at = end,
                None => return false,
            },
            b'[' | b'{' => match skip_flow_collection(bytes, at) {
                Some(end) => at = end,
                None => return false,
            },
            b']' | b'}' | b',' | b':' => at += 1,
            b'|' | b'>' => {
                at += 1;
                while bytes
                    .get(at)
                    .is_some_and(|b| b.is_ascii_digit() || matches!(b, b'+' | b'-'))
                {
                    at += 1;
                }
                while bytes
                    .get(at)
                    .is_some_and(|b| matches!(b, b' ' | b'\t' | b'\r'))
                {
                    at += 1;
                }
                // The block's own lines follow: nothing more to scan.
                return bytes.get(at).map_or(true, |b| matches!(b, b'\n' | b'#'));
            }
            b'&' | b'*' => {
                let start = at + 1;
                at = start;
                while bytes
                    .get(at)
                    .is_some_and(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
                {
                    at += 1;
                }
                if at == start
                    || !(blank_or_end(bytes.get(at))
                        || matches!(
                            bytes[at],
                            b'?' | b':' | b',' | b']' | b'}' | b'%' | b'@' | b'`'
                        ))
                {
                    return false;
                }
            }
            b'!' => {
                while bytes.get(at).is_some_and(|b| {
                    !matches!(
                        b,
                        b' ' | b'\t' | b'\r' | b'\n' | b',' | b'[' | b']' | b'{' | b'}'
                    )
                }) {
                    at += 1;
                }
            }
            _ => at = skip_plain_scalar(bytes, at),
        }
    }
    true
}

fn blank_or_end(byte: Option<&u8>) -> bool {
    byte.map_or(true, |b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
}

/// The end of the quoted scalar that opens at `start`, which may run over lines; `None` if it never
/// closes.
fn skip_quoted(bytes: &[u8], start: usize) -> Option<usize> {
    let quote = bytes[start];
    let mut at = start + 1;
    loop {
        match bytes.get(at)? {
            b'\\' if quote == b'"' => at += 2,
            b'\'' if quote == b'\'' && bytes.get(at + 1) == Some(&b'\'') => at += 2,
            &b if b == quote => return Some(at + 1),
            _ => at += 1,
        }
    }
}

/// The end of the flow collection that opens at `start`, scanning what is inside it; `None` for a
/// scanner error inside (an unterminated quote). One that never closes ends at the text's end.
fn skip_flow_collection(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut at = start;
    while at < bytes.len() {
        match bytes[at] {
            b'[' | b'{' => {
                depth += 1;
                at += 1;
            }
            b']' | b'}' => {
                depth -= 1;
                at += 1;
                if depth == 0 {
                    return Some(at);
                }
            }
            b'\'' | b'"' => at = skip_quoted(bytes, at)?,
            b'@' | b'`' | b'%' => return None,
            b'#' if at > 0 && matches!(bytes[at - 1], b' ' | b'\t' | b'\n') => {
                while bytes.get(at).is_some_and(|&b| b != b'\n') {
                    at += 1;
                }
            }
            _ => at += 1,
        }
    }
    Some(bytes.len())
}

/// The end of the plain scalar at `start`: up to ` #`, a `: ` or `:` at the end of a line, or a line
/// that opens with a document marker; a later line continues it.
fn skip_plain_scalar(bytes: &[u8], start: usize) -> usize {
    let mut at = start;
    while at < bytes.len() {
        match bytes[at] {
            b'#' if at > start && matches!(bytes[at - 1], b' ' | b'\t') => return at,
            b':' if blank_or_end(bytes.get(at + 1)) => return at,
            b'\n' => {
                let next = &bytes[at + 1..];
                let marker = (next.starts_with(b"---") || next.starts_with(b"..."))
                    && blank_or_end(next.get(3));
                let rest_blank = next
                    .iter()
                    .take_while(|&&b| b != b'\n')
                    .all(|b| matches!(b, b' ' | b'\t' | b'\r'));
                if marker || rest_blank || next.first() == Some(&b'#') {
                    return at;
                }
                at += 1;
            }
            _ => at += 1,
        }
    }
    at
}

/// How many closing brackets [`flow_collection_prefix`] tries before it gives up.
const FLOW_PREFIX_ATTEMPTS: usize = 512;

/// The prefix of `text` that is its first complete flow collection (#7136): `[1-4];[5-8];9` is read by
/// yq as `[1-4]`, since its decoder returns the first node of the document and never reads the rest.
/// The first `]` or `}` that leaves a prefix the YAML reader accepts as one flow collection ends it;
/// a closer inside a nested collection or a quoted string leaves a prefix that does not parse.
fn flow_collection_prefix(text: &str) -> Option<&str> {
    if !opens_with_flow_collection(text) {
        return None;
    }
    let prefix = text
        .match_indices([']', '}'])
        .take(FLOW_PREFIX_ATTEMPTS)
        .map(|(at, _)| &text[..=at])
        .find(|prefix| {
            YamlIndex::build_yq(prefix.as_bytes()).is_ok_and(|index| {
                matches!(
                    index.root(prefix.as_bytes()).value(),
                    YamlValue::Sequence(documents)
                        if documents.uncons_cursor().is_some_and(|(root, rest)| {
                            rest.uncons_cursor().is_none()
                                && matches!(root.value(), YamlValue::Mapping(_) | YamlValue::Sequence(_))
                        })
                )
            })
        })?;
    token_after_the_first_node_scans(&text[prefix.len()..], false).then_some(prefix)
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
    if is_simple_plain(text) {
        return if go_yaml_resolved_tag(text) == "!!str" {
            Cell::Str(text.to_string())
        } else {
            Cell::Scalar(text.to_string())
        };
    }
    // A tag with nothing to tag: a bare `!` is a null, any other (`!!`, `!!str`, `!x`) a string.
    // go-yaml reads a lone `\r` as a line break, as the snippet is read here; a string keeps it.
    let norm = text.replace('\r', "\n");
    let bare = yaml_trim(&norm);
    if bare.starts_with('!') && !bare.contains(char::is_whitespace) {
        return if bare == "!" {
            Cell::Scalar(bare.to_string())
        } else {
            Cell::Str(text.to_string())
        };
    }
    let Ok(index) = YamlIndex::build_yq(norm.as_bytes()) else {
        // #7136: yq decodes the first node and never reads what follows a complete flow collection.
        if let Some(prefix) = flow_collection_prefix(&norm) {
            return Cell::Collection(prefix.to_string());
        }
        // A later document the reader refuses (`1` / `...` / `2`) is never read by yq either: the
        // first document is the cell, and a string one keeps the whole text.
        let first = first_document(&norm);
        if first.len() < norm.len() && token_after_the_first_node_scans(&norm[first.len()..], true)
        {
            return match classify(first, auto_parse) {
                Cell::Str(_) => Cell::Str(text.to_string()),
                other => other,
            };
        }
        return Cell::Str(text.to_string());
    };
    // The documents sit in a virtual root sequence; a snippet is one of them, and one that holds
    // several is a string.
    let YamlValue::Sequence(documents) = index.root(norm.as_bytes()).value() else {
        return Cell::Str(text.to_string());
    };
    let Some((root, rest)) = documents.uncons_cursor() else {
        // Document markers and blanks only: yq's null, with no text.
        if norm
            .lines()
            .all(|line| matches!(line.trim(), "" | "---" | "..."))
        {
            return Cell::Empty;
        }
        // Nothing but comments, or nothing at all: a null, which keeps the text of the comment.
        return Cell::Scalar(yaml_trim(&norm).to_string());
    };
    if yaml_trim(&norm).is_empty() {
        // Blanks only: a null whose comment is the blanks, which yq writes behind its `# `.
        return Cell::Scalar(format!("# {norm}"));
    }
    // #7136: yq decodes one document, so a later one is never read: the first one is the cell. A first
    // document that is a string keeps the whole text, as below.
    let later_documents = rest.uncons_cursor().is_some();
    // What follows the first document must still scan, as it does after a flow collection.
    let first = first_document(&norm);
    if first.len() < norm.len() && !token_after_the_first_node_scans(&norm[first.len()..], true) {
        return Cell::Str(text.to_string());
    }
    match root.value() {
        // A document that is null and written with a marker is yq's null with no text; a comment-only
        // cell with no marker keeps the comment's text.
        YamlValue::Null
            if later_documents
                || norm.lines().any(|line| {
                    matches!(line.trim_end(), "---" | "...") || line.starts_with("--- #")
                }) =>
        {
            Cell::Empty
        }
        YamlValue::Mapping(_) | YamlValue::Sequence(_) => {
            Cell::Collection(first_document(&norm).to_string())
        }
        YamlValue::Null => Cell::Scalar(yaml_trim(&norm).to_string()),
        YamlValue::String(s) => {
            let explicit = root.explicit_tag();
            let unquoted = s.is_unquoted();
            let value = s.as_str().ok();
            let is_str = match (explicit, &value) {
                (Some(tag), _) => tag == "!!str",
                // A plain scalar is typed as go-yaml's `resolve` types it (a date is a timestamp
                // and `1_000` an integer); a quoted one is a string.
                (None, Some(value)) if unquoted => {
                    go_yaml_resolved_tag(yaml_trim(value)) == "!!str"
                }
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
                Some(value) if unquoted => scalar.push_str(yaml_trim(&value)),
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
pub(super) fn string_scalar(s: &str, block_indent: &str) -> String {
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
/// A header's name as the text that opens its entry, colon included: `name:` or the `? ` form.
fn key_text(name: &[u8]) -> String {
    let child = STEP.repeat(2);
    let name = String::from_utf8_lossy(name);
    if go_yaml_key_is_simple(&name) {
        format!("{}:", string_scalar(&name, &child))
    } else {
        // go-yaml's `? ` explicit key, for a key with a line break or over 128 bytes.
        let key = go_yaml_explicit_key_scalar(&name, &child, STEP.len())
            .unwrap_or_else(|| string_scalar(&name, &child));
        format!("? {key}\n{STEP}:")
    }
}

/// One record as an item of the document's sequence; `keys` are the header's [`key_text`]s.
fn write_row(out: &mut String, keys: &[String], record: &[Vec<u8>], auto_parse: bool) {
    let child = STEP.repeat(2);
    for (n, (key, cell)) in keys.iter().zip(record).enumerate() {
        out.push_str(if n == 0 { "- " } else { STEP });
        out.push_str(key);
        let cell = String::from_utf8_lossy(cell);
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

    #[test]
    fn a_parse_error_names_the_line_the_record_started_on() {
        assert_eq!(
            err("\"a\nb\"x,y\n"),
            "record on line 1; parse error on line 2, column 2: extraneous or missing \" in quoted-field"
        );
        assert_eq!(
            err("a\n\"x\ny\n"),
            "record on line 2; parse error on line 3, column 3: extraneous or missing \" in quoted-field"
        );
    }

    #[test]
    fn a_lone_cr_ends_the_last_line_and_a_quote_cannot_be_the_separator() {
        assert_eq!(yaml("a\n1\r"), "- a: 1\n");
        let message = transcode(b"a\n1\n", '"', true).unwrap_err();
        assert_eq!(message, "csv: invalid field or comment delimiter");
    }

    #[test]
    fn blanks_several_documents_and_an_alias_are_read_as_yq_reads_them() {
        let cell = |text: &str| classify(text, true);
        assert_eq!(cell(" "), Cell::Scalar("#  ".into()));
        assert_eq!(cell("a\n---\nb"), Cell::Str("a\n---\nb".into()));
        assert_eq!(cell("*a"), Cell::Str("*a".into()));
        assert_eq!(cell("!!int \"5\""), Cell::Scalar("!!int \"5\"".into()));
    }
}
