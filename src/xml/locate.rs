//! XML path location utilities.
//!
//! Finds the jq expression that navigates to a specific byte offset or
//! line/column position in an XML document — the `xq-locate` CLI's core,
//! mirroring `src/json/locate.rs`/`src/yaml/locate.rs`.
//!
//! Simpler than JSON's/YAML's version in one respect: a field's key is
//! always derivable from its own value node (see `light.rs`'s module docs),
//! so there's no separate key-BP-node lookup — just `cursor.value().key_string()`.
//! Every path component is a dotted/bracketed key (`.foo`, `["+@id"]`); XML
//! never produces an `[index]` component in this milestone, since elements
//! only project to `as_object()` (see `light.rs`).

#[cfg(not(test))]
use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

use crate::jq::document::DocumentValue;
use crate::xml::light::{XmlCursor, XmlIndex};

// ============================================================================
// Path building utilities
// ============================================================================

/// A component in a jq path expression.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PathComponent {
    /// A key that can use dot notation: `.foo`.
    DotKey(String),
    /// A key that needs bracket notation: `["+@id"]`.
    BracketKey(String),
}

impl PathComponent {
    fn to_jq_string(&self) -> String {
        match self {
            Self::DotKey(k) => format!(".{k}"),
            Self::BracketKey(k) => format!("[\"{}\"]", escape_jq_string(k)),
        }
    }
}

/// Check if a key can use dot notation in jq. XML's synthetic keys
/// (`+@attr`, `+content`) never qualify (`+` isn't alphabetic/underscore),
/// so they always fall back to bracket notation — this is intentional, not
/// a gap: `.foo["+@id"]` is the correct, unambiguous jq spelling.
fn can_use_dot_notation(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_alphabetic() || first == '_') && chars.all(|c| c.is_alphanumeric() || c == '_')
}

fn escape_jq_string(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            c => result.push(c),
        }
    }
    result
}

/// Find the BP position of the node containing a byte offset. Same
/// rank/select algorithm as `XmlCursor::cursor_at_offset` (`light.rs`) —
/// duplicated here rather than shared because this returns a bare BP
/// position for `path_to_bp` to walk, not a cursor.
fn find_node_at_offset<W: AsRef<[u64]>>(
    index: &XmlIndex<W>,
    text: &[u8],
    offset: usize,
) -> Option<usize> {
    if offset >= text.len() {
        return None;
    }

    let rank = index.ib_rank1(offset);
    let ib_idx = if let Some(struct_pos) = index.ib_select1(rank) {
        if struct_pos == offset {
            rank
        } else if rank > 0 {
            rank - 1
        } else {
            return None;
        }
    } else if rank > 0 {
        rank - 1
    } else {
        return None;
    };

    let bp = index.bp();
    let bp_len = bp.len();
    if bp_len == 0 {
        return None;
    }

    let mut lo = 0;
    let mut hi = bp_len;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if bp.rank1(mid + 1) <= ib_idx {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }

    if lo < bp_len && bp.rank1(lo + 1) == ib_idx + 1 {
        Some(lo)
    } else {
        None
    }
}

/// Build the jq path expression from root to a specific BP position.
pub fn path_to_bp<W: AsRef<[u64]> + Clone>(
    index: &XmlIndex<W>,
    text: &[u8],
    target_bp: usize,
) -> Option<String> {
    let mut components: Vec<PathComponent> = Vec::new();
    let mut current_bp = target_bp;

    while let Some(parent_bp) = index.bp().parent(current_bp) {
        let cursor = XmlCursor::from_bp_position(index, text, current_bp);
        let key = cursor.value().key_string()?.into_owned();
        if can_use_dot_notation(&key) {
            components.push(PathComponent::DotKey(key));
        } else {
            components.push(PathComponent::BracketKey(key));
        }
        current_bp = parent_bp;
    }

    components.reverse();

    if components.is_empty() {
        Some(".".to_string())
    } else {
        let mut result = String::new();
        for (i, comp) in components.iter().enumerate() {
            if i == 0 {
                if let PathComponent::BracketKey(_) = comp {
                    result.push('.');
                }
            }
            result.push_str(&comp.to_jq_string());
        }
        Some(result)
    }
}

/// Find the jq expression for a byte offset in XML text.
pub fn locate_offset<W: AsRef<[u64]> + Clone>(
    index: &XmlIndex<W>,
    text: &[u8],
    offset: usize,
) -> Option<String> {
    let bp_pos = find_node_at_offset(index, text, offset)?;
    path_to_bp(index, text, bp_pos)
}

/// Result of locating a position in XML.
#[derive(Debug, Clone)]
pub struct LocateResult {
    /// The jq expression to navigate to this position.
    pub expression: String,
    /// The byte range of the value in the original text. For `Element`
    /// nodes this is `(start, text.len())` rather than the precise closing
    /// tag position — finding an element's exact end would need a second
    /// tag-aware scan the way `JsonCursor::text_range` does for containers,
    /// which isn't worth the complexity for this milestone's peripheral
    /// `--format json` reporting; `Attribute`/`Text` ranges are exact.
    pub byte_range: (usize, usize),
    /// The jq type name of the value (`"object"` or `"string"`).
    pub value_type: &'static str,
}

/// Find detailed location info for a byte offset in XML text.
pub fn locate_offset_detailed<W: AsRef<[u64]> + Clone>(
    index: &XmlIndex<W>,
    text: &[u8],
    offset: usize,
) -> Option<LocateResult> {
    let bp_pos = find_node_at_offset(index, text, offset)?;
    let expression = path_to_bp(index, text, bp_pos)?;

    let cursor = XmlCursor::from_bp_position(index, text, bp_pos);
    let value = cursor.value();
    let byte_range = cursor
        .value_byte_range()
        .unwrap_or_else(|| (cursor.text_position().unwrap_or(0), text.len()));

    Some(LocateResult {
        expression,
        byte_range,
        value_type: value.type_name(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xml::light::XmlIndex;

    #[test]
    fn locate_nested_element() {
        let xml = b"<root><foo><bar>x</bar></foo></root>";
        let index = XmlIndex::build(xml).unwrap();
        let offset = xml.iter().position(|&b| b == b'x').unwrap();
        let expr = locate_offset(&index, xml, offset).unwrap();
        assert_eq!(expr, ".foo.bar[\"+content\"]");
    }

    #[test]
    fn locate_attribute() {
        let xml = br#"<root id="42"/>"#;
        let index = XmlIndex::build(xml).unwrap();
        let offset = xml.iter().position(|&b| b == b'4').unwrap();
        let detailed = locate_offset_detailed(&index, xml, offset).unwrap();
        assert_eq!(detailed.expression, ".[\"+@id\"]");
        assert_eq!(detailed.value_type, "string");
        assert_eq!(&xml[detailed.byte_range.0..detailed.byte_range.1], b"42");
    }

    #[test]
    fn locate_root_is_dot() {
        let xml = b"<root>x</root>";
        let index = XmlIndex::build(xml).unwrap();
        let offset = xml.iter().position(|&b| b == b'r').unwrap();
        let expr = locate_offset(&index, xml, offset).unwrap();
        assert_eq!(expr, ".");
    }
}
