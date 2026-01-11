//! High-performance DSV (Delimiter-Separated Values) parsing with succinct indexing.
//!
//! This module provides a fast CSV/TSV/DSV parser that uses succinct data structures
//! to enable O(1) rank and O(log n) select operations for field navigation.
//!
//! The key insight is that instead of parsing CSV into a data structure, we create
//! two bit vectors (markers and newlines) that enable fast navigation via rank/select.
//!
//! # Example
//!
//! ```
//! use succinctly::dsv::{Dsv, DsvConfig};
//!
//! let csv = b"name,age,city\nAlice,30,NYC\nBob,25,LA\n";
//! let dsv = Dsv::parse(csv);
//!
//! // Iterate over rows
//! for row in dsv.rows() {
//!     let fields: Vec<_> = row.fields().collect();
//!     println!("{:?}", fields);
//! }
//!
//! // Random access to row 1, column 0
//! if let Some(row) = dsv.row(1) {
//!     if let Some(field) = row.get(0) {
//!         println!("Name: {:?}", std::str::from_utf8(field));
//!     }
//! }
//! ```
//!
//! # Performance
//!
//! - Index building: ~100+ MiB/s with SIMD (AVX2/NEON), ~10 MiB/s scalar
//! - Memory overhead: ~3-4% of input size
//! - Field access: O(1) rank + O(log n) select
//!
//! ## SIMD Acceleration
//!
//! On x86_64 and ARM64 platforms, the parser uses SIMD instructions to process
//! 64 bytes at a time:
//! - **AVX2** (x86_64): 2x 32-byte loads with `_mm256_cmpeq_epi8`
//! - **SSE2** (x86_64 fallback): 4x 16-byte loads
//! - **NEON** (ARM64): 4x 16-byte loads with `vceqq_u8`
//!
//! The algorithm uses prefix XOR to compute quote state in parallel, avoiding
//! the sequential dependency of byte-by-byte quote tracking.

mod config;
mod cursor;
mod index;
mod index_lightweight;
mod parser;
pub mod simd;

pub use config::DsvConfig;
pub use cursor::{DsvCursor, DsvFields, DsvRow, DsvRows};
pub use index::DsvIndex;

// Use SIMD parser by default on supported platforms
#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
pub use simd::build_index_simd as build_index;

// Fallback to scalar parser on unsupported platforms
#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
pub use parser::build_index;

// Also export the scalar parser for testing/comparison
pub use parser::build_index as build_index_scalar;

#[cfg(not(test))]
use alloc::vec::Vec;

/// Owned DSV data with index.
///
/// This struct owns both the text data and the index, making it easy to
/// work with DSV data without lifetime management.
#[derive(Clone, Debug)]
pub struct Dsv {
    text: Vec<u8>,
    index: DsvIndex,
}

impl Dsv {
    /// Parse DSV data with default configuration (CSV).
    pub fn parse(text: &[u8]) -> Self {
        Self::parse_with_config(text, &DsvConfig::default())
    }

    /// Parse DSV data with custom configuration.
    pub fn parse_with_config(text: &[u8], config: &DsvConfig) -> Self {
        let index = build_index(text, config);
        Self {
            text: text.to_vec(),
            index,
        }
    }

    /// Create a cursor for navigation.
    pub fn cursor(&self) -> DsvCursor<'_> {
        DsvCursor::new(&self.text, &self.index)
    }

    /// Iterate over rows.
    pub fn rows(&self) -> DsvRows<'_> {
        DsvRows::new(&self.text, &self.index)
    }

    /// Number of rows (newline count).
    pub fn row_count(&self) -> usize {
        self.index.row_count()
    }

    /// Get a specific row by index (0-indexed).
    pub fn row(&self, n: usize) -> Option<DsvRow<'_>> {
        let mut cursor = self.cursor();
        if cursor.goto_row(n) {
            Some(DsvRow::from_cursor(cursor))
        } else {
            None
        }
    }

    /// Get the raw text data.
    pub fn text(&self) -> &[u8] {
        &self.text
    }

    /// Get the index.
    pub fn index(&self) -> &DsvIndex {
        &self.index
    }
}

