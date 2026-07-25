//! Advance Index for memory-efficient BP-to-text-end position mapping.
//!
//! This module provides `EndPositions`, a space-efficient alternative to `Vec<u32>`
//! for storing scalar end positions in YAML. Containers store 0 as a sentinel
//! (they have no text end position), so only scalar entries carry meaningful data.
//!
//! # Structure
//!
//! For monotonically non-decreasing non-zero positions, uses two bitmaps:
//!
//! - **IB (Interest Bits)**: One bit per text byte, set at each unique end position
//! - **Advance bitmap**: One bit per BP open, set when end position advances
//!
//! Container zeros are filled with the previous non-zero value before encoding,
//! making the sequence monotonic. This eliminates the need for a separate `has_end`
//! bitmap, reducing the hot path from 3 bitmap accesses to 2.
//!
//! For non-monotonic positions, falls back to `Vec<u32>`.
//!
//! # Memory
//!
//! With typical YAML density (N opens, L text bytes):
//! - Dense `Vec<u32>`: 4N bytes
//! - Compact: L/8 (IB) + N/8 (advance) + rank/select overhead
//! - **~1.4-2.5x smaller** (when monotonic; like `open_positions`, the ratio
//!   falls as text-bytes-per-open rises, because the IB map costs one bit per
//!   text byte — see `advance_positions` module docs)
//!
//! # API Note
//!
//! The two variants disagree on entries the parser never recorded: Compact returns
//! `Some(previous non-zero end)` (an artefact of the zero-filling above), Dense returns
//! `None`. What both guarantee, and what callers may rely on, is:
//!
//! > `get(i)` returns either node `i`'s own end, or a non-decreasing end recorded for an
//! > *earlier* node. Since the parser writes a node's open position while positioned at
//! > it, and every end it records is at or before the current position, an inherited value
//! > is always at or before the node's own text position. So `end > text_pos` means "this
//! > node has an extent of its own".
//!
//! `value()` in `light.rs` depends on exactly that: it distinguishes an empty block
//! sequence-item wrapper (no extent, reads as null) from a plain scalar that merely begins
//! `- ` (#332). The parser upholds the invariant with a debug assertion in
//! `set_bp_text_end`.

#[cfg(not(test))]
use alloc::boxed::Box;
#[cfg(not(test))]
use alloc::vec;
#[cfg(not(test))]
use alloc::vec::Vec;

use core::cell::Cell;

use crate::util::broadword::select_in_word;

use super::advance_positions::{build_cumulative_rank, build_select_samples, SELECT_SAMPLE_RATE};

/// Cursor state for sequential access optimization.
///
/// When `get()` is called with consecutively increasing `open_idx` values
/// (as happens during depth-first streaming), this cursor enables O(1)
/// amortized access by maintaining incremental rank/select state instead
/// of recomputing from scratch each time.
///
/// Invariants (when `next_open_idx < usize::MAX`):
/// - `adv_cumulative` = number of 1-bits in `advance[0..next_open_idx)`
/// - `ib_ones_before` = number of 1-bits in `ib_words[0..ib_word_idx)`
#[derive(Clone, Copy, Debug)]
struct SequentialCursor {
    /// The next expected open_idx for sequential access.
    next_open_idx: usize,
    /// advance_rank1(next_open_idx): cumulative 1-bits in advance before this position.
    adv_cumulative: usize,
    /// Current word index in IB bitmap for forward scanning.
    ib_word_idx: usize,
    /// Number of 1-bits in ib_words[0..ib_word_idx).
    ib_ones_before: usize,
    /// Last argument to ib_select1 (for duplicate detection). usize::MAX = uninitialized.
    last_ib_arg: usize,
    /// Cached result from last ib_select1 call.
    last_ib_result: usize,
}

impl Default for SequentialCursor {
    fn default() -> Self {
        Self {
            next_open_idx: 0,
            adv_cumulative: 0,
            ib_word_idx: 0,
            ib_ones_before: 0,
            last_ib_arg: usize::MAX,
            last_ib_result: 0,
        }
    }
}

