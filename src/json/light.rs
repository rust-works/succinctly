#![allow(clippy::items_after_test_module)]
//! StandardJson - Lazy JSON navigation using the standard cursor.
//!
//! This module provides a cursor-based API for navigating JSON structures
//! without fully parsing the JSON text. Values are only decoded when explicitly
//! requested, allowing efficient access to specific parts of large JSON documents.
//!
//! # Design
//!
//! The API is based on the haskell-works `hw-json` library, adapted for Rust:
//!
//! - **Zero-copy navigation**: Cursors are lightweight position markers that don't
//!   allocate memory or parse JSON text during navigation.
//!
//! - **Immutable iteration**: `JsonFields` and `JsonElements` provide immutable
//!   iteration via `uncons()` which returns `(head, tail)` without mutation.
//!
//! - **Lazy decoding**: String and number values are only parsed when you call
//!   methods like `as_str()` or `as_i64()`.
//!
//! - **Generic storage**: Works with both owned (`Vec<u64>`) and borrowed (`&[u64]`)
//!   index data, supporting mmap-based workflows.
//!
//! # Example
//!
//! ```
//! use succinctly::json::light::{JsonIndex, StandardJson};
//!
//! let json = br#"{"name": "Alice", "age": 30}"#;
//! let index = JsonIndex::build(json);
//! let root = index.root(json);
//!
//! if let StandardJson::Object(fields) = root.value() {
//!     if let Some(name) = fields.find("name") {
//!         if let StandardJson::String(s) = name {
//!             assert_eq!(&*s.as_str().unwrap(), "Alice");
//!         }
//!     }
//! }
//! ```

#[cfg(not(test))]
use alloc::vec;
#[cfg(not(test))]
use alloc::{borrow::Cow, string::String, vec::Vec};

#[cfg(test)]
use std::borrow::Cow;

use core::cell::Cell;

use crate::trees::BalancedParens;
use crate::util::broadword::select_in_word;

// ============================================================================
// SequentialCursor: O(1) amortized access for streaming traversals
// ============================================================================

/// Cursor state for sequential access optimization on IB select.
///
/// When `ib_select1_from()` is called with consecutively increasing k values
/// (as happens during depth-first streaming like `.[]` iteration), this cursor
/// enables O(1) amortized access by maintaining incremental scan state instead
/// of performing exponential search each time.
///
/// # Design (ported from YAML O1 optimization)
///
/// Three-path dispatch:
/// - **Sequential** (k == next_k): Forward scan from cursor position, O(1) amortized
/// - **Forward gap** (k > next_k): Jump to expected word, then forward scan
/// - **Backwards** (k < next_k): Full recomputation (rare in streaming)
///
/// # Invariants (when `next_k < usize::MAX`)
///
/// - `ib_ones_before` = number of 1-bits in `ib_words[0..ib_word_idx)`
#[derive(Clone, Copy, Debug, Default)]
struct SequentialCursor {
    /// The next expected k value (IB rank) for sequential access.
    next_k: usize,
    /// Current word index in IB bitmap for forward scanning.
    ib_word_idx: usize,
    /// Number of 1-bits in ib_words[0..ib_word_idx).
    ib_ones_before: usize,
}

// ============================================================================
// JsonIndex: Holds the IB and BP index structures
// ============================================================================

/// Index structures for navigating JSON.
///
/// The type parameter `W` controls how the underlying data is stored:
/// - `Vec<u64>` for owned data (built from JSON text)
/// - `&[u64]` for borrowed data (e.g., from mmap)
///
/// Use [`JsonIndex::build`] to create an owned index from JSON text,
/// or [`JsonIndex::from_parts`] to create from pre-existing index data.
#[derive(Debug)]
pub struct JsonIndex<W = Vec<u64>> {
    /// Interest bits - marks positions of structural characters and value starts
    ib: W,
    /// Number of valid bits in IB
    ib_len: usize,
    /// Cumulative popcount per word (for fast rank/select on IB)
    ib_rank: Vec<u32>,
    /// Balanced parentheses - encodes the JSON structure as a tree
    bp: BalancedParens<W>,
    /// Newline positions for fast line/column lookup.
    /// Bit i is set if position i is the start of a new line (immediately after a line terminator).
    newlines: crate::bits::BitVec,
    /// Cursor for sequential access optimization (interior mutability).
    /// Enables amortized O(1) access when k values are accessed in monotonically
    /// increasing order (streaming/depth-first traversal like `.[]` iteration).
    ib_cursor: Cell<SequentialCursor>,
}

// Manual Clone impl since Cell<SequentialCursor> needs special handling
impl<W: Clone> Clone for JsonIndex<W> {
    fn clone(&self) -> Self {
        Self {
            ib: self.ib.clone(),
            ib_len: self.ib_len,
            ib_rank: self.ib_rank.clone(),
            bp: self.bp.clone(),
            newlines: self.newlines.clone(),
            ib_cursor: Cell::new(self.ib_cursor.get()),
        }
    }
}

