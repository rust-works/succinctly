//! Line-start index for byte-offset <-> line/column conversion.
//!
//! Line starts are a *sparse monotone* set: real workloads run 19-78 bytes per
//! line (see `docs/benchmarks/corpus-shape.md`), so one line start every few
//! dozen bytes. Storing them as a dense bitmap over the whole text — which is
//! what the three separate `BitVec` newline indices did before #228 — costs
//! ~1.27 bits per *input byte* regardless of how few lines there are, and pays
//! the full rank-directory and select-sample overhead on top.
//!
//! [`LineIndex`] stores the line starts themselves, Elias-Fano encoded, at
//! roughly `2 + log2(average line length)` bits per *line*:
//!
//! | Average line length | Dense bitmap | `LineIndex` |
//! |---------------------|--------------|-------------|
//! | 20 bytes (YAML/CSV) | 15.9% of input | ~3.9% |
//! | 78 bytes (pretty JSON) | 15.7% of input | ~1.3% |
//! | minified (one line) | 15.6% of input | ~0 |
//!
//! # Line terminators
//!
//! Unix (LF), Windows (CRLF) and classic Mac (CR) are all recognised, via the
//! shared rule in `crate::text::line_break` (private) that the YAML scanners
//! also use — one definition, not a per-module copy (#341).
//! A terminator at the very end of the text does *not* start a new line, so
//! `"a\n"` has one line, not two.
//!
//! # Example
//!
//! ```
//! use succinctly::text::LineIndex;
//!
//! let index = LineIndex::build(b"line1\nline2\nline3");
//!
//! assert_eq!(index.line_count(), 3);
//! assert_eq!(index.to_line_column(6), (2, 1));
//! assert_eq!(index.to_offset(2, 1), Some(6));
//! ```

#[cfg(not(test))]
use alloc::vec::Vec;
use core::cell::Cell;

use crate::bits::EliasFano;
use crate::text::line_break::{is_line_break, line_break_len};

/// How many lines [`LineIndex::to_line_column`] will walk forward from the
/// cache before giving up and falling back to a full binary search. Bounds
/// the worst case of a query that lands just past the cached line while
/// keeping the common near-sequential case (`.[] | line` walking a document
/// top to bottom) cheap. Not benchmarked against other values — a reasonable
/// starting point, not a tuned constant.
const FORWARD_WALK_CAP: u32 = 16;

/// One cached `(offset, line, line_start)` lookup, so a monotone walk of
/// [`LineIndex::to_line_column`] — e.g. `.[] | line` visiting array elements
/// in document order — resolves in amortised O(1) instead of a fresh
/// `EliasFano::predecessor` binary search per call. Mirrors the one-entry
/// `Cell`-based cache `AdvancePositions` uses for its O1/O2 optimizations
/// (`src/yaml/advance_positions.rs`).
#[derive(Clone, Copy, Debug)]
struct LineCacheEntry {
    /// The clamped query offset this entry was computed for.
    offset: u32,
    /// 0-indexed line containing `offset`.
    line_idx: u32,
    /// Byte offset at which `line_idx` starts.
    line_start: u32,
}

/// Index over the byte offsets at which each line starts.
///
/// Line and column numbers are 1-indexed; byte offsets are 0-indexed.
///
/// # Out-of-range positions
///
/// The two directions disagree on purpose, each preserving the behaviour of
/// the `BitVec` implementation it replaced.
/// [`to_line_column`](Self::to_line_column) extrapolates past the end of the
/// text — offset 99 of a 5-byte text reports as a column on the last line —
/// while [`to_offset`](Self::to_offset) rejects anything that would land past
/// the end. So the round trip is total for in-bounds offsets only; feed it an
/// out-of-range offset and `to_offset` returns `None` on the way back.
///
/// See the [module docs](self) for the space rationale and terminator handling.
#[derive(Clone, Debug)]
pub struct LineIndex {
    /// Byte offset of every line start, including line 1 at offset 0.
    /// Always non-empty: text with no terminators still has one line.
    starts: EliasFano,
    /// Total length of the indexed text.
    text_len: usize,
    /// Last `to_line_column` lookup, reused to speed up nearby/repeated
    /// queries. `Cell<T: Copy>` keeps `LineIndex` both `Clone` and `Debug`.
    cache: Cell<Option<LineCacheEntry>>,
}