/// End position storage with automatic optimization.
///
/// Uses memory-efficient encoding when non-zero end positions are monotonically
/// non-decreasing, otherwise falls back to dense `Vec<u32>` storage.
#[derive(Clone, Debug)]
pub enum EndPositions {
    /// Memory-efficient encoding for monotonic end positions.
    Compact(Box<CompactEndPositions>),
    /// Fallback for non-monotonic positions or when compression isn't beneficial.
    Dense(Vec<u32>),
}

/// Compact encoding for scalar end positions using two bitmaps.
///
/// Container zeros are filled with the previous non-zero value, making the
/// entire sequence monotonically non-decreasing. The advance bitmap is indexed
/// by BP open index (one bit per open), not by scalar index.
///
/// This eliminates the `has_end` bitmap from the previous 3-bitmap design,
/// reducing the hot path from 3 bitmap accesses to 2 and shrinking the
/// sequential cursor from 56 to 48 bytes.
#[derive(Clone, Debug)]
pub struct CompactEndPositions {
    /// Interest bits: one bit per text byte, set at each unique end position.
    ib_words: Vec<u64>,
    /// Number of valid bits in IB (= text length).
    #[allow(dead_code)] // STYLE-0005: index metadata field retained
    ib_len: usize,
    /// Sampled select index for IB.
    ib_select_samples: Vec<u32>,
    /// Total number of unique end positions (1-bits in IB).
    ib_ones: usize,

    /// Advance bitmap: one bit per BP open, set when end position advances.
    /// Indexed by open_idx (includes containers), not by scalar index.
    advance_words: Vec<u64>,
    /// Total number of BP opens.
    num_opens: usize,
    /// Cumulative popcount for advance. O(1) rank via single array lookup.
    advance_rank: Vec<u32>,

    /// Cursor for sequential access optimization (interior mutability).
    /// Enables amortized O(1) access when open_idx values are accessed
    /// in monotonically increasing order (streaming/depth-first traversal).
    cursor: Cell<SequentialCursor>,
}

impl EndPositions {
    /// Build from a slice of end positions (one per BP open).
    ///
    /// Zero entries are treated as sentinels (containers with no end position).
    /// For compact encoding, zeros are filled with the previous non-zero value
    /// to create a monotonic sequence suitable for 2-bitmap encoding.
    ///
    /// Automatically chooses compact or dense storage based on monotonicity
    /// of the non-zero entries.
    pub fn build(positions: &[u32], text_len: usize) -> Self {
        if positions.is_empty() {
            return Self::Compact(Box::new(CompactEndPositions::empty(text_len)));
        }

        // Single-pass build with inline monotonicity check and zero-filling.
        // Returns None if non-zero positions are not monotonically non-decreasing.
        match CompactEndPositions::try_build(positions, text_len) {
            Some(compact) => Self::Compact(Box::new(compact)),
            None => Self::Dense(positions.to_vec()),
        }
    }

    /// Get the end position for the `open_idx`-th BP open.
    ///
    /// For the Dense variant, returns `None` if the entry is 0 (no end recorded).
    /// For the Compact variant, an unrecorded entry instead yields `Some(previous
    /// non-zero end)` because of zero-filling; `None` comes back only for leading
    /// entries (before any end was recorded) or out of bounds.
    ///
    /// Either way the result is node `open_idx`'s own end or an earlier node's, never a
    /// later one — so a caller holding the node's text position can test `end > text_pos`
    /// to ask "does this node have an extent of its own?". See the module-level API note;
    /// `value()` in `light.rs` relies on this for #332.
    #[inline]
    pub fn get(&self, open_idx: usize) -> Option<usize> {
        match self {
            Self::Compact(c) => c.get(open_idx),
            Self::Dense(v) => v
                .get(open_idx)
                .map(|&pos| pos as usize)
                .filter(|&pos| pos > 0),
        }
    }

    /// Returns the heap memory usage in bytes.
    pub fn heap_size(&self) -> usize {
        match self {
            Self::Compact(c) => c.heap_size(),
            Self::Dense(v) => v.len() * 4,
        }
    }

    /// Returns true if using compact storage.
    #[inline]
    pub fn is_compact(&self) -> bool {
        matches!(self, Self::Compact(_))
    }
}