/// Build cumulative popcount index for IB.
/// Returns a vector where entry i = total 1-bits in words [0, i).
fn build_ib_rank(words: &[u64]) -> Vec<u32> {
    let mut rank = Vec::with_capacity(words.len() + 1);
    let mut cumulative: u32 = 0;
    rank.push(0); // rank[0] = 0 (no words before word 0)
    for &word in words {
        cumulative += word.count_ones();
        rank.push(cumulative);
    }
    rank
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

impl JsonIndex<Vec<u64>> {
    /// Build a JSON index from JSON text.
    ///
    /// This parses the JSON to build the interest bits (IB) and balanced
    /// parentheses (BP) index structures, plus newline positions for
    /// fast line/column lookup.
    ///
    /// On supported platforms (aarch64, x86_64), this automatically uses
    /// SIMD-accelerated indexing for better performance.
    pub fn build(json: &[u8]) -> Self {
        #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
        let semi = crate::json::simd::build_semi_index_standard(json);

        #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
        let semi = crate::json::standard::build_semi_index(json);

        let ib_len = json.len();

        // Count actual BP bits
        let bp_bit_count = count_bp_bits(&semi.bp);

        // Build cumulative popcount index for IB
        let ib_rank = build_ib_rank(&semi.ib);

        // Build newline index for fast line/column lookup
        let newlines = build_newline_index(json);

        Self {
            ib: semi.ib,
            ib_len,
            ib_rank,
            bp: BalancedParens::new(semi.bp, bp_bit_count),
            newlines,
            ib_cursor: Cell::default(),
        }
    }
}

impl<W: AsRef<[u64]>> JsonIndex<W> {
    /// Create a JSON index from pre-existing IB and BP data.
    ///
    /// This is useful for loading serialized index data, e.g., from mmap.
    /// Note: This creates an empty newline index. For line/column lookup,
    /// use `from_parts_with_newlines` or rebuild with `build`.
    ///
    /// # Arguments
    ///
    /// * `ib` - Interest bits data
    /// * `ib_len` - Number of valid bits in IB (typically == JSON text length)
    /// * `bp` - Balanced parentheses data
    /// * `bp_len` - Number of valid bits in BP
    pub fn from_parts(ib: W, ib_len: usize, bp: W, bp_len: usize) -> Self {
        // Build cumulative popcount index for IB
        let ib_rank = build_ib_rank(ib.as_ref());

        Self {
            ib,
            ib_len,
            ib_rank,
            bp: BalancedParens::from_words(bp, bp_len),
            newlines: crate::bits::BitVec::new(),
            ib_cursor: Cell::default(),
        }
    }

    /// Create a JSON index from pre-existing IB, BP, and newline data.
    ///
    /// # Arguments
    ///
    /// * `ib` - Interest bits data
    /// * `ib_len` - Number of valid bits in IB (typically == JSON text length)
    /// * `bp` - Balanced parentheses data
    /// * `bp_len` - Number of valid bits in BP
    /// * `newlines` - Newline position bitvector
    pub fn from_parts_with_newlines(
        ib: W,
        ib_len: usize,
        bp: W,
        bp_len: usize,
        newlines: crate::bits::BitVec,
    ) -> Self {
        let ib_rank = build_ib_rank(ib.as_ref());

        Self {
            ib,
            ib_len,
            ib_rank,
            bp: BalancedParens::from_words(bp, bp_len),
            newlines,
            ib_cursor: Cell::default(),
        }
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
    pub fn bp(&self) -> &BalancedParens<W> {
        &self.bp
    }

    /// Convert byte offset to 1-indexed line and column.
    ///
    /// Returns (line, column) where both are 1-indexed.
    /// Useful for error reporting and `$__loc__` implementation.
    ///
    /// # Performance
    ///
    /// O(1) using rank on the newline bitvector.
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

    /// Convert 1-indexed line and column to byte offset.
    ///
    /// Column is 1-indexed byte offset within the line.
    /// Returns `None` if line/column is 0 or if the position is out of bounds.
    #[inline]
    pub fn to_offset(&self, line: usize, column: usize) -> Option<usize> {
        use crate::RankSelect;

        if line == 0 || column == 0 {
            return None;
        }

        let line_start = if line == 1 {
            // First line starts at offset 0
            0
        } else {
            // Line N starts at the (N-1)th line-start marker (0-indexed select)
            self.newlines.select1(line - 2)?
        };

        let offset = line_start + column - 1;
        if offset < self.ib_len {
            Some(offset)
        } else {
            None
        }
    }

    /// Create a cursor at the root of the JSON document.
    ///
    /// # Arguments
    ///
    /// * `text` - The original JSON text (must match the text used to build the index)
    #[inline]
    pub fn root<'a>(&'a self, text: &'a [u8]) -> JsonCursor<'a, W> {
        JsonCursor {
            text,
            index: self,
            bp_pos: 0,
        }
    }

    /// Perform select1 with sequential cursor optimization.
    ///
    /// Uses a sequential cursor to achieve O(1) amortized access for sequential
    /// patterns (like `.[]` iteration). Falls back to O(log d) exponential search
    /// for forward gaps and O(log n) for backwards jumps.
    ///
    /// # Performance (O1 optimization from YAML)
    ///
    /// - **Sequential access** (k == next_k): O(1) amortized via forward scan
    /// - **Forward gap** (k > next_k): O(1) rank lookup + O(d) forward scan
    /// - **Backwards** (k < next_k): O(log n) binary search (rare in streaming)
    ///
    /// The `hint` parameter is kept for API compatibility but is no longer used
    /// when the sequential cursor is active (sequential/forward-gap paths).
    ///
    /// For pure random access patterns (e.g., `.[42]`), prefer [`Self::ib_select1`].
    #[inline]
    pub fn ib_select1_from(&self, k: usize, hint: usize) -> Option<usize> {
        let cursor = self.ib_cursor.get();

        if k == cursor.next_k {
            // Fast path: direct sequential access (most common in streaming)
            self.ib_select1_sequential(k, cursor)
        } else if k > cursor.next_k {
            // Forward gap: advance cursor using rank, then forward scan
            let mut cursor = cursor;
            self.advance_ib_cursor_to(&mut cursor, k);
            self.ib_select1_sequential(k, cursor)
        } else {
            // Backwards jump: use exponential search from hint, then reset cursor
            self.ib_select1_with_hint(k, hint)
        }
    }

    /// Fast path for sequential access (k == cursor.next_k).
    ///
    /// Forward scans from the cursor position in IB, achieving O(1) amortized
    /// when called with consecutively increasing k values.
    #[inline]
    fn ib_select1_sequential(&self, k: usize, mut cursor: SequentialCursor) -> Option<usize> {
        let words = self.ib.as_ref();
        if words.is_empty() {
            return None;
        }

        // Forward scan in IB bitmap from cursor position
        let mut remaining = k - cursor.ib_ones_before;
        let mut wi = cursor.ib_word_idx;

        while wi < words.len() {
            let word = words[wi];
            let ones = word.count_ones() as usize;
            if remaining < ones {
                let bit_pos = select_in_word(word, remaining as u32) as usize;
                let result = wi * 64 + bit_pos;

                if result >= self.ib_len {
                    return None;
                }

                // Update cursor for next call
                cursor.ib_ones_before = k - remaining;
                cursor.ib_word_idx = wi;
                cursor.next_k = k + 1;
                self.ib_cursor.set(cursor);
                return Some(result);
            }
            remaining -= ones;
            wi += 1;
        }

        // k is out of range
        None
    }

    /// Advance the cursor from its current position to `target` k value.
    ///
    /// Uses cumulative rank to find the correct word. For small gaps, does linear
    /// scan from cursor position. For large gaps (>8 words), uses binary search.
    #[inline]
    fn advance_ib_cursor_to(&self, cursor: &mut SequentialCursor, target: usize) {
        let words = self.ib.as_ref();
        if words.is_empty() {
            return;
        }

        let k32 = target as u32;
        let n = words.len();

        // Estimate how far we need to go
        // Use binary search for large gaps to avoid O(N) scan
        let gap = target.saturating_sub(cursor.ib_ones_before);
        let estimated_words = gap / 8; // ~8 ones per word is typical for JSON

        let wi = if estimated_words > 8 {
            // Large gap: use binary search over rank array
            let mut lo = cursor.ib_word_idx;
            let mut hi = n;
            while lo < hi {
                let mid = lo + (hi - lo) / 2;
                if self.ib_rank[mid + 1] <= k32 {
                    lo = mid + 1;
                } else {
                    hi = mid;
                }
            }
            lo.min(n.saturating_sub(1))
        } else {
            // Small gap: linear scan from cursor (better cache locality)
            let mut wi = cursor.ib_word_idx;
            while wi < n && self.ib_rank[wi + 1] <= k32 {
                wi += 1;
            }
            wi.min(n.saturating_sub(1))
        };

        // Update cursor state
        cursor.ib_word_idx = wi;
        cursor.ib_ones_before = self.ib_rank[wi] as usize;
        cursor.next_k = target;
    }

    /// Exponential search fallback for backwards jumps.
    ///
    /// Used when k < cursor.next_k, which is rare during streaming access.
    /// Does NOT reset the cursor - backwards jumps indicate random access
    /// patterns where cursor reset would just cause more backwards jumps.
    fn ib_select1_with_hint(&self, k: usize, hint: usize) -> Option<usize> {
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

            // Gallop forward: double the step size until we overshoot
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

            // Gallop backward
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

        if result >= self.ib_len {
            return None;
        }

        // NOTE: Do NOT reset cursor here. Backwards jumps indicate random access
        // patterns where cursor reset would be wasted (next access is likely
        // also random, causing another backwards jump). The cursor is only
        // useful for sequential streaming patterns.

        Some(result)
    }

    /// Perform select1 on the IB using pure binary search.
    ///
    /// This is optimal for random access patterns (e.g., `.[42]`, slicing).
    /// For sequential access (e.g., `.[]` iteration), use [`Self::ib_select1_from`]
    /// with a hint for O(log d) instead of O(log n) performance.
    ///
    /// Returns the position of the k-th 1-bit (0-indexed).
    ///
    /// # Performance
    ///
    /// - **Random access**: O(log n) - optimal for indexed lookups
    /// - **Sequential access**: Use `ib_select1_from` instead for ~3.3x speedup
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

    /// Perform rank1 on the IB (count 1-bits in [0, pos)).
    ///
    /// Uses cumulative popcount index for O(1) performance.
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
}

// Helper to count actual BP bits (number of open + close parens)
fn count_bp_bits(bp_words: &[u64]) -> usize {
    // For standard cursor, we need to count actual meaningful bits
    // This is a simplification - in practice we'd track this during indexing
    // For now, estimate based on popcount (opens) * 2
    let total_ones: usize = bp_words.iter().map(|w| w.count_ones() as usize).sum();
    // Each node has one open and one close, so total bits = opens + closes = 2 * opens
    // But this is approximate - the actual length should be tracked during build
    total_ones * 2
}

// ============================================================================
// JsonCursor: Position in the JSON structure
// ============================================================================

/// A cursor pointing to a position in the JSON structure.
///
/// Cursors are lightweight (just a position integer) and cheap to copy.
/// Navigation methods return new cursors without mutation.
#[derive(Debug)]
pub struct JsonCursor<'a, W = Vec<u64>> {
    /// The original JSON text
    text: &'a [u8],
    /// Reference to the index
    index: &'a JsonIndex<W>,
    /// Position in the BP vector (0 = root)
    bp_pos: usize,
}

// Manual Clone/Copy impl since W is only used through a reference
impl<'a, W> Clone for JsonCursor<'a, W> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, W> Copy for JsonCursor<'a, W> {}

impl<'a, W: AsRef<[u64]>> JsonCursor<'a, W> {
    /// Create a cursor at a specific BP position.
    ///
    /// This is useful for constructing cursors when you know the BP position
    /// directly, such as when walking up the tree using `parent()`.
    #[inline]
    pub fn from_bp_position(index: &'a JsonIndex<W>, text: &'a [u8], bp_pos: usize) -> Self {
        Self {
            text,
            index,
            bp_pos,
        }
    }

    /// Get the position in the BP vector.
    #[inline]
    pub fn bp_position(&self) -> usize {
        self.bp_pos
    }

    /// Check if this cursor points to a container (array or object).
    ///
    /// This is a **fast** operation that only uses the BP structure -
    /// no text_position lookup is needed. Containers have children in
    /// the BP tree; leaves (strings, numbers, bools, null) don't.
    ///
    /// Use this when you only need to distinguish containers from leaves
    /// without reading the actual value content.
    #[inline]
    pub fn is_container(&self) -> bool {
        self.index.bp().first_child(self.bp_pos).is_some()
    }

    /// Get the byte position in the JSON text.
    ///
    /// This uses select1 on the IB to find the text position corresponding
    /// to this BP position.
    pub fn text_position(&self) -> Option<usize> {
        // The BP position corresponds to the n-th interest bit in IB
        // We need to find which 1-bit in IB corresponds to this BP position
        //
        // For standard cursor:
        // - BP has one open paren for each structural character/value start
        // - IB has one bit set for each structural character/value start
        // - So BP position N corresponds to the N-th set bit in IB
        //
        // Use BP's O(1) rank1 function instead of linear scan
        let rank = self.index.bp().rank1(self.bp_pos);

        // Use rank / 8 as a hint for where to start searching in IB.
        // JSON typically has ~7-8 structural characters per 64 bytes,
        // so rank / 8 is a reasonable estimate of the word index.
        // For sequential traversal, this gives O(log d) instead of O(log n)
        // where d is the distance from the hint.
        let hint = rank / 8;
        self.index.ib_select1_from(rank, hint)
    }

    /// Navigate to the first child.
    ///
    /// Returns `None` if this position has no children (is a leaf or close paren).
    #[inline]
    pub fn first_child(&self) -> Option<JsonCursor<'a, W>> {
        let new_pos = self.index.bp().first_child(self.bp_pos)?;
        Some(JsonCursor {
            text: self.text,
            index: self.index,
            bp_pos: new_pos,
        })
    }

    /// Navigate to the next sibling.
    ///
    /// Returns `None` if this is the last sibling.
    #[inline]
    pub fn next_sibling(&self) -> Option<JsonCursor<'a, W>> {
        let new_pos = self.index.bp().next_sibling(self.bp_pos)?;
        Some(JsonCursor {
            text: self.text,
            index: self.index,
            bp_pos: new_pos,
        })
    }

    /// Navigate to the parent.
    ///
    /// Returns `None` if this is the root.
    #[inline]
    pub fn parent(&self) -> Option<JsonCursor<'a, W>> {
        let new_pos = self.index.bp().parent(self.bp_pos)?;
        Some(JsonCursor {
            text: self.text,
            index: self.index,
            bp_pos: new_pos,
        })
    }

    /// Get the JSON value at this cursor position.
    ///
    /// This calls `text_position()` to determine the value type.
    pub fn value(&self) -> StandardJson<'a, W> {
        let Some(text_pos) = self.text_position() else {
            return StandardJson::Error("invalid cursor position");
        };

        if text_pos >= self.text.len() {
            return StandardJson::Error("text position out of bounds");
        }

        match self.text[text_pos] {
            b'{' => StandardJson::Object(JsonFields::from_object_cursor(*self)),
            b'[' => StandardJson::Array(JsonElements::from_array_cursor(*self)),
            b'"' => StandardJson::String(JsonString {
                text: self.text,
                start: text_pos,
            }),
            b't' | b'f' => {
                // true or false
                if self.text[text_pos..].starts_with(b"true") {
                    StandardJson::Bool(true)
                } else if self.text[text_pos..].starts_with(b"false") {
                    StandardJson::Bool(false)
                } else {
                    StandardJson::Error("invalid boolean")
                }
            }
            b'n' => {
                if self.text[text_pos..].starts_with(b"null") {
                    StandardJson::Null
                } else {
                    StandardJson::Error("invalid null")
                }
            }
            c if c == b'-' || c.is_ascii_digit() => StandardJson::Number(JsonNumber {
                text: self.text,
                start: text_pos,
            }),
            _ => StandardJson::Error("unexpected character"),
        }
    }

    /// Get children of this cursor for traversal.
    ///
    /// **Key optimization**: This method uses only BP structure operations
    /// (`first_child`, `next_sibling`) - no expensive `text_position()` calls.
    /// Use this for efficient traversal when you don't need to read values.
    ///
    /// Returns an iterator over child cursors.
    #[inline]
    pub fn children(&self) -> JsonChildren<'a, W> {
        JsonChildren {
            current: self.first_child(),
        }
    }

    /// Get the byte range in the original text for this value.
    ///
    /// Returns `(start, end)` where `text[start..end]` is the raw JSON bytes
    /// for this value, preserving original formatting.
    ///
    /// For containers (arrays/objects), uses BP structure to find the closing bracket.
    /// For scalars (strings/numbers/bools/null), scans text to find value end.
    pub fn text_range(&self) -> Option<(usize, usize)> {
        let start = self.text_position()?;

        if start >= self.text.len() {
            return None;
        }

        let end = match self.text[start] {
            // Containers: use BP to find closing bracket position
            b'{' | b'[' => {
                // Find the close paren in BP
                let close_bp = self.index.bp().find_close(self.bp_pos)?;
                // Map close BP position to text position
                // The close bracket is at the BP close position's corresponding IB bit
                let close_rank = self.index.bp().rank1(close_bp);
                // For containers, there's a close bracket at this position
                // We need to find the text position and then include the bracket
                let close_text = self.index.ib_select1_from(close_rank, close_rank / 8)?;
                close_text + 1 // Include the closing bracket
            }
            // String: scan for closing quote
            b'"' => {
                let mut i = start + 1;
                while i < self.text.len() {
                    match self.text[i] {
                        b'"' => return Some((start, i + 1)),
                        b'\\' => i += 2,
                        _ => i += 1,
                    }
                }
                self.text.len()
            }
            // Boolean true
            b't' => {
                if self.text[start..].starts_with(b"true") {
                    start + 4
                } else {
                    return None;
                }
            }
            // Boolean false
            b'f' => {
                if self.text[start..].starts_with(b"false") {
                    start + 5
                } else {
                    return None;
                }
            }
            // Null
            b'n' => {
                if self.text[start..].starts_with(b"null") {
                    start + 4
                } else {
                    return None;
                }
            }
            // Number: scan for end of number
            c if c == b'-' || c.is_ascii_digit() => {
                let mut i = start;
                while i < self.text.len() {
                    match self.text[i] {
                        b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E' => i += 1,
                        _ => break,
                    }
                }
                i
            }
            _ => return None,
        };

        Some((start, end))
    }

    /// Get the raw bytes for this JSON value.
    ///
    /// Returns the original bytes from the JSON text, preserving formatting.
    /// This is useful for zero-copy output of values.
    pub fn raw_bytes(&self) -> Option<&'a [u8]> {
        let (start, end) = self.text_range()?;
        Some(&self.text[start..end])
    }

    /// Create a cursor at the specified byte offset (0-indexed).
    ///
    /// Returns `None` if:
    /// - The offset is out of bounds
    /// - The offset doesn't correspond to a valid node
    ///
    /// This enables position-based navigation in jq queries via `at_offset(n)`.
    pub fn cursor_at_offset(&self, offset: usize) -> Option<JsonCursor<'a, W>> {
        if offset >= self.text.len() {
            return None;
        }

        // Get the rank at this position (count of structural bits before offset)
        let rank = self.index.ib_rank1(offset);

        // Determine which IB index contains this offset
        let ib_idx = if let Some(struct_pos) = self.index.ib_select1(rank) {
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

        // Convert IB index to BP position using binary search
        // Find the BP position where the ib_idx-th open paren is located
        let bp = self.index.bp();
        let bp_len = bp.len();

        if bp_len == 0 {
            return None;
        }

        // Binary search for the smallest bp_pos where rank1(bp_pos + 1) > ib_idx
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

        // Verify the position is valid
        if lo < bp_len && bp.rank1(lo + 1) == ib_idx + 1 {
            Some(JsonCursor {
                text: self.text,
                index: self.index,
                bp_pos: lo,
            })
        } else {
            None
        }
    }

    /// Create a cursor at the specified line and column (1-indexed).
    ///
    /// Returns `None` if:
    /// - Line or column is 0
    /// - The position is out of bounds
    /// - The position doesn't correspond to a valid node
    ///
    /// This enables position-based navigation in jq queries via `at_position(line; col)`.
    pub fn cursor_at_position(&self, line: usize, col: usize) -> Option<JsonCursor<'a, W>> {
        // Convert line/column to byte offset
        let offset = self.index.to_offset(line, col)?;

        // Use cursor_at_offset to find the node
        self.cursor_at_offset(offset)
    }
}

