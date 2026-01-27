//! Advance Index for memory-efficient open-to-text position mapping.
//!
//! This module provides `OpenPositions`, a space-efficient alternative to `Vec<u32>`
//! for storing BP open index to text position mappings in YAML. Unlike JSON, YAML containers share
//! text positions with their first child, creating duplicate entries.
//!
//! # Structure
//!
//! For monotonically non-decreasing positions, uses two bitmaps (Advance Index):
//!
//! - **IB (Interest Bits)**: One bit per text byte, set at each unique node start position
//! - **Advance bitmap**: One bit per BP open, set if this BP advances to a new text position
//!
//! For non-monotonic positions (e.g., explicit keys with `?`), falls back to `Vec<u32>`.
//!
//! # Example
//!
//! ```text
//! Text positions:  0  0  9  9  9  9
//! Advance bits:    1  0  1  0  0  0
//! IB bits set at:  positions 0 and 9 in text
//! ```
//!
//! # Memory
//!
//! With typical YAML density (N opens, L text bytes, N ≈ L/7.5):
//! - Dense `Vec<u32>`: 4N bytes
//! - Advance Index: L/8 + N/8 ≈ 1.1N bytes
//! - **~3.5× smaller** (when monotonic)
//!
//! # Performance
//!
//! - Random access: O(1) using select samples
//! - Sequential iteration: O(1) amortized, with O(1) fast path for duplicates

#[cfg(not(test))]
use alloc::vec;
#[cfg(not(test))]
use alloc::vec::Vec;

use crate::util::broadword::select_in_word;

/// Select sample rate - one sample per this many 1-bits.
/// Trade-off: lower = faster random access, more memory.
pub(super) const SELECT_SAMPLE_RATE: usize = 256;

/// Open position storage with automatic optimization.
///
/// Uses memory-efficient Advance Index encoding when positions are monotonically
/// non-decreasing, otherwise falls back to dense `Vec<u32>` storage.
///
/// This handles YAML edge cases like explicit keys (`?`) where the parser
/// may emit positions out of text order.
#[derive(Clone, Debug)]
pub enum OpenPositions {
    /// Memory-efficient encoding for monotonic positions (~3.5× compression).
    Compact(AdvancePositions),
    /// Fallback for non-monotonic positions (explicit keys, etc.).
    Dense(Vec<u32>),
}

impl OpenPositions {
    /// Build from a slice of open positions.
    ///
    /// Automatically chooses compact or dense storage based on position monotonicity.
    pub fn build(positions: &[u32], text_len: usize) -> Self {
        // Check if positions are monotonically non-decreasing
        let is_monotonic = positions.windows(2).all(|w| w[0] <= w[1]);

        if is_monotonic {
            OpenPositions::Compact(AdvancePositions::build_unchecked(positions, text_len))
        } else {
            OpenPositions::Dense(positions.to_vec())
        }
    }

    /// Returns the number of BP opens.
    #[inline]
    pub fn len(&self) -> usize {
        match self {
            OpenPositions::Compact(ap) => ap.len(),
            OpenPositions::Dense(v) => v.len(),
        }
    }

    /// Returns true if there are no positions.
    #[inline]
    pub fn is_empty(&self) -> bool {
        match self {
            OpenPositions::Compact(ap) => ap.is_empty(),
            OpenPositions::Dense(v) => v.is_empty(),
        }
    }

    /// Get the text position for the `open_idx`-th BP open.
    #[inline]
    pub fn get(&self, open_idx: usize) -> Option<u32> {
        match self {
            OpenPositions::Compact(ap) => ap.get(open_idx),
            OpenPositions::Dense(v) => v.get(open_idx).copied(),
        }
    }

    /// Find the last (deepest) open index that starts at the given text position.
    ///
    /// Returns `None` if no node starts at this position.
    pub fn find_last_open_at_text_pos(&self, text_pos: usize) -> Option<usize> {
        match self {
            OpenPositions::Compact(ap) => ap.find_last_open_at_text_pos(text_pos),
            OpenPositions::Dense(v) => {
                // Binary search for non-compact storage
                let text_pos_u32 = text_pos as u32;
                let search_result = v.binary_search(&text_pos_u32);

                match search_result {
                    Ok(idx) => {
                        // Found a match, scan right to find the last one
                        let mut last = idx;
                        while last + 1 < v.len() && v[last + 1] == text_pos_u32 {
                            last += 1;
                        }
                        Some(last)
                    }
                    Err(_) => None,
                }
            }
        }
    }