impl CompactEndPositions {
    /// Create an empty instance.
    fn empty(text_len: usize) -> Self {
        Self {
            ib_words: Vec::new(),
            ib_len: text_len,
            ib_select_samples: Vec::new(),
            ib_ones: 0,
            advance_words: Vec::new(),
            num_opens: 0,
            advance_rank: vec![0],
            cursor: Cell::default(),
        }
    }

    /// Build compact end positions from raw positions with inline zero-filling
    /// and monotonicity checking in a single pass.
    ///
    /// `positions` contains one entry per BP open. Zero entries are containers
    /// (no end position); non-zero entries are scalar end positions.
    ///
    /// Returns `None` if non-zero positions are not monotonically non-decreasing
    /// (caller should fall back to Dense storage).
    ///
    /// Zero entries are filled with the previous non-zero value inline during
    /// bitmap construction, avoiding a temporary `Vec<u32>` allocation.
    fn try_build(positions: &[u32], text_len: usize) -> Option<Self> {
        let num_opens = positions.len();

        // Build IB: set bit at each unique end position.
        // End positions can be at text_len (one past last byte for scalars ending at EOF),
        // so allocate one extra bit beyond text_len.
        let ib_num_words = (text_len + 1).div_ceil(64);
        let mut ib_words = vec![0u64; ib_num_words];

        // Build advance bitmap: one bit per BP open, set when position changes
        let advance_num_words = num_opens.div_ceil(64);
        let mut advance_words = vec![0u64; advance_num_words];

        let mut prev_effective: Option<u32> = None;
        let mut prev_nonzero = 0u32;
        let mut ib_ones = 0usize;

        for (i, &pos) in positions.iter().enumerate() {
            // Inline zero-fill: containers (pos == 0) inherit previous non-zero value.
            // Leading containers (before any scalar) have prev_nonzero == 0.
            let effective = if pos > 0 {
                // Monotonicity check: non-zero positions must be non-decreasing.
                if prev_nonzero > 0 && pos < prev_nonzero {
                    return None;
                }
                prev_nonzero = pos;
                pos
            } else {
                prev_nonzero
            };

            if effective == 0 {
                // Leading container (before any scalar): no advance, no IB bit.
                prev_effective = Some(0);
                continue;
            }

            let is_new_position = prev_effective != Some(effective);

            if is_new_position {
                // Set IB bit at this text position
                let word_idx = effective as usize / 64;
                let bit_idx = effective as usize % 64;
                if word_idx < ib_words.len() && (ib_words[word_idx] >> bit_idx) & 1 == 0 {
                    ib_words[word_idx] |= 1u64 << bit_idx;
                    ib_ones += 1;
                }

                // Set advance bit
                advance_words[i / 64] |= 1u64 << (i % 64);
            }

            prev_effective = Some(effective);
        }

        if ib_ones == 0 {
            // All positions were zero (containers only, no scalars).
            return Some(Self::empty(text_len));
        }

        let advance_rank = build_cumulative_rank(&advance_words);
        let ib_select_samples = build_select_samples(&ib_words, ib_ones);

        Some(Self {
            ib_words,
            ib_len: text_len,
            ib_select_samples,
            ib_ones,
            advance_words,
            num_opens,
            advance_rank,
            cursor: Cell::default(),
        })
    }

    /// Get the end position for the `open_idx`-th BP open.
    ///
    /// Returns the position for any valid open_idx. For containers that were
    /// zero-filled, returns the previous scalar's end position. Returns `None`
    /// only for leading containers (advance_count == 0) or out of bounds.
    ///
    /// Uses a sequential cursor optimization: when called with monotonically
    /// increasing `open_idx` values (streaming/depth-first traversal), this
    /// achieves amortized O(1) by using incremental bit tests instead of
    /// full rank/select recomputation.
    #[inline(always)]
    pub fn get(&self, open_idx: usize) -> Option<usize> {
        let cursor = self.cursor.get();
        if open_idx == cursor.next_open_idx {
            // Fast path: direct sequential access (most common in streaming)
            self.get_sequential(open_idx, cursor)
        } else if open_idx > cursor.next_open_idx {
            // Small gap (skipped containers): advance cursor through gap, then sequential
            let mut cursor = cursor;
            self.advance_cursor_to(&mut cursor, open_idx);
            self.get_sequential(open_idx, cursor)
        } else {
            // Backwards jump: full recomputation (rare, only in non-streaming access)
            self.get_random(open_idx)
        }
    }

