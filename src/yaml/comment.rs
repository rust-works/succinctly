//! Comment and tag tracking for YAML semi-indexing.
//!
//! This module provides efficient sparse tracking of comments and explicit tags
//! in YAML documents. The design is optimized for typical YAML files where
//! comments are sparse (0-5% of lines for CI/K8s, up to 37% for Helm values).
//!
//! # Design Principles
//!
//! - **Sparse storage**: Only nodes with comments/tags have entries
//! - **Cache-friendly**: Sorted vectors with contiguous memory
//! - **Adaptive indexing**: Optional superblock index for files with many comments
//! - **16-byte aligned entries**: 4 entries per cache line

#[cfg(not(test))]
use alloc::{string::String, vec::Vec};

/// Type of comment relative to its owning node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CommentKind {
    /// Comment appearing above the node (head comment)
    Head = 0,
    /// Comment appearing on the same line as the node (line comment)
    Line = 1,
    /// Comment appearing below the node (foot comment)
    Foot = 2,
}

/// A single comment entry in the index.
///
/// Packed to 16 bytes for cache-line alignment (4 entries per 64-byte cache line).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct CommentEntry {
    /// BP position of the node this comment belongs to
    pub node_bp: u32,
    /// Byte offset of comment start in source (including `#`)
    pub start: u32,
    /// Byte offset of comment end in source (excluding newline)
    pub end: u32,
    /// Comment type (head/line/foot)
    pub kind: CommentKind,
    /// Padding for alignment
    _pad: [u8; 3],
}

impl CommentEntry {
    /// Create a new comment entry.
    #[inline]
    pub fn new(node_bp: u32, start: u32, end: u32, kind: CommentKind) -> Self {
        Self {
            node_bp,
            start,
            end,
            kind,
            _pad: [0; 3],
        }
    }

    /// Get the comment text from the source.
    #[inline]
    pub fn text<'a>(&self, source: &'a [u8]) -> &'a [u8] {
        &source[self.start as usize..self.end as usize]
    }

    /// Get the comment text without the leading `#` and optional space.
    #[inline]
    pub fn text_content<'a>(&self, source: &'a [u8]) -> &'a [u8] {
        let raw = self.text(source);
        if raw.is_empty() {
            return raw;
        }
        // Skip leading `#`
        let after_hash = if raw[0] == b'#' { &raw[1..] } else { raw };
        // Skip optional leading space
        if !after_hash.is_empty() && after_hash[0] == b' ' {
            &after_hash[1..]
        } else {
            after_hash
        }
    }
}

/// Sparse comment index for YAML documents.
///
/// Stores comments in a sorted vector for O(log n) lookup by BP position.
/// For files with many comments (>100), an optional superblock index
/// enables O(1) approximate lookup followed by short linear scan.
#[derive(Debug, Clone, Default)]
pub struct CommentIndex {
    /// Comments sorted by node_bp for binary search
    entries: Vec<CommentEntry>,

    /// Optional sparse index for files with many comments.
    /// Entry i = first comment index for BP positions in range [i*64, (i+1)*64).
    /// Only built when entries.len() > SUPERBLOCK_THRESHOLD.
    superblock_idx: Option<Vec<u32>>,
}

/// Threshold for building superblock index (number of comments)
const SUPERBLOCK_THRESHOLD: usize = 100;

/// Superblock size (BP positions per superblock entry)
const SUPERBLOCK_SIZE: usize = 64;

