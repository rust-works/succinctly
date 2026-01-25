//! Elias-Fano encoding for monotone integer sequences with cursor-based access.
//!
//! This module provides space-efficient storage for monotonically increasing
//! sequences of integers (like `bp_to_text` positions) with O(1) amortized
//! forward iteration via a cursor.
//!
//! # Structure
//!
//! Each integer is split into high and low bits:
//! - **Low bits**: Stored densely in a packed array
//! - **High bits**: Unary encoded in a bitvector (one 1-bit per element)
//!
//! # Performance
//!
//! | Operation | Complexity | Notes |
//! |-----------|------------|-------|
//! | `get(i)` | O(1) | Uses select samples |
//! | `advance_one()` | O(1) amortized | Scans for next 1-bit |
//! | `advance_by(k)` | O(k/64) | Word-by-word scanning |
//! | `seek(i)` | O(1) | Uses select samples |
//!
//! # Example
//!
//! ```
//! use succinctly::bits::EliasFano;
//!
//! let positions = vec![0, 15, 23, 45, 52, 78, 120, 256];
//! let ef = EliasFano::build(&positions);
//!
//! // Random access
//! assert_eq!(ef.get(0), Some(0));
//! assert_eq!(ef.get(3), Some(45));
//!
//! // Cursor-based iteration
//! let mut cursor = ef.cursor();
//! assert_eq!(cursor.current(), Some(0));
//! assert_eq!(cursor.advance_one(), Some(15));
//! assert_eq!(cursor.advance_by(2), Some(45));
//! ```

#[cfg(not(test))]
use alloc::vec::Vec;

/// Select sample rate - one sample per this many elements.
/// Trade-off: lower = faster seek, more memory.
const SELECT_SAMPLE_RATE: usize = 256;

/// Elias-Fano encoded monotone integer sequence.
///
/// Provides O(1) random access and O(1) amortized forward iteration
/// with ~2-4x compression compared to `Vec<u32>`.
#[derive(Clone, Debug)]
pub struct EliasFano {
    /// Dense array of low bits, packed into 64-bit words.
    low_bits: Vec<u64>,
    /// Number of bits per low value.
    low_width: usize,
    /// High bits bitvector (unary encoded).
    /// Contains `len` 1-bits marking element positions.
    high_bits: Vec<u64>,
    /// Number of elements in the sequence.
    len: usize,
    /// Universe size (max value + 1).
    universe: u64,
    /// Sampled select index: entry i = position of (i * SAMPLE_RATE)-th 1-bit.
    select_samples: Vec<u32>,
}

/// Cursor for efficient forward iteration through Elias-Fano encoded data.
///
/// Maintains position state to enable O(1) amortized `advance_one()`.
#[derive(Clone, Debug)]
pub struct EliasFanoCursor<'a> {
    ef: &'a EliasFano,
    /// Current element index (0-based).
    idx: usize,
    /// Position of current element's 1-bit in high_bits bitvector.
    high_pos: usize,
    /// Index of current word in high_bits array.
    word_idx: usize,
    /// Bits remaining in current word after current position (masked).
    /// The lowest set bit corresponds to the current element.
    remaining_bits: u64,
}