// ============================================================================
// JsonChildren: Fast traversal iterator (BP-only operations)
// ============================================================================

/// Iterator over child cursors using only BP operations.
///
/// This is the fastest way to traverse the JSON structure when you
/// don't need to read the actual values - it uses only `first_child`
/// and `next_sibling` operations without any `text_position()` calls.
#[derive(Debug)]
pub struct JsonChildren<'a, W = Vec<u64>> {
    current: Option<JsonCursor<'a, W>>,
}

impl<'a, W> Clone for JsonChildren<'a, W> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, W> Copy for JsonChildren<'a, W> {}

impl<'a, W: AsRef<[u64]>> Iterator for JsonChildren<'a, W> {
    type Item = JsonCursor<'a, W>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let cursor = self.current?;
        self.current = cursor.next_sibling();
        Some(cursor)
    }
}

// ============================================================================
// StandardJson: The value type
// ============================================================================

/// A JSON value with lazy decoding.
///
/// For objects and arrays, the value contains an iterator-like structure
/// that yields children on demand. For strings and numbers, the raw bytes
/// are stored and only parsed when you call `as_str()` or `as_i64()`.
#[derive(Clone, Debug)]
pub enum StandardJson<'a, W = Vec<u64>> {
    /// A JSON string (quotes not yet stripped, escapes not yet decoded)
    String(JsonString<'a>),
    /// A JSON number (not yet parsed)
    Number(JsonNumber<'a>),
    /// A JSON object with lazy field iteration
    Object(JsonFields<'a, W>),
    /// A JSON array with lazy element iteration
    Array(JsonElements<'a, W>),
    /// A JSON boolean
    Bool(bool),
    /// JSON null
    Null,
    /// An error encountered during navigation
    Error(&'static str),
}

// ============================================================================
// JsonFields: Immutable iteration over object fields
// ============================================================================

/// Immutable "list" of JSON object fields.
///
/// Use `uncons()` to get the first field and the remaining fields,
/// or `is_empty()` to check if there are no more fields.
///
/// This is `Copy` because it just holds a cursor position.
///
/// # Iteration Model
///
/// `JsonFields` holds a cursor pointing to the current key (or None if empty).
/// Each `uncons` returns the (key, value) pair and a new `JsonFields` pointing
/// to the next key (or empty if no more fields).
#[derive(Debug)]
pub struct JsonFields<'a, W = Vec<u64>> {
    /// Cursor pointing to the current field key, or None if exhausted
    key_cursor: Option<JsonCursor<'a, W>>,
}

// Manual Clone/Copy impl since JsonCursor is Copy
impl<'a, W> Clone for JsonFields<'a, W> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, W> Copy for JsonFields<'a, W> {}

impl<'a, W: AsRef<[u64]>> JsonFields<'a, W> {
    /// Create a new JsonFields from an object cursor.
    fn from_object_cursor(object_cursor: JsonCursor<'a, W>) -> Self {
        Self {
            key_cursor: object_cursor.first_child(),
        }
    }

    /// Check if there are no more fields.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.key_cursor.is_none()
    }

    /// Get the first field and the remaining fields.
    ///
    /// Returns `None` if there are no more fields.
    pub fn uncons(&self) -> Option<(JsonField<'a, W>, JsonFields<'a, W>)> {
        let key_cursor = self.key_cursor?;

        // Next sibling of key is the value
        let value_cursor = key_cursor.next_sibling()?;

        // The rest starts at the value's next sibling (the next key, if any)
        let rest = JsonFields {
            key_cursor: value_cursor.next_sibling(),
        };

        let field = JsonField {
            key_cursor,
            value_cursor,
        };

        Some((field, rest))
    }

    /// Find a field by name.
    ///
    /// Returns the value of the first field with the given name,
    /// or `None` if not found.
    pub fn find(&self, name: &str) -> Option<StandardJson<'a, W>> {
        let mut fields = *self;
        while let Some((field, rest)) = fields.uncons() {
            if let StandardJson::String(key) = field.key() {
                if key.as_str().ok()? == name {
                    return Some(field.value());
                }
            }
            fields = rest;
        }
        None
    }
}

impl<'a, W: AsRef<[u64]>> Iterator for JsonFields<'a, W> {
    type Item = JsonField<'a, W>;

    fn next(&mut self) -> Option<Self::Item> {
        let (field, rest) = self.uncons()?;
        *self = rest;
        Some(field)
    }
}

// ============================================================================
// JsonField: A single key-value pair
// ============================================================================

/// A single field in a JSON object.
#[derive(Debug)]
pub struct JsonField<'a, W = Vec<u64>> {
    key_cursor: JsonCursor<'a, W>,
    value_cursor: JsonCursor<'a, W>,
}

// Manual Clone/Copy impl since JsonCursor is Copy
impl<'a, W> Clone for JsonField<'a, W> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, W> Copy for JsonField<'a, W> {}

impl<'a, W: AsRef<[u64]>> JsonField<'a, W> {
    /// Get the field key (always a string).
    #[inline]
    pub fn key(&self) -> StandardJson<'a, W> {
        self.key_cursor.value()
    }

    /// Get the field value.
    #[inline]
    pub fn value(&self) -> StandardJson<'a, W> {
        self.value_cursor.value()
    }

    /// Get the value cursor directly.
    ///
    /// This allows access to the cursor for lazy value handling.
    #[inline]
    pub fn value_cursor(&self) -> JsonCursor<'a, W> {
        self.value_cursor
    }
}

// ============================================================================
// JsonElements: Immutable iteration over array elements
// ============================================================================

/// Immutable "list" of JSON array elements.
///
/// Use `uncons()` to get the first element and the remaining elements,
/// or `is_empty()` to check if there are no more elements.
///
/// This is `Copy` because it just holds a cursor position.
///
/// # Iteration Model
///
/// `JsonElements` holds a cursor pointing to the current element (or None if empty).
/// Each `uncons` returns the element value and a new `JsonElements` pointing
/// to the next element (or empty if no more elements).
#[derive(Debug)]
pub struct JsonElements<'a, W = Vec<u64>> {
    /// Cursor pointing to the current element, or None if exhausted
    element_cursor: Option<JsonCursor<'a, W>>,
}

// Manual Clone/Copy impl since JsonCursor is Copy
impl<'a, W> Clone for JsonElements<'a, W> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, W> Copy for JsonElements<'a, W> {}