impl CommentIndex {
    /// Create a new empty comment index.
    #[inline]
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            superblock_idx: None,
        }
    }

    /// Create a comment index with pre-allocated capacity.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity),
            superblock_idx: None,
        }
    }

    /// Add a comment entry. Entries should be added in BP position order.
    #[inline]
    pub fn push(&mut self, entry: CommentEntry) {
        self.entries.push(entry);
    }

    /// Finalize the index after all entries have been added.
    ///
    /// This sorts entries by BP position and optionally builds
    /// a superblock index for faster lookup.
    pub fn finalize(&mut self, max_bp_pos: usize) {
        // Sort by BP position (should already be mostly sorted)
        self.entries.sort_by_key(|e| e.node_bp);

        // Build superblock index if we have many comments
        if self.entries.len() > SUPERBLOCK_THRESHOLD {
            self.build_superblock_index(max_bp_pos);
        }
    }

    /// Build the superblock index for O(1) approximate lookup.
    fn build_superblock_index(&mut self, max_bp_pos: usize) {
        let num_superblocks = max_bp_pos.div_ceil(SUPERBLOCK_SIZE) + 1;
        let mut idx = vec![0u32; num_superblocks + 1];

        let mut entry_idx = 0u32;
        for superblock in 0..num_superblocks {
            let superblock_start = (superblock * SUPERBLOCK_SIZE) as u32;
            // Advance entry_idx to first entry in or after this superblock
            while (entry_idx as usize) < self.entries.len()
                && self.entries[entry_idx as usize].node_bp < superblock_start
            {
                entry_idx += 1;
            }
            idx[superblock] = entry_idx;
        }
        idx[num_superblocks] = self.entries.len() as u32;

        self.superblock_idx = Some(idx);
    }

    /// Get all comments for a specific BP position.
    #[inline]
    pub fn get(&self, bp_pos: u32) -> CommentIter<'_> {
        let (start, end) = self.find_range(bp_pos);
        CommentIter {
            entries: &self.entries[start..end],
            pos: 0,
        }
    }

    /// Get comments of a specific kind for a BP position.
    #[inline]
    pub fn get_kind(&self, bp_pos: u32, kind: CommentKind) -> impl Iterator<Item = &CommentEntry> {
        self.get(bp_pos).filter(move |e| e.kind == kind)
    }

    /// Get the head comment for a BP position (first head comment if multiple).
    #[inline]
    pub fn head_comment(&self, bp_pos: u32) -> Option<&CommentEntry> {
        self.get_kind(bp_pos, CommentKind::Head).next()
    }

    /// Get the line comment for a BP position.
    #[inline]
    pub fn line_comment(&self, bp_pos: u32) -> Option<&CommentEntry> {
        self.get_kind(bp_pos, CommentKind::Line).next()
    }

    /// Get the foot comment for a BP position.
    #[inline]
    pub fn foot_comment(&self, bp_pos: u32) -> Option<&CommentEntry> {
        self.get_kind(bp_pos, CommentKind::Foot).next()
    }

    /// Check if a BP position has any comments.
    #[inline]
    pub fn has_comments(&self, bp_pos: u32) -> bool {
        let (start, end) = self.find_range(bp_pos);
        start < end
    }

    /// Total number of comments in the index.
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the index is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Find the range of entries for a given BP position.
    fn find_range(&self, bp_pos: u32) -> (usize, usize) {
        if self.entries.is_empty() {
            return (0, 0);
        }

        // Use superblock index if available
        let search_range = if let Some(ref idx) = self.superblock_idx {
            let superblock = (bp_pos as usize) / SUPERBLOCK_SIZE;
            let start = if superblock < idx.len() {
                idx[superblock] as usize
            } else {
                self.entries.len()
            };
            let end = if superblock + 1 < idx.len() {
                idx[superblock + 1] as usize
            } else {
                self.entries.len()
            };
            start..end
        } else {
            0..self.entries.len()
        };

        // Binary search within range
        let slice = &self.entries[search_range.clone()];
        let offset = search_range.start;

        let first = slice.partition_point(|e| e.node_bp < bp_pos);
        let last = slice[first..].partition_point(|e| e.node_bp == bp_pos) + first;

        (offset + first, offset + last)
    }

    /// Iterate over all comments in the index.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = &CommentEntry> {
        self.entries.iter()
    }
}

/// Iterator over comments for a specific BP position.
#[derive(Debug)]
pub struct CommentIter<'a> {
    entries: &'a [CommentEntry],
    pos: usize,
}

