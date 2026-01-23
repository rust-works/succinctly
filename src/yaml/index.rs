//! YAML index structures for efficient navigation.
//!
//! Holds the semi-index (IB, BP, TY) and provides rank/select operations.

#[cfg(not(test))]
use alloc::vec;
#[cfg(not(test))]
use alloc::{string::String, vec::Vec};

#[cfg(not(test))]
use alloc::collections::BTreeMap;

#[cfg(test)]
use std::collections::BTreeMap;

use crate::trees::{BalancedParens, WithSelect};
use crate::util::broadword::select_in_word;

use super::error::YamlError;
use super::light::YamlCursor;
use super::parser::build_semi_index;

/// Index structures for navigating YAML.
///
/// The type parameter `W` controls how the underlying data is stored:
/// - `Vec<u64>` for owned data (built from YAML text)
/// - `&[u64]` for borrowed data (e.g., from mmap)
///
/// Unlike JSON, YAML uses `WithSelect` for the balanced parentheses structure
/// because YAML has more BP opens than IB bits (containers don't have IB bits),
/// requiring efficient select1 queries for offset-to-BP lookups.
#[derive(Clone, Debug)]
pub struct YamlIndex<W = Vec<u64>> {
    /// Interest bits - marks positions of structural elements
    ib: W,
    /// Number of valid bits in IB
    ib_len: usize,
    /// Cumulative popcount per word (for fast rank/select on IB)
    ib_rank: Vec<u32>,
    /// Balanced parentheses - encodes the YAML structure as a tree.
    /// Uses WithSelect for O(1) select1 queries needed by find_bp_at_text_pos.
    bp: BalancedParens<W, WithSelect>,
    /// Type bits - 0 = mapping, 1 = sequence at each container position
    ty: W,
    /// Number of valid bits in TY
    #[allow(dead_code)]
    ty_len: usize,
    /// Direct BP open index to text position mapping.
    /// Entry i = text byte offset for the i-th BP open.
    bp_to_text: Vec<u32>,
    /// End positions for scalars. For each BP open, stores the end byte offset.
    /// For containers, stores 0 (containers don't have a text end position).
    bp_to_text_end: Vec<u32>,
    /// Sequence item markers - 1 if BP position is a sequence item wrapper
    seq_items: W,
    /// Container markers - 1 if BP position has a TY entry (is a mapping or sequence)
    containers: W,
    /// Cumulative popcount per word for containers (for fast rank on containers)
    containers_rank: Vec<u32>,
    /// Anchor definitions: anchor name → BP position of the anchored value
    anchors: BTreeMap<String, usize>,
    /// Reverse anchor mapping: BP position → anchor name (for metadata access)
    bp_to_anchor: BTreeMap<usize, String>,
    /// Alias references: BP position of alias → target BP position (resolved at parse time)
    aliases: BTreeMap<usize, usize>,
    /// Newline positions for fast line/column lookup.
    /// Bit i is set if position i is the start of a new line (immediately after a line terminator).
    newlines: crate::bits::BitVec,
}

/// Build cumulative popcount index for a bitvector.
/// Returns a vector where entry i = total 1-bits in words [0, i).
fn build_cumulative_rank(words: &[u64]) -> Vec<u32> {
    let mut rank = Vec::with_capacity(words.len() + 1);
    let mut cumulative: u32 = 0;
    rank.push(0);
    for &word in words {
        cumulative += word.count_ones();
        rank.push(cumulative);
    }
    rank
}

/// Build cumulative popcount index for IB.
/// Returns a vector where entry i = total 1-bits in words [0, i).
#[inline]
fn build_ib_rank(words: &[u64]) -> Vec<u32> {
    build_cumulative_rank(words)
}

/// Build cumulative popcount index for containers.
/// Returns a vector where entry i = total 1-bits in words [0, i).
#[inline]
fn build_containers_rank(words: &[u64]) -> Vec<u32> {
    build_cumulative_rank(words)
}