impl<'a, W: AsRef<[u64]>> JsonElements<'a, W> {
    /// Create a new JsonElements from an array cursor.
    fn from_array_cursor(array_cursor: JsonCursor<'a, W>) -> Self {
        Self {
            element_cursor: array_cursor.first_child(),
        }
    }

    /// Check if there are no more elements.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.element_cursor.is_none()
    }

    /// Get the first element and the remaining elements.
    ///
    /// Returns `None` if there are no more elements.
    pub fn uncons(&self) -> Option<(StandardJson<'a, W>, JsonElements<'a, W>)> {
        let element_cursor = self.element_cursor?;

        let rest = JsonElements {
            element_cursor: element_cursor.next_sibling(),
        };

        let value = element_cursor.value();
        Some((value, rest))
    }

    /// Get the first element's cursor and the remaining elements.
    ///
    /// This is like `uncons` but returns the cursor instead of the value.
    /// Useful for lazy evaluation where you want to defer calling `value()`.
    pub fn uncons_cursor(&self) -> Option<(JsonCursor<'a, W>, JsonElements<'a, W>)> {
        let element_cursor = self.element_cursor?;

        let rest = JsonElements {
            element_cursor: element_cursor.next_sibling(),
        };

        Some((element_cursor, rest))
    }

    /// Get element by index (slow path).
    ///
    /// Note: This is O(n) as it iterates through elements, calling `value()`
    /// for each intermediate element.
    ///
    /// For better performance with random access, use [`get_fast`](Self::get_fast) which
    /// only calls `value()` on the target element.
    pub fn get(&self, index: usize) -> Option<StandardJson<'a, W>> {
        let mut elements = *self;
        for _ in 0..index {
            let (_, rest) = elements.uncons()?;
            elements = rest;
        }
        elements.uncons().map(|(elem, _)| elem)
    }

    /// Get element by index (fast path for random access).
    ///
    /// This method navigates to the target element using only BP operations
    /// (`next_sibling`), avoiding expensive `text_position()` calls for
    /// intermediate elements.
    ///
    /// Complexity: O(n) BP operations + O(log n) IB select for final element.
    /// This is faster than `get()` which does O(n) IB selects.
    #[inline]
    pub fn get_fast(&self, index: usize) -> Option<StandardJson<'a, W>> {
        let mut cursor = self.element_cursor?;

        // Navigate to the target element using only BP operations
        for _ in 0..index {
            cursor = cursor.next_sibling()?;
        }

        // Only call value() (which uses text_position/ib_select) on the target
        Some(cursor.value())
    }
}

impl<'a, W: AsRef<[u64]>> Iterator for JsonElements<'a, W> {
    type Item = StandardJson<'a, W>;

    fn next(&mut self) -> Option<Self::Item> {
        let (elem, rest) = self.uncons()?;
        *self = rest;
        Some(elem)
    }
}

// ============================================================================
// ElementCursorIter: Iterator over element cursors
// ============================================================================

/// Iterator that yields cursors for each array element.
///
/// Unlike `JsonElements` which yields `StandardJson` values, this iterator
/// yields `JsonCursor` values, allowing lazy evaluation of element values.
#[derive(Clone, Copy, Debug)]
pub struct ElementCursorIter<'a, W = Vec<u64>> {
    elements: JsonElements<'a, W>,
}

impl<'a, W: AsRef<[u64]>> ElementCursorIter<'a, W> {
    /// Create a new cursor iterator from JsonElements.
    pub fn new(elements: JsonElements<'a, W>) -> Self {
        Self { elements }
    }
}

impl<'a, W: AsRef<[u64]>> Iterator for ElementCursorIter<'a, W> {
    type Item = JsonCursor<'a, W>;

    fn next(&mut self) -> Option<Self::Item> {
        let (cursor, rest) = self.elements.uncons_cursor()?;
        self.elements = rest;
        Some(cursor)
    }
}

impl<'a, W: AsRef<[u64]>> JsonElements<'a, W> {
    /// Get an iterator over element cursors.
    ///
    /// This allows iterating over array elements while keeping them as
    /// lazy cursor references, deferring value evaluation until needed.
    pub fn cursor_iter(self) -> ElementCursorIter<'a, W> {
        ElementCursorIter::new(self)
    }
}

// ============================================================================
// JsonString: Lazy string decoding
// ============================================================================

/// A JSON string that hasn't been decoded yet.
///
/// Call `as_str()` to decode escape sequences and get the string value.
#[derive(Clone, Copy, Debug)]
pub struct JsonString<'a> {
    text: &'a [u8],
    start: usize,
}

impl<'a> JsonString<'a> {
    /// Get the raw bytes including quotes.
    pub fn raw_bytes(&self) -> &'a [u8] {
        let end = self.find_end();
        &self.text[self.start..end]
    }

    /// Decode the string value.
    ///
    /// Returns a `Cow::Borrowed` for strings without escapes (zero-copy),
    /// or a `Cow::Owned` for strings that need escape decoding.
    ///
    /// Returns an error if the string contains invalid escape sequences
    /// or invalid UTF-8.
    pub fn as_str(&self) -> Result<Cow<'a, str>, JsonError> {
        // Skip opening quote
        let start = self.start + 1;
        let end = self.find_string_end();

        let bytes = &self.text[start..end];

        // Check if we need to decode escapes
        if !bytes.contains(&b'\\') {
            // No escapes - can return directly (zero-copy)
            let s = core::str::from_utf8(bytes).map_err(|_| JsonError::InvalidUtf8)?;
            Ok(Cow::Borrowed(s))
        } else {
            // Has escapes - need to decode
            decode_escapes(bytes).map(Cow::Owned)
        }
    }

    fn find_end(&self) -> usize {
        self.find_string_end() + 1 // Include closing quote
    }

    fn find_string_end(&self) -> usize {
        let mut i = self.start + 1; // Skip opening quote
        while i < self.text.len() {
            match self.text[i] {
                b'"' => return i,
                b'\\' => i += 2, // Skip escape sequence
                _ => i += 1,
            }
        }
        self.text.len()
    }
}

/// Decode JSON string escape sequences.
///
/// Handles: \\, \", \/, \b, \f, \n, \r, \t, and \uXXXX (including surrogate pairs)
fn decode_escapes(bytes: &[u8]) -> Result<String, JsonError> {
    let mut result = String::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'\\' {
            if i + 1 >= bytes.len() {
                return Err(JsonError::InvalidEscape);
            }
            i += 1;
            match bytes[i] {
                b'"' => result.push('"'),
                b'\\' => result.push('\\'),
                b'/' => result.push('/'),
                b'b' => result.push('\u{0008}'), // backspace
                b'f' => result.push('\u{000C}'), // form feed
                b'n' => result.push('\n'),
                b'r' => result.push('\r'),
                b't' => result.push('\t'),
                b'u' => {
                    // Unicode escape: \uXXXX
                    if i + 4 >= bytes.len() {
                        return Err(JsonError::InvalidUnicodeEscape);
                    }
                    let hex = &bytes[i + 1..i + 5];
                    let codepoint = parse_hex4(hex)?;
                    i += 4;

                    // Check for surrogate pair
                    if (0xD800..=0xDBFF).contains(&codepoint) {
                        // High surrogate - must be followed by low surrogate
                        if i + 6 < bytes.len() && bytes[i + 1] == b'\\' && bytes[i + 2] == b'u' {
                            let low_hex = &bytes[i + 3..i + 7];
                            let low = parse_hex4(low_hex)?;
                            if (0xDC00..=0xDFFF).contains(&low) {
                                // Valid surrogate pair
                                let cp = 0x10000
                                    + ((codepoint as u32 - 0xD800) << 10)
                                    + (low as u32 - 0xDC00);
                                if let Some(c) = char::from_u32(cp) {
                                    result.push(c);
                                    i += 6; // Skip \uXXXX for low surrogate
                                } else {
                                    return Err(JsonError::InvalidUnicodeEscape);
                                }
                            } else {
                                return Err(JsonError::InvalidUnicodeEscape);
                            }
                        } else {
                            return Err(JsonError::InvalidUnicodeEscape);
                        }
                    } else if (0xDC00..=0xDFFF).contains(&codepoint) {
                        // Lone low surrogate
                        return Err(JsonError::InvalidUnicodeEscape);
                    } else {
                        // Regular BMP character
                        if let Some(c) = char::from_u32(codepoint as u32) {
                            result.push(c);
                        } else {
                            return Err(JsonError::InvalidUnicodeEscape);
                        }
                    }
                }
                _ => return Err(JsonError::InvalidEscape),
            }
            i += 1;
        } else {
            // Regular UTF-8 byte - copy until next backslash or end
            let start = i;
            while i < bytes.len() && bytes[i] != b'\\' {
                i += 1;
            }
            let chunk =
                core::str::from_utf8(&bytes[start..i]).map_err(|_| JsonError::InvalidUtf8)?;
            result.push_str(chunk);
        }
    }

    Ok(result)
}

/// Parse 4 hex digits into a u16.
fn parse_hex4(hex: &[u8]) -> Result<u16, JsonError> {
    if hex.len() != 4 {
        return Err(JsonError::InvalidUnicodeEscape);
    }

    let mut value = 0u16;
    for &b in hex {
        let digit = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => return Err(JsonError::InvalidUnicodeEscape),
        };
        value = value * 16 + digit as u16;
    }
    Ok(value)
}

// ============================================================================
// JsonNumber: Lazy number parsing
// ============================================================================

/// A JSON number that hasn't been parsed yet.
///
/// Call `as_i64()` or `as_f64()` to parse the number.
#[derive(Clone, Copy, Debug)]
pub struct JsonNumber<'a> {
    text: &'a [u8],
    start: usize,
}

impl<'a> JsonNumber<'a> {
    /// Get the raw bytes of the number.
    pub fn raw_bytes(&self) -> &'a [u8] {
        let end = self.find_end();
        &self.text[self.start..end]
    }

    /// Parse as i64.
    pub fn as_i64(&self) -> Result<i64, JsonError> {
        let bytes = self.raw_bytes();
        let s = core::str::from_utf8(bytes).map_err(|_| JsonError::InvalidUtf8)?;
        s.parse().map_err(|_| JsonError::InvalidNumber)
    }

    /// Parse as f64.
    pub fn as_f64(&self) -> Result<f64, JsonError> {
        let bytes = self.raw_bytes();
        let s = core::str::from_utf8(bytes).map_err(|_| JsonError::InvalidUtf8)?;
        s.parse().map_err(|_| JsonError::InvalidNumber)
    }

    fn find_end(&self) -> usize {
        let mut i = self.start;
        while i < self.text.len() {
            match self.text[i] {
                b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E' => i += 1,
                _ => break,
            }
        }
        i
    }
}

// ============================================================================
// Error type
// ============================================================================

