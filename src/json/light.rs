#![allow(clippy::items_after_test_module)] // STYLE-0004: helper items intentionally follow `mod tests` in this file
//! StandardJson - Lazy JSON navigation using the standard cursor.
//!
//! This module provides a cursor-based API for navigating JSON structures
//! without fully parsing the JSON text. Values are only decoded when explicitly
//! requested, allowing efficient access to specific parts of large JSON documents.
//!
//! # Design
//!
//! The API is based on the haskell-works `hw-json` library, adapted for Rust:
//!
//! - **Zero-copy navigation**: Cursors are lightweight position markers that don't
//!   allocate memory or parse JSON text during navigation.
//!
//! - **Immutable iteration**: `JsonFields` and `JsonElements` provide immutable
//!   iteration via `uncons()` which returns `(head, tail)` without mutation.
//!
//! - **Lazy decoding**: String and number values are only parsed when you call
//!   methods like `as_str()` or `as_i64()`.
//!
//! - **Generic storage**: Works with both owned (`Vec<u64>`) and borrowed (`&[u64]`)
//!   index data, supporting mmap-based workflows.
//!
//! # Example
//!
//! ```
//! use succinctly::json::light::{JsonIndex, StandardJson};
//!
//! let json = br#"{"name": "Alice", "age": 30}"#;
//! let index = JsonIndex::build(json);
//! let root = index.root(json);
//!
//! if let StandardJson::Object(fields) = root.value() {
//!     if let Some(name) = fields.find("name").unwrap() {
//!         if let StandardJson::String(s) = name {
//!             assert_eq!(&*s.as_str().unwrap(), "Alice");
//!         }
//!     }
//! }
//! ```

#[cfg(not(test))]
use alloc::{borrow::Cow, string::String, vec::Vec};

#[cfg(test)]
use std::borrow::Cow;

use core::cell::{Cell, OnceCell};

use crate::trees::BalancedParens;
use crate::util::broadword::select_in_word;

// ============================================================================
// JsonIndex: Holds the IB and BP index structures
// ============================================================================

/// Index structures for navigating JSON.
///
/// The type parameter `W` controls how the underlying data is stored:
/// - `Vec<u64>` for owned data (built from JSON text)
/// - `&[u64]` for borrowed data (e.g., from mmap)
///
/// Use [`JsonIndex::build`] to create an owned index from JSON text,
/// or [`JsonIndex::from_parts`] to create from pre-existing index data.
#[derive(Clone)]
pub struct JsonIndex<W = Vec<u64>> {
    /// Interest bits - marks positions of structural characters and value starts
    ib: W,
    /// Number of valid bits in IB
    ib_len: usize,
    /// Cumulative popcount per word (for fast rank/select on IB)
    ib_rank: Vec<u32>,
    /// Balanced parentheses - encodes the JSON structure as a tree
    bp: BalancedParens<W>,
    /// Line starts for line/column lookup, built lazily on first use.
    ///
    /// Only [`to_line_column`](JsonIndex::to_line_column) and
    /// [`to_offset`](JsonIndex::to_offset) need it — the `at_position` jq
    /// builtin and the locate CLIs — so building it during `build()` would
    /// charge every jq query for something almost no query reads (#228).
    lines: OnceCell<crate::text::LineIndex>,
    /// Where the previous [`ib_select1_sequential`](JsonIndex::ib_select1_sequential)
    /// lookup landed, so the next forward lookup can start its gallop there
    /// instead of from the fixed `rank / 8` estimate (#2168).
    ///
    /// Same shape as YAML's `AdvancePositions` sequential cursor (O1): a
    /// `Cell` on the shared index, written by `&self` methods. This struct
    /// already holds a `core::cell::OnceCell` (`lines`), so it was never
    /// `Sync`; the `Cell` adds no new constraint.
    ///
    /// Deliberately left out of `Debug` (hand-written below): it changes on
    /// every navigation, so a derived impl would make `{:?}` of an index --
    /// or of any `JsonCursor`, which prints its index -- differ between two
    /// cursors that reached the same node by different routes. YAML's
    /// `AdvancePositions` cache has that exact trap on record.
    seq_hint: Cell<SeqHint>,
}

impl<W: core::fmt::Debug> core::fmt::Debug for JsonIndex<W> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("JsonIndex")
            .field("ib", &self.ib)
            .field("ib_len", &self.ib_len)
            .field("ib_rank", &self.ib_rank)
            .field("bp", &self.bp)
            .field("lines", &self.lines)
            .finish_non_exhaustive()
    }
}

/// The state behind [`JsonIndex::ib_select1_sequential`]: the rank the last
/// lookup answered and the text position it answered with.
///
/// Advisory except for the exact-repeat case. A seed derived from a stale
/// `pos` is still a legal seed for [`JsonIndex::ib_select1_from`], whose
/// gallop-then-bisect is exact for any seed -- the cache can only change how
/// many `ib_rank` probes a lookup costs, never what it answers. The one
/// answer it *does* supply directly is for `k == rank`, where `pos` is the
/// value the same deterministic search returned a moment ago.
#[derive(Clone, Copy, Debug)]
struct SeqHint {
    /// `u32::MAX` means "no previous lookup" -- a rank the index cannot hold,
    /// since every constructor asserts `ib_len <= u32::MAX` (#188).
    rank: u32,
    /// Fits: a `Some` from `ib_select1_from` is `< ib_len <= u32::MAX`.
    pos: u32,
}

impl SeqHint {
    const NONE: Self = Self {
        rank: u32::MAX,
        pos: 0,
    };
}

/// Build cumulative popcount index for IB.
/// Returns a vector where entry i = total 1-bits in words [0, i).
///
/// The `u32` accumulator is safe because every constructor asserts
/// `ib_len <= u32::MAX` (#188), and set bits <= ib_len. Widening to `u64`
/// would double a hot per-word array (~6.25% of input) for inputs the index
/// cannot represent anyway.
fn build_ib_rank(words: &[u64]) -> Vec<u32> {
    let mut rank = Vec::with_capacity(words.len() + 1);
    let mut cumulative: u32 = 0;
    rank.push(0); // rank[0] = 0 (no words before word 0)
    for &word in words {
        cumulative += word.count_ones();
        rank.push(cumulative);
    }
    rank
}

impl JsonIndex<Vec<u64>> {
    /// Build a JSON index from JSON text.
    ///
    /// This parses the JSON to build the interest bits (IB) and balanced
    /// parentheses (BP) index structures, plus newline positions for
    /// fast line/column lookup.
    ///
    /// On supported platforms (aarch64, x86_64), this automatically uses
    /// SIMD-accelerated indexing for better performance.
    ///
    /// # Panics
    ///
    /// Panics if the input exceeds `u32::MAX` bytes (just under 4 GiB): the
    /// IB rank directory stores cumulative counts as `u32` (#188). Larger
    /// inputs would previously truncate silently.
    pub fn build(json: &[u8]) -> Self {
        assert!(
            u32::try_from(json.len()).is_ok(),
            "JsonIndex supports inputs up to u32::MAX (4294967295) bytes; got {} bytes (#188)",
            json.len()
        );
        #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
        let semi = crate::json::simd::build_semi_index_standard(json);

        #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
        let semi = crate::json::standard::build_semi_index(json);

        let ib_len = json.len();

        // Count actual BP bits
        let bp_bit_count = count_bp_bits(&semi.bp);

        // Build cumulative popcount index for IB
        let ib_rank = build_ib_rank(&semi.ib);

        Self {
            ib: semi.ib,
            ib_len,
            ib_rank,
            bp: BalancedParens::new(semi.bp, bp_bit_count),
            lines: OnceCell::new(),
            seq_hint: Cell::new(SeqHint::NONE),
        }
    }
}

impl<W: AsRef<[u64]>> JsonIndex<W> {
    /// Create a JSON index from pre-existing IB and BP data.
    ///
    /// This is useful for loading serialized index data, e.g., from mmap.
    /// Line/column lookup works as usual: the line index is derived from the
    /// text on first use, not from the serialized parts.
    ///
    /// # Arguments
    ///
    /// * `ib` - Interest bits data
    /// * `ib_len` - Number of valid bits in IB (typically == JSON text length)
    /// * `bp` - Balanced parentheses data
    /// * `bp_len` - Number of valid bits in BP
    ///
    /// # Panics
    ///
    /// Panics if `ib_len` exceeds `u32::MAX` bits (#188): the IB rank
    /// directory stores cumulative counts as `u32`. (Pathological JSON can
    /// also push `bp_len` past `u32::MAX` first; `BalancedParens` asserts its
    /// own ceiling.)
    pub fn from_parts(ib: W, ib_len: usize, bp: W, bp_len: usize) -> Self {
        assert!(
            u32::try_from(ib_len).is_ok(),
            "JsonIndex supports inputs up to u32::MAX (4294967295) bytes; got {ib_len} bits (#188)"
        );
        // Build cumulative popcount index for IB
        let ib_rank = build_ib_rank(ib.as_ref());

        Self {
            ib,
            ib_len,
            ib_rank,
            bp: BalancedParens::from_words(bp, bp_len),
            lines: OnceCell::new(),
            seq_hint: Cell::new(SeqHint::NONE),
        }
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

    /// Convert byte offset to 1-indexed line and column.
    ///
    /// Returns (line, column) where both are 1-indexed.
    /// Useful for error reporting and position-based navigation.
    ///
    /// `text` must be the JSON text the index was built from; the line index
    /// is derived from it on first use and cached (#228).
    ///
    /// # Performance
    ///
    /// O(log lines) via [`LineIndex`](crate::text::LineIndex), plus a one-off
    /// O(n) scan the first time either this or [`Self::to_offset`] is called.
    #[inline]
    pub fn to_line_column(&self, offset: usize, text: &[u8]) -> (usize, usize) {
        self.ensure_lines(text).to_line_column(offset)
    }

    /// Convert 1-indexed line and column to byte offset.
    ///
    /// Column is 1-indexed byte offset within the line.
    /// Returns `None` if line/column is 0 or if the position is out of bounds.
    ///
    /// `text` must be the JSON text the index was built from; the line index
    /// is derived from it on first use and cached (#228).
    #[inline]
    pub fn to_offset(&self, line: usize, column: usize, text: &[u8]) -> Option<usize> {
        self.ensure_lines(text).to_offset(line, column)
    }

    /// Get the line index, building it lazily on first use.
    ///
    /// The first caller's `text` wins for the lifetime of the index, so a
    /// later call with different text silently reads the first one's line
    /// map. The debug assertion catches the cheap half of that mistake.
    #[inline]
    fn ensure_lines(&self, text: &[u8]) -> &crate::text::LineIndex {
        let lines = self
            .lines
            .get_or_init(|| crate::text::LineIndex::build(text));

        debug_assert_eq!(
            lines.text_len(),
            text.len(),
            "line index was built from different text ({} bytes) than this call passed ({} bytes)",
            lines.text_len(),
            text.len()
        );

        lines
    }

    /// Create a cursor at the root of the JSON document.
    ///
    /// # Arguments
    ///
    /// * `text` - The original JSON text (must match the text used to build the index)
    #[inline]
    pub fn root<'a>(&'a self, text: &'a [u8]) -> JsonCursor<'a, W> {
        JsonCursor {
            text,
            index: self,
            bp_pos: 0,
        }
    }

    /// Perform select1 with a hint for the starting word index.
    ///
    /// Uses exponential search (galloping) from the hint, which is optimal for
    /// sequential access patterns. When iterating through elements, the next
    /// select is typically near the previous one, so starting from the hint
    /// gives O(log d) where d is the distance, instead of O(log n).
    ///
    /// # Performance
    ///
    /// - **Sequential access**: O(log d) where d = distance from hint (~3.3x faster)
    /// - **Random access**: O(log n) with ~37% overhead vs pure binary search
    ///
    /// For random access patterns (e.g., `.[42]`), prefer [`Self::ib_select1`].
    #[inline]
    pub fn ib_select1_from(&self, k: usize, hint: usize) -> Option<usize> {
        let words = self.ib.as_ref();
        if words.is_empty() {
            return None;
        }

        let k32 = k as u32;
        let n = words.len();

        // #40: count `ib_rank` probes so this path's cost can be compared with
        // the word-scan sites. Starts at 1 for the `hint_rank` probe below.
        #[cfg(any(feature = "select-stats", test))]
        let mut probes = 1usize;

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

            // Gallop forward: double the step size until we overshoot
            loop {
                #[cfg(any(feature = "select-stats", test))]
                {
                    probes += 1;
                }
                let next = (hint + bound).min(n);
                if next >= n || self.ib_rank[next + 1] > k32 {
                    // Found the range: [prev, next]
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

            // Gallop backward
            loop {
                #[cfg(any(feature = "select-stats", test))]
                {
                    probes += 1;
                }
                let next = hint.saturating_sub(bound);
                if next == 0 || self.ib_rank[next + 1] <= k32 {
                    // Found the range: [next, prev]
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
            #[cfg(any(feature = "select-stats", test))]
            {
                probes += 1;
            }
            let mid = lo + (hi - lo) / 2;
            if self.ib_rank[mid + 1] <= k32 {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }

        #[cfg(any(feature = "select-stats", test))]
        crate::util::select_stats::record(
            crate::util::select_stats::Site::JsonIbSelectFrom,
            probes,
        );

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

    /// [`ib_select1_from`](Self::ib_select1_from) seeded from the previous
    /// call through this method (#2168).
    ///
    /// `ib_select1_from` needs a starting word. The seed this method replaced
    /// was the fixed estimate `k / 8`, which assumes eight interest bits per
    /// 64-byte word; real documents drift from that (a node-dense array of
    /// short numbers has ~12, one of long strings far fewer), and once the
    /// estimate is `d` words off every lookup pays an O(log d) gallop before
    /// it can bisect -- `docs/optimizations/select-scan.md` had already
    /// measured that at ~17 probes per call on the real-workload corpus.
    ///
    /// The seed is now extrapolated from the last answer at the document's
    /// own density: `last_word + (k - last_rank) * words / ones`, in either
    /// direction, or `k * words / ones` when nothing has been asked yet. For
    /// a document-order walk (`JsonCursor::text_position` from `to_owned`,
    /// the #1755/#1953 validity walk, streaming output, `.[]` iteration) the
    /// next rank is one or two above the last, so the seed is the answer's
    /// own word or the one after it and the gallop is O(1) amortized. A
    /// backward ask -- the value after its key in `find_cursor`, a `parent`
    /// step, a root ask from `key` between two elements -- extrapolates the
    /// same way, which is why this is not forward-only: a forward-only draft
    /// fell back to `k / 8` on every backward ask and measured the common
    /// `.users[] | .name` shape at ~23 probes for each of its two backward
    /// asks per element (review of PR #2578). Random access after a
    /// sequential run lands near the true word for any document whose
    /// density is roughly uniform, which is a strictly better fallback than
    /// `k / 8` for the same reason.
    ///
    /// An exact repeat (`k == last_rank`) answers from the cache without
    /// probing at all, the way YAML's `AdvancePositions` `last_ib_arg` fast
    /// path does; `DocumentCursor` callers routinely ask the same node twice
    /// (`text_position()` then `value()`).
    ///
    /// Cost is a function of how far the seed is from the true word, not a
    /// constant: `1 + ~2 log2(gap)` probes. Consecutive interest bits within
    /// a few words of each other (any document of short scalars) cost 3;
    /// a document of 64 KB strings, where consecutive bits are ~1000 words
    /// apart, costs ~20 -- still below the ~33 the fixed estimate needs
    /// there, because the extrapolation absorbs the density.
    #[inline]
    pub fn ib_select1_sequential(&self, k: usize) -> Option<usize> {
        let last = self.seq_hint.get();
        if last.rank != u32::MAX && k == last.rank as usize {
            return Some(last.pos as usize);
        }
        let hint = self.seed_word(k, last);
        let result = self.ib_select1_from(k, hint);
        if let Some(pos) = result {
            // Both fit: a `Some` means `k < ones <= ib_len` and
            // `pos < ib_len`, and every constructor asserts
            // `ib_len <= u32::MAX` (#188).
            self.seq_hint.set(SeqHint {
                rank: k as u32,
                pos: pos as u32,
            });
        }
        result
    }

    /// The starting word for [`ib_select1_sequential`](Self::ib_select1_sequential):
    /// the last answer's word, moved by the rank delta at the document's
    /// mean bits-per-word, or the delta from rank 0 when there is no last
    /// answer. Always in `0..words`, which `ib_select1_from` requires.
    ///
    /// `u64`/`i64` arithmetic throughout: `k * words` reaches 2^58 on a
    /// 4 GB document, which a 32-bit `usize` cannot hold.
    #[inline]
    fn seed_word(&self, k: usize, last: SeqHint) -> usize {
        let words = self.ib.as_ref().len();
        let ones = *self.ib_rank.last().unwrap_or(&0) as u64;
        if words == 0 || ones == 0 {
            return 0;
        }
        let (from_rank, from_word) = if last.rank == u32::MAX {
            (0i64, 0i64)
        } else {
            (i64::from(last.rank), i64::from(last.pos / 64))
        };
        let delta = k as i64 - from_rank;
        // `delta * words` is at most 2^32 * 2^26 in magnitude; no overflow.
        let seed = from_word + delta * words as i64 / ones as i64;
        seed.clamp(0, words as i64 - 1) as usize
    }

    /// Perform select1 on the IB using pure binary search.
    ///
    /// This is optimal for random access patterns (e.g., `.[42]`, slicing).
    /// For sequential access (e.g., `.[]` iteration), use [`Self::ib_select1_from`]
    /// with a hint for O(log d) instead of O(log n) performance.
    ///
    /// Returns the position of the k-th 1-bit (0-indexed).
    ///
    /// # Performance
    ///
    /// - **Random access**: O(log n) - optimal for indexed lookups
    /// - **Sequential access**: Use `ib_select1_from` instead for ~3.3x speedup
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

    /// Perform rank1 on the IB (count 1-bits in [0, pos)).
    ///
    /// Uses cumulative popcount index for O(1) performance.
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

// Helper to count actual BP bits (number of open + close parens)
fn count_bp_bits(bp_words: &[u64]) -> usize {
    // For standard cursor, we need to count actual meaningful bits
    // This is a simplification - in practice we'd track this during indexing
    // For now, estimate based on popcount (opens) * 2
    let total_ones: usize = bp_words.iter().map(|w| w.count_ones() as usize).sum();
    // Each node has one open and one close, so total bits = opens + closes = 2 * opens
    // But this is approximate - the actual length should be tracked during build
    total_ones * 2
}

// ============================================================================
// JsonCursor: Position in the JSON structure
// ============================================================================

/// A cursor pointing to a position in the JSON structure.
///
/// Cursors are lightweight (just a position integer) and cheap to copy.
/// Navigation methods return new cursors without mutation.
#[derive(Debug)]
pub struct JsonCursor<'a, W = Vec<u64>> {
    /// The original JSON text
    text: &'a [u8],
    /// Reference to the index
    index: &'a JsonIndex<W>,
    /// Position in the BP vector (0 = root)
    bp_pos: usize,
}

// Manual Clone/Copy impl since W is only used through a reference
impl<W> Clone for JsonCursor<'_, W> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<W> Copy for JsonCursor<'_, W> {}

impl<'a, W: AsRef<[u64]>> JsonCursor<'a, W> {
    /// Create a cursor at a specific BP position.
    ///
    /// This is useful for constructing cursors when you know the BP position
    /// directly, such as when walking up the tree using `parent()`.
    #[inline]
    pub fn from_bp_position(index: &'a JsonIndex<W>, text: &'a [u8], bp_pos: usize) -> Self {
        Self {
            text,
            index,
            bp_pos,
        }
    }

    /// Get the position in the BP vector.
    #[inline]
    pub fn bp_position(&self) -> usize {
        self.bp_pos
    }

    /// The document text this cursor navigates.
    ///
    /// With [`index`](Self::index) and [`bp_position`](Self::bp_position)
    /// this is the full input to [`from_bp_position`](Self::from_bp_position),
    /// so a caller that has to buffer many cursors can keep just the
    /// `bp_pos` of each and rebuild them against one hoisted
    /// `(text, index)` pair. Both members are invariant across a whole
    /// document, so storing them per cursor is pure duplication -- 24 of a
    /// cursor's 32 bytes (#1385).
    #[inline]
    pub fn text(&self) -> &'a [u8] {
        self.text
    }

    /// The semi-index this cursor navigates. See [`text`](Self::text).
    #[inline]
    pub fn index(&self) -> &'a JsonIndex<W> {
        self.index
    }

    /// Check if this cursor points to a container (array or object).
    ///
    /// This is a **fast** operation that only uses the BP structure -
    /// no text_position lookup is needed. Containers have children in
    /// the BP tree; leaves (strings, numbers, bools, null) don't.
    ///
    /// Use this when you only need to distinguish containers from leaves
    /// without reading the actual value content.
    #[inline]
    pub fn is_container(&self) -> bool {
        self.index.bp().first_child(self.bp_pos).is_some()
    }

    /// Get the byte position in the JSON text.
    ///
    /// This uses select1 on the IB to find the text position corresponding
    /// to this BP position.
    pub fn text_position(&self) -> Option<usize> {
        // The BP position corresponds to the n-th interest bit in IB
        // We need to find which 1-bit in IB corresponds to this BP position
        //
        // For standard cursor:
        // - BP has one open paren for each structural character/value start
        // - IB has one bit set for each structural character/value start
        // - So BP position N corresponds to the N-th set bit in IB
        //
        // Use BP's O(1) rank1 function instead of linear scan
        let rank = self.index.bp().rank1(self.bp_pos);

        // Seeded from the previous lookup rather than the fixed `rank / 8`
        // estimate this used to pass to `ib_select1_from` directly: a
        // document-order walk resolves every node's position in turn, and
        // the estimate's drift on node-dense input made each of those a
        // gallop (#2168, see `ib_select1_sequential`).
        self.index.ib_select1_sequential(rank)
    }

    /// Get the 1-based line number of this node's position in the JSON text.
    ///
    /// Returns 0 if the position cannot be resolved (should not normally
    /// happen for a valid cursor).
    #[inline]
    pub fn line(&self) -> usize {
        let offset = self.text_position().unwrap_or(0);
        let (line, _column) = self.index.to_line_column(offset, self.text);
        line
    }

    /// Get the 1-based column number of this node's position in the JSON text.
    ///
    /// Returns 0 if the position cannot be resolved (should not normally
    /// happen for a valid cursor).
    #[inline]
    pub fn column(&self) -> usize {
        let offset = self.text_position().unwrap_or(0);
        let (_line, column) = self.index.to_line_column(offset, self.text);
        column
    }

    /// Navigate to the first child.
    ///
    /// Returns `None` if this position has no children (is a leaf or close paren).
    #[inline]
    pub fn first_child(&self) -> Option<Self> {
        let new_pos = self.index.bp().first_child(self.bp_pos)?;
        Some(JsonCursor {
            text: self.text,
            index: self.index,
            bp_pos: new_pos,
        })
    }

    /// Navigate to the next sibling.
    ///
    /// Returns `None` if this is the last sibling.
    #[inline]
    pub fn next_sibling(&self) -> Option<Self> {
        let new_pos = self.index.bp().next_sibling(self.bp_pos)?;
        Some(JsonCursor {
            text: self.text,
            index: self.index,
            bp_pos: new_pos,
        })
    }

    /// Navigate to the previous sibling.
    ///
    /// Returns `None` if this is the first sibling.
    #[inline]
    pub fn prev_sibling(&self) -> Option<Self> {
        let new_pos = self.index.bp().prev_sibling(self.bp_pos)?;
        Some(JsonCursor {
            text: self.text,
            index: self.index,
            bp_pos: new_pos,
        })
    }

    /// Navigate to the parent.
    ///
    /// Returns `None` if this is the root.
    #[inline]
    pub fn parent(&self) -> Option<Self> {
        let new_pos = self.index.bp().parent(self.bp_pos)?;
        Some(JsonCursor {
            text: self.text,
            index: self.index,
            bp_pos: new_pos,
        })
    }

    /// Get the JSON value at this cursor position.
    ///
    /// This calls `text_position()` to determine the value type.
    pub fn value(&self) -> StandardJson<'a, W> {
        let Some(text_pos) = self.text_position() else {
            return StandardJson::Error("invalid cursor position");
        };
        self.value_at(text_pos)
    }

    /// Same as [`value`](Self::value), for a caller that has already
    /// resolved this cursor's `text_position()` for some other reason (a
    /// delimiter gap check against a sibling, #1643) and doesn't want to
    /// pay for the same rank/select lookup a second time.
    ///
    /// `text_pos` must be this cursor's own `text_position()` -- passing
    /// any other offset produces nonsense, silently, since there is
    /// nothing here to check it against.
    pub fn value_at(&self, text_pos: usize) -> StandardJson<'a, W> {
        if text_pos >= self.text.len() {
            return StandardJson::Error("text position out of bounds");
        }

        match self.text[text_pos] {
            b'{' => StandardJson::Object(JsonFields::from_object_cursor(*self)),
            b'[' => StandardJson::Array(JsonElements::from_array_cursor(*self)),
            b'"' => StandardJson::String(JsonString {
                text: self.text,
                start: text_pos,
            }),
            b't' | b'f' => {
                // true or false
                if self.text[text_pos..].starts_with(b"true") {
                    StandardJson::Bool(true)
                } else if self.text[text_pos..].starts_with(b"false") {
                    StandardJson::Bool(false)
                } else {
                    StandardJson::Error("invalid boolean")
                }
            }
            b'n' => {
                if self.text[text_pos..].starts_with(b"null") {
                    StandardJson::Null
                } else if special_number_end(self.text, text_pos).is_some() {
                    // `nan`, `NaN5`: decNumber's NaN shares its first byte
                    // with `null` (#2877). `null` stays first and
                    // unchanged, so a genuine null takes no new branch;
                    // the word test runs only once that prefix test has
                    // already failed, i.e. in what was the error arm.
                    StandardJson::Number(JsonNumber {
                        text: self.text,
                        start: text_pos,
                    })
                } else {
                    StandardJson::Error("invalid null")
                }
            }
            // A leading `.` is accepted here too (in addition to `-`/an
            // ASCII digit) -- real jq's own number reader is lenient
            // beyond strict JSON (`.5` -> `0.5`, #1171). No grammar
            // validation here beyond the leading byte: this is a nested
            // container's own field/element, and `nested_number_span`'s
            // own doc comment explains why a malformed trailing shape
            // must still resolve to one `Number` span rather than
            // `Error`, matching this crate's established #966 precedent.
            c if c == b'-' || c == b'.' || c.is_ascii_digit() => StandardJson::Number(JsonNumber {
                text: self.text,
                start: text_pos,
            }),
            // jq's remaining number spellings (#2877), each an error arm
            // until now so valid RFC 8259 input still reaches none of
            // them: a leading `+` before a digit or `.` (`+1`, `+.5`,
            // `+1.2.3` -- the last resolving to one span and then `null`
            // downstream, exactly as `1.2.3` does), and decNumber's
            // special words starting with `i`/`I`/`N`/`s`/`S` (`inf`,
            // `Infinity`, `NaN`, `sNaN12`) or with `+` (`+inf`, `+nan`).
            // A word that isn't one of those (`infx`, `nanx`, `Nope`) and
            // a `+` before anything else (`+`, `+-1`, `+x`) stay errors:
            // #966's "malformed span becomes `null`" precedent covers a
            // number-*shaped* span only and must not grow to letters.
            b'+' if matches!(self.text.get(text_pos + 1), Some(b'0'..=b'9' | b'.')) => {
                StandardJson::Number(JsonNumber {
                    text: self.text,
                    start: text_pos,
                })
            }
            b'+' | b'i' | b'I' | b'N' | b's' | b'S'
                if special_number_end(self.text, text_pos).is_some() =>
            {
                StandardJson::Number(JsonNumber {
                    text: self.text,
                    start: text_pos,
                })
            }
            _ => StandardJson::Error("unexpected character"),
        }
    }

    /// Get children of this cursor for traversal.
    ///
    /// **Key optimization**: This method uses only BP structure operations
    /// (`first_child`, `next_sibling`) - no expensive `text_position()` calls.
    /// Use this for efficient traversal when you don't need to read values.
    ///
    /// Returns an iterator over child cursors.
    #[inline]
    pub fn children(&self) -> JsonChildren<'a, W> {
        JsonChildren {
            current: self.first_child(),
        }
    }

    /// Get the byte range in the original text for this value.
    ///
    /// Returns `(start, end)` where `text[start..end]` is the raw JSON bytes
    /// for this value, preserving original formatting.
    ///
    /// For containers (arrays/objects), uses BP structure to find the closing bracket.
    /// For scalars (strings/numbers/bools/null), scans text to find value end.
    pub fn text_range(&self) -> Option<(usize, usize)> {
        let start = self.text_position()?;

        if start >= self.text.len() {
            return None;
        }

        let end = match self.text[start] {
            // Containers: scan text for matching close bracket.
            // Closing brackets have IB=0, so we cannot use ib_select1_from to
            // find their text position. Instead, scan forward tracking depth.
            b'{' | b'[' => {
                let close_char = if self.text[start] == b'{' { b'}' } else { b']' };
                let mut depth = 1u32;
                let mut i = start + 1;
                while i < self.text.len() {
                    match self.text[i] {
                        b'"' => {
                            // Skip string contents
                            i += 1;
                            while i < self.text.len() {
                                match self.text[i] {
                                    b'"' => {
                                        i += 1;
                                        break;
                                    }
                                    b'\\' => i += 2,
                                    _ => i += 1,
                                }
                            }
                        }
                        c if c == self.text[start] => {
                            depth += 1;
                            i += 1;
                        }
                        c if c == close_char => {
                            depth -= 1;
                            if depth == 0 {
                                return Some((start, i + 1));
                            }
                            i += 1;
                        }
                        _ => i += 1,
                    }
                }
                return None;
            }
            // String: scan for closing quote
            b'"' => {
                let mut i = start + 1;
                while i < self.text.len() {
                    match self.text[i] {
                        b'"' => return Some((start, i + 1)),
                        b'\\' => i += 2,
                        _ => i += 1,
                    }
                }
                self.text.len()
            }
            // Boolean true
            b't' => {
                if self.text[start..].starts_with(b"true") {
                    start + 4
                } else {
                    return None;
                }
            }
            // Boolean false
            b'f' => {
                if self.text[start..].starts_with(b"false") {
                    start + 5
                } else {
                    return None;
                }
            }
            // Null -- or decNumber's NaN, which shares the first byte
            // (#2877); the `value_at` arm above explains the ordering.
            b'n' => {
                if self.text[start..].starts_with(b"null") {
                    start + 4
                } else {
                    special_number_end(self.text, start)?
                }
            }
            // Number: scan for end of number, matching `value()`'s own
            // dispatch (shared `nested_number_span`, #1171 review) --
            // this arm previously had no leading-dot case at all, so a
            // cursor at a leading-dot number returned `None` here even
            // though `value()` correctly classified it as `Number`.
            c if c == b'-' || c == b'.' || c.is_ascii_digit() => {
                nested_number_span(self.text, start)
            }
            // The #2877 spellings, mirroring `value_at`'s arms exactly:
            // `nested_number_span` reads a `+digit`/`+.` token (and a
            // `+`-signed word, through its own word arm) the same way it
            // reads a `-` one, and a bare word is `special_number_end`'s.
            b'+' if matches!(self.text.get(start + 1), Some(b'0'..=b'9' | b'.')) => {
                nested_number_span(self.text, start)
            }
            b'+' | b'i' | b'I' | b'N' | b's' | b'S' => special_number_end(self.text, start)?,
            _ => return None,
        };

        Some((start, end))
    }

    /// Get the raw bytes for this JSON value.
    ///
    /// Returns the original bytes from the JSON text, preserving formatting.
    /// This is useful for zero-copy output of values.
    pub fn raw_bytes(&self) -> Option<&'a [u8]> {
        let (start, end) = self.text_range()?;
        Some(&self.text[start..end])
    }

    /// Create a cursor at the specified byte offset (0-indexed).
    ///
    /// Returns `None` if:
    /// - The offset is out of bounds
    /// - The offset doesn't correspond to a valid node
    ///
    /// This enables position-based navigation in jq queries via `at_offset(n)`.
    pub fn cursor_at_offset(&self, offset: usize) -> Option<Self> {
        if offset >= self.text.len() {
            return None;
        }

        // Get the rank at this position (count of structural bits before offset)
        let rank = self.index.ib_rank1(offset);

        // Determine which IB index contains this offset
        let ib_idx = if let Some(struct_pos) = self.index.ib_select1(rank) {
            if struct_pos == offset {
                // We're exactly at a structural position
                rank
            } else {
                // We're inside a value - the containing node started at rank-1
                if rank > 0 {
                    rank - 1
                } else {
                    return None;
                }
            }
        } else if rank > 0 {
            rank - 1
        } else {
            return None;
        };

        // Convert IB index to BP position using binary search
        // Find the BP position where the ib_idx-th open paren is located
        let bp = self.index.bp();
        let bp_len = bp.len();

        if bp_len == 0 {
            return None;
        }

        // Binary search for the smallest bp_pos where rank1(bp_pos + 1) > ib_idx
        let mut lo = 0;
        let mut hi = bp_len;

        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let count = bp.rank1(mid + 1);
            if count <= ib_idx {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }

        // Verify the position is valid
        if lo < bp_len && bp.rank1(lo + 1) == ib_idx + 1 {
            Some(JsonCursor {
                text: self.text,
                index: self.index,
                bp_pos: lo,
            })
        } else {
            None
        }
    }

    /// Create a cursor at the specified line and column (1-indexed).
    ///
    /// Returns `None` if:
    /// - Line or column is 0
    /// - The position is out of bounds
    /// - The position doesn't correspond to a valid node
    ///
    /// This enables position-based navigation in jq queries via `at_position(line; col)`.
    pub fn cursor_at_position(&self, line: usize, col: usize) -> Option<Self> {
        // Convert line/column to byte offset
        let offset = self.index.to_offset(line, col, self.text)?;

        // Use cursor_at_offset to find the node
        self.cursor_at_offset(offset)
    }
}

// ============================================================================
// JsonChildren: Fast traversal iterator (BP-only operations)
// ============================================================================

/// Iterator over child cursors using only BP operations.
///
/// This is the fastest way to traverse the JSON structure when you
/// don't need to read the actual values - it uses only `first_child`
/// and `next_sibling` operations without any `text_position()` calls.
#[derive(Debug)]
pub struct JsonChildren<'a, W = Vec<u64>> {
    current: Option<JsonCursor<'a, W>>,
}

impl<W> Clone for JsonChildren<'_, W> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<W> Copy for JsonChildren<'_, W> {}

impl<'a, W: AsRef<[u64]>> Iterator for JsonChildren<'a, W> {
    type Item = JsonCursor<'a, W>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let cursor = self.current?;
        self.current = cursor.next_sibling();
        Some(cursor)
    }
}

// ============================================================================
// StandardJson: The value type
// ============================================================================