    /// Returns the heap memory usage in bytes.
    pub fn heap_size(&self) -> usize {
        match self {
            OpenPositions::Compact(ap) => ap.heap_size(),
            OpenPositions::Dense(v) => v.len() * 4,
        }
    }

    /// Returns true if using compact (Advance Index) storage.
    #[inline]
    pub fn is_compact(&self) -> bool {
        matches!(self, OpenPositions::Compact(_))
    }
}

/// Advance Index for memory-efficient BP-to-text position mapping.
///
/// Stores positions using two bitmaps instead of a dense `Vec<u32>`,
/// achieving ~3.5× compression for typical YAML files.
#[derive(Clone, Debug)]
pub struct AdvancePositions {
    /// Interest bits - one bit per text byte, set at unique node positions.
    ib_words: Vec<u64>,
    /// Number of valid bits in IB (= text length).
    ib_len: usize,
    /// Cumulative popcount for IB. O(1) rank via single array lookup.
    ib_rank: Vec<u32>,
    /// Sampled select index for IB: entry i = position of (i * SAMPLE_RATE)-th 1-bit.
    ib_select_samples: Vec<u32>,
    /// Total number of unique positions (1-bits in IB).
    ib_ones: usize,

    /// Advance bitmap - one bit per BP open, set when position advances.
    advance_words: Vec<u64>,
    /// Number of BP opens.
    num_opens: usize,
    /// Cumulative popcount for advance bitmap. O(1) rank via single array lookup.
    advance_rank: Vec<u32>,
}

/// Cursor for efficient sequential iteration through positions.
///
/// Maintains state to enable O(1) `advance_one()` when the advance bit is 0
/// (duplicate position), avoiding IB lookup entirely.
#[derive(Clone, Debug)]
pub struct AdvancePositionsCursor<'a> {
    positions: &'a AdvancePositions,
    /// Current open index (0-based).
    open_idx: usize,
    /// Cached text position for current open.
    text_pos: usize,
    /// Current advance rank (number of advances up to current position).
    advance_rank: usize,
    /// Current word index in IB.
    ib_word_idx: usize,
    /// Remaining bits in current IB word (after current position).
    ib_remaining_bits: u64,
}

impl AdvancePositions {
    /// Build an Advance Index from a slice of BP-to-text positions.
    ///
    /// The input must be monotonically non-decreasing (duplicates allowed).
    /// For potentially non-monotonic input, use `OpenPositions::build()` which
    /// automatically falls back to dense storage when needed.
    ///
    /// # Arguments
    ///
    /// * `positions` - Monotonically non-decreasing text positions for each BP open
    /// * `text_len` - Length of the source text (for IB sizing)
    ///
    /// # Panics
    ///
    /// In debug builds, panics if positions are not monotonically non-decreasing.
    pub fn build_unchecked(positions: &[u32], text_len: usize) -> Self {
        // Debug: verify monotonicity
        debug_assert!(
            positions.windows(2).all(|w| w[0] <= w[1]),
            "AdvancePositions::build_unchecked requires monotonically non-decreasing positions"
        );
        if positions.is_empty() {
            return Self {
                ib_words: Vec::new(),
                ib_len: text_len,
                ib_rank: vec![0],
                ib_select_samples: Vec::new(),
                ib_ones: 0,
                advance_words: Vec::new(),
                num_opens: 0,
                advance_rank: vec![0],
            };
        }

        let num_opens = positions.len();

        // Build IB: set bit at each unique position
        let ib_num_words = text_len.div_ceil(64);
        let mut ib_words = vec![0u64; ib_num_words];

        // Build advance bitmap: set bit when position changes
        let advance_num_words = num_opens.div_ceil(64);
        let mut advance_words = vec![0u64; advance_num_words];

        let mut prev_pos: Option<u32> = None;
        let mut ib_ones = 0usize;

        for (i, &pos) in positions.iter().enumerate() {
            let is_new_position = prev_pos != Some(pos);

            if is_new_position {
                // Set IB bit at this text position
                let word_idx = pos as usize / 64;
                let bit_idx = pos as usize % 64;
                if word_idx < ib_words.len() {
                    // Only set if not already set (handles edge case of non-monotonic input)
                    if (ib_words[word_idx] >> bit_idx) & 1 == 0 {
                        ib_words[word_idx] |= 1u64 << bit_idx;
                        ib_ones += 1;
                    }
                }

                // Set advance bit
                let adv_word_idx = i / 64;
                let adv_bit_idx = i % 64;
                advance_words[adv_word_idx] |= 1u64 << adv_bit_idx;
            }

            prev_pos = Some(pos);
        }

        // Build cumulative rank for IB
        let ib_rank = build_cumulative_rank(&ib_words);

        // Build cumulative rank for advance (small bitmap: one bit per BP open)
        let advance_rank = build_cumulative_rank(&advance_words);

        // Build select samples for IB
        let ib_select_samples = build_select_samples(&ib_words, ib_ones);

        Self {
            ib_words,
            ib_len: text_len,
            ib_rank,
            ib_select_samples,
            ib_ones,
            advance_words,
            num_opens,
            advance_rank,
        }
    }

