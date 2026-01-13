//! JSON path location utilities.
//!
//! This module provides functionality to find the jq expression that navigates
//! to a specific byte offset or line/column position in a JSON document.

#[cfg(not(test))]
use alloc::{
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};

#[cfg(test)]
use std::vec;

use crate::bits::BitVec;
use crate::json::light::{JsonCursor, JsonIndex, StandardJson};
use crate::RankSelect;

// ============================================================================
// NewlineIndex: Fast line/column to byte offset conversion
// ============================================================================

/// Index for fast line/column to byte offset conversion.
///
/// Handles Unix (LF), Windows (CRLF), and classic Mac (CR) line endings.
#[derive(Debug)]
pub struct NewlineIndex {
    /// Bitvector with 1s at positions immediately after line terminators
    /// (i.e., the first byte of each line after line 1).
    line_starts: BitVec,
    /// Total length of the text
    text_len: usize,
}

impl NewlineIndex {
    /// Build a newline index from text.
    ///
    /// Scans the text once to find all line terminators and marks
    /// the start position of each subsequent line.
    pub fn build(text: &[u8]) -> Self {
        if text.is_empty() {
            return Self {
                line_starts: BitVec::new(),
                text_len: 0,
            };
        }

        let mut bits = vec![0u64; text.len().div_ceil(64)];
        let mut i = 0;

        while i < text.len() {
            match text[i] {
                b'\n' => {
                    // LF: next byte starts a new line
                    let next = i + 1;
                    if next < text.len() {
                        bits[next / 64] |= 1 << (next % 64);
                    }
                    i += 1;
                }
                b'\r' => {
                    // CR: check for CRLF
                    let next = if i + 1 < text.len() && text[i + 1] == b'\n' {
                        // CRLF: skip both, new line starts after \n
                        i + 2
                    } else {
                        // Standalone CR (classic Mac): new line starts after \r
                        i + 1
                    };
                    if next < text.len() {
                        bits[next / 64] |= 1 << (next % 64);
                    }
                    i = next;
                }
                _ => i += 1,
            }
        }

        Self {
            line_starts: BitVec::from_words(bits, text.len()),
            text_len: text.len(),
        }
    }

    /// Convert 1-indexed line and column to byte offset.
    ///
    /// Column is 1-indexed byte offset within the line.
    /// Returns `None` if line/column is 0 or if the position is out of bounds.
    pub fn to_offset(&self, line: usize, column: usize) -> Option<usize> {
        if line == 0 || column == 0 {
            return None;
        }

        let line_start = if line == 1 {
            // First line starts at offset 0
            0
        } else {
            // Line N starts at the (N-1)th line-start marker (0-indexed select)
            self.line_starts.select1(line - 2)?
        };

        let offset = line_start + column - 1;
        if offset < self.text_len {
            Some(offset)
        } else {
            None
        }
    }

    /// Convert byte offset to 1-indexed line and column.
    ///
    /// Useful for error reporting and display.
    pub fn to_line_column(&self, offset: usize) -> (usize, usize) {
        if self.text_len == 0 {
            return (1, 1);
        }

        // Line-start markers are at positions immediately after line terminators.
        // rank1(offset + 1) gives the count of line-start markers in [0, offset],
        // which equals the number of lines before the current line.
        // So line number = 1 + rank1(offset + 1).
        let markers_before_or_at = self.line_starts.rank1(offset + 1);
        let line = 1 + markers_before_or_at;

        // Column = offset - start of this line + 1
        let line_start = if line == 1 {
            0
        } else {
            // The start of line N is at the (N-2)th marker (0-indexed select)
            // because line 1 has no marker, line 2 has marker 0, line 3 has marker 1, etc.
            self.line_starts.select1(line - 2).unwrap_or(0)
        };

        let column = offset - line_start + 1;
        (line, column)
    }
}

// ============================================================================
// Path building utilities
// ============================================================================

/// A component in a jq path expression.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PathComponent {
    /// Array index: `[0]`, `[1]`, etc.
    Index(usize),
    /// Object key that can use dot notation: `.foo`
    DotKey(String),
    /// Object key that needs bracket notation: `["foo-bar"]`
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

/// Check if a key can use dot notation in jq.
///
/// Keys must start with a letter or underscore, and contain only
/// alphanumeric characters and underscores.
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

/// Escape a string for use in jq bracket notation.
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