/// A JSON value with lazy decoding.
///
/// For objects and arrays, the value contains an iterator-like structure
/// that yields children on demand. For strings and numbers, the raw bytes
/// are stored and only parsed when you call `as_str()` or `as_i64()`.
#[derive(Clone, Debug)]
pub enum StandardJson<'a, W = Vec<u64>> {
    /// A JSON string (quotes not yet stripped, escapes not yet decoded)
    String(JsonString<'a>),
    /// A JSON number (not yet parsed)
    Number(JsonNumber<'a>),
    /// A JSON object with lazy field iteration
    Object(JsonFields<'a, W>),
    /// A JSON array with lazy element iteration
    Array(JsonElements<'a, W>),
    /// A JSON boolean
    Bool(bool),
    /// JSON null
    Null,
    /// An error encountered during navigation
    Error(&'static str),
}

// ============================================================================
// JsonFields: Immutable iteration over object fields
// ============================================================================

/// Immutable "list" of JSON object fields.
///
/// Use `uncons()` to get the first field and the remaining fields,
/// or `is_empty()` to check if there are no more fields.
///
/// This is `Copy` because it just holds a cursor position.
///
/// # Iteration Model
///
/// `JsonFields` holds a cursor pointing to the current key (or None if empty).
/// Each `uncons` returns the (key, value) pair and a new `JsonFields` pointing
/// to the next key (or empty if no more fields).
#[derive(Debug)]
pub struct JsonFields<'a, W = Vec<u64>> {
    /// Cursor pointing to the current field key, or None if exhausted
    key_cursor: Option<JsonCursor<'a, W>>,
}

// Manual Clone/Copy impl since JsonCursor is Copy
impl<W> Clone for JsonFields<'_, W> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<W> Copy for JsonFields<'_, W> {}

impl<'a, W: AsRef<[u64]>> JsonFields<'a, W> {
    /// Create a new JsonFields from an object cursor.
    fn from_object_cursor(object_cursor: JsonCursor<'a, W>) -> Self {
        Self {
            key_cursor: object_cursor.first_child(),
        }
    }

    /// Check if there are no more fields.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.key_cursor.is_none()
    }

    /// Whether this field list ends on a lone child with no sibling to pair
    /// as a value -- `{invalid}`, `{"a"}`, or the trailing `2` of
    /// `{invalid, "b":2}` (#1194).
    ///
    /// The semi-index treats `:` and `,` alike (`json::standard::is_delim`),
    /// so an object's members are recovered by pairing its BP children two at
    /// a time. An odd child count means the text was never `key: value` at
    /// all, and bracket-matching accepted it anyway.
    ///
    /// This is the exact condition [`uncons`](Self::uncons) discards when its
    /// second `?` fires, named once here so the sites that must react to it
    /// cannot drift apart from the site that detects it. O(1) -- the same
    /// `next_sibling` test `uncons` already performs.
    ///
    /// Note this is *not* the negation of [`is_empty`](Self::is_empty): a
    /// malformed list is non-empty **and** yields nothing from `uncons`.
    ///
    /// Returns the offending child's cursor rather than a bare `bool` so a
    /// caller can reach the document text (`JsonCursor::text`) to diagnose it,
    /// and its position to report it. Use
    /// [`ends_unpaired`](Self::ends_unpaired) where only the answer matters.
    #[inline]
    pub fn unpaired_tail(&self) -> Option<JsonCursor<'a, W>> {
        let key_cursor = self.key_cursor?;
        match key_cursor.next_sibling() {
            Some(_) => None,
            None => Some(key_cursor),
        }
    }

    /// Whether this field list ends on an unpaired child (#1194).
    ///
    /// Thin wrapper over [`unpaired_tail`](Self::unpaired_tail) so the
    /// condition has exactly one definition -- two copies of a predicate
    /// drift, and this one is checked from several modules.
    #[inline]
    pub fn ends_unpaired(&self) -> bool {
        self.unpaired_tail().is_some()
    }

    /// Get the first field and the remaining fields.
    ///
    /// Returns `None` if there are no more fields -- **or** if the list ends
    /// on an unpaired child, which is structurally malformed JSON rather than
    /// exhaustion. Callers that must tell those apart ask
    /// [`ends_unpaired`](Self::ends_unpaired); see #1194 for why the
    /// distinction is not folded into this return type.
    pub fn uncons(&self) -> Option<(JsonField<'a, W>, Self)> {
        let key_cursor = self.key_cursor?;

        // Next sibling of key is the value
        let value_cursor = key_cursor.next_sibling()?;

        // The rest starts at the value's next sibling (the next key, if any)
        let rest = JsonFields {
            key_cursor: value_cursor.next_sibling(),
        };

        let field = JsonField {
            key_cursor,
            value_cursor,
        };

        Some((field, rest))
    }

    /// Find a field by name.
    ///
    /// A duplicate JSON key collapses to its *last* occurrence, matching
    /// real jq / RFC 8259 convention (see issue #1251) -- the same rule
    /// `YamlFields::find` already applies for YAML's own last-duplicate-
    /// key-wins semantics (#174), just the opposite of YAML's genuine-
    /// duplicates preservation elsewhere (`to_entries`, #443).
    ///
    /// `Err` when *any* sibling's key isn't string-shaped at all (#1995,
    /// same reasoning as [`find_cursor`](Self::find_cursor)'s own doc
    /// comment) -- real jq rejects a non-string object key at parse time,
    /// unconditional on the document as a whole, so this checks every
    /// candidate as it's found rather than deferring to whichever one (if
    /// any) would otherwise have won. Uses the same `key_is_malformed`/
    /// [`DocumentFields::malformed_member_error`] pair `#1194`'s own shared
    /// walks already use for this exact question, rather than a second,
    /// hand-rolled `match` on `StandardJson::String` -- one definition of
    /// "malformed key", not two that could silently diverge (#106).
    ///
    /// `Err` too when the *winning* occurrence's own `,`/`:` delimiters are
    /// malformed (#1677/#2288), or when there's a trailing stray comma
    /// after the object's real last field (`{"a":1,}`, #2261/#2288) --
    /// unlike the non-string-key check just above, both deferred to the
    /// end rather than checked as each candidate is found: only the field
    /// that actually wins (the *last* matching occurrence) needs its own
    /// `,`/`:` validated, since an earlier same-named field's own delimiter
    /// gap is moot once a later one supersedes it; the trailing-comma check
    /// is unconditional on whether `name` matched anything, same as every
    /// other #2261 fix. Exactly [`find_cursor`](Self::find_cursor)'s own
    /// checks, reused rather than re-derived (#106) -- `find` predates
    /// `find_cursor` and was missing both entirely until #2288 found the
    /// gap via `find_cursor`'s own inherent walk-through duplicate-key
    /// resolution, which `find`'s simpler last-write-wins loop doesn't
    /// share, so the checks have to be threaded through separately here
    /// rather than shared by construction.
    pub fn find(&self, name: &str) -> Result<Option<StandardJson<'a, W>>, EvalError>
    where
        W: Clone,
    {
        // STYLE-0013-TAIL: this is `child_tail_gap_ok`'s single `Some(last)` arm
        // written out, and routing it there would be behaviour-preserving --
        // `JsonCursor::malformed_delimiter_error()` *is*
        // `EvalError::malformed_json_text(self.text)`, so the two raise the same
        // value. It is left as-is because routing to that helper would fix
        // nothing: it shares the blind spot this walk actually has, its `None`
        // arm being `Ok(())`. What closes that is `container_tail_gap_ok`, which
        // needs a cursor to the object itself -- and a `JsonFields` holds only
        // "the first field's key cursor, or `None`", so a zero-field `{,}`
        // leaves it with nothing. #2594 closed the gap one level up instead, at
        // `eval_generic`'s `Expr::Field` arm, which does hold that cursor;
        // this walk is unchanged and still cannot make the check itself.
        let mut fields = *self;
        // (key's own text start, winning field, is this field the object's
        // first) -- same bookkeeping `find_cursor` keeps, needed so the
        // deferred `,`/`:` check below validates only the actual winner.
        let mut winner: Option<(usize, JsonField<'a, W>, bool)> = None;
        // #2261/#2288: the textually last field's own value cursor, in raw
        // document order -- independent of `winner`, which a later
        // non-matching field must not disturb. Same bookkeeping
        // `find_cursor` keeps.
        let mut last_value_cursor: Option<JsonCursor<'a, W>> = None;
        let mut index = 0usize;
        while let Some((field, rest)) = fields.uncons() {
            let key = field.key();
            if key_is_malformed(&key) {
                return Err(fields.malformed_member_error());
            }
            // An undecodable key (invalid UTF-8, an invalid escape, an
            // invalid `\u` codepoint) is *skipped*, not treated as the
            // end of the search. This `?` used to return from `find`
            // itself, so a single such key hid every field after it from
            // lookup -- `.b` answered `null` on `{"\ud800":1,"b":2}` --
            // while `keys`/`length`, which don't decode, still reported
            // them (#1247). Surfacing the decode failure as a real
            // `EvalError` is tracked separately (see `key_is_malformed`'s
            // own doc comment for why this case, unlike #1995's, is
            // deliberately *not* what it answers `true` for); this only
            // stops one bad key destroying valid results.
            if let StandardJson::String(key_str) = key {
                if key_str.as_str().is_ok_and(|k| k == name) {
                    winner = Some((key_str.start(), field, index == 0));
                }
            }
            last_value_cursor = Some(field.value_cursor());
            fields = rest;
            index += 1;
        }
        // STYLE-0013: `preceding_gap_ok` directly, not `key_delimiter_ok`/
        // `value_delimiter_ok` -- both take `is_first`/`text_start` from an
        // already-decoded key/value, but this walk only knows `key_start`
        // (captured once, at the moment a candidate becomes `winner`) and
        // defers the check to the end so only the field that actually wins
        // last-write-wins gets validated, never a superseded earlier
        // candidate. Neither primitive has a "check this position against
        // this cursor" shape that fits a deferred check.
        if let Some((key_start, field, is_first)) = winner {
            let value_cursor = field.value_cursor();
            let comma_expected = if is_first { None } else { Some(b',') };
            if !preceding_gap_ok(value_cursor.text(), key_start, comma_expected) {
                return Err(EvalError::malformed_json_text(value_cursor.text()));
            }
            if let Some(value_start) = value_cursor.text_position() {
                if !preceding_gap_ok(value_cursor.text(), value_start, Some(b':')) {
                    return Err(EvalError::malformed_json_text(value_cursor.text()));
                }
            }
        }
        // #2261/#2288: trailing stray comma after the object's real last
        // field, checked regardless of whether `name` matched anything --
        // see this function's own doc comment above.
        if let Some(last) = last_value_cursor {
            if !trailing_element_gap_ok(&last, b'}') {
                return Err(EvalError::malformed_json_text(last.text()));
            }
        }
        Ok(winner.map(|(_, field, _)| field.value()))
    }

    /// Find a field by name and return a cursor to its value.
    ///
    /// Same last-duplicate-key-wins semantics as [`find`](Self::find) — kept
    /// as a separate loop rather than reusing `find` so the returned cursor
    /// (needed for `line`/`column`) doesn't require re-navigating.
    ///
    /// `Err` when the *winning* occurrence's own `,`/`:` delimiters are
    /// malformed (#1677), when *any* sibling's key isn't even string-shaped
    /// at all (#1995), or when there's a trailing stray comma after the
    /// object's real last field (`{"a":1,}`, #2261) -- a targeted lookup
    /// like `.a` never walks every sibling the way `.[]`/`length`/`keys` do,
    /// so it needs its own checks for all three. A non-string key
    /// (`{"a":1,123:2}`) is real jq's own parse-time rejection ("Object keys
    /// must be strings"), unconditional on the document as a whole -- unlike
    /// the `,`/`:` gap check for the winner, checked as each candidate is
    /// found (not deferred to the winner alone): the document is malformed
    /// the moment *any* member's key isn't a string, whether or not that
    /// member is the one this lookup would otherwise have returned. Same
    /// shared `key_is_malformed`/[`DocumentFields::malformed_member_error`]
    /// pair as [`find`](Self::find) uses for this, not a second copy of the
    /// check (#106).
    ///
    /// **Not actually O(1)**, despite being a targeted single-field lookup:
    /// last-duplicate-key-wins means every candidate could still be
    /// superseded by a later one, so the `while` loop below always walks
    /// every field regardless of `name` or where a match sits (#2261 code
    /// review, verifying the O(1) assumption this function's doc comment
    /// never actually claimed but a caller could reasonably have inferred
    /// from "targeted lookup"). That means checking a trailing stray comma
    /// after the object's real last field here is free -- this walk already
    /// reaches it, for `.a` and `.nonexistent` alike (real jq can't even
    /// parse `{"a":1,}` to begin with, so it rejects *every* field access
    /// into it, matching field or not).
    pub fn find_cursor(&self, name: &str) -> Result<Option<JsonCursor<'a, W>>, EvalError>
    where
        W: Clone,
    {
        // STYLE-0013-TAIL: `find`'s cursor-returning twin, exempt for the same
        // reason -- routing to `child_tail_gap_ok` is behaviour-preserving but
        // pointless, and the `container_tail_gap_ok` that would actually close
        // the zero-child `{,}` case needs a container cursor this walk never
        // receives. Closed at the `Expr::Field` dispatch arm instead (#2594);
        // see `find`'s own marker just above.
        let mut fields = *self;
        // (key's own text start, value cursor, is this field the object's
        // first) for the winning candidate seen so far.
        let mut winner: Option<(usize, JsonCursor<'a, W>, bool)> = None;
        // #2261: the textually last field's own value cursor, in raw
        // document order -- independent of `winner`, which a later
        // non-matching field must not disturb.
        let mut last_value_cursor: Option<JsonCursor<'a, W>> = None;
        let mut index = 0usize;
        while let Some((field, rest)) = fields.uncons() {
            let key = field.key();
            if key_is_malformed(&key) {
                return Err(fields.malformed_member_error());
            }
            // Same undecodable-key skip as `find` above (#1247).
            if let StandardJson::String(key) = key {
                if key.as_str().is_ok_and(|k| k == name) {
                    winner = Some((key.start(), field.value_cursor(), index == 0));
                }
            }
            last_value_cursor = Some(field.value_cursor());
            fields = rest;
            index += 1;
        }
        // STYLE-0013: same reason as `find`'s own exemption -- only
        // `key_start` (captured when a candidate becomes `winner`) is kept,
        // and the check is deferred to the last-write-wins field alone.
        if let Some((key_start, value_cursor, is_first)) = winner {
            let comma_expected = if is_first { None } else { Some(b',') };
            if !preceding_gap_ok(value_cursor.text(), key_start, comma_expected) {
                return Err(EvalError::malformed_json_text(value_cursor.text()));
            }
            if let Some(value_start) = value_cursor.text_position() {
                if !preceding_gap_ok(value_cursor.text(), value_start, Some(b':')) {
                    return Err(EvalError::malformed_json_text(value_cursor.text()));
                }
            }
        }
        // #2261: trailing stray comma after the object's real last field
        // (`{"a":1,}`), checked regardless of whether `name` matched
        // anything -- see this function's own doc comment above.
        if let Some(last) = last_value_cursor {
            if !trailing_element_gap_ok(&last, b'}') {
                return Err(EvalError::malformed_json_text(last.text()));
            }
        }
        Ok(winner.map(|(_, value_cursor, _)| value_cursor))
    }
}

impl<'a, W: AsRef<[u64]>> Iterator for JsonFields<'a, W> {
    type Item = JsonField<'a, W>;

    fn next(&mut self) -> Option<Self::Item> {
        let (field, rest) = self.uncons()?;
        *self = rest;
        Some(field)
    }
}

// ============================================================================
// JsonField: A single key-value pair
// ============================================================================

/// A single field in a JSON object.
#[derive(Debug)]
pub struct JsonField<'a, W = Vec<u64>> {
    key_cursor: JsonCursor<'a, W>,
    value_cursor: JsonCursor<'a, W>,
}

// Manual Clone/Copy impl since JsonCursor is Copy
impl<W> Clone for JsonField<'_, W> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<W> Copy for JsonField<'_, W> {}

impl<'a, W: AsRef<[u64]>> JsonField<'a, W> {
    /// Get the field key.
    ///
    /// A well-formed JSON object's key is always a string, but this
    /// method doesn't itself enforce that (#1995) -- a lazily-indexed
    /// document can present a non-string key (`{"a":1,123:2}`), which
    /// real jq rejects at parse time. Callers that need this distinction
    /// (`find`/`find_cursor`, both in this file) check for it explicitly.
    #[inline]
    pub fn key(&self) -> StandardJson<'a, W> {
        self.key_cursor.value()
    }

    /// Get the field value.
    #[inline]
    pub fn value(&self) -> StandardJson<'a, W> {
        self.value_cursor.value()
    }

    /// Get the value cursor directly.
    ///
    /// This allows access to the cursor for lazy value handling.
    #[inline]
    pub fn value_cursor(&self) -> JsonCursor<'a, W> {
        self.value_cursor
    }

    /// Get the key cursor directly.
    ///
    /// This allows raw-byte access to the key (`raw_bytes()`) without
    /// decoding through `StandardJson::String` first.
    #[inline]
    pub fn key_cursor(&self) -> JsonCursor<'a, W> {
        self.key_cursor
    }
}

// ============================================================================
// JsonElements: Immutable iteration over array elements
// ============================================================================

/// Immutable "list" of JSON array elements.
///
/// Use `uncons()` to get the first element and the remaining elements,
/// or `is_empty()` to check if there are no more elements.
///
/// This is `Copy` because it just holds a cursor position.
///
/// # Iteration Model
///
/// `JsonElements` holds a cursor pointing to the current element (or None if empty).
/// Each `uncons` returns the element value and a new `JsonElements` pointing
/// to the next element (or empty if no more elements).
#[derive(Debug)]
pub struct JsonElements<'a, W = Vec<u64>> {
    /// Cursor pointing to the current element, or None if exhausted
    element_cursor: Option<JsonCursor<'a, W>>,
}

// Manual Clone/Copy impl since JsonCursor is Copy
impl<W> Clone for JsonElements<'_, W> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<W> Copy for JsonElements<'_, W> {}

impl<'a, W: AsRef<[u64]>> JsonElements<'a, W> {
    /// Create a new JsonElements from an array cursor.
    fn from_array_cursor(array_cursor: JsonCursor<'a, W>) -> Self {
        Self {
            element_cursor: array_cursor.first_child(),
        }
    }

    /// Check if there are no more elements.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.element_cursor.is_none()
    }

    /// Get the first element and the remaining elements.
    ///
    /// Returns `None` if there are no more elements.
    pub fn uncons(&self) -> Option<(StandardJson<'a, W>, Self)> {
        let element_cursor = self.element_cursor?;

        let rest = JsonElements {
            element_cursor: element_cursor.next_sibling(),
        };

        let value = element_cursor.value();
        Some((value, rest))
    }

    /// Get the first element's cursor and the remaining elements.
    ///
    /// This is like `uncons` but returns the cursor instead of the value.
    /// Useful for lazy evaluation where you want to defer calling `value()`.
    pub fn uncons_cursor(&self) -> Option<(JsonCursor<'a, W>, Self)> {
        let element_cursor = self.element_cursor?;

        let rest = JsonElements {
            element_cursor: element_cursor.next_sibling(),
        };

        Some((element_cursor, rest))
    }

    /// Get element by index (slow path).
    ///
    /// Note: This is O(n) as it iterates through elements, calling `value()`
    /// for each intermediate element.
    ///
    /// For better performance with random access, use [`get_fast`](Self::get_fast) which
    /// only calls `value()` on the target element.
    pub fn get(&self, index: usize) -> Option<StandardJson<'a, W>> {
        let mut elements = *self;
        for _ in 0..index {
            let (_, rest) = elements.uncons()?;
            elements = rest;
        }
        elements.uncons().map(|(elem, _)| elem)
    }

    /// Get element by index (fast path for random access).
    ///
    /// This method navigates to the target element using only BP operations
    /// (`next_sibling`), avoiding expensive `text_position()` calls for
    /// intermediate elements.
    ///
    /// Complexity: O(n) BP operations + O(log n) IB select for final element.
    /// This is faster than `get()` which does O(n) IB selects.
    #[inline]
    pub fn get_fast(&self, index: usize) -> Option<StandardJson<'a, W>> {
        let mut cursor = self.element_cursor?;

        // Navigate to the target element using only BP operations
        for _ in 0..index {
            cursor = cursor.next_sibling()?;
        }

        // Only call value() (which uses text_position/ib_select) on the target
        Some(cursor.value())
    }
}

impl<'a, W: AsRef<[u64]>> Iterator for JsonElements<'a, W> {
    type Item = StandardJson<'a, W>;

    fn next(&mut self) -> Option<Self::Item> {
        let (elem, rest) = self.uncons()?;
        *self = rest;
        Some(elem)
    }
}

// ============================================================================
// ElementCursorIter: Iterator over element cursors
// ============================================================================

/// Iterator that yields cursors for each array element.
///
/// Unlike `JsonElements` which yields `StandardJson` values, this iterator
/// yields `JsonCursor` values, allowing lazy evaluation of element values.
#[derive(Clone, Copy, Debug)]
pub struct ElementCursorIter<'a, W = Vec<u64>> {
    elements: JsonElements<'a, W>,
}

impl<'a, W: AsRef<[u64]>> ElementCursorIter<'a, W> {
    /// Create a new cursor iterator from JsonElements.
    pub fn new(elements: JsonElements<'a, W>) -> Self {
        Self { elements }
    }
}

impl<'a, W: AsRef<[u64]>> Iterator for ElementCursorIter<'a, W> {
    type Item = JsonCursor<'a, W>;

    fn next(&mut self) -> Option<Self::Item> {
        let (cursor, rest) = self.elements.uncons_cursor()?;
        self.elements = rest;
        Some(cursor)
    }
}

impl<'a, W: AsRef<[u64]>> JsonElements<'a, W> {
    /// Get an iterator over element cursors.
    ///
    /// This allows iterating over array elements while keeping them as
    /// lazy cursor references, deferring value evaluation until needed.
    pub fn cursor_iter(self) -> ElementCursorIter<'a, W> {
        ElementCursorIter::new(self)
    }
}

// ============================================================================
// JsonString: Lazy string decoding
// ============================================================================

/// A JSON string that hasn't been decoded yet.
///
/// Call `as_str()` to decode escape sequences and get the string value.
#[derive(Clone, Copy, Debug)]
pub struct JsonString<'a> {
    text: &'a [u8],
    start: usize,
}

impl<'a> JsonString<'a> {
    /// The byte offset of the opening quote in the document text.
    ///
    /// Lets a caller that already resolved this string via
    /// [`JsonCursor::value`] reuse that position (e.g. for a delimiter gap
    /// check, #1643) instead of paying for another `text_position()` --
    /// itself a rank/select lookup, not free -- to re-derive it.
    #[inline]
    pub fn start(&self) -> usize {
        self.start
    }

    /// The byte offset immediately past the closing quote in the document
    /// text -- a forward scan bounded by this string's own length, not a
    /// rank/select lookup. Lets a caller that only has this key (not the
    /// value that follows it) check the delimiter *forward* from here
    /// instead of resolving the next sibling's `text_position()`, the
    /// exact per-field cost `uncons_key()` exists to avoid (#1677/#1514).
    #[inline]
    pub fn end(&self) -> usize {
        self.find_end()
    }

    /// Get the raw bytes including quotes.
    pub fn raw_bytes(&self) -> &'a [u8] {
        let end = self.find_end();
        &self.text[self.start..end]
    }

    /// The raw source span (quotes included), whether it contains a
    /// backslash escape, and whether it contains a raw DEL byte (`0x7f`),
    /// in a single scan.
    ///
    /// A caller that needs several of these -- the JSON printer, which
    /// writes the span verbatim when nothing needs decoding, and the
    /// duplicate-key probe (#1385), which may compare raw spans only while
    /// nothing is escaped -- would otherwise pay separate passes over the
    /// same bytes: [`raw_bytes`](Self::raw_bytes) scans for the closing
    /// quote, `contains(&b'\\')` scans again, and a DEL check would be a
    /// third. The quote scan already visits every byte to recognise a
    /// backslash and skip what it escapes, so reporting both is free.
    /// Measured worth 7-10% of `sjq '.'` on a 10 MB document (the escape
    /// flag alone), which is the entire cost of the probe.
    ///
    /// `has_del` exists for #2591: DEL is legal raw (unescaped) JSON source,
    /// but jq's own escape table (unlike yq's) still escapes it to
    /// `\u007f` on output -- a span with no backslash is *not* safe to echo
    /// verbatim under jq's convention if it also contains a raw DEL byte.
    /// Never part of a multi-byte UTF-8 sequence (every continuation/lead
    /// byte is `>= 0x80`), so a bare byte-equality check is correct with no
    /// encoding awareness needed.
    pub fn raw_and_escaped(&self) -> (&'a [u8], bool, bool) {
        let mut i = self.start + 1; // skip the opening quote
        let mut escaped = false;
        let mut has_del = false;
        while i < self.text.len() {
            match self.text[i] {
                b'"' => return (&self.text[self.start..=i], escaped, has_del),
                b'\\' => {
                    escaped = true;
                    i += 2;
                }
                0x7f => {
                    has_del = true;
                    i += 1;
                }
                _ => i += 1,
            }
        }
        (&self.text[self.start..], escaped, has_del)
    }

    /// Decode the string value.
    ///
    /// Returns a `Cow::Borrowed` for strings without escapes (zero-copy),
    /// or a `Cow::Owned` for strings that need escape decoding.
    ///
    /// Returns an error if the string contains invalid escape sequences
    /// or invalid UTF-8.
    pub fn as_str(&self) -> Result<Cow<'a, str>, JsonError> {
        // Skip opening quote
        let start = self.start + 1;
        let end = self.find_string_end();

        let bytes = &self.text[start..end];

        // Check if we need to decode escapes
        if !bytes.contains(&b'\\') {
            // No escapes - can return directly (zero-copy)
            let s = core::str::from_utf8(bytes).map_err(|_| JsonError::InvalidUtf8)?;
            Ok(Cow::Borrowed(s))
        } else {
            // Has escapes - need to decode
            decode_escapes(bytes).map(Cow::Owned)
        }
    }

    fn find_end(&self) -> usize {
        self.find_string_end() + 1 // Include closing quote
    }

    fn find_string_end(&self) -> usize {
        let mut i = self.start + 1; // Skip opening quote
        while i < self.text.len() {
            match self.text[i] {
                b'"' => return i,
                b'\\' => i += 2, // Skip escape sequence
                _ => i += 1,
            }
        }
        self.text.len()
    }
}

/// Decode JSON string escape sequences.
///
/// Handles: \\, \", \/, \b, \f, \n, \r, \t, and \uXXXX (including surrogate pairs)
fn decode_escapes(bytes: &[u8]) -> Result<String, JsonError> {
    let mut out = Vec::with_capacity(bytes.len());
    decode_escapes_into::<true>(bytes, &mut out)?;
    // Unreachable in practice: with `VALIDATE_UTF8 = true` every literal
    // chunk was already checked and every escape contributes a `char`'s own
    // encoding, so the concatenation is valid UTF-8 by construction (a `\`
    // is ASCII and so can never split a multi-byte sequence). Mapped rather
    // than `expect`ed to keep this path panic-free regardless.
    //
    // It is therefore a second validation pass over bytes already known to
    // be valid, and `from_utf8_unchecked` would skip it -- measured (A/B,
    // interleaved, 400k escaped strings in a 28 MB document) at -1.3% to
    // -3.7% on the two workloads that decode every string, `-r '.[].s'` and
    // `-S -c '.'`. Not taken: that is too small a win to introduce `unsafe`
    // into this module for, and the obvious safe alternative -- dropping the
    // per-chunk check and relying on this one -- is not equivalent, because
    // it would report `InvalidEscape` where a string carrying both a bad
    // literal byte and a later bad escape reports `InvalidUtf8` today. A
    // sink trait letting `VALIDATE_UTF8 = true` build a `String` directly
    // would get it safely, if the cost ever justifies the machinery.
    String::from_utf8(out).map_err(|_| JsonError::InvalidUtf8)
}

/// The shared core of [`decode_escapes`], writing *bytes* rather than a
/// `String` so a caller holding text that is not (yet) valid UTF-8 can still
/// use it.
///
/// `VALIDATE_UTF8` selects between the two callers' differing needs, without
/// a second copy of this escape table -- three independent copies of one
/// predicate is the drift trap #106 records:
///
/// - `true` ([`decode_escapes`], the hot `as_str` path): a literal run that
///   is not valid UTF-8 fails immediately with [`JsonError::InvalidUtf8`],
///   exactly where it did when this loop pushed `&str` chunks into a
///   `String`. Preserving the *position* of that check, rather than deferring
///   it to one `String::from_utf8` at the end, keeps the reported error kind
///   unchanged for a string carrying both a bad literal byte and a later bad
///   escape.
/// - `false` (`jq::utf8_document`'s per-string repair, #1743): invalid bytes
///   pass through verbatim, because reproducing jq's own substitution timing
///   requires running the substitution over the *decoded* bytes -- jq
///   substitutes inside `jv_string_sized`, after its lexer has decoded the
///   escapes, not over the raw source span.
///
/// The escape errors themselves are reported identically under both, so a
/// string the `false` caller cannot decode is exactly one the parser will
/// reject anyway.
pub(crate) fn decode_escapes_into<const VALIDATE_UTF8: bool>(
    bytes: &[u8],
    out: &mut Vec<u8>,
) -> Result<(), JsonError> {
    /// Append `c`'s UTF-8 encoding. `char::encode_utf8` needs a scratch
    /// buffer; `String::push`'s own byte-level equivalent is not public.
    fn push_char(out: &mut Vec<u8>, c: char) {
        let mut buf = [0u8; 4];
        out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
    }

    let result = &mut *out;
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'\\' {
            if i + 1 >= bytes.len() {
                return Err(JsonError::InvalidEscape);
            }
            i += 1;
            match bytes[i] {
                b'"' => result.push(b'"'),
                b'\\' => result.push(b'\\'),
                b'/' => result.push(b'/'),
                b'b' => result.push(0x08), // backspace
                b'f' => result.push(0x0C), // form feed
                b'n' => result.push(b'\n'),
                b'r' => result.push(b'\r'),
                b't' => result.push(b'\t'),
                b'u' => {
                    // Unicode escape: \uXXXX
                    if i + 4 >= bytes.len() {
                        return Err(JsonError::InvalidUnicodeEscape);
                    }
                    let hex = &bytes[i + 1..i + 5];
                    let codepoint = parse_hex4(hex)?;
                    i += 4;

                    // Check for surrogate pair
                    if (0xD800..=0xDBFF).contains(&codepoint) {
                        // High surrogate - must be followed by low surrogate
                        if i + 6 < bytes.len() && bytes[i + 1] == b'\\' && bytes[i + 2] == b'u' {
                            let low_hex = &bytes[i + 3..i + 7];
                            let low = parse_hex4(low_hex)?;
                            if (0xDC00..=0xDFFF).contains(&low) {
                                // Valid surrogate pair
                                let cp = 0x10000
                                    + ((codepoint as u32 - 0xD800) << 10)
                                    + (low as u32 - 0xDC00);
                                if let Some(c) = char::from_u32(cp) {
                                    push_char(result, c);
                                    i += 6; // Skip \uXXXX for low surrogate
                                } else {
                                    return Err(JsonError::InvalidUnicodeEscape);
                                }
                            } else {
                                return Err(JsonError::InvalidUnicodeEscape);
                            }
                        } else {
                            return Err(JsonError::InvalidUnicodeEscape);
                        }
                    } else if (0xDC00..=0xDFFF).contains(&codepoint) {
                        // #2008: an unpaired *low* surrogate (`\uDC00`-`\uDFFF`)
                        // is a different case from the unpaired *high*
                        // surrogate arm above -- real jq 1.7.1 doesn't reject
                        // this one at all: it accepts the document and
                        // substitutes U+FFFD (confirmed live: `{"a":"\udc00"}`
                        // decodes to `{"a":"�"}`, exit 0). Substituting
                        // here rather than erroring matches that, where the
                        // high-surrogate arm's `Err` (falling back to the raw
                        // span) remains the already-documented leniency for
                        // the case jq genuinely does reject.
                        push_char(result, '\u{FFFD}');
                    } else {
                        // Regular BMP character: codepoint is a u16 (from
                        // parse_hex4) outside 0xD800-0xDFFF (both arms above
                        // already cover that whole range), so it's always a
                        // valid char and this can't fail.
                        push_char(
                            result,
                            char::from_u32(codepoint as u32)
                                .expect("non-surrogate u16 is always a valid char"),
                        );
                    }
                }
                _ => return Err(JsonError::InvalidEscape),
            }
            i += 1;
        } else {
            // Regular UTF-8 byte - copy until next backslash or end
            let start = i;
            while i < bytes.len() && bytes[i] != b'\\' {
                i += 1;
            }
            let chunk = &bytes[start..i];
            if VALIDATE_UTF8 && core::str::from_utf8(chunk).is_err() {
                return Err(JsonError::InvalidUtf8);
            }
            result.extend_from_slice(chunk);
        }
    }

    Ok(())
}

/// Parse 4 hex digits into a u16.
fn parse_hex4(hex: &[u8]) -> Result<u16, JsonError> {
    if hex.len() != 4 {
        return Err(JsonError::InvalidUnicodeEscape);
    }

    let mut value = 0u16;
    for &b in hex {
        let digit = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => return Err(JsonError::InvalidUnicodeEscape),
        };
        value = value * 16 + digit as u16;
    }
    Ok(value)
}

// ============================================================================
// JsonNumber: Lazy number parsing
// ============================================================================

/// Find the end of a JSON number literal starting at `start` in `text`.
///
/// `start` must point at a byte that begins a candidate number: `-`, an
/// ASCII digit, or -- real jq's own number reader is lenient beyond
/// strict JSON here -- a `.` immediately followed by a digit, or a `+`
/// (#2877). Returns `None` if the bytes at `start` don't actually form a
/// valid number token (`-e5`, `1e`, a bare `.`, `+-1`, ...). decNumber's
/// special words (`nan`, `-Infinity`) are not this function's: see
/// [`jq_number_token_end`], which tries this grammar first and then those.
///
/// Grammar: optional `-` or `+`; an integer part (0+ digits) and/or a
/// `.`-prefixed fractional part (`.` + 1+ digits) -- at least one of
/// the two must supply a digit, so a bare `.` alone is rejected, but a
/// leading-dot number (`.5`) is accepted; an optional `.`-fraction with
/// *zero* digits after it is also accepted when the integer part
/// already supplied one (`1.` -> `1`, matching real jq); an optional
/// exponent (`e`/`E`, optional sign, then 1+ digits) -- if the marker
/// is present but has no digit, the *whole* token is rejected, not
/// truncated before it, matching real jq rejecting `1e` outright rather
/// than accepting `1`. All confirmed live against jq 1.7.1.
///
/// **Only used for top-level document splitting** (the CLI's own
/// `find_json_values`, `src/bin/succinctly/jq_runner.rs`) -- a
/// malformed *top-level* input must error (#1171), matching real jq's
/// own behavior. Do **not** reuse this for a number reached while
/// materializing an already-recognized container's *nested* value: that
/// path has its own, deliberately more permissive established
/// precedent (`nested_number_span`, #966) of absorbing a malformed
/// trailing shape into one span and letting it fail to `Null` downstream
/// rather than erroring the whole document -- this stricter function
/// would instead truncate a span like `1.2.3` after `1.2`, silently
/// materializing the wrong, fabricated value `1.2` instead of `null`
/// (caught by review of #1171 before merge, 4 `#966` regression tests
/// failed).
///
/// One of (at least) three independent "find a JSON-ish number token's
/// boundaries" functions in the crate, each with a genuinely different
/// strictness grammar for its own caller (#1218) -- besides
/// `nested_number_span` above, see
/// [`crate::json::simple_light`]'s private `find_number_end` (backs the
/// separate `SimpleJsonIndex`, fully greedy). #1218's survey counted a
/// fourth, `jq_runner.rs`'s own `find_number_end`, which backed
/// `--argjson`'s leading-zero repair pass; #2052 deleted that pass and the
/// scanner with it. None delegate to any other; see #1218
/// for the full survey and why a blanket consolidation needs its own
/// design pass.
///
/// #2608 later added a fifth number-adjacent caller,
/// `scan_canonical_number` (this file) -- unlike the four above, it is
/// *not* independent: its span-finding loop has no grammar difference
/// from `nested_number_span`'s own greedy `[0-9.eE+-]*` run (same
/// permissive, malformed-shape-absorbing contract this caller also
/// needs, since a not-canonical span still has to resolve to *one* span
/// for [`is_jq_canonical_number`] to reject), so it calls that function
/// directly rather than adding a fifth copy of the loop (#2919 review).
pub fn number_literal_end(text: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    // A leading `+` (#2877) is peeled exactly like `-`: `+X` is `X` in
    // jq, so the grammar below decides the rest either way, and a `+`
    // followed by nothing it accepts (`+`, `+-1`, `+x`) is rejected the
    // same way `-` alone is.
    if i < text.len() && (text[i] == b'-' || text[i] == b'+') {
        i += 1;
    }
    let int_start = i;
    while i < text.len() && text[i].is_ascii_digit() {
        i += 1;
    }
    let has_int_digit = i > int_start;
    let mut has_frac_digit = false;
    if i < text.len() && text[i] == b'.' {
        let frac_start = i + 1;
        let mut j = frac_start;
        while j < text.len() && text[j].is_ascii_digit() {
            j += 1;
        }
        has_frac_digit = j > frac_start;
        i = if has_frac_digit { j } else { i + 1 };
    }
    if !has_int_digit && !has_frac_digit {
        return None;
    }
    if i < text.len() && (text[i] == b'e' || text[i] == b'E') {
        let mut j = i + 1;
        if j < text.len() && (text[j] == b'+' || text[j] == b'-') {
            j += 1;
        }
        let exp_digit_start = j;
        while j < text.len() && text[j].is_ascii_digit() {
            j += 1;
        }
        if j > exp_digit_start {
            i = j;
        } else {
            return None;
        }
    }
    Some(i)
}

/// Where the JSON string literal starting at `start` ends.
///
/// `start` must index the opening `"`; the returned index is one past the
/// closing quote. `None` means the string is unterminated **or contains a
/// raw, unescaped control character** (`U+0000`-`U+001F`).
///
/// Rejecting the control character here is what makes the CLI's document
/// splitter match real jq, which refuses `["a<TAB>b"]` on every input path
/// with `Invalid string: control characters from U+0000 through U+001F must
/// be escaped` (jq 1.7.1). `0x7F` (DEL) is deliberately *accepted*: jq's own
/// check compares a signed char, so only `0x00`-`0x1F` fail it -- a full
/// `0x00`-`0x7F` sweep against jq 1.7.1 confirms exactly that split, with no
/// per-byte exceptions to model (#2878).
///
/// Companion to [`number_literal_end`], and used by the same **top-level
/// document splitting** caller (`find_json_values` /
/// `scan_one_json_token` / `find_matching_close`, in
/// `src/bin/succinctly/jq_runner.rs`), for the same reason: one validated
/// implementation instead of independently-maintained copies. It lives in
/// the library rather than beside its caller because `crate::util` is
/// `pub(crate)`, so the SIMD scanner below is not reachable from the binary.
///
/// This does **not** make the splitter a validator. "Ends" stays structural
/// everywhere else -- `{"a":1 xyz}` still scans as one token and `{invalid}`
/// is still accepted document-wide (a deliberate, cost-driven divergence
/// recorded in `docs/compliance/jq/limitations.md`). The control-character
/// rule is affordable precisely because it rides a scan that already
/// inspects every byte of every string, so it costs no extra pass.
///
/// The scan delegates to [`crate::util::simd::escape::find_json_escape`],
/// whose predicate (`"`, `\`, or `< 0x20`) is already exactly this
/// function's three-way question, at 16-32 bytes per iteration instead of
/// the byte-at-a-time loop this replaced. The scanner sees the *rest of the
/// document*, not the string, so its length-vs-scalar threshold never fires
/// here and one SIMD chunk resolves every string shorter than the chunk: the
/// cost per string is flat, and dominated by the chunk's fixed cost rather
/// than by the bytes scanned. On x86_64 that reads as slightly *more*
/// instructions than the scalar loop on short-string documents (+2.2% on
/// the perf guard's `users_keys_unsorted` row) while running faster in
/// wall-clock time on every shape measured, so it is left alone there. On
/// aarch64 the same flat cost is a latency chain (NEON compare, movemask,
/// vector-to-GPR transfer) that a short scalar loop beats, so the first
/// `STRING_SCALAR_PREFIX` bytes are probed a word at a time first -- see
/// `docs/optimizations/simd.md` § "Instruction counts vs wall-clock" (#2963)
/// for both measurements before moving that line.
///
/// `#[inline(always)]` is load-bearing, as it is on `find_json_escape`
/// (O3): the probe and the chunk path each materialise a set of constants,
/// and out of line they are rebuilt on every call -- one call per string
/// -- on top of a prologue and epilogue. Fat LTO stopped inlining this
/// function on its own once the probe was added, and the ARM64 perf guard
/// read the difference as +6.2% on `users_keys_unsorted`; inlined into the
/// splitter's loops the constants hoist and the per-string cost is the
/// probe itself. On x86_64 the same inlining reads -2.7% instructions on
/// that row and times neutral on a 7950X.
#[inline(always)]
pub fn string_literal_end(bytes: &[u8], start: usize) -> Option<usize> {
    string_literal_end_probed::<STRING_SCALAR_PREFIX>(bytes, start)
}

/// How many bytes [`string_literal_end`] probes with 64-bit word arithmetic
/// before handing the rest of the document to the SIMD scanner: a multiple
/// of 8, or zero to disable the probe.
#[cfg(target_arch = "aarch64")]
pub(crate) const STRING_SCALAR_PREFIX: usize = 8;
#[cfg(not(target_arch = "aarch64"))]
pub(crate) const STRING_SCALAR_PREFIX: usize = 0;

