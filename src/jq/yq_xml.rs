//! `succinctly yq -o=xml` (#6924): a port of yq's XML encoder over Go's `encoding/xml` printer.
//!
//! A map is a set of elements: a key is an element, a sequence repeats its key's element for each item,
//! a key starting with the attribute prefix (`+@`) is an attribute of the enclosing element (a scalar
//! only), `+content` is the element's text, a key starting with `+p_` is a processing instruction and
//! `+directive` a directive. The printer's indentation is Go's: a start tag opens a line one level deeper
//! unless text follows it, which keeps `<a>1</a>` on one line. Comments are not written (the value model
//! keeps none).

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

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

    fn scalar_text(value: &OwnedValue) -> String {
        match value {
            OwnedValue::Array(_) | OwnedValue::Object(_) => String::new(),
            other => super::eval::props_value_to_string(other, false),
        }
    }

    fn element(&mut self, name: &str, value: &OwnedValue, depth: usize) -> Result<(), EvalError> {
        super::value::assert_value_tree_depth(depth);
        match value {
            OwnedValue::Object(entries) => self.map(name, entries, depth),
            OwnedValue::Array(items) => {
                for item in items {
                    self.element(name, item, depth + 1)?;
                }
                Ok(())
            }
            scalar => {
                self.printer.start(name, &[])?;
                self.printer.text(&Self::scalar_text(scalar));
                self.printer.end(name);
                Ok(())
            }
        }
    }

    fn map(&mut self, name: &str, entries: &ObjectMap, depth: usize) -> Result<(), EvalError> {
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
                        Self::scalar_text(scalar),
                    )),
                }
            }
        }
        self.printer.start(name, &attributes)?;
        for (key, value) in entries {
            if key.starts_with(&self.options.proc_inst_prefix) {
                let target = key.replacen(&self.options.proc_inst_prefix, "", 1);
                self.printer
                    .proc_inst(&target, &Self::scalar_text(value), false)?;
            } else if *key == self.options.directive_name {
                self.printer.directive(&Self::scalar_text(value))?;
            } else if *key == self.options.content_name {
                self.printer.text(&Self::scalar_text(value));
            } else if !self.is_attribute(key) {
                self.element(key, value, depth + 1)?;
            }
        }
        self.printer.end(name);
        Ok(())
    }
}

/// The result as XML, ending in a line break unless it is a bare scalar (yq writes that as text alone).
pub(crate) fn format_xml(value: &OwnedValue, options: &XmlOptions) -> Result<String, EvalError> {
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
    let entries = match value {
        OwnedValue::Object(entries) => entries,
        OwnedValue::Array(_) => {
            return Err(EvalError::new(
                "cannot encode !!seq to XML - only maps can be encoded".to_string(),
            ))
        }
        scalar => {
            let mut out = String::new();
            escape(&mut out, &Encoder::scalar_text(scalar), false);
            return Ok(out);
        }
    };
    // The declaration comes first wherever its key is.
    let declaration = format!("{}xml", options.proc_inst_prefix);
    let mut first = true;
    for (key, value) in entries {
        if *key == declaration {
            encoder
                .printer
                .proc_inst("xml", &Encoder::scalar_text(value), first)?;
            encoder.printer.out.push('\n');
            first = false;
        }
    }
    for (key, value) in entries {
        if *key == declaration {
            continue;
        }
        if key.starts_with(&options.proc_inst_prefix) {
            let target = key.replacen(&options.proc_inst_prefix, "", 1);
            encoder
                .printer
                .proc_inst(&target, &Encoder::scalar_text(value), false)?;
            encoder.printer.out.push('\n');
        } else if *key == options.directive_name {
            encoder.printer.directive(&Encoder::scalar_text(value))?;
            encoder.printer.out.push('\n');
        } else {
            encoder.element(key, value, 0)?;
        }
    }
    encoder.printer.out.push('\n');
    Ok(encoder.printer.out)
}
