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

use core::cell::OnceCell;

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
#[derive(Clone, Debug)]
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
        #[cfg(feature = "select-stats")]
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
                #[cfg(feature = "select-stats")]
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
                #[cfg(feature = "select-stats")]
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
            #[cfg(feature = "select-stats")]
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

        #[cfg(feature = "select-stats")]
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

        // Use rank / 8 as a hint for where to start searching in IB.
        // JSON typically has ~7-8 structural characters per 64 bytes,
        // so rank / 8 is a reasonable estimate of the word index.
        // For sequential traversal, this gives O(log d) instead of O(log n)
        // where d is the distance from the hint.
        let hint = rank / 8;
        self.index.ib_select1_from(rank, hint)
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
            // Null
            b'n' => {
                if self.text[start..].starts_with(b"null") {
                    start + 4
                } else {
                    return None;
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
    pub fn find(&self, name: &str) -> Result<Option<StandardJson<'a, W>>, EvalError>
    where
        W: Clone,
    {
        let mut fields = *self;
        let mut result = None;
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
            if let StandardJson::String(key) = key {
                if key.as_str().is_ok_and(|k| k == name) {
                    result = Some(field.value());
                }
            }
            fields = rest;
        }
        Ok(result)
    }

    /// Find a field by name and return a cursor to its value.
    ///
    /// Same last-duplicate-key-wins semantics as [`find`](Self::find) — kept
    /// as a separate loop rather than reusing `find` so the returned cursor
    /// (needed for `line`/`column`) doesn't require re-navigating.
    ///
    /// `Err` when the *winning* occurrence's own `,`/`:` delimiters are
    /// malformed (#1677), or when *any* sibling's key isn't even
    /// string-shaped at all (#1995) -- a targeted lookup like `.a` never
    /// walks every sibling the way `.[]`/`length`/`keys` do, so it needs
    /// its own check for both. A non-string key (`{"a":1,123:2}`) is real
    /// jq's own parse-time rejection ("Object keys must be strings"),
    /// unconditional on the document as a whole -- unlike the `,`/`:` gap
    /// check below, checked as each candidate is found (not deferred to
    /// the winner alone): the document is malformed the moment *any*
    /// member's key isn't a string, whether or not that member is the one
    /// this lookup would otherwise have returned. Same shared
    /// `key_is_malformed`/[`DocumentFields::malformed_member_error`]
    /// pair as [`find`](Self::find) uses for this, not a second copy of
    /// the check (#106).
    pub fn find_cursor(&self, name: &str) -> Result<Option<JsonCursor<'a, W>>, EvalError>
    where
        W: Clone,
    {
        let mut fields = *self;
        // (key's own text start, value cursor, is this field the object's
        // first) for the winning candidate seen so far.
        let mut winner: Option<(usize, JsonCursor<'a, W>, bool)> = None;
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
            fields = rest;
            index += 1;
        }
        let Some((key_start, value_cursor, is_first)) = winner else {
            return Ok(None);
        };
        let comma_expected = if is_first { None } else { Some(b',') };
        if !preceding_gap_ok(value_cursor.text(), key_start, comma_expected) {
            return Err(EvalError::malformed_json_text(value_cursor.text()));
        }
        if let Some(value_start) = value_cursor.text_position() {
            if !preceding_gap_ok(value_cursor.text(), value_start, Some(b':')) {
                return Err(EvalError::malformed_json_text(value_cursor.text()));
            }
        }
        Ok(Some(value_cursor))
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

    /// The raw source span (quotes included) *and* whether it contains a
    /// backslash escape, in a single scan.
    ///
    /// A caller that needs both -- the JSON printer, which writes the span
    /// verbatim when nothing needs decoding, and the duplicate-key probe
    /// (#1385), which may compare raw spans only while nothing is escaped --
    /// would otherwise pay two passes over the same bytes:
    /// [`raw_bytes`](Self::raw_bytes) scans for the closing quote and
    /// `contains(&b'\\')` scans again. The quote scan already has to
    /// recognise every backslash in order to skip what it escapes, so
    /// reporting it is free. Measured worth 7-10% of `sjq '.'` on a 10 MB
    /// document, which is the entire cost of the probe.
    pub fn raw_and_escaped(&self) -> (&'a [u8], bool) {
        let mut i = self.start + 1; // skip the opening quote
        let mut escaped = false;
        while i < self.text.len() {
            match self.text[i] {
                b'"' => return (&self.text[self.start..=i], escaped),
                b'\\' => {
                    escaped = true;
                    i += 2;
                }
                _ => i += 1,
            }
        }
        (&self.text[self.start..], escaped)
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
/// strict JSON here -- a `.` immediately followed by a digit. Returns
/// `None` if the bytes at `start` don't actually form a valid number
/// token (`-e5`, `1e`, a bare `.`, ...).
///
/// Grammar: optional `-`; an integer part (0+ digits) and/or a
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
/// One of (at least) four independent "find a JSON-ish number token's
/// boundaries" functions in the crate, each with a genuinely different
/// strictness grammar for its own caller (#1218) -- besides
/// `nested_number_span` above, see
/// [`crate::json::simple_light`]'s private `find_number_end` (backs the
/// separate `SimpleJsonIndex`, fully greedy) and
/// `src/bin/succinctly/jq_runner.rs`'s private `find_number_end` (a
/// `--argjson` leading-zero repair pass, lenient on a dangling exponent
/// marker unlike this function). None delegate to any other; see #1218
/// for the full survey and why a blanket consolidation needs its own
/// design pass.
pub fn number_literal_end(text: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    if i < text.len() && text[i] == b'-' {
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
/// tokens with no separator between them. `is_valid_number`/
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
    if i < text.len() && text[i] == b'-' {
        i += 1;
    }
    while i < text.len() {
        match text[i] {
            b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-' => i += 1,
            _ => break,
        }
    }
    i
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
    pub fn as_f64(&self) -> Result<f64, JsonError> {
        let bytes = self.raw_bytes();
        let s = core::str::from_utf8(bytes).map_err(|_| JsonError::InvalidUtf8)?;
        s.parse().map_err(|_| JsonError::InvalidNumber)
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
    effective_fields_checked, key_is_malformed, DocumentCursor, DocumentElements, DocumentField,
    DocumentFields, DocumentValue, IndentSpec, JsonConvention,
};
use crate::jq::escape::{write_json_body_jq, write_json_body_yq};
use crate::jq::stream::{StreamFailure, StreamResult};
use crate::jq::{
    format_number_jq_compat, nesting_depth_exceeded_message, nonfinite_display_string, EvalError,
    JqSemantics, OwnedValue, YqSemantics, MAX_VALUE_TREE_DEPTH,
};

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
pub fn following_gap_ok(text: &[u8], key_end: usize) -> bool {
    let mut i = key_end;
    let mut found = false;
    while i < text.len() {
        match text[i] {
            b':' => {
                if found {
                    return false; // doubled delimiter
                }
                found = true;
                i += 1;
            }
            b if b.is_ascii_whitespace() => i += 1,
            _ => break,
        }
    }
    found
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
    fn parent(&self) -> Option<Self> {
        JsonCursor::parent(self)
    }

    #[inline]
    fn is_container(&self) -> bool {
        JsonCursor::is_container(self)
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
        if indent.is_compact() && !sort_keys && numbers == JsonConvention::Preserve {
            if let Some(bytes) = self.raw_bytes() {
                // SAFETY: JSON input is valid UTF-8 (checked during indexing)
                let s = core::str::from_utf8(bytes).map_err(|_| core::fmt::Error)?;
                return Ok(out.write_str(s)?);
            }
            return Err(StreamFailure::Fmt);
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
    #[inline]
    fn is_falsy(&self, numbers: JsonConvention) -> bool {
        match self.value() {
            StandardJson::Null | StandardJson::Bool(false) => true,
            StandardJson::Number(_) => {
                numbers == JsonConvention::JqCompat && self.value().number_literal().is_none()
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
                // `parse_json_seq`'s `validate_and_materialize_json`), so
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
                let (raw, escaped) = s.raw_and_escaped();
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
            None => EvalError::new("Invalid JSON text"),
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
            None => EvalError::new("Invalid JSON text"),
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
                let elem_value = if let Some(pos) = elem_cursor.text_position() {
                    let expected = if first { None } else { Some(b',') };
                    if !elem_cursor.preceding_delimiter_ok(pos, expected) {
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
            // `effective_fields_checked` (`src/jq/document.rs`) is the
            // validating sibling of `effective_fields` this writer used to
            // call (#1576 review): besides applying the mode's own
            // `COLLAPSE_DUPLICATE_KEYS` rule the same way (true for jq --
            // a repeated key collapses to one field, first position, last
            // value, exactly `IndexMap::insert` semantics; false for
            // `--preserve-input`/yq, every occurrence kept, real yq's own
            // behavior #1008 -- the same axis `numbers` already selects,
            // ADR-0018 rule 5, so `JqCompat` doubles as the collapse flag
            // here too), it also runs the same `key_is_malformed`/
            // `key_delimiter_ok`/`value_delimiter_ok`/`ends_unpaired`
            // checks `to_owned_at_depth`'s own object loop performs --
            // closing the #1194 (bareword/non-string key) and #1677/#1676
            // (missing/doubled `,`/`:`) gaps this writer used to have.
            let mut items = effective_fields_checked(&fields, numbers == JsonConvention::JqCompat)
                .map_err(StreamFailure::Decode)?;
            // #1676: a trailing `,` (`{"a":1,}`) -- `effective_fields_checked`
            // (and its own `ends_unpaired` check) only catches an *unpaired*
            // trailing key (`{"a":1,"b"}`), not a dangling comma after a
            // complete pair, since `uncons` simply stops at the last real
            // field either way.
            //
            // Deliberately re-walks `fields` (unchecked -- delimiters are
            // already known good from `effective_fields_checked` above) for
            // the *true* last field in raw source/cursor order, rather than
            // using `items.last()`: when `collapse` is true, `items` is
            // `effective_fields_checked`'s already-collapsed, first-
            // position-ordered result, whose own last entry can be a
            // *earlier* duplicate key's position in the source text --
            // `{"b":1,"a":2,"b":3}` collapses to `[b, a]` for `items`, but
            // `a`'s value in the source is followed by `,"b":3}`, not `}`,
            // so checking `items.last()` there would misfire on a
            // perfectly well-formed document. The raw last field is always
            // the right one to check regardless of collapsing, since a
            // trailing comma is a property of the source text's own tail,
            // not of whichever field ends up last in the display order.
            let mut last_raw_field = None;
            let mut raw_walk = fields;
            while let Some((field, rest)) = raw_walk.uncons() {
                last_raw_field = Some(field);
                raw_walk = rest;
            }
            if let Some(last) = last_raw_field {
                let last_value_cursor = last.value_cursor();
                if let Some(pos) = last_value_cursor.text_position() {
                    if let Some(end) = scalar_end_pos(pos, &last.value()) {
                        if !trailing_gap_ok(last_value_cursor.text(), end, b'}') {
                            return Err(StreamFailure::Decode(EvalError::malformed_json_text(
                                last_value_cursor.text(),
                            )));
                        }
                    }
                }
            }
            out.write_char('{')?;
            let next_indent = current_indent + indent_spaces;
            let mut first = true;
            if sort_keys {
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
                items = keyed.into_iter().map(|(_, field)| field).collect();
            }
            for field in items {
                if !first {
                    out.write_char(',')?;
                }
                first = false;
                if indent_spaces > 0 {
                    out.write_char('\n')?;
                    write_json_indent(out, next_indent, unit)?;
                }
                if let StandardJson::String(k) = field.key {
                    write_json_string_pretty(out, k, numbers)?;
                } else {
                    out.write_str("\"\"")?;
                }
                out.write_str(if indent_spaces > 0 { ": " } else { ":" })?;
                // #1576 review: same nested-empty-container gap as the array
                // arm above (see `empty_container_gap_ok`'s doc comment) --
                // `stream_json`'s root-only check never reaches a field's
                // value here, so `{"a": {"b": {,}}}` needs its own check
                // against this field's own value cursor.
                if !empty_container_gap_ok(&field.value_cursor, &field.value) {
                    return Err(StreamFailure::Decode(EvalError::malformed_json_text(
                        field.value_cursor.text(),
                    )));
                }
                stream_json_pretty(
                    out,
                    field.value,
                    next_indent,
                    indent_spaces,
                    unit,
                    sort_keys,
                    numbers,
                    depth + 1,
                )?;
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
        // fabricated `null`.
        StandardJson::Error(msg) => Err(StreamFailure::Decode(EvalError::new(msg))),
    }
}

/// Write a JSON string value using `numbers`'s escaping convention
/// (`write_json_body_jq`/`write_json_body_yq`).
///
/// Zero-copy fast path, mirroring `src/bin/succinctly/jq_runner.rs`'s own
/// `print_json`: a span with no `\` escape needs no re-encoding under
/// *either* convention (both tables agree on every byte that can appear
/// unescaped in valid JSON source, DEL included -- see `print_json`'s own
/// long-standing identical choice not to special-case it), so it's echoed
/// verbatim, quotes and all.
fn write_json_string_pretty<Out: core::fmt::Write>(
    out: &mut Out,
    s: JsonString<'_>,
    numbers: JsonConvention,
) -> StreamResult {
    let (raw, escaped) = s.raw_and_escaped();
    // Unlike `print_json`'s own zero-copy check (`std::io::Write`, which
    // passes bytes through unvalidated), this writer is `core::fmt::Write`
    // -- a `str`-oriented trait -- so invalid UTF-8 in an unescaped span
    // can't just be blasted through. Falling through to the decode-and-
    // escape path below on that specific failure (rather than surfacing a
    // bare `core::fmt::Error` right here) keeps this a proper diagnosable
    // decode failure via `json_decode_failure`, matching #1615 -- not a
    // second, worse way for the same class of bad input to go undiagnosed.
    if !escaped {
        if let Ok(text) = core::str::from_utf8(raw) {
            return Ok(out.write_str(text)?);
        }
    }
    let decoded = s.as_str().map_err(json_decode_failure)?;
    out.write_char('"')?;
    match numbers {
        JsonConvention::Preserve => write_json_body_yq(out, &decoded)?,
        JsonConvention::JqCompat => write_json_body_jq(out, &decoded)?,
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
        JsonConvention::Preserve => {
            let text = core::str::from_utf8(raw).map_err(|_| core::fmt::Error)?;
            out.write_str(text)
        }
        JsonConvention::JqCompat => {
            if crate::json::validate::is_valid_number(raw) {
                return out.write_str(&format_number_jq_compat(raw));
            }
            // #966/#1171: the semi-index scanner accepts a number *span*
            // more leniently than RFC 8259 (leading zeros, a leading dot,
            // a malformed trailing shape like `1.2.3`). Sanitize via the
            // same fallback every other "raw bytes -> number" conversion
            // in this crate uses, instead of reformatting invalid text.
            match OwnedValue::from_number_bytes(raw) {
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
    // taken there is `numbers`'s, not the output format's. `Preserve` is
    // reachable from the CLI as `succinctly jq --preserve-input` with any
    // non-compact/sorting output style (compact `Preserve` short-circuits
    // to the raw echo in `stream_json` before this writer is reached).
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

    /// Companion to the test above: `nested_number_span` -- unlike
    /// `number_literal_end` -- absorbs the same dangling exponent marker
    /// into one span rather than rejecting it, by design (#966, #1218).
    #[test]
    fn test_nested_number_span_absorbs_dangling_exponent_marker_1218() {
        assert_eq!(nested_number_span(b"5e", 0), 2);
        assert_eq!(nested_number_span(b"1E", 0), 2);
    }
}