impl<'a> Iterator for CommentIter<'a> {
    type Item = &'a CommentEntry;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.pos < self.entries.len() {
            let entry = &self.entries[self.pos];
            self.pos += 1;
            Some(entry)
        } else {
            None
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.entries.len() - self.pos;
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for CommentIter<'_> {}

// ============================================================================
// Tag Index
// ============================================================================

/// Sparse tag index for YAML documents.
///
/// Stores explicit tags (like `!!str`, `!custom`) for nodes that have them.
/// Most YAML nodes use implicit typing, so this is typically very sparse.
#[derive(Debug, Clone, Default)]
pub struct TagIndex {
    /// Tags sorted by BP position: (bp_pos, tag_string)
    entries: Vec<(u32, String)>,
}

impl TagIndex {
    /// Create a new empty tag index.
    #[inline]
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Add a tag entry. Entries should be added in BP position order.
    #[inline]
    pub fn push(&mut self, bp_pos: u32, tag: String) {
        self.entries.push((bp_pos, tag));
    }

    /// Finalize the index after all entries have been added.
    pub fn finalize(&mut self) {
        // Sort by BP position (should already be mostly sorted)
        self.entries.sort_by_key(|(bp, _)| *bp);
    }

    /// Get the explicit tag for a BP position.
    #[inline]
    pub fn get(&self, bp_pos: u32) -> Option<&str> {
        self.entries
            .binary_search_by_key(&bp_pos, |(bp, _)| *bp)
            .ok()
            .map(|idx| self.entries[idx].1.as_str())
    }

    /// Check if a BP position has an explicit tag.
    #[inline]
    pub fn has_tag(&self, bp_pos: u32) -> bool {
        self.entries
            .binary_search_by_key(&bp_pos, |(bp, _)| *bp)
            .is_ok()
    }

    /// Total number of tags in the index.
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the index is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate over all tags in the index.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = (u32, &str)> {
        self.entries.iter().map(|(bp, tag)| (*bp, tag.as_str()))
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_comment_entry_size() {
        // Verify 16-byte alignment
        assert_eq!(core::mem::size_of::<CommentEntry>(), 16);
    }

    #[test]
    fn test_empty_comment_index() {
        let index = CommentIndex::new();
        assert!(index.is_empty());
        assert_eq!(index.len(), 0);
        assert!(!index.has_comments(0));
        assert!(index.get(0).next().is_none());
    }

    #[test]
    fn test_single_comment() {
        let mut index = CommentIndex::new();
        index.push(CommentEntry::new(5, 10, 20, CommentKind::Line));
        index.finalize(10);

        assert_eq!(index.len(), 1);
        assert!(index.has_comments(5));
        assert!(!index.has_comments(4));
        assert!(!index.has_comments(6));

        let comments: Vec<_> = index.get(5).collect();
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].node_bp, 5);
        assert_eq!(comments[0].kind, CommentKind::Line);
    }

    #[test]
    fn test_multiple_comments_same_node() {
        let mut index = CommentIndex::new();
        index.push(CommentEntry::new(5, 0, 10, CommentKind::Head));
        index.push(CommentEntry::new(5, 15, 25, CommentKind::Line));
        index.push(CommentEntry::new(5, 30, 40, CommentKind::Foot));
        index.finalize(10);

        assert_eq!(index.len(), 3);

        let comments: Vec<_> = index.get(5).collect();
        assert_eq!(comments.len(), 3);

        assert!(index.head_comment(5).is_some());
        assert!(index.line_comment(5).is_some());
        assert!(index.foot_comment(5).is_some());
    }

    #[test]
    fn test_comments_different_nodes() {
        let mut index = CommentIndex::new();
        index.push(CommentEntry::new(3, 0, 10, CommentKind::Head));
        index.push(CommentEntry::new(7, 15, 25, CommentKind::Line));
        index.push(CommentEntry::new(15, 30, 40, CommentKind::Foot));
        index.finalize(20);

        assert!(index.has_comments(3));
        assert!(index.has_comments(7));
        assert!(index.has_comments(15));
        assert!(!index.has_comments(5));
        assert!(!index.has_comments(10));

        assert_eq!(index.get(3).count(), 1);
        assert_eq!(index.get(7).count(), 1);
        assert_eq!(index.get(15).count(), 1);
        assert_eq!(index.get(5).count(), 0);
    }

    #[test]
    fn test_comment_text_extraction() {
        let source = b"name: value # this is a comment\n";
        let entry = CommentEntry::new(5, 12, 31, CommentKind::Line);

        let text = entry.text(source);
        assert_eq!(text, b"# this is a comment");

        let content = entry.text_content(source);
        assert_eq!(content, b"this is a comment");
    }

    #[test]
    fn test_comment_text_no_space_after_hash() {
        let source = b"#comment without space";
        let entry = CommentEntry::new(0, 0, 22, CommentKind::Head);

        let content = entry.text_content(source);
        assert_eq!(content, b"comment without space");
    }

    #[test]
    fn test_superblock_index_threshold() {
        // Below threshold - no superblock index
        let mut small_index = CommentIndex::new();
        for i in 0..50 {
            small_index.push(CommentEntry::new(i * 2, 0, 10, CommentKind::Line));
        }
        small_index.finalize(200);
        assert!(small_index.superblock_idx.is_none());

        // Above threshold - superblock index built
        let mut large_index = CommentIndex::new();
        for i in 0..150 {
            large_index.push(CommentEntry::new(i * 2, 0, 10, CommentKind::Line));
        }
        large_index.finalize(400);
        assert!(large_index.superblock_idx.is_some());
    }

    #[test]
    fn test_superblock_lookup() {
        let mut index = CommentIndex::new();
        // Create comments spread across multiple superblocks
        for i in 0..200 {
            index.push(CommentEntry::new(
                i * 4,
                i * 10,
                i * 10 + 5,
                CommentKind::Line,
            ));
        }
        index.finalize(800);

        // Verify lookups work correctly with superblock index
        assert!(index.superblock_idx.is_some());

        // Check various positions
        assert!(index.has_comments(0));
        assert!(index.has_comments(100));
        assert!(index.has_comments(400));
        assert!(!index.has_comments(1));
        assert!(!index.has_comments(101));
    }

    #[test]
    fn test_out_of_order_insertion() {
        let mut index = CommentIndex::new();
        // Insert out of order
        index.push(CommentEntry::new(10, 0, 5, CommentKind::Line));
        index.push(CommentEntry::new(5, 10, 15, CommentKind::Head));
        index.push(CommentEntry::new(15, 20, 25, CommentKind::Foot));
        index.push(CommentEntry::new(5, 30, 35, CommentKind::Line));
        index.finalize(20);

        // Should be sorted after finalize
        let all: Vec<_> = index.iter().collect();
        assert_eq!(all[0].node_bp, 5);
        assert_eq!(all[1].node_bp, 5);
        assert_eq!(all[2].node_bp, 10);
        assert_eq!(all[3].node_bp, 15);
    }

    // Tag index tests

    #[test]
    fn test_empty_tag_index() {
        let index = TagIndex::new();
        assert!(index.is_empty());
        assert_eq!(index.len(), 0);
        assert!(index.get(0).is_none());
    }

    #[test]
    fn test_single_tag() {
        let mut index = TagIndex::new();
        index.push(5, "!!str".to_string());
        index.finalize();

        assert_eq!(index.len(), 1);
        assert!(index.has_tag(5));
        assert!(!index.has_tag(4));

        assert_eq!(index.get(5), Some("!!str"));
        assert_eq!(index.get(4), None);
    }

    #[test]
    fn test_multiple_tags() {
        let mut index = TagIndex::new();
        index.push(3, "!!int".to_string());
        index.push(7, "!custom".to_string());
        index.push(15, "!!str".to_string());
        index.finalize();

        assert_eq!(index.get(3), Some("!!int"));
        assert_eq!(index.get(7), Some("!custom"));
        assert_eq!(index.get(15), Some("!!str"));
        assert_eq!(index.get(10), None);
    }

    #[test]
    fn test_tag_iteration() {
        let mut index = TagIndex::new();
        index.push(5, "!!str".to_string());
        index.push(10, "!!int".to_string());
        index.finalize();

        let tags: Vec<_> = index.iter().collect();
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0], (5, "!!str"));
        assert_eq!(tags[1], (10, "!!int"));
    }

    #[test]
    fn test_large_comment_index() {
        // Test that comment index works with many comments (triggers superblock)
        let mut index = CommentIndex::with_capacity(500);

        // Add 500 comments spread across BP positions 0-5000
        for i in 0..500 {
            let bp_pos = (i * 10) as u32;
            let start = (i * 100) as u32;
            let end = start + 50;
            index.push(CommentEntry::new(bp_pos, start, end, CommentKind::Head));
        }

        index.finalize(5000);

        // Should have superblock index built
        assert!(index.superblock_idx.is_some());
        assert_eq!(index.len(), 500);

        // Verify lookups work correctly
        for i in 0..500 {
            let bp_pos = (i * 10) as u32;
            assert!(
                index.has_comments(bp_pos),
                "BP pos {} should have comments",
                bp_pos
            );
            assert!(
                !index.has_comments(bp_pos + 1),
                "BP pos {} should not have comments",
                bp_pos + 1
            );
        }

        // Verify iteration works
        let all_comments: Vec<_> = index.iter().collect();
        assert_eq!(all_comments.len(), 500);
    }

    #[test]
    fn test_memory_layout() {
        // Verify cache-friendly layout (4 entries per 64-byte cache line)
        assert_eq!(core::mem::size_of::<CommentEntry>(), 16);
        assert_eq!(core::mem::align_of::<CommentEntry>(), 4); // Due to u32 fields
    }
}
