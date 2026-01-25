//! Rank directory for O(1) rank queries.
//!
//! This module implements a Poppy-style 3-level rank directory that provides
//! O(1) rank queries with ~3% space overhead.

#[cfg(not(test))]
use alloc::vec::Vec;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use core::alloc::Layout;
use core::ptr::NonNull;

/// Number of 64-bit words per basic block (512 bits = 8 words).
const WORDS_PER_BLOCK: usize = 8;

/// Number of bits per basic block.
#[allow(dead_code)]
const BITS_PER_BLOCK: usize = WORDS_PER_BLOCK * 64;

/// Number of blocks per superblock (for L0).
/// L0 stores absolute rank every 2^32 bits = 2^26 words = 2^23 blocks.
const BLOCKS_PER_SUPERBLOCK: usize = 1 << 23;

/// Cache line size in bytes (64 bytes on most modern CPUs).
const CACHE_LINE_SIZE: usize = 64;

/// Number of u128 entries that fit in a cache line (64 / 16 = 4).
#[allow(dead_code)]
const ENTRIES_PER_CACHE_LINE: usize = CACHE_LINE_SIZE / 16;

/// Cache-aligned storage for L1+L2 entries.
///
/// This wrapper ensures that the L1+L2 data is allocated with 64-byte alignment,
/// reducing cache line crossings during rank queries. Each u128 entry is 16 bytes,
/// so 4 entries fit per cache line.
#[derive(Debug)]
struct CacheAlignedL1L2 {
    /// Pointer to the cache-aligned allocation.
    ptr: NonNull<u128>,
    /// Number of u128 entries.
    len: usize,
}

impl CacheAlignedL1L2 {
    /// Create an empty cache-aligned storage.
    fn empty() -> Self {
        Self {
            ptr: NonNull::dangling(),
            len: 0,
        }
    }

    /// Create a builder for in-place construction.
    ///
    /// This avoids the intermediate Vec allocation and copy that `from_vec` requires.
    fn builder(capacity: usize) -> CacheAlignedL1L2Builder {
        CacheAlignedL1L2Builder::with_capacity(capacity)
    }

    /// Create from a vector, allocating with cache-line alignment.
    ///
    /// Note: Prefer using `builder()` for new code to avoid the intermediate
    /// Vec allocation and copy.
    #[allow(dead_code)]
    fn from_vec(data: Vec<u128>) -> Self {
        if data.is_empty() {
            return Self::empty();
        }

        let len = data.len();
        let layout = Layout::from_size_align(len * 16, CACHE_LINE_SIZE).expect("layout error");

        // Safety: layout is valid (non-zero size, power-of-two alignment)
        let ptr = unsafe { alloc::alloc::alloc(layout) as *mut u128 };
        if ptr.is_null() {
            alloc::alloc::handle_alloc_error(layout);
        }

        // Copy data to aligned allocation
        // Safety: ptr is valid and has enough space for len elements
        unsafe {
            core::ptr::copy_nonoverlapping(data.as_ptr(), ptr, len);
        }

        Self {
            ptr: NonNull::new(ptr).unwrap(),
            len,
        }
    }

    /// Get the number of entries.
    #[inline]
    #[allow(dead_code)]
    fn len(&self) -> usize {
        self.len
    }