/// [`string_literal_end`] with the probe length as a parameter, so
/// both the probed and the unprobed path are testable on every architecture.
#[inline(always)]
pub(crate) fn string_literal_end_probed<const PREFIX: usize>(
    bytes: &[u8],
    start: usize,
) -> Option<usize> {
    debug_assert_eq!(bytes.get(start), Some(&b'"'));
    let mut i = start + 1;
    loop {
        // `find_json_escape` returns `bytes.len()` when it finds nothing and
        // when `i` is already past the end, so the `\` -at-EOF case below
        // lands here as "unterminated" without a separate bounds check.
        let hit = next_string_special::<PREFIX>(bytes, i);
        match bytes.get(hit) {
            Some(b'"') => return Some(hit + 1),
            // Skip the escaped byte. A `\` as the final byte pushes `i` past
            // the end, which the fast-exit above turns into `None`.
            Some(b'\\') => i = hit + 2,
            // A raw control character: the whole string is rejected.
            Some(_) => return None,
            // Ran off the end without a closing quote.
            None => return None,
        }
    }
}

/// The index of the first `"`, `\` or control byte at or after `i`, or
/// `bytes.len()`: `find_json_escape`, behind a word-at-a-time probe of the
/// first `PREFIX` bytes.
///
/// The probe is O3's own finding (#87: "scalar is faster for tails shorter
/// than 16 bytes") applied where the scanner's threshold cannot express it.
/// That threshold compares against the *buffer* remainder, and here the
/// buffer is the whole document, so from inside a string it is never short;
/// the string is. Resolving the first few bytes in general registers settles
/// most strings of a small-record document (84% of the perf guard's `users`
/// shape are 8 bytes or shorter) without a SIMD chunk whose result must
/// travel back from a vector register before the next string can start. A
/// string longer than the probe pays the probe and then the chunk from
/// `i + PREFIX` on, which skips the bytes already checked.
///
/// The probe is one 64-bit load per 8 bytes and a handful of ALU ops
/// ([`word_special_mask`]), not a byte loop: a byte loop over the same
/// window read +8.8% on the perf guard's ARM64 instruction count for the
/// wall-clock win, and the guard cannot see the latter (#2963).
///
/// Fewer than 8 bytes left at `i` (including `i` past the end, a `\` as the
/// last byte) skips the probe; the scanner's own tail handling and
/// past-the-end fast exit cover it.
#[inline(always)]
fn next_string_special<const PREFIX: usize>(bytes: &[u8], i: usize) -> usize {
    debug_assert!(PREFIX % 8 == 0, "the probe is measured in 64-bit words");
    if PREFIX > 0 {
        let mut at = i;
        while at + 8 <= bytes.len() && at < i + PREFIX {
            // `at + 8 <= len` was just checked, so the slice is in bounds.
            let word = u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap_or([0; 8]));
            let mask = word_special_mask(word);
            if mask != 0 {
                // The lowest set bit is exact (see `word_special_mask`); it
                // is the high bit of the first special byte.
                return at + (mask.trailing_zeros() / 8) as usize;
            }
            at += 8;
        }
        return crate::util::simd::escape::find_json_escape(bytes, at);
    }
    crate::util::simd::escape::find_json_escape(bytes, i)
}

/// A mask whose *lowest* set bit is the high bit of the first byte of
/// `word` (little-endian byte order) that is `"`, `\` or below `0x20`; zero
/// when no byte is. Bits above that first hit may be spurious, so callers
/// use `mask.trailing_zeros() / 8` and nothing else.
///
/// The three terms are the classic `haszero` / `hasless` word tricks
/// (`(x - 0x01..01) & !x & 0x80..80`): each term's borrow chain starts only
/// at a byte that genuinely satisfies its test, so a false positive can only
/// sit *above* a true hit of that same term -- and therefore above the
/// lowest true hit of the OR. The `!x` factor keeps a byte at or above
/// `0x80` (every UTF-8 continuation and lead byte) from testing positive
/// without a borrow-in.
#[inline(always)]
fn word_special_mask(word: u64) -> u64 {
    const ONES: u64 = 0x0101_0101_0101_0101;
    const HIGHS: u64 = 0x8080_8080_8080_8080;
    let quote = word ^ (ONES * b'"' as u64);
    let backslash = word ^ (ONES * b'\\' as u64);
    let below_space = word.wrapping_sub(ONES * 0x20) & !word;
    (quote.wrapping_sub(ONES) & !quote | backslash.wrapping_sub(ONES) & !backslash | below_space)
        & HIGHS
}

/// Find the end of a number-*shaped* span starting at `start` in `text`
/// (a byte that begins a candidate number: `-`, an ASCII digit, or a
/// leading `.`), for a value reached while materializing an
/// already-recognized container's nested field/element (`value()`,
/// `text_range()`, [`JsonNumber::find_end`] below).
///
/// Deliberately permissive, unlike [`number_literal_end`]: greedily
/// consumes every subsequent `[0-9.eE+-]` byte with no grammar
/// validation at all, so a malformed trailing shape (`1.2.3`, an
/// exponent marker with no digit, ...) still resolves to *one*
/// recognized span instead of either fabricating a shorter,
/// wrong-but-valid-looking number or splitting into two adjacent
/// tokens with no separator between them. The one addition since #2877
/// is a *word* arm, taken only when that greedy loop consumed nothing
/// after the optional sign and only when the whole word is one of
/// decNumber's special values ([`special_number_end`]) -- so `nan`,
/// `-inf` and `+Infinity` are one span each, while `-nope` still yields
/// exactly the span it always did. `is_valid_number`/
/// `OwnedValue::from_number_bytes` are what decide, from that whole
/// span, whether it's safe to treat as a real number (falling back to
/// `Null` if not) -- matching this crate's own established, tested
/// precedent for a malformed *nested* number (#966: `{"a": 1.2.3}` ->
/// `{"a": null}`, not a document-wide error). Also what makes
/// [`OwnedValue::to_json_for_reindex`](crate::jq::OwnedValue::to_json_for_reindex)'s
/// `NAN_SENTINEL`/`INFINITY_SENTINEL` round-trip tokens (`9e999e999`,
/// deliberately unparseable via a repeated exponent marker, #472/#1083)
/// round-trip correctly: their whole span must survive intact for
/// `is_nan_sentinel`/`is_infinity_sentinel`'s exact-text comparison to
/// recognize them.
///
/// See [`number_literal_end`]'s own doc comment for the full four-way
/// survey of this crate's independent number-token scanners (#1218) --
/// this one's greedy character class (`[0-9.eE+-]`, no grammar
/// validation) is closest in spirit to `simple_light`'s own scanner, but
/// they still don't share an implementation.
fn nested_number_span(text: &[u8], start: usize) -> usize {
    let mut i = start;
    // `+` as well as `-` (#2877). `+` was already in the greedy class
    // below, so a `+digit` token's span is unchanged by this; skipping it
    // here is what lets the empty-body test that follows see `+nan` and
    // `-inf` as "a sign and then nothing numeric".
    if i < text.len() && (text[i] == b'-' || text[i] == b'+') {
        i += 1;
    }
    let body_start = i;
    while i < text.len() {
        match text[i] {
            b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-' => i += 1,
            _ => break,
        }
    }
    // Nothing numeric after the (optional) sign: a decNumber special word
    // (`nan`, `sNaN12`, `-Infinity`, `+inf`, #2877)? Only then, and only if
    // the whole word validates -- otherwise this returns exactly the span
    // it always did (`-` for `-nope`, `+` for `+x`), so the #1643
    // delimiter-gap check keeps rejecting those the way it does today.
    if i == body_start {
        if let Some(end) = special_number_end(text, start) {
            return end;
        }
    }
    i
}

/// Where the token starting at `start` ends, if it is one of decNumber's
/// special values as jq 1.7.1 reads them (`nan`, `NaN5`, `sNaN`, `inf`,
/// `Infinity`, with an optional `+`/`-`, case-insensitive --
/// [`crate::json::validate::jq_special_number`], #2877); `None` otherwise.
///
/// The candidate token ends where jq's own literal scanner stops
/// ([`jq_literal_run_end`]), and the *whole* run has to validate. That is
/// deliberately stricter than "the longest prefix that validates":
/// `nan1.5`, `nan-1`, `nan(1)`, `nane5` and `infinity1` are each one
/// `Invalid numeric literal` to jq, and reading a valid prefix out of them
/// (`nan1`, `nan`, `nan`, `nan`, `infinity`) would materialize a number
/// where jq errors -- with the leftover bytes caught only by the #1643 gap
/// check, which the materializing routes do not run between siblings
/// (`[nan(1)] | map(.)` would print `[null,1]`). Returning `None` makes the
/// dispatchers' error arm own them instead.
fn special_number_end(text: &[u8], start: usize) -> Option<usize> {
    let end = jq_literal_run_end(text, start);
    crate::json::validate::jq_special_number(&text[start..end]).map(|_| end)
}

/// Where the literal token starting at `start` ends under jq 1.7.1's own
/// scanner: `jv_parse.c`'s `scan` accumulates every byte that is not
/// whitespace, `"` or one of `[{,:]}` into one token and hands the whole
/// thing to `check_literal`. So `nan 1` is two tokens, while `nan1.5`,
/// `nan(1)` and `nanx` are each one token that then fails to validate.
fn jq_literal_run_end(text: &[u8], start: usize) -> usize {
    let mut end = start;
    while end < text.len()
        && !text[end].is_ascii_whitespace()
        && !matches!(text[end], b'"' | b'[' | b'{' | b',' | b':' | b']' | b'}')
    {
        end += 1;
    }
    end
}

/// Where the number token starting at `start` ends, under jq 1.7.1's own
/// reader, for the **top-level document splitter** (#2877).
///
/// The caller is `scan_one_json_token` (`src/bin/succinctly/jq_runner.rs`,
/// and through it `--seq`). The answer is either [`number_literal_end`]'s
/// decimal grammar, or -- when that finds no number -- one of decNumber's
/// special values. `None` when the bytes at `start` are neither, so a
/// malformed top-level token still errors (#1171) rather than being
/// truncated or absorbed.
///
/// A word token ends where jq's own literal scanner stops
/// (`jq_literal_run_end`, private): `nan 1` is two tokens (`null`, `1`, as
/// jq prints them) while `nan1.5`, `nan(1)` and `nanx` are each one token
/// that fails to validate -- the same whole-token rule `special_number_end`
/// (private) applies inside a container, and this is that function.
///
/// `start` must point at a byte that can begin either grammar: `-`, `+`,
/// `.`, an ASCII digit, or one of `n`/`N`/`i`/`I`/`s`/`S`; the caller
/// decides that `nu…` is `null`'s and not this function's, mirroring jq's
/// `check_literal`, which sends only `t…`/`f…`/`nu…` down its keyword path
/// and everything else -- a bare `n` included -- to its number parser.
#[must_use]
pub fn jq_number_token_end(text: &[u8], start: usize) -> Option<usize> {
    if let Some(end) = number_literal_end(text, start) {
        return Some(end);
    }
    special_number_end(text, start)
}

/// A JSON number that hasn't been parsed yet.
///
/// Call `as_i64()` or `as_f64()` to parse the number.
#[derive(Clone, Copy, Debug)]
pub struct JsonNumber<'a> {
    text: &'a [u8],
    start: usize,
}

impl<'a> JsonNumber<'a> {
    /// The byte offset of the number's first character in the document
    /// text. Mirrors [`JsonString::start`] for the same reuse purpose
    /// (#1643, #1677).
    #[inline]
    pub fn start(&self) -> usize {
        self.start
    }

    /// Get the raw bytes of the number.
    pub fn raw_bytes(&self) -> &'a [u8] {
        let end = self.find_end();
        &self.text[self.start..end]
    }

    /// Parse as i64.
    pub fn as_i64(&self) -> Result<i64, JsonError> {
        let bytes = self.raw_bytes();
        let s = core::str::from_utf8(bytes).map_err(|_| JsonError::InvalidUtf8)?;
        s.parse().map_err(|_| JsonError::InvalidNumber)
    }

    /// Parse as f64.
    ///
    /// Also decodes every spelling the ordinary parse refuses but this
    /// crate's one raw-bytes decoder,
    /// [`OwnedValue::from_number_bytes`](crate::jq::OwnedValue::from_number_bytes),
    /// reads as a number: the reindex bridge's computed-float token
    /// (`crate::json::validate::computed_float_token`, #2902) and its NaN/
    /// infinity tokens (#472/#1083 -- decoded here since #2877, when the
    /// `--slurp`/`--seq` input path started writing them), and decNumber's
    /// special values (#2877: Rust's `f64: FromStr` reads `nan`, `inf`,
    /// `infinity` and a leading `+` case-insensitively, but rejects a
    /// signalling NaN `sNaN` and a NaN payload `nan12`, both of which jq
    /// 1.7.1 accepts). Every cursor-level reader of a document number in
    /// both evaluators (`length`, the math builtins, dates, `isnan`, the
    /// generic materializer's `as_f64` arm, ...) reaches its value through
    /// this one accessor, so the decode lives here rather than being
    /// repeated at each of them -- and delegating to `from_number_bytes`
    /// rather than restating its arms keeps the two from drifting. Only
    /// consulted once the ordinary parse has already failed, so a genuine
    /// number pays nothing for it.
    ///
    /// The value is the **plain** correctly-rounded parse, the JSON
    /// library's own answer. jq mode's 17-digit literal model (#2936) is
    /// applied by the evaluators' number funnels
    /// (`OwnedValue::from_number_bytes::<S>`, `document_number_f64::<S>`),
    /// not here: this type has no mode, and every spelling below the
    /// fallback is a non-literal (a bridge token or a decNumber word) whose
    /// value the model cannot change.
    pub fn as_f64(&self) -> Result<f64, JsonError> {
        let bytes = self.raw_bytes();
        let s = core::str::from_utf8(bytes).map_err(|_| JsonError::InvalidUtf8)?;
        s.parse().or_else(|_| {
            crate::jq::OwnedValue::bridge_nonfinite_from_bytes(bytes)
                .or_else(|| crate::json::validate::parse_computed_float_token(bytes))
                .or_else(|| crate::json::validate::jq_special_number(bytes))
                .ok_or(JsonError::InvalidNumber)
        })
    }

    fn find_end(&self) -> usize {
        nested_number_span(self.text, self.start)
    }
}

// ============================================================================
// Error type
// ============================================================================

/// Errors that can occur during JSON value extraction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JsonError {
    /// Invalid UTF-8 in string
    InvalidUtf8,
    /// Invalid number format
    InvalidNumber,
    /// Invalid escape sequence in string
    InvalidEscape,
    /// Invalid unicode escape (not a valid hex digit or invalid codepoint)
    InvalidUnicodeEscape,
}

impl JsonError {
    /// The human-readable reason, as a `&'static str`.
    ///
    /// Split out of [`Display`](core::fmt::Display) (which now defers to it)
    /// so a caller that needs the text without allocating -- notably
    /// [`DocumentValue::string_decode_error`],
    /// which runs on a `no_std`-compatible path -- shares one definition with
    /// the formatter rather than restating the four strings next to it.
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::InvalidUtf8 => "invalid UTF-8 in string",
            Self::InvalidNumber => "invalid number format",
            Self::InvalidEscape => "invalid escape sequence in string",
            Self::InvalidUnicodeEscape => "invalid unicode escape sequence",
        }
    }
}

impl core::fmt::Display for JsonError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.message())
    }
}

// ============================================================================
// Type aliases for common configurations
// ============================================================================

/// JSON index with owned storage.
pub type OwnedJsonIndex = JsonIndex<Vec<u64>>;

/// JSON index with borrowed storage (e.g., from mmap).
pub type BorrowedJsonIndex<'a> = JsonIndex<&'a [u64]>;

/// JSON cursor with owned index.
pub type OwnedJsonCursor<'a> = JsonCursor<'a, Vec<u64>>;

/// JSON cursor with borrowed index.
pub type BorrowedJsonCursor<'a> = JsonCursor<'a, &'a [u64]>;

// ============================================================================
// Document trait implementations
// ============================================================================

use crate::jq::document::{
    collapsed_fields_checked, document_token_of, effective_fields_checked, key_hash,
    key_is_malformed, trailing_element_gap_ok, DocumentCursor, DocumentElements, DocumentField,
    DocumentFields, DocumentValue, IndentSpec, JsonConvention, KeyHashes,
};
use crate::jq::escape::{write_json_body_jq, write_json_body_yq};
use crate::jq::stream::{StreamFailure, StreamResult};
use crate::jq::{
    format_number_jq_compat, is_jq_canonical_number, nesting_depth_exceeded_message,
    nonfinite_display_string, EvalError, JqSemantics, OwnedValue, YqSemantics,
    MAX_VALUE_TREE_DEPTH,
};
use crate::text::utf8::decode_code_point;

/// A [`JsonError`] as the uncatchable decode failure (#1620) every
/// *materializing* route already raises for the same scalar, so a document
/// with a bad escape gets one answer whether it is streamed or materialized
/// (#1615). The YAML-side twin is `yaml::light::decode_failure`.
fn json_decode_failure(e: JsonError) -> StreamFailure {
    StreamFailure::Decode(EvalError::decode_failure(e.message()))
}

/// Whether the child starting at `child_start` is preceded by `expected`
/// (`,`/`:`), skipping whitespace, with nothing else in between.
///
/// Originally the CLI-only heart of #1643's `print_json` check
/// (`src/bin/succinctly/jq_runner.rs`); relocated here for #1677 so
/// [`DocumentCursor::preceding_delimiter_ok`] (below) and the CLI printer
/// share one definition instead of drifting into two.
///
/// Deliberately narrow, matching real jq's own leniency elsewhere in the
/// same bytes: this only inspects gap bytes between already-recognized
/// children, never a child's own content, so it can't regress this
/// crate's own established leniencies beyond strict RFC 8259 -- leading
/// zeros (#1149), a leading-dot number (#1171), a malformed nested number
/// like `1.2.3` (#1194/#966) -- none of which live in a gap.
///
/// Only catches a missing or doubled delimiter between two real children.
/// A trailing delimiter (`{"a":1,}`) or a delimiter in an apparently-empty
/// container (`{,}`) needs the *closing* bracket's text position, which is
/// exactly the expensive lookup this function exists to avoid -- tracked
/// as a follow-up rather than folded in here.
///
/// `pub`, not `pub(crate)`: the CLI binary (`src/bin/succinctly/`) is a
/// separate crate that only sees this library's public surface, and it is
/// the other caller of this exact check (`jq_runner.rs`'s `print_json`).
/// Not re-exported from the crate root -- an internal detail for this
/// crate's own two evaluators, not part of the supported library API.
pub fn preceding_gap_ok(text: &[u8], child_start: usize, expected: Option<u8>) -> bool {
    let mut i = child_start;
    let mut found = None;
    while i > 0 {
        match text[i - 1] {
            b @ (b',' | b':') => {
                if found.is_some() {
                    return false; // doubled delimiter
                }
                found = Some(b);
                i -= 1;
            }
            b if b.is_ascii_whitespace() => i -= 1,
            _ => break, // reached the previous sibling's own content
        }
    }
    found == expected
}

/// The forward-scan counterpart of [`preceding_gap_ok`]: whether exactly
/// one `:` separates `key_end` (a key's own already-known span end) from
/// the next non-whitespace byte.
///
/// Exists so a caller holding only a key (not its value's cursor) can check
/// the delimiter between them by scanning forward from a position it
/// already has -- `JsonString::end()` -- bounded by the gap itself, rather
/// than resolving the value's `text_position()` (a rank/select lookup)
/// purely to run `preceding_gap_ok` backward from there instead: measured
/// live, that naive version cost **+16%** on a 2 MB `wide` `keys_unsorted`
/// query (#1677) -- the exact per-field cost #1514 already measured for
/// `uncons` vs `uncons_key` in general, reintroduced here for one specific
/// lookup instead of a whole `DocumentField`.
///
/// This forward-scan version brought `keys`/`keys_unsorted` back to
/// noise-level (~1-4%), but `census`'s own key-only walk (`length`,
/// `keys | length`) still measured **~10%** on the same 2 MB `wide`
/// fixture (159K short top-level keys) -- `census` has nothing else to
/// dilute the cost against, unlike `keys_unsorted`, which also streams
/// output. Accepted deliberately rather than dropped: it is the
/// correctness fix for this issue's own headline repro
/// (`{"a" 1, "b": 2} | length`), and the alternative is silently wrong
/// output. `scripts/perf-guard.py`'s baseline needs a deliberate
/// `--update-baseline` run on a pinned bench box to reflect this.
///
/// Recognises both delimiter bytes, exactly as [`preceding_gap_ok`] does,
/// not only `:` -- #2720 review: scanning for `:` alone `break`s on the `,`
/// of a `:,` sequence with `found` already set, so `{"a":,1}` read as one
/// well-formed field here where the backward scan (`preceding_gap_ok`, the
/// check `effective_fields_checked` runs) refused it; `length` answered `1`
/// for it, and once the identity writer validated through this scan too
/// (#2720) it printed `{"a": 1}` where jq and the materializing path exit 5.
pub fn following_gap_ok(text: &[u8], key_end: usize) -> bool {
    let mut i = key_end;
    let mut found = None;
    while i < text.len() {
        match text[i] {
            b @ (b',' | b':') => {
                if found.is_some() {
                    return false; // doubled delimiter
                }
                found = Some(b);
                i += 1;
            }
            b if b.is_ascii_whitespace() => i += 1,
            _ => break,
        }
    }
    found == Some(b':')
}

/// Whether nothing but whitespace separates `gap_start` (a container's last
/// scalar child's own already-known span end) from `close_char` (`]`/`}`) --
/// #1576/#1676, mirroring `src/bin/succinctly/jq_runner.rs`'s own
/// `trailing_gap_ok`/`validate_json_delimiters`. Catches a trailing `,`
/// (`[1,2,]`, `{"a":1,}`) that [`stream_json_pretty`]'s own leading-delimiter
/// check (run *before* each child) can't: there is no next child to run it
/// against when the stray comma is the container's very last token.
fn trailing_gap_ok(text: &[u8], gap_start: usize, close_char: u8) -> bool {
    let mut i = gap_start;
    while i < text.len() {
        match text[i] {
            b if b.is_ascii_whitespace() => i += 1,
            b if b == close_char => return true,
            _ => return false,
        }
    }
    false
}

/// Whether an *apparently* empty container (`value` decodes to `[]`/`{}`)
/// at `cursor` is genuinely empty rather than a stray `,` with no real
/// child (`{,}`, `[,]`) -- #1576 review: `JsonCursor::stream_json`'s own
/// pre-recursion check only ever ran on the *root* value, because that was
/// the only place in the call chain that still held the container's own
/// cursor. `stream_json_pretty`'s array/object arms hit the identical gap
/// one level down (`JsonFields`/`JsonElements` carry only "the first
/// child's cursor, or `None`", nothing for the empty case to compare
/// against), so both now call this with whichever cursor they still have
/// for the child about to be checked, rather than only checking the root.
/// Returns `true` (nothing to flag) whenever `value` isn't an empty
/// container, or when `cursor` lacks a text position/raw span to check
/// (a synthetic value with no source span never has a stray token either).
fn empty_container_gap_ok<'a, W: AsRef<[u64]> + Clone>(
    cursor: &JsonCursor<'a, W>,
    value: &StandardJson<'a, W>,
) -> bool {
    let is_empty_container = matches!(value, StandardJson::Object(fields) if fields.is_empty())
        || matches!(value, StandardJson::Array(elements) if elements.is_empty());
    if !is_empty_container {
        return true;
    }
    let (Some(pos), Some(bytes)) = (cursor.text_position(), cursor.raw_bytes()) else {
        return true;
    };
    let close = bytes[bytes.len() - 1];
    trailing_gap_ok(cursor.text(), pos + 1, close)
}

/// Cheap end position (one past the last byte) of an already-resolved
/// scalar `value` known to start at `start` -- `None` for a container,
/// which [`trailing_gap_ok`]'s callers treat as "can't determine, skip"
/// (matching `src/bin/succinctly/jq_runner.rs`'s own `scalar_end_pos`: a
/// container's own last child might have arbitrary trailing whitespace
/// before its closing bracket, so this is deliberately deferred rather than
/// mistracked).
fn scalar_end_pos<W: AsRef<[u64]> + Clone>(
    start: usize,
    value: &StandardJson<'_, W>,
) -> Option<usize> {
    match value {
        StandardJson::String(s) => Some(start + s.raw_bytes().len()),
        StandardJson::Number(n) => Some(start + n.raw_bytes().len()),
        StandardJson::Bool(true) => Some(start + 4),
        StandardJson::Bool(false) => Some(start + 5),
        StandardJson::Null => Some(start + 4),
        StandardJson::Array(_) | StandardJson::Object(_) | StandardJson::Error(_) => None,
    }
}

/// Scans the *first* JSON value in `bytes` and returns the position just
/// past it iff that value's span is *exactly* the byte-for-byte compact,
/// unsorted-keys, jq-number-convention JSON that `stream_json_pretty`
/// would itself produce for the value those bytes encode -- i.e. safe to
/// echo verbatim instead of re-rendering (#2608). Whatever follows that
/// value is neither scanned nor echoed; see "Why the echoed span is
/// exactly the cursor's own value" at the end of this comment.
///
/// A strict, single-pass, non-backtracking recursive-descent scanner over
/// `value := object | array | string | number | true | false | null`, with
/// **zero whitespace tolerated anywhere** (compact mode inserts none). It
/// doubles as the structural validation `stream_json_pretty`'s walk
/// otherwise supplies -- a `[1,,2]`/`{,}`/trailing-comma/missing-colon
/// malformation has no legal token at the position the grammar above
/// expects one, so the scan simply fails closed (`None` bubbles to
/// `false`) the same as a genuine non-canonical span would. Bailing is
/// always *safe*, never a correctness risk: a `false` here only costs the
/// caller the echo and sends it back to the unchanged, already-correct
/// `stream_json_pretty` re-render.
///
/// Three sub-checks, each pinned to the writer it must agree with bit for
/// bit -- a divergence in any one is silent wrong output, not a crash, so
/// none of them may drift from its writer (the #106 "duplicated predicates
/// diverge silently" hazard, `CLAUDE.md`):
/// - Numbers: the maximal `[-+.eE0-9]*` run at the current position (safe
///   because a compact number is always immediately followed by
///   `,`/`}`/`]`/end-of-input -- no whitespace, no other adjacent token
///   char) is handed to [`is_jq_canonical_number`], which already answers
///   "would `format_number_jq_compat` echo these exact bytes" (#2206).
/// - Strings: scanned byte-by-byte between the quotes against exactly
///   [`write_json_body_jq`]'s escape table (see `scan_json_string_span`'s
///   own doc comment) -- **not** [`write_json_body_jq_ascii`]'s. That
///   distinction is what keeps `-a`/`--ascii-output` out of this fast path
///   entirely: this checker treats a raw non-ASCII byte as canonical
///   content, which is only true under the non-ASCII-escaping writer.
///   Nothing here gates on an ascii flag directly, because `stream_json`
///   itself -- and every caller of it -- never has one to gate on:
///   `JsonConvention::JqCompat` always renders through
///   [`write_json_string_pretty`]'s `write_json_body_jq` arm, never the
///   `_ascii` writer (see that function, `src/json/light.rs`). ASCII
///   output is produced by an entirely separate route in
///   `src/bin/succinctly/jq_runner.rs` (`format_json`/`write_output_jq_value`)
///   that materializes an `OwnedValue` and never calls `stream_json` at
///   all when `-a` is set -- confirmed at both of `stream_json`'s two
///   callers in that file: the M2 fast path's own gate excludes
///   `ascii_output` before it ever reaches `stream_json`
///   (`can_json_fast_path`), and the general fallback takes the
///   `format_json` materializing route instead precisely because `-a` is
///   set. So this checker is exactly as `-a`-safe as the unchanged
///   re-render branch directly below it in `stream_json` -- both are keyed
///   on `numbers: JsonConvention` alone, and neither is ever invoked with
///   ascii intent.
/// - Duplicate keys: each key's *raw source span* (the bytes strictly
///   between its quotes, undecoded) is checked against every key already
///   seen in the same object (never shared across siblings or nesting,
///   matching one-instance-per-object semantics). That's sound without a
///   decode step only because the string check above has *already*
///   certified this exact span as jq's own canonical encoding of its
///   decoded content -- a fixed, unconditional rule, so it's an injective
///   map from decoded string to span, and span equality is decoded-string
///   equality. Two tiers, mirroring `src/bin/succinctly/jq_runner.rs`'s
///   own `PAIRWISE_SPAN_SCAN_LIMIT`/`span_fingerprint` split (#2919
///   review): up to `SMALL_OBJECT_KEY_LIMIT` keys are compared pairwise
///   via a cheap fingerprint with no allocation at all -- most real
///   objects never cross that threshold, and [`KeyHashes::insert`]
///   heap-allocates its table on its very first call, which would
///   otherwise cost every tiny object a real allocation for no reason.
///   Past the threshold a real [`KeyHashes`] table takes over, fed each
///   key through [`key_hash`], and checks `KeyHashes::saturated` right
///   after every `insert` -- matching every other `KeyHashes` caller in
///   the tree (`DistinctKeyCursors::next`, `src/jq/document.rs`) -- so it
///   bails the moment the table can no longer grow instead of paying for
///   one more doomed string-scan-and-hash first; `insert` degrades to an
///   unconditional conservative `true` past that point regardless, so
///   this changes nothing about the answer, only the work spent reaching
///   it. Either tier's "seen before" answer covers both a genuine
///   duplicate key and a bare 64-bit hash collision between two distinct
///   keys; both get the same conservative answer here (bail, don't try to
///   disambiguate by comparing bytes) since either one means "cannot
///   certify this object's span as canonical", not "definitely not
///   canonical" -- `KeyHashes`'s own doc comment describes the same
///   conservatism for its other callers.
///
/// The safety argument for this whole function is exactly the equivalence
/// this module's own `is_canonical_compact_jq_span_agrees_with_rerender_2608`
/// and `is_canonical_compact_jq_span_fuzz_2608` tests assert (inline
/// `#[cfg(test)] mod tests`, this file -- #2919 review: an earlier draft of
/// this comment cited `is_canonical_compact_jq_span_agrees_with_stream_json_pretty`
/// in a `tests/json_canonical_echo_tests.rs` that never existed): the
/// load-bearing direction of `is_canonical_compact_jq_span(d) ==
/// (stream_json_pretty(d, compact, unsorted, JqCompat) == Ok(d))` -- `true
/// ==>` round-trips; see those two tests' own doc comments for why the
/// converse isn't asserted -- swept over a hand-picked corpus plus a
/// differential/fuzz corpus that deliberately includes non-ASCII and
/// edge-byte content per `CLAUDE.md`'s fuzz-alphabet rule.
///
/// This is the form `stream_json`'s echo branch uses, and the reason it can
/// skip [`JsonCursor::raw_bytes`] entirely (#2608 follow-up). `raw_bytes`
/// resolves the node's end through [`JsonCursor::text_range`], which for a
/// container is a *linear rescan of the whole container* looking for the
/// matching bracket -- measured at 9.2 Ir per input byte on a 7950X, 28% of
/// the gate's total cost, and entirely wasted work because the scan below
/// finds that same end on its way through. The caller therefore takes only
/// the node's O(1) start ([`JsonCursor::text_position`], a sequential
/// select) and lets this function report the end.
///
/// **Why the echoed span is exactly the cursor's own value**, not a prefix
/// of it and not a byte more:
/// - It starts where the cursor starts, so the two spans share a start.
/// - This scanner parses exactly *one* complete value by the strict JSON
///   grammar and stops at its last byte; it never consumes a following
///   sibling, a second top-level document, the file's trailing newline, or
///   any trailing garbage, because none of those can continue a value that
///   the grammar has already closed (a `}`/`]`/closing `"` ends its token
///   outright, and a number/literal token ends at the first byte outside its
///   own character class).
/// - It can only *accept* a span the writer would echo byte for byte, which
///   in particular means no whitespace anywhere inside it -- so the only way
///   the semi-index's own tokenization could disagree about where this value
///   ends is on a spelling this scanner has already rejected. The semi-index
///   accepts a strict superset of the tokens here (lenient numbers like
///   `1.2.3`, `NaN`/`Infinity` words, unterminated strings, `\`-escapes this
///   scanner declines); on the strict, whitespace-free, canonical subset the
///   two agree, which the `debug_assert_eq!` in `stream_json`'s echo branch
///   checks against [`JsonCursor::text_range`] on every accepted span in
///   every debug build and test run.
#[must_use]
pub(crate) fn canonical_compact_jq_span_end(bytes: &[u8]) -> Option<usize> {
    scan_canonical_value(bytes, 0, 0)
}

/// The whole-buffer form of [`canonical_compact_jq_span_end`]: true iff
/// that scan both accepts and consumes *all* of `bytes`. The accept
/// language the differential tests in this module's own `mod tests` are
/// written against, and the spelling the rest of this file's comments
/// use when they talk about "canonical" -- `stream_json`'s echo branch
/// itself wants the end position, not the predicate, so this exists for
/// the tests alone (#2608 follow-up).
#[cfg(test)]
#[must_use]
pub(crate) fn is_canonical_compact_jq_span(bytes: &[u8]) -> bool {
    canonical_compact_jq_span_end(bytes) == Some(bytes.len())
}

/// Scans one JSON value starting at `bytes[pos]`, returning the position
/// just past it iff that value's span is exactly the canonical compact jq
/// spelling of the value it encodes. `depth` mirrors `stream_json_pretty`'s
/// own recursion counter and is checked against the same
/// [`MAX_VALUE_TREE_DEPTH`] ceiling for the same reason: bounding the
/// native Rust call stack this recursive-descent scan itself uses. Hitting
/// the ceiling bails (`None`), never panics -- a pathologically deep
/// document is exactly the case that must fall through to
/// `stream_json_pretty`'s own real depth-exceeded error, not have this
/// checker's stack overflow instead.
fn scan_canonical_value(bytes: &[u8], pos: usize, depth: usize) -> Option<usize> {
    if depth >= MAX_VALUE_TREE_DEPTH {
        return None;
    }
    let byte = *bytes.get(pos)?;
    match byte {
        b'{' => scan_canonical_object(bytes, pos, depth),
        b'[' => scan_canonical_array(bytes, pos, depth),
        b'"' => scan_json_string_span(bytes, pos).map(|(.., end)| end),
        b't' => scan_canonical_literal(bytes, pos, b"true"),
        b'f' => scan_canonical_literal(bytes, pos, b"false"),
        b'n' => scan_canonical_literal(bytes, pos, b"null"),
        b'-' | b'0'..=b'9' => scan_canonical_number(bytes, pos),
        _ => None,
    }
}

/// Matches one of `true`/`false`/`null` at `pos` exactly -- no other
/// spelling is legal JSON, so there is no "canonical vs. not" axis here the
/// way there is for numbers/strings, only "present or not".
fn scan_canonical_literal(bytes: &[u8], pos: usize, literal: &[u8]) -> Option<usize> {
    let end = pos.checked_add(literal.len())?;
    if bytes.get(pos..end) == Some(literal) {
        Some(end)
    } else {
        None
    }
}

/// The number-token rule from `canonical_compact_jq_span_end`'s own doc
/// comment: the maximal `[-+.eE0-9]*` run is jq's own canonical spelling
/// iff [`is_jq_canonical_number`] says so. The span itself comes from
/// [`nested_number_span`] (#2919 review) rather than a second,
/// independently maintained copy of the same greedy character-class loop
/// -- see `number_literal_end`'s #1218 survey doc comment for why this
/// caller reuses it instead of counting as a fifth independent scanner.
fn scan_canonical_number(bytes: &[u8], pos: usize) -> Option<usize> {
    let end = nested_number_span(bytes, pos);
    if end == pos || !is_jq_canonical_number(&bytes[pos..end]) {
        return None;
    }
    Some(end)
}

/// The string-token rule: `bytes[pos]` must be `"`, and the scan returns
/// `(content_start, content_end, end)` -- `end` is one past the closing
/// quote, `content_start..content_end` the raw span strictly between the
/// quotes -- iff every byte in that content span is exactly what
/// [`write_json_body_jq`] would itself have emitted for the string this
/// span decodes to:
/// - a literal (unescaped) `"` ends the string;
/// - a literal `\` must be followed by one of the 7 short escapes (`"` `\`
///   `b` `f` `n` `r` `t` -- note `/` is deliberately absent: jq's writer
///   never escapes solidus, so a source `\/` is not canonical), or `u`
///   plus exactly 4 *lowercase* hex digits decoding to a control code
///   [`write_json_body_jq`] has no short form for -- `0x00..=0x07`,
///   `0x0B`, `0x0E..=0x1F`, or `0x7F` (DEL). A `\u00XX` for a codepoint
///   that *does* have a short form (`0x08`/`0x09`/`0x0A`/`0x0C`/`0x0D`) is
///   rejected here even though it decodes into the control range, because
///   the writer would have emitted the short form instead -- see
///   `canonical_compact_jq_span_end`'s own doc comment for why the
///   accept-set is narrower than "any control code fits". Anything else
///   after a backslash (uppercase hex, a surrogate half, any printable
///   codepoint, any other letter) is rejected;
/// - a literal byte `< 0x20` or `0x7F` appearing unescaped is rejected --
///   the writer always escapes these;
/// - every other byte, ASCII or a UTF-8 lead/continuation byte `>= 0x80`,
///   is literal content.
///
/// UTF-8 validity is checked *inline*, in the same loop below, rather than
/// as a separate pass over the finished span (#2919 review: an earlier
/// draft called `validate_utf8` on `content_start..content_end` only after
/// the loop found the closing quote -- a second full walk of the string's
/// bytes for the multi-byte-heavy case). That's still sound for exactly
/// the reason the old deferred check was: none of `"`, `\`, or a
/// control/DEL byte can ever appear as a lead or continuation byte of a
/// UTF-8 sequence (valid or not), since every one of those bytes is
/// `< 0x80` or exactly `0x7F`, and UTF-8 multi-byte bytes are always
/// `>= 0x80` -- so every byte the loop below classifies as plain "literal
/// content" is unambiguously either a one-byte ASCII character (`< 0x80`,
/// no check needed) or the first byte of a multi-byte sequence handed to
/// [`decode_code_point`], which decodes and bounds-checks the whole
/// sequence in one call. `decode_code_point` shares its bounds
/// (`code_point_bounds_violation`) with
/// [`validate_utf8_scalar`](crate::text::utf8::validate_utf8_scalar)
/// (#1423), so this can't silently drift from what the separate
/// `validate_utf8` call it replaces would have decided.
fn scan_json_string_span(bytes: &[u8], pos: usize) -> Option<(usize, usize, usize)> {
    debug_assert_eq!(bytes.get(pos), Some(&b'"'));
    let content_start = pos + 1;
    let mut i = content_start;
    loop {
        match *bytes.get(i)? {
            b'"' => {
                return Some((content_start, i, i + 1));
            }
            b'\\' => {
                let esc = *bytes.get(i + 1)?;
                match esc {
                    b'"' | b'\\' | b'b' | b'f' | b'n' | b'r' | b't' => i += 2,
                    b'u' => {
                        let hex = bytes.get(i + 2..i + 6)?;
                        if !hex
                            .iter()
                            .all(|h| h.is_ascii_digit() || matches!(h, b'a'..=b'f'))
                        {
                            return None;
                        }
                        let cp = hex.iter().fold(0u32, |acc, &h| {
                            let digit = if h.is_ascii_digit() {
                                h - b'0'
                            } else {
                                h - b'a' + 10
                            };
                            (acc << 4) | u32::from(digit)
                        });
                        if !matches!(cp, 0x00..=0x07 | 0x0B | 0x0E..=0x1F | 0x7F) {
                            return None;
                        }
                        i += 6;
                    }
                    // `/` (never escaped by jq's writer), any other letter,
                    // or an unrecognized byte after `\` -- all not
                    // canonical.
                    _ => return None,
                }
            }
            b if b < 0x20 || b == 0x7F => return None,
            b if b < 0x80 => i += 1,
            _ => {
                // A UTF-8 lead byte (`>= 0x80`) -- decode and
                // bounds-check the whole sequence at once; an invalid one
                // (truncated, overlong, a surrogate, past Unicode's own
                // range, or a bare continuation byte misread as a lead)
                // bails exactly as the separate `validate_utf8` pass this
                // replaces would have.
                let (_, len) = decode_code_point(&bytes[i..])?;
                i += len;
            }
        }
    }
}

