//! Semi-index for DSV data.

use super::index_lightweight::DsvIndexLightweight;
use crate::bits::BitVec;

#[cfg(not(test))]
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
///
/// Internally uses a lightweight index structure (simple cumulative rank arrays)
/// for 5-9x faster iteration compared to full BitVec with RankDirectory + SelectIndex.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct DsvIndex {
    inner: DsvIndexLightweight,
}

impl DsvIndex {
    /// Create a new DsvIndex from lightweight index.
    pub fn new_lightweight(inner: DsvIndexLightweight) -> Self {
        Self { inner }
    }

    /// Create a new DsvIndex from BitVec (legacy API).
    pub fn new(markers: BitVec, newlines: BitVec, text_len: usize) -> Self {
        let markers_words = markers.words().to_vec();
        let newlines_words = newlines.words().to_vec();
        let inner = DsvIndexLightweight::new(markers_words, newlines_words, text_len);
        Self { inner }
    }

    /// Get reference to inner lightweight index.
    #[inline]
    pub fn as_lightweight(&self) -> &DsvIndexLightweight {
        &self.inner
    }

    /// Number of field boundaries (delimiters + newlines).
    #[inline]
    pub fn marker_count(&self) -> usize {
        self.inner.marker_count()
    }

    /// Number of rows (newline count).
    #[inline]
    pub fn row_count(&self) -> usize {
        self.inner.row_count()
    }

    /// Check if the index is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Rank operation on markers.
    #[inline]
    pub fn markers_rank1(&self, i: usize) -> usize {
        self.inner.markers_rank1(i)
    }

    /// Select operation on markers.
    #[inline]
    pub fn markers_select1(&self, k: usize) -> Option<usize> {
        self.inner.markers_select1(k)
    }

    /// Rank operation on newlines.
    #[inline]
    pub fn newlines_rank1(&self, i: usize) -> usize {
        self.inner.newlines_rank1(i)
    }

    /// Select operation on newlines.
    #[inline]
    pub fn newlines_select1(&self, k: usize) -> Option<usize> {
        self.inner.newlines_select1(k)
    }
}