    /// Returns the number of BP opens.
    #[inline]
    pub fn len(&self) -> usize {
        self.num_opens
    }

    /// Returns true if there are no positions.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.num_opens == 0
    }

    /// Get the text position for the `open_idx`-th BP open.
    ///
    /// Returns `None` if `open_idx >= len()`.
    #[inline]
    pub fn get(&self, open_idx: usize) -> Option<u32> {
        if open_idx >= self.num_opens {
            return None;
        }

        // Count advances up to and including this position
        let advance_count = self.advance_rank1(open_idx + 1);

        if advance_count == 0 {
            return None; // Shouldn't happen with valid data
        }

        // Find the (advance_count - 1)-th unique position in IB
        let ib_pos = self.ib_select1(advance_count - 1)?;
        Some(ib_pos as u32)
    }

    /// Create a cursor starting at open index 0.
    #[inline]
    pub fn cursor(&self) -> AdvancePositionsCursor<'_> {
        if self.is_empty() {
            return AdvancePositionsCursor {
                positions: self,
                open_idx: 0,
                text_pos: 0,
                advance_rank: 0,
                ib_word_idx: 0,
                ib_remaining_bits: 0,
            };
        }

        // First position is always an advance (advance bit 0 is always set)
        let text_pos = self.ib_select1(0).unwrap_or(0);
        let ib_word_idx = text_pos / 64;
        let ib_bit_offset = text_pos % 64;

        let ib_remaining_bits = if ib_word_idx < self.ib_words.len() {
            // Mask to keep bits at and after current position
            self.ib_words[ib_word_idx] & !((1u64 << ib_bit_offset) - 1)
        } else {
            0
        };

        AdvancePositionsCursor {
            positions: self,
            open_idx: 0,
            text_pos,
            advance_rank: 1, // We've seen one advance (the first position)
            ib_word_idx,
            ib_remaining_bits,
        }
    }

    /// Create a cursor starting at the given open index.
    #[inline]
    pub fn cursor_from(&self, open_idx: usize) -> AdvancePositionsCursor<'_> {
        if open_idx >= self.num_opens {
            return AdvancePositionsCursor {
                positions: self,
                open_idx: self.num_opens,
                text_pos: 0,
                advance_rank: self.ib_ones,
                ib_word_idx: self.ib_words.len(),
                ib_remaining_bits: 0,
            };
        }

        if open_idx == 0 {
            return self.cursor();
        }

        // Count advances up to and including this position
        let advance_rank = self.advance_rank1(open_idx + 1);
        let text_pos = if advance_rank > 0 {
            self.ib_select1(advance_rank - 1).unwrap_or(0)
        } else {
            0
        };

        let ib_word_idx = text_pos / 64;
        let ib_bit_offset = text_pos % 64;

        let ib_remaining_bits = if ib_word_idx < self.ib_words.len() {
            self.ib_words[ib_word_idx] & !((1u64 << ib_bit_offset) - 1)
        } else {
            0
        };

        AdvancePositionsCursor {
            positions: self,
            open_idx,
            text_pos,
            advance_rank,
            ib_word_idx,
            ib_remaining_bits,
        }
    }

    /// Count 1-bits in advance bitmap in [0, pos).
    #[inline]
    fn advance_rank1(&self, pos: usize) -> usize {
        if pos == 0 {
            return 0;
        }

        let word_idx = pos / 64;
        let bit_idx = pos % 64;

        // Use cumulative rank for full words (single array lookup)
        let mut count = self.advance_rank[word_idx.min(self.advance_words.len())] as usize;

        // Add partial word
        if word_idx < self.advance_words.len() && bit_idx > 0 {
            let mask = (1u64 << bit_idx) - 1;
            count += (self.advance_words[word_idx] & mask).count_ones() as usize;
        }

        count
    }

    /// Check if advance bit is set at position.
    #[inline]
    fn advance_bit(&self, pos: usize) -> bool {
        if pos >= self.num_opens {
            return false;
        }
        let word_idx = pos / 64;
        let bit_idx = pos % 64;
        (self.advance_words[word_idx] >> bit_idx) & 1 == 1
    }

    /// Select the k-th 1-bit in IB (0-indexed).
    #[inline]
    fn ib_select1(&self, k: usize) -> Option<usize> {
        if k >= self.ib_ones {
            return None;
        }

        // Use select samples to find starting word
        let sample_idx = k / SELECT_SAMPLE_RATE;
        let (start_word, mut remaining) = if sample_idx < self.ib_select_samples.len() {
            let sample_pos = self.ib_select_samples[sample_idx] as usize;
            let skip_ones = sample_idx * SELECT_SAMPLE_RATE;
            (sample_pos / 64, k - skip_ones)
        } else {
            (0, k)
        };

        // Scan from sample position
        for word_idx in start_word..self.ib_words.len() {
            let word = if word_idx == start_word && sample_idx < self.ib_select_samples.len() {
                // Mask off bits before sample position
                let sample_pos = self.ib_select_samples[sample_idx] as usize;
                let bit_offset = sample_pos % 64;
                self.ib_words[word_idx] & !((1u64 << bit_offset) - 1)
            } else {
                self.ib_words[word_idx]
            };

            let ones = word.count_ones() as usize;
            if ones > remaining {
                let bit_pos = select_in_word(word, remaining as u32) as usize;
                return Some(word_idx * 64 + bit_pos);
            }
            remaining -= ones;
        }

        None
    }

    /// Returns the heap memory usage in bytes.
    pub fn heap_size(&self) -> usize {
        self.ib_words.len() * 8
            + self.ib_rank.len() * 4
            + self.ib_select_samples.len() * 4
            + self.advance_words.len() * 8
            + self.advance_rank.len() * 4
    }

    /// Find the last (deepest) open index that starts at the given text position.
    ///
    /// Returns `None` if no node starts at this position.
    ///
    /// This is used for reverse lookup (text position → BP position) in `yq-locate`.
    pub fn find_last_open_at_text_pos(&self, text_pos: usize) -> Option<usize> {
        if self.num_opens == 0 || text_pos >= self.ib_len {
            return None;
        }

        // Check if there's an IB bit at this position
        let word_idx = text_pos / 64;
        let bit_idx = text_pos % 64;
        if word_idx >= self.ib_words.len() {
            return None;
        }
        if (self.ib_words[word_idx] >> bit_idx) & 1 == 0 {
            return None; // No node starts at this position
        }

        // Count IB bits before this position to get the unique position index
        let ib_rank = self.ib_rank1(text_pos);

        // Find the first open_idx with this unique position
        // The (ib_rank)-th 1-bit in advance gives the first open at this position
        let first_open = self.advance_select1(ib_rank)?;

        // Scan forward to find the last open with the same position
        // (advance bit = 0 means same position as previous)
        let mut last_open = first_open;
        while last_open + 1 < self.num_opens && !self.advance_bit(last_open + 1) {
            last_open += 1;
        }

        Some(last_open)
    }

    /// Count 1-bits in IB in [0, pos).
    #[inline]
    fn ib_rank1(&self, pos: usize) -> usize {
        if pos == 0 {
            return 0;
        }

        let word_idx = pos / 64;
        let bit_idx = pos % 64;

        // Use cumulative rank for full words (single array lookup)
        let mut count = self.ib_rank[word_idx.min(self.ib_words.len())] as usize;

        // Add partial word
        if word_idx < self.ib_words.len() && bit_idx > 0 {
            let mask = (1u64 << bit_idx) - 1;
            count += (self.ib_words[word_idx] & mask).count_ones() as usize;
        }

        count
    }

    /// Select the k-th 1-bit in advance bitmap (0-indexed).
    #[inline]
    fn advance_select1(&self, k: usize) -> Option<usize> {
        let total_advances = *self.advance_rank.last().unwrap_or(&0) as usize;
        if k >= total_advances {
            return None;
        }

        // Linear scan through words (no select samples for advance bitmap)
        let mut remaining = k;
        for (word_idx, &word) in self.advance_words.iter().enumerate() {
            let ones = word.count_ones() as usize;
            if ones > remaining {
                let bit_pos = select_in_word(word, remaining as u32) as usize;
                return Some(word_idx * 64 + bit_pos);
            }
            remaining -= ones;
        }

        None
    }
}