/// The object-token rule: `{`, then either an immediate `}` or
/// `"key":value` pairs separated by exactly one `,` with no trailing
/// comma, each key checked against [`scan_json_string_span`] and hashed
/// (raw span, undecoded -- see `canonical_compact_jq_span_end`'s own doc
/// comment for why that's sound) into a fresh per-object [`KeyHashes`] to
/// bail on any repeat.
/// Above this many keys, [`scan_canonical_object`] switches from an
/// allocation-free pairwise key-span scan to a real [`KeyHashes`] table.
///
/// Mirrors `src/bin/succinctly/jq_runner.rs`'s own
/// `PAIRWISE_SPAN_SCAN_LIMIT` (same value, same reasoning -- real objects
/// are small, and below this a pairwise scan is free while `KeyHashes`
/// would heap-allocate on its very first `insert`). Kept as its own
/// constant rather than shared with that one because the dependency only
/// runs one way -- this `src/json/light.rs` module is part of the library
/// crate, and `jq_runner.rs` is part of the `succinctly` *binary* crate
/// that depends on it, so a lib module cannot reference a `bin` target's
/// private const. If one threshold changes, check whether the other
/// should too.
const SMALL_OBJECT_KEY_LIMIT: usize = 16;

/// A cheap discriminator for a key's raw quoted span (`"`...`"`, quotes
/// included), mirroring `jq_runner.rs`'s own `span_fingerprint`: length
/// plus the first and last content bytes, packed into one word.
///
/// Distinct fingerprints prove distinct keys, so [`scan_canonical_object`]'s
/// pairwise scan below compares these words first and only falls back to
/// comparing the spans themselves on a collision.
#[inline]
fn object_key_span_fingerprint(quoted: &[u8]) -> u64 {
    let n = quoted.len();
    let first = if n > 2 { quoted[1] } else { 0 };
    let last = if n > 3 { quoted[n - 2] } else { 0 };
    ((n as u64) << 16) | ((first as u64) << 8) | last as u64
}

fn scan_canonical_object(bytes: &[u8], pos: usize, depth: usize) -> Option<usize> {
    debug_assert_eq!(bytes.get(pos), Some(&b'{'));
    let mut i = pos + 1;
    if bytes.get(i) == Some(&b'}') {
        return Some(i + 1);
    }
    // Small-object fast path (#2919 review): up to `SMALL_OBJECT_KEY_LIMIT`
    // keys are compared pairwise via a cheap fingerprint, with no
    // allocation at all. `seen_keys` -- a real `KeyHashes` table -- only
    // comes into existence once an object turns out to have more keys
    // than that; see `SMALL_OBJECT_KEY_LIMIT`'s own doc comment.
    let mut small_spans: [(usize, usize); SMALL_OBJECT_KEY_LIMIT] =
        [(0, 0); SMALL_OBJECT_KEY_LIMIT];
    let mut small_fps: [u64; SMALL_OBJECT_KEY_LIMIT] = [0; SMALL_OBJECT_KEY_LIMIT];
    let mut small_count = 0usize;
    let mut seen_keys: Option<KeyHashes> = None;
    loop {
        if bytes.get(i) != Some(&b'"') {
            return None;
        }
        let (key_start, key_end, after_key) = scan_json_string_span(bytes, i)?;
        if let Some(seen) = seen_keys.as_mut() {
            // #2919 review: `saturated()` is checked right after `insert`,
            // matching every other `KeyHashes` caller in the tree
            // (`DistinctKeyCursors::next`, `src/jq/document.rs`), so this
            // bails the moment the table can no longer grow instead of
            // paying for one more doomed string-scan-and-hash first --
            // `insert` degrades to an unconditional conservative `true`
            // past that point regardless, so this changes nothing about
            // the answer, only the work spent reaching it.
            if seen.insert(key_hash(&bytes[key_start..key_end])) || seen.saturated() {
                return None;
            }
        } else {
            let fp = object_key_span_fingerprint(&bytes[i..after_key]);
            let repeat = (0..small_count).any(|j| {
                let (s, e) = small_spans[j];
                small_fps[j] == fp && bytes[s..e] == bytes[key_start..key_end]
            });
            if repeat {
                // A genuine duplicate key, or merely a hash collision --
                // either way this object's span cannot be certified
                // canonical (see this function's own doc comment).
                return None;
            }
            if small_count < SMALL_OBJECT_KEY_LIMIT {
                small_fps[small_count] = fp;
                small_spans[small_count] = (key_start, key_end);
                small_count += 1;
            } else {
                // Overflow past the pairwise limit: build a real table,
                // seeded from what the pairwise scan already collected.
                // None of those `small_count` keys can be a duplicate of
                // each other -- the pairwise scan above already proved
                // that -- so the only way this seeding loop's `insert`
                // reports `true` is a bare 64-bit hash collision between
                // two already-distinct keys, which this whole function
                // already treats as "bail, don't try to disambiguate"
                // (see `KeyHashes`'s own doc comment on `insert`).
                let mut table = KeyHashes::with_capacity(small_count + 1);
                for &(s, e) in &small_spans[..small_count] {
                    if table.insert(key_hash(&bytes[s..e])) {
                        return None;
                    }
                }
                if table.insert(key_hash(&bytes[key_start..key_end])) || table.saturated() {
                    return None;
                }
                seen_keys = Some(table);
            }
        }
        if bytes.get(after_key) != Some(&b':') {
            return None;
        }
        i = scan_canonical_value(bytes, after_key + 1, depth + 1)?;
        match bytes.get(i) {
            Some(b',') => i += 1,
            Some(b'}') => return Some(i + 1),
            _ => return None,
        }
    }
}

/// The array-token rule: `[`, then either an immediate `]` or `value`
/// items separated by exactly one `,` with no trailing comma.
fn scan_canonical_array(bytes: &[u8], pos: usize, depth: usize) -> Option<usize> {
    debug_assert_eq!(bytes.get(pos), Some(&b'['));
    let mut i = pos + 1;
    if bytes.get(i) == Some(&b']') {
        return Some(i + 1);
    }
    loop {
        i = scan_canonical_value(bytes, i, depth + 1)?;
        match bytes.get(i) {
            Some(b',') => i += 1,
            Some(b']') => return Some(i + 1),
            _ => return None,
        }
    }
}

impl<'a, W: AsRef<[u64]> + Clone> DocumentCursor for JsonCursor<'a, W> {
    type Value = StandardJson<'a, W>;

    #[inline]
    fn value(&self) -> Self::Value {
        JsonCursor::value(self)
    }

    #[inline]
    fn first_child(&self) -> Option<Self> {
        JsonCursor::first_child(self)
    }

    #[inline]
    fn next_sibling(&self) -> Option<Self> {
        JsonCursor::next_sibling(self)
    }

    #[inline]
    fn prev_sibling(&self) -> Option<Self> {
        JsonCursor::prev_sibling(self)
    }

    #[inline]
    fn parent(&self) -> Option<Self> {
        JsonCursor::parent(self)
    }

    #[inline]
    fn same_node(&self, other: &Self) -> bool {
        self.bp_pos == other.bp_pos
    }

    #[inline]
    fn is_container(&self) -> bool {
        JsonCursor::is_container(self)
    }

    /// #2072: a JSON cursor is `(text, index, bp_pos)`, and the first two
    /// are the document -- so the BP position alone is the node identity
    /// [`same_node`](DocumentCursor::same_node) just above compares.
    #[inline]
    fn node_id(&self) -> usize {
        self.bp_pos
    }

    /// #2072: `BalancedParens::is_open` answers `false` past the end of the
    /// vector, so one call rejects both an out-of-range id and a closing
    /// position -- the two ways an id can fail to name a node here.
    #[inline]
    fn at_node_id(&self, id: usize) -> Option<Self> {
        self.index
            .bp()
            .is_open(id)
            .then(|| JsonCursor::from_bp_position(self.index, self.text, id))
    }

    /// #2072: `text` is a borrow of the same buffer the index was built
    /// from, so the index alone identifies the document.
    #[inline]
    fn document_token(&self) -> usize {
        document_token_of(self.index, self.index.bp().len())
    }

    #[inline]
    fn text_position(&self) -> Option<usize> {
        JsonCursor::text_position(self)
    }

    /// JSON is the one format whose semi-index treats `:`/`,` as
    /// interchangeable gap bytes (#1643), so it is the one format that
    /// overrides this (#1677).
    #[inline]
    fn preceding_delimiter_ok(&self, text_pos: usize, expected: Option<u8>) -> bool {
        preceding_gap_ok(self.text, text_pos, expected)
    }

    /// Re-runs the strict validator over this cursor's own document to name
    /// the real syntax error, matching [`JsonFields::malformed_member_error`]'s
    /// own reasoning (#1194) for the sibling delimiter class (#1677).
    #[inline]
    fn malformed_delimiter_error(&self) -> EvalError {
        EvalError::malformed_json_text(self.text)
    }

    #[inline]
    fn following_colon_ok(&self, key_end: usize) -> bool {
        following_gap_ok(self.text, key_end)
    }

    /// #2211: reuses the same `trailing_gap_ok` primitive this module's own
    /// `empty_container_gap_ok` free function already runs for
    /// `stream_json`/`stream_json_pretty`, and `jq_runner.rs`'s copy already
    /// runs for `print_json` -- one gap-scanning definition, three entry
    /// points shaped for what each caller already has in hand (this one:
    /// only the container's own cursor and which bracket closes it, no
    /// resolved value or raw span required).
    #[inline]
    fn container_gap_ok(&self, close_char: u8) -> bool {
        match self.text_position() {
            Some(pos) => trailing_gap_ok(self.text, pos + 1, close_char),
            None => true,
        }
    }

    /// #2243: reuses the same `trailing_gap_ok` primitive
    /// [`container_gap_ok`](Self::container_gap_ok) does, just against a
    /// caller-supplied `gap_start` (a real last child's own end) instead of
    /// `self`'s own opening position + 1 -- `self` is only used for its
    /// shared `text` buffer, so any cursor from the same document answers
    /// identically regardless of which node it happens to point at.
    #[inline]
    fn trailing_element_gap_ok(&self, gap_start: usize, close_char: u8) -> bool {
        trailing_gap_ok(self.text, gap_start, close_char)
    }

    #[inline]
    fn line(&self) -> usize {
        JsonCursor::line(self)
    }

    #[inline]
    fn column(&self) -> usize {
        JsonCursor::column(self)
    }

    #[inline]
    fn cursor_at_offset(&self, offset: usize) -> Option<Self> {
        JsonCursor::cursor_at_offset(self, offset)
    }

    #[inline]
    fn cursor_at_position(&self, line: usize, col: usize) -> Option<Self> {
        JsonCursor::cursor_at_position(self, line, col)
    }

    /// #1576: pretty/sort-capable, unlike before -- compact+unsorted+
    /// `Preserve` still takes the cheap verbatim-echo path below (a raw copy
    /// beats re-walking the tree, and it's already atomic: one `write_str`
    /// call, nothing to buffer), everything else recurses through
    /// `stream_json_pretty`. `JqCompat` always recurses, even when
    /// compact: reformatting a number literal (`format_number_jq_compat`)
    /// is a per-node decision the whole-value echo can't make, regardless
    /// of indentation.
    ///
    /// The recursing branch buffers into a local `String` and only copies
    /// to `out` once `stream_json_pretty` fully succeeds (#1576 review):
    /// unlike `YamlCursor`'s own `stream_json`, which streams straight to
    /// `out` and accepts a partial prefix on a later structural failure as
    /// a settled yq-mode trade (`stream_maybe_colored`'s own doc comment,
    /// #1641/#1679), real jq's own architecture parses a document fully
    /// before printing any of it -- confirmed live: `jq -c .` on
    /// `[1,2,,]` prints nothing at all before erroring, not `[1,2`. This
    /// cursor is jq's own JSON writer (unlike `YamlCursor`, which serves
    /// both jq's and yq's JSON-target output), so it needs to match that
    /// per-value atomicity, not YAML's. The buffer costs one value's worth
    /// of memory, not the whole stream's: `.[]`'s own multiple top-level
    /// results still call this once per element (`GenericResult::
    /// stream_json`'s `ManyCursor` arm), so an earlier successfully-
    /// written result stays on `out` even when a later one fails --
    /// matching real jq's own non-atomic-*across*-results streaming.
    #[inline]
    fn stream_json<Out: core::fmt::Write>(
        &self,
        out: &mut Out,
        indent: IndentSpec,
        sort_keys: bool,
        numbers: JsonConvention,
    ) -> StreamResult {
        // #2209: `preserves_source_values`, not `== Preserve` -- jq mode's
        // `--preserve-input` echoes the source span verbatim exactly as yq
        // does (that is what "preserve" means on this path, and it predates
        // #2209), so it must keep this fast path rather than falling into
        // the re-encoding writer below and losing both the speed and the
        // verbatim spelling.
        //
        if indent.is_compact() && !sort_keys && numbers.preserves_source_values() {
            let Some(bytes) = self.raw_bytes() else {
                return Err(StreamFailure::Fmt);
            };
            // #2985: a jq-escape-table convention (`JqPreserveInput`) can't
            // let a raw DEL byte (`0x7f`) through unescaped -- jq's own
            // escape table re-encodes it, and this path otherwise writes
            // the *entire* raw span verbatim with no per-byte inspection at
            // all. `Preserve` (yq's own, never a jq-escape-table
            // convention) is unaffected and skips the scan entirely. Every
            // *other* byte legal unescaped in JSON source is shared between
            // both escape tables (confirmed by #2591/#2592's own scope), so
            // DEL is the only reason a jq-escape-table convention can't
            // otherwise take this branch unconditionally the way `Preserve`
            // always could.
            //
            // A DEL hit substitutes its escape in place and keeps writing
            // the surrounding bytes verbatim, rather than falling through
            // to the validating writer below: this fast path exists for
            // `--preserve-input`'s *lenient*, non-validating echo (a stray
            // trailing comma still round-trips), and falling back to a real
            // parse would make that leniency depend on whether an unrelated
            // DEL byte happens to also be in the document. Caught by review
            // on the first draft, which did exactly that.
            //
            // Gating on `uses_jq_escape_table()` alone with no byte scan at
            // all (an even earlier draft) has its own failure mode:
            // disabling this fast path for a `JqPreserveInput` span with no
            // DEL at all silently canonicalizes an already-escaped six
            // character source spelling to jq's own two-character short
            // form, breaking
            // `test_preserve_input_keeps_jq_escape_table_2209`'s pinned
            // "verbatim spelling survives" behavior, which predates #2209
            // and has nothing to do with DEL.
            if numbers.del_needs_escaping(bytes) {
                let mut rest = bytes;
                while let Some(pos) = rest.iter().position(|&b| b == 0x7f) {
                    // SAFETY: JSON input is valid UTF-8 (checked during
                    // indexing); DEL (`0x7f`) can never appear as a
                    // continuation byte of a multi-byte sequence, so
                    // splitting here can't cut one in half.
                    let chunk = core::str::from_utf8(&rest[..pos]).map_err(|_| core::fmt::Error)?;
                    out.write_str(chunk)?;
                    out.write_str("\\u007f")?;
                    rest = &rest[pos + 1..];
                }
                let chunk = core::str::from_utf8(rest).map_err(|_| core::fmt::Error)?;
                return Ok(out.write_str(chunk)?);
            }
            // SAFETY: JSON input is valid UTF-8 (checked during indexing)
            let s = core::str::from_utf8(bytes).map_err(|_| core::fmt::Error)?;
            return Ok(out.write_str(s)?);
        }
        // #2608: checked *after* the `--preserve-input` branch above, so
        // that branch's own behavior is completely unchanged -- this is an
        // additional, narrower fast path for the *default* (non-preserve)
        // compact jq-compat render. `numbers == JsonConvention::JqCompat`
        // (not `uses_jq_escape_table()`, which is also true for
        // `JqPreserveInput` -- already handled and returned above) is what
        // keeps this from ever firing for a preserve-input render, and it's
        // also -- see `canonical_compact_jq_span_end`'s own doc comment for
        // the full argument -- what keeps `-a`/`--ascii-output` from ever
        // reaching it: `stream_json` has no ascii parameter at all, and
        // every caller that wants ascii-escaped output routes around this
        // method entirely rather than calling it with `JqCompat`.
        if indent.is_compact() && !sort_keys && numbers == JsonConvention::JqCompat {
            // Only the node's *start* is taken from the cursor. The end
            // comes out of the scan itself -- see
            // `canonical_compact_jq_span_end`'s own doc comment for why that
            // span is exactly this node's, and for the 28%-of-the-gate
            // `text_range` rescan this avoids (`raw_bytes`, which the first
            // draft called here, walks the whole container a second time
            // just to find the closing bracket the scan is about to reach
            // anyway).
            if let Some(rest) = self
                .text_position()
                .and_then(|start| self.text.get(start..))
            {
                if let Some(end) = canonical_compact_jq_span_end(rest) {
                    debug_assert_eq!(
                        self.text_range().map(|(start, stop)| stop - start),
                        Some(end),
                        "the canonical scan and `text_range` must agree on an accepted span"
                    );
                    // SAFETY: `canonical_compact_jq_span_end` only returns
                    // `Some` after validating every string's content as
                    // UTF-8, so `rest[..end]` as a whole is valid UTF-8 too.
                    let s = core::str::from_utf8(&rest[..end]).map_err(|_| core::fmt::Error)?;
                    return Ok(out.write_str(s)?);
                }
            }
            // Falls through to the general re-render path below -- either
            // no text position was available, or the span wasn't certified
            // canonical.
        }
        // #1676/#1576 review: a stray `,` in an *apparently* empty
        // container (`{,}`, `[,]`) has no child cursor for
        // `stream_json_pretty` to check a delimiter against -- see
        // `empty_container_gap_ok`'s own doc comment. This is the root
        // value's own check; `stream_json_pretty`'s array/object arms run
        // the same check for a nested empty container, using whichever
        // child cursor they still have.
        let value = self.value();
        if !empty_container_gap_ok(self, &value) {
            return Err(StreamFailure::Decode(EvalError::malformed_json_text(
                self.text(),
            )));
        }
        let mut buf = String::new();
        stream_json_pretty(
            &mut buf,
            value,
            0,
            indent.width,
            indent.unit,
            sort_keys,
            numbers,
            0,
        )?;
        Ok(out.write_str(&buf)?)
    }

    #[inline]
    fn stream_yaml<Out: core::fmt::Write>(
        &self,
        out: &mut Out,
        indent: IndentSpec,
        sort_keys: bool,
    ) -> StreamResult {
        // For JSON->YAML conversion, we need to format as YAML. `sort_keys`
        // (and `indent.unit`, e.g. `--tab`) aren't implemented here for the
        // same reason as `stream_json` above; guard explicitly so a future
        // caller fails safe instead of silently getting unsorted,
        // always-space-indented output.
        if sort_keys {
            return Err(StreamFailure::Fmt);
        }
        stream_json_as_yaml(out, self.value(), 0, indent.width)
    }

    /// #966 follow-up (#1576 review): a structurally invalid number
    /// (`1.2.3`) that `write_json_number` (`JsonConvention::JqCompat`)
    /// sanitizes to `null` in *output* must also report falsy *here* under
    /// that same convention, or `-e` on `.a` over `{"a": 1.2.3}` would exit
    /// 0 despite the printed `null` -- inconsistent with the older
    /// `to_owned`-based materializing path (still used whenever
    /// `can_json_fast_path` excludes a query, e.g. `-S`), which already
    /// correctly exits 1. `--preserve-input`/`Preserve` echoes the same
    /// span unsanitized (still nominally a `Number`), so it stays truthy
    /// there -- only `JqCompat` treats a malformed number as falsy.
    ///
    /// "Malformed" is "decodes to no number at all" (`as_f64` is `None`,
    /// i.e. `from_number_bytes` gives `Null`), not "has no preservable
    /// literal": since #2877 a document `nan`/`Infinity` is a number with no
    /// literal spelling, and it is truthy in jq even though a NaN *prints*
    /// as `null` (`jq -ne '[nan] | .[0]'` exits 0). The old literal-based
    /// test also called `[1.]`'s element falsy while printing it as `1`.
    ///
    /// This runs once per streamed output value (`.[]` over a large array
    /// reaches it per element), so the ordinary case is settled by the
    /// `is_valid_number` scan alone -- a valid RFC 8259 span always decodes
    /// -- and only a lenient span pays for the decode.
    #[inline]
    fn is_falsy(&self, numbers: JsonConvention) -> bool {
        match self.value() {
            StandardJson::Null | StandardJson::Bool(false) => true,
            StandardJson::Number(n) => {
                numbers == JsonConvention::JqCompat && {
                    let raw = n.raw_bytes();
                    !crate::json::validate::is_valid_number(raw) && n.as_f64().is_err()
                }
            }
            _ => false,
        }
    }

    /// #1576: `JsonCursor` now implements both `stream_sequence_*` methods
    /// below (JSON only -- `stream_sequence_yaml` stays at the trait
    /// default; JSON->YAML sequence streaming is a separate, unimplemented
    /// gap, tracked as a follow-up rather than folded into this issue), so a
    /// `LazySeq` whose elements are all still cursors renders straight from
    /// the source document rather than through an `OwnedValue::Array`,
    /// matching what #757 already did for `YamlCursor`.
    #[inline]
    fn supports_sequence_streaming() -> bool {
        true
    }

    #[inline]
    fn stream_sequence_json<Out: core::fmt::Write>(
        cursors: &[Self],
        out: &mut Out,
        indent: IndentSpec,
        sort_keys: bool,
        numbers: JsonConvention,
    ) -> StreamResult {
        stream_json_sequence(
            cursors,
            out,
            0,
            indent.width,
            indent.unit,
            sort_keys,
            numbers,
        )
    }
}

impl<'a, W: AsRef<[u64]> + Clone> DocumentValue for StandardJson<'a, W> {
    type Cursor = JsonCursor<'a, W>;
    type Fields = JsonFields<'a, W>;
    type Elements = JsonElements<'a, W>;

    #[inline]
    fn is_null(&self) -> bool {
        matches!(self, StandardJson::Null)
    }

    fn as_bool(&self) -> Option<bool> {
        match self {
            StandardJson::Bool(b) => Some(*b),
            _ => None,
        }
    }

    fn as_i64(&self) -> Option<i64> {
        match self {
            StandardJson::Number(n) => n.as_i64().ok(),
            _ => None,
        }
    }

    fn as_f64(&self) -> Option<f64> {
        match self {
            StandardJson::Number(n) => n.as_f64().ok(),
            _ => None,
        }
    }

    fn bridge_computed_float(&self) -> Option<f64> {
        match self {
            StandardJson::Number(n) => {
                crate::json::validate::parse_computed_float_token(n.raw_bytes())
            }
            _ => None,
        }
    }

    fn number_text(&self) -> Option<Cow<'_, str>> {
        match self {
            StandardJson::Number(n) => core::str::from_utf8(n.raw_bytes()).ok().map(Cow::Borrowed),
            _ => None,
        }
    }

    fn number_literal(&self) -> Option<Cow<'_, str>> {
        match self {
            StandardJson::Number(n) => {
                let bytes = n.raw_bytes();
                if crate::json::validate::is_valid_number(bytes) {
                    return core::str::from_utf8(bytes).ok().map(Cow::Borrowed);
                }
                // Real jq's own number reader tolerates a redundant
                // leading zero that strict RFC 8259 doesn't (`007` ->
                // `7`, `007e5` -> `7E+5`, `007.500` -> `7.500`) -- #1149,
                // same leniency as `OwnedValue::from_number_bytes`'s own
                // leading-zero handling (shared gate via
                // `strip_redundant_leading_zeros`, since this trait impl
                // has no access to that jq-layer type). Reached by
                // `--argjson`/`--jsonargs` (`parse_json_value`'s own
                // normalize-and-retry validation lets a leading-zero
                // literal survive to materialize here) and this crate's
                // own `.json`-file input path, which both materialize
                // through this generic `DocumentValue` trait rather than
                // `from_number_bytes` directly. `--slurpfile`/`--seq`
                // don't reach this arm at all -- both validate via a
                // stricter path with no leading-zero retry
                // (`parse_json_stream`'s `serde_json::Deserializer`,
                // `parse_json_seq`'s own `validate::validate`), so
                // a leading-zero literal there still errors/is dropped
                // before ever reaching `number_literal()` (confirmed
                // live; pre-existing, out-of-scope gap, unchanged by this
                // fix). Returns the *original*
                // `bytes` here, not the stripped copy the gate check
                // builds -- dropping the redundant zero is purely a
                // display-time concern (`format_number_jq_compat`), not
                // something the stored spelling itself needs to already
                // reflect (matches `from_number_bytes`'s own leading-dot
                // and leading-zero handling, both of which store the
                // original text for the same reason).
                // Real jq's own number reader also accepts a leading `.`
                // (with or without a preceding `-`) when at least one digit
                // follows (`.5` -> `0.5`, `-.5` -> `-0.5`, #1171) -- same
                // leniency as `OwnedValue::from_number_bytes`'s own
                // leading-dot handling, shared gate via `has_leading_dot`.
                // Checked before the leading-zero-strip escape below (no
                // int digits to strip in the first place for a leading-dot
                // token, so the two never compose the way the leading-zero
                // and trailing-dot escapes do) -- missing entirely until
                // #2240's own code review found it: `--argjson` navigating
                // through this generic-evaluator path (not
                // `from_number_bytes`'s own primary document-input path)
                // silently lost precision on a leading-dot literal instead
                // of preserving its spelling, once #2240's own fix let
                // `--argjson` accept one in the first place.
                if crate::json::validate::has_leading_dot(bytes) {
                    return core::str::from_utf8(bytes).ok().map(Cow::Borrowed);
                }
                let zero_stripped = crate::json::validate::strip_redundant_leading_zeros(bytes);
                if let Some(stripped) = &zero_stripped {
                    if crate::json::validate::is_valid_number(stripped) {
                        return core::str::from_utf8(bytes).ok().map(Cow::Borrowed);
                    }
                }
                // Real jq's own number reader also tolerates a trailing
                // `.` immediately before an exponent marker (`1.e999` ->
                // `1.0e999`) -- same leniency as
                // `OwnedValue::from_number_bytes`'s own trailing-dot
                // handling (#2220, shared gate via
                // `has_trailing_dot_before_exponent`). Checked against the
                // leading-zero-stripped form above (when one exists), not
                // always `bytes` itself, so the two escapes compose: a
                // token can have both a redundant leading zero *and* a
                // trailing dot before its exponent at once (`007.e999`).
                // Returns the *original* `bytes`, matching every other
                // escape in this function and in `from_number_bytes`.
                let base = zero_stripped.as_deref().unwrap_or(bytes);
                if crate::json::validate::has_trailing_dot_before_exponent(base) {
                    return core::str::from_utf8(bytes).ok().map(Cow::Borrowed);
                }
                // A leading `+` (#2877): `+X` is `X` in jq, and the
                // literal is the *unsigned* text, borrowed from the same
                // span one byte in -- the same peel
                // `OwnedValue::from_number_bytes` makes, re-running the
                // four gates above on the unsigned text
                // (`is_preservable_number_literal` names that exact set).
                // decNumber's special words (`nan`, `-Infinity`, ...)
                // deliberately reach none of these gates and return `None`
                // here: they have no JSON spelling to preserve, so the
                // caller's `as_f64` fallback hands back the bare value
                // instead, exactly as `from_number_bytes` does.
                if let Some(unsigned) = crate::json::validate::strip_leading_plus(bytes) {
                    if crate::json::validate::is_preservable_number_literal(unsigned) {
                        return core::str::from_utf8(unsigned).ok().map(Cow::Borrowed);
                    }
                }
                // The semi-index scanner accepts number *spans* more
                // leniently than RFC 8259 beyond just a leading zero
                // (e.g. `1.2.3` — see #966): echoing such text verbatim
                // would produce invalid JSON output. Fall through to
                // `as_i64`/`as_f64` (still lenient, but numerically
                // sound) or `Null`.
                None
            }
            _ => None,
        }
    }

    fn as_str(&self) -> Option<Cow<'_, str>> {
        match self {
            StandardJson::String(s) => s.as_str().ok(),
            _ => None,
        }
    }

    /// The span's content bytes, quotes stripped, when it carries no
    /// escape -- in which case they *are* the decoded key, so the
    /// duplicate-key probe can hash them without going through `as_str`
    /// (#1514). `raw_and_escaped` reports both from the one scan it makes
    /// for the closing quote, so the escape test is free.
    fn key_raw_unescaped(&self) -> Option<&[u8]> {
        match self {
            StandardJson::String(s) => {
                // `has_del` (#2591) doesn't matter here: this probe hashes
                // the *decoded* content for duplicate-key comparison, and a
                // raw DEL byte's decoded value is itself either way -- the
                // convention-specific re-escaping question it exists for is
                // strictly an output concern.
                let (raw, escaped, _has_del) = s.raw_and_escaped();
                // A well-formed span is `"..."`; anything shorter than the
                // two quotes is a truncated document, and has no content to
                // hand back.
                if escaped || raw.len() < 2 {
                    None
                } else {
                    Some(&raw[1..raw.len() - 1])
                }
            }
            _ => None,
        }
    }

    fn string_decode_error(&self) -> Option<&'static str> {
        match self {
            StandardJson::String(s) => s.as_str().err().map(JsonError::message),
            _ => None,
        }
    }

    /// One `as_str()` for both answers, where the trait default would run
    /// two (#965 item 10).
    ///
    /// This type does not override `key_string()`, so it inherits
    /// `as_str()` there -- meaning the default `decoded_key_str` decodes the
    /// same span twice on the success path, discarding the first `Cow`
    /// entirely. `JsonString::as_str` caches nothing: each call re-scans for
    /// the closing quote, re-scans for a backslash, re-validates UTF-8, and
    /// for an escaped key allocates a fresh `decode_escapes` buffer.
    ///
    /// `Ok(None)` for a non-`String` key preserves `key_string()`'s `None`
    /// -- the #1194 fault a caller still has to raise on.
    fn decoded_key_str(&self) -> Result<Option<Cow<'_, str>>, &'static str> {
        match self {
            StandardJson::String(s) => s.as_str().map(Some).map_err(JsonError::message),
            _ => Ok(None),
        }
    }

    /// Unlike [`key_raw_unescaped`](Self::key_raw_unescaped), this answers
    /// for an escaped span too -- including one whose escape is invalid --
    /// since it exists only as a display fallback for a key that fails to
    /// *decode* (#1642), not for hashing.
    fn key_raw_source_span(&self) -> Option<&[u8]> {
        match self {
            StandardJson::String(s) => {
                let raw = s.raw_bytes();
                (raw.len() >= 2).then(|| &raw[1..raw.len() - 1])
            }
            // Unreachable as it stands -- `key_display_string`'s only
            // caller (`document.rs`) reaches this method solely behind
            // `string_decode_error().is_some()`, which for this type is
            // itself only `Some` on the `String` arm above -- but the
            // trait's return type is an `Option` and something has to be
            // written here. `None` matches the trait default.
            _ => None,
        }
    }

    /// The two token-shaped variants each already carry their own opening
    /// position (#1643's `JsonString::start`/`JsonNumber::start`), so a
    /// caller that has decoded either can reuse it for #1677's delimiter
    /// check for free.
    fn text_start(&self) -> Option<usize> {
        match self {
            StandardJson::String(s) => Some(s.start()),
            StandardJson::Number(n) => Some(n.start()),
            _ => None,
        }
    }

    /// Only `String`: a key is never a `Number` on a well-formed document,
    /// and this is used for nothing else (#1677).
    fn text_end(&self) -> Option<usize> {
        match self {
            StandardJson::String(s) => Some(s.end()),
            _ => None,
        }
    }

    /// #2243: delegates to this module's own (now-shared) `scalar_end_pos`
    /// free function -- every variant answers `start + its own byte length`
    /// (`s`/`n`'s own `raw_bytes().len()` for `String`/`Number`, a fixed
    /// 4/5/4 for `Bool(true)`/`Bool(false)`/`Null`), trusting the caller's
    /// `start` rather than re-deriving it from `s`/`n`'s own `start()`
    /// (which would be redundant work when `start` is already this same
    /// value's own resolved position, the only way callers ever have one
    /// to pass).
    fn scalar_text_end(&self, start: usize) -> Option<usize> {
        scalar_end_pos(start, self)
    }

    fn as_object(&self) -> Option<Self::Fields> {
        match self {
            StandardJson::Object(fields) => Some(*fields),
            _ => None,
        }
    }

    fn as_array(&self) -> Option<Self::Elements> {
        match self {
            StandardJson::Array(elements) => Some(*elements),
            _ => None,
        }
    }

    fn type_name(&self) -> &'static str {
        match self {
            StandardJson::Null => "null",
            StandardJson::Bool(_) => "boolean",
            StandardJson::Number(_) => "number",
            StandardJson::String(_) => "string",
            StandardJson::Array(_) => "array",
            StandardJson::Object(_) => "object",
            StandardJson::Error(_) => "error",
        }
    }

    fn is_error(&self) -> bool {
        matches!(self, StandardJson::Error(_))
    }

    fn error_message(&self) -> Option<&'static str> {
        match self {
            StandardJson::Error(msg) => Some(msg),
            _ => None,
        }
    }
}

impl<'a, W: AsRef<[u64]> + Clone> DocumentFields for JsonFields<'a, W> {
    type Value = StandardJson<'a, W>;
    type Cursor = JsonCursor<'a, W>;

    fn uncons(&self) -> Option<(DocumentField<Self::Value, Self::Cursor>, Self)> {
        let (field, rest) = JsonFields::uncons(self)?;
        Some((
            DocumentField {
                key: field.key(),
                value: field.value(),
                key_cursor: field.key_cursor(),
                value_cursor: field.value_cursor(),
            },
            rest,
        ))
    }

    /// The inherent `JsonFields::uncons` builds a `JsonField` of two
    /// cursors and materializes nothing, so a key-only walk pays for one
    /// `key()` and no `value()` -- which is the difference between this
    /// and the trait default (#1514).
    fn uncons_key(&self) -> Option<(Self::Value, Self::Cursor, Self)> {
        let (field, rest) = JsonFields::uncons(self)?;
        Some((field.key(), field.key_cursor(), rest))
    }

    fn find(&self, name: &str) -> Result<Option<Self::Value>, EvalError> {
        JsonFields::find(self, name)
    }

    fn find_cursor(&self, name: &str) -> Result<Option<Self::Cursor>, EvalError> {
        JsonFields::find_cursor(self, name)
    }

    fn is_empty(&self) -> bool {
        JsonFields::is_empty(self)
    }

    /// Re-runs the strict validator over this object's own document to name
    /// the real syntax error, rather than the generic wording the trait
    /// default has to settle for (#1194).
    ///
    /// Reachable only once a malformed member has already been found, so a
    /// well-formed document never pays for the pass. The cursor is what makes
    /// this possible here and not in the generic evaluator: `JsonCursor` keeps
    /// the document text, where `DocumentCursor` exposes only a position.
    ///
    /// Falls back to the trait default's shape when the list is somehow empty
    /// -- there is no cursor to read a document from, and inventing a position
    /// would be worse than saying less.
    fn malformed_member_error(&self) -> EvalError {
        match self.key_cursor {
            Some(cursor) => EvalError::malformed_json_text(cursor.text()),
            // #2286: decode_failure, not new -- same always-uncatchable tag
            // the cursor-present arm gets via malformed_json_text, so this
            // fallback doesn't quietly reintroduce the gap for the one case
            // (empty list, no cursor to read) that skips it.
            None => EvalError::decode_failure("Invalid JSON text"),
        }
    }

    /// JSON is the one format that can present an unpaired child, so it is
    /// the one format that overrides this (#1194). See the inherent
    /// [`JsonFields::ends_unpaired`].
    fn ends_unpaired(&self) -> bool {
        JsonFields::ends_unpaired(self)
    }
}

impl<'a, W: AsRef<[u64]> + Clone> DocumentElements for JsonElements<'a, W> {
    type Value = StandardJson<'a, W>;
    type Cursor = JsonCursor<'a, W>;

    fn uncons(&self) -> Option<(Self::Value, Self)> {
        JsonElements::uncons(self)
    }

    fn uncons_cursor(&self) -> Option<(Self::Cursor, Self)> {
        JsonElements::uncons_cursor(self)
    }

    fn get(&self, index: usize) -> Option<Self::Value> {
        JsonElements::get_fast(self, index)
    }

    fn is_empty(&self) -> bool {
        JsonElements::is_empty(self)
    }

    /// Re-runs the strict validator, mirroring
    /// [`JsonFields::malformed_member_error`]'s own reasoning (#1194) for
    /// the array delimiter class (#1677).
    fn malformed_element_error(&self) -> EvalError {
        match self.element_cursor {
            Some(cursor) => EvalError::malformed_json_text(cursor.text()),
            // #2286: same fallback fix as JsonFields::malformed_member_error
            // above.
            None => EvalError::decode_failure("Invalid JSON text"),
        }
    }
}

