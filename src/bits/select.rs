//! Select index for accelerated select queries.
//!
//! This module implements a sampled select index that provides O(1) jump
//! to approximate position, followed by a short linear scan.
//!
//! # Sample width
//!
//! [`SelectIndex`] is generic over the width of its sample fields:
//!
//! - `SelectIndex<u64>` (the default) imposes no ceiling, and is what
//!   [`BitVec`](crate::bits::BitVec) uses. Bitvectors past 2^32 set bits wrapped
//!   the old `u32` fields (#188), so this width is required in the general case.
//! - `SelectIndex<u32>` halves the samples array. It is only sound for callers
//!   that bound their length to `u32::MAX` bits, which
//!   [`BalancedParens`](crate::trees::BalancedParens) asserts at construction —
//!   its rank directory stores absolute cumulative counts as `u32`.

#[cfg(not(test))]
use alloc::vec::Vec;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Default sample rate for select index.
pub const DEFAULT_SAMPLE_RATE: u32 = 256;

mod sealed {
    pub trait Sealed {}
    impl Sealed for u32 {}
    impl Sealed for u64 {}
}

/// Width of the integers stored in a [`SelectIndex`] sample entry.
///
/// Sealed: implemented only for `u32` and `u64`. Both conversions must be
/// lossless for the values the index actually stores, so this trait is not
/// open to types that would truncate.
pub trait SampleWord: sealed::Sealed + Copy + Default + core::fmt::Debug {
    /// Converts from `usize`, which must be representable in this width.
    ///
    /// Debug builds assert representability; release builds truncate, so
    /// callers are responsible for bounding their inputs.
    fn from_usize(value: usize) -> Self;

    /// Converts to `usize`, always losslessly.
    fn to_usize(self) -> usize;
}

impl SampleWord for u32 {
    #[inline]
    fn from_usize(value: usize) -> Self {
        debug_assert!(
            Self::try_from(value).is_ok(),
            "{value} exceeds u32 in a narrow SelectIndex; \
             only callers bounded to u32::MAX bits may use this width (#188)"
        );
        value as Self
    }

    #[inline]
    fn to_usize(self) -> usize {
        self as usize
    }
}

impl SampleWord for u64 {
    #[inline]
    fn from_usize(value: usize) -> Self {
        debug_assert!(Self::try_from(value).is_ok(), "{value} exceeds u64");
        value as Self
    }

    #[inline]
    fn to_usize(self) -> usize {
        self as usize
    }
}

/// Sample entry storing word index and cumulative count before that word.
///
/// The width is chosen by the caller via [`SampleWord`]; see the module docs
/// for when each is sound. The samples array holds one entry per `sample_rate`
/// set bits, so the width choice scales the whole index.
#[derive(Clone, Copy, Debug, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
struct SampleEntry<T> {
    /// Word index containing the sample point.
    word_idx: T,
    /// Cumulative count of ones before this word (not including this word).
    cumulative_before: T,
}

/// Sampled select index for accelerated select queries.
///
/// Stores the word index containing every k-th 1-bit, where k is the sample rate,
/// along with the cumulative count before that word. This allows O(1) jump to
/// the approximate location, followed by a short linear scan.
///
/// # Space Overhead
///
/// Overhead is **density-dependent**, not fixed: with `n` bits at density `d`,
/// the index holds `n·d / rate` entries, so its size relative to the bitvector
/// is `8·size_of::<SampleEntry<T>>()·d / rate`.
///
/// | Density | Where                | `<u64>` at rate 256 | `<u32>` at rate 256 |
/// |---------|----------------------|---------------------|---------------------|
/// | 0.5     | Balanced parentheses | 25.00%              | 12.50%              |
/// | 0.125   | Newline indexes      | 6.25%               | 3.125%              |
/// | 0.01    | Sparse bitmaps       | 0.50%               | 0.25%               |
///
/// Balanced parens are ~50% ones by construction, which is why the narrow
/// width matters most there.
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct SelectIndex<T = u64> {
    /// Sample entries: samples[i] contains info for (i * sample_rate)-th 1-bit.
    samples: Vec<SampleEntry<T>>,
    /// Sample rate (e.g., 256)
    sample_rate: u32,
}