impl<'a> AdvancePositionsCursor<'a> {
    /// Returns the current text position, or `None` if exhausted.
    #[inline]
    pub fn current(&self) -> Option<u32> {
        if self.open_idx >= self.positions.num_opens {
            return None;
        }
        Some(self.text_pos as u32)
    }

    /// Returns the current open index.
    #[inline]
    pub fn index(&self) -> usize {
        self.open_idx
    }

    /// Returns true if the cursor has been exhausted.
    #[inline]
    pub fn is_exhausted(&self) -> bool {
        self.open_idx >= self.positions.num_opens
    }

    /// Advance to the next position.
    ///
    /// Returns the new text position, or `None` if exhausted.
    ///
    /// **O(1) fast path**: When the next position is a duplicate (advance bit = 0),
    /// returns the cached position without any bitmap lookups.
    #[inline]
    pub fn advance_one(&mut self) -> Option<u32> {
        if self.open_idx + 1 >= self.positions.num_opens {
            self.open_idx = self.positions.num_opens;
            return None;
        }

        self.open_idx += 1;

        // Fast path: if advance bit is 0, position is same as previous
        if !self.positions.advance_bit(self.open_idx) {
            return Some(self.text_pos as u32);
        }

        // Slow path: advance to next IB position
        self.advance_rank += 1;
        self.advance_ib_cursor();

        Some(self.text_pos as u32)
    }

