//! yq's `-p xml` input format (#6925): `decoder_xml.go` over Go's `encoding/xml`, transcoded to the
//! YAML text yq prints for the same nodes.
//!
//! The tokens come from [`lexer`], a port of the decoder Go ships (`RawToken` by default, `Token`
//! under `--xml-raw-token=false`, lenient unless `--xml-strict-mode`). yq folds them into a tree
//! that keeps the order of a parent's children: an element is a mapping of its attributes (under
//! `--xml-attribute-prefix`) and its children's names, a name that repeats is a sequence, text is
//! the `--xml-content-name` entry (a sequence when there is more than one piece), and an element
//! with neither children nor attributes is its text or `null`. Every value is a `!!str`: nothing
//! is read as a number or a boolean. A comment becomes a head, line or foot comment by where it
//! sits relative to the element's start and its text ([`Builder`]), and the nodes are then
//! written as yq's YAML encoder writes them and read as `-p yaml`, so every route after the read
//! is the YAML route's.
//!
//! One divergence from the reference: yq dereferences a nil element when a token other than an end
//! tag follows an end tag that closes the document's root (`</a><b/>`) and dies of a Go panic;
//! this reads such an input as an error instead.

mod lexer;

use lexer::{Decoder, LexerOptions, Token};
use succinctly::yaml::{
    go_yaml_dom_scalar, go_yaml_explicit_key_scalar, go_yaml_key_is_simple,
    go_yaml_multiline_scalar,
};

/// One level of indentation in the text written.
const STEP: &str = "  ";

/// The flags `-p xml` reads (yq's `XmlPreferences`).
#[derive(Clone, Debug)]
pub(super) struct XmlOptions {
    /// `--xml-attribute-prefix`.
    pub attribute_prefix: String,
    /// `--xml-content-name`.
    pub content_name: String,
    /// `--xml-proc-inst-prefix`.
    pub proc_inst_prefix: String,
    /// `--xml-directive-name`.
    pub directive_name: String,
    /// `--xml-keep-namespace`.
    pub keep_namespace: bool,
    /// `--xml-raw-token`.
    pub raw_token: bool,
    /// `--xml-skip-directives`.
    pub skip_directives: bool,
    /// `--xml-skip-proc-inst`.
    pub skip_proc_inst: bool,
    /// `--xml-strict-mode`.
    pub strict: bool,
}