impl EliasFano {
    /// Build Elias-Fano encoding from a slice of monotonically increasing values.
    ///
    /// # Panics
    ///
    /// Panics if the input is not monotonically increasing.
    pub fn build(values: &[u32]) -> Self {
        if values.is_empty() {
            return Self {
                low_bits: Vec::new(),
                low_width: 0,
                high_bits: Vec::new(),
                len: 0,
                universe: 0,
                select_samples: Vec::new(),
            };
        }

        // Verify monotonicity in debug builds
        debug_assert!(
            values.windows(2).all(|w| w[0] <= w[1]),
            "EliasFano::build requires monotonically increasing values"
        );

        let n = values.len();
        let max_value = *values.last().unwrap() as u64;
        let universe = max_value + 1;

        // Calculate bit widths
        // low_width = floor(log2(universe / n)), minimum 0
        let low_width = if n == 0 || universe <= n as u64 {
            0
        } else {
            (64 - (universe / n as u64).leading_zeros() as usize).saturating_sub(1)
        };

        let low_mask = if low_width == 0 {
            0
        } else {
            (1u64 << low_width) - 1
        };

        // Allocate low bits array
        let low_bits_total = n * low_width;
        let low_words = low_bits_total.div_ceil(64);
        let mut low_bits = vec![0u64; low_words];

        // Allocate high bits array
        // High bits bitvector has n 1-bits and approximately (max_value >> low_width) 0-bits
        let high_bits_len = n + (max_value >> low_width) as usize + 1;
        let high_words = high_bits_len.div_ceil(64);
        let mut high_bits = vec![0u64; high_words];

        // Encode each value
        for (i, &value) in values.iter().enumerate() {
            let v = value as u64;

            // Extract and store low bits
            if low_width > 0 {
                let low_value = v & low_mask;
                let bit_pos = i * low_width;
                let word_idx = bit_pos / 64;
                let bit_offset = bit_pos % 64;

                low_bits[word_idx] |= low_value << bit_offset;

                // Handle overflow into next word
                if bit_offset + low_width > 64 && word_idx + 1 < low_bits.len() {
                    low_bits[word_idx + 1] |= low_value >> (64 - bit_offset);
                }
            }

            // Set high bit
            // Position in high_bits = high_value + i = (v >> low_width) + i
            let high_value = v >> low_width;
            let high_pos = high_value as usize + i;
            let word_idx = high_pos / 64;
            let bit_offset = high_pos % 64;
            high_bits[word_idx] |= 1u64 << bit_offset;
        }

        // Build select samples
        let num_samples = n.div_ceil(SELECT_SAMPLE_RATE);
        let mut select_samples = Vec::with_capacity(num_samples);

        let mut ones_seen = 0usize;
        let mut sample_target = 0usize;

        'outer: for (word_idx, &word) in high_bits.iter().enumerate() {
            if word == 0 {
                continue;
            }

            let word_ones = word.count_ones() as usize;

            while sample_target < ones_seen + word_ones {
                // The sample_target-th 1-bit is in this word
                let local_rank = sample_target - ones_seen;
                let bit_pos = select_in_word(word, local_rank);
                let global_pos = word_idx * 64 + bit_pos;
                select_samples.push(global_pos as u32);

                sample_target += SELECT_SAMPLE_RATE;
                if select_samples.len() >= num_samples {
                    break 'outer;
                }
            }

            ones_seen += word_ones;
        }