impl LineIndex {
    /// Build a line index by scanning the text once for line terminators.
    ///
    /// # Panics
    ///
    /// Panics if `text.len() > u32::MAX`, the crate-wide 4 GiB input ceiling
    /// (#188). See `docs/reference/limits.md`.
    pub fn build(text: &[u8]) -> Self {
        assert!(
            u32::try_from(text.len()).is_ok(),
            "LineIndex supports inputs up to u32::MAX bytes, got {} (#188)",
            text.len()
        );

        // Pass 1: reserve the scratch vector up front. This counts break
        // *bytes*, so CRLF contributes two where it needs one — an over-reserve
        // on Windows text, exact on LF/CR-only text. Either way the vector
        // never reallocates, which is the point: doubling would make the
        // transient allocation 1.5-2x the vector it is building.
        let terminators = text.iter().copied().filter(|&b| is_line_break(b)).count();
        let mut starts = Vec::with_capacity(terminators + 1);

        // Line 1 starts at offset 0 even when the text is empty. Storing it
        // removes the `line == 1` special case from every query.
        starts.push(0u32);

        // Pass 2: one line start per break that has text after it. `\r\n` is a
        // single break two bytes wide, so the rule lives in `text::line_break`
        // rather than being spelled out again here (#341).
        let mut i = 0;
        while i < text.len() {
            let width = line_break_len(text, i);
            if width == 0 {
                i += 1;
                continue;
            }

            i += width;
            if i < text.len() {
                starts.push(i as u32);
            }
        }

        Self {
            starts: EliasFano::build(&starts),
            text_len: text.len(),
            cache: Cell::new(None),
        }
    }

    /// Number of lines in the indexed text. Always at least 1.
    #[inline]
    pub fn line_count(&self) -> usize {
        self.starts.len()
    }

    /// Length in bytes of the indexed text.
    #[inline]
    pub fn text_len(&self) -> usize {
        self.text_len
    }

    /// Byte offset at which the given 1-indexed line starts.
    ///
    /// Returns `None` if `line` is 0 or past the last line.
    #[inline]
    pub fn line_start(&self, line: usize) -> Option<usize> {
        let idx = line.checked_sub(1)?;
        self.starts.get(idx).map(|start| start as usize)
    }

    /// Convert a 0-indexed byte offset to a 1-indexed `(line, column)`.
    ///
    /// Offsets past the end of the text are reported against the last line
    /// rather than rejected, matching the previous `BitVec`-backed behaviour.
    ///
    /// # Performance
    ///
    /// A repeated or monotonically-forward query (the access pattern
    /// `.[] | line` produces when walking a document top to bottom) resolves
    /// in amortised O(1) via a one-entry cache of the last lookup, up to
    /// `FORWARD_WALK_CAP` lines of forward movement. Beyond that — or on
    /// the first call, or a backward/large jump — this is `O(log lines)`,
    /// still cheap enough for the cold paths that predate the cache: error
    /// reporting, `at_position`, and the locate CLIs.
    pub fn to_line_column(&self, offset: usize) -> (usize, usize) {
        // text_len <= u32::MAX, so clamping only affects offsets that are
        // already past the end.
        let query = offset.min(u32::MAX as usize) as u32;

        if let Some(entry) = self.cache.get() {
            if query == entry.offset {
                // Exact repeat (e.g. `{l: line, c: column}` querying the
                // same node twice): no lookup at all.
                return (
                    entry.line_idx as usize + 1,
                    offset - entry.line_start as usize + 1,
                );
            }

            if query > entry.offset {
                if let Some(result) = self.walk_forward_from(entry, offset, query) {
                    return result;
                }
                // Walk exceeded FORWARD_WALK_CAP: fall through to the full
                // binary search below, same as a cold/backward query.
            }
        }

        // Always `Some`: element 0 is offset 0.
        let (idx, start) = self
            .starts
            .predecessor(query)
            .expect("LineIndex always holds line 1");

        self.cache.set(Some(LineCacheEntry {
            offset: query,
            line_idx: idx as u32,
            line_start: start,
        }));

        (idx + 1, offset - start as usize + 1)
    }