/// Borrowed DSV reference (for memory-mapped files or zero-copy use).
#[derive(Clone, Copy, Debug)]
pub struct DsvRef<'a> {
    text: &'a [u8],
    index: &'a DsvIndex,
}

impl<'a> DsvRef<'a> {
    /// Create a new borrowed DSV reference.
    pub fn new(text: &'a [u8], index: &'a DsvIndex) -> Self {
        Self { text, index }
    }

    /// Create a cursor for navigation.
    pub fn cursor(&self) -> DsvCursor<'a> {
        DsvCursor::new(self.text, self.index)
    }

    /// Iterate over rows.
    pub fn rows(&self) -> DsvRows<'a> {
        DsvRows::new(self.text, self.index)
    }

    /// Number of rows.
    pub fn row_count(&self) -> usize {
        self.index.row_count()
    }

    /// Get a specific row by index (0-indexed).
    pub fn row(&self, n: usize) -> Option<DsvRow<'a>> {
        let mut cursor = self.cursor();
        if cursor.goto_row(n) {
            Some(DsvRow::from_cursor(cursor))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_csv() {
        let csv = b"a,b,c\n1,2,3\n";
        let dsv = Dsv::parse(csv);

        assert_eq!(dsv.row_count(), 2);

        let row0: Vec<_> = dsv.row(0).unwrap().fields().collect();
        assert_eq!(
            row0,
            vec![b"a".as_slice(), b"b".as_slice(), b"c".as_slice()]
        );

        let row1: Vec<_> = dsv.row(1).unwrap().fields().collect();
        assert_eq!(
            row1,
            vec![b"1".as_slice(), b"2".as_slice(), b"3".as_slice()]
        );
    }

    #[test]
    fn test_quoted_fields() {
        let csv = b"\"hello, world\",b\n";
        let dsv = Dsv::parse(csv);

        let row: Vec<_> = dsv.row(0).unwrap().fields().collect();
        assert_eq!(row.len(), 2);
        assert_eq!(row[0], b"\"hello, world\"");
        assert_eq!(row[1], b"b");
    }

    #[test]
    fn test_quoted_newlines() {
        let csv = b"\"line1\nline2\",b\n";
        let dsv = Dsv::parse(csv);

        // Only one logical row (the newline inside quotes is not a row delimiter)
        assert_eq!(dsv.row_count(), 1);
    }

    #[test]
    fn test_empty_csv() {
        let csv = b"";
        let dsv = Dsv::parse(csv);
        assert_eq!(dsv.row_count(), 0);
    }

    #[test]
    fn test_single_field() {
        let csv = b"hello\n";
        let dsv = Dsv::parse(csv);

        assert_eq!(dsv.row_count(), 1);
        let row: Vec<_> = dsv.row(0).unwrap().fields().collect();
        assert_eq!(row, vec![b"hello".as_slice()]);
    }

    #[test]
    fn test_tsv() {
        let tsv = b"a\tb\tc\n1\t2\t3\n";
        let dsv = Dsv::parse_with_config(tsv, &DsvConfig::tsv());

        let row0: Vec<_> = dsv.row(0).unwrap().fields().collect();
        assert_eq!(
            row0,
            vec![b"a".as_slice(), b"b".as_slice(), b"c".as_slice()]
        );
    }

    #[test]
    fn test_row_iteration() {
        let csv = b"a,b\n1,2\n3,4\n";
        let dsv = Dsv::parse(csv);

        let rows: Vec<Vec<_>> = dsv.rows().map(|r| r.fields().collect()).collect();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], vec![b"a".as_slice(), b"b".as_slice()]);
        assert_eq!(rows[1], vec![b"1".as_slice(), b"2".as_slice()]);
        assert_eq!(rows[2], vec![b"3".as_slice(), b"4".as_slice()]);
    }

    #[test]
    fn test_get_column() {
        let csv = b"name,age,city\nAlice,30,NYC\n";
        let dsv = Dsv::parse(csv);

        let row = dsv.row(1).unwrap();
        assert_eq!(row.get(0), Some(b"Alice".as_slice()));
        assert_eq!(row.get(1), Some(b"30".as_slice()));
        assert_eq!(row.get(2), Some(b"NYC".as_slice()));
        assert_eq!(row.get(3), None);
    }
}
