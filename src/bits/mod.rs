//! Bitvector implementations with rank and select support.
//!
//! This module provides succinct bitvector data structures with O(1) rank
//! and O(log n) select operations.
//!
//! # Data Structures
//!
//! - [`BitVec`] - Main bitvector with integrated rank/select indices
//! - [`RankDirectory`] - 3-level Poppy-style rank index (~25% overhead, cache-aligned)
//! - [`SelectIndex`] - Sampled select index (density-dependent; see its docs)
//! - [`EliasFano`](crate::bits::EliasFano) - Elias-Fano encoding for monotone integer sequences
//!
//! # Example
//!
//! ```
//! use succinctly::bits::BitVec;
//! use succinctly::RankSelect;
//!
//! let bv = BitVec::from_words(vec![0b1010_1010u64], 8);
//! assert_eq!(bv.rank1(4), 2);
//! assert_eq!(bv.select1(1), Some(3));
//! ```

mod bitvec;
mod elias_fano;
pub(crate) mod popcount;
mod rank;
mod scan;
mod select;

pub use bitvec::BitVec;
pub use elias_fano::{EliasFano, EliasFanoCursor, EliasFanoIter};
pub use popcount::{popcount_word, popcount_word_portable, popcount_words};
pub use rank::RankDirectory;
pub use scan::{block_popcount_portable, scan_select, scan_select_scalar, select_from, BLOCK};
pub use select::{SampleWord, SelectIndex};
