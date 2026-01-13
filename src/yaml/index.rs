//! YAML index structures for efficient navigation.
//!
//! Holds the semi-index (IB, BP, TY) and provides rank/select operations.

#[cfg(not(test))]
use alloc::vec::Vec;

use crate::trees::BalancedParens;
use crate::util::broadword::select_in_word;

use super::error::YamlError;
use super::light::YamlCursor;
use super::parser::build_semi_index;

/// Index structures for navigating YAML.
///
/// The type parameter `W` controls how the underlying data is stored:
/// - `Vec<u64>` for owned data (built from YAML text)
/// - `&[u64]` for borrowed data (e.g., from mmap)
#[derive(Clone, Debug)]
pub struct YamlIndex<W = Vec<u64>> {
    /// Interest bits - marks positions of structural elements
    ib: W,
    /// Number of valid bits in IB
    ib_len: usize,
    /// Cumulative popcount per word (for fast rank/select on IB)
    ib_rank: Vec<u32>,
    /// Balanced parentheses - encodes the YAML structure as a tree
    bp: BalancedParens<W>,
    /// Type bits - 0 = mapping, 1 = sequence at each container position
    ty: W,
    /// Number of valid bits in TY
    #[allow(dead_code)]
    ty_len: usize,
    /// Direct BP open index to text position mapping.
    /// Entry i = text byte offset for the i-th BP open.
    bp_to_text: Vec<u32>,
}

/// Build cumulative popcount index for IB.
/// Returns a vector where entry i = total 1-bits in words [0, i).
fn build_ib_rank(words: &[u64]) -> Vec<u32> {
    let mut rank = Vec::with_capacity(words.len() + 1);
    let mut cumulative: u32 = 0;
    rank.push(0);
    for &word in words {
        cumulative += word.count_ones();
        rank.push(cumulative);
    }
    rank
}

impl YamlIndex<Vec<u64>> {
    /// Build a YAML index from YAML text.
    ///
    /// This parses the YAML to build the interest bits (IB), balanced
    /// parentheses (BP), and type bits (TY) index structures.
    pub fn build(yaml: &[u8]) -> Result<Self, YamlError> {
        let semi = build_semi_index(yaml)?;

        let ib_len = yaml.len();
        let ib_rank = build_ib_rank(&semi.ib);

        Ok(Self {
            ib: semi.ib,
            ib_len,
            ib_rank,
            bp: BalancedParens::new(semi.bp, semi.bp_len),
            ty: semi.ty,
            ty_len: semi.ty_len,
            bp_to_text: semi.bp_to_text,
        })
    }
}

impl<W: AsRef<[u64]>> YamlIndex<W> {
    /// Create a YAML index from pre-existing IB, BP, TY, and bp_to_text data.
    ///
    /// This is useful for loading serialized index data.
    pub fn from_parts(
        ib: W,
        ib_len: usize,
        bp: W,
        bp_len: usize,
        ty: W,
        ty_len: usize,
        bp_to_text: Vec<u32>,
    ) -> Self {
        let ib_rank = build_ib_rank(ib.as_ref());

        Self {
            ib,
            ib_len,
            ib_rank,
            bp: BalancedParens::from_words(bp, bp_len),
            ty,
            ty_len,
            bp_to_text,
        }
    }

    /// Get the text byte offset for a BP position.
    ///
    /// The BP position must be at an open parenthesis (1-bit).
    /// Returns `None` if the position is invalid.
    #[inline]
    pub fn bp_to_text_pos(&self, bp_pos: usize) -> Option<usize> {
        // We need the index of this open in bp_to_text
        // That's the count of 1-bits in BP before bp_pos (inclusive)
        // which is rank1(bp_pos + 1) - 1 if the bit at bp_pos is 1
        // Actually it's simpler: rank1(bp_pos) gives count before,
        // so the index is rank1(bp_pos) if the bit is 1
        let open_idx = self.bp.rank1(bp_pos);
        self.bp_to_text.get(open_idx).map(|&pos| pos as usize)
    }

    /// Get a reference to the interest bits words.
    #[inline]
    pub fn ib(&self) -> &[u64] {
        self.ib.as_ref()
    }

    /// Get the number of valid bits in IB.
    #[inline]
    pub fn ib_len(&self) -> usize {
        self.ib_len
    }

    /// Get a reference to the balanced parentheses.
    #[inline]
    pub fn bp(&self) -> &BalancedParens<W> {
        &self.bp
    }

    /// Get a reference to the type bits.
    #[inline]
    pub fn ty(&self) -> &[u64] {
        self.ty.as_ref()
    }