/// Convert an IB index (which structural element) to a BP position.
///
/// The BP position for the k-th structural element is the position of the
/// k-th open parenthesis in the BP bitvector.
fn ib_index_to_bp_pos<W: AsRef<[u64]>>(index: &JsonIndex<W>, ib_idx: usize) -> Option<usize> {
    // We need to find the BP position where rank1(bp_pos) = ib_idx
    // and the bit at bp_pos is 1 (open paren).
    //
    // Use binary search: find smallest bp_pos where rank1(bp_pos + 1) > ib_idx
    let bp = index.bp();
    let bp_len = bp.len();

    if bp_len == 0 {
        return None;
    }

    // Binary search for the position
    let mut lo = 0;
    let mut hi = bp_len;

    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        // rank1(mid + 1) = count of opens in [0, mid]
        let count = bp.rank1(mid + 1);
        if count <= ib_idx {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }

    // lo is now the smallest position where rank1(lo + 1) > ib_idx
    // This means rank1(lo) <= ib_idx and rank1(lo + 1) > ib_idx
    // So the (ib_idx)-th open is at position lo

    if lo < bp_len && bp.rank1(lo + 1) == ib_idx + 1 {
        Some(lo)
    } else {
        None
    }
}

/// Find the BP position of the structural node containing a byte offset.
///
/// Returns `None` if the offset is out of bounds or in invalid position.
fn find_node_at_offset<W: AsRef<[u64]>>(
    index: &JsonIndex<W>,
    text: &[u8],
    offset: usize,
) -> Option<usize> {
    if offset >= text.len() {
        return None;
    }

    // Get the rank at this position (count of structural bits before offset)
    let rank = index.ib_rank1(offset);

    // Determine which IB index contains this offset
    let ib_idx = if let Some(struct_pos) = index.ib_select1(rank) {
        if struct_pos == offset {
            // We're exactly at a structural position
            rank
        } else {
            // We're inside a value - the containing node started at rank-1
            if rank > 0 {
                rank - 1
            } else {
                return None;
            }
        }
    } else if rank > 0 {
        rank - 1
    } else {
        return None;
    };

    // Convert IB index to BP position
    ib_index_to_bp_pos(index, ib_idx)
}