/// Errors that can occur during JSON value extraction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JsonError {
    /// Invalid UTF-8 in string
    InvalidUtf8,
    /// Invalid number format
    InvalidNumber,
    /// Invalid escape sequence in string
    InvalidEscape,
    /// Invalid unicode escape (not a valid hex digit or invalid codepoint)
    InvalidUnicodeEscape,
}

impl core::fmt::Display for JsonError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            JsonError::InvalidUtf8 => write!(f, "invalid UTF-8 in string"),
            JsonError::InvalidNumber => write!(f, "invalid number format"),
            JsonError::InvalidEscape => write!(f, "invalid escape sequence in string"),
            JsonError::InvalidUnicodeEscape => write!(f, "invalid unicode escape sequence"),
        }
    }
}

// ============================================================================
// Type aliases for common configurations
// ============================================================================

/// JSON index with owned storage.
pub type OwnedJsonIndex = JsonIndex<Vec<u64>>;

/// JSON index with borrowed storage (e.g., from mmap).
pub type BorrowedJsonIndex<'a> = JsonIndex<&'a [u64]>;

/// JSON cursor with owned index.
pub type OwnedJsonCursor<'a> = JsonCursor<'a, Vec<u64>>;

/// JSON cursor with borrowed index.
pub type BorrowedJsonCursor<'a> = JsonCursor<'a, &'a [u64]>;

// ============================================================================
// Document trait implementations
// ============================================================================

use crate::jq::document::{
    DocumentCursor, DocumentElements, DocumentField, DocumentFields, DocumentValue,
};

impl<'a, W: AsRef<[u64]> + Clone> DocumentCursor for JsonCursor<'a, W> {
    type Value = StandardJson<'a, W>;

    #[inline]
    fn value(&self) -> Self::Value {
        JsonCursor::value(self)
    }

    #[inline]
    fn first_child(&self) -> Option<Self> {
        JsonCursor::first_child(self)
    }

    #[inline]
    fn next_sibling(&self) -> Option<Self> {
        JsonCursor::next_sibling(self)
    }

    #[inline]
    fn parent(&self) -> Option<Self> {
        JsonCursor::parent(self)
    }

    #[inline]
    fn is_container(&self) -> bool {
        JsonCursor::is_container(self)
    }

    #[inline]
    fn text_position(&self) -> Option<usize> {
        JsonCursor::text_position(self)
    }

    #[inline]
    fn cursor_at_offset(&self, offset: usize) -> Option<Self> {
        JsonCursor::cursor_at_offset(self, offset)
    }

    #[inline]
    fn cursor_at_position(&self, line: usize, col: usize) -> Option<Self> {
        JsonCursor::cursor_at_position(self, line, col)
    }

    #[inline]
    fn stream_json<Out: core::fmt::Write>(&self, out: &mut Out) -> core::fmt::Result {
        // For JSON, we can directly output the raw bytes - they're already valid JSON
        if let Some(bytes) = self.raw_bytes() {
            // SAFETY: JSON input is valid UTF-8 (checked during indexing)
            let s = core::str::from_utf8(bytes).map_err(|_| core::fmt::Error)?;
            out.write_str(s)
        } else {
            Err(core::fmt::Error)
        }
    }

    #[inline]
    fn stream_yaml<Out: core::fmt::Write>(
        &self,
        out: &mut Out,
        indent_spaces: usize,
    ) -> core::fmt::Result {
        // For JSON->YAML conversion, we need to format as YAML
        stream_json_as_yaml(out, self.value(), 0, indent_spaces)
    }

    #[inline]
    fn is_falsy(&self) -> bool {
        // A value is falsy if it's null or false
        matches!(self.value(), StandardJson::Null | StandardJson::Bool(false))
    }
}

impl<'a, W: AsRef<[u64]> + Clone> DocumentValue for StandardJson<'a, W> {
    type Cursor = JsonCursor<'a, W>;
    type Fields = JsonFields<'a, W>;
    type Elements = JsonElements<'a, W>;

    #[inline]
    fn is_null(&self) -> bool {
        matches!(self, StandardJson::Null)
    }

    fn as_bool(&self) -> Option<bool> {
        match self {
            StandardJson::Bool(b) => Some(*b),
            _ => None,
        }
    }

    fn as_i64(&self) -> Option<i64> {
        match self {
            StandardJson::Number(n) => n.as_i64().ok(),
            _ => None,
        }
    }

    fn as_f64(&self) -> Option<f64> {
        match self {
            StandardJson::Number(n) => n.as_f64().ok(),
            _ => None,
        }
    }

    fn as_str(&self) -> Option<Cow<'_, str>> {
        match self {
            StandardJson::String(s) => s.as_str().ok(),
            _ => None,
        }
    }

    fn as_object(&self) -> Option<Self::Fields> {
        match self {
            StandardJson::Object(fields) => Some(*fields),
            _ => None,
        }
    }

    fn as_array(&self) -> Option<Self::Elements> {
        match self {
            StandardJson::Array(elements) => Some(*elements),
            _ => None,
        }
    }

    fn type_name(&self) -> &'static str {
        match self {
            StandardJson::Null => "null",
            StandardJson::Bool(_) => "boolean",
            StandardJson::Number(_) => "number",
            StandardJson::String(_) => "string",
            StandardJson::Array(_) => "array",
            StandardJson::Object(_) => "object",
            StandardJson::Error(_) => "error",
        }
    }

    fn is_error(&self) -> bool {
        matches!(self, StandardJson::Error(_))
    }

    fn error_message(&self) -> Option<&'static str> {
        match self {
            StandardJson::Error(msg) => Some(msg),
            _ => None,
        }
    }
}

impl<'a, W: AsRef<[u64]> + Clone> DocumentFields for JsonFields<'a, W> {
    type Value = StandardJson<'a, W>;
    type Cursor = JsonCursor<'a, W>;

    fn uncons(&self) -> Option<(DocumentField<Self::Value, Self::Cursor>, Self)> {
        let (field, rest) = JsonFields::uncons(self)?;
        Some((
            DocumentField {
                key: field.key(),
                value: field.value(),
                value_cursor: field.value_cursor(),
            },
            rest,
        ))
    }

    fn find(&self, name: &str) -> Option<Self::Value> {
        JsonFields::find(self, name)
    }

    fn is_empty(&self) -> bool {
        JsonFields::is_empty(self)
    }
}

impl<'a, W: AsRef<[u64]> + Clone> DocumentElements for JsonElements<'a, W> {
    type Value = StandardJson<'a, W>;
    type Cursor = JsonCursor<'a, W>;

    fn uncons(&self) -> Option<(Self::Value, Self)> {
        JsonElements::uncons(self)
    }

    fn uncons_cursor(&self) -> Option<(Self::Cursor, Self)> {
        JsonElements::uncons_cursor(self)
    }

    fn get(&self, index: usize) -> Option<Self::Value> {
        JsonElements::get_fast(self, index)
    }

    fn is_empty(&self) -> bool {
        JsonElements::is_empty(self)
    }
}

// ============================================================================
// JSON to YAML Streaming Helpers
// ============================================================================

/// Stream a JSON value as YAML.
fn stream_json_as_yaml<'a, W: AsRef<[u64]> + Clone, Out: core::fmt::Write>(
    out: &mut Out,
    value: StandardJson<'a, W>,
    current_indent: usize,
    indent_spaces: usize,
) -> core::fmt::Result {
    match value {
        StandardJson::Null => out.write_str("null"),
        StandardJson::Bool(b) => out.write_str(if b { "true" } else { "false" }),
        StandardJson::Number(n) => {
            // Try integer first, then float
            if let Ok(i) = n.as_i64() {
                write!(out, "{}", i)
            } else if let Ok(f) = n.as_f64() {
                if f.is_nan() {
                    out.write_str(".nan")
                } else if f.is_infinite() {
                    if f > 0.0 {
                        out.write_str(".inf")
                    } else {
                        out.write_str("-.inf")
                    }
                } else {
                    write!(out, "{}", f)
                }
            } else {
                out.write_str("null")
            }
        }
        StandardJson::String(s) => {
            // SAFETY: JSON strings are valid UTF-8
            let str_val = core::str::from_utf8(s.raw_bytes()).map_err(|_| core::fmt::Error)?;
            stream_json_string_as_yaml(out, str_val)
        }
        StandardJson::Array(elements) => {
            if elements.is_empty() {
                return out.write_str("[]");
            }
            if indent_spaces == 0 {
                // Flow style
                out.write_char('[')?;
                let mut first = true;
                for elem in elements {
                    if !first {
                        out.write_str(", ")?;
                    }
                    first = false;
                    stream_json_as_yaml(out, elem, 0, 0)?;
                }
                out.write_char(']')
            } else {
                // Block style
                let mut first = true;
                for elem in elements {
                    if !first {
                        out.write_char('\n')?;
                        write_json_yaml_indent(out, current_indent)?;
                    }
                    first = false;
                    out.write_str("- ")?;
                    if is_json_container(&elem) {
                        out.write_char('\n')?;
                        write_json_yaml_indent(out, current_indent + indent_spaces)?;
                        stream_json_as_yaml(
                            out,
                            elem,
                            current_indent + indent_spaces,
                            indent_spaces,
                        )?;
                    } else {
                        stream_json_as_yaml(
                            out,
                            elem,
                            current_indent + indent_spaces,
                            indent_spaces,
                        )?;
                    }
                }
                Ok(())
            }
        }
        StandardJson::Object(fields) => {
            if fields.is_empty() {
                return out.write_str("{}");
            }
            if indent_spaces == 0 {
                // Flow style
                out.write_char('{')?;
                let mut first = true;
                for field in fields {
                    if !first {
                        out.write_str(", ")?;
                    }
                    first = false;
                    // Key
                    if let StandardJson::String(k) = field.key() {
                        let key_str =
                            core::str::from_utf8(k.raw_bytes()).map_err(|_| core::fmt::Error)?;
                        stream_json_string_as_yaml(out, key_str)?;
                    } else {
                        out.write_str("\"\"")?;
                    }
                    out.write_str(": ")?;
                    stream_json_as_yaml(out, field.value(), 0, 0)?;
                }
                out.write_char('}')
            } else {
                // Block style
                let mut first = true;
                for field in fields {
                    if !first {
                        out.write_char('\n')?;
                        write_json_yaml_indent(out, current_indent)?;
                    }
                    first = false;
                    // Key
                    if let StandardJson::String(k) = field.key() {
                        let key_str =
                            core::str::from_utf8(k.raw_bytes()).map_err(|_| core::fmt::Error)?;
                        stream_json_string_as_yaml(out, key_str)?;
                    } else {
                        out.write_str("\"\"")?;
                    }
                    out.write_char(':')?;
                    let val = field.value();
                    if is_json_container(&val) {
                        out.write_char('\n')?;
                        write_json_yaml_indent(out, current_indent + indent_spaces)?;
                        stream_json_as_yaml(
                            out,
                            val,
                            current_indent + indent_spaces,
                            indent_spaces,
                        )?;
                    } else {
                        out.write_char(' ')?;
                        stream_json_as_yaml(
                            out,
                            val,
                            current_indent + indent_spaces,
                            indent_spaces,
                        )?;
                    }
                }
                Ok(())
            }
        }
        StandardJson::Error(_) => out.write_str("null"),
    }
}