impl<T: SampleWord> SelectIndex<T> {
    /// Creates an empty select index.
    pub fn empty() -> Self {
        Self {
            samples: Vec::new(),
            sample_rate: DEFAULT_SAMPLE_RATE,
        }
    }

    /// Builds a select index from word data.
    ///
    /// # Arguments
    ///
    /// * `words` - The raw bit data
    /// * `total_ones` - Total number of 1-bits (for capacity estimation)
    /// * `sample_rate` - How often to sample (e.g., 256 = sample every 256th one)
    ///
    /// # Panics
    ///
    /// In debug builds, panics if a word index or cumulative count exceeds `T`.
    /// For `T = u32` the caller must bound its length to `u32::MAX` bits.
    pub fn build(words: &[u64], total_ones: usize, sample_rate: u32) -> Self {
        if words.is_empty() || total_ones == 0 {
            return Self {
                samples: Vec::new(),
                sample_rate,
            };
        }

        let sample_rate = sample_rate.max(1);
        let num_samples = total_ones / sample_rate as usize + 1;
        let mut samples = Vec::with_capacity(num_samples);

        let mut count: usize = 0;
        let mut next_sample = 0usize;

        for (word_idx, &word) in words.iter().enumerate() {
            let pop = word.count_ones() as usize;

            // Check if any sample points fall within this word
            while next_sample < total_ones && count + pop > next_sample {
                samples.push(SampleEntry {
                    word_idx: T::from_usize(word_idx),
                    cumulative_before: T::from_usize(count),
                });
                next_sample += sample_rate as usize;
            }

            count += pop;
        }

        Self {
            samples,
            sample_rate,
        }
    }

    /// Jumps to the word position for finding the k-th 1-bit.
    ///
    /// Returns `(start_word_idx, remaining_count)` where:
    /// - `start_word_idx` is the word to start scanning from
    /// - `remaining_count` is the number of 1-bits to skip within/after that word
    ///
    /// The caller should scan from start_word_idx, counting ones until
    /// remaining_count is exhausted.
    #[inline]
    pub fn jump_to(&self, k: usize) -> (usize, usize) {
        if self.samples.is_empty() {
            return (0, k);
        }

        let sample_rate = self.sample_rate as usize;
        let sample_idx = k / sample_rate;

        if sample_idx >= self.samples.len() {
            // Beyond our samples, use the last one
            let last = &self.samples[self.samples.len() - 1];
            let start_word = last.word_idx.to_usize();
            let remaining = k - last.cumulative_before.to_usize();
            return (start_word, remaining);
        }

        let entry = &self.samples[sample_idx];
        let start_word = entry.word_idx.to_usize();
        let remaining = k - entry.cumulative_before.to_usize();

        (start_word, remaining)
    }

