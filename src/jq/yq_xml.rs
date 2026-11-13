//! `succinctly yq -o=xml` (#6924): a port of yq's XML encoder over Go's `encoding/xml` printer.
//!
//! A map is a set of elements: a key is an element, a sequence repeats its key's element for each item,
//! a key starting with the attribute prefix (`+@`) is an attribute of the enclosing element (a scalar
//! only), `+content` is the element's text, a key starting with `+p_` is a processing instruction and
//! `+directive` a directive. The printer's indentation is Go's: a start tag opens a line one level deeper
//! unless text follows it, which keeps `<a>1</a>` on one line. A comment is written where yq's encoder
//! writes it when the caller has the nodes' comments ([`EncoderComments`], #7117); the value model keeps none.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use super::encoder_comments::{EncoderComments, EncoderNodeComments, NO_COMMENTS};
use super::error::EvalError;
use super::expr::XmlOptions;
use super::value::{ObjectMap, OwnedValue};

/// Go's `xml.Encoder` indentation state.
struct Printer<'a> {
    out: String,
    indent: &'a str,
    depth: usize,
    indented_in: bool,
    put_newline: bool,
}

impl Printer<'_> {
    /// `printer.writeIndent`: a newline and the indent before a tag, none inside text.
    fn write_indent(&mut self, delta: i32) {
        if self.indent.is_empty() {
            return;
        }
        if delta < 0 {
            self.depth -= 1;
            if self.indented_in {
                self.indented_in = false;
                return;
            }
            self.indented_in = false;
        }
        if self.put_newline {
            self.out.push('\n');
        } else {
            self.put_newline = true;
        }
        for _ in 0..self.depth {
            self.out.push_str(self.indent);
        }
        if delta > 0 {
            self.depth += 1;
            self.indented_in = true;
        }
    }

    fn start(&mut self, name: &str, attributes: &[(String, String)]) -> Result<(), EvalError> {
        if name.is_empty() {
            return Err(EvalError::new("xml: start tag with no name".to_string()));
        }
        self.write_indent(1);
        self.out.push('<');
        self.out.push_str(name);
        for (key, value) in attributes {
            if key.is_empty() {
                continue;
            }
            self.out.push(' ');
            self.out.push_str(key);
            self.out.push_str("=\"");
            escape(&mut self.out, value, true);
            self.out.push('"');
        }
        self.out.push('>');
        Ok(())
    }

    fn end(&mut self, name: &str) {
        self.write_indent(-1);
        self.out.push_str("</");
        self.out.push_str(name);
        self.out.push('>');
    }

    fn text(&mut self, text: &str) {
        escape(&mut self.out, text, false);
    }

    /// Go's `EncodeToken(xml.Comment)`: written as it is, with no indentation, and refused when it holds
    /// the closing marker.
    fn comment(&mut self, text: &str) -> Result<(), EvalError> {
        if text.contains("-->") {
            return Err(EvalError::new(
                "xml: EncodeToken of Comment containing --> marker".to_string(),
            ));
        }
        self.out.push_str("<!--");
        self.out.push_str(text);
        self.out.push_str("-->");
        Ok(())
    }

    fn proc_inst(&mut self, target: &str, inst: &str, first: bool) -> Result<(), EvalError> {
        if target == "xml" && !first {
            return Err(EvalError::new(
                "xml: EncodeToken of ProcInst xml target only valid for xml declaration, first token encoded"
                    .to_string(),
            ));
        }
        if !is_name(target) {
            return Err(EvalError::new(
                "xml: EncodeToken of ProcInst with invalid Target".to_string(),
            ));
        }
        if inst.contains("?>") {
            return Err(EvalError::new(
                "xml: EncodeToken of ProcInst containing ?> marker".to_string(),
            ));
        }
        self.out.push_str("<?");
        self.out.push_str(target);
        if !inst.is_empty() {
            self.out.push(' ');
            self.out.push_str(inst);
        }
        self.out.push_str("?>");
        Ok(())
    }

    fn directive(&mut self, directive: &str) -> Result<(), EvalError> {
        if !is_valid_directive(directive) {
            return Err(EvalError::new(
                "xml: EncodeToken of Directive containing wrong < or > markers".to_string(),
            ));
        }
        self.out.push_str("<!");
        self.out.push_str(directive);
        self.out.push('>');
        Ok(())
    }
}