    /// Get the number of valid bits in TY.
    #[inline]
    pub fn ty_len(&self) -> usize {
        self.ty_len
    }

    /// Create a cursor at the root of the YAML document.
    #[inline]
    pub fn root<'a>(&'a self, text: &'a [u8]) -> YamlCursor<'a, W> {
        YamlCursor::new(self, text, 0)
    }

    /// Check if the container at the given TY index is a sequence.
    ///
    /// Returns `true` for sequence, `false` for mapping.
    #[inline]
    pub fn is_sequence_at(&self, ty_idx: usize) -> bool {
        if ty_idx >= self.ty_len {
            return false;
        }
        let word_idx = ty_idx / 64;
        let bit_idx = ty_idx % 64;
        let ty_words = self.ty.as_ref();
        if word_idx < ty_words.len() {
            (ty_words[word_idx] >> bit_idx) & 1 == 1
        } else {
            false
        }
    }

    /// Perform select1 with a hint for the starting word index.
    ///
    /// Uses exponential search (galloping) from the hint, which is optimal for
    /// sequential access patterns.
    #[inline]
    pub fn ib_select1_from(&self, k: usize, hint: usize) -> Option<usize> {
        let words = self.ib.as_ref();
        if words.is_empty() {
            return None;
        }

        let k32 = k as u32;
        let n = words.len();

        // Clamp hint to valid range
        let hint = hint.min(n.saturating_sub(1));

        // Check if hint is already past k
        let hint_rank = self.ib_rank[hint + 1];
        let lo;
        let hi;

        if hint_rank <= k32 {
            // k is at or after hint - search forward with exponential expansion
            let mut bound = 1usize;
            let mut prev = hint;

            loop {
                let next = (hint + bound).min(n);
                if next >= n || self.ib_rank[next + 1] > k32 {
                    lo = prev;
                    hi = next;
                    break;
                }
                prev = next;
                bound *= 2;
            }
        } else {
            // k is before hint - search backward with exponential expansion
            let mut bound = 1usize;
            let mut prev = hint;

            loop {
                let next = hint.saturating_sub(bound);
                if next == 0 || self.ib_rank[next + 1] <= k32 {
                    lo = next;
                    hi = prev;
                    break;
                }
                prev = next;
                bound *= 2;
            }
        }

        // Binary search within [lo, hi]
        let mut lo = lo;
        let mut hi = hi;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.ib_rank[mid + 1] <= k32 {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }

        if lo >= n {
            return None;
        }

        // Now lo is the word index, and ib_rank[lo] is count before this word
        let remaining = k - self.ib_rank[lo] as usize;
        let word = words[lo];
        let bit_pos = select_in_word(word, remaining as u32) as usize;
        let result = lo * 64 + bit_pos;

        if result < self.ib_len {
            Some(result)
        } else {
            None
        }
    }

    /// Perform select1 on the IB using pure binary search.
    #[inline]
    pub fn ib_select1(&self, k: usize) -> Option<usize> {
        let words = self.ib.as_ref();
        if words.is_empty() {
            return None;
        }

        let k32 = k as u32;
        let n = words.len();

        // Binary search over all words
        let mut lo = 0usize;
        let mut hi = n;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.ib_rank[mid + 1] <= k32 {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }

        if lo >= n {
            return None;
        }

        let remaining = k - self.ib_rank[lo] as usize;
        let word = words[lo];
        let bit_pos = select_in_word(word, remaining as u32) as usize;
        let result = lo * 64 + bit_pos;

        if result < self.ib_len {
            Some(result)
        } else {
            None
        }
    }

    /// Perform rank1 on the IB (count 1-bits in [0, pos)).
    pub fn ib_rank1(&self, pos: usize) -> usize {
        if pos == 0 {
            return 0;
        }

        let words = self.ib.as_ref();
        let word_idx = pos / 64;
        let bit_idx = pos % 64;

        // Use cumulative index for full words
        let mut count = self.ib_rank[word_idx.min(words.len())] as usize;

        // Add partial word
        if word_idx < words.len() && bit_idx > 0 {
            let mask = (1u64 << bit_idx) - 1;
            count += (words[word_idx] & mask).count_ones() as usize;
        }

        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_simple_mapping() {
        let yaml = b"name: Alice";
        let index = YamlIndex::build(yaml);
        assert!(index.is_ok());
    }

    #[test]
    fn test_build_simple_sequence() {
        let yaml = b"- item1\n- item2";
        let index = YamlIndex::build(yaml);
        assert!(index.is_ok());
    }

    #[test]
    fn test_root_cursor() {
        let yaml = b"name: Alice";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);
        assert_eq!(root.bp_position(), 0);
    }
}