    /// Advance the cursor from its current position to `target` open_idx.
    ///
    /// Uses O(1) advance_rank1 lookup instead of linear bit scanning.
    /// IB cursor state (ib_word_idx, ib_ones_before) remains valid for
    /// forward scanning since target > cursor.next_open_idx.
    #[inline]
    fn advance_cursor_to(&self, cursor: &mut SequentialCursor, target: usize) {
        cursor.adv_cumulative = self.advance_rank1(target);
        cursor.next_open_idx = target;
    }

    /// Fast path for sequential access (open_idx == cursor.next_open_idx).
    ///
    /// Uses incremental bit test for advance rank (O(1) per call) and forward
    /// scanning for IB select (amortized O(1) per call). Total: ~1-2
    /// memory accesses per call vs ~70 for the random access path.
    #[inline]
    fn get_sequential(&self, open_idx: usize, mut cursor: SequentialCursor) -> Option<usize> {
        if open_idx >= self.num_opens {
            return None;
        }

        // 1. Check advance bit (single word access)
        let word_idx = open_idx / 64;
        let bit_idx = open_idx % 64;
        let adv_bit = if word_idx < self.advance_words.len() {
            (self.advance_words[word_idx] >> bit_idx) & 1
        } else {
            0
        };

        // 2. advance_count = adv_cumulative + advance[open_idx]
        let advance_count = cursor.adv_cumulative + adv_bit as usize;

        // 3. Update cursor for next call
        cursor.adv_cumulative = advance_count;
        cursor.next_open_idx = open_idx + 1;

        if advance_count == 0 {
            // Before any real position (leading containers with filled value 0)
            self.cursor.set(cursor);
            return None;
        }

        // 4. ib_select1(advance_count - 1) — forward scan from cursor position
        let k = advance_count - 1;

        // Fast path: duplicate end position (same IB select argument as last time)
        if k == cursor.last_ib_arg {
            self.cursor.set(cursor);
            return Some(cursor.last_ib_result);
        }

        // Forward scan in IB bitmap from cursor position
        let mut remaining = k - cursor.ib_ones_before;
        let mut wi = cursor.ib_word_idx;

        while wi < self.ib_words.len() {
            let word = self.ib_words[wi];
            let ones = word.count_ones() as usize;
            if remaining < ones {
                let bit_pos = select_in_word(word, remaining as u32) as usize;
                let result = wi * 64 + bit_pos;

                // Update IB cursor state (stay at this word for potential next select)
                cursor.ib_ones_before = k - remaining; // ones in [0..wi)
                cursor.ib_word_idx = wi;
                cursor.last_ib_arg = k;
                cursor.last_ib_result = result;
                self.cursor.set(cursor);
                return Some(result);
            }
            remaining -= ones;
            wi += 1;
        }

        self.cursor.set(cursor);
        None
    }

    /// Random access path (backwards jump): full rank/select computation,
    /// then sets up cursor for subsequent sequential access.
    fn get_random(&self, open_idx: usize) -> Option<usize> {
        if open_idx >= self.num_opens {
            return None;
        }

        let advance_count = self.advance_rank1(open_idx + 1);

        let (result, ib_word_idx, ib_ones_before, last_ib_arg) = if advance_count == 0 {
            (None, 0, 0, usize::MAX)
        } else {
            let k = advance_count - 1;
            match self.ib_select1_with_state(k) {
                Some((pos, word_idx, ones_before)) => (Some(pos), word_idx, ones_before, k),
                None => (None, 0, 0, usize::MAX),
            }
        };

        // Set up cursor for subsequent sequential access starting at open_idx + 1,
        // resuming IB scanning from exactly where this lookup left off (the
        // word/ones-before pair `ib_select1_with_state` already computed)
        // rather than resetting to word 0. A reset here forces the *next*
        // `get_sequential` call to rescan the IB bitmap from the very start of
        // the document to catch back up — O(current document position) instead
        // of the O(1) amortized cost `get_sequential` is documented to have.
        // That cost is paid once per backward jump, so a caller that
        // repeatedly decodes-then-rewinds (e.g. a per-mapping merge-key check
        // followed by the mapping's real field walk) turns an O(n) document
        // walk into O(n^2) — see `resolve_merge_keys` in `src/yaml/light.rs`.
        self.cursor.set(SequentialCursor {
            next_open_idx: open_idx + 1,
            adv_cumulative: advance_count,
            ib_word_idx,
            ib_ones_before,
            last_ib_arg,
            last_ib_result: result.unwrap_or(0),
        });

        result
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

        if word_idx < self.advance_words.len() && bit_idx > 0 {
            let mask = (1u64 << bit_idx) - 1;
            count += (self.advance_words[word_idx] & mask).count_ones() as usize;
        }

        count
    }