        Self {
            low_bits,
            low_width,
            high_bits,
            len: n,
            universe,
            select_samples,
        }
    }

    /// Returns the number of elements in the sequence.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns true if the sequence is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the universe size (maximum value + 1).
    #[inline]
    pub fn universe(&self) -> u64 {
        self.universe
    }

    /// Random access to the i-th element.
    ///
    /// Returns `None` if `i >= len()`.
    #[inline]
    pub fn get(&self, i: usize) -> Option<u32> {
        if i >= self.len {
            return None;
        }

        // Find position of i-th 1-bit in high_bits using select
        let high_pos = self.select1(i);
        let high_value = high_pos - i;

        // Read low bits
        let low_value = self.read_low_bits(i);

        let value = ((high_value as u64) << self.low_width) | low_value;
        Some(value as u32)
    }

    /// Create a cursor starting at element 0.
    #[inline]
    pub fn cursor(&self) -> EliasFanoCursor<'_> {
        if self.is_empty() {
            return EliasFanoCursor {
                ef: self,
                idx: 0,
                high_pos: 0,
                word_idx: 0,
                remaining_bits: 0,
            };
        }

        // Find first 1-bit
        let mut word_idx = 0;
        while word_idx < self.high_bits.len() && self.high_bits[word_idx] == 0 {
            word_idx += 1;
        }

        let word = if word_idx < self.high_bits.len() {
            self.high_bits[word_idx]
        } else {
            0
        };

        let bit_pos = word.trailing_zeros() as usize;
        let high_pos = word_idx * 64 + bit_pos;

        EliasFanoCursor {
            ef: self,
            idx: 0,
            high_pos,
            word_idx,
            remaining_bits: word,
        }
    }

    /// Create a cursor starting at element `idx`.
    ///
    /// Returns a cursor positioned at the end if `idx >= len()`.
    #[inline]
    pub fn cursor_from(&self, idx: usize) -> EliasFanoCursor<'_> {
        if idx >= self.len {
            return EliasFanoCursor {
                ef: self,
                idx: self.len,
                high_pos: 0,
                word_idx: 0,
                remaining_bits: 0,
            };
        }

        if idx == 0 {
            return self.cursor();
        }

        // Use select samples to find starting position
        let high_pos = self.select1(idx);
        let word_idx = high_pos / 64;
        let bit_offset = high_pos % 64;

        // Mask to get bits at and after current position
        let word = self.high_bits[word_idx];
        let remaining_bits = word & !((1u64 << bit_offset) - 1);

        EliasFanoCursor {
            ef: self,
            idx,
            high_pos,
            word_idx,
            remaining_bits,
        }
    }

    /// Select1: find position of the k-th 1-bit (0-indexed).
    ///
    /// Uses sampled select index for O(1) average case.
    fn select1(&self, k: usize) -> usize {
        debug_assert!(k < self.len, "select1 out of bounds");

        // Use sample to find starting position
        let sample_idx = k / SELECT_SAMPLE_RATE;
        let (start_word, mut remaining) = if sample_idx < self.select_samples.len() {
            let sample_pos = self.select_samples[sample_idx] as usize;
            let skip_ones = sample_idx * SELECT_SAMPLE_RATE;
            (sample_pos / 64, k - skip_ones)
        } else {
            (0, k)
        };

        // Scan from sample position
        for word_idx in start_word..self.high_bits.len() {
            let word = if word_idx == start_word && sample_idx < self.select_samples.len() {
                // Mask off bits before sample position
                let sample_pos = self.select_samples[sample_idx] as usize;
                let bit_offset = sample_pos % 64;
                self.high_bits[word_idx] & !((1u64 << bit_offset) - 1)
            } else {
                self.high_bits[word_idx]
            };

            let ones = word.count_ones() as usize;
            if ones > remaining {
                let bit_pos = select_in_word(word, remaining);
                return word_idx * 64 + bit_pos;
            }
            remaining -= ones;
        }

        unreachable!("select1: not enough 1-bits");
    }

    /// Read low bits for element i.
    #[inline]
    fn read_low_bits(&self, i: usize) -> u64 {
        if self.low_width == 0 {
            return 0;
        }

        let bit_pos = i * self.low_width;
        let word_idx = bit_pos / 64;
        let bit_offset = bit_pos % 64;

        let low_mask = (1u64 << self.low_width) - 1;

        let mut value = (self.low_bits[word_idx] >> bit_offset) & low_mask;

        // Handle spanning two words
        if bit_offset + self.low_width > 64 && word_idx + 1 < self.low_bits.len() {
            let overflow_bits = bit_offset + self.low_width - 64;
            let overflow_mask = (1u64 << overflow_bits) - 1;
            value |=
                (self.low_bits[word_idx + 1] & overflow_mask) << (self.low_width - overflow_bits);
        }

        value
    }

    /// Returns the heap memory usage in bytes.
    pub fn heap_size(&self) -> usize {
        self.low_bits.len() * 8 + self.high_bits.len() * 8 + self.select_samples.len() * 4
    }
}

impl<'a> EliasFanoCursor<'a> {
    /// Returns the current element value, or `None` if exhausted.
    #[inline]
    pub fn current(&self) -> Option<u32> {
        if self.idx >= self.ef.len {
            return None;
        }

        let high_value = self.high_pos - self.idx;
        let low_value = self.ef.read_low_bits(self.idx);
        let value = ((high_value as u64) << self.ef.low_width) | low_value;
        Some(value as u32)
    }

    /// Returns the current element index.
    #[inline]
    pub fn index(&self) -> usize {
        self.idx
    }

    /// Returns true if the cursor has been exhausted.
    #[inline]
    pub fn is_exhausted(&self) -> bool {
        self.idx >= self.ef.len
    }