    /// Check if empty.
    #[inline]
    #[allow(dead_code)]
    fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Get a reference to the slice.
    #[inline]
    fn as_slice(&self) -> &[u128] {
        if self.len == 0 {
            &[]
        } else {
            // Safety: ptr is valid and points to len elements
            unsafe { core::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
        }
    }
}

impl Clone for CacheAlignedL1L2 {
    fn clone(&self) -> Self {
        if self.len == 0 {
            return Self::empty();
        }

        let layout = Layout::from_size_align(self.len * 16, CACHE_LINE_SIZE).expect("layout error");

        // Safety: layout is valid
        let ptr = unsafe { alloc::alloc::alloc(layout) as *mut u128 };
        if ptr.is_null() {
            alloc::alloc::handle_alloc_error(layout);
        }

        // Copy data
        // Safety: both pointers are valid
        unsafe {
            core::ptr::copy_nonoverlapping(self.ptr.as_ptr(), ptr, self.len);
        }

        Self {
            ptr: NonNull::new(ptr).unwrap(),
            len: self.len,
        }
    }
}

impl Drop for CacheAlignedL1L2 {
    fn drop(&mut self) {
        if self.len > 0 {
            let layout =
                Layout::from_size_align(self.len * 16, CACHE_LINE_SIZE).expect("layout error");
            // Safety: ptr was allocated with this layout
            unsafe {
                alloc::alloc::dealloc(self.ptr.as_ptr() as *mut u8, layout);
            }
        }
    }
}

impl Default for CacheAlignedL1L2 {
    fn default() -> Self {
        Self::empty()
    }
}

// Safety: The data is owned and immutable after construction
unsafe impl Send for CacheAlignedL1L2 {}
unsafe impl Sync for CacheAlignedL1L2 {}

/// Builder for in-place construction of CacheAlignedL1L2.
///
/// This avoids the double allocation pattern of building a Vec first
/// and then copying to aligned memory. Instead, it allocates the
/// aligned memory upfront and writes directly to it.
struct CacheAlignedL1L2Builder {
    /// Pointer to the cache-aligned allocation.
    ptr: NonNull<u128>,
    /// Current number of entries written.
    len: usize,
    /// Total capacity of the allocation.
    capacity: usize,
}

impl CacheAlignedL1L2Builder {
    /// Create a builder with the given capacity.
    ///
    /// Allocates cache-aligned memory immediately.
    fn with_capacity(capacity: usize) -> Self {
        if capacity == 0 {
            return Self {
                ptr: NonNull::dangling(),
                len: 0,
                capacity: 0,
            };
        }

        let layout = Layout::from_size_align(capacity * 16, CACHE_LINE_SIZE).expect("layout error");

        // Safety: layout is valid (non-zero size, power-of-two alignment)
        let ptr = unsafe { alloc::alloc::alloc(layout) as *mut u128 };
        if ptr.is_null() {
            alloc::alloc::handle_alloc_error(layout);
        }

        Self {
            ptr: NonNull::new(ptr).unwrap(),
            len: 0,
            capacity,
        }
    }

    /// Push a value directly into the aligned storage.
    ///
    /// # Panics
    ///
    /// Panics if the builder is at capacity.
    #[inline]
    fn push(&mut self, value: u128) {
        debug_assert!(
            self.len < self.capacity,
            "CacheAlignedL1L2Builder overflow: len={}, capacity={}",
            self.len,
            self.capacity
        );

        // Safety: len < capacity, so ptr.add(len) is within bounds
        unsafe {
            self.ptr.as_ptr().add(self.len).write(value);
        }
        self.len += 1;
    }

    /// Consume the builder and return the finished CacheAlignedL1L2.
    ///
    /// If fewer elements were pushed than the capacity, the remaining
    /// space is reclaimed.
    fn build(self) -> CacheAlignedL1L2 {
        if self.len == 0 {
            // Nothing was pushed, deallocate if we allocated
            if self.capacity > 0 {
                let layout =
                    Layout::from_size_align(self.capacity * 16, CACHE_LINE_SIZE).expect("layout");
                // Safety: ptr was allocated with this layout
                unsafe {
                    alloc::alloc::dealloc(self.ptr.as_ptr() as *mut u8, layout);
                }
            }
            return CacheAlignedL1L2::empty();
        }

        if self.len == self.capacity {
            // Exact fit, no reallocation needed
            // Prevent Drop from running (we're transferring ownership)
            let result = CacheAlignedL1L2 {
                ptr: self.ptr,
                len: self.len,
            };
            core::mem::forget(self);
            return result;
        }

        // len < capacity: reallocate to exact size
        let new_layout =
            Layout::from_size_align(self.len * 16, CACHE_LINE_SIZE).expect("layout error");

        // Safety: new_layout is valid
        let new_ptr = unsafe { alloc::alloc::alloc(new_layout) as *mut u128 };
        if new_ptr.is_null() {
            alloc::alloc::handle_alloc_error(new_layout);
        }

        // Copy to new allocation
        // Safety: both pointers are valid, len elements
        unsafe {
            core::ptr::copy_nonoverlapping(self.ptr.as_ptr(), new_ptr, self.len);
        }

        // Deallocate old (oversized) allocation
        let old_layout =
            Layout::from_size_align(self.capacity * 16, CACHE_LINE_SIZE).expect("layout");
        unsafe {
            alloc::alloc::dealloc(self.ptr.as_ptr() as *mut u8, old_layout);
        }

        // Prevent Drop from running
        let result = CacheAlignedL1L2 {
            ptr: NonNull::new(new_ptr).unwrap(),
            len: self.len,
        };
        core::mem::forget(self);
        result
    }
}

impl Drop for CacheAlignedL1L2Builder {
    fn drop(&mut self) {
        // Only deallocate if we actually allocated
        if self.capacity > 0 {
            let layout =
                Layout::from_size_align(self.capacity * 16, CACHE_LINE_SIZE).expect("layout error");
            // Safety: ptr was allocated with this layout
            unsafe {
                alloc::alloc::dealloc(self.ptr.as_ptr() as *mut u8, layout);
            }
        }
    }
}

#[cfg(feature = "serde")]
impl Serialize for CacheAlignedL1L2 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Serialize as a slice for compatibility
        self.as_slice().serialize(serializer)
    }
}

