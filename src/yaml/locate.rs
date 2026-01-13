//! YAML path location utilities.
//!
//! This module provides functionality to find the jq-like expression that
//! navigates to a specific byte offset in a YAML document.

#[cfg(not(test))]
use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

use super::index::YamlIndex;
use super::light::{YamlCursor, YamlValue};

// ============================================================================
// Path building utilities
// ============================================================================

/// A component in a path expression.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PathComponent {
    /// Sequence index: `[0]`, `[1]`, etc.
    Index(usize),
    /// Mapping key that can use dot notation: `.foo`
    DotKey(String),
    /// Mapping key that needs bracket notation: `["foo-bar"]`
    BracketKey(String),
}

impl PathComponent {
    /// Format as a jq path component.
    fn to_jq_string(&self) -> String {
        match self {
            PathComponent::Index(i) => format!("[{}]", i),
            PathComponent::DotKey(k) => format!(".{}", k),
            PathComponent::BracketKey(k) => format!("[\"{}\"]", escape_jq_string(k)),
        }
    }
}

/// Check if a key can use dot notation.
fn can_use_dot_notation(key: &str) -> bool {
    if key.is_empty() {
        return false;
    }

    let mut chars = key.chars();
    let first = chars.next().unwrap();

    if !first.is_alphabetic() && first != '_' {
        return false;
    }

    chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// Escape a string for use in bracket notation.
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

/// Convert an IB index to a BP position.
fn ib_index_to_bp_pos<W: AsRef<[u64]>>(index: &YamlIndex<W>, ib_idx: usize) -> Option<usize> {
    let bp = index.bp();
    let bp_len = bp.len();

    if bp_len == 0 {
        return None;
    }

    let mut lo = 0;
    let mut hi = bp_len;

    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let count = bp.rank1(mid + 1);
        if count <= ib_idx {
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

/// Find the BP position of the structural node containing a byte offset.
fn find_node_at_offset<W: AsRef<[u64]>>(
    index: &YamlIndex<W>,
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

    ib_index_to_bp_pos(index, ib_idx)
}

/// Count siblings before target_bp in a container.
fn count_siblings_before<W: AsRef<[u64]>>(
    index: &YamlIndex<W>,
    container_bp: usize,
    target_bp: usize,
) -> usize {
    let mut count = 0;
    let mut child_bp = match index.bp().first_child(container_bp) {
        Some(c) => c,
        None => return 0,
    };

    while child_bp < target_bp {
        count += 1;
        child_bp = match index.bp().next_sibling(child_bp) {
            Some(c) => c,
            None => break,
        };
    }

    count
}

/// Check if ancestor_bp's subtree contains descendant_bp.
fn is_ancestor<W: AsRef<[u64]>>(
    index: &YamlIndex<W>,
    ancestor_bp: usize,
    descendant_bp: usize,
) -> bool {
    if ancestor_bp >= descendant_bp {
        return false;
    }

    match index.bp().find_close(ancestor_bp) {
        Some(close) => descendant_bp < close,
        None => false,
    }
}

/// Find the key BP position for a value in a mapping.
fn find_key_for_value<W: AsRef<[u64]>>(
    index: &YamlIndex<W>,
    mapping_bp: usize,
    target_bp: usize,
) -> Option<(usize, usize)> {
    let mut child_bp = index.bp().first_child(mapping_bp)?;

    loop {
        let key_bp = child_bp;
        let value_bp = index.bp().next_sibling(key_bp)?;

        if key_bp == target_bp {
            return Some((key_bp, value_bp));
        }

        if value_bp == target_bp || is_ancestor(index, value_bp, target_bp) {
            return Some((key_bp, value_bp));
        }

        child_bp = index.bp().next_sibling(value_bp)?;
    }
}

/// Extract the key string from a key cursor.
fn extract_key_string<W: AsRef<[u64]>>(cursor: YamlCursor<'_, W>) -> Option<String> {
    match cursor.value() {
        YamlValue::String(s) => s.as_str().ok().map(|cow| cow.into_owned()),
        _ => None,
    }
}

/// Check if a container is a sequence (vs mapping).
fn is_sequence<W: AsRef<[u64]>>(index: &YamlIndex<W>, text: &[u8], bp_pos: usize) -> bool {
    let cursor = YamlCursor::new(index, text, bp_pos);
    if let Some(pos) = cursor.text_position() {
        if pos < text.len() {
            return text[pos] == b'-';
        }
    }
    false
}

/// Build the path expression from root to a specific BP position.
pub fn path_to_bp<W: AsRef<[u64]>>(
    index: &YamlIndex<W>,
    text: &[u8],
    target_bp: usize,
) -> Option<String> {
    let mut components: Vec<PathComponent> = Vec::new();
    let mut current_bp = target_bp;

    while let Some(parent_bp) = index.bp().parent(current_bp) {
        if is_sequence(index, text, parent_bp) {
            // Parent is a sequence
            let idx = count_siblings_before(index, parent_bp, current_bp);
            components.push(PathComponent::Index(idx));
        } else {
            // Parent is a mapping
            let (key_bp, _value_bp) = find_key_for_value(index, parent_bp, current_bp)?;
            let key_cursor = YamlCursor::new(index, text, key_bp);
            let key = extract_key_string(key_cursor)?;

            if can_use_dot_notation(&key) {
                components.push(PathComponent::DotKey(key));
            } else {
                components.push(PathComponent::BracketKey(key));
            }
        }

        current_bp = parent_bp;
    }

    components.reverse();

    if components.is_empty() {
        Some(".".to_string())
    } else {
        let mut result = String::new();
        for (i, comp) in components.iter().enumerate() {
            match comp {
                PathComponent::Index(_) | PathComponent::BracketKey(_) => {
                    if i == 0 {
                        result.push('.');
                    }
                    result.push_str(&comp.to_jq_string());
                }
                PathComponent::DotKey(_) => {
                    result.push_str(&comp.to_jq_string());
                }
            }
        }
        Some(result)
    }
}

/// Find the jq-like expression for a byte offset in YAML text.
///
/// Returns `None` if the offset is invalid or the path cannot be determined.
///
/// # Example
///
/// ```ignore
/// let yaml = b"person:\n  name: Alice\n  age: 30";
/// let index = YamlIndex::build(yaml)?;
///
/// // Find path to "Alice"
/// let path = locate_offset(&index, yaml, 18);
/// assert_eq!(path, Some(".person.name".to_string()));
/// ```
pub fn locate_offset<W: AsRef<[u64]>>(
    index: &YamlIndex<W>,
    text: &[u8],
    offset: usize,
) -> Option<String> {
    let bp_pos = find_node_at_offset(index, text, offset)?;
    path_to_bp(index, text, bp_pos)
}

/// Result of locating a position in YAML.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct LocateResult {
    /// The jq-like expression to navigate to this position
    pub expression: String,
    /// The byte range of the value in the original text
    pub byte_range: (usize, usize),
    /// The type of the YAML value
    pub value_type: &'static str,
}

/// Find detailed location info for a byte offset in YAML text.
#[allow(dead_code)]
pub fn locate_offset_detailed<W: AsRef<[u64]>>(
    index: &YamlIndex<W>,
    text: &[u8],
    offset: usize,
) -> Option<LocateResult> {
    let bp_pos = find_node_at_offset(index, text, offset)?;
    let expression = path_to_bp(index, text, bp_pos)?;

    let cursor = YamlCursor::new(index, text, bp_pos);

    let byte_range = if let Some(bytes) = cursor.raw_bytes() {
        let start = cursor.text_position().unwrap_or(0);
        (start, start + bytes.len())
    } else {
        let start = cursor.text_position().unwrap_or(0);
        (start, text.len())
    };

    let value_type = match cursor.value() {
        YamlValue::Mapping(_) => "mapping",
        YamlValue::Sequence(_) => "sequence",
        YamlValue::String(_) => "string",
        YamlValue::Error(_) => "error",
    };

    Some(LocateResult {
        expression,
        byte_range,
        value_type,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yaml::YamlIndex;

    #[test]
    fn test_locate_simple_mapping() {
        let yaml = b"name: Alice";
        let index = YamlIndex::build(yaml).unwrap();

        // Offset 0 should be at root
        let path = locate_offset(&index, yaml, 0);
        // May return . or .name depending on BP structure
        assert!(path.is_some());
    }

    #[test]
    fn test_can_use_dot_notation() {
        assert!(can_use_dot_notation("name"));
        assert!(can_use_dot_notation("_private"));
        assert!(can_use_dot_notation("name123"));
        assert!(!can_use_dot_notation("123name"));
        assert!(!can_use_dot_notation("name-with-dash"));
        assert!(!can_use_dot_notation(""));
    }

    #[test]
    fn test_escape_jq_string() {
        assert_eq!(escape_jq_string("hello"), "hello");
        assert_eq!(escape_jq_string("hello\"world"), "hello\\\"world");
        assert_eq!(escape_jq_string("line1\nline2"), "line1\\nline2");
    }
}