// ============================================================================
// JSON to JSON Streaming Helpers (#1576)
// ============================================================================

/// Stream a JSON value as (pretty- or compact-, sorted- or unsorted-) JSON,
/// without materializing an `OwnedValue` -- the pretty-capable counterpart
/// [`JsonCursor::stream_json`] falls back to for anything but the
/// compact+unsorted+`Preserve` case, which echoes raw source bytes instead
/// (cheaper than re-walking the tree to reproduce them unchanged).
///
/// Structurally mirrors [`stream_json_as_yaml`] above (and
/// `YamlCursor::stream_json_value` in `src/yaml/light.rs`, #757's own
/// pretty-capable cursor writer): thread `current_indent`/`indent_spaces`/
/// `unit`/`sort_keys` down through the recursion, write `,`/newline+indent
/// between entries only when `indent_spaces > 0`, collect-and-sort object
/// fields only when `sort_keys`. Simpler than both siblings in one respect:
/// JSON has no tags, aliases or per-position scalar-type resolution to
/// worry about, so this recurses on plain [`StandardJson`] values (not
/// cursors) exactly like [`stream_json_as_yaml`] does.
#[allow(clippy::too_many_arguments)] // STYLE-0004: mirrors stream_owned_value_json_with_at_depth's own suppression; every param is threaded through this function's own recursion.
fn stream_json_pretty<W: AsRef<[u64]> + Clone, Out: core::fmt::Write>(
    out: &mut Out,
    value: StandardJson<'_, W>,
    current_indent: usize,
    indent_spaces: usize,
    unit: char,
    sort_keys: bool,
    numbers: JsonConvention,
    depth: usize,
) -> StreamResult {
    // #1576 review: this writer recurses on plain values, not through
    // `to_owned_at_depth`/`to_owned_cursor_at_depth`, so it doesn't get
    // either of those functions' own `assert_nesting_depth` guard for
    // free. Matches `print_json`'s own choice (`src/bin/succinctly/
    // jq_runner.rs`, its own doc comment explains why in detail) rather
    // than `to_owned_at_depth`'s: this writer, like `print_json`, already
    // streams partial output before a leaf can fail, so a catchable error
    // is required, not a panic -- and `MAX_VALUE_TREE_DEPTH` (384), not
    // the narrower `MAX_NESTING_DEPTH` (256), is the ceiling every other
    // value-tree consumer (including `print_json`) uses for exactly this
    // reason (#1819).
    if depth >= MAX_VALUE_TREE_DEPTH {
        return Err(StreamFailure::Decode(EvalError::new(
            nesting_depth_exceeded_message(MAX_VALUE_TREE_DEPTH),
        )));
    }
    match value {
        StandardJson::Null => Ok(out.write_str("null")?),
        StandardJson::Bool(b) => Ok(out.write_str(if b { "true" } else { "false" })?),
        StandardJson::Number(n) => Ok(write_json_number(out, n, numbers)?),
        StandardJson::String(s) => write_json_string_pretty(out, s, numbers),
        StandardJson::Array(elements) => {
            if elements.is_empty() {
                return Ok(out.write_str("[]")?);
            }
            out.write_char('[')?;
            let next_indent = current_indent + indent_spaces;
            let mut first = true;
            let mut rest = elements;
            // Tracks the last *scalar* element's own end position, for the
            // trailing-comma check after the loop (`[1,2,]`) -- `None` once
            // a container element is seen, matching `scalar_end_pos`'s own
            // deferral (see its doc comment).
            let mut last_scalar_end: Option<(&[u8], usize)> = None;
            // Cursor-yielding `uncons_cursor`, not the plain value
            // `Iterator`/`IntoIterator` impl: #1677's missing/doubled
            // `,` check needs each element's own `text_position()`, which
            // only the cursor carries -- `to_owned_at_depth`'s identical
            // array arm (`eval_generic.rs`) is the precedent this mirrors.
            while let Some((elem_cursor, next)) = rest.uncons_cursor() {
                if !first {
                    out.write_char(',')?;
                }
                last_scalar_end = None;
                // Resolved once and reused below for both the trailing-gap
                // bookkeeping and the recursive render, instead of a second
                // `elem_cursor.value()` re-deriving the same value (#1576
                // review).
                // #1803: `element_gap_ok_at`, not `element_gap_ok` -- this
                // site needs the resolved `pos` itself for `value_at(pos)`
                // and `scalar_end_pos(pos, ..)` just below, so it cannot
                // give up the position the way the `bool`-only sibling does.
                // Routing through the shared method anyway keeps the
                // `is_first` -> delimiter mapping at one definition.
                let elem_value = if let Some(pos) = elem_cursor.text_position() {
                    if !elem_cursor.element_gap_ok_at(pos, first) {
                        return Err(StreamFailure::Decode(
                            elem_cursor.malformed_delimiter_error(),
                        ));
                    }
                    let elem_value = elem_cursor.value_at(pos);
                    // #1576 review: `stream_json`'s root-only empty-
                    // container check (see `empty_container_gap_ok`) never
                    // reaches a non-root element like this one, so a stray
                    // `,` inside a *nested* empty container (`[{"a": [,]}]`)
                    // needs its own check here, against this element's own
                    // cursor.
                    if !empty_container_gap_ok(&elem_cursor, &elem_value) {
                        return Err(StreamFailure::Decode(EvalError::malformed_json_text(
                            elem_cursor.text(),
                        )));
                    }
                    last_scalar_end =
                        scalar_end_pos(pos, &elem_value).map(|end| (elem_cursor.text(), end));
                    elem_value
                } else {
                    elem_cursor.value()
                };
                first = false;
                rest = next;
                if indent_spaces > 0 {
                    out.write_char('\n')?;
                    write_json_indent(out, next_indent, unit)?;
                }
                stream_json_pretty(
                    out,
                    elem_value,
                    next_indent,
                    indent_spaces,
                    unit,
                    sort_keys,
                    numbers,
                    depth + 1,
                )?;
            }
            // #1676: a trailing `,` (`[1,2,]`) -- deferred (not checked) when
            // the last element is itself a container, matching
            // `scalar_end_pos`'s own precedent.
            if let Some((text, gap_start)) = last_scalar_end {
                if !trailing_gap_ok(text, gap_start, b']') {
                    return Err(StreamFailure::Decode(EvalError::malformed_json_text(text)));
                }
            }
            if indent_spaces > 0 {
                out.write_char('\n')?;
                write_json_indent(out, current_indent, unit)?;
            }
            Ok(out.write_char(']')?)
        }
        StandardJson::Object(fields) => {
            if fields.is_empty() {
                return Ok(out.write_str("{}")?);
            }
            // The mode's own `COLLAPSE_DUPLICATE_KEYS` rule (true for jq --
            // a repeated key collapses to one field, first position, last
            // value, exactly `IndexMap::insert` semantics; false for
            // `--preserve-input`/yq, every occurrence kept, real yq's own
            // behavior #1008 -- the same axis `numbers` already selects,
            // ADR-0018 rule 5, so `JqCompat` doubles as the collapse flag
            // here too).
            let collapse = numbers == JsonConvention::JqCompat;
            // #2720: the fields are materialized only when the writer
            // genuinely needs all of them in hand before it can write the
            // first -- `-S` (every key, to sort) and an object that
            // actually repeats a key under a collapsing mode (a later
            // occurrence changes an earlier field's value). Every other
            // object -- the common one -- is validated by one key-only
            // walk (`collapsed_fields_checked`, #1194/#1677/#2261's checks,
            // the same ones the materializing `effective_fields_checked`
            // runs) and then streamed straight off `uncons` in document
            // order. The materialized path used to be unconditional: a
            // 144-byte `DocumentField` per field, 22.9 MB for perf-guard's
            // 2 MB `wide` fixture, and half of the identity print's
            // instructions on a 7950X.
            let materialized = if sort_keys {
                let items =
                    effective_fields_checked(&fields, collapse).map_err(StreamFailure::Decode)?;
                // Sort by the *decoded* key, matching `-S`'s meaning
                // everywhere else in this codebase (`write_object_entries`
                // in `src/jq/stream.rs`, `YamlCursor::stream_json_value` in
                // `src/yaml/light.rs`) -- not by the raw source span, which
                // could disagree with decoded order for an escaped key.
                let mut keyed = Vec::with_capacity(items.len());
                for field in items {
                    let StandardJson::String(k) = field.key else {
                        // `effective_fields_checked` already refused any
                        // key that isn't a well-formed `String` token
                        // (#1194's `key_is_malformed`), so this is
                        // unreachable in practice, matching
                        // `stream_json_as_yaml`'s own key arm.
                        keyed.push((String::new(), field));
                        continue;
                    };
                    let key_str = k.as_str().map_err(json_decode_failure)?;
                    keyed.push((key_str.into_owned(), field));
                }
                keyed.sort_by(|a, b| a.0.cmp(&b.0));
                Some(
                    keyed
                        .into_iter()
                        .map(|(_, field)| field)
                        .collect::<Vec<_>>(),
                )
            } else {
                collapsed_fields_checked(&fields, collapse).map_err(StreamFailure::Decode)?
            };
            out.write_char('{')?;
            let next_indent = current_indent + indent_spaces;
            let mut first = true;
            match materialized {
                Some(items) => {
                    for field in items {
                        stream_json_field(
                            out,
                            field.key,
                            field.value,
                            &field.value_cursor,
                            first,
                            next_indent,
                            indent_spaces,
                            unit,
                            sort_keys,
                            numbers,
                            depth,
                        )?;
                        first = false;
                    }
                }
                None => {
                    // The inherent `JsonFields::uncons`, not the trait's:
                    // two cursors per field and nothing decoded until the
                    // writer asks (`key()`/`value()` below), where the trait
                    // wrapper materializes a `DocumentField` up front.
                    let mut walk = fields;
                    while let Some((field, rest)) = JsonFields::uncons(&walk) {
                        stream_json_field(
                            out,
                            field.key(),
                            field.value(),
                            &field.value_cursor(),
                            first,
                            next_indent,
                            indent_spaces,
                            unit,
                            sort_keys,
                            numbers,
                            depth,
                        )?;
                        first = false;
                        walk = rest;
                    }
                }
            }
            if indent_spaces > 0 {
                out.write_char('\n')?;
                write_json_indent(out, current_indent, unit)?;
            }
            Ok(out.write_char('}')?)
        }
        // Unlike `stream_json_as_yaml`'s identical-looking arm below (a
        // pre-existing, unrelated writer this fix intentionally leaves
        // alone), silently substituting `null` here is a real regression
        // for this writer specifically (#1576 review): before this writer
        // existed, every `map`/`sort`/etc. result reached `to_owned_cursor`
        // (via the `OwnedValue::Array` fallback), which already raises a
        // proper `#1194`-class diagnostic for a structurally malformed
        // member/element (confirmed live: `map(.)` on
        // `[1, {"bad": xyz123}]` reports "unexpected character" and exits
        // 5 through that path) -- `map(.)` never used to silently emit
        // `null` for this, so this writer must not start doing that either
        // now that it renders `map`'s cursors directly. `EvalError::new`
        // with `msg` isn't as specific as `to_owned_cursor`'s own
        // `malformed_member_error`/`malformed_element_error` (this writer
        // recurses on plain values, not cursors+fields, so that richer
        // context isn't available here) -- but a real error is what
        // matters: the caller (`GenericResult::stream_json`'s `LazySeq`
        // arm) discards this writer's own partial output rather than the
        // whole document silently reading back as valid JSON with a
        // fabricated `null`. #2286: `decode_failure`, not `new` -- matches
        // this same function's `malformed_json_text`-tagged arms just above
        // (same uncatchable #1194 error class), and keeps this arm
        // consistent with every other `StandardJson::Error` site fixed by
        // #2286.
        StandardJson::Error(msg) => Err(StreamFailure::Decode(EvalError::decode_failure(msg))),
    }
}

/// One object member of [`stream_json_pretty`]'s object arm -- the
/// separator, indentation, key, `:`, the nested-empty-container check and
/// the value's own recursive render -- shared by its materialized and
/// streaming loops (#2720) so the two cannot drift.
#[allow(clippy::too_many_arguments)] // STYLE-0004: stream_json_pretty's own ambients, plus the field
fn stream_json_field<W: AsRef<[u64]> + Clone, Out: core::fmt::Write>(
    out: &mut Out,
    key: StandardJson<'_, W>,
    value: StandardJson<'_, W>,
    value_cursor: &JsonCursor<'_, W>,
    first: bool,
    next_indent: usize,
    indent_spaces: usize,
    unit: char,
    sort_keys: bool,
    numbers: JsonConvention,
    depth: usize,
) -> StreamResult {
    if !first {
        out.write_char(',')?;
    }
    if indent_spaces > 0 {
        out.write_char('\n')?;
        write_json_indent(out, next_indent, unit)?;
    }
    if let StandardJson::String(k) = key {
        write_json_string_pretty(out, k, numbers)?;
    } else {
        out.write_str("\"\"")?;
    }
    out.write_str(if indent_spaces > 0 { ": " } else { ":" })?;
    // #1576 review: same nested-empty-container gap as the array arm (see
    // `empty_container_gap_ok`'s doc comment) -- `stream_json`'s root-only
    // check never reaches a field's value here, so `{"a": {"b": {,}}}`
    // needs its own check against this field's own value cursor.
    if !empty_container_gap_ok(value_cursor, &value) {
        return Err(StreamFailure::Decode(EvalError::malformed_json_text(
            value_cursor.text(),
        )));
    }
    stream_json_pretty(
        out,
        value,
        next_indent,
        indent_spaces,
        unit,
        sort_keys,
        numbers,
        depth + 1,
    )
}

/// Write a JSON string value using `numbers`'s escaping convention
/// (`write_json_body_jq`/`write_json_body_yq`).
///
/// Zero-copy fast path: a span with no `\` escape needs no re-encoding
/// under yq's own convention (`Preserve`), which agrees with source on
/// every byte legal unescaped in JSON -- but a jq-escape-table convention
/// (`JqCompat`/`JqPreserveInput`) diverges from that at exactly one such
/// byte, DEL (`0x7f`, #2591: `jq::escape`'s own
/// `conventions_differ_at_exactly_five_code_points` names all five
/// differences -- three within the control-character range this comment's
/// own claim is scoped to, plus two multi-byte separators, #1982, that are
/// out of scope for a single-*byte* fast path like this one; the other two
/// control-range differences, backspace/form-feed, always arrive
/// pre-escaped with a `\` and so never reach this fast path at all).
/// `del_unsafe` below is what keeps such a span containing a raw DEL byte
/// off this path (#2591/#2985 -- the three sibling call sites in
/// `src/bin/succinctly/jq_runner.rs`'s own `print_json`, tracked in #2592,
/// and this function's own `JqPreserveInput` gap, tracked in #2985, are
/// both fixed now too).
fn write_json_string_pretty<Out: core::fmt::Write>(
    out: &mut Out,
    s: JsonString<'_>,
    numbers: JsonConvention,
) -> StreamResult {
    let (raw, escaped, has_del) = s.raw_and_escaped();
    // #2591: a raw DEL byte needs no backslash to be legal JSON source, but
    // jq's own escape table (unlike yq's -- `Preserve`) still escapes it on
    // output. The two conventions agree on every *other* byte legal
    // unescaped in source, so this is the one case the fast path below
    // can't take unconditionally under a jq-escape-table convention.
    // `has_del` comes free from `raw_and_escaped`'s own existing scan
    // (already visiting every byte of the span to find a backslash), so
    // this check costs nothing beyond what that scan already pays for
    // every span, escaped or not.
    //
    // #2985: keyed on `uses_jq_escape_table()`, not `== JqCompat` -- the
    // latter silently excluded `JqPreserveInput`, which also uses jq's
    // escape table (#2209) despite preserving source *values*, so a raw
    // DEL echoed through unescaped under `--preserve-input` pretty-printing
    // (`succinctly jq` with no `-c`), unlike real jq, which escapes it
    // there too.
    let del_unsafe = has_del && numbers.uses_jq_escape_table();
    // Unlike `print_json`'s own zero-copy check (`std::io::Write`, which
    // passes bytes through unvalidated), this writer is `core::fmt::Write`
    // -- a `str`-oriented trait -- so invalid UTF-8 in an unescaped span
    // can't just be blasted through. Falling through to the decode-and-
    // escape path below on that specific failure (rather than surfacing a
    // bare `core::fmt::Error` right here) keeps this a proper diagnosable
    // decode failure via `json_decode_failure`, matching #1615 -- not a
    // second, worse way for the same class of bad input to go undiagnosed.
    if !escaped && !del_unsafe {
        if let Ok(text) = core::str::from_utf8(raw) {
            return Ok(out.write_str(text)?);
        }
    }
    let decoded = s.as_str().map_err(json_decode_failure)?;
    out.write_char('"')?;
    // #2209: keyed on the escape-table axis alone, never on
    // `== Preserve`. jq mode's `--preserve-input` (`JqPreserveInput`)
    // preserves numbers and duplicate keys but keeps *jq's* table --
    // selecting yq's here was the divergence that issue fixed.
    if numbers.uses_jq_escape_table() {
        write_json_body_jq(out, &decoded)?;
    } else {
        write_json_body_yq(out, &decoded)?;
    }
    Ok(out.write_char('"')?)
}

/// Write a JSON number literal per `numbers`'s convention -- `Preserve`
/// echoes the source spelling verbatim (#1008, matching
/// `real_output_finite_literal` in `src/jq/stream.rs`); `JqCompat`
/// canonicalizes it via `format_number_jq_compat`, matching real jq's own
/// reader/writer and the jq CLI's existing non-streaming `print_json`
/// (`formatter.format_raw_number`, `src/bin/succinctly/output.rs`).
///
/// JSON source numbers are always finite (the grammar has no NaN/Infinity
/// literal), unlike `OwnedValue`'s `NumberLiteral`, which can hold a
/// *computed* infinite/NaN float from arithmetic -- so this needs none of
/// `stream_owned_value_json_with`'s infinite-value handling.
///
/// #1576 review: mirrors `JqCompatFormatter`/`PreserveFormatter::
/// format_raw_number` (`src/bin/succinctly/jq_runner.rs`) exactly, rather
/// than the narrower `is_valid_number`-only gate an earlier revision of
/// this function used -- that gate rejected a leading-dot span (`.500`)
/// outright, where real jq (and this crate's own `print_json`) accepts it
/// (`.500` -> `0.500`, #1171) via `OwnedValue::from_number_bytes`'s own
/// prepend-`0`-and-reparse leniency. `Preserve` doesn't validate at all
/// (`PreserveFormatter`'s own contract: echo the source spelling
/// unconditionally, matching real yq's #1008 convention even for text
/// that isn't a valid number at all); only `JqCompat` needs the fallback
/// chain, since only it ever reformats.
fn write_json_number<Out: core::fmt::Write>(
    out: &mut Out,
    n: JsonNumber<'_>,
    numbers: JsonConvention,
) -> core::fmt::Result {
    let raw = n.raw_bytes();
    match numbers {
        // #2209: both source-preserving conventions echo the spelling
        // verbatim -- they differ only in escape table, which is no
        // concern of a number literal's.
        JsonConvention::Preserve | JsonConvention::JqPreserveInput => {
            let text = core::str::from_utf8(raw).map_err(|_| core::fmt::Error)?;
            out.write_str(text)
        }
        JsonConvention::JqCompat => {
            // #2206: echo the source span when the formatter would hand
            // back exactly these bytes. `format_number_jq_compat` returns
            // an owned `String` unconditionally, so without this every
            // number in the document costs an allocation purely to
            // reproduce itself -- several hundred thousand of them on a
            // 10 MB array, and the single reason `-c .data` could not
            // reach the raw-echo fast path's throughput (that path renders
            // the same document 4x faster; measured on both a 7950X and an
            // M4 Pro, #2206).
            //
            // `is_jq_canonical_number` answers "certainly unchanged" only,
            // and is pinned against the formatter by
            // `is_jq_canonical_number_agrees_with_the_formatter_2206`.
            // Ahead of `is_valid_number` on purpose: the predicate proves
            // RFC-validity itself, so a `true` skips both that scan and the
            // formatter's allocation. Ordering it *after* the gate instead
            // leaves the scan in place and measures as a net loss (#2206).
            // ASCII digits only, so the span is valid UTF-8 by construction.
            if is_jq_canonical_number(raw) {
                if let Ok(text) = core::str::from_utf8(raw) {
                    return out.write_str(text);
                }
            }
            if crate::json::validate::is_valid_number(raw) {
                return out.write_str(&format_number_jq_compat(raw));
            }
            // #966/#1171: the semi-index scanner accepts a number *span*
            // more leniently than RFC 8259 (leading zeros, a leading dot,
            // a malformed trailing shape like `1.2.3`). Sanitize via the
            // same fallback every other "raw bytes -> number" conversion
            // in this crate uses, instead of reformatting invalid text.
            // `JqCompat` is jq's own output convention, so the literal
            // model is jq's too (#2936) -- only the sanitized value of a
            // lenient span is printed from the double here, never a
            // preserved literal's.
            match OwnedValue::from_number_bytes::<JqSemantics>(raw) {
                OwnedValue::Int(i) => write!(out, "{i}"),
                OwnedValue::Float(f) => {
                    if f.is_finite() {
                        write!(out, "{f}")
                    } else {
                        out.write_str(nonfinite_display_string::<JqSemantics>(f))
                    }
                }
                // A leading-dot span (`.5`, `-.5`): `from_number_bytes`
                // preserves its spelling as a `NumberLiteral` here instead
                // of degrading to a plain `Float`, so trailing zeros
                // survive (`.500` -> `0.500`, not `0.5`) -- route it
                // through the same jq-compat reformatting a strictly-valid
                // span gets above, via the literal's own text.
                OwnedValue::NumberLiteral(_, literal) => {
                    out.write_str(&format_number_jq_compat(literal.as_bytes()))
                }
                _ => out.write_str("null"),
            }
        }
    }
}

/// Stream `cursors` as a single JSON array, one element per cursor, without
/// materializing an `OwnedValue` for any of them (#1576, mirroring #757's
/// `stream_json_sequence`/`stream_yaml_sequence` in `src/yaml/light.rs`).
///
/// Each cursor renders via its own `.value()` through [`stream_json_pretty`]
/// -- cursors need not be siblings or share an index, since this is what
/// renders a `map` chain's drained output (`LazySeq::drain_atomic`), where
/// each element is wherever its own sub-expression navigated to.
fn stream_json_sequence<W: AsRef<[u64]> + Clone, Out: core::fmt::Write>(
    cursors: &[JsonCursor<'_, W>],
    out: &mut Out,
    current_indent: usize,
    indent_spaces: usize,
    unit: char,
    sort_keys: bool,
    numbers: JsonConvention,
) -> StreamResult {
    if cursors.is_empty() {
        return Ok(out.write_str("[]")?);
    }
    out.write_char('[')?;
    let next_indent = current_indent + indent_spaces;
    let mut first = true;
    for cursor in cursors {
        if !first {
            out.write_char(',')?;
        }
        first = false;
        if indent_spaces > 0 {
            out.write_char('\n')?;
            write_json_indent(out, next_indent, unit)?;
        }
        stream_json_pretty(
            out,
            cursor.value(),
            next_indent,
            indent_spaces,
            unit,
            sort_keys,
            numbers,
            // Each cursor is its own independent root (#757's own
            // reasoning: "cursors need not be siblings"), so depth restarts
            // at 0 per element -- matching `JsonCursor::stream_json`'s own
            // top-level call, not a continuation of some shared ancestry.
            0,
        )?;
    }
    if indent_spaces > 0 {
        out.write_char('\n')?;
        write_json_indent(out, current_indent, unit)?;
    }
    Ok(out.write_char(']')?)
}

/// Write `spaces` copies of `unit` -- the JSON-to-JSON pretty writer's own
/// indent primitive, distinct from [`write_json_yaml_indent`] below (which
/// is JSON-to-*YAML*'s and always uses a space, since `JsonCursor::stream_yaml`
/// doesn't honor `--tab` either -- see its own doc comment).
fn write_json_indent<Out: core::fmt::Write>(
    out: &mut Out,
    spaces: usize,
    unit: char,
) -> core::fmt::Result {
    for _ in 0..spaces {
        out.write_char(unit)?;
    }
    Ok(())
}

// ============================================================================
// JSON to YAML Streaming Helpers
// ============================================================================

/// Stream a JSON value as YAML.
fn stream_json_as_yaml<W: AsRef<[u64]> + Clone, Out: core::fmt::Write>(
    out: &mut Out,
    value: StandardJson<'_, W>,
    current_indent: usize,
    indent_spaces: usize,
) -> StreamResult {
    match value {
        StandardJson::Null => Ok(out.write_str("null")?),
        StandardJson::Bool(b) => Ok(out.write_str(if b { "true" } else { "false" })?),
        StandardJson::Number(n) => {
            // Try integer first, then float
            if let Ok(i) = n.as_i64() {
                Ok(write!(out, "{i}")?)
            } else if let Ok(f) = n.as_f64() {
                if f.is_nan() || f.is_infinite() {
                    Ok(out.write_str(nonfinite_display_string::<YqSemantics>(f))?)
                } else {
                    Ok(write!(out, "{f}")?)
                }
            } else {
                Ok(out.write_str("null")?)
            }
        }
        StandardJson::String(s) => {
            // Decoded content, not `raw_bytes()` -- the latter includes the
            // source's surrounding quotes and escape sequences verbatim,
            // which would then get YAML-quoted a second time on top.
            // #1615: a scalar that will not decode raises a *diagnosable*
            // decode failure here. Before, this was a bare `core::fmt::Error`
            // -- which the CLI could only report as a generic write failure,
            // the "bare, undiagnosed abort" the design doc named as the reason
            // Stage 6 was deferred rather than attempted.
            let str_val = s.as_str().map_err(json_decode_failure)?;
            Ok(stream_json_string_as_yaml(out, &str_val)?)
        }
        StandardJson::Array(elements) => {
            if elements.is_empty() {
                return Ok(out.write_str("[]")?);
            }
            if indent_spaces == 0 {
                // Flow style
                out.write_char('[')?;
                let mut first = true;
                for elem in elements {
                    if !first {
                        out.write_str(", ")?;
                    }
                    first = false;
                    stream_json_as_yaml(out, elem, 0, 0)?;
                }
                Ok(out.write_char(']')?)
            } else {
                // Block style
                let mut first = true;
                for elem in elements {
                    if !first {
                        out.write_char('\n')?;
                        write_json_yaml_indent(out, current_indent)?;
                    }
                    first = false;
                    out.write_str("- ")?;
                    if is_json_container(&elem) {
                        out.write_char('\n')?;
                        write_json_yaml_indent(out, current_indent + indent_spaces)?;
                        stream_json_as_yaml(
                            out,
                            elem,
                            current_indent + indent_spaces,
                            indent_spaces,
                        )?;
                    } else {
                        stream_json_as_yaml(
                            out,
                            elem,
                            current_indent + indent_spaces,
                            indent_spaces,
                        )?;
                    }
                }
                Ok(())
            }
        }
        StandardJson::Object(fields) => {
            if fields.is_empty() {
                return Ok(out.write_str("{}")?);
            }
            if indent_spaces == 0 {
                // Flow style
                out.write_char('{')?;
                let mut first = true;
                for field in fields {
                    if !first {
                        out.write_str(", ")?;
                    }
                    first = false;
                    // Key -- decoded content, not `raw_bytes()` (see the
                    // scalar `String` arm above for why).
                    if let StandardJson::String(k) = field.key() {
                        let key_str = k.as_str().map_err(|_| core::fmt::Error)?;
                        stream_json_string_as_yaml(out, &key_str)?;
                    } else {
                        out.write_str("\"\"")?;
                    }
                    out.write_str(": ")?;
                    stream_json_as_yaml(out, field.value(), 0, 0)?;
                }
                Ok(out.write_char('}')?)
            } else {
                // Block style
                let mut first = true;
                for field in fields {
                    if !first {
                        out.write_char('\n')?;
                        write_json_yaml_indent(out, current_indent)?;
                    }
                    first = false;
                    // Key -- decoded content, not `raw_bytes()` (see the
                    // scalar `String` arm above for why).
                    if let StandardJson::String(k) = field.key() {
                        let key_str = k.as_str().map_err(|_| core::fmt::Error)?;
                        stream_json_string_as_yaml(out, &key_str)?;
                    } else {
                        out.write_str("\"\"")?;
                    }
                    out.write_char(':')?;
                    let val = field.value();
                    if is_json_container(&val) {
                        out.write_char('\n')?;
                        write_json_yaml_indent(out, current_indent + indent_spaces)?;
                        stream_json_as_yaml(
                            out,
                            val,
                            current_indent + indent_spaces,
                            indent_spaces,
                        )?;
                    } else {
                        out.write_char(' ')?;
                        stream_json_as_yaml(
                            out,
                            val,
                            current_indent + indent_spaces,
                            indent_spaces,
                        )?;
                    }
                }
                Ok(())
            }
        }
        // A *structural* malformation (#1194's class), not a decode failure:
        // left writing `null` exactly as before. #1615 is scoped to scalars
        // that fail to *decode*; whether this arm should raise too is the same
        // open question #1194 tracks for every other `StandardJson::Error`
        // site, and answering it here alone would split that decision across
        // two issues.
        StandardJson::Error(_) => Ok(out.write_str("null")?),
    }
}

/// Check if a JSON value is a non-empty container.
fn is_json_container<W: AsRef<[u64]> + Clone>(value: &StandardJson<'_, W>) -> bool {
    match value {
        StandardJson::Array(elements) => !elements.is_empty(),
        StandardJson::Object(fields) => !fields.is_empty(),
        _ => false,
    }
}

/// Write indentation spaces.
fn write_json_yaml_indent<Out: core::fmt::Write>(
    out: &mut Out,
    spaces: usize,
) -> core::fmt::Result {
    for _ in 0..spaces {
        out.write_char(' ')?;
    }
    Ok(())
}

/// Stream a JSON string value as YAML with smart quoting.
fn stream_json_string_as_yaml<Out: core::fmt::Write>(out: &mut Out, s: &str) -> core::fmt::Result {
    if s.is_empty() {
        return out.write_str("''");
    }

    // Check if we need quoting
    if needs_json_yaml_quoting(s) {
        stream_json_yaml_double_quoted(out, s)
    } else {
        out.write_str(s)
    }
}

/// Check if a JSON string needs quoting when output as YAML.
fn needs_json_yaml_quoting(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }

    let bytes = s.as_bytes();

    // Check first character
    let first = bytes[0];
    if matches!(
        first,
        b'-' | b'?'
            | b':'
            | b','
            | b'['
            | b']'
            | b'{'
            | b'}'
            | b'#'
            | b'&'
            | b'*'
            | b'!'
            | b'|'
            | b'>'
            | b'\''
            | b'"'
            | b'%'
            | b'@'
            | b'`'
    ) {
        return true;
    }

    // Check for leading/trailing whitespace
    if bytes[0] == b' ' || bytes[bytes.len() - 1] == b' ' {
        return true;
    }

    // Check for special values
    let lower = s.to_lowercase();
    if matches!(
        lower.as_str(),
        "null" | "~" | "true" | "false" | "yes" | "no" | "on" | "off" | ".inf" | "-.inf" | ".nan"
    ) {
        return true;
    }

    // Check if it looks like a number
    if looks_like_json_yaml_number(s) {
        return true;
    }

    // Check for special characters
    for b in bytes {
        if *b < 0x20 || *b == b':' || *b == b'#' {
            return true;
        }
    }

    false
}

/// Check if a string looks like a number.
fn looks_like_json_yaml_number(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }

    let bytes = s.as_bytes();
    let mut i = 0;

    // Optional sign
    if bytes[i] == b'-' || bytes[i] == b'+' {
        i += 1;
        if i >= bytes.len() {
            return false;
        }
    }

    // Must have at least one digit
    if !bytes[i].is_ascii_digit() {
        return false;
    }

    // Check remaining
    let mut has_dot = false;
    let mut has_exp = false;
    while i < bytes.len() {
        match bytes[i] {
            b'0'..=b'9' => {}
            b'.' if !has_dot && !has_exp => has_dot = true,
            b'e' | b'E' if !has_exp => {
                has_exp = true;
                if i + 1 < bytes.len() && (bytes[i + 1] == b'-' || bytes[i + 1] == b'+') {
                    i += 1;
                }
            }
            _ => return false,
        }
        i += 1;
    }

    true
}

