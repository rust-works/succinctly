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