    /// Returns the sample rate.
    #[inline]
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Returns the heap memory usage in bytes.
    ///
    /// One `SampleEntry<T>` per [`sample_rate`](Self::sample_rate) set bits;
    /// see the module docs' space overhead table for how the byte count
    /// varies with `T`.
    #[inline]
    pub fn heap_size(&self) -> usize {
        self.samples.len() * core::mem::size_of::<SampleEntry<T>>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_index() {
        let idx = SelectIndex::<u64>::build(&[], 0, 256);
        assert_eq!(idx.jump_to(0), (0, 0));
        assert_eq!(idx.jump_to(100), (0, 100));
        assert_eq!(idx.heap_size(), 0);
    }

    #[test]
    fn test_single_word() {
        let words = vec![0b1111u64]; // 4 ones
        let idx = SelectIndex::<u64>::build(&words, 4, 2);

        // Sample rate 2: samples at positions 0, 2
        assert_eq!(idx.jump_to(0), (0, 0));
        assert_eq!(idx.jump_to(1), (0, 1));
        assert_eq!(idx.jump_to(2), (0, 2));
        assert_eq!(idx.jump_to(3), (0, 3));
    }

    #[test]
    fn test_multiple_words() {
        // 4 words with 4 ones each = 16 ones total
        let words = vec![0b1111u64; 4];
        let idx = SelectIndex::<u64>::build(&words, 16, 4);

        // Samples at positions 0, 4, 8, 12
        // samples[0] = (word 0, cumulative 0) for position 0
        // samples[1] = (word 1, cumulative 4) for position 4
        // samples[2] = (word 2, cumulative 8) for position 8
        // samples[3] = (word 3, cumulative 12) for position 12

        // jump_to(0) = use samples[0] = word 0, remaining 0-0=0
        assert_eq!(idx.jump_to(0), (0, 0));

        // jump_to(5) = sample_idx=1, use samples[1] = (word 1, cumulative 4)
        // remaining = 5 - 4 = 1
        let (word, rem) = idx.jump_to(5);
        assert_eq!(word, 1);
        assert_eq!(rem, 1);
    }

    #[test]
    fn test_sparse_data() {
        // One bit set every 64 bits (first bit of each word)
        let words: Vec<u64> = vec![1; 100];
        let idx = SelectIndex::<u64>::build(&words, 100, 10);

        // Samples at positions 0, 10, 20, ...
        // samples[0] = word 0
        // samples[1] = word 10
        // etc.

        let (word, _rem) = idx.jump_to(25);
        // sample_idx = 2, use samples[1] = word 10
        // remaining = 25 - 10 = 15
        assert!(word <= 25);
    }

    #[test]
    fn test_dense_data() {
        // All bits set
        let words: Vec<u64> = vec![u64::MAX; 10];
        let idx = SelectIndex::<u64>::build(&words, 640, 64);

        // Samples every 64 ones, which is every word
        let (word, _rem) = idx.jump_to(128);
        // sample_idx = 2, use samples[1] = word containing 64th bit = word 1
        assert!(word <= 2);
    }

    #[test]
    fn test_sample_rate_one() {
        let words = vec![0b1111u64];
        let idx = SelectIndex::<u64>::build(&words, 4, 1);

        // Every bit is sampled
        assert_eq!(idx.jump_to(0), (0, 0));
        assert_eq!(idx.jump_to(1), (0, 1));
        assert_eq!(idx.jump_to(2), (0, 2));
        assert_eq!(idx.jump_to(3), (0, 3));
    }

    #[test]
    fn test_large_sample_rate() {
        let words = vec![0b1111u64; 4];
        let idx = SelectIndex::<u64>::build(&words, 16, 256);

        // Sample rate larger than total ones - only sample at 0
        assert_eq!(idx.jump_to(0), (0, 0));
        assert_eq!(idx.jump_to(15), (0, 15));
    }

    #[test]
    fn test_jump_to_past_the_last_sample() {
        // 4 words x 4 ones at sample rate 4 gives samples for ones 0, 4, 8, 12.
        let words = vec![0b1111u64; 4];
        let idx = SelectIndex::<u64>::build(&words, 16, 4);
        assert_eq!(idx.samples.len(), 4);

        // k == total_ones and beyond both land past the last sample, so `jump_to`
        // falls back to it: scanning from there just runs off the end, which is
        // how `BitVec::select1` yields None. It must never point past the k-th one.
        assert_eq!(idx.jump_to(16), (3, 4));
        assert_eq!(idx.jump_to(100), (3, 88));
    }

    #[test]
    fn test_sample_rate_accessor_reports_the_rate_built_with() {
        let words = vec![0b1111u64; 4];
        assert_eq!(SelectIndex::<u64>::build(&words, 16, 4).sample_rate(), 4);
        // `build` clamps 0 to 1 to avoid a divide-by-zero in `jump_to`.
        assert_eq!(SelectIndex::<u64>::build(&words, 16, 0).sample_rate(), 1);
        assert_eq!(
            SelectIndex::<u64>::empty().sample_rate(),
            DEFAULT_SAMPLE_RATE
        );
    }

    /// Regression test for #188: `cumulative_before` was u32 and wrapped past
    /// 2^32 set bits. Needs ~800 MB RAM (512 MB of all-ones words plus the
    /// sample array), hence the huge-tests gate.
    #[test]
    #[cfg(feature = "huge-tests")]
    fn test_sample_entry_beyond_u32_max() {
        let num_words = (u32::MAX as usize / 64) + 2;
        let words = vec![u64::MAX; num_words];
        let total_ones = num_words * 64;
        assert!(total_ones > u32::MAX as usize);

        let idx = SelectIndex::<u64>::build(&words, total_ones, 256);

        // All bits are ones, so the k-th 1-bit lives at bit position k and
        // cumulative_before == start_word * 64 exactly.
        let k = u32::MAX as usize + 1;
        let (start_word, remaining) = idx.jump_to(k);
        assert_eq!(start_word * 64 + remaining, k);
        // The sample that answered this query sits beyond the old u32 range.
        assert!(start_word as u64 * 64 > u64::from(u32::MAX));
    }

    // ========================================================================
    // Narrow (u32) sample width — Step A of #64
    // ========================================================================

    /// The whole point of the narrow width: half the bytes for the same answers.
    #[test]
    fn test_narrow_index_is_half_the_size() {
        let words: Vec<u64> = vec![0xAAAA_AAAA_AAAA_AAAA; 512]; // d = 0.5
        let total_ones = words.iter().map(|w| w.count_ones() as usize).sum();

        let wide = SelectIndex::<u64>::build(&words, total_ones, 256);
        let narrow = SelectIndex::<u32>::build(&words, total_ones, 256);

        assert_eq!(wide.heap_size(), narrow.heap_size() * 2);
        assert!(narrow.heap_size() > 0, "test data must produce samples");

        // At density 0.5 and rate 256 the wide index costs 25% of the bitmap
        // and the narrow one 12.5% — the numbers Step A is built on.
        let bitmap_bytes = words.len() * 8;
        assert_eq!(wide.heap_size() * 4, bitmap_bytes);
        assert_eq!(narrow.heap_size() * 8, bitmap_bytes);
    }

    /// Narrowing must not change a single answer.
    #[test]
    fn test_narrow_matches_wide() {
        // Mixed densities so sample points land in varied places.
        for (label, words) in [
            ("dense", vec![u64::MAX; 200]),
            ("alternating", vec![0xAAAA_AAAA_AAAA_AAAA; 200]),
            ("sparse", vec![1u64; 200]),
            (
                "irregular",
                (0..200u64).map(|i| i.wrapping_mul(0x9E37_79B9)).collect(),
            ),
        ] {
            let total_ones: usize = words.iter().map(|w| w.count_ones() as usize).sum();

            for rate in [1u32, 2, 64, 256, 1024] {
                let wide = SelectIndex::<u64>::build(&words, total_ones, rate);
                let narrow = SelectIndex::<u32>::build(&words, total_ones, rate);

                assert_eq!(
                    wide.sample_rate(),
                    narrow.sample_rate(),
                    "{label}/rate {rate}: sample rate"
                );

                for k in 0..total_ones {
                    assert_eq!(
                        wide.jump_to(k),
                        narrow.jump_to(k),
                        "{label}/rate {rate}: jump_to({k})"
                    );
                }

                // Past the last sample, where jump_to takes its fallback branch.
                assert_eq!(
                    wide.jump_to(total_ones + 1000),
                    narrow.jump_to(total_ones + 1000),
                    "{label}/rate {rate}: jump_to beyond total_ones"
                );
            }
        }
    }

    #[test]
    fn test_narrow_empty_and_all_zero() {
        let empty = SelectIndex::<u32>::build(&[], 0, 256);
        assert_eq!(empty.jump_to(0), (0, 0));
        assert_eq!(empty.jump_to(100), (0, 100));

        // Words present but no ones: build takes the total_ones == 0 path.
        let zeros = SelectIndex::<u32>::build(&[0u64; 16], 0, 256);
        assert_eq!(zeros.heap_size(), 0);
        assert_eq!(zeros.jump_to(0), (0, 0));
    }

    #[test]
    fn test_sample_word_roundtrip() {
        assert_eq!(<u32 as SampleWord>::from_usize(0).to_usize(), 0);
        assert_eq!(
            <u32 as SampleWord>::from_usize(u32::MAX as usize).to_usize(),
            u32::MAX as usize
        );
        assert_eq!(<u64 as SampleWord>::from_usize(12345).to_usize(), 12345);
    }

    /// The narrow width is only sound below `u32::MAX`; debug builds must catch
    /// misuse rather than silently truncating as #188 did.
    #[test]
    #[should_panic(expected = "exceeds u32 in a narrow SelectIndex")]
    #[cfg(target_pointer_width = "64")]
    fn test_narrow_rejects_out_of_range_in_debug() {
        let _ = <u32 as SampleWord>::from_usize(u32::MAX as usize + 1);
    }
}