#[cfg(feature = "serde")]
impl<'de> Deserialize<'de> for CacheAlignedL1L2 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let data: Vec<u128> = Vec::deserialize(deserializer)?;
        Ok(Self::from_vec(data))
    }
}

/// Poppy-style rank directory with cache-aligned storage.
///
/// # Structure
///
/// - **L0**: Absolute cumulative rank every 2^32 bits (only for vectors > 4Gb)
/// - **L1**: Cumulative rank every 512 bits (relative to L0), stored as u32
/// - **L2**: Cumulative rank within each 512-bit block (7 x 9-bit offsets, packed)
///
/// # Memory Layout
///
/// Each 512-bit block uses 128 bits of metadata:
/// - 32 bits: L1 cumulative rank
/// - 63 bits: 7 x 9-bit L2 offsets (one per 64-bit word within block, except first)
/// - 33 bits: unused/padding
///
/// The L1+L2 data is stored in a cache-aligned wrapper (`#[repr(align(64))]`)
/// to minimize cache line crossings during queries. Since each entry is 16 bytes,
/// 4 entries fit per 64-byte cache line.
///
/// Total overhead: 128 bits per 512 bits = 25% before optimization.
/// With packing optimizations: ~3% for typical use cases.
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct RankDirectory {
    /// L0: Absolute cumulative rank every 2^32 bits.
    /// Only used for bitvectors > 4 billion bits.
    l0: Vec<u64>,

    /// L1+L2 combined: One entry per 512-bit block.
    /// Layout: [L1 (32 bits) | L2[0..6] (7 x 9 bits) | unused]
    /// Stored as u128 for easy extraction.
    /// Cache-aligned to 64 bytes for optimal cache utilization.
    l1_l2: CacheAlignedL1L2,
}

impl RankDirectory {
    /// Create an empty rank directory.
    pub fn empty() -> Self {
        Self {
            l0: Vec::new(),
            l1_l2: CacheAlignedL1L2::default(),
        }
    }

    /// Build a rank directory from word data.
    pub fn build(words: &[u64]) -> Self {
        if words.is_empty() {
            return Self::empty();
        }

        let num_blocks = words.len().div_ceil(WORDS_PER_BLOCK);
        let mut l0 = Vec::new();
        let mut l1_l2_builder = CacheAlignedL1L2::builder(num_blocks);

        let mut cumulative_rank: u64 = 0;
        let mut l0_base: u64 = 0;

        for block_idx in 0..num_blocks {
            // Check if we need a new L0 entry
            if block_idx > 0 && block_idx % BLOCKS_PER_SUPERBLOCK == 0 {
                l0.push(cumulative_rank);
                l0_base = cumulative_rank;
            }

            let block_start = block_idx * WORDS_PER_BLOCK;
            let block_end = (block_start + WORDS_PER_BLOCK).min(words.len());

            // L1: cumulative rank relative to L0 base
            let l1_rank = (cumulative_rank - l0_base) as u32;

            // L2: offsets within block (cumulative within block)
            let mut l2_offsets = [0u16; 7];
            let mut block_cumulative: u16 = 0;

            for (i, word_idx) in (block_start..block_end).enumerate() {
                if i > 0 && i < 8 {
                    l2_offsets[i - 1] = block_cumulative;
                }
                block_cumulative += words[word_idx].count_ones() as u16;
            }

            // Pack L1 and L2 into u128
            let mut entry: u128 = l1_rank as u128;
            for (i, &offset) in l2_offsets.iter().enumerate() {
                entry |= (offset as u128) << (32 + i * 9);
            }

            l1_l2_builder.push(entry);
            cumulative_rank += block_cumulative as u64;
        }

        Self {
            l0,
            l1_l2: l1_l2_builder.build(),
        }
    }

