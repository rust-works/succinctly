//! Semi-index for DSV data.

use crate::bits::BitVec;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Semi-index for DSV data enabling fast field/row navigation.
///
/// The index consists of two bit vectors:
/// - `markers`: Bits set at field delimiter positions (filtered by quote state)
/// - `newlines`: Bits set at newline positions (filtered by quote state)
///
/// Both vectors are filtered to exclude delimiters and newlines that appear
/// inside quoted fields.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct DsvIndex {
    /// Bit vector marking field delimiter positions (filtered by quote state).
    /// A bit is set at position i if `text[i]` is a delimiter outside quotes.
    pub markers: BitVec,

    /// Bit vector marking newline positions (filtered by quote state).
    /// A bit is set at position i if `text[i]` is a newline outside quotes.
    pub newlines: BitVec,

    /// Total byte length of indexed text.
    pub text_len: usize,
}

impl DsvIndex {
    /// Create a new DsvIndex.
    pub fn new(markers: BitVec, newlines: BitVec, text_len: usize) -> Self {
        Self {
            markers,
            newlines,
            text_len,
        }
    }

    /// Number of field boundaries (delimiters + newlines).
    pub fn marker_count(&self) -> usize {
        self.markers.count_ones()
    }

    /// Number of rows (newline count).
    pub fn row_count(&self) -> usize {
        self.newlines.count_ones()
    }

    /// Check if the index is empty.
    pub fn is_empty(&self) -> bool {
        self.text_len == 0
    }
}