    /// Try to resolve `query` by walking forward at most
    /// [`FORWARD_WALK_CAP`] lines from `entry`, using O(1)
    /// [`EliasFano::get`] per step instead of a fresh binary search.
    /// Returns `None` if the walk exceeds the cap without reaching `query`.
    fn walk_forward_from(
        &self,
        entry: LineCacheEntry,
        offset: usize,
        query: u32,
    ) -> Option<(usize, usize)> {
        let mut line_idx = entry.line_idx;
        let mut line_start = entry.line_start;

        for _ in 0..FORWARD_WALK_CAP {
            match self.starts.get(line_idx as usize + 1) {
                Some(next_start) if next_start <= query => {
                    line_idx += 1;
                    line_start = next_start;
                }
                // `next_start > query`, or no further line (`None`, `query`
                // is on/past the last line either way): `query` is on
                // `line_idx`.
                _ => {
                    self.cache.set(Some(LineCacheEntry {
                        offset: query,
                        line_idx,
                        line_start,
                    }));
                    return Some((line_idx as usize + 1, offset - line_start as usize + 1));
                }
            }
        }

        None
    }

    /// Convert a 1-indexed line and column to a 0-indexed byte offset.
    ///
    /// Returns `None` if `line` or `column` is 0, if the line does not exist,
    /// or if the resulting offset is past the end of the text.
    #[inline]
    pub fn to_offset(&self, line: usize, column: usize) -> Option<usize> {
        if column == 0 {
            return None;
        }

        let offset = self.line_start(line)? + column - 1;
        if offset < self.text_len {
            Some(offset)
        } else {
            None
        }
    }

