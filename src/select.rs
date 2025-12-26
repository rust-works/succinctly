//! Select index for accelerated select queries.
//!
//! This module implements a sampled select index that provides O(1) jump
//! to approximate position, followed by a short linear scan.

#[cfg(not(test))]
use alloc::vec::Vec;

/// Default sample rate for select index.
pub const DEFAULT_SAMPLE_RATE: u32 = 256;

/// Sample entry storing word index and cumulative count before that word.
#[derive(Clone, Copy, Debug, Default)]
struct SampleEntry {
    /// Word index containing the sample point.
    word_idx: u32,
    /// Cumulative count of ones before this word (not including this word).
    cumulative_before: u32,
}

/// Sampled select index for accelerated select queries.
///
/// Stores the word index containing every k-th 1-bit, where k is the sample rate,
/// along with the cumulative count before that word. This allows O(1) jump to
/// the approximate location, followed by a short linear scan.
///
/// # Space Overhead
///
/// With sample rate k, overhead is approximately `8 / k` bytes per bit set.
/// - Sample rate 64: ~12.5% overhead
/// - Sample rate 256: ~3% overhead
/// - Sample rate 512: ~1.5% overhead
#[derive(Clone, Debug, Default)]
pub struct SelectIndex {
    /// Sample entries: samples[i] contains info for (i * sample_rate)-th 1-bit.
    samples: Vec<SampleEntry>,
    /// Sample rate (e.g., 256)
    sample_rate: u32,
}

impl SelectIndex {
    /// Create an empty select index.
    pub fn empty() -> Self {
        Self {
            samples: Vec::new(),
            sample_rate: DEFAULT_SAMPLE_RATE,
        }
    }

    /// Build a select index from word data.
    ///
    /// # Arguments
    ///
    /// * `words` - The raw bit data
    /// * `total_ones` - Total number of 1-bits (for capacity estimation)
    /// * `sample_rate` - How often to sample (e.g., 256 = sample every 256th one)
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
                    word_idx: word_idx as u32,
                    cumulative_before: count as u32,
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

    /// Jump to the word position for finding the k-th 1-bit.
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
            let start_word = last.word_idx as usize;
            let remaining = k - last.cumulative_before as usize;
            return (start_word, remaining);
        }

        let entry = &self.samples[sample_idx];
        let start_word = entry.word_idx as usize;
        let remaining = k - entry.cumulative_before as usize;

        (start_word, remaining)
    }

    /// Get the sample rate.
    #[inline]
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_index() {
        let idx = SelectIndex::build(&[], 0, 256);
        assert_eq!(idx.jump_to(0), (0, 0));
        assert_eq!(idx.jump_to(100), (0, 100));
    }

    #[test]
    fn test_single_word() {
        let words = vec![0b1111u64]; // 4 ones
        let idx = SelectIndex::build(&words, 4, 2);

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
        let idx = SelectIndex::build(&words, 16, 4);

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
        let idx = SelectIndex::build(&words, 100, 10);

        // Samples at positions 0, 10, 20, ...
        // samples[0] = word 0
        // samples[1] = word 10
        // etc.

        let (word, rem) = idx.jump_to(25);
        // sample_idx = 2, use samples[1] = word 10
        // remaining = 25 - 10 = 15
        assert!(word <= 25);
    }

    #[test]
    fn test_dense_data() {
        // All bits set
        let words: Vec<u64> = vec![u64::MAX; 10];
        let idx = SelectIndex::build(&words, 640, 64);

        // Samples every 64 ones, which is every word
        let (word, rem) = idx.jump_to(128);
        // sample_idx = 2, use samples[1] = word containing 64th bit = word 1
        assert!(word <= 2);
    }

    #[test]
    fn test_sample_rate_one() {
        let words = vec![0b1111u64];
        let idx = SelectIndex::build(&words, 4, 1);

        // Every bit is sampled
        assert_eq!(idx.jump_to(0), (0, 0));
        assert_eq!(idx.jump_to(1), (0, 1));
        assert_eq!(idx.jump_to(2), (0, 2));
        assert_eq!(idx.jump_to(3), (0, 3));
    }

    #[test]
    fn test_large_sample_rate() {
        let words = vec![0b1111u64; 4];
        let idx = SelectIndex::build(&words, 16, 256);

        // Sample rate larger than total ones - only sample at 0
        assert_eq!(idx.jump_to(0), (0, 0));
        assert_eq!(idx.jump_to(15), (0, 15));
    }
}