/// Stream a double-quoted YAML string.
fn stream_json_yaml_double_quoted<Out: core::fmt::Write>(
    out: &mut Out,
    s: &str,
) -> core::fmt::Result {
    out.write_char('"')?;

    for ch in s.chars() {
        match ch {
            '"' => out.write_str("\\\"")?,
            '\\' => out.write_str("\\\\")?,
            '\n' => out.write_str("\\n")?,
            '\r' => out.write_str("\\r")?,
            '\t' => out.write_str("\\t")?,
            c if (c as u32) < 0x20 => {
                let b = c as u8;
                out.write_str("\\x")?;
                const HEX: &[u8; 16] = b"0123456789abcdef";
                out.write_char(HEX[(b >> 4) as usize] as char)?;
                out.write_char(HEX[(b & 0xf) as usize] as char)?;
            }
            c => out.write_char(c)?,
        }
    }

    out.write_char('"')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A node-dense document for the #2168 tests below: ~12 interest bits
    /// per 64-byte word in the number array, where the old fixed `rank / 8`
    /// seed for `ib_select1_from` drifted furthest from the true word, plus
    /// an object section so the walk covers keys and nested containers.
    fn dense_document_2168() -> Vec<u8> {
        let mut json = String::from("{\"numbers\":[");
        for i in 0..4000 {
            if i > 0 {
                json.push(',');
            }
            json.push_str(&i.to_string());
        }
        json.push_str("],\"objects\":[");
        for i in 0..800 {
            if i > 0 {
                json.push(',');
            }
            json.push_str(&format!(
                "{{\"a\":{i},\"b\":\"s{i}\",\"c\":[{i},null,true]}}"
            ));
        }
        json.push_str("]}");
        json.into_bytes()
    }

    /// Every open paren in BP, in document order, as cursors -- the same
    /// order `to_owned`, the #1755/#1953 validity walk and streaming output
    /// resolve positions in.
    fn document_order_cursors(root: JsonCursor<'_, Vec<u64>>) -> Vec<JsonCursor<'_, Vec<u64>>> {
        let mut out = Vec::new();
        let mut stack = vec![root];
        while let Some(c) = stack.pop() {
            out.push(c);
            // Push siblings first so children come off the stack before them.
            if let Some(next) = c.next_sibling() {
                stack.push(next);
            }
            if let Some(child) = c.first_child() {
                stack.push(child);
            }
        }
        out
    }

    /// `ib_select1_sequential` is `ib_select1_from` with a seed extrapolated
    /// from the previous call (#2168). The seed is advisory -- gallop-then-
    /// bisect is exact for any seed -- so the answer must match the pure
    /// binary search `ib_select1` for every rank, in every access order:
    /// forward, backward, and a scrambled order whose jumps land the seed
    /// well off the true word. The exact-repeat fast path is covered by the
    /// mechanism test below; the scrambled stride here revisits nothing.
    #[test]
    fn test_ib_select1_sequential_agrees_with_binary_search_in_every_order_2168() {
        let json = dense_document_2168();
        let index = JsonIndex::build(&json);
        let ones = index.ib_rank1(index.ib_len());
        assert!(
            ones > 5000,
            "document is not node-dense: {ones} interest bits"
        );

        for k in 0..ones {
            assert_eq!(
                index.ib_select1_sequential(k),
                index.ib_select1(k),
                "forward, rank {k}"
            );
        }
        for k in (0..ones).rev() {
            assert_eq!(
                index.ib_select1_sequential(k),
                index.ib_select1(k),
                "backward, rank {k}"
            );
        }
        // Deterministic scramble: a full-period LCG over 0..ones is overkill,
        // a large stride coprime with `ones` visits every rank once.
        let stride = 7919usize;
        let mut k = 0usize;
        for _ in 0..ones {
            assert_eq!(
                index.ib_select1_sequential(k),
                index.ib_select1(k),
                "scrambled, rank {k}"
            );
            k = (k + stride) % ones;
        }
        // Past the last set bit: both must decline, and the miss must not
        // poison the seed for the lookup after it.
        assert_eq!(index.ib_select1_sequential(ones), None);
        assert_eq!(index.ib_select1(ones), None);
        assert_eq!(index.ib_select1_sequential(0), index.ib_select1(0));
    }

    /// The cursor-level twin: a document-order walk's `text_position()`s
    /// through the seeded path equal the pure-binary-search positions, and
    /// so do positions asked *out* of order afterwards -- a `parent` step
    /// back to an ancestor, a re-walk from the root.
    #[test]
    fn test_text_position_document_walk_matches_random_access_2168() {
        let json = dense_document_2168();
        let index = JsonIndex::build(&json);
        let cursors = document_order_cursors(index.root(&json));
        assert!(cursors.len() > 5000);

        let expected: Vec<Option<usize>> = cursors
            .iter()
            .map(|c| index.ib_select1(index.bp().rank1(c.bp_pos)))
            .collect();
        for (c, want) in cursors.iter().zip(&expected) {
            assert_eq!(c.text_position(), *want);
        }
        // Backward: the root, then every ancestor-shaped jump the walk above
        // left the seed pointing past.
        for (c, want) in cursors.iter().zip(&expected).rev() {
            assert_eq!(c.text_position(), *want);
        }
        // Positions really are document order, so the seed was exercised
        // rather than trivially reset at every step.
        let positions: Vec<usize> = expected.iter().map(|p| p.unwrap()).collect();
        assert!(positions.windows(2).all(|w| w[0] < w[1]));
    }

    /// The mechanism, not just the answer (#2168). The two tests above pass
    /// for *any* seed policy, since gallop-then-bisect is exact whatever it
    /// starts from; this one counts `ib_rank` probes, which is the only
    /// thing the seed changes. The counter is compiled under `cfg(test)` as
    /// well as `select-stats`, so this runs in every `cargo test` -- a
    /// version gated on the feature alone would never have run in CI.
    ///
    /// On this document the fixed `rank / 8` estimate this replaced sits up
    /// to ~150 words from the true one and costs ~15 probes per lookup (a
    /// gallop of ~8 and a bisect of ~7); reverting to it, or seeding from
    /// the wrong unit (`pos` for a word), fails the bounds below.
    #[test]
    fn test_sequential_walk_costs_constant_probes_per_lookup_2168() {
        use crate::util::select_stats::{reset, snapshot, Site};
        let json = dense_document_2168();
        let index = JsonIndex::build(&json);
        let cursors = document_order_cursors(index.root(&json));

        // Document order: consecutive ranks, seed is the last answer's word.
        reset();
        for c in &cursors {
            let _ = c.text_position();
        }
        let forward = snapshot(Site::JsonIbSelectFrom);
        assert_eq!(forward.calls(), cursors.len() as u64);
        assert!(
            forward.max() <= 6,
            "a seeded document-order lookup should never need more than 6 probes; max was {}",
            forward.max()
        );

        // Reverse order: the seed extrapolates backward just as well -- a
        // draft that only reused the seed going forward fell back to
        // `rank / 8` here and paid ~15.
        reset();
        for c in cursors.iter().rev() {
            let _ = c.text_position();
        }
        let backward = snapshot(Site::JsonIbSelectFrom);
        assert!(
            backward.max() <= 6,
            "a seeded reverse-order lookup should never need more than 6 probes; max was {}",
            backward.max()
        );

        // An exact repeat answers from the cache: no probe is recorded at
        // all. A fresh index, so the root's rank is not already the cached
        // one from the reverse walk above.
        let fresh = JsonIndex::build(&json);
        let fresh_cursors = document_order_cursors(fresh.root(&json));
        reset();
        for c in &fresh_cursors {
            let _ = c.text_position();
            let _ = c.text_position();
        }
        let repeated = snapshot(Site::JsonIbSelectFrom);
        assert_eq!(
            repeated.calls(),
            fresh_cursors.len() as u64,
            "a repeated ask must not probe"
        );

        // Root then last, alternating -- the shape a base-node `key` between
        // two elements produces. Extrapolating from rank 0 at the document's
        // density lands near the last bit; a seed that stored the root's
        // word and galloped from there paid ~35.
        let ones = index.ib_rank1(index.ib_len());
        reset();
        for _ in 0..64 {
            let _ = index.ib_select1_sequential(0);
            let _ = index.ib_select1_sequential(ones - 1);
        }
        let alternating = snapshot(Site::JsonIbSelectFrom);
        assert!(
            alternating.max() <= 8,
            "a root/last alternation should stay within 8 probes; max was {}",
            alternating.max()
        );
    }

    #[test]
    fn test_build_index() {
        let json = br#"{"a": 1}"#;
        let index = JsonIndex::build(json);
        assert!(!index.bp().is_empty());
    }

    /// `DocumentCursor::line_comment`'s default impl (`document.rs`,
    /// unconditionally `None`) is what every JSON cursor uses - JSON has no
    /// comment syntax, so `JsonCursor` never overrides it. Its only
    /// production caller (the `line_comment` jq builtin) switched to
    /// `line_comment_checked` for YAML's #797 fix, which has its own
    /// JSON-side default (`Ok(None)`) - this pins the older, still-public
    /// default directly, since nothing else reaches it anymore.
    #[test]
    fn test_document_cursor_line_comment_default_is_none_for_json() {
        let json = br#"{"a": 1}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);
        assert_eq!(DocumentCursor::line_comment(&root), None);
    }

    /// `DocumentCursor::is_document_content`'s default (`document.rs`,
    /// unconditionally `false`) is what every JSON cursor uses -- JSON has no
    /// document-stream concept (no `---`/`...` boundaries), so `JsonCursor`
    /// never overrides it. Its only production caller
    /// (`to_owned_with_comments`, #2795 PR B) is only ever invoked with a
    /// `YamlCursor` in practice (`yq_runner.rs`), so this pins the default
    /// directly, mirroring `test_document_cursor_line_comment_default_is_none_for_json`
    /// just above.
    #[test]
    fn test_document_cursor_is_document_content_default_is_false_for_json() {
        let json = br#"{"a": 1}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);
        assert!(!DocumentCursor::is_document_content(&root));
    }

    /// `DocumentCursor::head_comment_raw`/`foot_comment_raw`'s defaults
    /// (`document.rs`, unconditionally empty) are what every JSON cursor
    /// uses -- JSON has no standalone-comment concept, so `JsonCursor` never
    /// overrides either. Both production call sites
    /// (`to_owned_with_comments_at_depth`, #2795 PR B review) gate on
    /// `document_has_standalone_comments`, whose own JSON default is
    /// `false`, so neither is ever reached for JSON in practice -- this pins
    /// both defaults directly, same rationale as the `is_document_content`
    /// test just above.
    #[test]
    fn test_document_cursor_head_and_foot_comment_raw_defaults_are_empty_for_json() {
        let json = br#"{"a": 1}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);
        assert!(DocumentCursor::head_comment_raw(&root).is_empty());
        assert!(DocumentCursor::foot_comment_raw(&root).is_empty());
    }

    /// `key_raw_source_span`'s non-`String` arm is unreachable via any real
    /// evaluation path -- `key_display_string_kind` only calls it once
    /// `string_decode_error()` is `Some`, which for this type is itself
    /// only `Some` on the `String` arm (#1642) -- but the trait method
    /// still needs an answer for every variant. Calls it directly on a
    /// non-string value to pin that contract: no span to show for
    /// something that was never a string in the first place.
    #[test]
    fn test_key_raw_source_span_is_none_for_non_string_value_1642() {
        let json = br"1";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();
        assert!(matches!(value, StandardJson::Number(_)));
        assert_eq!(value.key_raw_source_span(), None);
    }

    /// `stream_json_as_yaml`'s scalar-string arm previously read
    /// `raw_bytes()` (the source JSON bytes, quotes and escapes included)
    /// instead of the decoded `as_str()` content, so a plain string value
    /// got YAML-quoted a second time on top of its own JSON quoting --
    /// `"hi"` came out as the four-character string `"hi"` (quotes as
    /// content), which then needed its own YAML quoting: `"\"hi\""`.
    #[test]
    fn test_stream_json_as_yaml_string_value_not_double_quoted() {
        let json = br#"{"a": "hi"}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);
        let StandardJson::Object(fields) = root.value() else {
            panic!("expected object");
        };
        let (field, _) = fields.uncons().unwrap();
        let mut buf = String::new();
        stream_json_as_yaml(&mut buf, field.value(), 0, 0).unwrap();
        assert_eq!(buf, "hi");
    }

    /// Same bug, but for an object key rather than a value -- a separate
    /// (and separately buggy) code path in `stream_json_as_yaml`'s `Object`
    /// arm, covering both its flow-style and block-style branches.
    #[test]
    fn test_stream_json_as_yaml_object_key_not_double_quoted() {
        let json = br#"{"a": 1}"#;
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let mut flow = String::new();
        stream_json_as_yaml(&mut flow, value.clone(), 0, 0).unwrap();
        assert_eq!(flow, "{a: 1}");

        let mut block = String::new();
        stream_json_as_yaml(&mut block, value, 0, 2).unwrap();
        assert_eq!(block, "a: 1");
    }

    /// A key/value that itself needs YAML quoting (leading `:`, one of
    /// `needs_json_yaml_quoting`'s special first characters) must be quoted
    /// exactly once -- not left raw (ambiguous with a YAML mapping) and not
    /// double-quoted by the bug the tests above pin.
    #[test]
    fn test_stream_json_as_yaml_string_needing_quotes_is_quoted_once() {
        let json = br#"{":a": ":b"}"#;
        let index = JsonIndex::build(json);
        let value = index.root(json).value();
        let mut buf = String::new();
        stream_json_as_yaml(&mut buf, value, 0, 0).unwrap();
        assert_eq!(buf, r#"{":a": ":b"}"#);
    }

    /// #1064: an exponent overflowing to `f64::INFINITY` (JSON has no
    /// direct Infinity/NaN literal, so this is the only way to reach a
    /// non-finite float through this parser) spells with yq's YAML-native
    /// `.inf`/`-.inf`, not a bare `write!(out, "{f}")` -- this is the one
    /// call site into `nonfinite_display_string` this issue's dedup can't
    /// exercise through the CLI directly (the M2.5 streaming gate this
    /// function backs doesn't trigger on a plain top-level `.` query), so
    /// it's covered here at the unit level instead.
    ///
    /// Internal-consistency pin only, not oracle-verified: this exact
    /// input has no comparable real-tool behavior to check against (real
    /// yq hard-errors on a JSON `1e400` input entirely, "value out of
    /// range"; real jq, which never emits YAML, just echoes the literal
    /// text unchanged). The trigger condition and overall shape here
    /// predate #1064 -- this PR only changed which function computes the
    /// resulting string, not when it's called or what it does.
    #[test]
    fn test_stream_json_as_yaml_overflow_exponent_spells_infinity() {
        let json = br#"{"a": 1e400, "b": -1e400}"#;
        let index = JsonIndex::build(json);
        let value = index.root(json).value();
        let mut buf = String::new();
        stream_json_as_yaml(&mut buf, value, 0, 0).unwrap();
        assert_eq!(buf, "{a: .inf, b: -.inf}");
    }

    #[test]
    #[should_panic(expected = "up to u32::MAX")]
    #[cfg(target_pointer_width = "64")]
    fn test_from_parts_len_guard_panics() {
        // ib_len is a plain parameter, so exercising the #188 guard needs no
        // 4 GiB allocation.
        let _ = JsonIndex::from_parts(vec![], u32::MAX as usize + 1, vec![], 0);
    }

    #[test]
    fn test_root_cursor() {
        let json = br#"{"a": 1}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);
        assert_eq!(root.bp_position(), 0);
    }

    // #1576: `JsonCursor::stream_json` now implements `sort_keys` (via
    // `stream_json_pretty`'s `effective_fields`-then-sort, mirroring
    // `YamlCursor::stream_json_value`); `stream_yaml` still doesn't --
    // see `test_stream_yaml_rejects_sort_keys` just below, unchanged.
    #[test]
    fn test_stream_json_sort_keys_1576() {
        let json = br#"{"b": 1, "a": 2}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        let mut out = String::new();
        root.stream_json(
            &mut out,
            IndentSpec::COMPACT,
            true,
            JsonConvention::Preserve,
        )
        .unwrap();
        assert_eq!(out, r#"{"a":2,"b":1}"#);

        // sort_keys: false on the same input still takes the normal
        // (compact, raw-echo) path, order unchanged.
        out.clear();
        root.stream_json(
            &mut out,
            IndentSpec::COMPACT,
            false,
            JsonConvention::Preserve,
        )
        .unwrap();
        assert_eq!(out, r#"{"b": 1, "a": 2}"#);
    }

    // #1576 review: `stream_json`'s own stray-`,`-in-an-empty-container check
    // (`{,}`, `[,]`) only ever ran on the *root* value, because that was the
    // only place in the call chain still holding the container's own cursor
    // -- a nested empty container one level down silently "healed" into a
    // valid-looking `[]`/`{}` instead of erroring, unlike `-S`'s DOM path and
    // real jq (both reject `[,]` outright). `empty_container_gap_ok` closes
    // this by running the same check wherever `stream_json_pretty`'s array/
    // object arms still hold a child cursor for the container about to be
    // recursed into.
    #[test]
    fn test_stream_json_pretty_nested_empty_array_stray_comma_1576() {
        let json = br#"{"a": [,]}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        let mut out = String::new();
        let err = root
            .stream_json(
                &mut out,
                IndentSpec::COMPACT,
                false,
                JsonConvention::JqCompat,
            )
            .unwrap_err();
        assert!(matches!(err, StreamFailure::Decode(_)), "{err:?}");
    }

    #[test]
    fn test_stream_json_pretty_nested_empty_object_stray_comma_1576() {
        let json = br"[1, {,}]";
        let index = JsonIndex::build(json);
        let root = index.root(json);

        let mut out = String::new();
        let err = root
            .stream_json(
                &mut out,
                IndentSpec::COMPACT,
                false,
                JsonConvention::JqCompat,
            )
            .unwrap_err();
        assert!(matches!(err, StreamFailure::Decode(_)), "{err:?}");
    }

    // Control: a genuinely empty nested array/object (no stray token) must
    // keep streaming cleanly -- `empty_container_gap_ok` must not flag every
    // empty container, only one with unexplained content before its closer.
    #[test]
    fn test_stream_json_pretty_genuinely_empty_nested_containers_still_stream_1576() {
        let json = br#"{"a": [], "b": {}, "c": [1, {}, []]}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        let mut out = String::new();
        root.stream_json(
            &mut out,
            IndentSpec::COMPACT,
            false,
            JsonConvention::JqCompat,
        )
        .unwrap();
        assert_eq!(out, r#"{"a":[],"b":{},"c":[1,{},[]]}"#);
    }

    // #1576 coverage: `scalar_end_pos`'s `false`/`null` arms are the two
    // fixed-width literals whose byte length differs from `true`'s, and the
    // trailing-comma check (`[1, false,]`) is wrong by one byte if either
    // width is. The streaming array arm is the only caller, so the widths
    // are pinned through it: a container whose *last* element is the
    // literal under test, once well-formed and once with a stray trailing
    // comma that only lands on `]` if the width was right.
    #[test]
    fn test_stream_json_pretty_scalar_end_pos_false_and_null_1576() {
        for (json, expected) in [
            (&b"[1, false]"[..], "[1,false]"),
            (&b"[1, null]"[..], "[1,null]"),
            (&b"[1, true]"[..], "[1,true]"),
        ] {
            let index = JsonIndex::build(json);
            let root = index.root(json);
            let mut out = String::new();
            root.stream_json(
                &mut out,
                IndentSpec::COMPACT,
                false,
                JsonConvention::JqCompat,
            )
            .unwrap();
            assert_eq!(out, expected, "input {}", String::from_utf8_lossy(json));
        }

        for json in [&b"[1, false,]"[..], &b"[1, null,]"[..], &b"[1, true,]"[..]] {
            let index = JsonIndex::build(json);
            let root = index.root(json);
            let mut out = String::new();
            let err = root
                .stream_json(
                    &mut out,
                    IndentSpec::COMPACT,
                    false,
                    JsonConvention::JqCompat,
                )
                .unwrap_err();
            assert!(
                matches!(err, StreamFailure::Decode(_)),
                "input {}: {err:?}",
                String::from_utf8_lossy(json)
            );
        }
    }

    // #1576 coverage: `trailing_gap_ok`'s loop can also run off the end of
    // the text without ever meeting `close_char` -- an unterminated
    // container. No document input reaches it (the semi-index rejects
    // `[1` long before any streamer sees it), so the "ran out of text"
    // answer is pinned directly: it must be `false` (not the `true` a
    // whitespace-only tail gets), because a missing closer is exactly the
    // malformation the check exists to catch.
    #[test]
    fn test_trailing_gap_ok_unterminated_container_1576() {
        assert!(trailing_gap_ok(b"[1]", 2, b']'), "closer present");
        assert!(
            trailing_gap_ok(b"[1 \n ]", 2, b']'),
            "whitespace then closer"
        );
        assert!(!trailing_gap_ok(b"[1", 2, b']'), "text ends before closer");
        assert!(
            !trailing_gap_ok(b"[1  ", 2, b']'),
            "whitespace then end of text, still no closer"
        );
        assert!(!trailing_gap_ok(b"[1,]", 2, b']'), "stray comma");
    }

    // #1576 coverage: `write_json_number`'s `JqCompat` fallback chain. The
    // semi-index accepts a number *span* more leniently than RFC 8259, so a
    // trailing-dot mantissa (`1.`) reaches `from_number_bytes`.
    //
    // `1.` matches real jq (`[1.]` -> `[1]`, jq 1.7.1) via the plain-`Float`
    // fallback -- that spelling isn't preserved by either tool. `1.e999`
    // used to diverge the same way (degrading to an infinite `Float` and
    // printing the clamped `JqSemantics` stand-in instead of jq's own
    // `1E+999`), but #2220 added a third escape to `from_number_bytes`
    // (alongside the pre-existing leading-dot/leading-zero ones) that
    // preserves a trailing dot immediately before an exponent marker the
    // same way, so this now matches jq exactly via the `NumberLiteral`
    // reformatting arm below instead of the non-finite one.
    #[test]
    fn test_stream_json_number_invalid_span_float_fallback_1576() {
        for (json, expected) in [
            (&b"[1.]"[..], "[1]"),
            (&b"[-1.]"[..], "[-1]"),
            (&b"[1.e999]"[..], "[1E+999]"),
            (&b"[-1.e999]"[..], "[-1E+999]"),
        ] {
            let index = JsonIndex::build(json);
            let root = index.root(json);
            let mut out = String::new();
            root.stream_json(
                &mut out,
                IndentSpec::COMPACT,
                false,
                JsonConvention::JqCompat,
            )
            .unwrap();
            assert_eq!(out, expected, "input {}", String::from_utf8_lossy(json));
        }
    }

    // #1576 coverage: `write_json_string_pretty`'s escaping arms. The
    // zero-copy fast path only fires for a span with no `\`, so an escaped
    // string is what reaches the decode-and-re-encode tail -- and the arm
    // taken there is `numbers`'s, not the output format's.
    //
    // #2209 corrected this comment's original claim that `Preserve` is
    // "reachable from the CLI as `succinctly jq --preserve-input`": it is
    // not, and believing it was is what let jq mode render strings through
    // yq's table. `Preserve` reaches this writer only from `yq_runner.rs`
    // (yq mode, which always passes it); jq mode passes `JqCompat`, or
    // `JqPreserveInput` under `--preserve-input`. `\t` below is one of the
    // code points both tables agree on, so this test is blind to that
    // distinction by construction -- see
    // `test_stream_json_escape_table_is_mode_not_preserve_input_2209` for
    // the three code points where they differ.
    #[test]
    fn test_stream_json_escaped_string_both_conventions_1576() {
        let json = br#"{"a": "x\ty"}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        let mut out = String::new();
        root.stream_json(
            &mut out,
            IndentSpec::COMPACT,
            false,
            JsonConvention::JqCompat,
        )
        .unwrap();
        assert_eq!(out, r#"{"a":"x\ty"}"#);

        let mut out = String::new();
        root.stream_json(
            &mut out,
            IndentSpec::spaces(2),
            false,
            JsonConvention::Preserve,
        )
        .unwrap();
        assert_eq!(out, "{\n  \"a\": \"x\\ty\"\n}");
    }

    /// #2209: the escape table is a *mode* rule, not a `--preserve-input`
    /// one. `JqPreserveInput` must agree with `JqCompat` and differ from
    /// `Preserve` on exactly the three control-range code points the two
    /// tables disagree on (0x08, 0x0c, DEL) -- three of the five pinned by
    /// `jq::escape`'s own `conventions_differ_at_exactly_five_code_points`
    /// (the other two, U+2028/U+2029, are multi-byte and out of scope for
    /// this control-range check). Before #2209 jq
    /// mode's `--preserve-input` selected `Preserve` and so produced the yq
    /// column here, diverging from real jq on any navigation-only filter.
    #[test]
    fn test_stream_json_escape_table_is_mode_not_preserve_input_2209() {
        // The backslashes keep `write_json_string_pretty`'s zero-copy echo
        // from firing, so the re-encoding tail (the arm under test) runs.
        let json = br#"{"a": "\b\f\u007f"}"#;
        let index = JsonIndex::build(json);
        let render = |numbers| {
            let mut out = String::new();
            index
                .root(json)
                .stream_json(&mut out, IndentSpec::spaces(2), false, numbers)
                .unwrap();
            out
        };

        let jq = render(JsonConvention::JqCompat);
        let preserve_input = render(JsonConvention::JqPreserveInput);
        let yq = render(JsonConvention::Preserve);

        // jq's table: short forms kept, DEL escaped.
        assert!(jq.contains("\\b"), "jq lost the short form: {jq}");
        assert!(jq.contains("\\f"), "jq lost the short form: {jq}");
        assert!(jq.contains("\\u007f"), "jq left DEL raw: {jq}");
        // yq's table: long forms only, DEL left raw.
        assert!(yq.contains("\\u0008"), "yq table: {yq}");
        assert!(yq.contains("\\u000c"), "yq table: {yq}");
        assert!(yq.contains('\u{7f}'), "yq must leave DEL raw: {yq}");

        assert_eq!(
            preserve_input, jq,
            "--preserve-input must keep jq mode's own escape table"
        );
        assert_ne!(
            preserve_input, yq,
            "--preserve-input must never adopt yq's escape table"
        );
    }

    /// #2209: the fix must not cost `--preserve-input` its actual job.
    /// Number spellings still survive verbatim under both preserving
    /// conventions, and are canonicalized only by `JqCompat`.
    #[test]
    fn test_stream_json_preserve_input_still_preserves_numbers_2209() {
        let json = br#"{"n": 1e100}"#;
        let index = JsonIndex::build(json);
        let render = |numbers| {
            let mut out = String::new();
            index
                .root(json)
                .stream_json(&mut out, IndentSpec::spaces(2), false, numbers)
                .unwrap();
            out
        };

        assert!(render(JsonConvention::Preserve).contains("1e100"));
        assert!(render(JsonConvention::JqPreserveInput).contains("1e100"));
        assert!(render(JsonConvention::JqCompat).contains("1E+100"));
    }

    /// The *ground truth* `is_canonical_compact_jq_span` must agree with,
    /// for the differential tests below (#2608): the re-render path
    /// `stream_json` itself falls through to once neither of its two
    /// raw-echo branches fires, called directly rather than through
    /// `stream_json` -- calling `stream_json` here instead would make the
    /// property trivially agree with itself now that its new echo branch
    /// *is* `is_canonical_compact_jq_span`, defeating the whole point of
    /// the check. Mirrors `stream_json`'s own tail exactly (its
    /// `empty_container_gap_ok` guard, then `stream_json_pretty` at
    /// `IndentSpec::COMPACT`/unsorted/`JqCompat`). `None` for empty input
    /// (skips even building an index -- a zero-length document never
    /// reaches this far in the real CLI path either, `find_json_values`
    /// yields no values for it) and for anything `stream_json_pretty`
    /// itself rejects.
    ///
    /// Only the `is_canonical_compact_jq_span(doc) == true ==>
    /// round-trips` direction is load-bearing (see the two differential
    /// tests below) -- **not** the converse. Testing found a real,
    /// pre-existing gap on the converse side, unrelated to #2608's own
    /// code: `write_json_string_pretty`'s zero-copy echo
    /// (`!escaped && !del_unsafe`) only special-cases a raw DEL byte
    /// (`del_unsafe`, #2591); a raw *other* control byte (e.g. `0x01`)
    /// left unescaped in the source slips through that same zero-copy
    /// check uncaught, so this ground-truth renderer echoes it verbatim
    /// -- where real `/usr/bin/jq 1.7.1` rejects the document outright
    /// (`Invalid string: control characters from U+0000 through U+001F
    /// must be escaped`). `is_canonical_compact_jq_span` still answers
    /// `false` for that document (correctly, matching real jq's refusal
    /// to treat it as valid input at all, not this renderer's own
    /// leniency), so the gap never reaches the new echo path -- but it
    /// does mean this helper cannot be trusted as a "definitely not
    /// canonical" oracle, only a "definitely canonical" one. Not fixed
    /// here: out of scope for #2608, which only adds an *echo* fast path
    /// and must not change the unrelated re-render path's own behavior.
    fn render_compact_jq_by_rerender(bytes: &[u8]) -> Option<Vec<u8>> {
        if bytes.is_empty() {
            return None;
        }
        let index = JsonIndex::build(bytes);
        let root = index.root(bytes);
        let value = root.value();
        if !empty_container_gap_ok(&root, &value) {
            return None;
        }
        let mut buf = String::new();
        stream_json_pretty(
            &mut buf,
            value,
            0,
            0,
            ' ',
            false,
            JsonConvention::JqCompat,
            0,
        )
        .ok()?;
        Some(buf.into_bytes())
    }

    /// Wraps `content` in `"`...`"` to make a JSON string token, from
    /// explicit byte literals rather than a string/byte-string literal
    /// containing backslash-escape *text* (#2608 review: an earlier draft
    /// of the corpus below typed those directly and two entries came out
    /// wrong in ways that were not obvious to spot by eye -- an intended
    /// `A` source spelling silently ended up as the literal letter
    /// `A`, and an intended `\t` ended up as an actual raw tab byte).
    /// A `&[u8]` of individual byte literals (`b'\\'`, `b'u'`, `b'0'`, ...)
    /// has no such failure mode: every element is either a plain ASCII
    /// byte or the single well-known escape `b'\\'`, so what is on the
    /// page is exactly what ends up in the compiled byte sequence.
    fn json_string_token(content: &[u8]) -> Vec<u8> {
        let mut v = Vec::with_capacity(content.len() + 2);
        v.push(b'"');
        v.extend_from_slice(content);
        v.push(b'"');
        v
    }

    /// The whole safety argument for `is_canonical_compact_jq_span`, run
    /// against a hand-picked corpus covering every rule in its own doc
    /// comment, split into two groups asserted differently:
    ///
    /// - **Canonical** entries get the full, strong check: the checker
    ///   must accept them (`is_canonical_compact_jq_span(doc)`) *and* the
    ///   ground-truth re-render must agree byte-for-byte
    ///   (`render_compact_jq_by_rerender(doc) == Some(doc)`) -- e.g.
    ///   live-checked against `/usr/bin/jq 1.7.1`: `1.0`/`0.10` land here
    ///   because jq preserves trailing zeros, despite looking
    ///   superficially like a reformat target.
    /// - **Non-canonical** entries only assert the checker *declines*
    ///   them (`!is_canonical_compact_jq_span(doc)`) -- deliberately not
    ///   the round-trip side. That is the direction that actually
    ///   protects the new echo path (a decline can never cause wrong
    ///   output, only a missed fast path), and two entries in this group
    ///   (a raw control byte, an unterminated string) hit a pre-existing,
    ///   unrelated leniency in the hand-rolled test-only ground truth --
    ///   see `render_compact_jq_by_rerender`'s own doc comment -- that
    ///   would fail a round-trip assertion for reasons that have nothing
    ///   to do with `is_canonical_compact_jq_span` being wrong.
    // The individual-byte-literal arrays below are deliberate, not an
    // oversight clippy's `byte_char_slices` should collapse into a
    // terser `b"..."` literal -- that terser form containing literal
    // backslash-escape *text* is exactly what `json_string_token`'s own
    // doc comment explains went wrong, twice, in an earlier draft.
    #[test]
    #[allow(clippy::byte_char_slices)]
    fn is_canonical_compact_jq_span_agrees_with_rerender_2608() {
        // All 7 short escapes, one string: `"a\tb\nc\rd\be\ff\"g\\h"`.
        let all_short_escapes = json_string_token(&[
            b'a', b'\\', b't', b'b', b'\\', b'n', b'c', b'\\', b'r', b'd', b'\\', b'b', b'e',
            b'\\', b'f', b'f', b'\\', b'"', b'g', b'\\', b'\\', b'h',
        ]);
        // Every control code with no short form, spelled canonically.
        let u00xx_no_short_form = json_string_token(&[
            b'\\', b'u', b'0', b'0', b'0', b'0', b'\\', b'u', b'0', b'0', b'0', b'7', b'\\', b'u',
            b'0', b'0', b'0', b'b', b'\\', b'u', b'0', b'0', b'1', b'f', b'\\', b'u', b'0', b'0',
            b'7', b'f',
        ]);
        // `"a\/b"` -- jq's writer never escapes solidus.
        let escaped_solidus = json_string_token(&[b'a', b'\\', b'/', b'b']);
        // `"A"` -- decodes to 'A', which the writer would leave raw.
        let escaped_u0041 = json_string_token(&[b'\\', b'u', b'0', b'0', b'4', b'1']);
        // U+0008 spelled via \u -- decodes to a control code that *has* a short form
        // (`\b`); the writer would use that, never this spelling.
        let escaped_u0008_has_short_form =
            json_string_token(&[b'\\', b'u', b'0', b'0', b'0', b'8']);
        // uppercase hex digit, never emitted by the writer
        // (lowercase only).
        let escaped_uppercase_hex = json_string_token(&[b'\\', b'u', b'0', b'0', b'1', b'B']);
        // `"\uD800"` -- a lone surrogate half, outside the control/DEL set
        // entirely.
        let escaped_lone_surrogate = json_string_token(&[b'\\', b'u', b'D', b'8', b'0', b'0']);
        // Raw (unescaped) bytes the writer always escapes if present.
        let raw_control_byte = json_string_token(&[0x01]);
        let raw_del_byte = json_string_token(&[0x7f]);
        // Invalid UTF-8: a bare continuation byte, and a truncated 2-byte
        // lead with nothing (a valid continuation byte) after it.
        let invalid_utf8_bare_continuation = json_string_token(&[0x80]);
        let invalid_utf8_truncated_lead = json_string_token(&[0xC2]);

        let canonical: Vec<Vec<u8>> = vec![
            // -- scalars --
            b"0".to_vec(),
            b"-7".to_vec(),
            b"42".to_vec(),
            b"1000".to_vec(),
            b"-0".to_vec(),
            b"1.0".to_vec(), // live-verified: `1.0` -> `1.0` (jq preserves trailing zeros)
            b"0.10".to_vec(), // live-verified: `0.10` -> `0.10`
            b"12.345".to_vec(),
            b"9.999".to_vec(),
            b"true".to_vec(),
            b"false".to_vec(),
            b"null".to_vec(),
            b"[]".to_vec(),
            b"{}".to_vec(),
            b"\"\"".to_vec(),
            b"\"hello world\"".to_vec(),
            b"\" \"".to_vec(), // a literal space is plain printable content
            // -- escapes --
            all_short_escapes,
            u00xx_no_short_form,
            // -- nesting, mixed types, unsorted keys kept as-is --
            br#"{"b":2,"a":[1,2,3],"c":{"x":"y"},"d":null,"e":true,"f":false}"#.to_vec(),
            br#"[1,-2,3.5,0.25,true,false,null,"x",{"k":"v"}]"#.to_vec(),
        ];

        let non_canonical: Vec<Vec<u8>> = vec![
            // -- number spellings the formatter would rewrite --
            b"1e2".to_vec(),
            b"1E5".to_vec(),
            b"007".to_vec(),
            b"+7".to_vec(),
            b"1.".to_vec(),
            b"-000".to_vec(), // formatter rewrites to `-0`, not identity
            b"01.5".to_vec(),
            b"1.2.3".to_vec(),
            b"-".to_vec(),
            br"[1e2]".to_vec(),
            br#"{"n":007}"#.to_vec(),
            // -- string escape rules --
            escaped_solidus,
            escaped_u0041,
            escaped_u0008_has_short_form,
            escaped_uppercase_hex,
            escaped_lone_surrogate,
            raw_control_byte, // pre-existing renderer leniency -- see this test's own doc comment
            raw_del_byte,
            invalid_utf8_bare_continuation,
            invalid_utf8_truncated_lead,
            b"\"abc".to_vec(), // unterminated string -- same pre-existing leniency
            // -- duplicate keys, first/middle/last position --
            br#"{"a":1,"a":2}"#.to_vec(),
            br#"{"x":0,"a":1,"a":2,"b":3}"#.to_vec(),
            br#"{"a":1,"b":2,"a":3}"#.to_vec(),
            // -- structural malformation (#1677/#1676/#2594-style) --
            b"[1,,2]".to_vec(),
            b"{,}".to_vec(),
            b"[,]".to_vec(),
            b"[1,2,]".to_vec(),
            br#"{"a":1,}"#.to_vec(),
            br#"{"a"1}"#.to_vec(),
            br#"{"a" :1}"#.to_vec(), // valid JSON, but not *compact* -- extra space before `:`
            br"[1, 2]".to_vec(),     // valid JSON, but not compact -- space after `,`
            b"123abc".to_vec(),      // trailing garbage past the number token
            b" 123".to_vec(),        // leading garbage before the value starts
            b"".to_vec(),            // empty input
        ];

        for doc in &canonical {
            assert!(
                is_canonical_compact_jq_span(doc),
                "expected canonical: {:?} (as text: {})",
                doc,
                String::from_utf8_lossy(doc)
            );
            assert_eq!(
                render_compact_jq_by_rerender(doc).as_deref(),
                Some(doc.as_slice()),
                "expected round-trip: {:?} (as text: {})",
                doc,
                String::from_utf8_lossy(doc)
            );
        }
        for doc in &non_canonical {
            assert!(
                !is_canonical_compact_jq_span(doc),
                "expected non-canonical: {:?} (as text: {})",
                doc,
                String::from_utf8_lossy(doc)
            );
        }
        // Acceptance is a decision of `stream_json`'s echo branch, not of
        // the predicate alone -- it also resolves the node's start, lets
        // the scan report the end, and runs `from_utf8` over the result
        // (#2608 follow-up). Drive that branch over the same corpus and
        // require it to agree with the fall-through re-render for every
        // entry in *both* groups, which is the only property the echo is
        // ever allowed to have.
        for doc in canonical.iter().chain(non_canonical.iter()) {
            assert_eq!(
                render_compact_jq_through_gate(doc),
                render_compact_jq_by_rerender(doc)
                    .map(|b| String::from_utf8_lossy(&b).into_owned()),
                "gate vs re-render: {:?} (as text: {})",
                doc,
                String::from_utf8_lossy(doc)
            );
        }
    }

    /// The render `stream_json`'s own #2608 echo branch produces for the
    /// document's root value -- i.e. the gate exactly as the CLI reaches
    /// it (compact, unsorted, `JqCompat`), scan and `from_utf8` included --
    /// or `None` when it fails. The companion to
    /// `render_compact_jq_by_rerender` above, which deliberately calls the
    /// *fall-through* path instead so the two can be compared.
    fn render_compact_jq_through_gate(doc: &[u8]) -> Option<String> {
        let index = JsonIndex::build(doc);
        let root = index.root(doc);
        let mut out = String::new();
        root.stream_json(
            &mut out,
            IndentSpec::COMPACT,
            false,
            JsonConvention::JqCompat,
        )
        .ok()?;
        Some(out)
    }

    /// The echo branch no longer asks the cursor for its raw span; it takes
    /// the node's start and lets the canonical scan report the end
    /// (`canonical_compact_jq_span_end`, #2608 follow-up). That makes
    /// "stops exactly at the value's last byte" a property of the *scanner*
    /// rather than of `text_range`, so it needs its own pin: every document
    /// below has something after the root value that must not be echoed --
    /// the trailing newline every real file ends with, a second top-level
    /// document (`jq` reads a stream, not one value), and outright garbage.
    ///
    /// Asserted two ways: against the literal expected bytes, and against
    /// the unchanged re-render path, which is the behaviour the echo is
    /// only ever allowed to be a faster spelling of.
    #[test]
    fn canonical_echo_stops_at_the_value_end_2608() {
        for (doc, want) in [
            (&b"{\"a\":1}\n"[..], "{\"a\":1}"),
            (&b"[1,2,3]\n\n"[..], "[1,2,3]"),
            (&b"\"hi\"\n"[..], "\"hi\""),
            (&b"12\n"[..], "12"),
            (&b"true\n"[..], "true"),
            (&b"null\n"[..], "null"),
            // A second top-level value: `jq` treats the input as a stream,
            // and this cursor is the *first* value in it.
            (&b"{\"a\":1} {\"b\":2}"[..], "{\"a\":1}"),
            (&b"[1] [2]"[..], "[1]"),
            (&b"1 2"[..], "1"),
            // Trailing garbage: the token still ends where its own grammar
            // says it does.
            (&b"{\"a\":1}xyz"[..], "{\"a\":1}"),
            (&b"12xyz"[..], "12"),
            (&b"truexyz"[..], "true"),
        ] {
            assert_eq!(
                render_compact_jq_through_gate(doc).as_deref(),
                Some(want),
                "gate echo for {:?}",
                String::from_utf8_lossy(doc)
            );
            assert_eq!(
                render_compact_jq_by_rerender(doc)
                    .as_deref()
                    .map(|b| String::from_utf8_lossy(b).into_owned()),
                Some(want.to_string()),
                "re-render for {:?}",
                String::from_utf8_lossy(doc)
            );
        }
    }

    /// Same load-bearing direction as the corpus test above --
    /// `is_canonical_compact_jq_span(doc) == true ==>` `doc` round-trips
    /// through the re-render unchanged -- swept over multi-byte-UTF-8-and-
    /// edge-byte documents a seeded generator builds, rather than a
    /// hand-picked list (the fuzz-alphabet rule in `CLAUDE.md`: a fuzzer
    /// that only ever emits ASCII cannot find a bug in a UTF-8-aware
    /// checker). One direction only, not the full biconditional the
    /// corpus test above checks where it can afford to: the generator
    /// deliberately includes a raw (unescaped) control byte among its
    /// ingredients, which can trip the same pre-existing, unrelated
    /// `render_compact_jq_by_rerender` gap that function's own doc
    /// comment describes (a document `is_canonical_compact_jq_span`
    /// correctly declines can still, by that gap, "round-trip" through
    /// the renderer's own leniency) -- asserting the converse here would
    /// make this test fail on a bug this change neither introduces nor
    /// is responsible for fixing.
    // See the corpus test's identical `#[allow]` just above for why:
    // the individual-byte-literal arrays are the deliberate, safe
    // choice here, not something clippy's suggested `b"..."` literal
    // should replace.
    #[test]
    #[allow(clippy::byte_char_slices)]
    fn is_canonical_compact_jq_span_fuzz_2608() {
        use rand::{RngExt, SeedableRng};
        use rand_chacha::ChaCha8Rng;

        // Ingredients a generated string's content is assembled from --
        // deliberately mixing plain ASCII, multi-byte UTF-8 (2/3/4-byte
        // sequences), every canonical escape spelling, and every
        // non-canonical one this function's own doc comment calls out.
        // Every ingredient is `&[u8]` (not `&str`) built from explicit
        // byte literals for the escape sequences -- #2608 review: an
        // earlier draft used `&str` raw-string literals containing
        // backslash-escape *text* and several came out wrong in ways
        // that were not obvious to spot by eye (see
        // `json_string_token`'s own doc comment above, same lesson).
        const STRING_INGREDIENTS: &[&[u8]] = &[
            b"abcXYZ019 !#$%&()*+-.:;<=>?@[]^_`{|}~",
            "日本語".as_bytes(), // 3-byte sequences
            "café".as_bytes(),   // 2-byte (é)
            "🎉🎊".as_bytes(),   // 4-byte sequences
            "Ω∑".as_bytes(),     // more 2/3-byte mixes
            &[b'\\', b't'],      // canonical short escape
            &[b'\\', b'n'],
            &[b'\\', b'r'],
            &[b'\\', b'b'],
            &[b'\\', b'f'],
            &[b'\\', b'"'],
            &[b'\\', b'\\'],
            &[b'\\', b'u', b'0', b'0', b'0', b'0'], // canonical \u00xx (no short form)
            &[b'\\', b'u', b'0', b'0', b'0', b'7'],
            &[b'\\', b'u', b'0', b'0', b'0', b'b'],
            &[b'\\', b'u', b'0', b'0', b'1', b'f'],
            &[b'\\', b'u', b'0', b'0', b'7', b'f'],
            &[b'\\', b'/'],                         // non-canonical: solidus escape
            &[b'\\', b'u', b'0', b'0', b'4', b'1'], // non-canonical: should be raw 'A'
            &[b'\\', b'u', b'0', b'0', b'0', b'8'], // non-canonical: has short form \b
            &[b'\\', b'u', b'D', b'8', b'0', b'0'], // non-canonical: lone surrogate
            &[b'\\', b'u', b'0', b'0', b'1', b'B'], // non-canonical: uppercase hex
            &[0x01],                                // raw unescaped control byte
            &[0x7f],                                // raw unescaped DEL byte
        ];

        // Raw (possibly invalid-UTF-8) byte-level ingredients, appended
        // directly rather than through a `&str`.
        const RAW_BYTE_INGREDIENTS: &[&[u8]] = &[&[0x80], &[0xC2], &[0xFF], &[0xE0, 0x80]];

        const NUMBER_INGREDIENTS: &[&str] = &[
            "0", "1", "-1", "42", "-7", "1.0", "0.10", "-0", "12.345", "9.999", "1000", "1e2",
            "1E5", "007", "+7", "1.", "01.5", "-",
        ];

        fn gen_string_content(rng: &mut ChaCha8Rng, out: &mut Vec<u8>) {
            let picks = rng.random_range(0..4);
            for _ in 0..picks {
                if rng.random_bool(0.15) {
                    let raw = RAW_BYTE_INGREDIENTS[rng.random_range(0..RAW_BYTE_INGREDIENTS.len())];
                    out.extend_from_slice(raw);
                } else {
                    let s = STRING_INGREDIENTS[rng.random_range(0..STRING_INGREDIENTS.len())];
                    out.extend_from_slice(s);
                }
            }
        }

        fn gen_string(rng: &mut ChaCha8Rng, out: &mut Vec<u8>) {
            out.push(b'"');
            gen_string_content(rng, out);
            out.push(b'"');
        }

        fn gen_number(rng: &mut ChaCha8Rng, out: &mut Vec<u8>) {
            let n = NUMBER_INGREDIENTS[rng.random_range(0..NUMBER_INGREDIENTS.len())];
            out.extend_from_slice(n.as_bytes());
        }

        fn gen_value(rng: &mut ChaCha8Rng, depth: u32, out: &mut Vec<u8>) {
            if depth == 0 || rng.random_bool(0.35) {
                match rng.random_range(0..6) {
                    0 => gen_string(rng, out),
                    1 => gen_number(rng, out),
                    2 => out.extend_from_slice(b"true"),
                    3 => out.extend_from_slice(b"false"),
                    4 => out.extend_from_slice(b"null"),
                    _ => gen_string(rng, out),
                }
                return;
            }
            if rng.random_bool(0.5) {
                gen_object(rng, depth, out);
            } else {
                gen_array(rng, depth, out);
            }
        }

        fn gen_array(rng: &mut ChaCha8Rng, depth: u32, out: &mut Vec<u8>) {
            out.push(b'[');
            let n = rng.random_range(0..4);
            for i in 0..n {
                if i > 0 {
                    out.push(b',');
                }
                gen_value(rng, depth - 1, out);
            }
            out.push(b']');
        }

        fn gen_object(rng: &mut ChaCha8Rng, depth: u32, out: &mut Vec<u8>) {
            out.push(b'{');
            let n = rng.random_range(0..4);
            // Occasionally force a repeated key, deliberately -- the
            // duplicate-key rule needs positive coverage from the
            // generator too, not just the hand-picked corpus. `key_0`
            // captures index 0's *exact* bytes (as written to `out`, not
            // regenerated from a second RNG -- a second draw would almost
            // certainly produce a different string, defeating the whole
            // point) so a later index can push a byte-for-byte copy,
            // guaranteeing a real duplicate rather than a merely-possible
            // one.
            let dup_key: Option<usize> = if n > 1 && rng.random_bool(0.25) {
                Some(rng.random_range(1..n))
            } else {
                None
            };
            let mut key_0: Vec<u8> = Vec::new();
            for i in 0..n {
                if i > 0 {
                    out.push(b',');
                }
                if dup_key == Some(i) {
                    out.extend_from_slice(&key_0);
                } else {
                    let key_start = out.len();
                    gen_string(rng, out);
                    if i == 0 {
                        key_0 = out[key_start..].to_vec();
                    }
                }
                out.push(b':');
                gen_value(rng, depth - 1, out);
            }
            out.push(b'}');
        }

        let mut mismatches = Vec::new();
        let mut canonical_count = 0usize;
        const ITERATIONS: u64 = 2000;
        for seed in 0..ITERATIONS {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let mut doc = Vec::new();
            gen_value(&mut rng, 3, &mut doc);

            let checker_says_canonical = is_canonical_compact_jq_span(&doc);
            let rerendered = render_compact_jq_by_rerender(&doc);
            let actually_round_trips = rerendered.as_deref() == Some(doc.as_slice());
            if checker_says_canonical {
                canonical_count += 1;
                // One direction only -- see this test's own doc comment
                // for why the converse is not asserted here.
                if !actually_round_trips {
                    mismatches.push((seed, doc.clone()));
                }
            }
            // The whole echo *branch*, not just the predicate: it must
            // produce exactly what the fall-through re-render would, on
            // every generated document regardless of which side of the
            // gate it lands. This is also what drives the branch's
            // `debug_assert_eq!` that the scan's end agrees with
            // `text_range`'s over the whole sweep (#2608 follow-up).
            if render_compact_jq_through_gate(&doc)
                != rerendered.map(|b| String::from_utf8_lossy(&b).into_owned())
            {
                mismatches.push((seed, doc.clone()));
            }
        }

        assert!(
            mismatches.is_empty(),
            "{} of {ITERATIONS} generated documents were certified canonical but did not round-trip; first few: {:#?}",
            mismatches.len(),
            mismatches
                .iter()
                .take(5)
                .map(|(seed, doc)| (*seed, String::from_utf8_lossy(doc).into_owned()))
                .collect::<Vec<_>>()
        );
        // Sanity: the generator must actually exercise the fast path some
        // of the time, or this whole test would pass vacuously by only
        // ever generating non-canonical documents.
        assert!(
            canonical_count > ITERATIONS as usize / 20,
            "generator produced too few canonical documents to be a meaningful sweep: {canonical_count}/{ITERATIONS}"
        );
    }

    // #1576 coverage: `stream_json_sequence`'s empty case. `map(...)` over
    // an empty array (or one whose every element a `select` dropped) drains
    // to zero cursors, and the writer still owes the caller a well-formed
    // `[]` -- not the bare `[` + `]` the general loop would emit around no
    // elements at a non-zero indent.
    #[test]
    fn test_json_cursor_streams_empty_sequence_json_1576() {
        let cursors: [JsonCursor<'_, Vec<u64>>; 0] = [];
        for indent in [IndentSpec::COMPACT, IndentSpec::spaces(2)] {
            let mut out = String::new();
            JsonCursor::stream_sequence_json(
                &cursors,
                &mut out,
                indent,
                false,
                JsonConvention::Preserve,
            )
            .unwrap();
            assert_eq!(out, "[]");
        }
    }

    #[test]
    fn test_stream_yaml_rejects_sort_keys() {
        let json = br#"{"b": 1, "a": 2}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        let mut out = String::new();
        assert!(root
            .stream_yaml(&mut out, IndentSpec::COMPACT, true)
            .is_err());

        // sort_keys: false on the same input still takes the normal path
        // (exact formatting of JSON->YAML conversion is covered elsewhere;
        // this just confirms the new guard doesn't reject the false case).
        out.clear();
        assert!(root
            .stream_yaml(&mut out, IndentSpec::COMPACT, false)
            .is_ok());
        assert!(!out.is_empty());
    }

    /// #1615 converted every arm of `stream_json_as_yaml` to `StreamResult`,
    /// but only its string and nested-container arms had any coverage --
    /// integers, floats, empty containers and the block-style closers were all
    /// reformatted blind. This walks one document through every arm, in both
    /// block and flow style, so a future edit to any of them is caught.
    ///
    /// The `StandardJson::Error` arm is deliberately not exercised: it still
    /// writes `null` for a *structural* malformation (#1194's class, not a
    /// decode failure), and reaching it needs a malformed index rather than a
    /// document this constructor can build.
    #[test]
    fn test_stream_yaml_covers_every_value_arm_1615() {
        let json = br#"{"i": 42, "neg": -7, "f": 1.5, "t": true, "fa": false,
                        "n": null, "s": "x", "ea": [], "eo": {},
                        "arr": [1, "two", [3], {"k": 4}],
                        "obj": {"nested": {"deep": [5]}}}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        // Block style (indent 2) and flow style (COMPACT) take different
        // branches for every container arm, so both are walked.
        for indent in [IndentSpec::spaces(2), IndentSpec::COMPACT] {
            let mut out = String::new();
            root.stream_yaml(&mut out, indent, false)
                .expect("a fully decodable document must stream");
            for expected in ["42", "-7", "1.5", "true", "false", "null", "x", "[]", "{}"] {
                assert!(
                    out.contains(expected),
                    "missing {expected:?} in {out:?} (indent {indent:?})"
                );
            }
        }
    }

    #[test]
    fn test_cursor_line_column() {
        // JsonCursor previously had no `line()`/`column()` at all — it fell
        // through to `DocumentCursor`'s `0` default even at the root (#532).
        let json = b"{\n  \"a\": 1,\n  \"b\": 2\n}";
        let index = JsonIndex::build(json);
        let root = index.root(json);

        assert_eq!(root.line(), 1);
        assert_eq!(root.column(), 1);

        let StandardJson::Object(fields) = root.value() else {
            panic!("expected object");
        };
        let b_cursor = fields.find_cursor("b").unwrap().expect("field b");
        assert_eq!(b_cursor.line(), 3);
        assert_eq!(b_cursor.column(), 8);
    }

    #[test]
    fn test_empty_object() {
        let json = br"{}";
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::Object(fields) => {
                assert!(fields.is_empty());
            }
            _ => panic!("expected object"),
        }
    }

    #[test]
    fn test_empty_array() {
        let json = br"[]";
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::Array(elements) => {
                assert!(elements.is_empty());
            }
            _ => panic!("expected array"),
        }
    }

    #[test]
    fn test_simple_values() {
        // Test boolean true
        let json = b"true";
        let index = JsonIndex::build(json);
        let root = index.root(json);
        assert!(matches!(root.value(), StandardJson::Bool(true)));

        // Test boolean false
        let json = b"false";
        let index = JsonIndex::build(json);
        let root = index.root(json);
        assert!(matches!(root.value(), StandardJson::Bool(false)));

        // Test null
        let json = b"null";
        let index = JsonIndex::build(json);
        let root = index.root(json);
        assert!(matches!(root.value(), StandardJson::Null));
    }

    #[test]
    fn test_number() {
        let json = b"42";
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::Number(n) => {
                assert_eq!(n.as_i64().unwrap(), 42);
            }
            _ => panic!("expected number"),
        }
    }

    #[test]
    fn test_string() {
        let json = br#""hello""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                assert_eq!(s.as_str().unwrap(), "hello");
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_object_single_field() {
        let json = br#"{"name": "Alice"}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::Object(fields) => {
                assert!(!fields.is_empty());

                // Uncons the first field
                let (field, rest) = fields.uncons().expect("should have one field");

                // Check key
                match field.key() {
                    StandardJson::String(s) => {
                        assert_eq!(s.as_str().unwrap(), "name");
                    }
                    _ => panic!("expected string key"),
                }

                // Check value
                match field.value() {
                    StandardJson::String(s) => {
                        assert_eq!(s.as_str().unwrap(), "Alice");
                    }
                    _ => panic!("expected string value"),
                }

                // Rest should be empty
                assert!(rest.is_empty());
            }
            _ => panic!("expected object"),
        }
    }

    #[test]
    fn test_object_multiple_fields() {
        let json = br#"{"name": "Bob", "age": 30}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::Object(fields) => {
                // First field: name
                let (field1, rest1) = fields.uncons().expect("should have first field");
                match field1.key() {
                    StandardJson::String(s) => assert_eq!(s.as_str().unwrap(), "name"),
                    _ => panic!("expected string key"),
                }
                match field1.value() {
                    StandardJson::String(s) => assert_eq!(s.as_str().unwrap(), "Bob"),
                    _ => panic!("expected string value"),
                }

                // Second field: age
                let (field2, rest2) = rest1.uncons().expect("should have second field");
                match field2.key() {
                    StandardJson::String(s) => assert_eq!(s.as_str().unwrap(), "age"),
                    _ => panic!("expected string key"),
                }
                match field2.value() {
                    StandardJson::Number(n) => assert_eq!(n.as_i64().unwrap(), 30),
                    _ => panic!("expected number value"),
                }

                // No more fields
                assert!(rest2.is_empty());
            }
            _ => panic!("expected object"),
        }
    }

    #[test]
    fn test_object_find_field() {
        let json = br#"{"name": "Charlie", "age": 25, "city": "NYC"}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::Object(fields) => {
                // Find existing field
                match fields.find("age").unwrap() {
                    Some(StandardJson::Number(n)) => assert_eq!(n.as_i64().unwrap(), 25),
                    _ => panic!("expected number"),
                }

                // Find first field
                match fields.find("name").unwrap() {
                    Some(StandardJson::String(s)) => assert_eq!(s.as_str().unwrap(), "Charlie"),
                    _ => panic!("expected string"),
                }

                // Find last field
                match fields.find("city").unwrap() {
                    Some(StandardJson::String(s)) => assert_eq!(s.as_str().unwrap(), "NYC"),
                    _ => panic!("expected string"),
                }

                // Non-existent field
                assert!(fields.find("missing").unwrap().is_none());
            }
            _ => panic!("expected object"),
        }
    }

    /// #1251: a duplicate JSON key must resolve to its *last* value,
    /// matching real jq / RFC 8259 -- this used to return the first,
    /// diverging from `.a` field access in real jq (`{"a":1,"a":3}|.a`
    /// is `3`, not `1`).
    #[test]
    fn test_object_find_field_duplicate_key_last_wins_1251() {
        let json = br#"{"a": 1, "b": 2, "a": 3}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::Object(fields) => {
                match fields.find("a").unwrap() {
                    Some(StandardJson::Number(n)) => assert_eq!(n.as_i64().unwrap(), 3),
                    other => panic!("expected number 3, got {other:?}"),
                }
                let value_cursor = fields
                    .find_cursor("a")
                    .unwrap()
                    .expect("should find a cursor");
                match value_cursor.value() {
                    StandardJson::Number(n) => assert_eq!(n.as_i64().unwrap(), 3),
                    other => panic!("expected number 3, got {other:?}"),
                }
            }
            _ => panic!("expected object"),
        }
    }

    /// #1247: an object key that fails to decode must not end the field
    /// search. `find`/`find_cursor` used to `?` out of the whole function on
    /// the first undecodable key, so every *valid* field after it became
    /// invisible to lookup -- `.b` answered `null` on `{"\ud800":1,"b":2}`
    /// even though `keys`/`length`, which never decode, still reported `b`.
    #[test]
    fn test_object_find_skips_undecodable_key_1247() {
        // Both halves of "fails to decode": an unpaired surrogate escape and
        // a raw invalid UTF-8 byte. Each is a structurally valid `String`
        // token that `as_str()` rejects.
        let cases: [&[u8]; 2] = [br#"{"\ud800": 1, "b": 2}"#, b"{\"\xff\": 1, \"b\": 2}"];
        for json in cases {
            let index = JsonIndex::build(json);
            let root = index.root(json);

            let StandardJson::Object(fields) = root.value() else {
                panic!("expected object");
            };
            match fields.find("b").unwrap() {
                Some(StandardJson::Number(n)) => assert_eq!(n.as_i64().unwrap(), 2),
                other => panic!("expected number 2, got {other:?}"),
            }
            let cursor = fields
                .find_cursor("b")
                .unwrap()
                .expect("find_cursor should reach b past an undecodable key");
            match cursor.value() {
                StandardJson::Number(n) => assert_eq!(n.as_i64().unwrap(), 2),
                other => panic!("expected number 2, got {other:?}"),
            }
        }
    }

    /// #1995: `JsonFields::find` (not just its `find_cursor` sibling,
    /// already covered end-to-end via the CLI's `.b` dispatch) raises on a
    /// non-string sibling key too -- exercised directly here since `find`'s
    /// only production caller (`eval.rs`'s own separate evaluator) isn't
    /// reachable from the CLI test suite with raw document text.
    #[test]
    fn test_object_find_raises_on_non_string_sibling_key_1995() {
        let json = br#"{"a":1,123:2}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        let StandardJson::Object(fields) = root.value() else {
            panic!("expected object");
        };
        let err = fields
            .find("a")
            .expect_err("non-string sibling key must raise");
        assert!(
            err.to_string().contains("expected string key"),
            "unexpected error: {err}"
        );
    }

    /// #2261: `find_cursor` walks every field regardless of `name` (it must,
    /// to honour last-duplicate-key-wins), so a trailing stray comma after
    /// the object's real last field (`{"a":1,}`) is caught here too --
    /// whether or not `name` matches anything, matching real jq's own
    /// behavior (a malformed document can't be parsed at all, so *every*
    /// field access into it raises).
    #[test]
    fn test_find_cursor_rejects_trailing_comma_after_real_last_field_2261() {
        let json = br#"{"a":1,}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);
        let StandardJson::Object(fields) = root.value() else {
            panic!("expected object");
        };
        for name in ["a", "nonexistent"] {
            let err = fields
                .find_cursor(name)
                .expect_err(&format!("{name}: trailing comma should raise"));
            assert!(
                err.message.contains("Invalid JSON text"),
                "{name}: message: {}",
                err.message
            );
        }
    }

    /// #2261: unlike the single-real-field case above, a duplicate key
    /// followed by a trailing stray comma (`{"a":1,"b":2,"a":3,}`) must
    /// still raise -- `find_cursor`'s own winning-occurrence resolution
    /// (last-duplicate-key-wins) and the #2261 trailing-gap check are two
    /// independent questions the walk answers from the same pass, and a
    /// non-matching key's own gap must not stop the walk before it reaches
    /// the object's real last field.
    #[test]
    fn test_find_cursor_rejects_trailing_comma_with_duplicate_key_2261() {
        let json = br#"{"a":1,"b":2,"a":3,}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);
        let StandardJson::Object(fields) = root.value() else {
            panic!("expected object");
        };
        for name in ["a", "b"] {
            let err = fields
                .find_cursor(name)
                .expect_err(&format!("{name}: trailing comma should raise"));
            assert!(
                err.message.contains("Invalid JSON text"),
                "{name}: message: {}",
                err.message
            );
        }
    }

    /// #2261: well-formed objects (including a duplicate key, so the
    /// winning-occurrence resolution above is exercised alongside the new
    /// trailing-gap check) are unaffected.
    #[test]
    fn test_find_cursor_wellformed_unaffected_by_trailing_comma_check_2261() {
        for json in [
            br#"{"a":1}"#.as_slice(),
            br#"{"a":1,"b":2}"#.as_slice(),
            br#"{"a":1,"b":2,"a":3}"#.as_slice(),
        ] {
            let index = JsonIndex::build(json);
            let root = index.root(json);
            let StandardJson::Object(fields) = root.value() else {
                panic!("{json:?}: expected object");
            };
            let cursor = fields
                .find_cursor("a")
                .unwrap_or_else(|e| panic!("{json:?}: {e:?}"));
            assert!(cursor.is_some(), "{json:?}: expected to find `a`");
        }
    }

    /// #2288: `find` (unlike `find_cursor`, already fixed for this shape)
    /// had no #1677 delimiter check at all -- a missing `:` before the
    /// winning occurrence's value used to slip through silently.
    #[test]
    fn test_find_rejects_missing_colon_before_winning_value_2288() {
        let json = br#"{"a" 1}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);
        let StandardJson::Object(fields) = root.value() else {
            panic!("expected object");
        };
        let err = fields
            .find("a")
            .expect_err("missing colon before value should raise");
        assert!(
            err.message.contains("Invalid JSON text"),
            "message: {}",
            err.message
        );
    }

    /// #2288: the sibling gap on the *other* side of the winning
    /// occurrence's key -- a missing `,` before a non-first key (as
    /// opposed to the missing `:` after it, tested above).
    #[test]
    fn test_find_rejects_missing_comma_before_winning_key_2288() {
        let json = br#"{"a":1"b":2}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);
        let StandardJson::Object(fields) = root.value() else {
            panic!("expected object");
        };
        let err = fields
            .find("b")
            .expect_err("missing comma before a non-first key should raise");
        assert!(
            err.message.contains("Invalid JSON text"),
            "message: {}",
            err.message
        );
    }

    /// #2288: same shape, but the winning occurrence is a *later* duplicate
    /// -- `find`'s last-duplicate-key-wins resolution must still validate
    /// only the actual winner's own delimiters, not an earlier, superseded
    /// occurrence's (which may be perfectly well-formed).
    #[test]
    fn test_find_rejects_missing_colon_on_winning_duplicate_2288() {
        let json = br#"{"a":1,"a" 2}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);
        let StandardJson::Object(fields) = root.value() else {
            panic!("expected object");
        };
        let err = fields
            .find("a")
            .expect_err("missing colon on the winning duplicate should raise");
        assert!(
            err.message.contains("Invalid JSON text"),
            "message: {}",
            err.message
        );
    }

    /// #2288: the mirror image of the two tests above -- an *earlier*,
    /// non-winning duplicate's own missing colon must be ignored once a
    /// later, well-formed occurrence supersedes it. Pins the doc comment's
    /// own key claim ("an earlier same-named field's own delimiter gap is
    /// moot once a later one supersedes it") in the direction the other
    /// two tests don't cover.
    #[test]
    fn test_find_ignores_missing_colon_on_superseded_earlier_duplicate_2288() {
        let json = br#"{"a" 1,"a":2}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);
        let StandardJson::Object(fields) = root.value() else {
            panic!("expected object");
        };
        let value = fields
            .find("a")
            .unwrap_or_else(|e| panic!("superseded duplicate's own gap should be ignored: {e:?}"));
        match value {
            Some(StandardJson::Number(n)) => assert_eq!(n.as_i64().unwrap(), 2),
            other => panic!("expected the winning duplicate's value 2, got {other:?}"),
        }
    }

    /// #2288: `find` (unlike `find_cursor`, already fixed for this shape by
    /// #2261) had no trailing-stray-comma check either -- a trailing `,`
    /// after the object's real last field used to slip through silently,
    /// whether or not `name` matched anything.
    #[test]
    fn test_find_rejects_trailing_comma_after_real_last_field_2288() {
        let json = br#"{"a":1,}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);
        let StandardJson::Object(fields) = root.value() else {
            panic!("expected object");
        };
        for name in ["a", "nonexistent"] {
            let err = fields
                .find(name)
                .expect_err(&format!("{name}: trailing comma should raise"));
            assert!(
                err.message.contains("Invalid JSON text"),
                "{name}: message: {}",
                err.message
            );
        }
    }

    /// #2288: well-formed objects (including a duplicate key, exercising
    /// the same winning-occurrence resolution as the tests above) are
    /// unaffected by the new delimiter and trailing-comma checks.
    #[test]
    fn test_find_wellformed_unaffected_by_delimiter_check_2288() {
        for json in [
            br#"{"a":1}"#.as_slice(),
            br#"{"a":1,"b":2}"#.as_slice(),
            br#"{"a":1,"b":2,"a":3}"#.as_slice(),
        ] {
            let index = JsonIndex::build(json);
            let root = index.root(json);
            let StandardJson::Object(fields) = root.value() else {
                panic!("{json:?}: expected object");
            };
            let value = fields
                .find("a")
                .unwrap_or_else(|e| panic!("{json:?}: {e:?}"));
            assert!(value.is_some(), "{json:?}: expected to find `a`");
        }
    }

    #[test]
    fn test_array_single_element() {
        let json = br"[42]";
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::Array(elements) => {
                assert!(!elements.is_empty());

                let (elem, rest) = elements.uncons().expect("should have one element");
                match elem {
                    StandardJson::Number(n) => assert_eq!(n.as_i64().unwrap(), 42),
                    _ => panic!("expected number"),
                }

                assert!(rest.is_empty());
            }
            _ => panic!("expected array"),
        }
    }

    #[test]
    fn test_array_multiple_elements() {
        let json = br"[1, 2, 3]";
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::Array(elements) => {
                let (e1, rest1) = elements.uncons().expect("first");
                let (e2, rest2) = rest1.uncons().expect("second");
                let (e3, rest3) = rest2.uncons().expect("third");

                match e1 {
                    StandardJson::Number(n) => assert_eq!(n.as_i64().unwrap(), 1),
                    _ => panic!("expected number"),
                }
                match e2 {
                    StandardJson::Number(n) => assert_eq!(n.as_i64().unwrap(), 2),
                    _ => panic!("expected number"),
                }
                match e3 {
                    StandardJson::Number(n) => assert_eq!(n.as_i64().unwrap(), 3),
                    _ => panic!("expected number"),
                }

                assert!(rest3.is_empty());
            }
            _ => panic!("expected array"),
        }
    }

    #[test]
    fn test_array_get() {
        let json = br#"["a", "b", "c"]"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::Array(elements) => {
                match elements.get(0) {
                    Some(StandardJson::String(s)) => assert_eq!(s.as_str().unwrap(), "a"),
                    _ => panic!("expected string at index 0"),
                }
                match elements.get(1) {
                    Some(StandardJson::String(s)) => assert_eq!(s.as_str().unwrap(), "b"),
                    _ => panic!("expected string at index 1"),
                }
                match elements.get(2) {
                    Some(StandardJson::String(s)) => assert_eq!(s.as_str().unwrap(), "c"),
                    _ => panic!("expected string at index 2"),
                }
                assert!(elements.get(3).is_none());
            }
            _ => panic!("expected array"),
        }
    }

    #[test]
    fn test_nested_object() {
        let json = br#"{"person": {"name": "Dave"}}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::Object(fields) => match fields.find("person").unwrap() {
                Some(StandardJson::Object(inner_fields)) => {
                    match inner_fields.find("name").unwrap() {
                        Some(StandardJson::String(s)) => {
                            assert_eq!(s.as_str().unwrap(), "Dave");
                        }
                        _ => panic!("expected string"),
                    }
                }
                _ => panic!("expected nested object"),
            },
            _ => panic!("expected object"),
        }
    }

    #[test]
    fn test_array_of_objects() {
        let json = br#"[{"a": 1}, {"b": 2}]"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::Array(elements) => {
                // First object
                match elements.get(0) {
                    Some(StandardJson::Object(fields)) => match fields.find("a").unwrap() {
                        Some(StandardJson::Number(n)) => assert_eq!(n.as_i64().unwrap(), 1),
                        _ => panic!("expected number"),
                    },
                    _ => panic!("expected object"),
                }

                // Second object
                match elements.get(1) {
                    Some(StandardJson::Object(fields)) => match fields.find("b").unwrap() {
                        Some(StandardJson::Number(n)) => assert_eq!(n.as_i64().unwrap(), 2),
                        _ => panic!("expected number"),
                    },
                    _ => panic!("expected object"),
                }
            }
            _ => panic!("expected array"),
        }
    }

    #[test]
    fn test_negative_number() {
        let json = b"-123";
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::Number(n) => {
                assert_eq!(n.as_i64().unwrap(), -123);
            }
            _ => panic!("expected number"),
        }
    }

    #[test]
    fn test_float_number() {
        let json = b"1.23456";
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::Number(n) => {
                let f = n.as_f64().unwrap();
                assert!((f - 1.23456).abs() < 0.0001);
            }
            _ => panic!("expected number"),
        }
    }

    #[test]
    fn test_immutable_iteration() {
        // Test that iteration is truly immutable - we can iterate multiple times
        let json = br"[1, 2, 3]";
        let index = JsonIndex::build(json);
        let root = index.root(json);

        if let StandardJson::Array(elements) = root.value() {
            // First iteration
            let (e1, rest1) = elements.uncons().unwrap();
            assert!(matches!(e1, StandardJson::Number(_)));

            // Start over - elements is still valid
            let (e1_again, _) = elements.uncons().unwrap();
            assert!(matches!(e1_again, StandardJson::Number(_)));

            // Continue first iteration
            let (e2, _) = rest1.uncons().unwrap();
            assert!(matches!(e2, StandardJson::Number(_)));
        }
    }

    // ========================================================================
    // Escape sequence tests
    // ========================================================================

    #[test]
    fn test_string_no_escapes_is_borrowed() {
        let json = br#""hello world""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                let result = s.as_str().unwrap();
                // Should be Cow::Borrowed for strings without escapes
                assert!(matches!(result, Cow::Borrowed(_)));
                assert_eq!(&*result, "hello world");
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_escaped_quote() {
        let json = br#""hello\"world""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                let result = s.as_str().unwrap();
                assert_eq!(&*result, "hello\"world");
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_escaped_backslash() {
        let json = br#""hello\\world""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                let result = s.as_str().unwrap();
                assert_eq!(&*result, "hello\\world");
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_escaped_slash() {
        let json = br#""hello\/world""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                let result = s.as_str().unwrap();
                assert_eq!(&*result, "hello/world");
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_escaped_newline() {
        let json = br#""hello\nworld""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                let result = s.as_str().unwrap();
                assert_eq!(&*result, "hello\nworld");
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_escaped_tab() {
        let json = br#""hello\tworld""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                let result = s.as_str().unwrap();
                assert_eq!(&*result, "hello\tworld");
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_escaped_carriage_return() {
        let json = br#""hello\rworld""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                let result = s.as_str().unwrap();
                assert_eq!(&*result, "hello\rworld");
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_escaped_backspace() {
        let json = br#""hello\bworld""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                let result = s.as_str().unwrap();
                assert_eq!(&*result, "hello\u{0008}world");
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_escaped_formfeed() {
        let json = br#""hello\fworld""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                let result = s.as_str().unwrap();
                assert_eq!(&*result, "hello\u{000C}world");
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_unicode_escape_bmp() {
        // \u0041 is 'A'
        let json = br#""\u0041""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                let result = s.as_str().unwrap();
                assert_eq!(&*result, "A");
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_unicode_escape_euro() {
        // \u20AC is €
        let json = br#""\u20AC""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                let result = s.as_str().unwrap();
                assert_eq!(&*result, "€");
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_unicode_escape_lowercase() {
        // \u00e9 is é (lowercase hex)
        let json = br#""\u00e9""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                let result = s.as_str().unwrap();
                assert_eq!(&*result, "é");
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_unicode_surrogate_pair() {
        // \uD83D\uDE00 is 😀 (U+1F600)
        let json = br#""\uD83D\uDE00""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                let result = s.as_str().unwrap();
                assert_eq!(&*result, "😀");
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_multiple_escapes() {
        let json = br#""line1\nline2\ttab\r\n""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                let result = s.as_str().unwrap();
                assert_eq!(&*result, "line1\nline2\ttab\r\n");
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_mixed_escapes_and_unicode() {
        let json = br#""Price: \u20AC100\nTax: \u00A310""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                let result = s.as_str().unwrap();
                assert_eq!(&*result, "Price: €100\nTax: £10");
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_invalid_escape() {
        let json = br#""\x""#; // \x is not valid JSON
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                assert_eq!(s.as_str(), Err(JsonError::InvalidEscape));
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_lone_high_surrogate() {
        // Lone high surrogate without low surrogate
        let json = br#""\uD83D""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                assert_eq!(s.as_str(), Err(JsonError::InvalidUnicodeEscape));
            }
            _ => panic!("expected string"),
        }
    }

    /// #2008: a lone low surrogate is a different case from the lone high
    /// surrogate above -- real jq 1.7.1 doesn't reject it at all, it
    /// substitutes U+FFFD and accepts the document (confirmed live:
    /// `{"a":"\udc00"}` decodes to `{"a":"\u{FFFD}"}`, exit 0). Matches that
    /// instead of erroring, unlike the high-surrogate case, which stays the
    /// already-documented "echo the raw span" leniency since jq genuinely
    /// rejects it.
    #[test]
    fn test_string_lone_low_surrogate() {
        // Lone low surrogate
        let json = br#""\uDE00""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                let result = s.as_str().unwrap();
                assert_eq!(&*result, "\u{FFFD}");
            }
            _ => panic!("expected string"),
        }
    }

    /// #2008: pins the exact range boundary and mid-string/multiple-escape
    /// cases the issue's own repro covers.
    #[test]
    fn test_string_lone_low_surrogate_range_and_mid_string_2008() {
        for (json, want) in [
            (&br#""\uDC00""#[..], "\u{FFFD}"),
            (&br#""\uDFFF""#[..], "\u{FFFD}"),
            (&br#""x\uDC00y""#[..], "x\u{FFFD}y"),
        ] {
            let index = JsonIndex::build(json);
            let root = index.root(json);
            match root.value() {
                StandardJson::String(s) => {
                    let result = s.as_str().unwrap();
                    assert_eq!(&*result, want, "json={json:?}");
                }
                _ => panic!("expected string for json={json:?}"),
            }
        }
    }

    #[test]
    fn test_string_invalid_unicode_hex() {
        // Invalid hex digit
        let json = br#""\uXXXX""#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::String(s) => {
                assert_eq!(s.as_str(), Err(JsonError::InvalidUnicodeEscape));
            }
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_string_with_escaped_key_in_object() {
        let json = br#"{"na\nme": "value"}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        match root.value() {
            StandardJson::Object(fields) => {
                // find should handle escaped keys
                let (field, _) = fields.uncons().unwrap();
                match field.key() {
                    StandardJson::String(s) => {
                        assert_eq!(&*s.as_str().unwrap(), "na\nme");
                    }
                    _ => panic!("expected string key"),
                }
            }
            _ => panic!("expected object"),
        }
    }

    // ========================================================================
    // Iterator tests
    // ========================================================================

    #[test]
    fn test_json_fields_iterator() {
        let json = br#"{"a": 1, "b": 2, "c": 3}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        if let StandardJson::Object(fields) = root.value() {
            let keys: Vec<_> = fields
                .map(|f| {
                    if let StandardJson::String(s) = f.key() {
                        s.as_str().unwrap().into_owned()
                    } else {
                        panic!("expected string key")
                    }
                })
                .collect();
            assert_eq!(keys, vec!["a", "b", "c"]);
        } else {
            panic!("expected object");
        }
    }

    #[test]
    fn test_json_elements_iterator() {
        let json = br"[1, 2, 3, 4, 5]";
        let index = JsonIndex::build(json);
        let root = index.root(json);

        if let StandardJson::Array(elements) = root.value() {
            let nums: Vec<_> = elements
                .filter_map(|e| {
                    if let StandardJson::Number(n) = e {
                        n.as_i64().ok()
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(nums, vec![1, 2, 3, 4, 5]);
        } else {
            panic!("expected array");
        }
    }

    #[test]
    fn test_iterator_empty_object() {
        let json = br"{}";
        let index = JsonIndex::build(json);
        let root = index.root(json);

        if let StandardJson::Object(fields) = root.value() {
            assert_eq!(fields.count(), 0);
        } else {
            panic!("expected object");
        }
    }

    #[test]
    fn test_iterator_empty_array() {
        let json = br"[]";
        let index = JsonIndex::build(json);
        let root = index.root(json);

        if let StandardJson::Array(elements) = root.value() {
            assert_eq!(elements.count(), 0);
        } else {
            panic!("expected array");
        }
    }

    // ========================================================================
    // Display tests
    // ========================================================================

    #[test]
    fn test_json_error_display() {
        use std::string::ToString;
        assert_eq!(
            JsonError::InvalidUtf8.to_string(),
            "invalid UTF-8 in string"
        );
        assert_eq!(
            JsonError::InvalidNumber.to_string(),
            "invalid number format"
        );
        assert_eq!(
            JsonError::InvalidEscape.to_string(),
            "invalid escape sequence in string"
        );
        assert_eq!(
            JsonError::InvalidUnicodeEscape.to_string(),
            "invalid unicode escape sequence"
        );
    }

    // ========================================================================
    // Fast traversal tests (is_container, children)
    // ========================================================================

    #[test]
    fn test_is_container_object() {
        let json = br#"{"a": 1}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);
        assert!(root.is_container());
    }

    #[test]
    fn test_is_container_array() {
        let json = br"[1, 2, 3]";
        let index = JsonIndex::build(json);
        let root = index.root(json);
        assert!(root.is_container());
    }

    #[test]
    fn test_is_container_empty_object() {
        let json = br"{}";
        let index = JsonIndex::build(json);
        let root = index.root(json);
        // Empty containers have no children, so is_container returns false
        assert!(!root.is_container());
    }

    #[test]
    fn test_is_container_empty_array() {
        let json = br"[]";
        let index = JsonIndex::build(json);
        let root = index.root(json);
        // Empty containers have no children, so is_container returns false
        assert!(!root.is_container());
    }

    #[test]
    fn test_is_container_leaf_values() {
        // String
        let json = br#""hello""#;
        let index = JsonIndex::build(json);
        assert!(!index.root(json).is_container());

        // Number
        let json = b"42";
        let index = JsonIndex::build(json);
        assert!(!index.root(json).is_container());

        // Boolean
        let json = b"true";
        let index = JsonIndex::build(json);
        assert!(!index.root(json).is_container());

        // Null
        let json = b"null";
        let index = JsonIndex::build(json);
        assert!(!index.root(json).is_container());
    }

    #[test]
    fn test_children_array() {
        let json = br"[1, 2, 3]";
        let index = JsonIndex::build(json);
        let root = index.root(json);

        // Count children using the fast iterator
        let count: usize = root.children().count();
        assert_eq!(count, 3);
    }

    #[test]
    fn test_children_object() {
        let json = br#"{"a": 1, "b": 2}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        // Object children include both keys and values
        // {"a": 1, "b": 2} -> children are: "a", 1, "b", 2
        let count: usize = root.children().count();
        assert_eq!(count, 4);
    }

    #[test]
    fn test_children_nested() {
        let json = br#"{"arr": [1, 2]}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        // Root's direct children: "arr", [1, 2]
        let direct_children: Vec<_> = root.children().collect();
        assert_eq!(direct_children.len(), 2);

        // The array has 2 children: 1, 2
        let array_cursor = direct_children[1]; // [1, 2]
        assert!(array_cursor.is_container());
        assert_eq!(array_cursor.children().count(), 2);
    }

    #[test]
    fn test_children_empty() {
        let json = br"[]";
        let index = JsonIndex::build(json);
        let root = index.root(json);

        assert_eq!(root.children().count(), 0);
    }

    #[test]
    fn test_children_recursive_count() {
        // Test that recursive counting works correctly
        let json = br#"{"a": [1, 2], "b": {"c": 3}}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        fn count_all(cursor: super::JsonCursor) -> usize {
            1 + cursor.children().map(count_all).sum::<usize>()
        }

        // Structure (BP nodes):
        // root object (1)
        //   "a" key (1)
        //   [1, 2] value (1)
        //     1 (1)
        //     2 (1)
        //   "b" key (1)
        //   {"c": 3} value (1)
        //     "c" key (1)
        //     3 value (1)
        // Total: 9 nodes
        assert_eq!(count_all(root), 9);
    }

    // ========================================================================
    // Newline index tests
    // ========================================================================

    #[test]
    fn test_newline_index_single_line() {
        let json = br#"{"name": "Alice"}"#;
        let index = JsonIndex::build(json);

        // All positions on line 1
        assert_eq!(index.to_line_column(0, json), (1, 1)); // '{'
        assert_eq!(index.to_line_column(8, json), (1, 9)); // ' '
        assert_eq!(index.to_line_column(16, json), (1, 17)); // '}'

        // Reverse lookup
        assert_eq!(index.to_offset(1, 1, json), Some(0));
        assert_eq!(index.to_offset(1, 9, json), Some(8));
        assert_eq!(index.to_offset(2, 1, json), None); // No line 2
    }

    #[test]
    fn test_newline_index_multi_line() {
        let json = b"{\n  \"name\": \"Alice\"\n}";
        let index = JsonIndex::build(json);

        // Line 1: position 0 ('{')
        assert_eq!(index.to_line_column(0, json), (1, 1));
        assert_eq!(index.to_line_column(1, json), (1, 2)); // '\n'

        // Line 2: positions 2-18 ('  "name": "Alice"')
        assert_eq!(index.to_line_column(2, json), (2, 1)); // first space
        assert_eq!(index.to_line_column(4, json), (2, 3)); // '"'

        // Line 3: position 20 ('}')
        assert_eq!(index.to_line_column(20, json), (3, 1));

        // Reverse lookup
        assert_eq!(index.to_offset(1, 1, json), Some(0));
        assert_eq!(index.to_offset(2, 1, json), Some(2));
        assert_eq!(index.to_offset(3, 1, json), Some(20));
    }

    #[test]
    fn test_newline_index_array() {
        // Layout: "[\n  1,\n  2,\n  3\n]"
        // Pos 0: '[' (line 1)
        // Pos 1: '\n'
        // Pos 2-5: '  1,' (line 2)
        // Pos 6: '\n'
        // Pos 7-10: '  2,' (line 3)
        // Pos 11: '\n'
        // Pos 12-14: '  3' (line 4)
        // Pos 15: '\n'
        // Pos 16: ']' (line 5)
        let json = b"[\n  1,\n  2,\n  3\n]";
        let index = JsonIndex::build(json);

        // Line 1: '['
        assert_eq!(index.to_line_column(0, json), (1, 1));

        // Line 2: '  1,' starts at position 2
        assert_eq!(index.to_line_column(2, json), (2, 1));
        assert_eq!(index.to_line_column(5, json), (2, 4)); // the comma

        // Line 3: '  2,' starts at position 7
        assert_eq!(index.to_line_column(7, json), (3, 1));

        // Line 4: '  3' starts at position 12
        assert_eq!(index.to_line_column(12, json), (4, 1));

        // Line 5: ']' starts at position 16
        assert_eq!(index.to_line_column(16, json), (5, 1));

        // Reverse lookup
        assert_eq!(index.to_offset(1, 1, json), Some(0));
        assert_eq!(index.to_offset(2, 1, json), Some(2));
        assert_eq!(index.to_offset(3, 1, json), Some(7));
        assert_eq!(index.to_offset(5, 1, json), Some(16));
    }

    #[test]
    fn test_newline_index_crlf() {
        let json = b"{\r\n\"a\": 1\r\n}";
        let index = JsonIndex::build(json);

        // Line 1: '{'
        assert_eq!(index.to_line_column(0, json), (1, 1));

        // Line 2: '"a": 1' (starts at position 3, after \r\n)
        assert_eq!(index.to_line_column(3, json), (2, 1));
        assert_eq!(index.to_offset(2, 1, json), Some(3));

        // Line 3: '}' (starts at position 11, after \r\n)
        assert_eq!(index.to_line_column(11, json), (3, 1));
        assert_eq!(index.to_offset(3, 1, json), Some(11));
    }

    #[test]
    fn test_newline_index_invalid_inputs() {
        let json = b"{\n\"a\": 1\n}";
        let index = JsonIndex::build(json);

        assert_eq!(index.to_offset(0, 1, json), None); // line 0 invalid
        assert_eq!(index.to_offset(1, 0, json), None); // column 0 invalid
    }

    #[test]
    fn test_newline_index_round_trip() {
        let json =
            b"{\n  \"users\": [\n    {\"name\": \"Alice\"},\n    {\"name\": \"Bob\"}\n  ]\n}";
        let index = JsonIndex::build(json);

        // Test round-trip: offset -> line/column -> offset
        for offset in 0..json.len() {
            let (line, col) = index.to_line_column(offset, json);
            let result = index.to_offset(line, col, json);
            assert_eq!(
                result,
                Some(offset),
                "Round-trip failed for offset {offset}"
            );
        }
    }

    /// #1576: `JsonCursor` now implements `stream_sequence_json` (JSON
    /// output only -- `stream_sequence_yaml`, JSON cursors rendered as a
    /// YAML sequence, stays at the trait default and is pinned declining
    /// below, a real gap tracked as a follow-up rather than folded into
    /// this issue), which is what lets `GenericResult::stream_json`'s
    /// `LazySeq` arm stream straight from cursors for JSON instead of
    /// always materializing an `OwnedValue::Array`, mirroring what #757
    /// already did for `YamlCursor`.
    #[test]
    fn test_json_cursor_streams_sequence_json_1576() {
        let json = br#"[{"a": 1}, {"b": 2}]"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);
        let elements = root.value().as_array().unwrap();
        let (first, rest) = elements.uncons_cursor().unwrap();
        let (second, _) = rest.uncons_cursor().unwrap();
        let cursors = [first, second];

        assert!(
            <JsonCursor<'_, Vec<u64>> as DocumentCursor>::supports_sequence_streaming(),
            "JsonCursor has a sequence writer now; the probe must say so"
        );

        let mut out = String::new();
        JsonCursor::stream_sequence_json(
            &cursors,
            &mut out,
            IndentSpec::COMPACT,
            false,
            JsonConvention::Preserve,
        )
        .unwrap();
        assert_eq!(out, r#"[{"a":1},{"b":2}]"#);

        let mut out = String::new();
        JsonCursor::stream_sequence_json(
            &cursors,
            &mut out,
            IndentSpec::spaces(2),
            false,
            JsonConvention::Preserve,
        )
        .unwrap();
        assert_eq!(out, "[\n  {\n    \"a\": 1\n  },\n  {\n    \"b\": 2\n  }\n]");
    }

    /// `stream_sequence_yaml` (JSON cursors rendered as a YAML sequence)
    /// stays at the `DocumentCursor` trait default -- out of #1576's scope
    /// (JSON output only). Pinned the same way #757's original test pinned
    /// both writers: the writer failing *before writing anything* is what
    /// makes a future caller safe if it ever skips
    /// `supports_sequence_streaming`'s probe -- but that probe itself now
    /// answers `true` (see the JSON test above), so a caller that skips it
    /// only to reach this arm would already be doing something else wrong.
    #[test]
    fn test_json_cursor_declines_sequence_streaming_yaml_757() {
        let json = br#"[{"a": 1}, {"b": 2}]"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);
        let elements = root.value().as_array().unwrap();
        let (first, rest) = elements.uncons_cursor().unwrap();
        let (second, _) = rest.uncons_cursor().unwrap();
        let cursors = [first, second];

        for indent in [IndentSpec::COMPACT, IndentSpec::spaces(2)] {
            let mut out = String::new();
            assert!(
                JsonCursor::stream_sequence_yaml(&cursors, &mut out, indent, false).is_err(),
                "the default must decline, not half-write"
            );
            assert!(out.is_empty(), "nothing may reach `out`: {out:?}");
        }
    }

    // === text_range tests for containers (issue #137) ===

    #[test]
    fn test_text_range_nested_object_value() {
        let json = br#"{"key": {"key2": "value"}}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        let fields = root.value().as_object().unwrap();
        let (value, _) = fields.uncons().unwrap();
        let range = value.value_cursor().text_range().unwrap();
        assert_eq!(range, (8, 25));
    }

    #[test]
    fn test_text_range_empty_object_value() {
        let json = br#"{"key": {}}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);
        let fields = root.value().as_object().unwrap();
        let (field, _) = fields.uncons().unwrap();
        let range = field.value_cursor().text_range().unwrap();
        assert_eq!(range, (8, 10));
        assert_eq!(&json[range.0..range.1], b"{}");
    }

    #[test]
    fn test_text_range_empty_array_value() {
        let json = br#"{"list": []}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);
        let fields = root.value().as_object().unwrap();
        let (field, _) = fields.uncons().unwrap();
        let range = field.value_cursor().text_range().unwrap();
        assert_eq!(range, (9, 11));
        assert_eq!(&json[range.0..range.1], b"[]");
    }

    #[test]
    fn test_text_range_array_value() {
        let json = br#"{"items": [1, 2, 3]}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);
        let fields = root.value().as_object().unwrap();
        let (field, _) = fields.uncons().unwrap();
        let range = field.value_cursor().text_range().unwrap();
        assert_eq!(range, (10, 19));
        assert_eq!(&json[range.0..range.1], b"[1, 2, 3]");
    }

    #[test]
    fn test_text_range_second_field() {
        let json = br#"{"a": 1, "b": "hello"}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);
        let fields = root.value().as_object().unwrap();
        let (_, rest) = fields.uncons().unwrap();
        let (field_b, _) = rest.uncons().unwrap();
        let range = field_b.value_cursor().text_range().unwrap();
        assert_eq!(range, (14, 21));
        assert_eq!(&json[range.0..range.1], br#""hello""#);
    }

    #[test]
    fn test_text_range_deeply_nested() {
        let json = br#"{"a": {"b": {"c": 1}}}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);

        let fields = root.value().as_object().unwrap();
        let (field_a, _) = fields.uncons().unwrap();
        assert_eq!(field_a.value_cursor().text_range().unwrap(), (6, 21));
        assert_eq!(&json[6..21], br#"{"b": {"c": 1}}"#);

        let fields_b = field_a.value().as_object().unwrap();
        let (field_b, _) = fields_b.uncons().unwrap();
        assert_eq!(field_b.value_cursor().text_range().unwrap(), (12, 20));
        assert_eq!(&json[12..20], br#"{"c": 1}"#);

        let fields_c = field_b.value().as_object().unwrap();
        let (field_c, _) = fields_c.uncons().unwrap();
        assert_eq!(field_c.value_cursor().text_range().unwrap(), (18, 19));
        assert_eq!(&json[18..19], b"1");
    }

    #[test]
    fn test_text_range_root_object() {
        let json = br#"{"a": 1}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);
        let range = root.text_range().unwrap();
        assert_eq!(range, (0, 8));
        assert_eq!(&json[range.0..range.1], br#"{"a": 1}"#);
    }

    #[test]
    fn test_text_range_root_array() {
        let json = b"[1, 2, 3]";
        let index = JsonIndex::build(json);
        let root = index.root(json);
        let range = root.text_range().unwrap();
        assert_eq!(range, (0, 9));
        assert_eq!(&json[range.0..range.1], b"[1, 2, 3]");
    }

    /// Pins `number_literal_end`'s deliberate rejection of a dangling
    /// exponent marker (#1218's documented example of the crate's 4-way
    /// number-scanner divergence) -- a future edit that accidentally makes
    /// this function lenient here, matching `nested_number_span`'s own
    /// permissive contract, would defeat the whole reason it's the
    /// stricter of the two (see the type's own doc comment).
    #[test]
    fn test_number_literal_end_rejects_dangling_exponent_marker_1218() {
        assert_eq!(number_literal_end(b"5e", 0), None);
        assert_eq!(number_literal_end(b"1E", 0), None);
        // A well-formed exponent still parses, so this isn't blanket
        // exponent-hostility.
        assert_eq!(number_literal_end(b"5e1", 0), Some(3));
    }

    /// `string_literal_end` rejects every raw `U+0000`-`U+001F` byte and
    /// accepts `0x7F`, which is the exact split real jq draws (its check is
    /// on a signed char, so DEL passes). Sweeping the whole range rather
    /// than spot-checking TAB is deliberate: a hand-picked matrix would not
    /// catch an off-by-one at either boundary, and `0x20`/`0x7F` are the two
    /// bytes that prove the rule is "< 0x20" and not "non-printable" (#2878).
    #[test]
    fn test_string_literal_end_rejects_only_raw_c0_controls_2878() {
        for byte in 0u8..=0x7F {
            let mut doc = Vec::from(*b"\"a");
            doc.push(byte);
            doc.extend_from_slice(b"b\"");
            let got = string_literal_end(&doc, 0);
            if byte < 0x20 {
                assert_eq!(got, None, "raw control byte 0x{byte:02X} must be rejected");
            } else if byte == b'"' || byte == b'\\' {
                // Not a control character: these two end or escape the
                // string rather than sitting in it, so they are not part of
                // this rule and are covered by the tests below.
                continue;
            } else {
                assert_eq!(
                    got,
                    Some(doc.len()),
                    "byte 0x{byte:02X} is not a C0 control and must be accepted"
                );
            }
        }
    }

    /// The escaped spelling of the same character stays valid -- the rule is
    /// about *raw* bytes, so `\\t` (two bytes: backslash, `t`) must still
    /// scan, or the fix would break every well-formed document (#2878).
    #[test]
    fn test_string_literal_end_accepts_escaped_control_characters_2878() {
        // Each of these is one complete string literal, so the answer is
        // always the whole slice -- spelled `.len()` rather than a counted
        // constant so the assertion cannot drift from the literal above it.
        for doc in [
            &br#""a\tb""#[..],
            &br#""a\u0009b""#[..],
            // An escaped quote does not end the string.
            &br#""a\"b""#[..],
        ] {
            assert_eq!(
                string_literal_end(doc, 0),
                Some(doc.len()),
                "{doc:?} is one whole string literal"
            );
        }
    }

    /// Unterminated strings still return `None` -- the pre-existing contract
    /// this function inherited from the scalar loop it replaced. The final
    /// case pushes the escape-skip past the end of the buffer, which the
    /// SIMD scanner's own past-the-end fast exit has to turn into `None`
    /// rather than panicking on an out-of-bounds index (#2878).
    #[test]
    fn test_string_literal_end_rejects_unterminated_2878() {
        assert_eq!(string_literal_end(b"\"abc", 0), None);
        assert_eq!(string_literal_end(b"\"", 0), None);
        assert_eq!(string_literal_end(br#""abc\"#, 0), None);
    }

    /// The scan starts at `start`, not at zero, and a control character
    /// *before* the string it is asked about is none of its business --
    /// pinning that `find_json_escape`'s `start` argument is threaded
    /// through rather than dropped (#2878).
    #[test]
    fn test_string_literal_end_honours_its_start_offset_2878() {
        // The first string holds a raw TAB, so scanning *it* must fail --
        // and scanning from the *second* string must not notice it at all.
        // `start` always indexes an opening quote, never the `[` before it.
        let doc = b"[\"a\tb\", \"ok\"]";
        assert_eq!(string_literal_end(doc, 1), None);
        let second = doc
            .windows(4)
            .position(|w| w == b"\"ok\"")
            .expect("second literal present");
        assert_eq!(string_literal_end(doc, second), Some(second + 4));
    }

    /// `word_special_mask`'s contract is "lowest set bit is exact": every
    /// byte value at every one of the 8 positions, alone, and then with a
    /// borrow-provoking neighbour above it (`0x20`, which the `hasless`
    /// term flags spuriously only *after* a true hit below it), so the
    /// first-hit index never moves (#2963).
    #[test]
    fn test_word_special_mask_lowest_bit_is_exact_2963() {
        let is_special = |b: u8| b == b'"' || b == b'\\' || b < 0x20;
        for pos in 0..8 {
            for byte in 0u8..=255 {
                let mut w = [b'a'; 8];
                w[pos] = byte;
                let mask = word_special_mask(u64::from_le_bytes(w));
                if is_special(byte) {
                    assert_eq!(
                        mask.trailing_zeros() / 8,
                        pos as u32,
                        "0x{byte:02X} at {pos}"
                    );
                } else {
                    assert_eq!(mask, 0, "0x{byte:02X} at {pos} is not special");
                }
                if is_special(byte) && pos < 7 {
                    for above in [0x20u8, 0x21, 0x80, 0xFF, b'"', 0x00] {
                        w[pos + 1] = above;
                        let mask = word_special_mask(u64::from_le_bytes(w));
                        assert_eq!(
                            mask.trailing_zeros() / 8,
                            pos as u32,
                            "0x{byte:02X} at {pos} with 0x{above:02X} above"
                        );
                    }
                }
            }
        }
        assert_eq!(word_special_mask(u64::from_le_bytes(*b"abcdefgh")), 0);
        assert_eq!(word_special_mask(u64::from_le_bytes([0xFF; 8])), 0);
        assert_eq!(word_special_mask(u64::from_le_bytes([0x20; 8])), 0);
        assert_eq!(word_special_mask(u64::from_le_bytes([0x80; 8])), 0);
    }

    /// Independent scalar reference for the tests below -- the pre-#2878
    /// loop, with the control-character rule -- kept separate from
    /// `next_string_special`'s own probe so a shared bug cannot hide.
    fn reference_string_end(bytes: &[u8], start: usize) -> Option<usize> {
        let mut i = start + 1;
        while i < bytes.len() {
            match bytes[i] {
                b'"' => return Some(i + 1),
                b'\\' => i += 2,
                b if b < 0x20 => return None,
                _ => i += 1,
            }
        }
        None
    }

    /// Every probe length the tests exercise: off, the shipped aarch64
    /// value, and one chunk wide. Each runs on every architecture.
    fn for_each_probe(mut check: impl FnMut(&str, fn(&[u8], usize) -> Option<usize>)) {
        check("K=0", string_literal_end_probed::<0>);
        check("K=8", string_literal_end_probed::<8>);
        check("K=16", string_literal_end_probed::<16>);
        check("shipped", string_literal_end);
    }

    /// The probed byte at every offset that matters for an 8-byte probe --
    /// the last byte inside it, the first two past it, and the far edge of
    /// the SIMD chunk that follows -- and every byte value at each one, so a
    /// byte the probe accepts and the chunk rejects (or the reverse, and the
    /// word trick's `>= 0x80` bytes in particular) cannot slip between the
    /// two (#2963).
    #[test]
    fn test_string_literal_end_probe_edges_agree_with_reference_2963() {
        for_each_probe(|label, scan| {
            for pos in [1usize, 7, 8, 9, 15, 16, 17, 23, 24, 25, 31, 32, 33] {
                for byte in 0u8..=0xFF {
                    let mut doc = vec![b'a'; pos + 40];
                    doc[0] = b'"';
                    doc[pos] = byte;
                    doc[pos + 20] = b'"';
                    assert_eq!(
                        scan(&doc, 0),
                        reference_string_end(&doc, 0),
                        "{label}: byte 0x{byte:02X} at offset {pos}"
                    );
                }
            }
        });
    }

    /// Escapes across the probe boundary: a `\` as the probe's last byte so
    /// the byte it protects lands on the far side, an escaped quote
    /// straddling the boundary, and a `\` as the buffer's last byte inside
    /// the window, which must still read as unterminated (#2963).
    #[test]
    fn test_string_literal_end_probe_boundary_escapes_2963() {
        for_each_probe(|label, scan| {
            for k in [8usize, 16] {
                // `\` on the probe's last byte (index k: the probe starts at
                // 1) protects a `"` on the first byte past it.
                let mut doc = vec![b'a'; k + 8];
                doc[0] = b'"';
                doc[k] = b'\\';
                doc[k + 1] = b'"';
                doc[k + 7] = b'"';
                assert_eq!(scan(&doc, 0), Some(k + 8), "{label}: escape at {k}");
                assert_eq!(scan(&doc, 0), reference_string_end(&doc, 0), "{label}");
                // A `\` at k+1 with the protected `"` at k+2 straddles the
                // boundary from the other side.
                let mut doc = vec![b'a'; k + 10];
                doc[0] = b'"';
                doc[k + 1] = b'\\';
                doc[k + 2] = b'"';
                doc[k + 9] = b'"';
                assert_eq!(scan(&doc, 0), Some(k + 10), "{label}: escape at {}", k + 1);
                // `\` as the final byte, inside the window.
                let mut doc = vec![b'a'; k / 2];
                doc[0] = b'"';
                doc[k / 2 - 1] = b'\\';
                assert_eq!(scan(&doc, 0), None, "{label}: trailing escape");
            }
        });
    }

    /// Buffers shorter than the probe: an unterminated string, a `start`
    /// within the probe's reach of the end, and a `start` that is the last
    /// byte. The probe must stop at the buffer's end, never read past it
    /// (#2963).
    #[test]
    fn test_string_literal_end_probe_stops_at_buffer_end_2963() {
        for_each_probe(|label, scan| {
            assert_eq!(scan(b"\"abc", 0), None, "{label}");
            assert_eq!(scan(b"\"", 0), None, "{label}");
            assert_eq!(scan(b"\"ab\"", 0), Some(4), "{label}");
            let doc = b"[1234567890, \"xy\"]";
            let start = doc.len() - 5;
            assert_eq!(scan(doc, start), Some(doc.len() - 1), "{label}");
            assert_eq!(
                scan(doc, doc.len() - 2),
                None,
                "{label}: opening quote last"
            );
        });
    }

    /// Randomised agreement with the scalar reference over strings of 0-80
    /// bytes drawn from the bytes that matter (`"`, `\`, the control
    /// boundary, DEL, and two non-ASCII bytes), at random start offsets so
    /// the probe window lands at every alignment (#2963).
    #[test]
    fn test_string_literal_end_probe_matches_reference_randomised_2963() {
        const ALPHABET: [u8; 9] = [b'a', b'"', b'\\', 0x00, 0x1F, 0x20, 0x7F, 0x80, 0xFF];
        let mut state = 0x2963_u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..4000 {
            let lead = (next() % 40) as usize;
            let len = (next() % 81) as usize;
            let mut doc = Vec::with_capacity(lead + len + 2);
            doc.extend(core::iter::repeat_n(b'a', lead));
            doc.push(b'"');
            for _ in 0..len {
                // Weight towards plain bytes so many strings are long and
                // reach the chunk behind the probe.
                doc.push(if next() % 4 != 0 {
                    b'a'
                } else {
                    ALPHABET[(next() % 9) as usize]
                });
            }
            if next() % 4 != 0 {
                doc.push(b'"');
            }
            let want = reference_string_end(&doc, lead);
            for_each_probe(|label, scan| {
                assert_eq!(scan(&doc, lead), want, "{label}: {doc:?} from {lead}");
            });
        }
    }

    /// Companion to the test above: `nested_number_span` -- unlike
    /// `number_literal_end` -- absorbs the same dangling exponent marker
    /// into one span rather than rejecting it, by design (#966, #1218).
    #[test]
    fn test_nested_number_span_absorbs_dangling_exponent_marker_1218() {
        assert_eq!(nested_number_span(b"5e", 0), 2);
        assert_eq!(nested_number_span(b"1E", 0), 2);
    }

    /// #2902: a reindexed computed float arrives here as the bridge's token,
    /// which the scanner captures whole and `as_f64` decodes, while
    /// `as_i64` and `number_literal()` both refuse it -- so a materializer
    /// that tries the literal first (`to_owned_at_depth`) still ends on a
    /// bare `Float`, never a `NumberLiteral` carrying the token's text. The
    /// tokens come from the encoder itself: this pins the plumbing, not the
    /// spelling (`validate.rs`'s own tests pin that).
    #[test]
    fn json_number_decodes_the_computed_float_token_2902() {
        for f in [
            1.0,
            2e16,
            -0.5,
            0.0,
            -0.0,
            5e-324,
            f64::MAX,
            1e300,
            0.1 + 0.2,
        ] {
            let token = crate::json::validate::computed_float_token(f);
            let json = format!("[{token}]");
            let bytes = json.as_bytes();
            assert_eq!(nested_number_span(bytes, 1), 1 + token.len());
            let index = JsonIndex::build(bytes);
            let root = index.root(bytes);
            let StandardJson::Array(mut items) = root.value() else {
                panic!("expected an array");
            };
            let StandardJson::Number(n) = items.next().expect("one element") else {
                panic!("expected a number");
            };
            assert_eq!(n.raw_bytes(), token.as_bytes());
            assert_eq!(n.as_f64().map(f64::to_bits), Ok(f64::to_bits(f)), "{token}");
            assert_eq!(n.as_i64(), Err(JsonError::InvalidNumber), "{token}");
            let number: StandardJson<'_, Vec<u64>> = StandardJson::Number(n);
            assert_eq!(number.number_literal(), None, "{token}");
        }
    }

    /// #2877: every spelling jq 1.7.1 reads as a number, nested inside a
    /// container, is one `Number` node here -- `value()`, `text_range()`
    /// and `raw_bytes()` agree on its span, `as_f64` gives jq's value, and
    /// `number_literal()` keeps only a spelling JSON can carry (unsigned).
    /// Each row's value column was captured live (`printf '[%s]' | jq -c
    /// '.[0] | [., type, isnan, isinfinite]'`).
    #[test]
    fn nested_jq_number_spellings_resolve_to_one_number_node_2877() {
        // (token, expected f64 or None for NaN, expected `number_literal()`)
        let rows: &[(&str, Option<f64>, Option<&str>)] = &[
            ("nan", None, None),
            ("NaN", None, None),
            ("-nan", None, None),
            ("+nan", None, None),
            ("nan1", None, None),
            ("sNaN12", None, None),
            ("inf", Some(f64::INFINITY), None),
            ("Infinity", Some(f64::INFINITY), None),
            ("iNfInItY", Some(f64::INFINITY), None),
            ("+inf", Some(f64::INFINITY), None),
            ("-inf", Some(f64::NEG_INFINITY), None),
            ("-Infinity", Some(f64::NEG_INFINITY), None),
            ("+1", Some(1.0), Some("1")),
            ("+0", Some(0.0), Some("0")),
            ("+01", Some(1.0), Some("01")),
            ("+.5", Some(0.5), Some(".5")),
            ("+1.500", Some(1.5), Some("1.500")),
            ("+1e400", Some(f64::INFINITY), Some("1e400")),
            ("+007.e5", Some(7e5), Some("007.e5")),
            ("+1.e5", Some(1e5), Some("1.e5")),
        ];
        for (token, value, literal) in rows {
            for (doc, start) in [
                (format!("[{token}]"), 1),
                (format!("[{token},2]"), 1),
                (format!("[0,{token}]"), 3),
                (format!(r#"{{"a":{token}}}"#), 5),
            ] {
                let bytes = doc.as_bytes();
                let index = JsonIndex::build(bytes);
                let root = index.root(bytes);
                let cursor = document_order_cursors(root)
                    .into_iter()
                    .find(|c| c.text_position() == Some(start))
                    .unwrap_or_else(|| panic!("no node at {start} in {doc}"));
                let StandardJson::Number(n) = cursor.value() else {
                    panic!(
                        "{doc}: expected a Number at {start}, got {:?}",
                        cursor.value() // omni-dev: coverage tolerate-line reason="unreachable in a passing suite by design -- the failure message for the assertion this #2877 test exists to make"
                    );
                };
                assert_eq!(n.raw_bytes(), token.as_bytes(), "{doc}");
                assert_eq!(
                    cursor.text_range(),
                    Some((start, start + token.len())),
                    "{doc}"
                );
                match value {
                    None => assert!(n.as_f64().is_ok_and(f64::is_nan), "{doc}"),
                    Some(v) => assert_eq!(n.as_f64(), Ok(*v), "{doc}"),
                }
                let number: StandardJson<'_, Vec<u64>> = StandardJson::Number(n);
                assert_eq!(
                    number.number_literal().as_deref(),
                    *literal,
                    "{doc}: number_literal"
                );
            }
        }
    }

    /// #2877: a word that is not one of decNumber's, and a `+` before
    /// anything but a digit, `.` or such a word, stay in the dispatchers'
    /// error arms -- they do not become `null` the way a malformed
    /// number-*shaped* span does (#966), because #966's precedent covers
    /// digits only. The whole scalar run has to validate: `nan1.5` is one
    /// node to the index and `Invalid numeric literal` to jq, and reading a
    /// valid prefix out of it would fabricate a NaN. `nullx`/`truex` are
    /// deliberately absent -- that prefix match is #3035's, not this
    /// issue's.
    #[test]
    fn non_decnumber_words_and_bare_plus_stay_errors_2877() {
        for token in [
            "nana",
            "nanx",
            "nan.",
            "nan1.5",
            "nan1e3",
            "nan-1",
            "nan(1)",
            "nane5",
            "qnan",
            "ssnan",
            "nA",
            "Nope",
            "infin",
            "infinit",
            "Infinite",
            "infinity1",
            "inf1",
            "sinf",
            "infx",
            "infinityx",
            "snan.",
            "+",
            "+x",
            "+e5",
            "+nanx",
            "+infx",
            "iu",
            "s",
            "S1",
        ] {
            for doc in [
                format!("[{token}]"),
                format!("[{token},2]"),
                format!(r#"{{"a":{token}}}"#),
            ] {
                let bytes = doc.as_bytes();
                let index = JsonIndex::build(bytes);
                let root = index.root(bytes);
                let child = root.first_child().expect("one child");
                let child = if doc.starts_with('{') {
                    child.next_sibling().expect("the value after the key")
                } else {
                    child
                };
                assert!(
                    matches!(child.value(), StandardJson::Error(_)),
                    "{doc}: expected Error, got {:?}",
                    child.value() // omni-dev: coverage tolerate-line reason="unreachable in a passing suite by design -- the failure message for the assertion this #2877 test exists to make"
                );
                assert_eq!(child.text_range(), None, "{doc}: text_range");
            }
        }
        // `-` before a word keeps today's span (`-` alone), so the #1643
        // gap check still rejects it downstream -- unchanged by #2877.
        assert_eq!(nested_number_span(b"[-nope]", 1), 2);
        assert_eq!(nested_number_span(b"[+x]", 1), 2);
        // ... while a valid signed word is the whole word.
        assert_eq!(nested_number_span(b"[-inf]", 1), 5);
        assert_eq!(nested_number_span(b"[+nan]", 1), 5);
        assert_eq!(nested_number_span(b"[sNaN12,1]", 1), 7);
        // A `+digit` span is exactly what the greedy class already gave.
        assert_eq!(nested_number_span(b"[+1.2.3]", 1), 7);
        assert_eq!(nested_number_span(b"[+1e0e0]", 1), 7);
    }

    /// #2877: the top-level splitter's own question. `number_literal_end`
    /// peels `+` like `-`; `jq_number_token_end` adds the words, ending them
    /// where jq's literal scanner does (whitespace, `"`, `[{,:]}`), and
    /// refuses a token that is neither grammar so the splitter errors
    /// (#1171) instead of truncating.
    #[test]
    fn jq_number_token_end_reads_both_grammars_2877() {
        assert_eq!(number_literal_end(b"+1", 0), Some(2));
        assert_eq!(number_literal_end(b"+.5e3", 0), Some(5));
        assert_eq!(number_literal_end(b"+1.500 ", 0), Some(6));
        for bad in [&b"+"[..], b"+-1", b"++1", b"+e5", b"+x", b"+nan"] {
            assert_eq!(number_literal_end(bad, 0), None, "{bad:?}");
        }
        for (text, end) in [
            (&b"nan"[..], 3),
            (b"nan 1", 3),
            (b"nan\n", 3),
            (b"NaN5,", 4),
            (b"sNaN12]", 6),
            (b"-Infinity}", 9),
            (b"+inf\"", 4),
            (b"inf:", 3),
            (b"+1.500", 6),
            (b"-.5", 3),
            (b"007", 3),
        ] {
            assert_eq!(jq_number_token_end(text, 0), Some(end), "{text:?}");
        }
        for bad in [
            &b"nanx"[..],
            b"nan1.5",
            b"nan(1)",
            b"nan-1",
            b"infinity1",
            b"inf1",
            b"nul",
            b"n",
            b"+",
            b"+-1",
            b"-",
            b"-nope",
            b"1e",
        ] {
            assert_eq!(jq_number_token_end(bad, 0), None, "{bad:?}");
        }
    }

    /// #2877: `as_f64` is the funnel every cursor-level reader uses, so it
    /// has to read what `from_number_bytes` reads -- the reindex bridge's
    /// NaN/infinity tokens included, now that the `--slurp`/`--seq` input
    /// path writes them (`OwnedValue::to_json_input_bridge`).
    #[test]
    fn json_number_as_f64_decodes_the_bridge_nonfinite_tokens_2877() {
        // The tokens come from the encoder itself, as the #2902 test above
        // does: this pins the plumbing, not the spelling.
        for (f, expected) in [
            (f64::NAN, None),
            (f64::INFINITY, Some(f64::INFINITY)),
            (f64::NEG_INFINITY, Some(f64::NEG_INFINITY)),
        ] {
            let token = crate::jq::OwnedValue::Float(f).to_json_input_bridge();
            assert!(
                token.parse::<f64>().is_err(),
                "the bridge token must not be an ordinary number: {token}"
            );
            let json = format!("[{token}]");
            let bytes = json.as_bytes();
            let index = JsonIndex::build(bytes);
            let root = index.root(bytes);
            let StandardJson::Number(n) = root.first_child().expect("one child").value() else {
                panic!("expected a number"); // omni-dev: coverage tolerate-line reason="unreachable in a passing suite by design -- the failure message for the assertion this #2877 test exists to make"
            };
            match expected {
                None => assert!(n.as_f64().is_ok_and(f64::is_nan), "{token}"),
                Some(v) => assert_eq!(n.as_f64(), Ok(v), "{token}"),
            }
        }
    }

    /// #2072 step 1: the `'static` handle round-trips on *every* node of a
    /// nested document -- the root object, nested arrays and objects, and
    /// each scalar leaf -- and re-basing from an unrelated cursor of the
    /// same document (the root) lands on the same node as re-basing from the
    /// cursor itself. That second half is the whole point of the handle: the
    /// AST that will store one has long since lost the cursor it came from,
    /// and can only offer whatever cursor is ambient at the point of use.
    #[test]
    fn node_id_round_trips_for_every_node_2072() {
        let json: &[u8] = br#"{"a":[1,2,{"b":null}],"c":{"d":"e","f":[true,false]},"g":[],"h":{}}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);
        let cursors = document_order_cursors(root);
        assert!(
            cursors.len() > 10,
            "the fixture must be big enough to be worth walking, got {}",
            cursors.len() // omni-dev: coverage tolerate-line reason="unreachable in a passing suite by design -- this is a panic-message format argument for the #2072 pin itself, only evaluated if the assert's own condition is false (#2072)"
        );

        for c in &cursors {
            let id = c.node_id();
            let back = c
                .at_node_id(id)
                .expect("a node's own id always names a node in its own document");
            assert!(
                back.same_node(c),
                "id {id} re-resolved to bp {} instead of {}",
                back.node_id(), // omni-dev: coverage tolerate-line reason="unreachable in a passing suite by design -- this is a panic-message format argument for the #2072 pin itself, only evaluated if the assert's own condition is false (#2072)"
                id
            );
            assert_eq!(back.node_id(), id, "the round trip must be idempotent");

            let from_root = root
                .at_node_id(id)
                .expect("any cursor of the document rebases the same id");
            assert!(
                from_root.same_node(c),
                "rebasing id {id} from the root landed elsewhere"
            );
        }

        // Distinct nodes must get distinct ids, or `same_node` and equality
        // of ids would not be the same relation.
        let mut ids: Vec<usize> = cursors.iter().map(DocumentCursor::node_id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), cursors.len(), "two distinct nodes shared an id");
    }

    /// #2072 step 1: an id that does not name a node -- a *closing*
    /// parenthesis, or anything past the end of the BP vector -- is `None`,
    /// never a cursor onto some neighbouring node. Every open position, by
    /// contrast, is a real node, which is why one `is_open` check is the
    /// whole implementation.
    #[test]
    fn at_node_id_rejects_closing_and_out_of_range_ids_2072() {
        let json: &[u8] = br#"{"a":[1,2],"b":"c"}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);
        let bp = index.bp();

        let mut closes = 0usize;
        let mut opens = 0usize;
        for id in 0..bp.len() {
            match root.at_node_id(id) {
                Some(c) => {
                    assert!(bp.is_open(id), "position {id} is a close but resolved");
                    assert_eq!(c.node_id(), id);
                    opens += 1;
                }
                None => {
                    assert!(
                        !bp.is_open(id),
                        "position {id} is an open but did not resolve"
                    );
                    closes += 1;
                }
            }
        }
        assert!(opens > 0 && closes > 0, "the fixture must contain both");

        assert!(root.at_node_id(bp.len()).is_none(), "one past the end");
        assert!(
            root.at_node_id(bp.len() + 1000).is_none(),
            "far past the end"
        );
        assert!(
            root.at_node_id(usize::MAX).is_none(),
            "no overflow, just None"
        );
    }

    /// #2072 step 1: the token is constant across a document and differs
    /// between two documents that are alive at the same time -- including
    /// two indices built from *identical* bytes, which is exactly the shape
    /// the reindex bridge produces and the shape a value-based check could
    /// not tell apart.
    #[test]
    fn document_token_is_per_index_not_per_node_2072() {
        let json: &[u8] = br#"{"a":[1,2],"b":"c"}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);
        let token = root.document_token();
        for c in document_order_cursors(root) {
            assert_eq!(
                c.document_token(),
                token,
                "bp {} disagreed about its own document",
                c.node_id() // omni-dev: coverage tolerate-line reason="unreachable in a passing suite by design -- this is a panic-message format argument for the #2072 pin itself, only evaluated if the assert's own condition is false (#2072)"
            );
        }

        let same_bytes: &[u8] = br#"{"a":[1,2],"b":"c"}"#;
        let other = JsonIndex::build(same_bytes);
        assert_ne!(
            token,
            other.root(same_bytes).document_token(),
            "two live indices over equal bytes are still two documents"
        );
    }
}