    /// Select the k-th 1-bit in IB (0-indexed), also returning the resulting
    /// word index and the ones-count before that word (word-boundary-aligned,
    /// matching the `SequentialCursor` invariant) — the state a caller needs
    /// to resume forward scanning from here instead of from word 0. See
    /// `get_random`'s doc comment for why that matters.
    #[inline]
    fn ib_select1_with_state(&self, k: usize) -> Option<(usize, usize, usize)> {
        if k >= self.ib_ones {
            return None;
        }

        // Use select samples to find starting word
        let sample_idx = k / SELECT_SAMPLE_RATE;
        let has_sample = sample_idx < self.ib_select_samples.len();

        let (start_word, mut remaining, mut ones_before) = if has_sample {
            let sample_pos = self.ib_select_samples[sample_idx] as usize;
            let skip_ones = sample_idx * SELECT_SAMPLE_RATE;
            let word_idx = sample_pos / 64;
            let bit_offset = sample_pos % 64;
            // `skip_ones` counts ones up to the sample *bit*, not the word
            // boundary — recover the word-boundary-aligned count by
            // subtracting the ones already seen earlier in this same word.
            let prefix_ones =
                (self.ib_words[word_idx] & ((1u64 << bit_offset) - 1)).count_ones() as usize;
            (word_idx, k - skip_ones, skip_ones - prefix_ones)
        } else {
            (0, k, 0)
        };

        // Scan from sample position
        for word_idx in start_word..self.ib_words.len() {
            let full_word = self.ib_words[word_idx];
            let word = if word_idx == start_word && has_sample {
                let sample_pos = self.ib_select_samples[sample_idx] as usize;
                let bit_offset = sample_pos % 64;
                full_word & !((1u64 << bit_offset) - 1)
            } else {
                full_word
            };

            let ones = word.count_ones() as usize;
            if ones > remaining {
                let bit_pos = select_in_word(word, remaining as u32) as usize;
                return Some((word_idx * 64 + bit_pos, word_idx, ones_before));
            }
            remaining -= ones;
            ones_before += full_word.count_ones() as usize;
        }

        None
    }