impl Default for XmlOptions {
    /// yq's defaults (`NewDefaultXmlPreferences`).
    fn default() -> Self {
        XmlOptions {
            attribute_prefix: "+@".to_string(),
            content_name: "+content".to_string(),
            proc_inst_prefix: "+p_".to_string(),
            directive_name: "+directive".to_string(),
            keep_namespace: true,
            raw_token: true,
            skip_directives: false,
            skip_proc_inst: false,
            strict: false,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The tree: decoder_xml.go's xmlNode
// ---------------------------------------------------------------------------------------------

#[derive(Default)]
struct XNode {
    children: Vec<Kv>,
    head: String,
    foot: String,
    line: String,
    data: Vec<String>,
}

struct Kv {
    key: String,
    values: Vec<usize>,
    foot: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Fresh,
    Started,
    Chardata,
    Finished,
}

struct Element {
    parent: Option<usize>,
    node: usize,
    label: String,
    state: State,
}

struct Builder {
    nodes: Vec<XNode>,
}

impl Builder {
    fn new_node(&mut self) -> usize {
        self.nodes.push(XNode::default());
        self.nodes.len() - 1
    }

    /// `xmlNode.AddChild`: the value joins the entry for `key` when there is one.
    fn add_child(&mut self, node: usize, key: &str, child: usize) {
        let children = &mut self.nodes[node].children;
        if let Some(entry) = children.iter_mut().find(|kv| kv.key == key) {
            entry.values.push(child);
            return;
        }
        children.push(Kv {
            key: key.to_string(),
            values: vec![child],
            foot: String::new(),
        });
    }

    fn add_text_child(&mut self, node: usize, key: &str, text: String) {
        let child = self.new_node();
        self.nodes[child].data = vec![text];
        self.add_child(node, key, child);
    }
}

fn join_comments(parts: &[&str], sep: &str) -> String {
    parts
        .iter()
        .filter(|part| !part.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(sep)
}

/// `applyFootComment`: the comment goes on the last child's last value when that child is a
/// scalar, on the child's entry otherwise, and on the element when it has no children.
fn apply_foot_comment(b: &mut Builder, element_node: usize, comment: &str) {
    let Some(last) = b.nodes[element_node].children.len().checked_sub(1) else {
        let foot = join_comments(&[&b.nodes[element_node].foot, comment], " ");
        b.nodes[element_node].foot = foot;
        return;
    };
    let values = b.nodes[element_node].children[last].values.clone();
    if !values.is_empty() && b.nodes[values[0]].children.is_empty() {
        let target = values[values.len() - 1];
        let foot = join_comments(&[&b.nodes[target].foot, comment], " ");
        b.nodes[target].foot = foot;
    } else {
        // yq joins the element's own foot comment here, not the entry's.
        let foot = join_comments(&[&b.nodes[element_node].foot, comment], " ");
        b.nodes[element_node].children[last].foot = foot;
    }
}

/// `decodeXML`: the tree of one input.
fn decode_xml(raw: &[u8], options: &XmlOptions) -> Result<(Builder, usize), String> {
    let mut b = Builder { nodes: Vec::new() };
    let root = b.new_node();
    let mut decoder = Decoder::new(
        raw,
        LexerOptions {
            strict: options.strict,
            raw_token: options.raw_token,
        },
    );
    let mut elements = vec![Element {
        parent: None,
        node: root,
        label: String::new(),
        state: State::Fresh,
    }];
    let mut current: Option<usize> = Some(0);
    let mut started = false;
    // yq dies of a nil dereference when it reads past the end of the root; this is the message
    // for the same input.
    let past_root = || "invalid XML: markup after the end of the document's root".to_string();

    while let Some(token) = decoder.next().map_err(|e| e.message())? {
        match token {
            Token::Start { name, attr } => {
                let Some(cur) = current else {
                    return Err(past_root());
                };
                elements[cur].state = State::Started;
                let mut label = name.local.clone();
                if options.keep_namespace && !name.space.is_empty() {
                    label = format!("{}:{}", name.space, name.local);
                }
                let node = b.new_node();
                elements.push(Element {
                    parent: Some(cur),
                    node,
                    label,
                    state: State::Fresh,
                });
                current = Some(elements.len() - 1);
                for a in attr {
                    let mut local = a.name.local;
                    if options.keep_namespace && !a.name.space.is_empty() {
                        local = format!("{}:{}", a.name.space, local);
                    }
                    let key = format!("{}{}", options.attribute_prefix, local);
                    b.add_text_child(node, &key, a.value);
                }
            }
            Token::CharData(data) => {
                let text = String::from_utf8_lossy(&data).into_owned();
                let new_bit = lexer::trim_non_graphic(&text);
                if !started && !new_bit.is_empty() {
                    return Err(format!(
                        "invalid XML: Encountered chardata [{new_bit}] outside of XML node"
                    ));
                }
                if !new_bit.is_empty() {
                    let Some(cur) = current else {
                        return Err(past_root());
                    };
                    let node = elements[cur].node;
                    b.nodes[node].data.push(new_bit.to_string());
                    elements[cur].state = State::Chardata;
                }
            }
            Token::End(_) => {
                let Some(cur) = current else {
                    continue;
                };
                elements[cur].state = State::Finished;
                if let Some(parent) = elements[cur].parent {
                    let label = elements[cur].label.clone();
                    let (parent_node, node) = (elements[parent].node, elements[cur].node);
                    b.add_child(parent_node, &label, node);
                }
                current = elements[cur].parent;
            }
            Token::Comment(data) => {
                let Some(cur) = current else {
                    return Err(past_root());
                };
                let comment = String::from_utf8_lossy(&data).into_owned();
                let node = elements[cur].node;
                match elements[cur].state {
                    State::Started => apply_foot_comment(&mut b, node, &comment),
                    State::Chardata => {
                        let line = join_comments(&[&b.nodes[node].line, &comment], " ");
                        b.nodes[node].line = line;
                    }
                    State::Fresh | State::Finished => {
                        let head = join_comments(&[&b.nodes[node].head, &comment], " ");
                        b.nodes[node].head = head;
                    }
                }
            }
            Token::ProcInst { target, inst } => {
                if !options.skip_proc_inst {
                    let Some(cur) = current else {
                        return Err(past_root());
                    };
                    let key = format!("{}{}", options.proc_inst_prefix, target);
                    let node = elements[cur].node;
                    b.add_text_child(node, &key, String::from_utf8_lossy(&inst).into_owned());
                }
            }
            Token::Directive(data) => {
                if !options.skip_directives {
                    let Some(cur) = current else {
                        return Err(past_root());
                    };
                    let node = elements[cur].node;
                    b.add_text_child(
                        node,
                        &options.directive_name.clone(),
                        String::from_utf8_lossy(&data).into_owned(),
                    );
                }
            }
        }
        started = true;
    }
    Ok((b, root))
}

// ---------------------------------------------------------------------------------------------
// The nodes yq makes of the tree
// ---------------------------------------------------------------------------------------------

/// A scalar, a sequence or a mapping with the comments yq puts on it.
enum Y {
    /// `None` is `null`.
    Scalar {
        text: Option<String>,
        head: String,
        line: String,
        foot: String,
    },
    Seq {
        items: Vec<Y>,
        head: String,
        foot: String,
    },
    Map(Vec<Entry>),
}

struct Entry {
    key: String,
    head: String,
    line: String,
    foot: String,
    value: Y,
}

/// `decoder.processComment`: `# ` before a comment that starts with a letter, `#` before each line.
fn process_comment(c: &str) -> String {
    if c.is_empty() {
        return String::new();
    }
    let mut spaced = String::with_capacity(c.len() + 2);
    let mut at_line_start = true;
    for ch in c.chars() {
        if at_line_start && ch.is_ascii_alphabetic() {
            spaced.push(' ');
        }
        spaced.push(ch);
        at_line_start = ch == '\n';
    }
    format!("#{}", spaced.trim_end_matches(' ').replace('\n', "\n#"))
}

fn bare_scalar(text: Option<String>) -> Y {
    Y::Scalar {
        text,
        head: String::new(),
        line: String::new(),
        foot: String::new(),
    }
}

/// `createValueNodeFromData`.
fn value_from_data(values: &[String]) -> Y {
    match values.len() {
        0 => bare_scalar(None),
        1 => bare_scalar(Some(values[0].clone())),
        _ => Y::Seq {
            items: values
                .iter()
                .map(|v| bare_scalar(Some(v.clone())))
                .collect(),
            head: String::new(),
            foot: String::new(),
        },
    }
}

struct Converter<'a> {
    b: &'a mut Builder,
    options: &'a XmlOptions,
}

impl Converter<'_> {
    fn convert(&mut self, n: usize) -> Y {
        if !self.b.nodes[n].children.is_empty() {
            return self.create_map(n);
        }
        let node = &self.b.nodes[n];
        let mut scalar = value_from_data(&node.data);
        let (head, line, foot) = (
            process_comment(&node.head),
            process_comment(&node.line),
            process_comment(&node.foot),
        );
        match &mut scalar {
            Y::Scalar {
                head: h,
                line: l,
                foot: f,
                ..
            } => {
                *h = head;
                *l = line;
                *f = foot;
            }
            Y::Seq {
                items,
                head: h,
                foot: f,
            } => {
                // The line comment moves to the first element's head comment.
                if let Y::Scalar { head: first, .. } = &mut items[0] {
                    *first = line;
                }
                *h = head;
                *f = foot;
            }
            Y::Map(_) => {}
        }
        scalar
    }

    fn create_sequence(&mut self, nodes: &[usize]) -> Y {
        let items = nodes.iter().map(|&child| self.convert(child)).collect();
        Y::Seq {
            items,
            head: String::new(),
            foot: String::new(),
        }
    }

    fn create_map(&mut self, n: usize) -> Y {
        let mut entries = Vec::new();
        let (head, line, foot, data) = {
            let node = &self.b.nodes[n];
            (
                node.head.clone(),
                node.line.clone(),
                node.foot.clone(),
                node.data.clone(),
            )
        };
        if !data.is_empty() {
            entries.push(Entry {
                key: self.options.content_name.clone(),
                head: process_comment(&head),
                line: process_comment(&line),
                foot: process_comment(&foot),
                value: value_from_data(&data),
            });
        }
        let kvs: Vec<(String, Vec<usize>, String)> = self.b.nodes[n]
            .children
            .iter()
            .map(|kv| (kv.key.clone(), kv.values.clone(), kv.foot.clone()))
            .collect();
        for (i, (key, values, kv_foot)) in kvs.into_iter().enumerate() {
            let mut entry_head = String::new();
            if i == 0 {
                entry_head = process_comment(&head);
            }
            let value = if values.len() > 1 {
                self.create_sequence(&values)
            } else {
                let child = values[0];
                let c = &mut self.b.nodes[child];
                if c.children.is_empty() && !c.head.is_empty() {
                    if !c.data.is_empty() {
                        entry_head = join_comments(&[&entry_head, c.head.trim()], "\n");
                        c.head = String::new();
                    } else {
                        c.line = std::mem::take(&mut c.head);
                    }
                }
                self.convert(child)
            };
            entries.push(Entry {
                key,
                head: entry_head,
                line: String::new(),
                foot: process_comment(&kv_foot),
                value,
            });
        }
        Y::Map(entries)
    }
}

// ---------------------------------------------------------------------------------------------
// The YAML text
// ---------------------------------------------------------------------------------------------

/// `s` as go-yaml writes a string with no style of its own; `block_indent` is the indent of a
/// literal block's lines.
fn string_scalar(s: &str, block_indent: &str) -> String {
    go_yaml_dom_scalar(s, false, false, false)
        .or_else(|| go_yaml_multiline_scalar(s, block_indent, STEP.len()))
        .unwrap_or_else(|| succinctly::yaml::go_yaml_double_quoted_scalar(s))
}

/// A comment as lines: go-yaml writes `# ` before a line that does not begin with `#`.
fn comment_lines(c: &str) -> impl Iterator<Item = String> + '_ {
    c.split('\n').map(|line| {
        if line.starts_with('#') {
            line.to_string()
        } else {
            format!("# {}", line.trim())
        }
    })
}

fn write_comment(out: &mut String, comment: &str, indent: &str) {
    for line in comment_lines(comment) {
        out.push_str(indent);
        out.push_str(&line);
        out.push('\n');
    }
}

/// `name:` at `indent`; the `? ` explicit-key form for a key with a line break or over 128 bytes.
fn key_text(name: &str, indent: &str) -> String {
    let child = format!("{indent}{STEP}");
    if go_yaml_key_is_simple(name) {
        format!("{}:", string_scalar(name, &child))
    } else {
        let key = go_yaml_explicit_key_scalar(name, &child, STEP.len())
            .unwrap_or_else(|| string_scalar(name, &child));
        format!("? {key}\n{indent}:")
    }
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

/// The writer: the text so far and the foot comments of sequences that yq's encoder holds back.
///
/// go-yaml writes a scalar's foot comment right after the scalar and a mapping entry's right after
/// its value, but the foot comment of a block sequence waits for the next entry or item written
/// after the sequence ended (whatever its depth), goes after that one at its indent, and is
/// written after the whole document, behind a blank line, when nothing follows.
struct Writer {
    out: String,
    pending: Vec<String>,
}

impl Writer {
    /// A scalar value: ` text`, then the line comment (its first line inline, the rest as comment
    /// lines at `indent`). A literal block takes the comment after its `|` header line.
    fn scalar(&mut self, text: Option<&str>, line: &str, indent: &str) {
        let child = format!("{indent}{STEP}");
        let mut rest = Vec::new();
        let mut inline = String::new();
        if !line.is_empty() {
            let mut lines = comment_lines(line);
            if let Some(first) = lines.next() {
                inline = format!(" {first}");
            }
            rest.extend(lines);
        }
        if let Some(text) = text {
            self.out.push(' ');
            let rendered = hang(&string_scalar(text, &child), &child);
            match rendered.split_once('\n') {
                Some((header, body)) if header.starts_with(['|', '>']) => {
                    self.out.push_str(header);
                    self.out.push_str(&inline);
                    self.out.push('\n');
                    self.out.push_str(body);
                    inline.clear();
                }
                _ => self.out.push_str(&rendered),
            }
        }
        self.out.push_str(&inline);
        self.out.push('\n');
        for line in rest {
            self.out.push_str(indent);
            self.out.push_str(&line);
            self.out.push('\n');
        }
    }

    /// The comments written after an entry or an item at `indent`: its own foot comments, then
    /// the held-back foot comments of sequences that ended before it began.
    fn after(&mut self, foot: &[&str], held: Vec<String>, indent: &str, more_follow: bool) {
        let mut wrote = false;
        for comment in foot.iter().copied().chain(held.iter().map(String::as_str)) {
            if !comment.is_empty() {
                write_comment(&mut self.out, comment, indent);
                wrote = true;
            }
        }
        if wrote && more_follow {
            self.out.push('\n');
        }
    }

    fn map(&mut self, entries: &[Entry], indent: &str) {
        let child = format!("{indent}{STEP}");
        for (at, entry) in entries.iter().enumerate() {
            let held = std::mem::take(&mut self.pending);
            if !entry.head.is_empty() {
                write_comment(&mut self.out, &entry.head, indent);
            }
            self.out.push_str(indent);
            self.out.push_str(&key_text(&entry.key, indent));
            let mut foot = "";
            match &entry.value {
                Y::Scalar {
                    text,
                    line,
                    foot: f,
                    ..
                } => {
                    let line = if line.is_empty() { &entry.line } else { line };
                    self.scalar(text.as_deref(), line, indent);
                    foot = f;
                }
                Y::Seq {
                    items,
                    head,
                    foot: f,
                } => {
                    if !entry.line.is_empty() {
                        // The key's line comment follows its colon.
                        let mut lines = comment_lines(&entry.line);
                        if let Some(first) = lines.next() {
                            self.out.push(' ');
                            self.out.push_str(&first);
                        }
                        let rest: Vec<String> = lines.collect();
                        self.out.push('\n');
                        for line in rest {
                            self.out.push_str(indent);
                            self.out.push_str(&line);
                            self.out.push('\n');
                        }
                    } else {
                        self.out.push('\n');
                    }
                    if !head.is_empty() {
                        write_comment(&mut self.out, head, &child);
                    }
                    self.seq(items, &child);
                    if !f.is_empty() {
                        self.pending.push(f.clone());
                    }
                }
                Y::Map(inner) => {
                    self.out.push('\n');
                    self.map(inner, &child);
                }
            }
            self.after(&[foot, &entry.foot], held, indent, at + 1 < entries.len());
        }
    }

    fn seq(&mut self, items: &[Y], indent: &str) {
        let child = format!("{indent}{STEP}");
        for (at, item) in items.iter().enumerate() {
            let held = std::mem::take(&mut self.pending);
            let mut foot = "";
            match item {
                Y::Scalar {
                    text,
                    head,
                    line,
                    foot: f,
                } => {
                    if !head.is_empty() {
                        write_comment(&mut self.out, head, indent);
                    }
                    self.out.push_str(indent);
                    self.out.push('-');
                    self.scalar(text.as_deref(), line, indent);
                    foot = f;
                }
                Y::Map(entries) => {
                    // The first key's head comment follows the dash.
                    let mut inner = Writer {
                        out: String::new(),
                        pending: Vec::new(),
                    };
                    inner.map(entries, &child);
                    self.out.push_str(indent);
                    self.out.push_str("- ");
                    self.out
                        .push_str(inner.out.get(child.len()..).unwrap_or(""));
                    self.pending.append(&mut inner.pending);
                }
                Y::Seq {
                    items: inner_items,
                    head,
                    foot: f,
                } => {
                    // A sequence in a sequence: its first dash follows ours.
                    let mut inner = Writer {
                        out: String::new(),
                        pending: Vec::new(),
                    };
                    inner.seq(inner_items, &child);
                    self.out.push_str(indent);
                    self.out.push_str("- ");
                    if !head.is_empty() {
                        write_comment(&mut self.out, head, &child);
                    }
                    self.out
                        .push_str(inner.out.get(child.len()..).unwrap_or(""));
                    self.pending.append(&mut inner.pending);
                    if !f.is_empty() {
                        self.pending.push(f.clone());
                    }
                }
            }
            self.after(&[foot], held, indent, at + 1 < items.len());
        }
    }
}

/// `raw` as the YAML text of the document yq decodes it to, or the message of the error yq reports
/// (printed after `bad file '<name>': `).
pub(super) fn transcode(raw: &[u8], options: &XmlOptions) -> Result<Vec<u8>, String> {
    let (mut builder, root) = decode_xml(raw, options)?;
    let mut converter = Converter {
        b: &mut builder,
        options,
    };
    let document = converter.convert(root);
    let mut writer = Writer {
        out: String::new(),
        pending: Vec::new(),
    };
    match &document {
        Y::Map(entries) => writer.map(entries, ""),
        // A root with no element is `null` (yq prints an empty line) or the text that followed a
        // comment (`<!--c-->x`); yq prints neither with the comments that surround it, and a
        // document of no text is that empty line.
        Y::Scalar { text, .. } => {
            if let Some(text) = text {
                writer.out.push_str(&string_scalar(text, STEP));
                writer.out.push('\n');
            }
        }
        Y::Seq { items, head, foot } => {
            if !head.is_empty() {
                write_comment(&mut writer.out, head, "");
            }
            writer.seq(items, "");
            if !foot.is_empty() {
                writer.pending.push(foot.clone());
            }
        }
    }
    // The foot comments nothing followed are written after the document.
    for comment in std::mem::take(&mut writer.pending) {
        writer.out.push('\n');
        write_comment(&mut writer.out, &comment, "");
    }
    Ok(writer.out.into_bytes())
}