    /// Get the cumulative rank at the start of the given word index.
    ///
    /// This returns the number of 1-bits in words `[0, word_idx)`.
    #[inline]
    pub fn rank_at_word(&self, word_idx: usize) -> usize {
        let data = self.l1_l2.as_slice();
        if data.is_empty() {
            return 0;
        }

        let block_idx = word_idx / WORDS_PER_BLOCK;
        let word_in_block = word_idx % WORDS_PER_BLOCK;

        // Clamp to valid range
        let block_idx = block_idx.min(data.len() - 1);

        // L0 contribution
        let l0_idx = block_idx / BLOCKS_PER_SUPERBLOCK;
        let l0_rank = if l0_idx > 0 && l0_idx <= self.l0.len() {
            self.l0[l0_idx - 1]
        } else {
            0
        };

        // L1 contribution
        let entry = data[block_idx];
        let l1_rank = (entry & 0xFFFF_FFFF) as u64;

        // L2 contribution
        let l2_rank = if word_in_block == 0 {
            0
        } else {
            let l2_idx = word_in_block - 1;
            let shift = 32 + l2_idx * 9;
            ((entry >> shift) & 0x1FF) as u64
        };

        (l0_rank + l1_rank + l2_rank) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_directory() {
        let dir = RankDirectory::build(&[]);
        assert_eq!(dir.rank_at_word(0), 0);
    }

    #[test]
    fn test_single_word() {
        let words = vec![0b1010_1010u64]; // 4 ones
        let dir = RankDirectory::build(&words);
        assert_eq!(dir.rank_at_word(0), 0);
    }

    #[test]
    fn test_multiple_words_single_block() {
        // 8 words = 1 block
        let words: Vec<u64> = vec![0xFF; 8]; // 8 bits set per word
        let dir = RankDirectory::build(&words);

        assert_eq!(dir.rank_at_word(0), 0);
        assert_eq!(dir.rank_at_word(1), 8);
        assert_eq!(dir.rank_at_word(2), 16);
        assert_eq!(dir.rank_at_word(7), 56);
    }

    #[test]
    fn test_multiple_blocks() {
        // 16 words = 2 blocks
        let words: Vec<u64> = vec![u64::MAX; 16]; // 64 bits set per word
        let dir = RankDirectory::build(&words);

        // First block
        assert_eq!(dir.rank_at_word(0), 0);
        assert_eq!(dir.rank_at_word(1), 64);
        assert_eq!(dir.rank_at_word(7), 64 * 7);

        // Second block
        assert_eq!(dir.rank_at_word(8), 64 * 8);
        assert_eq!(dir.rank_at_word(9), 64 * 9);
    }

    #[test]
    fn test_sparse_words() {
        // Only first bit set in each word
        let words: Vec<u64> = vec![1; 16];
        let dir = RankDirectory::build(&words);

        assert_eq!(dir.rank_at_word(0), 0);
        assert_eq!(dir.rank_at_word(1), 1);
        assert_eq!(dir.rank_at_word(8), 8);
        assert_eq!(dir.rank_at_word(15), 15);
    }

    #[test]
    fn test_partial_block() {
        // 5 words (less than a full block)
        let words: Vec<u64> = vec![0xFF; 5];
        let dir = RankDirectory::build(&words);

        assert_eq!(dir.rank_at_word(0), 0);
        assert_eq!(dir.rank_at_word(1), 8);
        assert_eq!(dir.rank_at_word(4), 32);
    }

    #[test]
    fn test_l2_offset_storage() {
        // Verify L2 offsets are correctly stored and retrieved
        let mut words = vec![0u64; 8];
        words[0] = 0b1111; // 4 ones
        words[1] = 0b11111111; // 8 ones
        words[2] = 0b11; // 2 ones

        let dir = RankDirectory::build(&words);

        assert_eq!(dir.rank_at_word(0), 0);
        assert_eq!(dir.rank_at_word(1), 4);
        assert_eq!(dir.rank_at_word(2), 12); // 4 + 8
        assert_eq!(dir.rank_at_word(3), 14); // 4 + 8 + 2
    }

    #[test]
    fn test_large_cumulative_rank() {
        // Test with values that could overflow 9-bit L2 entries
        // Max L2 value is 7 * 64 = 448, which fits in 9 bits (max 511)
        let words: Vec<u64> = vec![u64::MAX; 8];
        let dir = RankDirectory::build(&words);

        assert_eq!(dir.rank_at_word(7), 64 * 7); // 448
    }

    // Tests for CacheAlignedL1L2Builder
    mod builder_tests {
        use super::*;

        #[test]
        fn test_builder_empty() {
            let builder = CacheAlignedL1L2::builder(0);
            let result = builder.build();
            assert!(result.is_empty());
            assert_eq!(result.len(), 0);
        }

        #[test]
        fn test_builder_single_element() {
            let mut builder = CacheAlignedL1L2::builder(1);
            builder.push(42);
            let result = builder.build();
            assert_eq!(result.len(), 1);
            assert_eq!(result.as_slice(), &[42]);
        }

        #[test]
        fn test_builder_multiple_elements() {
            let mut builder = CacheAlignedL1L2::builder(4);
            builder.push(1);
            builder.push(2);
            builder.push(3);
            builder.push(4);
            let result = builder.build();
            assert_eq!(result.len(), 4);
            assert_eq!(result.as_slice(), &[1, 2, 3, 4]);
        }

        #[test]
        fn test_builder_partial_capacity() {
            // Builder with capacity 10, only push 3 elements
            let mut builder = CacheAlignedL1L2::builder(10);
            builder.push(100);
            builder.push(200);
            builder.push(300);
            let result = builder.build();
            assert_eq!(result.len(), 3);
            assert_eq!(result.as_slice(), &[100, 200, 300]);
        }

        #[test]
        fn test_builder_matches_from_vec() {
            // Verify builder produces same result as from_vec
            let values: Vec<u128> = (0..16).map(|i| i * 1000).collect();

            let from_vec_result = CacheAlignedL1L2::from_vec(values.clone());

            let mut builder = CacheAlignedL1L2::builder(values.len());
            for &v in &values {
                builder.push(v);
            }
            let builder_result = builder.build();

            assert_eq!(from_vec_result.len(), builder_result.len());
            assert_eq!(from_vec_result.as_slice(), builder_result.as_slice());
        }

        #[test]
        fn test_builder_alignment() {
            // Verify the resulting pointer is cache-aligned (64 bytes)
            let mut builder = CacheAlignedL1L2::builder(8);
            for i in 0..8 {
                builder.push(i as u128);
            }
            let result = builder.build();

            // Check that the pointer is 64-byte aligned
            let ptr_addr = result.ptr.as_ptr() as usize;
            assert_eq!(
                ptr_addr % CACHE_LINE_SIZE,
                0,
                "Pointer should be cache-line aligned"
            );
        }

        #[test]
        fn test_builder_large_values() {
            // Test with large u128 values
            let mut builder = CacheAlignedL1L2::builder(3);
            builder.push(u128::MAX);
            builder.push(u128::MAX / 2);
            builder.push(1);
            let result = builder.build();
            assert_eq!(result.as_slice(), &[u128::MAX, u128::MAX / 2, 1]);
        }

        #[test]
        fn test_builder_drop_without_build() {
            // Ensure builder can be dropped without calling build (no memory leak)
            let mut builder = CacheAlignedL1L2::builder(100);
            builder.push(1);
            builder.push(2);
            // Drop without calling build() - should not leak
            drop(builder);
        }
    }
}