    /// Returns the heap memory usage in bytes.
    pub fn heap_size(&self) -> usize {
        self.ib_words.len() * 8
            + self.ib_select_samples.len() * 4
            + self.advance_words.len() * 8
            + self.advance_rank.len() * 4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty() {
        let ep = EndPositions::build(&[], 0);
        assert!(ep.is_compact());
        assert_eq!(ep.get(0), None);
    }

    #[test]
    fn test_all_zeros() {
        // All containers, no scalars — all return None (no advances)
        let positions = vec![0, 0, 0, 0, 0];
        let ep = EndPositions::build(&positions, 100);
        assert!(ep.is_compact());
        for i in 0..5 {
            assert_eq!(ep.get(i), None, "get({i}) should be None");
        }
    }

    #[test]
    fn test_simple_scalars() {
        // BP opens: container(0), scalar(10), scalar(20), container(0), scalar(30)
        // After filling: [0, 10, 20, 20, 30]
        let positions = vec![0, 10, 20, 0, 30];
        let ep = EndPositions::build(&positions, 100);
        assert!(ep.is_compact());

        assert_eq!(ep.get(0), None); // leading container (advance_count == 0)
        assert_eq!(ep.get(1), Some(10)); // scalar
        assert_eq!(ep.get(2), Some(20)); // scalar
                                         // Container between scalars: returns previous scalar's end (20)
        assert_eq!(ep.get(3), Some(20));
        assert_eq!(ep.get(4), Some(30)); // scalar
        assert_eq!(ep.get(5), None); // out of bounds
    }

    #[test]
    fn test_duplicate_end_positions() {
        // Multiple scalars ending at the same position (unlikely but possible)
        let positions = vec![0, 10, 10, 20, 20, 20];
        // After filling: [0, 10, 10, 20, 20, 20]
        let ep = EndPositions::build(&positions, 100);
        assert!(ep.is_compact());

        assert_eq!(ep.get(0), None); // leading container
        assert_eq!(ep.get(1), Some(10));
        assert_eq!(ep.get(2), Some(10));
        assert_eq!(ep.get(3), Some(20));
        assert_eq!(ep.get(4), Some(20));
        assert_eq!(ep.get(5), Some(20));
    }

    #[test]
    fn test_non_monotonic_falls_back_to_dense() {
        // Non-monotonic non-zero positions: 20, 10
        let positions = vec![0, 20, 10, 0, 30];
        let ep = EndPositions::build(&positions, 100);
        assert!(!ep.is_compact()); // Should use Dense fallback

        assert_eq!(ep.get(0), None);
        assert_eq!(ep.get(1), Some(20));
        assert_eq!(ep.get(2), Some(10));
        assert_eq!(ep.get(3), None);
        assert_eq!(ep.get(4), Some(30));
    }

    #[test]
    fn test_all_scalars() {
        // No containers, all scalars
        let positions = vec![5, 10, 15, 20, 25];
        let ep = EndPositions::build(&positions, 100);
        assert!(ep.is_compact());

        for (i, &expected) in positions.iter().enumerate() {
            assert_eq!(ep.get(i), Some(expected as usize), "get({i}) failed");
        }
    }

    #[test]
    fn test_single_scalar() {
        let positions = vec![0, 42, 0];
        // After filling: [0, 42, 42]
        let ep = EndPositions::build(&positions, 100);
        assert!(ep.is_compact());

        assert_eq!(ep.get(0), None); // leading container
        assert_eq!(ep.get(1), Some(42)); // scalar
                                         // Container after scalar: returns previous value (42)
        assert_eq!(ep.get(2), Some(42));
    }

    #[test]
    fn test_memory_savings() {
        // Simulate realistic YAML: ~40% containers (zeros), ~60% scalars
        let mut positions = Vec::with_capacity(1000);
        let mut pos = 5u32;
        for i in 0..1000 {
            if i % 5 < 2 {
                // Container
                positions.push(0);
            } else {
                // Scalar
                positions.push(pos);
                pos += 7 + (i as u32 % 15);
            }
        }

        let text_len = pos as usize + 100;
        let ep = EndPositions::build(&positions, text_len);
        assert!(ep.is_compact());

        let vec_size = positions.len() * 4;
        let ep_size = ep.heap_size();

        eprintln!(
            "Memory: Vec<u32>={} bytes, EndPositions={} bytes ({:.2}x compression)",
            vec_size,
            ep_size,
            vec_size as f64 / ep_size as f64
        );

        // Should achieve meaningful compression
        assert!(
            ep_size < vec_size,
            "EndPositions {ep_size} should be smaller than Vec<u32> {vec_size}"
        );

        // Verify scalar values are correct
        for (i, &expected) in positions.iter().enumerate() {
            let got = ep.get(i);
            if expected > 0 {
                assert_eq!(
                    got,
                    Some(expected as usize),
                    "get({i}) failed: expected {expected}, got {got:?}"
                );
            }
            // Containers may return Some(prev_value) or None — don't check
        }
    }

    #[test]
    fn test_realistic_yaml_pattern() {
        // Simulate: mapping(0), key_scalar(5), value_scalar(11), key_scalar(15), value_scalar(22)
        // After filling: [0, 5, 11, 15, 22]
        let positions = vec![0, 5, 11, 15, 22];
        let ep = EndPositions::build(&positions, 30);
        assert!(ep.is_compact());

        assert_eq!(ep.get(0), None); // leading container
        assert_eq!(ep.get(1), Some(5));
        assert_eq!(ep.get(2), Some(11));
        assert_eq!(ep.get(3), Some(15));
        assert_eq!(ep.get(4), Some(22));
    }

    #[test]
    fn test_large_positions() {
        // Test with positions near u32 range
        // After filling: [0, 100000, 200000, 200000, 300000]
        let positions = vec![0, 100_000, 200_000, 0, 300_000];
        let ep = EndPositions::build(&positions, 400_000);
        assert!(ep.is_compact());

        assert_eq!(ep.get(0), None); // leading container
        assert_eq!(ep.get(1), Some(100_000));
        assert_eq!(ep.get(2), Some(200_000));
        // Container between scalars: returns previous value (200000)
        assert_eq!(ep.get(3), Some(200_000));
        assert_eq!(ep.get(4), Some(300_000));
    }

    #[test]
    fn test_many_elements_with_samples() {
        // Test with enough elements to exercise select samples (> 256)
        let mut positions = Vec::with_capacity(1000);
        let mut pos = 1u32;
        for i in 0..1000 {
            if i % 3 == 0 {
                positions.push(0); // container
            } else {
                positions.push(pos);
                pos += 5;
            }
        }

        let text_len = pos as usize + 100;
        let ep = EndPositions::build(&positions, text_len);
        assert!(ep.is_compact());

        // Verify scalar values are correct
        for (i, &expected) in positions.iter().enumerate() {
            let got = ep.get(i);
            if expected > 0 {
                assert_eq!(got, Some(expected as usize), "get({i}) failed");
            }
            // Containers may return Some(prev_value) or None
        }
    }

    #[test]
    fn test_real_yaml_compression() {
        // Generate a realistic YAML structure and measure EndPositions savings
        let mut yaml = String::new();
        for i in 0..500 {
            yaml.push_str(&format!("key_{i}: value_{i}\n"));
        }
        yaml.push_str("nested:\n");
        for i in 0..200 {
            yaml.push_str(&format!("  sub_{i}: sub_val_{i}\n"));
        }
        yaml.push_str("list:\n");
        for i in 0..300 {
            yaml.push_str(&format!("  - item_{i}\n"));
        }

        let semi = super::super::parser::build_semi_index(yaml.as_bytes()).unwrap();
        let text_len = yaml.len();
        let dense_size = semi.bp_to_text_end.len() * 4;
        let ep = EndPositions::build(&semi.bp_to_text_end, text_len);
        let compact_size = ep.heap_size();

        let num_opens = semi.bp_to_text_end.len();
        let num_nonzero = semi.bp_to_text_end.iter().filter(|&&v| v > 0).count();

        eprintln!(
            "Real YAML ({} bytes, {} opens, {} scalars): Vec<u32>={} bytes, EndPositions={} bytes ({:.2}x compression)",
            text_len, num_opens, num_nonzero, dense_size, compact_size,
            dense_size as f64 / compact_size as f64
        );

        assert!(ep.is_compact(), "Real YAML should use compact encoding");
        assert!(compact_size < dense_size, "Compact should be smaller");

        // Verify correctness: scalar values must match
        for (i, &expected) in semi.bp_to_text_end.iter().enumerate() {
            let got = ep.get(i);
            if expected > 0 {
                assert_eq!(got, Some(expected as usize), "open {i} mismatch");
            }
            // Containers (expected == 0) may return Some(prev_value) or None
        }
    }

    #[test]
    fn test_end_position_at_text_len() {
        // Edge case: end position equals text_len (scalar ending at EOF).
        // This can happen when the last scalar has no trailing newline.
        // After filling: [0, 64]
        let positions = vec![0, 64];
        let ep = EndPositions::build(&positions, 64);
        assert!(ep.is_compact());

        assert_eq!(ep.get(0), None); // leading container
        assert_eq!(ep.get(1), Some(64));
    }

    #[test]
    fn test_end_position_at_text_len_multiple_of_64() {
        // Regression test: text_len is a multiple of 64, and the last scalar
        // ends exactly at text_len. The IB bitmap must accommodate this extra bit.
        // After filling: [0, 50, 100, 100, 192]
        let positions = vec![0, 50, 100, 0, 192];
        let ep = EndPositions::build(&positions, 192);
        assert!(ep.is_compact());

        assert_eq!(ep.get(0), None); // leading container
        assert_eq!(ep.get(1), Some(50));
        assert_eq!(ep.get(2), Some(100));
        // Container between scalars: returns previous value (100)
        assert_eq!(ep.get(3), Some(100));
        assert_eq!(ep.get(4), Some(192));
    }

    #[test]
    fn test_containers_between_scalars_return_previous() {
        // Explicit test for the 2-bitmap behavior: containers between scalars
        // return the previous scalar's end position.
        let positions = vec![0, 10, 0, 0, 20, 0, 30];
        // After filling: [0, 10, 10, 10, 20, 20, 30]
        let ep = EndPositions::build(&positions, 100);
        assert!(ep.is_compact());

        assert_eq!(ep.get(0), None); // leading container
        assert_eq!(ep.get(1), Some(10)); // scalar
        assert_eq!(ep.get(2), Some(10)); // container → prev scalar (10)
        assert_eq!(ep.get(3), Some(10)); // container → prev scalar (10)
        assert_eq!(ep.get(4), Some(20)); // scalar
        assert_eq!(ep.get(5), Some(20)); // container → prev scalar (20)
        assert_eq!(ep.get(6), Some(30)); // scalar
    }

    #[test]
    fn test_random_access_after_sequential() {
        // Test backwards jump resets cursor correctly
        let positions = vec![5, 10, 15, 20, 25];
        let ep = EndPositions::build(&positions, 100);

        // Sequential access
        assert_eq!(ep.get(0), Some(5));
        assert_eq!(ep.get(1), Some(10));
        assert_eq!(ep.get(2), Some(15));

        // Backwards jump
        assert_eq!(ep.get(0), Some(5));
        assert_eq!(ep.get(1), Some(10));

        // Forward again
        assert_eq!(ep.get(3), Some(20));
        assert_eq!(ep.get(4), Some(25));
    }

    /// Monotonically non-decreasing, non-zero positions with irregular
    /// (non-arithmetic) gaps. A plain `i * constant` sequence can
    /// accidentally make every `SELECT_SAMPLE_RATE`-th select sample land
    /// exactly on a 64-bit word boundary (e.g. any multiple of 256, since
    /// 256 is itself a multiple of 64) — which would silently stop a test
    /// from ever exercising `get_random`'s within-word case, the one an
    /// earlier (broken) version of the #171-review fix got wrong. Irregular
    /// gaps avoid that trap.
    fn irregular_positions(n: usize, seed: u64) -> Vec<u32> {
        use rand::{Rng, SeedableRng};
        use rand_chacha::ChaCha8Rng;

        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let mut pos = 0u32;
        (0..n)
            .map(|_| {
                pos += rng.random_range(1..=6);
                pos
            })
            .collect()
    }

    /// A backward jump (`get_random`) must leave the cursor correctly seeded
    /// for the *next* sequential call, not just for the jump itself.
    /// `test_random_access_after_sequential` above (5 entries) is far too
    /// small to exercise the select-sample path this needs.
    #[test]
    fn test_get_random_then_sequential_resumes_correctly() {
        let positions = irregular_positions(2000, 1);
        let ep = EndPositions::build(&positions, 20_000);
        assert!(ep.is_compact());

        for (i, &pos) in positions.iter().enumerate() {
            assert_eq!(ep.get(i), Some(pos as usize), "forward pass 1, i={i}");
        }
        for (i, &pos) in positions.iter().enumerate() {
            assert_eq!(ep.get(i), Some(pos as usize), "forward pass 2, i={i}");
        }

        for &jump_to in &[0, 1, 63, 64, 65, 255, 256, 257, 511, 1000, 1999] {
            for (i, &pos) in positions.iter().enumerate().skip(jump_to) {
                assert_eq!(
                    ep.get(i),
                    Some(pos as usize),
                    "after jump to {jump_to}, i={i}"
                );
            }
        }
    }

    #[test]
    fn test_get_random_access_pattern_matches_reference() {
        use rand::{Rng, SeedableRng};
        use rand_chacha::ChaCha8Rng;

        let positions = irregular_positions(3000, 2);
        let ep = EndPositions::build(&positions, 20_000);
        assert!(ep.is_compact());

        let mut rng = ChaCha8Rng::seed_from_u64(171);
        for _ in 0..20_000 {
            let i = rng.random_range(0..positions.len());
            assert_eq!(
                ep.get(i),
                Some(positions[i] as usize),
                "random access i={i}"
            );
        }
    }
}
