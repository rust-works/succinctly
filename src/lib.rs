//! # Succinctly
//!
//! High-performance succinct data structures for Rust.
//!
//! This crate provides space-efficient data structures with fast rank and select operations,
//! optimized for both x86_64 and ARM (NEON) architectures.
//!
//! ## Module Organization
//!
//! - [`bits`] - Bitvector with O(1) rank and O(log n) select
//! - [`trees`] - Succinct tree representations (balanced parentheses)
//! - [`json`] - JSON semi-indexing with SIMD acceleration
//! - [`jq`] - jq-style query language for JSON navigation
//! - [`dsv`] - High-performance CSV/TSV parsing with succinct indexing
//! - [`yaml`] - YAML semi-indexing (Phase 1: block style)
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
//! - `serde` - Enable serialization/deserialization support

// Use no_std unless std feature is enabled or we're in test mode
#![cfg_attr(not(any(test, feature = "std")), no_std)]

// When using no_std, we need to explicitly link the alloc crate
#[cfg(not(any(test, feature = "std")))]
extern crate alloc;

// When using std, re-export alloc types from std for compatibility
#[cfg(any(test, feature = "std"))]
extern crate std as alloc;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

// =============================================================================
// Core modules (organized by category)
// =============================================================================

/// Bitvector implementations with rank and select support.
pub mod bits;

/// Succinct tree representations.
pub mod trees;

/// Internal utilities (not part of public API).
pub(crate) mod util;

/// Binary serialization utilities.
pub mod binary;

// =============================================================================
// Application modules
// =============================================================================

/// JSON semi-indexing with SIMD acceleration.
pub mod json;

/// jq-style query language for JSON navigation.
pub mod jq;

/// High-performance DSV (CSV/TSV) parsing with succinct indexing.
pub mod dsv;

/// YAML semi-indexing (Phase 1: YAML-lite).
pub mod yaml;

// =============================================================================
// Public re-exports (convenience + backward compatibility)
// =============================================================================

// Core types
pub use bits::BitVec;
pub use bits::{popcount_word, popcount_words, RankDirectory, SelectIndex};
pub use trees::BalancedParens;
pub use util::select_in_word;

// DSV types
pub use dsv::{Dsv, DsvConfig, DsvCursor, DsvIndex};

// =============================================================================
// Backward compatibility aliases
// =============================================================================

/// Backward compatibility alias for [`trees`] module.
///
/// Use `succinctly::trees` instead.
#[doc(hidden)]
pub mod bp {
    pub use crate::trees::*;
}

// =============================================================================
// Core traits
// =============================================================================

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
}

// =============================================================================
// Configuration
// =============================================================================

/// Configuration for building indices.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Config {
    /// Sample rate for select acceleration (default: 256)
    pub select_sample_rate: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            select_sample_rate: 256,
        }
    }
}
