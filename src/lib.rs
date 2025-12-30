//! # Succinctly
//!
//! High-performance succinct data structures for Rust.
//!
//! This crate provides space-efficient data structures with fast rank and select operations,
//! optimized for both x86_64 and ARM (NEON) architectures.
//!
//! ## Quick Start
//!
//! ```
//! use succinctly::{BitVec, RankSelect};
//!
//! // Create a bitvector from u64 words
//! let words = vec![0b1010_1010_1010_1010u64; 8];
//! let bv = BitVec::from_words(words, 512);
//!
//! // Query rank (count of 1-bits in [0, i))
//! assert_eq!(bv.rank1(8), 4);
//!
//! // Query select (position of k-th 1-bit)
//! assert_eq!(bv.select1(0), Some(1));
//! ```
//!
//! ## Features
//!
//! Popcount strategies (mutually exclusive, for benchmarking):
//! - Default: Uses Rust's `count_ones()` which auto-vectorizes
//! - `simd` - Use explicit SIMD intrinsics (NEON on ARM, POPCNT on x86)
//! - `portable-popcount` - Use portable bitwise algorithm (no intrinsics)
//!
//! Other features:
//! - `select0` - Include select0 index (increases memory usage)
//! - `serde` - Enable serialization/deserialization support

#![cfg_attr(not(test), no_std)]

#[cfg(not(test))]
extern crate alloc;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

mod bitvec;
pub mod bp;
mod broadword;
pub mod json;
mod popcount;
mod rank;
mod select;
mod table;

// Keep simd module for its tests, but the main code uses popcount module
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
mod simd;

pub use bitvec::BitVec;
pub use broadword::select_in_word;
pub use popcount::{popcount_word, popcount_words};
pub use rank::RankDirectory;
pub use select::SelectIndex;

/// Trait for rank/select operations on bitvectors.
///
/// Rank and select are fundamental operations for succinct data structures:
/// - `rank1(i)`: Count 1-bits in positions `[0, i)`
/// - `select1(k)`: Find position of the k-th 1-bit (0-indexed)
pub trait RankSelect {
    /// Count 1-bits in positions `[0, i)`.
    ///
    /// Returns 0 if `i == 0`.
    fn rank1(&self, i: usize) -> usize;

    /// Count 0-bits in positions `[0, i)`.
    ///
    /// Default implementation: `i - rank1(i)`
    #[inline]
    fn rank0(&self, i: usize) -> usize {
        i - self.rank1(i)
    }

    /// Find position of the k-th 1-bit (0-indexed).
    ///
    /// Returns `None` if fewer than `k+1` ones exist.
    fn select1(&self, k: usize) -> Option<usize>;

    /// Find position of the k-th 0-bit (0-indexed).
    ///
    /// Returns `None` if fewer than `k+1` zeros exist.
    fn select0(&self, k: usize) -> Option<usize>;
}

/// Configuration for building indices.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Config {
    /// Sample rate for select acceleration (default: 256)
    pub select_sample_rate: u32,
    /// Whether to build select0 index (default: false)
    pub build_select0: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            select_sample_rate: 256,
            build_select0: false,
        }
    }
}