/// Go's `xml.EscapeText` (`newline` is its `escapeNewline`): `"` `'` `&` `<` `>`, tab and carriage
/// return become references, a line break does only in an attribute, and a character XML cannot hold
/// becomes U+FFFD.
fn escape(out: &mut String, text: &str, newline: bool) {
    for c in text.chars() {
        match c {
            '"' => out.push_str("&#34;"),
            '\'' => out.push_str("&#39;"),
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\t' => out.push_str("&#x9;"),
            '\n' if newline => out.push_str("&#xA;"),
            '\n' => out.push('\n'),
            '\r' => out.push_str("&#xD;"),
            c if in_character_range(c) => out.push(c),
            _ => out.push('\u{FFFD}'),
        }
    }
}

/// XML 1.0 `Char`.
fn in_character_range(c: char) -> bool {
    matches!(c as u32, 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF)
}

/// Go's `isNameString`, approximated: a letter or `_` or `:` first, then those and digits, `-`, `.` and the
/// combining marks and middle dot XML allows. Go reads the XML 1.0 name tables; a character they accept
/// and Unicode's `Alphabetic` does not (or the reverse) is the gap.
fn is_name(name: &str) -> bool {
    let mut chars = name.chars();
    let first = chars
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_' || c == ':');
    first
        && chars.all(|c| {
            c.is_alphanumeric()
                || matches!(c, '_' | ':' | '-' | '.' | '\u{B7}')
                || (!c.is_ascii() && is_combining_mark(c))
        })
}

/// The combining-mark blocks XML 1.0 lists among a name's later characters.
fn is_combining_mark(c: char) -> bool {
    matches!(c as u32, 0x300..=0x36F | 0x483..=0x486 | 0x591..=0x5C4 | 0x64B..=0x652 | 0x900..=0x903 | 0x93C..=0x94D | 0x20D0..=0x20E1 | 0x203F..=0x2040)
}

/// Go's `isValidDirective`: balanced `<` `>`, closed quotes and comments.
fn is_valid_directive(directive: &str) -> bool {
    let bytes = directive.as_bytes();
    let (mut depth, mut quote, mut comment) = (0usize, 0u8, false);
    for (i, &c) in bytes.iter().enumerate() {
        if comment {
            if c == b'>' && i + 1 >= 3 && &bytes[i + 1 - 3..=i] == b"-->" {
                comment = false;
            }
        } else if quote != 0 {
            if c == quote {
                quote = 0;
            }
        } else if c == b'\'' || c == b'"' {
            quote = c;
        } else if c == b'<' {
            if i + 4 <= bytes.len() && &bytes[i..i + 4] == b"<!--" {
                comment = true;
            } else {
                depth += 1;
            }
        } else if c == b'>' {
            if depth == 0 {
                return false;
            }
            depth -= 1;
        }
    }
    depth == 0 && quote == 0 && !comment
}

/// yq's `headComment`, `lineComment` and `footComment`: the comment without its first `#`.
fn without_first_hash(comment: &str) -> String {
    comment.replacen('#', "", 1)
}

fn head_and_line(node: &EncoderNodeComments) -> String {
    without_first_hash(&node.head) + &without_first_hash(&node.line)
}

/// Go's regexp `\s`.
fn is_go_space(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\x0C' | '\r' | ' ')
}

/// `^\s*#(.*)\n?` replaced by `$1`: one comment line's marker, and the line break after it.
fn strip_single_line_marker(text: &str) -> String {
    let trimmed = text.trim_start_matches(is_go_space);
    let Some(rest) = trimmed.strip_prefix('#') else {
        return text.to_string();
    };
    let line_end = rest.find('\n').unwrap_or(rest.len());
    let (line, after) = rest.split_at(line_end);
    let after = after.strip_prefix('\n').unwrap_or(after);
    format!("{line}{after}")
}

/// `(^|\n) *# ?(.*)` replaced by `$1$2` over every line: a line that starts with blanks and a `#` loses
/// them and one space after the `#`.
fn strip_multi_line_markers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (n, line) in text.split('\n').enumerate() {
        if n > 0 {
            out.push('\n');
        }
        match line.trim_start_matches(' ').strip_prefix('#') {
            Some(rest) => out.push_str(rest.strip_prefix(' ').unwrap_or(rest)),
            None => out.push_str(line),
        }
    }
    out
}

/// The text of the comment yq's `encodeComment` writes for `comment` (already without its first `#`), or
/// `None` for the empty string, which it skips.
fn comment_text(comment: &str) -> Option<String> {
    if comment.is_empty() {
        return None;
    }
    let bytes = comment.as_bytes();
    let mut text = if bytes.len() > 2 && bytes[1..bytes.len() - 1].contains(&b'\n') {
        let chomped = comment.strip_suffix('\n').unwrap_or(comment);
        let mut text = strip_multi_line_markers(chomped);
        // The first line gets a space unless it starts with one or is empty.
        if !text.starts_with(['\n', ' ']) {
            text.insert(0, ' ');
        }
        text
    } else {
        strip_single_line_marker(comment)
    };
    if !text.ends_with([' ', '\n']) {
        text.push(' ');
    }
    Some(text)
}

