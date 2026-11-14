//! The comments an output encoder writes (#7117, #7156): a node's head, line and foot comment as yq's node
//! holds them, in the shape of the value they belong to. Read off a cursor (`collect_encoder_comments`) or
//! from the presentation tree a write comes with (`encoder_comments_of_tree`), and consumed by the encoders
//! that write comments: `-o xml` (`yq_xml`), `-o lua` and the rest.

use alloc::string::String;
use alloc::vec::Vec;

/// The head, line and foot comment of one node as yq's node holds it: `#` and all, the lines of a
/// block joined with a line break (go-yaml's `HeadComment`, `LineComment`, `FootComment`).
#[derive(Debug, Clone, Default)]
pub(crate) struct EncoderNodeComments {
    pub head: String,
    pub line: String,
    pub foot: String,
}

/// What a node's members hold: a mapping's entries in order, or a sequence's items.
#[derive(Debug, Clone, Default)]
pub(crate) enum EncoderMembers {
    #[default]
    None,
    Map(Vec<EncoderEntry>),
    Seq(Vec<EncoderComments>),
}

/// The comments of a node and of everything below it, in the shape of the value they came from.
#[derive(Debug, Clone, Default)]
pub(crate) struct EncoderComments {
    pub own: EncoderNodeComments,
    pub members: EncoderMembers,
}

/// A mapping entry: the key's own comments and the value's tree.
#[derive(Debug, Clone)]
pub(crate) struct EncoderEntry {
    pub key: String,
    pub key_comments: EncoderNodeComments,
    pub value: EncoderComments,
}

pub(crate) static NO_COMMENTS: EncoderComments = EncoderComments {
    own: EncoderNodeComments {
        head: String::new(),
        line: String::new(),
        foot: String::new(),
    },
    members: EncoderMembers::None,
};

impl EncoderComments {
    /// The comments of entry `index` of a mapping, if they are those of `key`: its key's and its value's.
    /// A write that moved or renamed the entry leaves none, which writes none.
    pub(crate) fn entry(&self, index: usize, key: &str) -> (&EncoderNodeComments, &Self) {
        match &self.members {
            EncoderMembers::Map(entries) => match entries.get(index) {
                Some(entry) if entry.key == key => (&entry.key_comments, &entry.value),
                _ => (&NO_COMMENTS.own, &NO_COMMENTS),
            },
            _ => (&NO_COMMENTS.own, &NO_COMMENTS),
        }
    }

    /// The comments of item `index` of a sequence.
    pub(crate) fn item(&self, index: usize) -> &Self {
        match &self.members {
            EncoderMembers::Seq(items) => items.get(index).unwrap_or(&NO_COMMENTS),
            _ => &NO_COMMENTS,
        }
    }
}

/// The comments of a document read with `-p toml`, moved to the value nodes yq's TOML decoder gave them to
/// (#7170). A TOML file has no comment of a key, so what the text holds above a key is its value's head
/// comment, and what follows a key whose value is a non-empty array on the next lines is the array's line
/// comment (the read writes it there, `input_toml::render_map_for_toml`). A sequence's items keep their own.
pub(crate) fn toml_value_comments(comments: &EncoderComments) -> EncoderComments {
    let members = match &comments.members {
        EncoderMembers::Map(entries) => EncoderMembers::Map(
            entries
                .iter()
                .map(|entry| {
                    let mut value = toml_value_comments(&entry.value);
                    value.own.head.clone_from(&entry.key_comments.head);
                    // Whatever the value is now: a write that replaced the array keeps its comment, as yq's does.
                    if !entry.key_comments.line.is_empty() {
                        value.own.line.clone_from(&entry.key_comments.line);
                    }
                    EncoderEntry {
                        key: entry.key.clone(),
                        key_comments: EncoderNodeComments::default(),
                        value,
                    }
                })
                .collect(),
        ),
        EncoderMembers::Seq(items) => {
            EncoderMembers::Seq(items.iter().map(toml_value_comments).collect())
        }
        EncoderMembers::None => EncoderMembers::None,
    };
    EncoderComments {
        own: comments.own.clone(),
        members,
    }
}

/// The comment lines of the leading block of a TOML file read with `-p toml`, split at the explicit `---`
/// the read writes after the root mapping's head comment (#7170). The YAML reader holds every comment before
/// the first key as one block, the first key's head comment among them, and spells the `---` in it
/// `$yqDocSeparator$`: what is ahead of the marker is the root's head, what follows it the first entry's (a
/// table whose comments follow its header and that has no key of its own).
fn toml_leading_blocks(leading: &str) -> (Vec<&str>, Vec<&str>) {
    let mut root = Vec::new();
    let mut entry = Vec::new();
    let mut after_marker = false;
    for line in leading.lines().map(str::trim) {
        if line == "$yqDocSeparator$" {
            after_marker = true;
        } else if line.starts_with('#') {
            if after_marker {
                entry.push(line);
            } else {
                root.push(line);
            }
        }
    }
    (root, entry)
}

/// The root mapping's head comment of a TOML file read with `-p toml`: the first block of its leading block.
pub(crate) fn toml_root_head(leading: &str) -> String {
    toml_leading_blocks(leading).0.join("\n")
}

/// Give the first entry's head comment, which the YAML reader folded into the leading block, back to the
/// entry it belonged to: `first_key`, the key that was first when the document was read (a write may have
/// sorted another one in front of it). It becomes the head of that key, as every other entry's is.
pub(crate) fn toml_first_entry_head(
    comments: &mut EncoderComments,
    leading: &str,
    first_key: Option<&str>,
) {
    let entry = toml_leading_blocks(leading).1;
    let (false, Some(first_key), EncoderMembers::Map(entries)) =
        (entry.is_empty(), first_key, &mut comments.members)
    else {
        return;
    };
    if let Some(found) = entries.iter_mut().find(|e| e.key == first_key) {
        found.key_comments.head = entry.join("\n");
    }
}

/// The key that comes first among `comments`' entries.
pub(crate) fn first_entry_key(comments: &EncoderComments) -> Option<&str> {
    match &comments.members {
        EncoderMembers::Map(entries) => entries.first().map(|entry| entry.key.as_str()),
        _ => None,
    }
}

/// A document read with `-p xml` and written as XML: the comments before the first element belong to
/// the first key, as yq's XML decoder attached them (#7202). The YAML reader holds them as the document's
/// leading block, which the XML encoder writes through the `# ` marker rules of a comment of its own and so
/// trims a first line's trailing blank where the key's head comment (what yq has) does not. They are moved
/// back to `first_key` (the key that was first when the document was read), unchanged, and the leading block
/// is left empty.
pub(crate) fn xml_first_entry_head(
    comments: &mut EncoderComments,
    leading: &mut String,
    first_key: Option<&str>,
) {
    let (Some(first_key), EncoderMembers::Map(entries)) = (first_key, &mut comments.members) else {
        return;
    };
    let Some(found) = entries.iter_mut().find(|e| e.key == first_key) else {
        return;
    };
    let head: Vec<&str> = leading
        .lines()
        .map(str::trim_start)
        .filter(|line| line.starts_with('#'))
        .collect();
    if head.is_empty() || !found.key_comments.head.is_empty() {
        return;
    }
    found.key_comments.head = head.join("\n");
    leading.clear();
}