    /// Advance to the next element.
    ///
    /// Returns the new current value, or `None` if exhausted.
    #[inline]
    pub fn advance_one(&mut self) -> Option<u32> {
        if self.idx + 1 >= self.ef.len {
            self.idx = self.ef.len;
            return None;
        }

        // Clear the current bit (lowest set bit in remaining_bits)
        self.remaining_bits &= self.remaining_bits.wrapping_sub(1);

        if self.remaining_bits != 0 {
            // Next 1-bit is in the same word
            let bit_pos = self.remaining_bits.trailing_zeros() as usize;
            self.high_pos = self.word_idx * 64 + bit_pos;
        } else {
            // Scan to next word with 1-bits
            self.word_idx += 1;
            while self.word_idx < self.ef.high_bits.len() && self.ef.high_bits[self.word_idx] == 0 {
                self.word_idx += 1;
            }

            if self.word_idx >= self.ef.high_bits.len() {
                self.idx = self.ef.len;
                return None;
            }

            self.remaining_bits = self.ef.high_bits[self.word_idx];
            let bit_pos = self.remaining_bits.trailing_zeros() as usize;
            self.high_pos = self.word_idx * 64 + bit_pos;
        }

        self.idx += 1;
        self.current()
    }

    /// Advance by `k` elements.
    ///
    /// Returns the new current value, or `None` if exhausted.
    #[inline]
    pub fn advance_by(&mut self, k: usize) -> Option<u32> {
        if k == 0 {
            return self.current();
        }
        if k == 1 {
            return self.advance_one();
        }

        let target_idx = self.idx + k;
        if target_idx >= self.ef.len {
            self.idx = self.ef.len;
            return None;
        }

        // For large skips, use seek
        if k > 64 {
            return self.seek(target_idx);
        }

        // For small skips, scan forward counting 1-bits
        let mut remaining = k;

        // Count 1-bits remaining in current word (excluding current position)
        // Clear current bit first
        let word_after_current = self.remaining_bits & self.remaining_bits.wrapping_sub(1);
        let ones_in_current = word_after_current.count_ones() as usize;

        if ones_in_current >= remaining {
            // Target is in current word
            self.remaining_bits = word_after_current;
            for _ in 0..remaining - 1 {
                self.remaining_bits &= self.remaining_bits.wrapping_sub(1);
            }
            let bit_pos = self.remaining_bits.trailing_zeros() as usize;
            self.high_pos = self.word_idx * 64 + bit_pos;
            self.idx = target_idx;
            return self.current();
        }

        remaining -= ones_in_current;
        self.word_idx += 1;

        // Scan through subsequent words
        while self.word_idx < self.ef.high_bits.len() {
            let word = self.ef.high_bits[self.word_idx];
            let ones = word.count_ones() as usize;

            if ones >= remaining {
                // Target is in this word
                self.remaining_bits = word;
                for _ in 0..remaining - 1 {
                    self.remaining_bits &= self.remaining_bits.wrapping_sub(1);
                }
                let bit_pos = self.remaining_bits.trailing_zeros() as usize;
                self.high_pos = self.word_idx * 64 + bit_pos;
                self.idx = target_idx;
                return self.current();
            }

            remaining -= ones;
            self.word_idx += 1;
        }

        self.idx = self.ef.len;
        None
    }

    /// Seek to absolute element index.
    ///
    /// Returns the value at that index, or `None` if out of bounds.
    #[inline]
    pub fn seek(&mut self, idx: usize) -> Option<u32> {
        if idx >= self.ef.len {
            self.idx = self.ef.len;
            return None;
        }

        // Use select to find position
        let high_pos = self.ef.select1(idx);
        let word_idx = high_pos / 64;
        let bit_offset = high_pos % 64;

        self.idx = idx;
        self.high_pos = high_pos;
        self.word_idx = word_idx;
        self.remaining_bits = self.ef.high_bits[word_idx] & !((1u64 << bit_offset) - 1);

        self.current()
    }
}

/// Broadword select: find position of k-th set bit in a 64-bit word (0-indexed).
///
/// Uses a simple loop which is efficient for small k and portable.
/// For architecture-specific optimizations (PDEP+TZCNT on x86), see the simd module.
#[inline]
fn select_in_word(word: u64, k: usize) -> usize {
    if word == 0 {
        return 64;
    }

    // Simple loop implementation - efficient for small k (typical case)
    // and fully portable across all architectures
    let mut remaining = k;
    let mut w = word;

    loop {
        if remaining == 0 {
            return w.trailing_zeros() as usize;
        }
        // Clear lowest set bit
        w &= w.wrapping_sub(1);
        if w == 0 {
            return 64; // Not enough bits
        }
        remaining -= 1;
    }
}