/// Build newline index from text.
/// Sets bit i if position i is the start of a new line (immediately after a line terminator).
/// Handles Unix (LF), Windows (CRLF), and classic Mac (CR) line endings.
fn build_newline_index(text: &[u8]) -> crate::bits::BitVec {
    if text.is_empty() {
        return crate::bits::BitVec::new();
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

    crate::bits::BitVec::from_words(bits, text.len())
}

impl YamlIndex<Vec<u64>> {
    /// Build a YAML index from YAML text.
    ///
    /// This parses the YAML to build the interest bits (IB), balanced
    /// parentheses (BP), and type bits (TY) index structures, plus
    /// newline positions for fast line/column lookup.
    pub fn build(yaml: &[u8]) -> Result<Self, YamlError> {
        let semi = build_semi_index(yaml)?;

        let ib_len = yaml.len();
        let ib_rank = build_ib_rank(&semi.ib);
        let containers_rank = build_containers_rank(&semi.containers);

        // Build newline index for fast line/column lookup
        let newlines = build_newline_index(yaml);

        // Build reverse anchor mapping (bp_pos → anchor_name)
        let bp_to_anchor: BTreeMap<usize, String> = semi
            .anchors
            .iter()
            .map(|(name, &bp_pos)| (bp_pos, name.clone()))
            .collect();

        Ok(Self {
            ib: semi.ib,
            ib_len,
            ib_rank,
            bp: BalancedParens::new_with_select(semi.bp, semi.bp_len),
            ty: semi.ty,
            ty_len: semi.ty_len,
            bp_to_text: semi.bp_to_text,
            bp_to_text_end: semi.bp_to_text_end,
            seq_items: semi.seq_items,
            containers: semi.containers,
            containers_rank,
            anchors: semi.anchors,
            bp_to_anchor,
            aliases: semi.aliases,
            newlines,
        })
    }
}

impl<W: AsRef<[u64]>> YamlIndex<W> {
    /// Create a YAML index from pre-existing IB, BP, TY, and bp_to_text data.
    ///
    /// This is useful for loading serialized index data.
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        ib: W,
        ib_len: usize,
        bp: W,
        bp_len: usize,
        ty: W,
        ty_len: usize,
        bp_to_text: Vec<u32>,
        bp_to_text_end: Vec<u32>,
        seq_items: W,
        containers: W,
        anchors: BTreeMap<String, usize>,
        aliases: BTreeMap<usize, usize>,
    ) -> Self {
        let ib_rank = build_ib_rank(ib.as_ref());
        let containers_rank = build_containers_rank(containers.as_ref());

        // Build reverse anchor mapping
        let bp_to_anchor: BTreeMap<usize, String> = anchors
            .iter()
            .map(|(name, &bp_pos)| (bp_pos, name.clone()))
            .collect();

        Self {
            ib,
            ib_len,
            ib_rank,
            bp: BalancedParens::from_words_with_select(bp, bp_len),
            ty,
            ty_len,
            bp_to_text,
            bp_to_text_end,
            seq_items,
            containers,
            containers_rank,
            anchors,
            bp_to_anchor,
            aliases,
            newlines: crate::bits::BitVec::new(),
        }
    }

    /// Create a YAML index from pre-existing data including newlines.
    ///
    /// This is useful for loading serialized index data with full line/column support.
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts_with_newlines(
        ib: W,
        ib_len: usize,
        bp: W,
        bp_len: usize,
        ty: W,
        ty_len: usize,
        bp_to_text: Vec<u32>,
        bp_to_text_end: Vec<u32>,
        seq_items: W,
        containers: W,
        anchors: BTreeMap<String, usize>,
        aliases: BTreeMap<usize, usize>,
        newlines: crate::bits::BitVec,
    ) -> Self {
        let ib_rank = build_ib_rank(ib.as_ref());
        let containers_rank = build_containers_rank(containers.as_ref());

        // Build reverse anchor mapping
        let bp_to_anchor: BTreeMap<usize, String> = anchors
            .iter()
            .map(|(name, &bp_pos)| (bp_pos, name.clone()))
            .collect();

        Self {
            ib,
            ib_len,
            ib_rank,
            bp: BalancedParens::from_words_with_select(bp, bp_len),
            ty,
            ty_len,
            bp_to_text,
            bp_to_text_end,
            seq_items,
            containers,
            containers_rank,
            anchors,
            bp_to_anchor,
            aliases,
            newlines,
        }
    }

    /// Get the text byte offset for a BP position.
    ///
    /// The BP position must be at an open parenthesis (1-bit).
    /// Returns `None` if the position is invalid.
    #[inline]
    pub fn bp_to_text_pos(&self, bp_pos: usize) -> Option<usize> {
        // We need the index of this open in bp_to_text
        // That's the count of 1-bits in BP before bp_pos (inclusive)
        // which is rank1(bp_pos + 1) - 1 if the bit at bp_pos is 1
        // Actually it's simpler: rank1(bp_pos) gives count before,
        // so the index is rank1(bp_pos) if the bit is 1
        let open_idx = self.bp.rank1(bp_pos);
        self.bp_to_text.get(open_idx).map(|&pos| pos as usize)
    }

    /// Get the text end byte offset for a BP position.
    ///
    /// For scalars, returns the end position. For containers and null values, returns `None`.
    /// The value 0 is used as a sentinel meaning "no end position" (null/empty values).
    #[inline]
    pub fn bp_to_text_end_pos(&self, bp_pos: usize) -> Option<usize> {
        let open_idx = self.bp.rank1(bp_pos);
        self.bp_to_text_end
            .get(open_idx)
            .map(|&pos| pos as usize)
            .filter(|&pos| pos > 0) // 0 is sentinel for "no end position"
    }

    /// Get a reference to the interest bits words.
    #[inline]
    pub fn ib(&self) -> &[u64] {
        self.ib.as_ref()
    }

    /// Get the number of valid bits in IB.
    #[inline]
    pub fn ib_len(&self) -> usize {
        self.ib_len
    }

    /// Get a reference to the balanced parentheses.
    #[inline]
    pub fn bp(&self) -> &BalancedParens<W, WithSelect> {
        &self.bp
    }

    /// Get a reference to the type bits.
    #[inline]
    pub fn ty(&self) -> &[u64] {
        self.ty.as_ref()
    }

    /// Get the number of valid bits in TY.
    #[inline]
    pub fn ty_len(&self) -> usize {
        self.ty_len
    }

    /// Create a cursor at the root of the YAML document.
    #[inline]
    pub fn root<'a>(&'a self, text: &'a [u8]) -> YamlCursor<'a, W> {
        YamlCursor::new(self, text, 0)
    }

    /// Check if the container at the given TY index is a sequence.
    ///
    /// Returns `true` for sequence, `false` for mapping.
    #[inline]
    pub fn is_sequence_at(&self, ty_idx: usize) -> bool {
        if ty_idx >= self.ty_len {
            return false;
        }
        let word_idx = ty_idx / 64;
        let bit_idx = ty_idx % 64;
        let ty_words = self.ty.as_ref();
        if word_idx < ty_words.len() {
            (ty_words[word_idx] >> bit_idx) & 1 == 1
        } else {
            false
        }
    }

    /// Check if the container at the given BP position is a sequence.
    ///
    /// This converts the BP position to a TY index and checks the type bit.
    /// Only valid for BP positions that are container opens (1-bits).
    ///
    /// Returns `true` for sequence, `false` for mapping.
    #[inline]
    pub fn is_sequence_at_bp(&self, bp_pos: usize) -> bool {
        // The TY index equals the number of container opens before this position.
        // Use the containers bitvector to count how many BP opens before bp_pos
        // actually have TY entries (are mappings or sequences).
        let ty_idx = self.count_containers_before(bp_pos);
        self.is_sequence_at(ty_idx)
    }

    /// Check if a BP position is a container (has a TY entry).
    ///
    /// Containers are mappings and sequences. Other BP nodes (scalars, sequence items)
    /// don't have TY entries.
    #[inline]
    pub fn is_container(&self, bp_pos: usize) -> bool {
        let word_idx = bp_pos / 64;
        let bit_idx = bp_pos % 64;
        let container_words = self.containers.as_ref();
        if word_idx < container_words.len() {
            (container_words[word_idx] >> bit_idx) & 1 == 1
        } else {
            false
        }
    }

    /// Count containers at BP positions 0..bp_pos.
    ///
    /// This gives the TY index for a BP position that is a container.
    /// Uses a cumulative popcount index for O(1) lookup.
    #[inline]
    pub fn count_containers_before(&self, bp_pos: usize) -> usize {
        let container_words = self.containers.as_ref();
        if container_words.is_empty() {
            return 0;
        }

        let word_idx = bp_pos / 64;
        let bit_idx = bp_pos % 64;

        // Use cumulative index for full words (O(1) lookup)
        let mut count = self.containers_rank[word_idx.min(container_words.len())] as usize;

        // Add partial word bits
        if word_idx < container_words.len() && bit_idx > 0 {
            let mask = (1u64 << bit_idx) - 1;
            count += (container_words[word_idx] & mask).count_ones() as usize;
        }

        count
    }

    /// Check if a BP position corresponds to an alias.
    #[inline]
    pub fn is_alias(&self, bp_pos: usize) -> bool {
        self.aliases.contains_key(&bp_pos)
    }

    /// Check if a BP position is a sequence item wrapper.
    ///
    /// Sequence items have BP open/close but no TY entry.
    /// They are wrapper nodes around the item's content.
    #[inline]
    pub fn is_seq_item(&self, bp_pos: usize) -> bool {
        let word_idx = bp_pos / 64;
        let bit_idx = bp_pos % 64;
        let seq_item_words = self.seq_items.as_ref();
        if word_idx < seq_item_words.len() {
            (seq_item_words[word_idx] >> bit_idx) & 1 == 1
        } else {
            false
        }
    }

    /// Count sequence items at BP positions 0..bp_pos.
    ///
    /// This is used to adjust TY index calculations since sequence items
    /// have BP opens but no TY entries.
    #[inline]
    pub fn count_seq_items_before(&self, bp_pos: usize) -> usize {
        let seq_item_words = self.seq_items.as_ref();
        if seq_item_words.is_empty() {
            return 0;
        }

        let word_idx = bp_pos / 64;
        let bit_idx = bp_pos % 64;

        let mut count = 0usize;
        for word in seq_item_words.iter().take(word_idx) {
            count += word.count_ones() as usize;
        }

        if word_idx < seq_item_words.len() && bit_idx > 0 {
            let mask = (1u64 << bit_idx) - 1;
            count += (seq_item_words[word_idx] & mask).count_ones() as usize;
        }

        count
    }

    /// Debug helper to get bp_to_text array.
    #[cfg(test)]
    pub fn debug_bp_to_text(&self) -> &[u32] {
        &self.bp_to_text
    }

    /// Get a reference to the bp_to_text array.
    ///
    /// This array maps BP open index to text byte offset. It is sorted in
    /// non-decreasing order (multiple opens can have the same text position
    /// when containers share position with their first child).
    #[inline]
    pub fn bp_to_text(&self) -> &[u32] {
        &self.bp_to_text
    }

    /// Find the BP position for the deepest node starting at a given text position.
    ///
    /// Uses binary search on bp_to_text to find the last open_idx with matching
    /// text position, then O(1) select1 to convert to BP position.
    /// Returns None if no match found.
    pub fn find_bp_at_text_pos(&self, text_pos: usize) -> Option<usize> {
        let bp_to_text = &self.bp_to_text;
        if bp_to_text.is_empty() {
            return None;
        }

        let text_pos_u32 = text_pos as u32;

        // Binary search to find any matching position
        let search_result = bp_to_text.binary_search(&text_pos_u32);

        // Find the last (rightmost) index with this text position
        // (deepest node when multiple nodes share the same text position)
        let last_match_idx = match search_result {
            Ok(idx) => {
                // Found a match, scan right to find the last one
                let mut last = idx;
                while last + 1 < bp_to_text.len() && bp_to_text[last + 1] == text_pos_u32 {
                    last += 1;
                }
                last
            }
            Err(_) => return None, // No match found
        };

        // Convert open_idx to bp_pos using O(1) select1
        // select1(k) returns the position of the k-th 1-bit (0-indexed)
        self.bp.select1(last_match_idx)
    }

    /// Get the target BP position for an alias at the given BP position.
    ///
    /// Returns `None` if the position is not an alias.
    #[inline]
    pub fn get_alias_target(&self, bp_pos: usize) -> Option<usize> {
        self.aliases.get(&bp_pos).copied()
    }

    /// Get the anchor name that an alias at the given BP position references.
    ///
    /// Returns `None` if the position is not an alias.
    /// This is equivalent to yq's `alias` function.
    #[inline]
    pub fn get_alias_anchor_name(&self, bp_pos: usize) -> Option<&str> {
        let target_bp_pos = self.aliases.get(&bp_pos)?;
        self.bp_to_anchor.get(target_bp_pos).map(|s| s.as_str())
    }

    /// Get the BP position of an anchor by name.
    ///
    /// Returns `None` if the anchor is not defined.
    #[inline]
    pub fn get_anchor_bp_pos(&self, anchor_name: &str) -> Option<usize> {
        self.anchors.get(anchor_name).copied()
    }

    /// Get the anchor name for a BP position.
    ///
    /// Returns `None` if the position does not have an anchor.
    #[inline]
    pub fn get_anchor_name(&self, bp_pos: usize) -> Option<&str> {
        self.bp_to_anchor.get(&bp_pos).map(|s| s.as_str())
    }

    /// Resolve an alias at the given BP position to a cursor pointing to
    /// the anchored value.
    ///
    /// Returns `None` if:
    /// - The position is not an alias
    /// - The referenced anchor is not defined
    pub fn resolve_alias<'a>(&'a self, bp_pos: usize, text: &'a [u8]) -> Option<YamlCursor<'a, W>> {
        let target_bp_pos = self.aliases.get(&bp_pos)?;
        Some(YamlCursor::new(self, text, *target_bp_pos))
    }

    /// Create a cursor at the given BP position.
    ///
    /// This is useful for navigating to a specific position in the index.
    #[inline]
    pub fn cursor_at<'a>(&'a self, bp_pos: usize, text: &'a [u8]) -> YamlCursor<'a, W> {
        YamlCursor::new(self, text, bp_pos)
    }

    /// Perform select1 with a hint for the starting word index.
    ///
    /// Uses exponential search (galloping) from the hint, which is optimal for
    /// sequential access patterns.
    #[inline]
    pub fn ib_select1_from(&self, k: usize, hint: usize) -> Option<usize> {
        let words = self.ib.as_ref();
        if words.is_empty() {
            return None;
        }

        let k32 = k as u32;
        let n = words.len();

        // Clamp hint to valid range
        let hint = hint.min(n.saturating_sub(1));

        // Check if hint is already past k
        let hint_rank = self.ib_rank[hint + 1];
        let lo;
        let hi;

        if hint_rank <= k32 {
            // k is at or after hint - search forward with exponential expansion
            let mut bound = 1usize;
            let mut prev = hint;

            loop {
                let next = (hint + bound).min(n);
                if next >= n || self.ib_rank[next + 1] > k32 {
                    lo = prev;
                    hi = next;
                    break;
                }
                prev = next;
                bound *= 2;
            }
        } else {
            // k is before hint - search backward with exponential expansion
            let mut bound = 1usize;
            let mut prev = hint;

            loop {
                let next = hint.saturating_sub(bound);
                if next == 0 || self.ib_rank[next + 1] <= k32 {
                    lo = next;
                    hi = prev;
                    break;
                }
                prev = next;
                bound *= 2;
            }
        }

        // Binary search within [lo, hi]
        let mut lo = lo;
        let mut hi = hi;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.ib_rank[mid + 1] <= k32 {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }

        if lo >= n {
            return None;
        }

        // Now lo is the word index, and ib_rank[lo] is count before this word
        let remaining = k - self.ib_rank[lo] as usize;
        let word = words[lo];
        let bit_pos = select_in_word(word, remaining as u32) as usize;
        let result = lo * 64 + bit_pos;

        if result < self.ib_len {
            Some(result)
        } else {
            None
        }
    }

    /// Perform select1 on the IB using pure binary search.
    #[inline]
    pub fn ib_select1(&self, k: usize) -> Option<usize> {
        let words = self.ib.as_ref();
        if words.is_empty() {
            return None;
        }

        let k32 = k as u32;
        let n = words.len();

        // Binary search over all words
        let mut lo = 0usize;
        let mut hi = n;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.ib_rank[mid + 1] <= k32 {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }

        if lo >= n {
            return None;
        }

        let remaining = k - self.ib_rank[lo] as usize;
        let word = words[lo];
        let bit_pos = select_in_word(word, remaining as u32) as usize;
        let result = lo * 64 + bit_pos;

        if result < self.ib_len {
            Some(result)
        } else {
            None
        }
    }

    /// Perform rank1 on the IB (count 1-bits in [0, pos)).
    pub fn ib_rank1(&self, pos: usize) -> usize {
        if pos == 0 {
            return 0;
        }

        let words = self.ib.as_ref();
        let word_idx = pos / 64;
        let bit_idx = pos % 64;

        // Use cumulative index for full words
        let mut count = self.ib_rank[word_idx.min(words.len())] as usize;

        // Add partial word
        if word_idx < words.len() && bit_idx > 0 {
            let mask = (1u64 << bit_idx) - 1;
            count += (words[word_idx] & mask).count_ones() as usize;
        }

        count
    }

    /// Convert a byte offset to (line, column) using the newline index.
    ///
    /// Lines and columns are 1-based to match common editor conventions.
    /// Returns (1, offset+1) if newline index is empty.
    ///
    /// # Example
    ///
    /// ```
    /// use succinctly::yaml::YamlIndex;
    ///
    /// let yaml = b"name: Alice\nage: 30";
    /// let index = YamlIndex::build(yaml).unwrap();
    ///
    /// // Position 0 is line 1, column 1
    /// assert_eq!(index.to_line_column(0), (1, 1));
    ///
    /// // Position 12 is line 2, column 1 (right after the newline)
    /// assert_eq!(index.to_line_column(12), (2, 1));
    /// ```
    #[inline]
    pub fn to_line_column(&self, offset: usize) -> (usize, usize) {
        use crate::RankSelect;

        if self.newlines.is_empty() {
            return (1, offset + 1);
        }

        // Line-start markers are at positions immediately after line terminators.
        // rank1(offset + 1) gives the count of line-start markers in [0, offset],
        // which equals the number of lines before the current line.
        // So line number = 1 + rank1(offset + 1).
        let markers_before_or_at = self.newlines.rank1(offset + 1);
        let line = 1 + markers_before_or_at;

        // Column = offset - start of this line + 1
        let line_start = if line == 1 {
            0
        } else {
            // The start of line N is at the (N-2)th marker (0-indexed select)
            // because line 1 has no marker, line 2 has marker 0, line 3 has marker 1, etc.
            self.newlines.select1(line - 2).unwrap_or(0)
        };

        let column = offset - line_start + 1;
        (line, column)
    }

    /// Convert (line, column) to a byte offset using the newline index.
    ///
    /// Lines and columns are 1-based to match common editor conventions.
    /// Returns None if the line/column is out of bounds or the newline index is empty.
    ///
    /// # Example
    ///
    /// ```
    /// use succinctly::yaml::YamlIndex;
    ///
    /// let yaml = b"name: Alice\nage: 30";
    /// let index = YamlIndex::build(yaml).unwrap();
    ///
    /// // Line 1, column 1 is offset 0
    /// assert_eq!(index.to_offset(1, 1), Some(0));
    ///
    /// // Line 2, column 1 is offset 12 (first char after newline)
    /// assert_eq!(index.to_offset(2, 1), Some(12));
    /// ```
    #[inline]
    pub fn to_offset(&self, line: usize, column: usize) -> Option<usize> {
        use crate::RankSelect;

        if line == 0 || column == 0 {
            return None;
        }

        if self.newlines.is_empty() {
            // No newline markers means single-line document
            if line == 1 {
                return Some(column - 1);
            } else {
                return None;
            }
        }

        // Line 1 starts at offset 0
        // Line N starts at the (N-2)th line-start marker (0-indexed select)
        let line_start = if line == 1 {
            0
        } else {
            self.newlines.select1(line - 2)?
        };

        Some(line_start + column - 1)
    }

    /// Get a reference to the newline bitvector.
    #[inline]
    pub fn newlines(&self) -> &crate::bits::BitVec {
        &self.newlines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_simple_mapping() {
        let yaml = b"name: Alice";
        let index = YamlIndex::build(yaml);
        assert!(index.is_ok());
    }

    #[test]
    fn test_build_simple_sequence() {
        let yaml = b"- item1\n- item2";
        let index = YamlIndex::build(yaml);
        assert!(index.is_ok());
    }

    #[test]
    fn test_root_cursor() {
        let yaml = b"name: Alice";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);
        assert_eq!(root.bp_position(), 0);
    }

    // ------------------------------------------------------------------------
    // Newline index tests
    // ------------------------------------------------------------------------

    #[test]
    fn test_newline_index_single_line() {
        let yaml = b"name: Alice";
        let index = YamlIndex::build(yaml).unwrap();

        // All positions on line 1
        assert_eq!(index.to_line_column(0), (1, 1)); // 'n'
        assert_eq!(index.to_line_column(5), (1, 6)); // ' '
        assert_eq!(index.to_line_column(10), (1, 11)); // 'e'

        // Reverse lookup
        assert_eq!(index.to_offset(1, 1), Some(0));
        assert_eq!(index.to_offset(1, 6), Some(5));
        assert_eq!(index.to_offset(2, 1), None); // No line 2
    }

    #[test]
    fn test_newline_index_multi_line_unix() {
        let yaml = b"name: Alice\nage: 30";
        let index = YamlIndex::build(yaml).unwrap();

        // Line 1: positions 0-10 (name: Alice)
        assert_eq!(index.to_line_column(0), (1, 1)); // 'n'
        assert_eq!(index.to_line_column(10), (1, 11)); // 'e'
        assert_eq!(index.to_line_column(11), (1, 12)); // '\n'

        // Line 2: positions 12-18 (age: 30)
        assert_eq!(index.to_line_column(12), (2, 1)); // 'a'
        assert_eq!(index.to_line_column(17), (2, 6)); // '3'
        assert_eq!(index.to_line_column(18), (2, 7)); // '0'

        // Reverse lookup
        assert_eq!(index.to_offset(1, 1), Some(0));
        assert_eq!(index.to_offset(2, 1), Some(12));
        assert_eq!(index.to_offset(2, 6), Some(17));
    }

    #[test]
    fn test_newline_index_sequence() {
        let yaml = b"- item1\n- item2\n- item3";
        let index = YamlIndex::build(yaml).unwrap();

        // Line 1: positions 0-6 (- item1)
        assert_eq!(index.to_line_column(0), (1, 1));
        assert_eq!(index.to_line_column(7), (1, 8)); // '\n'

        // Line 2: positions 8-14 (- item2)
        assert_eq!(index.to_line_column(8), (2, 1));

        // Line 3: positions 16-22 (- item3)
        assert_eq!(index.to_line_column(16), (3, 1));

        // Reverse lookup
        assert_eq!(index.to_offset(1, 1), Some(0));
        assert_eq!(index.to_offset(2, 1), Some(8));
        assert_eq!(index.to_offset(3, 1), Some(16));
    }

    #[test]
    fn test_newline_index_crlf() {
        let yaml = b"name: Alice\r\nage: 30";
        let index = YamlIndex::build(yaml).unwrap();

        // Line 1: positions 0-10 (name: Alice)
        assert_eq!(index.to_line_column(0), (1, 1));
        assert_eq!(index.to_line_column(10), (1, 11));

        // Line 2: positions 13-19 (age: 30) - after \r\n
        assert_eq!(index.to_line_column(13), (2, 1));
        assert_eq!(index.to_offset(2, 1), Some(13));
    }

    #[test]
    fn test_newline_index_classic_mac_cr() {
        let yaml = b"name: Alice\rage: 30";
        let index = YamlIndex::build(yaml).unwrap();

        // Line 2: position 12 (age: 30) - after \r
        assert_eq!(index.to_line_column(12), (2, 1));
        assert_eq!(index.to_offset(2, 1), Some(12));
    }

    #[test]
    fn test_newline_index_invalid_inputs() {
        let yaml = b"name: Alice\nage: 30";
        let index = YamlIndex::build(yaml).unwrap();

        assert_eq!(index.to_offset(0, 1), None); // line 0 invalid
        assert_eq!(index.to_offset(1, 0), None); // column 0 invalid
    }

    #[test]
    fn test_newline_index_round_trip() {
        let yaml = b"users:\n  - name: Alice\n    age: 30\n  - name: Bob\n    age: 25";
        let index = YamlIndex::build(yaml).unwrap();

        // Test round-trip: offset -> line/column -> offset
        for offset in 0..yaml.len() {
            let (line, col) = index.to_line_column(offset);
            let result = index.to_offset(line, col);
            assert_eq!(
                result,
                Some(offset),
                "Round-trip failed for offset {}",
                offset
            );
        }
    }
}