    /// Advance the IB cursor to the next set bit.
    #[inline]
    fn advance_ib_cursor(&mut self) {
        // Clear current bit
        self.ib_remaining_bits &= self.ib_remaining_bits.wrapping_sub(1);

        if self.ib_remaining_bits != 0 {
            // Next bit is in the same word
            let bit_pos = self.ib_remaining_bits.trailing_zeros() as usize;
            self.text_pos = self.ib_word_idx * 64 + bit_pos;
        } else {
            // Scan to next word with 1-bits
            self.ib_word_idx += 1;
            while self.ib_word_idx < self.positions.ib_words.len()
                && self.positions.ib_words[self.ib_word_idx] == 0
            {
                self.ib_word_idx += 1;
            }

            if self.ib_word_idx < self.positions.ib_words.len() {
                self.ib_remaining_bits = self.positions.ib_words[self.ib_word_idx];
                let bit_pos = self.ib_remaining_bits.trailing_zeros() as usize;
                self.text_pos = self.ib_word_idx * 64 + bit_pos;
            }
        }
    }
}

/// Build cumulative popcount index for a bitvector.
/// Returns a vector where entry i = total 1-bits in words [0, i).
/// This gives O(1) rank queries via a single array lookup.
pub(super) fn build_cumulative_rank(words: &[u64]) -> Vec<u32> {
    let mut rank = Vec::with_capacity(words.len() + 1);
    let mut cumulative: u32 = 0;
    rank.push(0);
    for &word in words {
        cumulative += word.count_ones();
        rank.push(cumulative);
    }
    rank
}