/// Iterator adapter for EliasFano.
impl<'a> IntoIterator for &'a EliasFano {
    type Item = u32;
    type IntoIter = EliasFanoIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        EliasFanoIter {
            cursor: self.cursor(),
            started: false,
        }
    }
}

/// Iterator over EliasFano elements.
pub struct EliasFanoIter<'a> {
    cursor: EliasFanoCursor<'a>,
    started: bool,
}

impl<'a> Iterator for EliasFanoIter<'a> {
    type Item = u32;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if !self.started {
            self.started = true;
            self.cursor.current()
        } else {
            self.cursor.advance_one()
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.cursor.ef.len - self.cursor.idx;
        (remaining, Some(remaining))
    }
}

impl<'a> ExactSizeIterator for EliasFanoIter<'a> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty() {
        let ef = EliasFano::build(&[]);
        assert!(ef.is_empty());
        assert_eq!(ef.len(), 0);
        assert_eq!(ef.get(0), None);

        let cursor = ef.cursor();
        assert!(cursor.is_exhausted());
        assert_eq!(cursor.current(), None);
    }

    #[test]
    fn test_single_element() {
        let ef = EliasFano::build(&[42]);
        assert_eq!(ef.len(), 1);
        assert_eq!(ef.get(0), Some(42));
        assert_eq!(ef.get(1), None);

        let mut cursor = ef.cursor();
        assert_eq!(cursor.current(), Some(42));
        assert_eq!(cursor.advance_one(), None);
        assert!(cursor.is_exhausted());
    }

    #[test]
    fn test_sequential_values() {
        let values: Vec<u32> = (0..100).collect();
        let ef = EliasFano::build(&values);

        assert_eq!(ef.len(), 100);

        // Test random access
        for (i, &v) in values.iter().enumerate() {
            assert_eq!(ef.get(i), Some(v), "get({}) failed", i);
        }

        // Test cursor iteration
        let mut cursor = ef.cursor();
        for &expected in &values {
            assert_eq!(cursor.current(), Some(expected));
            cursor.advance_one();
        }
        assert!(cursor.is_exhausted());
    }

    #[test]
    fn test_sparse_values() {
        let values = vec![0, 15, 23, 45, 52, 78, 120, 256, 1000, 5000];
        let ef = EliasFano::build(&values);

        assert_eq!(ef.len(), values.len());

        // Test random access
        for (i, &v) in values.iter().enumerate() {
            assert_eq!(ef.get(i), Some(v), "get({}) failed", i);
        }

        // Test cursor iteration
        let collected: Vec<u32> = ef.into_iter().collect();
        assert_eq!(collected, values);
    }

    #[test]
    fn test_advance_by() {
        let values: Vec<u32> = (0..1000).map(|i| i * 10).collect();
        let ef = EliasFano::build(&values);

        let mut cursor = ef.cursor();
        assert_eq!(cursor.current(), Some(0));

        // Advance by 5
        assert_eq!(cursor.advance_by(5), Some(50));
        assert_eq!(cursor.index(), 5);

        // Advance by 10
        assert_eq!(cursor.advance_by(10), Some(150));
        assert_eq!(cursor.index(), 15);

        // Advance by 100 (uses seek path)
        assert_eq!(cursor.advance_by(100), Some(1150));
        assert_eq!(cursor.index(), 115);
    }

    #[test]
    fn test_seek() {
        let values: Vec<u32> = (0..500).map(|i| i * 7).collect();
        let ef = EliasFano::build(&values);

        let mut cursor = ef.cursor();

        // Seek forward
        assert_eq!(cursor.seek(100), Some(700));
        assert_eq!(cursor.index(), 100);

        // Seek backward (resets state)
        assert_eq!(cursor.seek(50), Some(350));
        assert_eq!(cursor.index(), 50);

        // Seek to end
        assert_eq!(cursor.seek(499), Some(3493));
        assert_eq!(cursor.index(), 499);

        // Seek past end
        assert_eq!(cursor.seek(500), None);
        assert!(cursor.is_exhausted());
    }

    #[test]
    fn test_cursor_from() {
        let values: Vec<u32> = (0..100).map(|i| i * 3).collect();
        let ef = EliasFano::build(&values);

        let cursor = ef.cursor_from(50);
        assert_eq!(cursor.index(), 50);
        assert_eq!(cursor.current(), Some(150));

        let cursor_end = ef.cursor_from(100);
        assert!(cursor_end.is_exhausted());
    }

    #[test]
    fn test_cursor_from_comprehensive() {
        // Test with realistic bp_to_text-like data
        let mut values = Vec::with_capacity(1000);
        let mut pos = 0u32;
        for i in 0..1000 {
            values.push(pos);
            // Varying gaps like real bp_to_text
            pos += if i % 10 == 0 {
                50 + (i as u32 % 100)
            } else {
                10 + (i as u32 % 20)
            };
        }
        let ef = EliasFano::build(&values);

        // cursor_from(0) should equal cursor()
        let cursor0 = ef.cursor_from(0);
        let cursor = ef.cursor();
        assert_eq!(cursor0.index(), cursor.index());
        assert_eq!(cursor0.current(), cursor.current());

        // Test various positions
        for &idx in &[0, 1, 10, 50, 100, 255, 256, 257, 500, 512, 513, 999] {
            let cursor = ef.cursor_from(idx);
            assert_eq!(cursor.index(), idx, "cursor_from({}).index()", idx);
            assert_eq!(
                cursor.current(),
                Some(values[idx]),
                "cursor_from({}).current()",
                idx
            );
        }

        // cursor_from past end
        assert!(ef.cursor_from(1000).is_exhausted());
        assert!(ef.cursor_from(1001).is_exhausted());
        assert!(ef.cursor_from(usize::MAX).is_exhausted());
    }

    #[test]
    fn test_cursor_from_then_iterate() {
        let values: Vec<u32> = (0..500).map(|i| i * 7).collect();
        let ef = EliasFano::build(&values);

        // Start from middle, iterate to end
        let mut cursor = ef.cursor_from(250);
        let mut collected = Vec::new();
        while let Some(v) = cursor.current() {
            collected.push(v);
            cursor.advance_one();
        }
        assert_eq!(collected, values[250..].to_vec());

        // Start from near end
        let mut cursor = ef.cursor_from(495);
        let mut collected = Vec::new();
        while let Some(v) = cursor.current() {
            collected.push(v);
            cursor.advance_one();
        }
        assert_eq!(collected, values[495..].to_vec());
    }

    #[test]
    fn test_cursor_from_then_advance_by() {
        let values: Vec<u32> = (0..1000).map(|i| i * 5).collect();
        let ef = EliasFano::build(&values);

        // Start from position 100, skip by 50
        let mut cursor = ef.cursor_from(100);
        assert_eq!(cursor.current(), Some(500));
        assert_eq!(cursor.advance_by(50), Some(750)); // position 150
        assert_eq!(cursor.index(), 150);
        assert_eq!(cursor.advance_by(100), Some(1250)); // position 250
        assert_eq!(cursor.index(), 250);

        // Skip to near end
        assert_eq!(cursor.advance_by(740), Some(4950)); // position 990
        assert_eq!(cursor.index(), 990);

        // Skip past end
        assert_eq!(cursor.advance_by(100), None);
        assert!(cursor.is_exhausted());
    }

    #[test]
    fn test_cursor_from_at_sample_boundaries() {
        // Create data that spans multiple select sample boundaries (every 256 elements)
        let values: Vec<u32> = (0..1000).map(|i| i * 5).collect();
        let ef = EliasFano::build(&values);

        // Test positions around sample boundaries
        for boundary in [256i32, 512, 768] {
            for offset in [-2, -1, 0, 1, 2] {
                let idx = (boundary + offset) as usize;
                if idx < values.len() {
                    let cursor = ef.cursor_from(idx);
                    assert_eq!(
                        cursor.current(),
                        Some(values[idx]),
                        "cursor_from({}) near boundary {}",
                        idx,
                        boundary
                    );
                }
            }
        }
    }

    #[test]
    fn test_large_values() {
        // Test with values near u32::MAX
        let values = vec![
            0,
            1_000_000,
            100_000_000,
            1_000_000_000,
            u32::MAX - 1000,
            u32::MAX,
        ];
        let ef = EliasFano::build(&values);

        for (i, &v) in values.iter().enumerate() {
            assert_eq!(ef.get(i), Some(v), "get({}) failed", i);
        }

        let collected: Vec<u32> = ef.into_iter().collect();
        assert_eq!(collected, values);
    }

    #[test]
    fn test_duplicate_values() {
        // Elias-Fano supports duplicates (monotonically non-decreasing)
        let values = vec![0, 0, 5, 5, 5, 10, 10, 20];
        let ef = EliasFano::build(&values);

        let collected: Vec<u32> = ef.into_iter().collect();
        assert_eq!(collected, values);
    }

    #[test]
    fn test_select_in_word() {
        assert_eq!(select_in_word(0b1010_1010, 0), 1);
        assert_eq!(select_in_word(0b1010_1010, 1), 3);
        assert_eq!(select_in_word(0b1010_1010, 2), 5);
        assert_eq!(select_in_word(0b1010_1010, 3), 7);

        assert_eq!(select_in_word(0b1, 0), 0);
        assert_eq!(select_in_word(0b1000_0000, 0), 7);

        assert_eq!(select_in_word(u64::MAX, 0), 0);
        assert_eq!(select_in_word(u64::MAX, 63), 63);
    }

    #[test]
    fn test_heap_size() {
        let values: Vec<u32> = (0..10000).map(|i| i * 10).collect();
        let ef = EliasFano::build(&values);

        let vec_size = values.len() * 4; // 40,000 bytes
        let ef_size = ef.heap_size();

        // EF should be significantly smaller
        assert!(
            ef_size < vec_size,
            "EF size {} should be less than Vec size {}",
            ef_size,
            vec_size
        );

        // Print compression ratio for inspection
        let ratio = vec_size as f64 / ef_size as f64;
        eprintln!(
            "Compression: {} bytes -> {} bytes ({:.2}x)",
            vec_size, ef_size, ratio
        );
    }

    #[test]
    fn test_iterator() {
        let values: Vec<u32> = (0..50).map(|i| i * i).collect();
        let ef = EliasFano::build(&values);

        // Test iterator
        let collected: Vec<u32> = ef.into_iter().collect();
        assert_eq!(collected, values);

        // Test size_hint
        let iter = ef.into_iter();
        assert_eq!(iter.size_hint(), (50, Some(50)));
    }

    #[test]
    fn test_many_elements_with_samples() {
        // Test with more elements than sample rate to exercise sampling
        let values: Vec<u32> = (0..1000).map(|i| i * 5).collect();
        let ef = EliasFano::build(&values);

        // Random access should work with samples
        assert_eq!(ef.get(0), Some(0));
        assert_eq!(ef.get(256), Some(1280)); // At sample boundary
        assert_eq!(ef.get(512), Some(2560)); // At sample boundary
        assert_eq!(ef.get(999), Some(4995));

        // Cursor seek should use samples
        let mut cursor = ef.cursor();
        assert_eq!(cursor.seek(300), Some(1500));
        assert_eq!(cursor.seek(600), Some(3000));
    }

    /// Benchmark comparing Vec<u32> vs EliasFano for different access patterns.
    /// Run with: cargo test --release benchmark_access_patterns -- --nocapture --ignored
    #[test]
    #[ignore]
    fn benchmark_access_patterns() {
        use std::time::Instant;

        // Simulate realistic bp_to_text: positions with small gaps (10-100 bytes typical)
        let n = 100_000;
        let mut values = Vec::with_capacity(n);
        let mut pos = 0u32;
        for i in 0..n {
            values.push(pos);
            // Varying gaps: small for scalars, larger for containers
            pos += if i % 10 == 0 {
                50 + (i as u32 % 100)
            } else {
                10 + (i as u32 % 20)
            };
        }

        let ef = EliasFano::build(&values);

        eprintln!("\n=== EliasFano Benchmark ===");
        eprintln!("Elements: {}", n);
        eprintln!("Universe: {}", values.last().unwrap());
        eprintln!(
            "Size: Vec<u32>={} bytes, EliasFano={} bytes ({:.2}x compression)",
            n * 4,
            ef.heap_size(),
            (n * 4) as f64 / ef.heap_size() as f64
        );

        let iterations = 10;

        // 1. Sequential iteration (cursor advance_one)
        let start = Instant::now();
        for _ in 0..iterations {
            let mut cursor = ef.cursor();
            let mut sum = 0u64;
            while let Some(v) = cursor.current() {
                sum += v as u64;
                cursor.advance_one();
            }
            std::hint::black_box(sum);
        }
        let ef_seq_time = start.elapsed();

        let start = Instant::now();
        for _ in 0..iterations {
            let mut sum = 0u64;
            for &v in &values {
                sum += v as u64;
            }
            std::hint::black_box(sum);
        }
        let vec_seq_time = start.elapsed();

        eprintln!(
            "\nSequential iteration ({} elements × {} iterations):",
            n, iterations
        );
        eprintln!(
            "  Vec<u32>:    {:?} ({:.1} ns/elem)",
            vec_seq_time,
            vec_seq_time.as_nanos() as f64 / (n * iterations) as f64
        );
        eprintln!(
            "  EliasFano:   {:?} ({:.1} ns/elem)",
            ef_seq_time,
            ef_seq_time.as_nanos() as f64 / (n * iterations) as f64
        );
        eprintln!(
            "  Ratio:       {:.2}x",
            ef_seq_time.as_nanos() as f64 / vec_seq_time.as_nanos() as f64
        );

        // 2. Random access
        let indices: Vec<usize> = (0..10000).map(|i| (i * 7) % n).collect();

        let start = Instant::now();
        for _ in 0..iterations {
            let mut sum = 0u64;
            for &i in &indices {
                sum += ef.get(i).unwrap() as u64;
            }
            std::hint::black_box(sum);
        }
        let ef_rand_time = start.elapsed();

        let start = Instant::now();
        for _ in 0..iterations {
            let mut sum = 0u64;
            for &i in &indices {
                sum += values[i] as u64;
            }
            std::hint::black_box(sum);
        }
        let vec_rand_time = start.elapsed();

        eprintln!(
            "\nRandom access ({} accesses × {} iterations):",
            indices.len(),
            iterations
        );
        eprintln!(
            "  Vec<u32>:    {:?} ({:.1} ns/access)",
            vec_rand_time,
            vec_rand_time.as_nanos() as f64 / (indices.len() * iterations) as f64
        );
        eprintln!(
            "  EliasFano:   {:?} ({:.1} ns/access)",
            ef_rand_time,
            ef_rand_time.as_nanos() as f64 / (indices.len() * iterations) as f64
        );
        eprintln!(
            "  Ratio:       {:.2}x",
            ef_rand_time.as_nanos() as f64 / vec_rand_time.as_nanos() as f64
        );

        // 3. Skip-by-small (advance_by with k=2..8, simulating next_sibling on nested structures)
        let start = Instant::now();
        for _ in 0..iterations {
            let mut cursor = ef.cursor();
            let mut sum = 0u64;
            let mut skip = 2;
            while let Some(v) = cursor.current() {
                sum += v as u64;
                cursor.advance_by(skip);
                skip = (skip % 7) + 2; // Vary skip 2-8
            }
            std::hint::black_box(sum);
        }
        let ef_skip_time = start.elapsed();

        let start = Instant::now();
        for _ in 0..iterations {
            let mut sum = 0u64;
            let mut i = 0;
            let mut skip = 2;
            while i < values.len() {
                sum += values[i] as u64;
                i += skip;
                skip = (skip % 7) + 2;
            }
            std::hint::black_box(sum);
        }
        let vec_skip_time = start.elapsed();

        eprintln!(
            "\nSkip-by-small (advance_by 2-8 × {} iterations):",
            iterations
        );
        eprintln!("  Vec<u32>:    {:?}", vec_skip_time);
        eprintln!("  EliasFano:   {:?}", ef_skip_time);
        eprintln!(
            "  Ratio:       {:.2}x",
            ef_skip_time.as_nanos() as f64 / vec_skip_time.as_nanos() as f64
        );
    }
}