struct Encoder<'a> {
    printer: Printer<'a>,
    options: &'a XmlOptions,
}

impl Encoder<'_> {
    fn is_attribute(&self, name: &str) -> bool {
        let options = self.options;
        name.starts_with(&options.attribute_prefix)
            && name != options.content_name
            && name != options.directive_name
            && !name.starts_with(&options.proc_inst_prefix)
    }

    fn scalar_text(&self, value: &OwnedValue) -> String {
        match value {
            OwnedValue::Array(_) | OwnedValue::Object(_) => String::new(),
            OwnedValue::Null if self.options.empty_null => String::new(),
            other => super::eval::props_value_to_string(other, false),
        }
    }

    /// yq's `encodeComment`.
    fn comment(&mut self, comment: &str) -> Result<(), EvalError> {
        match comment_text(comment) {
            Some(text) => self.printer.comment(&text),
            None => Ok(()),
        }
    }

    /// yq's `doEncode`: the element `name` for `value`.
    fn element(
        &mut self,
        name: &str,
        value: &OwnedValue,
        comments: &EncoderComments,
        depth: usize,
    ) -> Result<(), EvalError> {
        super::value::assert_value_tree_depth(depth);
        match value {
            OwnedValue::Object(entries) => self.map(name, entries, comments, depth),
            OwnedValue::Array(items) => {
                self.comment(&head_and_line(&comments.own))?;
                for (index, item) in items.iter().enumerate() {
                    self.element(name, item, comments.item(index), depth + 1)?;
                }
                self.comment(&without_first_hash(&comments.own.foot))
            }
            scalar => {
                self.printer.start(name, &[])?;
                self.comment(&without_first_hash(&comments.own.head))?;
                self.printer.text(&self.scalar_text(scalar));
                self.comment(&without_first_hash(&comments.own.line))?;
                self.printer.end(name);
                self.comment(&without_first_hash(&comments.own.foot))
            }
        }
    }

    /// yq's `encodeMap`.
    fn map(
        &mut self,
        name: &str,
        entries: &ObjectMap,
        comments: &EncoderComments,
        depth: usize,
    ) -> Result<(), EvalError> {
        let mut attributes = Vec::new();
        for (key, value) in entries {
            if self.is_attribute(key) {
                match value {
                    OwnedValue::Array(_) | OwnedValue::Object(_) => {
                        return Err(EvalError::new(format!(
                            "cannot use {} as attribute, only scalars are supported",
                            if matches!(value, OwnedValue::Array(_)) {
                                "!!seq"
                            } else {
                                "!!map"
                            }
                        )));
                    }
                    scalar => attributes.push((
                        key.replacen(&self.options.attribute_prefix, "", 1),
                        self.scalar_text(scalar),
                    )),
                }
            }
        }
        self.printer.start(name, &attributes)?;
        self.comment(&without_first_hash(&comments.own.head))?;
        for (index, (key, value)) in entries.into_iter().enumerate() {
            let (key_comments, value_comments) = comments.entry(index, key);
            self.comment(&head_and_line(key_comments))?;
            if key.starts_with(&self.options.proc_inst_prefix) {
                let target = key.replacen(&self.options.proc_inst_prefix, "", 1);
                self.printer
                    .proc_inst(&target, &self.scalar_text(value), false)?;
            } else if *key == self.options.directive_name {
                self.printer.directive(&self.scalar_text(value))?;
            } else if *key == self.options.content_name {
                self.comment(&head_and_line(&value_comments.own))?;
                self.printer.text(&self.scalar_text(value));
                self.comment(&without_first_hash(&value_comments.own.foot))?;
            } else if !self.is_attribute(key) {
                self.element(key, value, value_comments, depth + 1)?;
            }
            self.comment(&without_first_hash(&key_comments.foot))?;
        }
        self.printer.end(name);
        self.comment(&without_first_hash(&comments.own.foot))
    }
}

/// The result as XML, ending in a line break unless it is a bare scalar (yq writes that as text alone).
pub(crate) fn format_xml(value: &OwnedValue, options: &XmlOptions) -> Result<String, EvalError> {
    format_xml_commented(value, options, &NO_COMMENTS, "")
}