/// Check if a JSON value is a non-empty container.
fn is_json_container<W: AsRef<[u64]> + Clone>(value: &StandardJson<'_, W>) -> bool {
    match value {
        StandardJson::Array(elements) => !elements.is_empty(),
        StandardJson::Object(fields) => !fields.is_empty(),
        _ => false,
    }
}

/// Write indentation spaces.
fn write_json_yaml_indent<Out: core::fmt::Write>(
    out: &mut Out,
    spaces: usize,
) -> core::fmt::Result {
    for _ in 0..spaces {
        out.write_char(' ')?;
    }
    Ok(())
}

/// Stream a JSON string value as YAML with smart quoting.
fn stream_json_string_as_yaml<Out: core::fmt::Write>(out: &mut Out, s: &str) -> core::fmt::Result {
    if s.is_empty() {
        return out.write_str("''");
    }

    // Check if we need quoting
    if needs_json_yaml_quoting(s) {
        stream_json_yaml_double_quoted(out, s)
    } else {
        out.write_str(s)
    }
}

/// Check if a JSON string needs quoting when output as YAML.
fn needs_json_yaml_quoting(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }

    let bytes = s.as_bytes();

    // Check first character
    let first = bytes[0];
    if matches!(
        first,
        b'-' | b'?'
            | b':'
            | b','
            | b'['
            | b']'
            | b'{'
            | b'}'
            | b'#'
            | b'&'
            | b'*'
            | b'!'
            | b'|'
            | b'>'
            | b'\''
            | b'"'
            | b'%'
            | b'@'
            | b'`'
    ) {
        return true;
    }

    // Check for leading/trailing whitespace
    if bytes[0] == b' ' || bytes[bytes.len() - 1] == b' ' {
        return true;
    }

    // Check for special values
    let lower = s.to_lowercase();
    if matches!(
        lower.as_str(),
        "null" | "~" | "true" | "false" | "yes" | "no" | "on" | "off" | ".inf" | "-.inf" | ".nan"
    ) {
        return true;
    }

    // Check if it looks like a number
    if looks_like_json_yaml_number(s) {
        return true;
    }

    // Check for special characters
    for b in bytes {
        if *b < 0x20 || *b == b':' || *b == b'#' {
            return true;
        }
    }

    false
}

/// Check if a string looks like a number.
fn looks_like_json_yaml_number(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }

    let bytes = s.as_bytes();
    let mut i = 0;

    // Optional sign
    if bytes[i] == b'-' || bytes[i] == b'+' {
        i += 1;
        if i >= bytes.len() {
            return false;
        }
    }

    // Must have at least one digit
    if !bytes[i].is_ascii_digit() {
        return false;
    }

    // Check remaining
    let mut has_dot = false;
    let mut has_exp = false;
    while i < bytes.len() {
        match bytes[i] {
            b'0'..=b'9' => {}
            b'.' if !has_dot && !has_exp => has_dot = true,
            b'e' | b'E' if !has_exp => {
                has_exp = true;
                if i + 1 < bytes.len() && (bytes[i + 1] == b'-' || bytes[i + 1] == b'+') {
                    i += 1;
                }
            }
            _ => return false,
        }
        i += 1;
    }

    true
}