/// Build select samples for a bitvector.
pub(super) fn build_select_samples(words: &[u64], total_ones: usize) -> Vec<u32> {
    if total_ones == 0 {
        return Vec::new();
    }

    let num_samples = total_ones.div_ceil(SELECT_SAMPLE_RATE);
    let mut samples = Vec::with_capacity(num_samples);

    let mut ones_seen = 0usize;
    let mut sample_target = 0usize;

    'outer: for (word_idx, &word) in words.iter().enumerate() {
        if word == 0 {
            continue;
        }

        let word_ones = word.count_ones() as usize;

        while sample_target < ones_seen + word_ones {
            let local_rank = sample_target - ones_seen;
            let bit_pos = select_in_word(word, local_rank as u32) as usize;
            let global_pos = word_idx * 64 + bit_pos;
            samples.push(global_pos as u32);

            sample_target += SELECT_SAMPLE_RATE;
            if samples.len() >= num_samples {
                break 'outer;
            }
        }

        ones_seen += word_ones;
    }

    samples
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty() {
        let ap = AdvancePositions::build_unchecked(&[], 0);
        assert!(ap.is_empty());
        assert_eq!(ap.len(), 0);
        assert_eq!(ap.get(0), None);

        let cursor = ap.cursor();
        assert!(cursor.is_exhausted());
        assert_eq!(cursor.current(), None);
    }

    #[test]
    fn test_single_position() {
        let ap = AdvancePositions::build_unchecked(&[42], 100);
        assert_eq!(ap.len(), 1);
        assert_eq!(ap.get(0), Some(42));
        assert_eq!(ap.get(1), None);

        let mut cursor = ap.cursor();
        assert_eq!(cursor.current(), Some(42));
        assert_eq!(cursor.advance_one(), None);
        assert!(cursor.is_exhausted());
    }

    #[test]
    fn test_no_duplicates() {
        let positions = vec![0, 10, 20, 30, 40];
        let ap = AdvancePositions::build_unchecked(&positions, 100);

        assert_eq!(ap.len(), 5);

        // Test random access
        for (i, &expected) in positions.iter().enumerate() {
            assert_eq!(ap.get(i), Some(expected), "get({}) failed", i);
        }

        // Test cursor iteration
        let mut cursor = ap.cursor();
        for &expected in &positions {
            assert_eq!(cursor.current(), Some(expected));
            cursor.advance_one();
        }
        assert!(cursor.is_exhausted());
    }

    #[test]
    fn test_with_duplicates() {
        // Typical YAML pattern: containers share position with first child
        let positions = vec![0, 0, 10, 10, 10, 20];
        let ap = AdvancePositions::build_unchecked(&positions, 100);

        assert_eq!(ap.len(), 6);

        // Test random access
        for (i, &expected) in positions.iter().enumerate() {
            assert_eq!(ap.get(i), Some(expected), "get({}) failed", i);
        }

        // Test cursor iteration
        let mut cursor = ap.cursor();
        for &expected in &positions {
            assert_eq!(cursor.current(), Some(expected));
            cursor.advance_one();
        }
        assert!(cursor.is_exhausted());
    }

    #[test]
    fn test_all_duplicates() {
        // Edge case: all positions are the same
        let positions = vec![5, 5, 5, 5, 5];
        let ap = AdvancePositions::build_unchecked(&positions, 100);

        assert_eq!(ap.len(), 5);

        for i in 0..5 {
            assert_eq!(ap.get(i), Some(5));
        }

        let mut cursor = ap.cursor();
        for _ in 0..5 {
            assert_eq!(cursor.current(), Some(5));
            cursor.advance_one();
        }
        assert!(cursor.is_exhausted());
    }

    #[test]
    fn test_cursor_from() {
        let positions = vec![0, 0, 10, 20, 20, 30];
        let ap = AdvancePositions::build_unchecked(&positions, 100);

        // Test cursor_from at various positions
        for (i, &expected) in positions.iter().enumerate() {
            let cursor = ap.cursor_from(i);
            assert_eq!(cursor.index(), i);
            assert_eq!(
                cursor.current(),
                Some(expected),
                "cursor_from({}) failed",
                i
            );
        }

        // cursor_from past end
        let cursor = ap.cursor_from(10);
        assert!(cursor.is_exhausted());
    }

    #[test]
    fn test_cursor_from_then_iterate() {
        let positions: Vec<u32> = (0..100).map(|i| i * 10).collect();
        let ap = AdvancePositions::build_unchecked(&positions, 1000);

        // Start from middle, iterate to end
        let mut cursor = ap.cursor_from(50);
        let mut collected = Vec::new();
        while let Some(v) = cursor.current() {
            collected.push(v);
            cursor.advance_one();
        }
        assert_eq!(collected, positions[50..].to_vec());
    }

    #[test]
    fn test_memory_savings() {
        // Simulate realistic YAML with duplicates
        let mut positions = Vec::with_capacity(1000);
        let mut pos = 0u32;
        for i in 0..1000 {
            positions.push(pos);
            // Every 3rd position is a duplicate (container sharing)
            if i % 3 != 0 {
                pos += 10 + (i as u32 % 20);
            }
        }

        let ap = AdvancePositions::build_unchecked(&positions, pos as usize + 100);

        let vec_size = positions.len() * 4;
        let ap_size = ap.heap_size();

        eprintln!(
            "Memory: Vec<u32>={} bytes, AdvancePositions={} bytes ({:.2}x compression)",
            vec_size,
            ap_size,
            vec_size as f64 / ap_size as f64
        );

        // Should achieve meaningful compression
        assert!(
            ap_size < vec_size,
            "AdvancePositions {} should be smaller than Vec<u32> {}",
            ap_size,
            vec_size
        );
    }

    #[test]
    fn test_large_positions() {
        // Test with positions near the end of a large file
        let positions = vec![0, 100_000, 200_000, 300_000];
        let ap = AdvancePositions::build_unchecked(&positions, 400_000);

        for (i, &expected) in positions.iter().enumerate() {
            assert_eq!(ap.get(i), Some(expected));
        }
    }

    #[test]
    fn test_many_elements_with_samples() {
        // Test with more elements than sample rate to exercise sampling
        let positions: Vec<u32> = (0..1000).map(|i| i * 5).collect();
        let ap = AdvancePositions::build_unchecked(&positions, 5000);

        // Random access should work with samples
        assert_eq!(ap.get(0), Some(0));
        assert_eq!(ap.get(256), Some(1280)); // At sample boundary
        assert_eq!(ap.get(512), Some(2560)); // At sample boundary
        assert_eq!(ap.get(999), Some(4995));

        // Verify all positions via cursor
        let mut cursor = ap.cursor();
        for &expected in &positions {
            assert_eq!(cursor.current(), Some(expected));
            cursor.advance_one();
        }
    }

    #[test]
    fn test_find_last_open_at_text_pos() {
        // Test reverse lookup: text position → open index
        let positions = vec![0, 0, 10, 10, 10, 20];
        let ap = AdvancePositions::build_unchecked(&positions, 100);

        // Position 0 has opens 0, 1 → should return last (1)
        assert_eq!(ap.find_last_open_at_text_pos(0), Some(1));

        // Position 10 has opens 2, 3, 4 → should return last (4)
        assert_eq!(ap.find_last_open_at_text_pos(10), Some(4));

        // Position 20 has opens 5 → should return 5
        assert_eq!(ap.find_last_open_at_text_pos(20), Some(5));

        // Position 5 has no opens → should return None
        assert_eq!(ap.find_last_open_at_text_pos(5), None);

        // Position out of range → should return None
        assert_eq!(ap.find_last_open_at_text_pos(100), None);
    }

    #[test]
    fn test_find_last_open_at_text_pos_no_duplicates() {
        let positions = vec![0, 10, 20, 30];
        let ap = AdvancePositions::build_unchecked(&positions, 100);

        assert_eq!(ap.find_last_open_at_text_pos(0), Some(0));
        assert_eq!(ap.find_last_open_at_text_pos(10), Some(1));
        assert_eq!(ap.find_last_open_at_text_pos(20), Some(2));
        assert_eq!(ap.find_last_open_at_text_pos(30), Some(3));
        assert_eq!(ap.find_last_open_at_text_pos(15), None);
    }

    #[test]
    fn test_find_last_open_at_text_pos_all_duplicates() {
        let positions = vec![5, 5, 5, 5, 5];
        let ap = AdvancePositions::build_unchecked(&positions, 100);

        // All opens are at position 5 → should return last (4)
        assert_eq!(ap.find_last_open_at_text_pos(5), Some(4));
        assert_eq!(ap.find_last_open_at_text_pos(0), None);
    }
}