/// [`format_xml`] with the comments of the nodes the value came from, and the first document's
/// `leading` comment block, which yq writes ahead of everything but a `<?xml ?>` declaration.
pub(crate) fn format_xml_commented(
    value: &OwnedValue,
    options: &XmlOptions,
    comments: &EncoderComments,
    leading: &str,
) -> Result<String, EvalError> {
    let indent = " ".repeat(options.indent);
    let mut encoder = Encoder {
        printer: Printer {
            out: String::new(),
            indent: &indent,
            depth: 0,
            indented_in: false,
            put_newline: false,
        },
        options,
    };
    // The declaration comes first wherever its key is.
    let declaration = format!("{}xml", options.proc_inst_prefix);
    if let OwnedValue::Object(entries) = value {
        let mut first = true;
        for (key, value) in entries {
            if *key == declaration {
                let text = encoder.scalar_text(value);
                encoder.printer.proc_inst("xml", &text, first)?;
                encoder.printer.out.push('\n');
                first = false;
            }
        }
    }
    if !leading.is_empty() {
        encoder.comment(leading)?;
        encoder.printer.out.push('\n');
    }
    let entries = match value {
        OwnedValue::Object(entries) => entries,
        OwnedValue::Array(_) => {
            return Err(EvalError::new(
                "cannot encode !!seq to XML - only maps can be encoded".to_string(),
            ))
        }
        scalar => {
            encoder.printer.text(&encoder.scalar_text(scalar));
            return Ok(encoder.printer.out);
        }
    };
    encoder.comment(&head_and_line(&comments.own))?;
    for (index, (key, value)) in entries.into_iter().enumerate() {
        let (key_comments, value_comments) = comments.entry(index, key);
        let key_comment = head_and_line(key_comments);
        encoder.comment(&key_comment)?;
        if !key_comment.is_empty() {
            encoder.printer.out.push('\n');
        }
        if *key == declaration {
            // Written first, not twice.
        } else if key.starts_with(&options.proc_inst_prefix) {
            let target = key.replacen(&options.proc_inst_prefix, "", 1);
            let text = encoder.scalar_text(value);
            encoder.printer.proc_inst(&target, &text, false)?;
            encoder.printer.out.push('\n');
        } else if *key == options.directive_name {
            let text = encoder.scalar_text(value);
            encoder.printer.directive(&text)?;
            encoder.printer.out.push('\n');
        } else {
            encoder.element(key, value, value_comments, 0)?;
        }
        encoder.comment(&without_first_hash(&key_comments.foot))?;
    }
    encoder.comment(&without_first_hash(&comments.own.foot))?;
    encoder.printer.out.push('\n');
    Ok(encoder.printer.out)
}

#[cfg(test)]
mod tests {
    use super::super::encoder_comments::{EncoderEntry, EncoderMembers};
    use super::*;

    /// yq's `encodeComment` on the text it is given (a comment already without its first `#`), as
    /// captured from yq v4.53.3: a marker after the first is stripped, a block's later lines lose theirs,
    /// and the text ends in a space unless it ends in a line break.
    #[test]
    fn comment_text_follows_yqs_two_regexps_7117() {
        let rows: &[(&str, Option<&str>)] = &[
            ("", None),
            (" c", Some(" c ")),
            ("c", Some("c ")),
            ("# x", Some(" x ")),
            ("  # x", Some(" x ")),
            ("\n# x", Some(" x ")),
            (" f1\n# f2", Some(" f1\nf2 ")),
            (" a\n b", Some(" a\n b ")),
            (" a\n  # b # c", Some(" a\nb # c ")),
            (" a\n#b\n", Some(" a\nb ")),
            ("\n", Some("\n")),
            (" a\n", Some(" a\n")),
            ("x\ny", Some(" x\ny ")),
            ("#\n#", Some("\n")),
        ];
        for &(comment, want) in rows {
            assert_eq!(comment_text(comment).as_deref(), want, "{comment:?}");
        }
    }

    #[test]
    fn a_comment_holding_the_closing_marker_is_refused_7117() {
        let options = XmlOptions {
            indent: 2,
            attribute_prefix: "+@".into(),
            content_name: "+content".into(),
            proc_inst_prefix: "+p_".into(),
            directive_name: "+directive".into(),
            empty_null: false,
        };
        let comments = EncoderComments {
            own: EncoderNodeComments::default(),
            members: EncoderMembers::Map(alloc::vec![EncoderEntry {
                key: "a".into(),
                key_comments: EncoderNodeComments::default(),
                value: EncoderComments {
                    own: EncoderNodeComments {
                        line: "# x --> y".into(),
                        ..EncoderNodeComments::default()
                    },
                    members: EncoderMembers::None,
                },
            }]),
        };
        let value = OwnedValue::object_from([("a".to_string(), OwnedValue::Int(1))]);
        let err = format_xml_commented(&value, &options, &comments, "").unwrap_err();
        assert_eq!(
            err.message,
            "xml: EncodeToken of Comment containing --> marker"
        );
    }
}