/// Stream a double-quoted YAML string.
fn stream_json_yaml_double_quoted<Out: core::fmt::Write>(
    out: &mut Out,
    s: &str,
) -> core::fmt::Result {
    out.write_char('"')?;

    for ch in s.chars() {
        match ch {
            '"' => out.write_str("\\\"")?,
            '\\' => out.write_str("\\\\")?,
            '\n' => out.write_str("\\n")?,
            '\r' => out.write_str("\\r")?,
            '\t' => out.write_str("\\t")?,
            c if (c as u32) < 0x20 => {
                let b = c as u8;
                out.write_str("\\x")?;
                const HEX: &[u8; 16] = b"0123456789abcdef";
                out.write_char(HEX[(b >> 4) as usize] as char)?;
                out.write_char(HEX[(b & 0xf) as usize] as char)?;
            }
            c => out.write_char(c)?,
        }
    }

    out.write_char('"')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_index() {
        let json = br#"{"a": 1}"#;
        let index = JsonIndex::build(json);
        assert!(!index.bp().is_empty());
    }

    #[test]
    fn test_root_cursor() {
        let json = br#"{"a": 1}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);
        assert_eq!(root.bp_position(), 0);
    }

    #[test]
    fn test_empty_object() {
        let json = br#"{}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::Object(fields) => {
                assert!(fields.is_empty());
            }
            _ => panic!("expected object"),
        }
    }

    #[test]
    fn test_empty_array() {
        let json = br#"[]"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::Array(elements) => {
                assert!(elements.is_empty());
            }
            _ => panic!("expected array"),
        }
    }

    #[test]
    fn test_simple_values() {
        // Test boolean true
        let json = b"true";
        let index = JsonIndex::build(json);
        let root = index.root(json);
        assert!(matches!(root.value(), StandardJson::Bool(true)));

        // Test boolean false
        let json = b"false";
        let index = JsonIndex::build(json);
        let root = index.root(json);
        assert!(matches!(root.value(), StandardJson::Bool(false)));

        // Test null
        let json = b"null";
        let index = JsonIndex::build(json);
        let root = index.root(json);
        assert!(matches!(root.value(), StandardJson::Null));
    }

    #[test]
    fn test_number() {
        let json = b"42";
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::Number(n) => {
                assert_eq!(n.as_i64().unwrap(), 42);
            }
            _ => panic!("expected number"),
        }
    }

    #[test]
    fn test_string() {
        let json = br#""hello""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                assert_eq!(s.as_str().unwrap(), "hello");
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_object_single_field() {
        let json = br#"{"name": "Alice"}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::Object(fields) => {
                assert!(!fields.is_empty());

                // Uncons the first field
                let (field, rest) = fields.uncons().expect("should have one field");

                // Check key
                match field.key() {
                    StandardJson::String(s) => {
                        assert_eq!(s.as_str().unwrap(), "name");
                    }
                    _ => panic!("expected string key"),
                }

                // Check value
                match field.value() {
                    StandardJson::String(s) => {
                        assert_eq!(s.as_str().unwrap(), "Alice");
                    }
                    _ => panic!("expected string value"),
                }

                // Rest should be empty
                assert!(rest.is_empty());
            }
            _ => panic!("expected object"),
        }
    }

    #[test]
    fn test_object_multiple_fields() {
        let json = br#"{"name": "Bob", "age": 30}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::Object(fields) => {
                // First field: name
                let (field1, rest1) = fields.uncons().expect("should have first field");
                match field1.key() {
                    StandardJson::String(s) => assert_eq!(s.as_str().unwrap(), "name"),
                    _ => panic!("expected string key"),
                }
                match field1.value() {
                    StandardJson::String(s) => assert_eq!(s.as_str().unwrap(), "Bob"),
                    _ => panic!("expected string value"),
                }

                // Second field: age
                let (field2, rest2) = rest1.uncons().expect("should have second field");
                match field2.key() {
                    StandardJson::String(s) => assert_eq!(s.as_str().unwrap(), "age"),
                    _ => panic!("expected string key"),
                }
                match field2.value() {
                    StandardJson::Number(n) => assert_eq!(n.as_i64().unwrap(), 30),
                    _ => panic!("expected number value"),
                }

                // No more fields
                assert!(rest2.is_empty());
            }
            _ => panic!("expected object"),
        }
    }

    #[test]
    fn test_object_find_field() {
        let json = br#"{"name": "Charlie", "age": 25, "city": "NYC"}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::Object(fields) => {
                // Find existing field
                match fields.find("age") {
                    Some(StandardJson::Number(n)) => assert_eq!(n.as_i64().unwrap(), 25),
                    _ => panic!("expected number"),
                }

                // Find first field
                match fields.find("name") {
                    Some(StandardJson::String(s)) => assert_eq!(s.as_str().unwrap(), "Charlie"),
                    _ => panic!("expected string"),
                }

                // Find last field
                match fields.find("city") {
                    Some(StandardJson::String(s)) => assert_eq!(s.as_str().unwrap(), "NYC"),
                    _ => panic!("expected string"),
                }

                // Non-existent field
                assert!(fields.find("missing").is_none());
            }
            _ => panic!("expected object"),
        }
    }

    #[test]
    fn test_array_single_element() {
        let json = br#"[42]"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::Array(elements) => {
                assert!(!elements.is_empty());

                let (elem, rest) = elements.uncons().expect("should have one element");
                match elem {
                    StandardJson::Number(n) => assert_eq!(n.as_i64().unwrap(), 42),
                    _ => panic!("expected number"),
                }

                assert!(rest.is_empty());
            }
            _ => panic!("expected array"),
        }
    }

    #[test]
    fn test_array_multiple_elements() {
        let json = br#"[1, 2, 3]"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::Array(elements) => {
                let (e1, rest1) = elements.uncons().expect("first");
                let (e2, rest2) = rest1.uncons().expect("second");
                let (e3, rest3) = rest2.uncons().expect("third");

                match e1 {
                    StandardJson::Number(n) => assert_eq!(n.as_i64().unwrap(), 1),
                    _ => panic!("expected number"),
                }
                match e2 {
                    StandardJson::Number(n) => assert_eq!(n.as_i64().unwrap(), 2),
                    _ => panic!("expected number"),
                }
                match e3 {
                    StandardJson::Number(n) => assert_eq!(n.as_i64().unwrap(), 3),
                    _ => panic!("expected number"),
                }

                assert!(rest3.is_empty());
            }
            _ => panic!("expected array"),
        }
    }

    #[test]
    fn test_array_get() {
        let json = br#"["a", "b", "c"]"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::Array(elements) => {
                match elements.get(0) {
                    Some(StandardJson::String(s)) => assert_eq!(s.as_str().unwrap(), "a"),
                    _ => panic!("expected string at index 0"),
                }
                match elements.get(1) {
                    Some(StandardJson::String(s)) => assert_eq!(s.as_str().unwrap(), "b"),
                    _ => panic!("expected string at index 1"),
                }
                match elements.get(2) {
                    Some(StandardJson::String(s)) => assert_eq!(s.as_str().unwrap(), "c"),
                    _ => panic!("expected string at index 2"),
                }
                assert!(elements.get(3).is_none());
            }
            _ => panic!("expected array"),
        }
    }

    #[test]
    fn test_nested_object() {
        let json = br#"{"person": {"name": "Dave"}}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::Object(fields) => match fields.find("person") {
                Some(StandardJson::Object(inner_fields)) => match inner_fields.find("name") {
                    Some(StandardJson::String(s)) => {
                        assert_eq!(s.as_str().unwrap(), "Dave");
                    }
                    _ => panic!("expected string"),
                },
                _ => panic!("expected nested object"),
            },
            _ => panic!("expected object"),
        }
    }

    #[test]
    fn test_array_of_objects() {
        let json = br#"[{"a": 1}, {"b": 2}]"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::Array(elements) => {
                // First object
                match elements.get(0) {
                    Some(StandardJson::Object(fields)) => match fields.find("a") {
                        Some(StandardJson::Number(n)) => assert_eq!(n.as_i64().unwrap(), 1),
                        _ => panic!("expected number"),
                    },
                    _ => panic!("expected object"),
                }

                // Second object
                match elements.get(1) {
                    Some(StandardJson::Object(fields)) => match fields.find("b") {
                        Some(StandardJson::Number(n)) => assert_eq!(n.as_i64().unwrap(), 2),
                        _ => panic!("expected number"),
                    },
                    _ => panic!("expected object"),
                }
            }
            _ => panic!("expected array"),
        }
    }

    #[test]
    fn test_negative_number() {
        let json = b"-123";
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::Number(n) => {
                assert_eq!(n.as_i64().unwrap(), -123);
            }
            _ => panic!("expected number"),
        }
    }

    #[test]
    fn test_float_number() {
        let json = b"1.23456";
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::Number(n) => {
                let f = n.as_f64().unwrap();
                assert!((f - 1.23456).abs() < 0.0001);
            }
            _ => panic!("expected number"),
        }
    }

    #[test]
    fn test_immutable_iteration() {
        // Test that iteration is truly immutable - we can iterate multiple times
        let json = br#"[1, 2, 3]"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        if let StandardJson::Array(elements) = root.value() {
            // First iteration
            let (e1, rest1) = elements.uncons().unwrap();
            assert!(matches!(e1, StandardJson::Number(_)));

            // Start over - elements is still valid
            let (e1_again, _) = elements.uncons().unwrap();
            assert!(matches!(e1_again, StandardJson::Number(_)));

            // Continue first iteration
            let (e2, _) = rest1.uncons().unwrap();
            assert!(matches!(e2, StandardJson::Number(_)));
        }
    }

    // ========================================================================
    // Escape sequence tests
    // ========================================================================

    #[test]
    fn test_string_no_escapes_is_borrowed() {
        let json = br#""hello world""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                let result = s.as_str().unwrap();
                // Should be Cow::Borrowed for strings without escapes
                assert!(matches!(result, Cow::Borrowed(_)));
                assert_eq!(&*result, "hello world");
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_escaped_quote() {
        let json = br#""hello\"world""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                let result = s.as_str().unwrap();
                assert_eq!(&*result, "hello\"world");
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_escaped_backslash() {
        let json = br#""hello\\world""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                let result = s.as_str().unwrap();
                assert_eq!(&*result, "hello\\world");
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_escaped_slash() {
        let json = br#""hello\/world""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                let result = s.as_str().unwrap();
                assert_eq!(&*result, "hello/world");
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_escaped_newline() {
        let json = br#""hello\nworld""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                let result = s.as_str().unwrap();
                assert_eq!(&*result, "hello\nworld");
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_escaped_tab() {
        let json = br#""hello\tworld""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                let result = s.as_str().unwrap();
                assert_eq!(&*result, "hello\tworld");
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_escaped_carriage_return() {
        let json = br#""hello\rworld""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                let result = s.as_str().unwrap();
                assert_eq!(&*result, "hello\rworld");
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_escaped_backspace() {
        let json = br#""hello\bworld""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                let result = s.as_str().unwrap();
                assert_eq!(&*result, "hello\u{0008}world");
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_escaped_formfeed() {
        let json = br#""hello\fworld""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                let result = s.as_str().unwrap();
                assert_eq!(&*result, "hello\u{000C}world");
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_unicode_escape_bmp() {
        // \u0041 is 'A'
        let json = br#""\u0041""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                let result = s.as_str().unwrap();
                assert_eq!(&*result, "A");
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_unicode_escape_euro() {
        // \u20AC is €
        let json = br#""\u20AC""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                let result = s.as_str().unwrap();
                assert_eq!(&*result, "€");
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_unicode_escape_lowercase() {
        // \u00e9 is é (lowercase hex)
        let json = br#""\u00e9""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                let result = s.as_str().unwrap();
                assert_eq!(&*result, "é");
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_unicode_surrogate_pair() {
        // \uD83D\uDE00 is 😀 (U+1F600)
        let json = br#""\uD83D\uDE00""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                let result = s.as_str().unwrap();
                assert_eq!(&*result, "😀");
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_multiple_escapes() {
        let json = br#""line1\nline2\ttab\r\n""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                let result = s.as_str().unwrap();
                assert_eq!(&*result, "line1\nline2\ttab\r\n");
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_mixed_escapes_and_unicode() {
        let json = br#""Price: \u20AC100\nTax: \u00A310""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                let result = s.as_str().unwrap();
                assert_eq!(&*result, "Price: €100\nTax: £10");
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_invalid_escape() {
        let json = br#""\x""#; // \x is not valid JSON
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                assert_eq!(s.as_str(), Err(JsonError::InvalidEscape));
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_lone_high_surrogate() {
        // Lone high surrogate without low surrogate
        let json = br#""\uD83D""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                assert_eq!(s.as_str(), Err(JsonError::InvalidUnicodeEscape));
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_lone_low_surrogate() {
        // Lone low surrogate
        let json = br#""\uDE00""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                assert_eq!(s.as_str(), Err(JsonError::InvalidUnicodeEscape));
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_invalid_unicode_hex() {
        // Invalid hex digit
        let json = br#""\uXXXX""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                assert_eq!(s.as_str(), Err(JsonError::InvalidUnicodeEscape));
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_with_escaped_key_in_object() {
        let json = br#"{"na\nme": "value"}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::Object(fields) => {
                // find should handle escaped keys
                let (field, _) = fields.uncons().unwrap();
                match field.key() {
                    StandardJson::String(s) => {
                        assert_eq!(&*s.as_str().unwrap(), "na\nme");
                    }
                    _ => panic!("expected string key"),
                }
            }
            _ => panic!("expected object"),
        }
    }

    // ========================================================================
    // Iterator tests
    // ========================================================================

    #[test]
    fn test_json_fields_iterator() {
        let json = br#"{"a": 1, "b": 2, "c": 3}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        if let StandardJson::Object(fields) = root.value() {
            let keys: Vec<_> = fields
                .map(|f| {
                    if let StandardJson::String(s) = f.key() {
                        s.as_str().unwrap().into_owned()
                    } else {
                        panic!("expected string key")
                    }
                })
                .collect();
            assert_eq!(keys, vec!["a", "b", "c"]);
        } else {
            panic!("expected object");
        }
    }

    #[test]
    fn test_json_elements_iterator() {
        let json = br#"[1, 2, 3, 4, 5]"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        if let StandardJson::Array(elements) = root.value() {
            let nums: Vec<_> = elements
                .filter_map(|e| {
                    if let StandardJson::Number(n) = e {
                        n.as_i64().ok()
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(nums, vec![1, 2, 3, 4, 5]);
        } else {
            panic!("expected array");
        }
    }

    #[test]
    fn test_iterator_empty_object() {
        let json = br#"{}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        if let StandardJson::Object(fields) = root.value() {
            assert_eq!(fields.count(), 0);
        } else {
            panic!("expected object");
        }
    }

    #[test]
    fn test_iterator_empty_array() {
        let json = br#"[]"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        if let StandardJson::Array(elements) = root.value() {
            assert_eq!(elements.count(), 0);
        } else {
            panic!("expected array");
        }
    }

    // ========================================================================
    // Display tests
    // ========================================================================

    #[test]
    fn test_json_error_display() {
        use std::string::ToString;
        assert_eq!(
            JsonError::InvalidUtf8.to_string(),
            "invalid UTF-8 in string"
        );
        assert_eq!(
            JsonError::InvalidNumber.to_string(),
            "invalid number format"
        );
        assert_eq!(
            JsonError::InvalidEscape.to_string(),
            "invalid escape sequence in string"
        );
        assert_eq!(
            JsonError::InvalidUnicodeEscape.to_string(),
            "invalid unicode escape sequence"
        );
    }

    // ========================================================================
    // Fast traversal tests (is_container, children)
    // ========================================================================

    #[test]
    fn test_is_container_object() {
        let json = br#"{"a": 1}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);
        assert!(root.is_container());
    }

    #[test]
    fn test_is_container_array() {
        let json = br#"[1, 2, 3]"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);
        assert!(root.is_container());
    }

    #[test]
    fn test_is_container_empty_object() {
        let json = br#"{}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);
        // Empty containers have no children, so is_container returns false
        assert!(!root.is_container());
    }

    #[test]
    fn test_is_container_empty_array() {
        let json = br#"[]"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);
        // Empty containers have no children, so is_container returns false
        assert!(!root.is_container());
    }

    #[test]
    fn test_is_container_leaf_values() {
        // String
        let json = br#""hello""#;
        let index = JsonIndex::build(json);
        assert!(!index.root(json).is_container());

        // Number
        let json = b"42";
        let index = JsonIndex::build(json);
        assert!(!index.root(json).is_container());

        // Boolean
        let json = b"true";
        let index = JsonIndex::build(json);
        assert!(!index.root(json).is_container());

        // Null
        let json = b"null";
        let index = JsonIndex::build(json);
        assert!(!index.root(json).is_container());
    }

    #[test]
    fn test_children_array() {
        let json = br#"[1, 2, 3]"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        // Count children using the fast iterator
        let count: usize = root.children().count();
        assert_eq!(count, 3);
    }

    #[test]
    fn test_children_object() {
        let json = br#"{"a": 1, "b": 2}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        // Object children include both keys and values
        // {"a": 1, "b": 2} -> children are: "a", 1, "b", 2
        let count: usize = root.children().count();
        assert_eq!(count, 4);
    }

    #[test]
    fn test_children_nested() {
        let json = br#"{"arr": [1, 2]}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        // Root's direct children: "arr", [1, 2]
        let direct_children: Vec<_> = root.children().collect();
        assert_eq!(direct_children.len(), 2);

        // The array has 2 children: 1, 2
        let array_cursor = direct_children[1]; // [1, 2]
        assert!(array_cursor.is_container());
        assert_eq!(array_cursor.children().count(), 2);
    }

    #[test]
    fn test_children_empty() {
        let json = br#"[]"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        assert_eq!(root.children().count(), 0);
    }

    #[test]
    fn test_children_recursive_count() {
        // Test that recursive counting works correctly
        let json = br#"{"a": [1, 2], "b": {"c": 3}}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        fn count_all(cursor: super::JsonCursor) -> usize {
            1 + cursor.children().map(count_all).sum::<usize>()
        }

        // Structure (BP nodes):
        // root object (1)
        //   "a" key (1)
        //   [1, 2] value (1)
        //     1 (1)
        //     2 (1)
        //   "b" key (1)
        //   {"c": 3} value (1)
        //     "c" key (1)
        //     3 value (1)
        // Total: 9 nodes
        assert_eq!(count_all(root), 9);
    }

    // ========================================================================
    // Newline index tests
    // ========================================================================

    #[test]
    fn test_newline_index_single_line() {
        let json = br#"{"name": "Alice"}"#;
        let index = JsonIndex::build(json);

        // All positions on line 1
        assert_eq!(index.to_line_column(0), (1, 1)); // '{'
        assert_eq!(index.to_line_column(8), (1, 9)); // ' '
        assert_eq!(index.to_line_column(16), (1, 17)); // '}'

        // Reverse lookup
        assert_eq!(index.to_offset(1, 1), Some(0));
        assert_eq!(index.to_offset(1, 9), Some(8));
        assert_eq!(index.to_offset(2, 1), None); // No line 2
    }

    #[test]
    fn test_newline_index_multi_line() {
        let json = b"{\n  \"name\": \"Alice\"\n}";
        let index = JsonIndex::build(json);

        // Line 1: position 0 ('{')
        assert_eq!(index.to_line_column(0), (1, 1));
        assert_eq!(index.to_line_column(1), (1, 2)); // '\n'

        // Line 2: positions 2-18 ('  "name": "Alice"')
        assert_eq!(index.to_line_column(2), (2, 1)); // first space
        assert_eq!(index.to_line_column(4), (2, 3)); // '"'

        // Line 3: position 20 ('}')
        assert_eq!(index.to_line_column(20), (3, 1));

        // Reverse lookup
        assert_eq!(index.to_offset(1, 1), Some(0));
        assert_eq!(index.to_offset(2, 1), Some(2));
        assert_eq!(index.to_offset(3, 1), Some(20));
    }

    #[test]
    fn test_newline_index_array() {
        // Layout: "[\n  1,\n  2,\n  3\n]"
        // Pos 0: '[' (line 1)
        // Pos 1: '\n'
        // Pos 2-5: '  1,' (line 2)
        // Pos 6: '\n'
        // Pos 7-10: '  2,' (line 3)
        // Pos 11: '\n'
        // Pos 12-14: '  3' (line 4)
        // Pos 15: '\n'
        // Pos 16: ']' (line 5)
        let json = b"[\n  1,\n  2,\n  3\n]";
        let index = JsonIndex::build(json);

        // Line 1: '['
        assert_eq!(index.to_line_column(0), (1, 1));

        // Line 2: '  1,' starts at position 2
        assert_eq!(index.to_line_column(2), (2, 1));
        assert_eq!(index.to_line_column(5), (2, 4)); // the comma

        // Line 3: '  2,' starts at position 7
        assert_eq!(index.to_line_column(7), (3, 1));

        // Line 4: '  3' starts at position 12
        assert_eq!(index.to_line_column(12), (4, 1));

        // Line 5: ']' starts at position 16
        assert_eq!(index.to_line_column(16), (5, 1));

        // Reverse lookup
        assert_eq!(index.to_offset(1, 1), Some(0));
        assert_eq!(index.to_offset(2, 1), Some(2));
        assert_eq!(index.to_offset(3, 1), Some(7));
        assert_eq!(index.to_offset(5, 1), Some(16));
    }

    #[test]
    fn test_newline_index_crlf() {
        let json = b"{\r\n\"a\": 1\r\n}";
        let index = JsonIndex::build(json);

        // Line 1: '{'
        assert_eq!(index.to_line_column(0), (1, 1));

        // Line 2: '"a": 1' (starts at position 3, after \r\n)
        assert_eq!(index.to_line_column(3), (2, 1));
        assert_eq!(index.to_offset(2, 1), Some(3));

        // Line 3: '}' (starts at position 11, after \r\n)
        assert_eq!(index.to_line_column(11), (3, 1));
        assert_eq!(index.to_offset(3, 1), Some(11));
    }

    #[test]
    fn test_newline_index_invalid_inputs() {
        let json = b"{\n\"a\": 1\n}";
        let index = JsonIndex::build(json);

        assert_eq!(index.to_offset(0, 1), None); // line 0 invalid
        assert_eq!(index.to_offset(1, 0), None); // column 0 invalid
    }

    #[test]
    fn test_newline_index_round_trip() {
        let json =
            b"{\n  \"users\": [\n    {\"name\": \"Alice\"},\n    {\"name\": \"Bob\"}\n  ]\n}";
        let index = JsonIndex::build(json);

        // Test round-trip: offset -> line/column -> offset
        for offset in 0..json.len() {
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

    // ========================================================================
    // Sequential Cursor Tests (O1 optimization)
    // ========================================================================

    #[test]
    fn test_sequential_cursor_basic() {
        // Test that sequential ib_select1_from calls work correctly
        let json = br#"[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]"#;
        let index = JsonIndex::build(json);

        // Sequential access: k = 0, 1, 2, ...
        let mut prev_pos = 0;
        for k in 0..11 {
            // 11 IB bits: '[', 1, 2, 3, 4, 5, 6, 7, 8, 9, 10
            let pos = index.ib_select1_from(k, prev_pos / 64);
            assert!(pos.is_some(), "ib_select1_from({}) should succeed", k);
            if k > 0 {
                assert!(
                    pos.unwrap() > prev_pos,
                    "positions should increase: {} > {}",
                    pos.unwrap(),
                    prev_pos
                );
            }
            prev_pos = pos.unwrap();
        }
    }

    #[test]
    fn test_sequential_cursor_forward_gap() {
        // Test forward gap: access k=0, then k=5 (skip 1,2,3,4)
        let json = br#"[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]"#;
        let index = JsonIndex::build(json);

        // First access at k=0
        let pos0 = index.ib_select1_from(0, 0);
        assert!(pos0.is_some());

        // Skip ahead to k=5 (forward gap)
        let pos5 = index.ib_select1_from(5, 0);
        assert!(pos5.is_some());
        assert!(pos5.unwrap() > pos0.unwrap());

        // Continue sequentially from k=5
        let pos6 = index.ib_select1_from(6, pos5.unwrap() / 64);
        assert!(pos6.is_some());
        assert!(pos6.unwrap() > pos5.unwrap());
    }

    #[test]
    fn test_sequential_cursor_backwards_jump() {
        // Test backwards jump: access k=5, then k=2
        let json = br#"[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]"#;
        let index = JsonIndex::build(json);

        // First access at k=5
        let pos5 = index.ib_select1_from(5, 0);
        assert!(pos5.is_some());

        // Jump backwards to k=2
        let pos2 = index.ib_select1_from(2, 0);
        assert!(pos2.is_some());
        assert!(pos2.unwrap() < pos5.unwrap());

        // Verify we can continue from k=2 sequentially
        let pos3 = index.ib_select1_from(3, pos2.unwrap() / 64);
        assert!(pos3.is_some());
        assert!(pos3.unwrap() > pos2.unwrap());
    }

    #[test]
    fn test_sequential_cursor_mixed_patterns() {
        // Test mixed access patterns
        let json = br#"{"a": [1, 2, 3], "b": [4, 5, 6], "c": [7, 8, 9]}"#;
        let index = JsonIndex::build(json);

        // Collect all positions using both methods
        let mut sequential_positions = Vec::new();
        let mut random_positions = Vec::new();

        // Sequential access
        for k in 0..20 {
            if let Some(pos) = index.ib_select1_from(k, 0) {
                sequential_positions.push(pos);
            }
        }

        // Random access (using ib_select1)
        for k in 0..20 {
            if let Some(pos) = index.ib_select1(k) {
                random_positions.push(pos);
            }
        }

        // Both methods should return the same positions
        assert_eq!(
            sequential_positions, random_positions,
            "Sequential and random access should return same positions"
        );
    }

    #[test]
    fn test_sequential_cursor_large_json() {
        // Test with larger JSON to exercise cursor across word boundaries
        let mut json = String::from("[");
        for i in 0..1000 {
            if i > 0 {
                json.push_str(", ");
            }
            json.push_str(&i.to_string());
        }
        json.push(']');
        let json = json.into_bytes();

        let index = JsonIndex::build(&json);

        // Sequential iteration through all elements
        let mut prev_pos = 0;
        for k in 0..1001 {
            // '[' + 1000 numbers
            let pos = index.ib_select1_from(k, prev_pos / 64);
            assert!(pos.is_some(), "k={} should succeed", k);
            if k > 0 {
                assert!(
                    pos.unwrap() > prev_pos,
                    "k={}: {} > {}",
                    k,
                    pos.unwrap(),
                    prev_pos
                );
            }
            prev_pos = pos.unwrap();
        }

        // Verify against random access
        for k in [0, 100, 500, 999, 1000] {
            let seq_pos = index.ib_select1_from(k, 0);
            let rand_pos = index.ib_select1(k);
            assert_eq!(
                seq_pos, rand_pos,
                "k={}: sequential and random should match",
                k
            );
        }
    }

    #[test]
    fn test_sequential_cursor_consistency() {
        // Verify cursor state doesn't corrupt results
        let json = br#"{"users": [{"name": "Alice"}, {"name": "Bob"}]}"#;
        let index = JsonIndex::build(json);

        // Access pattern: 0, 1, 2, back to 0, 3, 4, 5
        let p0a = index.ib_select1_from(0, 0);
        let p1 = index.ib_select1_from(1, 0);
        let p2 = index.ib_select1_from(2, 0);
        let p0b = index.ib_select1_from(0, 0); // backwards
        let p3 = index.ib_select1_from(3, 0);
        let p4 = index.ib_select1_from(4, 0);
        let p5 = index.ib_select1_from(5, 0);

        // Both accesses to k=0 should return the same position
        assert_eq!(p0a, p0b, "k=0 should return same position");

        // Verify ordering
        assert!(p0a.unwrap() < p1.unwrap());
        assert!(p1.unwrap() < p2.unwrap());
        assert!(p2.unwrap() < p3.unwrap());
        assert!(p3.unwrap() < p4.unwrap());
        assert!(p4.unwrap() < p5.unwrap());
    }
}