    /// Returns the heap memory usage in bytes.
    #[inline]
    pub fn heap_size(&self) -> usize {
        self.starts.heap_size()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference implementation: the obvious forward scan.
    fn naive_line_starts(text: &[u8]) -> Vec<usize> {
        let mut starts = vec![0usize];
        let mut i = 0;
        while i < text.len() {
            match text[i] {
                b'\n' => {
                    if i + 1 < text.len() {
                        starts.push(i + 1);
                    }
                    i += 1;
                }
                b'\r' => {
                    let next = if i + 1 < text.len() && text[i + 1] == b'\n' {
                        i + 2
                    } else {
                        i + 1
                    };
                    if next < text.len() {
                        starts.push(next);
                    }
                    i = next;
                }
                _ => i += 1,
            }
        }
        starts
    }

    #[test]
    fn empty_text_has_one_line() {
        let index = LineIndex::build(b"");

        assert_eq!(index.line_count(), 1);
        assert_eq!(index.text_len(), 0);
        assert_eq!(index.line_start(1), Some(0));
        assert_eq!(index.to_line_column(0), (1, 1));
        // No offset is in bounds, so no position resolves.
        assert_eq!(index.to_offset(1, 1), None);
    }

    #[test]
    fn single_line_without_terminator() {
        let text = b"hello world";
        let index = LineIndex::build(text);

        assert_eq!(index.line_count(), 1);
        assert_eq!(index.to_offset(1, 1), Some(0));
        assert_eq!(index.to_offset(1, 6), Some(5));
        assert_eq!(index.to_offset(1, 12), None, "past end");
        assert_eq!(index.to_offset(2, 1), None, "no line 2");
        assert_eq!(index.to_line_column(0), (1, 1));
        assert_eq!(index.to_line_column(5), (1, 6));
    }

    #[test]
    fn unix_lf() {
        let text = b"line1\nline2\nline3";
        let index = LineIndex::build(text);

        assert_eq!(index.line_count(), 3);
        assert_eq!(index.to_offset(1, 1), Some(0));
        assert_eq!(index.to_offset(1, 5), Some(4));
        assert_eq!(index.to_offset(2, 1), Some(6));
        assert_eq!(index.to_offset(2, 5), Some(10));
        assert_eq!(index.to_offset(3, 1), Some(12));
        assert_eq!(index.to_offset(3, 5), Some(16));

        assert_eq!(index.to_line_column(0), (1, 1));
        assert_eq!(index.to_line_column(5), (1, 6), "the \\n itself");
        assert_eq!(index.to_line_column(6), (2, 1));
        assert_eq!(index.to_line_column(12), (3, 1));
    }

    #[test]
    fn windows_crlf() {
        let text = b"line1\r\nline2\r\nline3";
        let index = LineIndex::build(text);

        assert_eq!(index.line_count(), 3);
        assert_eq!(index.to_offset(1, 1), Some(0));
        assert_eq!(index.to_offset(1, 5), Some(4));
        assert_eq!(index.to_offset(2, 1), Some(7));
        assert_eq!(index.to_offset(2, 5), Some(11));
        assert_eq!(index.to_offset(3, 1), Some(14));

        assert_eq!(index.to_line_column(0), (1, 1));
        assert_eq!(index.to_line_column(7), (2, 1));
        assert_eq!(index.to_line_column(14), (3, 1));
    }

    #[test]
    fn classic_mac_cr() {
        let text = b"line1\rline2\rline3";
        let index = LineIndex::build(text);

        assert_eq!(index.line_count(), 3);
        assert_eq!(index.to_offset(1, 1), Some(0));
        assert_eq!(index.to_offset(2, 1), Some(6));
        assert_eq!(index.to_offset(3, 1), Some(12));
        assert_eq!(index.to_line_column(6), (2, 1));
    }

    #[test]
    fn mixed_terminators() {
        // LF, CRLF, CR in one document.
        let text = b"a\nb\r\nc\rd";
        let index = LineIndex::build(text);

        assert_eq!(index.line_count(), 4);
        assert_eq!(index.line_start(1), Some(0)); // a
        assert_eq!(index.line_start(2), Some(2)); // b
        assert_eq!(index.line_start(3), Some(5)); // c
        assert_eq!(index.line_start(4), Some(7)); // d
        assert_eq!(index.to_line_column(7), (4, 1));
    }

    #[test]
    fn trailing_terminator_adds_no_phantom_line() {
        for text in [&b"a\n"[..], &b"a\r\n"[..], &b"a\r"[..]] {
            let index = LineIndex::build(text);
            assert_eq!(index.line_count(), 1, "text {text:?}");
            assert_eq!(index.line_start(2), None, "text {text:?}");
        }
    }

    #[test]
    fn leading_and_consecutive_terminators() {
        // Leading newline, then an empty line in the middle.
        let text = b"\na\n\nb";
        let index = LineIndex::build(text);

        assert_eq!(index.line_count(), 4);
        assert_eq!(index.line_start(1), Some(0)); // the leading \n
        assert_eq!(index.line_start(2), Some(1)); // a
        assert_eq!(index.line_start(3), Some(3)); // the empty line
        assert_eq!(index.line_start(4), Some(4)); // b
        assert_eq!(index.to_line_column(3), (3, 1), "empty line");
    }

    #[test]
    fn offset_past_end_reports_against_last_line() {
        // Matches the clamping the BitVec rank1 path had before #228.
        let index = LineIndex::build(b"ab\ncd");
        assert_eq!(index.to_line_column(4), (2, 2));
        assert_eq!(index.to_line_column(5), (2, 3), "one past the end");
        assert_eq!(index.to_line_column(99), (2, 97));
    }

    #[test]
    fn zero_line_or_column_is_rejected() {
        let index = LineIndex::build(b"hello\nworld");
        assert_eq!(index.to_offset(0, 1), None);
        assert_eq!(index.to_offset(1, 0), None);
        assert_eq!(index.line_start(0), None);
    }

    #[test]
    fn one_huge_line() {
        // The case a dense bitmap handled worst: 1 MiB, a single line.
        let text = vec![b'x'; 1024 * 1024];
        let index = LineIndex::build(&text);

        assert_eq!(index.line_count(), 1);
        assert_eq!(index.to_line_column(1_000_000), (1, 1_000_001));
        assert_eq!(index.to_offset(1, 1_000_001), Some(1_000_000));
        // One line costs a handful of bytes, not 1/8th of the text.
        assert!(
            index.heap_size() < 1024,
            "heap_size {} should be trivial for one line",
            index.heap_size()
        );
    }

    #[test]
    fn crosses_elias_fano_select_sample_boundaries() {
        // EliasFano samples every 256 elements; exercise several boundaries.
        for line_count in [255usize, 256, 257, 512, 8193] {
            let mut text = Vec::new();
            for i in 0..line_count {
                if i > 0 {
                    text.push(b'\n');
                }
                text.extend_from_slice(b"abcdefgh");
            }
            let index = LineIndex::build(&text);

            assert_eq!(index.line_count(), line_count, "{line_count} lines");
            for line in [1usize, 2, 256, 257, line_count] {
                if line <= line_count {
                    let expected = (line - 1) * 9;
                    assert_eq!(index.line_start(line), Some(expected), "line {line}");
                    assert_eq!(index.to_line_column(expected), (line, 1), "line {line}");
                }
            }
        }
    }

    #[test]
    fn matches_naive_scanner_at_every_offset() {
        let texts: [&[u8]; 6] = [
            b"",
            b"no terminators here",
            b"a\nb\nc",
            b"a\r\nb\r\nc",
            b"\r\r\n\n\ra",
            b"{\n  \"key\": \"value\",\n  \"n\": 42\n}\n",
        ];

        for text in texts {
            let index = LineIndex::build(text);
            let starts = naive_line_starts(text);

            assert_eq!(index.line_count(), starts.len(), "text {text:?}");

            for offset in 0..text.len() {
                // Expected line = index of the last start <= offset.
                let line = starts.partition_point(|&s| s <= offset);
                let expected = (line, offset - starts[line - 1] + 1);
                assert_eq!(
                    index.to_line_column(offset),
                    expected,
                    "text {text:?} offset {offset}"
                );

                // Round-trip.
                let (l, c) = expected;
                assert_eq!(
                    index.to_offset(l, c),
                    Some(offset),
                    "round-trip text {text:?} offset {offset}"
                );
            }
        }
    }

    #[test]
    fn heap_size_beats_a_dense_bitmap_on_realistic_lines() {
        // 20 bytes per line, the corpus median shape.
        let mut text = Vec::new();
        for _ in 0..1000 {
            text.extend_from_slice(b"key: value-abcdefg\n");
        }
        let index = LineIndex::build(&text);

        // What the pre-#228 representation would have cost for the same text.
        let mut bits = vec![0u64; text.len().div_ceil(64)];
        for line in 2..=index.line_count() {
            let pos = index.line_start(line).unwrap();
            bits[pos / 64] |= 1 << (pos % 64);
        }
        let dense = crate::bits::BitVec::from_words(bits, text.len());

        assert!(
            index.heap_size() * 3 < dense.heap_size(),
            "LineIndex {} should be several times smaller than BitVec {}",
            index.heap_size(),
            dense.heap_size()
        );
    }

    /// 40 one-line-per-`\n` lines, `line`'s 1-indexed start is `(line - 1) * 4`
    /// (each line is `"L%02d\n"`, 5 bytes wide including the terminator, but
    /// the last line has no trailing `\n` counted into it). Large enough to
    /// exercise both sides of [`FORWARD_WALK_CAP`] (16).
    fn many_lines_text() -> Vec<u8> {
        let mut text = Vec::new();
        for i in 0..40 {
            text.extend_from_slice(format!("L{i:02}\n").as_bytes());
        }
        text
    }

    #[test]
    fn to_line_column_forward_gap_within_cap_matches_naive() {
        let text = many_lines_text();
        let index = LineIndex::build(&text);
        let starts = naive_line_starts(&text);

        // Prime the cache on line 3, then jump forward 10 lines (< cap):
        // should resolve via the forward walk, not a fresh binary search.
        let seed_offset = starts[2];
        assert_eq!(index.to_line_column(seed_offset), (3, 1));

        let target_offset = starts[12];
        let expected_line = starts.partition_point(|&s| s <= target_offset);
        assert_eq!(
            index.to_line_column(target_offset),
            (expected_line, target_offset - starts[expected_line - 1] + 1)
        );
    }

    #[test]
    fn to_line_column_forward_gap_beyond_cap_falls_back_correctly() {
        let text = many_lines_text();
        let index = LineIndex::build(&text);
        let starts = naive_line_starts(&text);

        // Prime the cache on line 1, then jump forward more than
        // FORWARD_WALK_CAP lines — must fall back to the binary search and
        // still return the correct answer, not a truncated/capped one.
        assert_eq!(index.to_line_column(0), (1, 1));

        let target_line = 1 + FORWARD_WALK_CAP as usize + 5;
        let target_offset = starts[target_line - 1];
        assert_eq!(index.to_line_column(target_offset), (target_line, 1));
    }

    #[test]
    fn to_line_column_backward_after_forward_run_matches_naive() {
        let text = many_lines_text();
        let index = LineIndex::build(&text);
        let starts = naive_line_starts(&text);

        // Walk forward to populate the cache with a "late" entry, then query
        // an earlier offset — the cache must not poison a backward lookup.
        for &line in &[1usize, 5, 10, 20] {
            let offset = starts[line - 1];
            assert_eq!(index.to_line_column(offset), (line, 1), "forward to {line}");
        }

        let backward_offset = starts[1]; // line 2
        assert_eq!(
            index.to_line_column(backward_offset),
            (2, 1),
            "backward jump"
        );

        // And a subsequent forward query from that backward position must
        // still be correct too (cache state after a backward miss).
        let offset = starts[7]; // line 8
        assert_eq!(index.to_line_column(offset), (8, 1), "forward again");
    }

    #[test]
    fn to_line_column_repeated_same_offset_is_exact_hit() {
        let text = many_lines_text();
        let index = LineIndex::build(&text);
        let starts = naive_line_starts(&text);

        let offset = starts[9]; // line 10
        let first = index.to_line_column(offset);
        let second = index.to_line_column(offset);
        assert_eq!(first, (10, 1));
        assert_eq!(first, second, "repeated query must return the same result");
    }

    #[test]
    fn to_line_column_shuffled_access_matches_naive() {
        // A non-monotonic access pattern (unlike `matches_naive_scanner_at_-
        // every_offset`'s strictly increasing scan) exercises every cache
        // branch — hit, forward-within-cap, forward-beyond-cap, and
        // backward — in one pass, order chosen to be deterministic rather
        // than random (no RNG dependency in this crate's core tests).
        let text = many_lines_text();
        let index = LineIndex::build(&text);
        let starts = naive_line_starts(&text);

        let order = [0usize, 39, 5, 5, 6, 4, 20, 21, 22, 10, 39, 0, 15];
        for &line in &order {
            let offset = starts[line];
            let expected_line = starts.partition_point(|&s| s <= offset);
            assert_eq!(
                index.to_line_column(offset),
                (expected_line, offset - starts[expected_line - 1] + 1),
                "line index {line}"
            );
        }
    }
}