/// Count siblings before target_bp in a container.
fn count_siblings_before<W: AsRef<[u64]>>(
    index: &JsonIndex<W>,
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
    index: &JsonIndex<W>,
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

/// Find the key BP position for a value in an object.
///
/// Returns `(key_bp, value_bp)` for the key-value pair containing target_bp.
fn find_key_for_value<W: AsRef<[u64]>>(
    index: &JsonIndex<W>,
    object_bp: usize,
    target_bp: usize,
) -> Option<(usize, usize)> {
    let mut child_bp = index.bp().first_child(object_bp)?;

    loop {
        let key_bp = child_bp;
        let value_bp = index.bp().next_sibling(key_bp)?;

        // Check if target is the key itself
        if key_bp == target_bp {
            return Some((key_bp, value_bp));
        }

        // Check if target is the value or within value's subtree
        if value_bp == target_bp || is_ancestor(index, value_bp, target_bp) {
            return Some((key_bp, value_bp));
        }

        // Move to next key-value pair
        child_bp = index.bp().next_sibling(value_bp)?;
    }
}

/// Extract the key string from a key cursor.
fn extract_key_string<W: AsRef<[u64]>>(cursor: JsonCursor<'_, W>, _text: &[u8]) -> Option<String> {
    match cursor.value() {
        StandardJson::String(s) => s.as_str().ok().map(|cow| cow.into_owned()),
        _ => None,
    }
}

/// Build the jq path expression from root to a specific BP position.
///
/// Returns `None` if the path cannot be determined.
pub fn path_to_bp<W: AsRef<[u64]>>(
    index: &JsonIndex<W>,
    text: &[u8],
    target_bp: usize,
) -> Option<String> {
    let mut components: Vec<PathComponent> = Vec::new();
    let mut current_bp = target_bp;

    // Walk up from target to root using parent()
    while let Some(parent_bp) = index.bp().parent(current_bp) {
        // Create a cursor at the parent to check its type
        let parent_cursor = JsonCursor::from_bp_position(index, text, parent_bp);
        let parent_pos = parent_cursor.text_position()?;

        match text.get(parent_pos)? {
            b'[' => {
                // Parent is an array - find index of current among siblings
                let idx = count_siblings_before(index, parent_bp, current_bp);
                components.push(PathComponent::Index(idx));
            }
            b'{' => {
                // Parent is an object - find which key-value pair contains current
                let (key_bp, _value_bp) = find_key_for_value(index, parent_bp, current_bp)?;

                // Extract the key string
                let key_cursor = JsonCursor::from_bp_position(index, text, key_bp);
                let key = extract_key_string(key_cursor, text)?;

                // Determine notation
                if can_use_dot_notation(&key) {
                    components.push(PathComponent::DotKey(key));
                } else {
                    components.push(PathComponent::BracketKey(key));
                }
            }
            _ => {
                // Shouldn't happen - parent should be a container
                break;
            }
        }

        current_bp = parent_bp;
    }

    // Reverse to get root-to-target order
    components.reverse();

    // Build the final expression
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

/// Find the jq expression for a byte offset in JSON text.
///
/// Returns `None` if the offset is invalid or the path cannot be determined.
pub fn locate_offset<W: AsRef<[u64]>>(
    index: &JsonIndex<W>,
    text: &[u8],
    offset: usize,
) -> Option<String> {
    let bp_pos = find_node_at_offset(index, text, offset)?;
    path_to_bp(index, text, bp_pos)
}

/// Result of locating a position in JSON.
#[derive(Debug, Clone)]
pub struct LocateResult {
    /// The jq expression to navigate to this position
    pub expression: String,
    /// The byte range of the value in the original text
    pub byte_range: (usize, usize),
    /// The type of the JSON value
    pub value_type: &'static str,
}

/// Find detailed location info for a byte offset in JSON text.
pub fn locate_offset_detailed<W: AsRef<[u64]>>(
    index: &JsonIndex<W>,
    text: &[u8],
    offset: usize,
) -> Option<LocateResult> {
    let bp_pos = find_node_at_offset(index, text, offset)?;
    let expression = path_to_bp(index, text, bp_pos)?;

    let cursor = JsonCursor::from_bp_position(index, text, bp_pos);

    // text_range may return None for the root element (known issue with container close detection)
    // Fall back to (start, text.len()) for root
    let byte_range = cursor.text_range().unwrap_or_else(|| {
        let start = cursor.text_position().unwrap_or(0);
        (start, text.len())
    });

    let value_type = match cursor.value() {
        StandardJson::Object(_) => "object",
        StandardJson::Array(_) => "array",
        StandardJson::String(_) => "string",
        StandardJson::Number(_) => "number",
        StandardJson::Bool(_) => "boolean",
        StandardJson::Null => "null",
        StandardJson::Error(_) => "error",
    };

    Some(LocateResult {
        expression,
        byte_range,
        value_type,
    })
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    // NewlineIndex tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_newline_index_empty() {
        let index = NewlineIndex::build(b"");
        // Empty string has no valid positions
        assert_eq!(index.to_offset(1, 1), None);
        // But if we ask for line/column of offset 0, it's (1, 1)
        assert_eq!(index.to_line_column(0), (1, 1));
    }

    #[test]
    fn test_newline_index_no_newlines() {
        let text = b"hello world";
        let index = NewlineIndex::build(text);

        assert_eq!(index.to_offset(1, 1), Some(0));
        assert_eq!(index.to_offset(1, 6), Some(5)); // space
        assert_eq!(index.to_offset(1, 12), None); // past end
        assert_eq!(index.to_offset(2, 1), None); // no line 2

        assert_eq!(index.to_line_column(0), (1, 1));
        assert_eq!(index.to_line_column(5), (1, 6));
    }

    #[test]
    fn test_newline_index_unix_lf() {
        let text = b"line1\nline2\nline3";
        let index = NewlineIndex::build(text);

        // Line 1: positions 0-4 (line1)
        assert_eq!(index.to_offset(1, 1), Some(0));
        assert_eq!(index.to_offset(1, 5), Some(4));

        // Line 2: positions 6-10 (line2)
        assert_eq!(index.to_offset(2, 1), Some(6));
        assert_eq!(index.to_offset(2, 5), Some(10));

        // Line 3: positions 12-16 (line3)
        assert_eq!(index.to_offset(3, 1), Some(12));
        assert_eq!(index.to_offset(3, 5), Some(16));

        // Reverse lookup
        assert_eq!(index.to_line_column(0), (1, 1));
        assert_eq!(index.to_line_column(5), (1, 6)); // the \n
        assert_eq!(index.to_line_column(6), (2, 1));
        assert_eq!(index.to_line_column(12), (3, 1));
    }

    #[test]
    fn test_newline_index_windows_crlf() {
        let text = b"line1\r\nline2\r\nline3";
        let index = NewlineIndex::build(text);

        // Line 1: positions 0-4 (line1)
        assert_eq!(index.to_offset(1, 1), Some(0));
        assert_eq!(index.to_offset(1, 5), Some(4));

        // Line 2: positions 7-11 (line2) - after \r\n
        assert_eq!(index.to_offset(2, 1), Some(7));
        assert_eq!(index.to_offset(2, 5), Some(11));

        // Line 3: positions 14-18 (line3)
        assert_eq!(index.to_offset(3, 1), Some(14));

        // Reverse lookup
        assert_eq!(index.to_line_column(0), (1, 1));
        assert_eq!(index.to_line_column(7), (2, 1));
        assert_eq!(index.to_line_column(14), (3, 1));
    }

    #[test]
    fn test_newline_index_classic_mac_cr() {
        let text = b"line1\rline2\rline3";
        let index = NewlineIndex::build(text);

        // Line 1: positions 0-4 (line1)
        assert_eq!(index.to_offset(1, 1), Some(0));

        // Line 2: positions 6-10 (line2) - after \r
        assert_eq!(index.to_offset(2, 1), Some(6));

        // Line 3: positions 12-16 (line3)
        assert_eq!(index.to_offset(3, 1), Some(12));

        // Reverse lookup
        assert_eq!(index.to_line_column(6), (2, 1));
    }

    #[test]
    fn test_newline_index_invalid_inputs() {
        let index = NewlineIndex::build(b"hello\nworld");

        assert_eq!(index.to_offset(0, 1), None); // line 0
        assert_eq!(index.to_offset(1, 0), None); // column 0
    }

    // ------------------------------------------------------------------------
    // Path notation tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_can_use_dot_notation() {
        assert!(can_use_dot_notation("foo"));
        assert!(can_use_dot_notation("_bar"));
        assert!(can_use_dot_notation("foo123"));
        assert!(can_use_dot_notation("foo_bar"));

        assert!(!can_use_dot_notation("")); // empty
        assert!(!can_use_dot_notation("123")); // starts with digit
        assert!(!can_use_dot_notation("foo-bar")); // contains hyphen
        assert!(!can_use_dot_notation("foo.bar")); // contains dot
        assert!(!can_use_dot_notation("foo bar")); // contains space
    }

    #[test]
    fn test_escape_jq_string() {
        assert_eq!(escape_jq_string("hello"), "hello");
        assert_eq!(escape_jq_string("hello\"world"), "hello\\\"world");
        assert_eq!(escape_jq_string("back\\slash"), "back\\\\slash");
        assert_eq!(escape_jq_string("new\nline"), "new\\nline");
    }

    // ------------------------------------------------------------------------
    // Location tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_locate_simple_object() {
        let json = br#"{"name": "Alice"}"#;
        let index = JsonIndex::build(json);

        // At the opening brace - root
        assert_eq!(locate_offset(&index, json, 0), Some(".".to_string()));

        // At "name" key
        assert_eq!(locate_offset(&index, json, 1), Some(".name".to_string()));

        // Inside "Alice" value
        assert_eq!(locate_offset(&index, json, 10), Some(".name".to_string()));
    }

    #[test]
    fn test_locate_nested_array() {
        let json = br#"{"users": [{"name": "Bob"}]}"#;
        let index = JsonIndex::build(json);

        // At users array
        assert_eq!(locate_offset(&index, json, 10), Some(".users".to_string()));

        // At first object in array
        assert_eq!(
            locate_offset(&index, json, 11),
            Some(".users[0]".to_string())
        );

        // At name in first object
        assert_eq!(
            locate_offset(&index, json, 12),
            Some(".users[0].name".to_string())
        );
    }

    #[test]
    fn test_locate_array_indices() {
        let json = br#"[1, 2, 3]"#;
        let index = JsonIndex::build(json);

        // At array
        assert_eq!(locate_offset(&index, json, 0), Some(".".to_string()));

        // At first element (1)
        assert_eq!(locate_offset(&index, json, 1), Some(".[0]".to_string()));

        // At second element (2)
        assert_eq!(locate_offset(&index, json, 4), Some(".[1]".to_string()));

        // At third element (3)
        assert_eq!(locate_offset(&index, json, 7), Some(".[2]".to_string()));
    }

    #[test]
    fn test_locate_special_key() {
        let json = br#"{"foo-bar": 1, "with space": 2}"#;
        let index = JsonIndex::build(json);

        // At foo-bar (needs bracket notation)
        assert_eq!(
            locate_offset(&index, json, 1),
            Some(r#".["foo-bar"]"#.to_string())
        );

        // At "with space"
        assert_eq!(
            locate_offset(&index, json, 15),
            Some(r#".["with space"]"#.to_string())
        );
    }

    #[test]
    fn test_locate_deeply_nested() {
        let json = br#"{"a": {"b": {"c": 42}}}"#;
        let index = JsonIndex::build(json);

        // At innermost value
        assert_eq!(locate_offset(&index, json, 18), Some(".a.b.c".to_string()));
    }

    #[test]
    fn test_locate_large_array_deep_in_tree() {
        // Regression test for a bug where enclose() would skip words when the
        // word_excess was 0 (equal opens and closes) even though the enclosing
        // parent was within that word. The fix uses word_max_excess_rev() to
        // correctly identify words that might contain the enclosing parent.
        //
        // This test creates a structure where:
        // - There's a deeply nested array with many elements
        // - The array's opening bracket is in word N
        // - Some elements are at positions where parent() must scan back through
        //   a word with equal opens/closes to find the array
        let json = br#"{"outer":{"inner":[0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32,33,34,35,36,37,38,39,40,41,42,43,44,45,46,47,48,49]}}"#;
        let index = JsonIndex::build(json);

        // Test locating elements at various positions in the array
        // Element 0 is at the start
        let elem0_pos = json.windows(1).position(|w| w == b"0").unwrap();
        assert_eq!(
            locate_offset(&index, json, elem0_pos),
            Some(".outer.inner[0]".to_string())
        );

        // Element 49 is near the end - this is the one most likely to trigger
        // the bug as it's furthest from the array's opening bracket
        let elem49_start = json
            .windows(4)
            .position(|w| w == b",49]")
            .map(|p| p + 1)
            .unwrap();
        assert_eq!(
            locate_offset(&index, json, elem49_start),
            Some(".outer.inner[49]".to_string())
        );
    }

    #[test]
    fn test_locate_offset_at_word_boundary() {
        // Test with a JSON structure that causes positions to cross 64-bit word boundaries
        // Word 0: bits 0-63, Word 1: bits 64-127, Word 2: bits 128-191, etc.
        //
        // Create a structure with many elements so that elements span multiple words
        // in the BP (balanced parentheses) index
        let mut json = String::from(r#"{"data":["#);
        for i in 0..100 {
            if i > 0 {
                json.push(',');
            }
            json.push_str(&format!("\"item{}\"", i));
        }
        json.push_str("]}");

        let json_bytes = json.as_bytes();
        let index = JsonIndex::build(json_bytes);

        // Test that we can locate elements throughout the array
        // Earlier elements
        let item5_pos = json.find("\"item5\"").unwrap();
        assert_eq!(
            locate_offset(&index, json_bytes, item5_pos),
            Some(".data[5]".to_string())
        );

        // Middle elements
        let item50_pos = json.find("\"item50\"").unwrap();
        assert_eq!(
            locate_offset(&index, json_bytes, item50_pos),
            Some(".data[50]".to_string())
        );

        // Late elements
        let item99_pos = json.find("\"item99\"").unwrap();
        assert_eq!(
            locate_offset(&index, json_bytes, item99_pos),
            Some(".data[99]".to_string())
        );
    }

    #[test]
    fn test_locate_enclose_word_boundary_regression() {
        // Regression test for a bug in bp.rs enclose() where words with word_excess=0
        // (equal opens and closes) were incorrectly skipped even when they contained
        // the enclosing parent.
        //
        // The bug was originally found at byte offset 956 in a JSON file with specific
        // structure. This test uses a JSON structure that creates enough BP positions
        // to span multiple 64-bit words with the problematic pattern.
        //
        // The fix is in enclose() in bp.rs - this test ensures locate_offset() works
        // correctly with structures that exercise that code path.

        // This JSON is based on a minimal reproduction of the original bug.
        // It has:
        // - A complex nested structure creating many BP positions
        // - Multiple arrays and objects to fill BP words
        // - Elements at positions that span word boundaries
        let json = include_str!("../../tests/testdata/locate_regression.json");

        let json_bytes = json.as_bytes();
        let index = JsonIndex::build(json_bytes);

        // Test offset 956 which failed in the original bug
        let result = locate_offset(&index, json_bytes, 956);
        assert!(
            result.is_some(),
            "Failed to locate offset 956 - this indicates the enclose() word boundary bug"
        );

        // The path should be in the strings array (element 10 in original file)
        let path = result.unwrap();
        assert!(
            path.starts_with(".strings["),
            "Expected path starting with '.strings[', got: {path}"
        );

        // Test more offsets that were affected by the bug
        for offset in [1000, 1500, 2000, 2500] {
            if offset < json.len() {
                let result = locate_offset(&index, json_bytes, offset);
                assert!(
                    result.is_some(),
                    "Failed to locate offset {offset} - may indicate enclose() word boundary bug"
                );
            }
        }
    }
}
