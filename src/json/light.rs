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
use alloc::{borrow::Cow, boxed::Box, string::String, vec::Vec};

#[cfg(test)]
use std::borrow::Cow;

use core::cell::{Cell, OnceCell, RefCell};

use crate::trees::BalancedParens;
use crate::util::broadword::select_in_word;
use crate::util::simd::specials::SpecialMask;

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
    /// The closer runs [`JsonCursor::container_end_bp`] has read, so the next container on the
    /// same run is answered by stepping from a known closer rather than by rescanning from the
    /// run's leaf (#5869). See [`CloseRun`].
    ///
    /// Advisory, like `seq_hint`, and left out of `Debug` for the same reason.
    close_runs: RefCell<Vec<CloseRun>>,
    /// Whether this index was built over the jq reindex bridge's own text
    /// ([`JsonIndex::build_reindex`]), the only text allowed to carry the
    /// bridge's number tokens (`NAN_SENTINEL`, `INFINITY_SENTINEL`,
    /// `computed_float_token`, #472/#1083/#2902). Every [`JsonNumber`] a
    /// cursor over this index hands out copies it, and
    /// [`JsonNumber::bridge_value`] decodes a token only when it is set.
    ///
    /// The tokens are spelled so `str::parse` rejects them, but a document
    /// can still contain the same bytes, and the scanners capture them as
    /// one span either way; without this flag a document `[9e999e999]` read
    /// as NaN (#3034). Provenance, not spelling, is what keeps the two
    /// apart.
    bridge_tokens: bool,
}

impl<W: core::fmt::Debug> core::fmt::Debug for JsonIndex<W> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("JsonIndex")
            .field("ib", &self.ib)
            .field("ib_len", &self.ib_len)
            .field("ib_rank", &self.ib_rank)
            .field("bp", &self.bp)
            .field("lines", &self.lines)
            .field("bridge_tokens", &self.bridge_tokens)
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
    /// since every constructor holds the interest bits to `u32::MAX` (#188).
    rank: u32,
    /// A text position, which passes `u32::MAX` in a text of 4 GiB or more
    /// (#7537).
    pos: usize,
}

impl SeqHint {
    const NONE: Self = Self {
        rank: u32::MAX,
        pos: 0,
    };
}

/// A run of consecutive close parentheses after one leaf, and where one of its closers sits in
/// the text: the state behind [`JsonIndex::close_run_end`] (#5869).
///
/// Every container whose close parenthesis falls in `leaf + 1..=hi` has the open parenthesis at
/// `leaf` as the last one before it, so [`JsonCursor::container_end_bp`] would count closers from
/// that same leaf. Those closers are consecutive `]`/`}` bytes in the text with no other closer
/// between them, so the closer of close `c - 1` is the last closer byte before the closer of `c`,
/// and the closer of `c + 1` the first one after it. A walk down a spine asks `c`, `c - 1`,
/// `c - 2`, ... in turn, which costs one gap each from the previous answer instead of a scan of
/// every closer from the leaf: that scan made `..` over a linear nest quadratic in its depth.
/// A walk from the innermost container outwards extends the run the same way. Only the newest
/// run is consulted, so two deep spines asked alternately still rescan.
///
/// Advisory: a lookup it cannot answer falls back to the scan, which then records the run.
#[derive(Clone, Copy, Debug)]
struct CloseRun {
    /// The open parenthesis of the run's leaf.
    leaf: usize,
    /// The lowest close in the run that has a closer byte in the text: `leaf + 1` when the leaf
    /// is an empty container (its own closer), `leaf + 2` when it is a scalar.
    first: usize,
    /// The highest close known to be in the run: every bit in `leaf + 1..=hi` is a close.
    hi: usize,
    /// The close the run last answered, in `first..=hi`.
    at: usize,
    /// One past the closer of `at`.
    end: usize,
}

/// How many runs [`JsonIndex::close_run_end`] keeps. A pre-order walk keeps one per comb level
/// it is inside (a spine whose earlier siblings have spines of their own); past this the newest
/// replaces the innermost, which only costs a rescan.
const CLOSE_RUN_CAPACITY: usize = 32;

#[cfg(test)]
std::thread_local! {
    /// How many times [`JsonCursor::container_end_bp`] scanned closers from a leaf instead of
    /// answering from a recorded run (#5869), for the test that pins the walk linear.
    static CLOSE_RUN_SCANS: Cell<usize> = const { Cell::new(0) };
}

/// Whether `b` closes a container, as [`JsonCursor::container_end_bp`] counts closers.
#[inline]
fn is_closer(b: u8) -> bool {
    matches!(b, b']' | b'}')
}

/// Build cumulative popcount index for IB.
/// Returns a vector where entry i = total 1-bits in words [0, i).
///
/// The `u32` accumulator is safe because every constructor holds the set bits
/// to `u32::MAX` ([`JsonIndex::MAX_COUNT`], #188, #7537): a text of up to
/// `u32::MAX` bytes cannot have more, and a longer one is counted first.
/// Widening to `u64` would double a hot per-word array (~6.25% of input) for
/// inputs the index cannot represent anyway.
fn build_ib_rank(words: &[u64]) -> Vec<u32> {
    let mut rank = Vec::with_capacity(words.len() + 1);
    fill_ib_rank(&mut rank, words);
    rank
}

/// [`build_ib_rank`] into `rank`'s existing buffer, which is cleared first.
#[inline]
fn fill_ib_rank(rank: &mut Vec<u32>, words: &[u64]) {
    rank.clear();
    rank.reserve(words.len() + 1);
    let mut cumulative: u32 = 0;
    rank.push(0); // rank[0] = 0 (no words before word 0)
    for &word in words {
        cumulative += word.count_ones();
        rank.push(cumulative);
    }
}

/// Whether `words` has more set bits than [`JsonIndex::MAX_COUNT`] (#7537): counted in `u64`, for a
/// text longer than `u32::MAX` bytes, before [`build_ib_rank`] counts them in `u32`.
fn ones_exceed_max_count(words: &[u64]) -> bool {
    let ones: u64 = words.iter().map(|w| u64::from(w.count_ones())).sum();
    ones > JsonIndex::MAX_COUNT as u64
}

/// [`LineIndex::to_line_column`](crate::text::LineIndex::to_line_column) by a scan of `text`, for a
/// text longer than a line index holds (#7537): the same line starts (one after every line break
/// that has text after it) and the same answer for an offset past the end.
fn scan_line_column(text: &[u8], offset: usize) -> (usize, usize) {
    let (mut line, mut start, mut at) = (1, 0, 0);
    while at < offset.min(text.len()) {
        let width = crate::text::line_break::line_break_len(text, at);
        if width == 0 {
            at += 1;
            continue;
        }
        at += width;
        if at <= offset && at < text.len() {
            line += 1;
            start = at;
        }
    }
    (line, offset - start + 1)
}

/// [`LineIndex::to_offset`](crate::text::LineIndex::to_offset) by a scan of `text`, as
/// [`scan_line_column`] is for its sibling (#7537).
fn scan_offset(text: &[u8], line: usize, column: usize) -> Option<usize> {
    if column == 0 || line == 0 {
        return None;
    }
    let (mut current, mut at) = (1, 0);
    while current < line {
        if at >= text.len() {
            return None;
        }
        let width = crate::text::line_break::line_break_len(text, at);
        if width == 0 {
            at += 1;
            continue;
        }
        at += width;
        if at >= text.len() {
            return None;
        }
        current += 1;
    }
    let offset = at + column - 1;
    (offset < text.len()).then_some(offset)
}

/// The three heap buffers of a scalar bridge index (#4217): interest bits,
/// their rank, and the leaf's balanced-parentheses word. Their contents are
/// scratch -- [`JsonIndex::build_reindex_scalar_from`] overwrites all of
/// them -- so a retired index's buffers can build the next one without
/// touching the allocator.
#[derive(Default)]
pub(crate) struct ScalarIndexParts {
    ib: Vec<u64>,
    ib_rank: Vec<u32>,
    bp_words: Vec<u64>,
}

impl ScalarIndexParts {
    /// Whether keeping these buffers around is cheap: the index they last
    /// held was that of a token of 1 KiB or less, so a free list of them never
    /// pins a big one. Judged by what they held, not by capacity, which
    /// amortized growth rounds past any fixed limit.
    #[must_use]
    pub(crate) fn is_small(&self) -> bool {
        self.ib.len() <= 16 && self.ib_rank.len() <= 17
    }
}

impl JsonIndex<Vec<u64>> {
    /// The longest text, in bytes, every index holds whatever it contains: `u32::MAX`, just under
    /// 4 GiB (#188). A longer text is held when its counts are within [`Self::MAX_COUNT`]
    /// ([`try_build`](Self::try_build), #7537).
    pub const MAX_TEXT_LEN: usize = u32::MAX as usize;

    /// The most interest bits, and the most balanced-parentheses bits, an index holds: its rank
    /// directories count both as `u32` (#188). A text of up to [`Self::MAX_TEXT_LEN`] bytes is
    /// always within it. A longer one is within it unless it holds more than `u32::MAX / 2`
    /// values, keys included, since each value has one interest bit and two parentheses: a text
    /// of two bytes a value (`[0,0,...]`) passes that at 4 GiB (#7537).
    pub const MAX_COUNT: usize = u32::MAX as usize;

    /// Whether [`build`](Self::build) accepts every text of `len` bytes, rather than panicking.
    ///
    /// A longer text may still be held: [`try_build`](Self::try_build) says (#7537).
    #[must_use]
    pub const fn can_index(len: usize) -> bool {
        len <= Self::MAX_TEXT_LEN
    }

    /// [`build`](Self::build), or `None` for a text whose interest bits or balanced parentheses
    /// pass [`Self::MAX_COUNT`] (#7537). Only a text of more than [`Self::MAX_TEXT_LEN`] bytes
    /// can, and only such a text pays for the count, after the scan.
    #[must_use]
    pub fn try_build(json: &[u8]) -> Option<Self> {
        #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
        let semi = crate::json::simd::build_semi_index_standard(json);

        #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
        let semi = crate::json::standard::build_semi_index(json);

        let ib_len = json.len();

        // Two bits per open; a malformed document short of closes gets them
        // as zero words (#7482).
        let mut bp_words = semi.bp;
        let bp_bit_count = bp_len_supplying_missing_closes(&mut bp_words);
        if !Self::can_index(ib_len)
            && (bp_bit_count > Self::MAX_COUNT || ones_exceed_max_count(&semi.ib))
        {
            return None;
        }

        // Build cumulative popcount index for IB
        let ib_rank = build_ib_rank(&semi.ib);

        Some(Self {
            ib: semi.ib,
            ib_len,
            ib_rank,
            bp: BalancedParens::new(bp_words, bp_bit_count),
            lines: OnceCell::new(),
            seq_hint: Cell::new(SeqHint::NONE),
            close_runs: RefCell::new(Vec::new()),
            bridge_tokens: false,
        })
    }

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
    /// Panics if the input's interest bits or balanced parentheses pass
    /// [`Self::MAX_COUNT`]: the rank directories store cumulative counts as
    /// `u32` (#188). Only an input of more than `u32::MAX` bytes (4 GiB) can
    /// (#7537); [`try_build`](Self::try_build) answers `None` for one instead.
    pub fn build(json: &[u8]) -> Self {
        Self::try_build(json).unwrap_or_else(|| {
            panic!(
                "JsonIndex holds at most u32::MAX (4294967295) interest bits and balanced \
                 parentheses; a text of {} bytes has more (#188, #7537)",
                json.len()
            )
        })
    }

    /// [`build`](Self::build) for text the jq reindex bridge wrote
    /// (`OwnedValue::to_json_for_reindex`/`to_json_input_bridge`), whose
    /// number tokens stand for NaN, ±Infinity and computed floats. Only a
    /// number read through an index built here decodes those tokens
    /// ([`JsonNumber::bridge_value`]); the same bytes in a user document are
    /// an ordinary malformed span (#3034).
    ///
    /// Not for user input. Reach it through `OwnedValue::reindexed` /
    /// `OwnedValue::input_bridge_doc`, which pair the text with its index so
    /// the flag cannot be forgotten.
    #[doc(hidden)]
    pub fn build_reindex(json: &[u8]) -> Self {
        let mut index = Self::build(json);
        index.bridge_tokens = true;
        index
    }

    /// [`build_reindex`](Self::build_reindex), or `None` where
    /// [`try_build`](Self::try_build) is (#7537).
    #[doc(hidden)]
    #[must_use]
    pub fn try_build_reindex(json: &[u8]) -> Option<Self> {
        let mut index = Self::try_build(json)?;
        index.bridge_tokens = true;
        Some(index)
    }

    /// [`build_reindex`](Self::build_reindex) for bridge text that is exactly
    /// one scalar token starting at offset 0 (#4154): what an
    /// `OwnedValue::reindexed` over a non-container root writes.
    ///
    /// The index of such text depends only on its length -- one interest bit
    /// at offset 0 and one leaf pair of balanced parentheses -- so it is
    /// written directly instead of running the scanner, whose temporaries and
    /// directory build dominate the cost of indexing a single token. A
    /// differential test pins the result to `build_reindex`'s on every scalar
    /// spelling the bridge writes.
    ///
    /// `len` must be the byte length of that one-token text. Anything else
    /// (a container, leading whitespace, an empty text) yields an index that
    /// does not describe it: memory-safe, but wrong. Reach it through
    /// `OwnedValue::reindexed`, which only calls it for a scalar root.
    #[doc(hidden)]
    pub fn build_reindex_scalar(len: usize) -> Self {
        Self::build_reindex_scalar_from(len, ScalarIndexParts::default())
    }

    /// [`build_reindex_scalar`](Self::build_reindex_scalar) written into
    /// `parts`' buffers (#4217), which are cleared and refilled -- their old
    /// contents never reach the result. With buffers from
    /// [`take_scalar_parts`](Self::take_scalar_parts) this makes no
    /// allocator call once they are large enough, which a token of 64 bytes
    /// or fewer always is after the first.
    ///
    /// Same contract as `build_reindex_scalar` for `len`.
    ///
    /// Any `len` is held: the index has one interest bit and one pair of
    /// parentheses whatever the token's length (#7537).
    pub(crate) fn build_reindex_scalar_from(len: usize, parts: ScalarIndexParts) -> Self {
        let ScalarIndexParts {
            mut ib,
            mut ib_rank,
            bp_words,
        } = parts;
        ib.clear();
        ib.resize(len.div_ceil(64), 0);
        if let Some(first) = ib.first_mut() {
            *first = 1;
        }
        fill_ib_rank(&mut ib_rank, &ib);
        Self {
            ib,
            ib_len: len,
            ib_rank,
            bp: BalancedParens::leaf_in(bp_words),
            lines: OnceCell::new(),
            seq_hint: Cell::new(SeqHint::NONE),
            close_runs: RefCell::new(Vec::new()),
            bridge_tokens: true,
        }
    }

    /// Take this index's buffers for the next
    /// [`build_reindex_scalar_from`](Self::build_reindex_scalar_from),
    /// leaving `self` empty (#4217). Only for an index that call (or
    /// `build_reindex_scalar`) built: it is the retired scalar document's
    /// last act, and `self` must be dropped afterwards, never read.
    pub(crate) fn take_scalar_parts(&mut self) -> ScalarIndexParts {
        self.ib_len = 0;
        self.close_runs.get_mut().clear();
        ScalarIndexParts {
            ib: core::mem::take(&mut self.ib),
            ib_rank: core::mem::take(&mut self.ib_rank),
            bp_words: self.bp.take_words(),
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
    /// Panics if `ib` has more than `u32::MAX` set bits (#188, #7537): the IB
    /// rank directory stores cumulative counts as `u32`. Only an `ib_len` past
    /// `u32::MAX` can. (`BalancedParens` asserts its own ceiling on `bp_len`.)
    pub fn from_parts(ib: W, ib_len: usize, bp: W, bp_len: usize) -> Self {
        assert!(
            JsonIndex::can_index(ib_len) || !ones_exceed_max_count(ib.as_ref()),
            "JsonIndex holds at most u32::MAX (4294967295) interest bits; {ib_len} bits have more \
             (#188, #7537)"
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
            close_runs: RefCell::new(Vec::new()),
            bridge_tokens: false,
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

    /// One past the closer of the close parenthesis `close`, from a run a previous
    /// [`JsonCursor::container_end_bp`] recorded (#5869), or `None` when no recorded run holds
    /// it. `text` is the text this index was built from.
    ///
    /// Runs entirely before `close` are dropped as they are met: a pre-order walk that has moved
    /// past a run never asks it again. A run that ends just before `close` is extended when the
    /// bits between are all closes.
    fn close_run_end(&self, close: usize, text: &[u8]) -> Option<usize> {
        let mut runs = self.close_runs.borrow_mut();
        loop {
            let run = runs.last_mut()?;
            if close <= run.leaf {
                return None;
            }
            if close > run.hi {
                // Every bit in `hi + 1..=close` is a close iff no open parenthesis lies there.
                let all_closes = match last_open_before(self.bp.words(), close + 1) {
                    Some(open) => open <= run.hi,
                    None => true,
                };
                if !all_closes {
                    runs.pop();
                    continue;
                }
                run.hi = close;
            }
            if close < run.first {
                return None;
            }
            let mut end = run.end;
            for _ in close..run.at {
                // The closer of the close before: the last closer byte before this one.
                let mut q = end.checked_sub(1)?;
                loop {
                    q = q.checked_sub(1)?;
                    if is_closer(*text.get(q)?) {
                        break;
                    }
                }
                end = q + 1;
            }
            for _ in run.at..close {
                // The closer of the close after: the first closer byte after this one.
                let mut i = end;
                while !is_closer(*text.get(i)?) {
                    i += 1;
                }
                end = i + 1;
            }
            run.at = close;
            run.end = end;
            return Some(end);
        }
    }

    /// Record a run [`JsonCursor::container_end_bp`] scanned, for [`close_run_end`](Self::close_run_end).
    fn record_close_run(&self, run: CloseRun) {
        let mut runs = self.close_runs.borrow_mut();
        if runs.len() == CLOSE_RUN_CAPACITY {
            runs.pop();
        }
        runs.push(run);
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
    ///
    /// A text longer than [`LineIndex`](crate::text::LineIndex) holds (4 GiB or more) is scanned
    /// on each call instead (#7537).
    #[inline]
    pub fn to_line_column(&self, offset: usize, text: &[u8]) -> (usize, usize) {
        if !JsonIndex::can_index(text.len()) {
            return scan_line_column(text, offset);
        }
        self.ensure_lines(text).to_line_column(offset)
    }

    /// Convert 1-indexed line and column to byte offset.
    ///
    /// Column is 1-indexed byte offset within the line.
    /// Returns `None` if line/column is 0 or if the position is out of bounds.
    ///
    /// `text` must be the JSON text the index was built from; the line index
    /// is derived from it on first use and cached (#228).
    ///
    /// A text of 4 GiB or more is scanned on each call, as for [`Self::to_line_column`] (#7537).
    #[inline]
    pub fn to_offset(&self, line: usize, column: usize, text: &[u8]) -> Option<usize> {
        if !JsonIndex::can_index(text.len()) {
            return scan_offset(text, line, column);
        }
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
        let (result, _probes) = crate::util::gallop::select1_from(
            self.ib.as_ref(),
            &self.ib_rank,
            self.ib_len,
            k,
            hint,
        )?;
        #[cfg(any(feature = "select-stats", test))]
        crate::util::select_stats::record(
            crate::util::select_stats::Site::JsonIbSelectFrom,
            _probes,
        );
        Some(result)
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
            return Some(last.pos);
        }
        let hint = self.seed_word(k, last);
        let result = self.ib_select1_from(k, hint);
        if let Some(pos) = result {
            // The rank fits: a `Some` means `k < ones`, and every
            // constructor holds the ones to `u32::MAX` (#188, #7537).
            self.seq_hint.set(SeqHint {
                rank: k as u32,
                pos,
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
            (i64::from(last.rank), (last.pos / 64) as i64)
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

/// The balanced-parentheses length of a standard semi-index, two bits per open,
/// with `bp_words` extended to hold every one of them (#7482).
///
/// The scanners record no length, so it is derived as `2 * opens`: the exact
/// length of a balanced document. A malformed one can be short of closes
/// (`{"a":[[[1,"b":2}` leaves two arrays open at end of input), and the
/// missing closes are then the zero bits after the last bit written. While
/// they fit in the last word nothing more is needed; a deficit that crossed a
/// word boundary left `len` past the last stored word, and navigation
/// (`is_open` under `next_sibling`) indexed out of bounds. Zero words appended
/// here supply those closes, so `len <= 64 * words` holds for every input:
/// `is_open`, and the sibling and child walks through it, read `words[p / 64]`
/// for any `p < len` (`find_close` and `rank1` tolerate a shorter slice,
/// #3140). A balanced document already holds all its bits and is not touched.
/// (A surplus of closes is the other malformed shape: those past `len` are
/// simply not part of the sequence.)
fn bp_len_supplying_missing_closes(bp_words: &mut Vec<u64>) -> usize {
    let total_ones: usize = bp_words.iter().map(|w| w.count_ones() as usize).sum();
    let len = total_ones * 2;
    let needed_words = len.div_ceil(64);
    if bp_words.len() < needed_words {
        bp_words.resize(needed_words, 0);
    }
    len
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

    /// Check if this cursor points to a container (array or object) that has
    /// children.
    ///
    /// This is a **fast** operation that only uses the BP structure -
    /// no text_position lookup is needed. Containers have children in
    /// the BP tree; leaves (strings, numbers, bools, null) don't. An
    /// **empty** `{}`/`[]` has none either, so it answers `false` here:
    /// this is "has a subtree", not an array/object type test (#3180).
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

    /// One past the closing bracket of the container this cursor opens, from the balanced
    /// parentheses and not the text (#5654): the last node in document order inside it is a
    /// leaf (a scalar, or an empty container), so the container ends at the last of the closers
    /// that follow that leaf, one for each parenthesis that closes after it.
    ///
    /// Only the closers are looked for: a byte between them that is not one is the container's
    /// own business, found when it is read, and is skipped here. That keeps a filter that never
    /// reads a malformed member from being made to validate it (#2103), while a byte after the
    /// container's last closer is what the caller's gap check sees.
    ///
    /// A scan of the text ([`container_end`]) is a pass over the container, so a document that is
    /// one big array under one key paid a second pass over itself; this reads the leaf and the
    /// text after it. The kind of an inner closer is not compared with its opener's.
    ///
    /// The closers read are recorded on the index (`CloseRun`, #5869): the containers along a
    /// spine share their leaf, and a walk asks them one after another, so each is answered from
    /// the one asked before it. Rescanning from the leaf every time made `..` over a linear nest
    /// quadratic in its depth.
    ///
    /// `None` where the end cannot be told (a leaf that has no span, or fewer closers than
    /// parentheses).
    pub fn container_end_bp(&self) -> Option<usize>
    where
        W: Clone,
    {
        let bp = self.index.bp();
        let close = bp.find_close(self.bp_pos)?;
        if let Some(end) = self.index.close_run_end(close, self.text) {
            return Some(end);
        }
        #[cfg(test)]
        CLOSE_RUN_SCANS.with(|n| n.set(n.get() + 1));
        let leaf_open = last_open_before(bp.words(), close)?;
        if leaf_open < self.bp_pos {
            return None;
        }
        let leaf = JsonCursor::from_bp_position(self.index, self.text, leaf_open);
        let start = leaf.text_position()?;
        let (mut i, closers) = match *self.text.get(start)? {
            // An empty container: its closer is part of the leaf, so it is one of the closers.
            b'{' | b'[' => (start + 1, close - leaf_open),
            _ => (
                scalar_end_pos(start, &leaf.value_at(start))?,
                close - leaf_open - 1,
            ),
        };
        for _ in 0..closers {
            while !is_closer(*self.text.get(i)?) {
                i += 1;
            }
            i += 1;
        }
        if closers > 0 {
            self.index.record_close_run(CloseRun {
                leaf: leaf_open,
                first: close + 1 - closers,
                hi: close,
                at: close,
                end: i,
            });
        }
        Some(i)
    }

    /// Where the child at `text_pos` (this cursor's own `text_position()`) ends, for the
    /// trailing-gap check after a container's last child (#5654): a scalar's own length, or the
    /// scan [`container_end`] makes for a container. `None` where the end cannot be told.
    pub fn last_child_text_end(&self, text_pos: usize) -> Option<usize>
    where
        W: Clone,
    {
        let at = text_pos;
        match self.text.get(at)? {
            b'{' | b'[' => self.container_end_bp(),
            _ => scalar_end_pos(at, &self.value_at(at)),
        }
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
                // true or false. Same whole-token rule as `keyword_span`
                // applies: `truex`/`falsey` are not `true`/`false` (#3035).
                if keyword_span(self.text, text_pos, b"true").is_some() {
                    StandardJson::Bool(true)
                } else if keyword_span(self.text, text_pos, b"false").is_some() {
                    StandardJson::Bool(false)
                } else {
                    StandardJson::Error("invalid boolean")
                }
            }
            b'n' => {
                if keyword_span(self.text, text_pos, b"null").is_some() {
                    StandardJson::Null
                } else if special_number_end(self.text, text_pos).is_some() {
                    // `nan`, `NaN5`: decNumber's NaN shares its first byte
                    // with `null` (#2877). `null` stays first and
                    // unchanged, so a genuine null takes no new branch;
                    // the word test runs only once that prefix test has
                    // already failed, i.e. in what was the error arm.
                    self.number_at(text_pos)
                } else {
                    StandardJson::Error("invalid null")
                }
            }
            // A leading `.` is accepted here too (in addition to `-`/an
            // ASCII digit) -- real jq's own number reader is lenient
            // beyond strict JSON (`.5` -> `0.5`, #1171). `number_at`
            // decides from the whole span whether it is a number at all;
            // a malformed one (`1.2.3`) is an `Error`, raised by whatever
            // reads it (#3222).
            c if c == b'-' || c == b'.' || c.is_ascii_digit() => self.number_at(text_pos),
            // jq's remaining number spellings (#2877), each an error arm
            // until now so valid RFC 8259 input still reaches none of
            // them: a leading `+` before a digit or `.` (`+1`, `+.5`,
            // `+1.2.3` -- the last resolving to one span and then an
            // `Error`, exactly as `1.2.3` does), and decNumber's special
            // words starting with `i`/`I`/`N`/`s`/`S` (`inf`, `Infinity`,
            // `NaN`, `sNaN12`) or with `+` (`+inf`, `+nan`). A word that
            // isn't one of those (`infx`, `nanx`, `Nope`) and a `+` before
            // anything else (`+`, `+-1`, `+x`) stay errors here, before any
            // span is taken, so the #1643 delimiter-gap check keeps seeing
            // the span it always did.
            b'+' if matches!(self.text.get(text_pos + 1), Some(b'0'..=b'9' | b'.')) => {
                self.number_at(text_pos)
            }
            b'+' | b'i' | b'I' | b'N' | b's' | b'S'
                if special_number_end(self.text, text_pos).is_some() =>
            {
                self.number_at(text_pos)
            }
            _ => StandardJson::Error("unexpected character"),
        }
    }

    /// The number whose span starts at `text_pos`, or an `Error` when the
    /// span is not a number jq reads (#3222).
    ///
    /// jq's parser rejects the whole document for a malformed number
    /// (`[1.2.3]`, `[1ee5]`). The semi-index does not validate up front, so
    /// the rejection happens here instead, where the value is read -- as a
    /// malformed object member's does (#1194) -- and every route that reads
    /// the value raises, while one that never reaches it (`.[1]`, `length`)
    /// still answers: ADR-0018's #2103 amendment. Until #3222 the span was a
    /// `Number` that the funnels read as `null` (#966) and `type` read as
    /// `"number"`.
    ///
    /// The span is measured once, here, and kept in the `JsonNumber`, so the
    /// check costs no extra pass over the digits: a strict scan settles an
    /// ordinary number and yields its end, which `raw_bytes` would otherwise
    /// have found with the greedy scan. Anything else -- a lenient spelling,
    /// a decNumber word, a bridge token, a malformed span -- takes the
    /// greedy span and asks
    /// [`number_span_decodes`](crate::json::validate::number_span_decodes),
    /// after [`JsonNumber::bridge_value`] and [`JsonNumber::bridge_spelling`],
    /// which alone may decode a bridge token (#3034, #2802).
    #[inline]
    fn number_at(&self, text_pos: usize) -> StandardJson<'a, W> {
        if let Some(end) = strict_number_end(self.text, text_pos) {
            return StandardJson::Number(JsonNumber {
                text: self.text,
                start: text_pos,
                len: u32::try_from(end - text_pos).unwrap_or(UNMEASURED_SPAN),
                bridge: self.index.bridge_tokens,
            });
        }
        self.lenient_number_at(text_pos)
    }

    /// [`number_at`](Self::number_at) for a span [`strict_number_end`] did not settle: a lenient
    /// spelling, a decNumber word, a bridge token or a malformed span.
    ///
    /// Out of line and cold so that `number_at`, which every number a document walk reads goes
    /// through, is the strict scan and nothing else (#7384). With this and
    /// `lenient_number_literal` split out, `.numbers | add` and `.numbers | max` over 100 MB of
    /// floats measured about 2% faster on an M4 Pro.
    #[cold]
    #[inline(never)]
    fn lenient_number_at(&self, text_pos: usize) -> StandardJson<'a, W> {
        let end = nested_number_span(self.text, text_pos);
        let n = JsonNumber {
            text: self.text,
            start: text_pos,
            len: u32::try_from(end - text_pos).unwrap_or(UNMEASURED_SPAN),
            bridge: self.index.bridge_tokens,
        };
        if n.bridge_value().is_some()
            || n.bridge_spelling().is_some()
            || crate::json::validate::number_span_decodes(&self.text[text_pos..end])
        {
            StandardJson::Number(n)
        } else {
            StandardJson::Error(MALFORMED_NUMBER)
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
            // Boolean true. Same whole-token rule as `keyword_span` applies
            // (#3035): `truex` is not `true`.
            b't' => {
                if keyword_span(self.text, start, b"true").is_some() {
                    start + 4
                } else {
                    return None;
                }
            }
            // Boolean false
            b'f' => {
                if keyword_span(self.text, start, b"false").is_some() {
                    start + 5
                } else {
                    return None;
                }
            }
            // Null -- or decNumber's NaN, which shares the first byte
            // (#2877); the `value_at` arm above explains the ordering.
            b'n' => {
                if keyword_span(self.text, start, b"null").is_some() {
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

    /// This node's source span when it is already exactly what a compact,
    /// default-convention (`JsonConvention::JqCompat`) render would write for
    /// it, or `None` when it is not or cannot be told (#2608, #3909).
    ///
    /// The echo decision `stream_json` makes, lifted out so the CLI's
    /// general-path printer can make it too: that printer (`print_json`)
    /// walks the node field by field where this reads it once. `Some` is
    /// both the output and its validation -- the scan is the structural check
    /// a re-render would otherwise supply, see
    /// `canonical_compact_jq_span_end` -- so a caller that gets one has
    /// nothing left to check about the node's text. It does not say the node
    /// is cheap to *evaluate*: only that its text is well formed and
    /// canonical.
    ///
    /// `depth` is how many levels down the node is counted to be, against
    /// [`MAX_VALUE_TREE_DEPTH`]: the printer's own `level`, or a larger number
    /// for a caller that owes a tighter ceiling than the printer's.
    ///
    /// Only the node's *start* is taken from the cursor. The end comes out of
    /// the scan itself -- see `canonical_compact_jq_span_end`'s own doc
    /// comment for why that span is exactly this node's, and for the
    /// 28%-of-the-gate `text_range` rescan this avoids (`raw_bytes` walks the
    /// whole container a second time just to find the closing bracket the
    /// scan is about to reach anyway).
    pub fn canonical_compact_span(&self, depth: usize) -> Option<&'a str> {
        let rest = self.text.get(self.text_position()?..)?;
        let end = canonical_compact_jq_span_end_at_depth(rest, depth)?;
        self.certified_span(rest, end)
    }

    /// [`canonical_compact_span`](Self::canonical_compact_span), also saying
    /// when the node is an object the scan declined only because its keys'
    /// hashes repeat ([`CanonicalSpan::RepeatedKeys`], #7310).
    ///
    /// Or when the node is canonical but for one object inside it that the
    /// scan declined that way ([`CanonicalSpan::NestedRepeatedKeys`], #7366).
    pub(crate) fn canonical_compact_scan(&self, depth: usize) -> CanonicalSpan<'a> {
        let Some((start, rest)) = self
            .text_position()
            .and_then(|start| Some((start, self.text.get(start..)?)))
        else {
            return CanonicalSpan::Declined;
        };
        match canonical_compact_jq_scan_at_depth(rest, depth) {
            CanonicalScan::Canonical(end) => match self.certified_span(rest, end) {
                Some(span) => CanonicalSpan::Echo(span),
                None => CanonicalSpan::Declined,
            },
            // The span must be valid UTF-8, as an echo's must, and for a
            // reason of its own: then every key decodes (a certified escape
            // always does), which is what lets the caller's plan compare only
            // the keys whose hash is shared. A key that does not decode makes
            // the plan compare every key (`repeat_plan_checked`), so that
            // object takes its route.
            CanonicalScan::RepeatedKeys { end, shared } => match self.certified_span(rest, end) {
                Some(span) => CanonicalSpan::RepeatedKeys {
                    start,
                    span,
                    shared,
                },
                None => CanonicalSpan::Declined,
            },
            // The whole span is checked, as an echo's is, which covers the
            // object's keys too (see the arm above).
            CanonicalScan::NestedRepeatedKeys { end, object } => {
                match self.certified_span(rest, end) {
                    Some(span) => CanonicalSpan::NestedRepeatedKeys { span, object },
                    None => CanonicalSpan::Declined,
                }
            }
            CanonicalScan::Declined => CanonicalSpan::Declined,
        }
    }

    /// The `end` bytes of `rest` the canonical scan accepted, as text, or
    /// `None` when they are not valid UTF-8.
    fn certified_span(&self, rest: &'a [u8], end: usize) -> Option<&'a str> {
        debug_assert_eq!(
            self.text_range().map(|(start, stop)| stop - start),
            Some(end),
            "the canonical scan and `text_range` must agree on an accepted span"
        );
        // The scan certifies "canonical *given* valid UTF-8" and no longer
        // decodes the sequences itself, so this is the half of the decision
        // that rules on UTF-8 -- one vectorised pass instead of the scan's
        // old byte-at-a-time decode. A failure is *not* an error of this
        // method's: callers fall through to the re-render, which is what
        // raises the real decode diagnostic.
        core::str::from_utf8(&rest[..end]).ok()
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
pub struct JsonFields<'a, W = Vec<u64>> {
    /// The current field key's cursor, or `None` once exhausted -- **or**,
    /// for an object that was empty from the start, that object's own
    /// cursor with [`EMPTY_CONTAINER_TAG`] set in its `bp_pos` (#3180).
    ///
    /// Never read directly: navigation goes through
    /// [`current`](Self::current), which sees a tagged slot as `None`, and
    /// only [`whole_container_cursor`](Self::whole_container_cursor) asks for
    /// the tagged container, through [`empty_container`](Self::empty_container).
    slot: Option<JsonCursor<'a, W>>,
}

/// The `bp_pos` bit a [`JsonFields`]/[`JsonElements`] slot sets to mean "this
/// list is an *empty* container, and the cursor is the container's own"
/// (#3180).
///
/// A list keeps a *child* cursor, so an empty `{}`/`[]` used to keep nothing
/// at all, and `jq::eval`'s bind sites could not name its node. Holding the
/// container cursor next to the slot would grow both `Copy` types by a word
/// on the hottest JSON iteration path; the top bit of a BP position is free
/// instead (a BP vector that long does not fit in the address space on a
/// 64-bit target), so the tag costs no bytes.
///
/// On a target narrower than 64 bits the bit is *not* free -- a real child's
/// position can reach it on a multi-GiB input, and [`slot_current`] would
/// then read that child as the end of its list and silently truncate the
/// iteration. There the tag is `0`, [`TAGGING`] is off, and an empty
/// container is stored as plain `None`: an under-report (its bind names no
/// node, exactly as before #3180), never a wrong answer.
const EMPTY_CONTAINER_TAG: usize = if TAGGING { 1 << (usize::BITS - 1) } else { 0 };

/// Whether [`EMPTY_CONTAINER_TAG`] may be used: only where the top bit of a
/// BP position cannot belong to a real node.
const TAGGING: bool = usize::BITS >= 64;

// Without tagging the tag must be inert: `slot_current` masks with it.
const _: () = assert!(TAGGING || EMPTY_CONTAINER_TAG == 0);

/// The slot a freshly opened container's list starts from: its first child,
/// or the container itself tagged with [`EMPTY_CONTAINER_TAG`] when it has
/// none. The one place a tag is written; `uncons` builds `rest` from
/// `next_sibling()`, so an *exhausted* list is plain `None` and never names
/// the container a suffix of it came from.
#[inline]
fn container_slot<W: AsRef<[u64]>>(container: JsonCursor<'_, W>) -> Option<JsonCursor<'_, W>> {
    container.first_child().or_else(|| {
        TAGGING.then_some(JsonCursor {
            bp_pos: container.bp_pos | EMPTY_CONTAINER_TAG,
            ..container
        })
    })
}

/// A slot's navigable cursor: `None` for a tagged empty container.
#[inline(always)]
fn slot_current<W>(slot: Option<JsonCursor<'_, W>>) -> Option<JsonCursor<'_, W>> {
    slot.filter(|c| c.bp_pos & EMPTY_CONTAINER_TAG == 0)
}

/// A slot's tagged empty container, untagged; `None` for any other slot.
#[inline]
fn slot_empty_container<W>(slot: Option<JsonCursor<'_, W>>) -> Option<JsonCursor<'_, W>> {
    slot.filter(|c| c.bp_pos & EMPTY_CONTAINER_TAG != 0)
        .map(|c| JsonCursor {
            bp_pos: c.bp_pos & !EMPTY_CONTAINER_TAG,
            ..c
        })
}

// Manual Clone/Copy impl since JsonCursor is Copy
impl<W> Clone for JsonFields<'_, W> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<W> Copy for JsonFields<'_, W> {}

/// Prints the decoded slot, so the empty-container tag never shows up as a
/// huge `bp_pos` in `{:?}` output (#3180). A list that is not an untouched
/// empty container prints exactly the one field it always did.
impl<W: core::fmt::Debug> core::fmt::Debug for JsonFields<'_, W> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut shown = f.debug_struct("JsonFields");
        shown.field("key_cursor", &self.current());
        if self.empty_container().is_some() {
            shown.field("empty_container", &true);
        }
        shown.finish()
    }
}

impl<'a, W> JsonFields<'a, W> {
    /// The current key's cursor; `None` when exhausted or empty (#3180).
    #[inline(always)]
    fn current(&self) -> Option<JsonCursor<'a, W>> {
        slot_current(self.slot)
    }

    /// The object's own cursor, when the list is an object that was empty
    /// from the start (#3180).
    #[inline]
    fn empty_container(&self) -> Option<JsonCursor<'a, W>> {
        slot_empty_container(self.slot)
    }
}

impl<'a, W: AsRef<[u64]>> JsonFields<'a, W> {
    /// Create a new JsonFields from an object cursor.
    fn from_object_cursor(object_cursor: JsonCursor<'a, W>) -> Self {
        Self {
            slot: container_slot(object_cursor),
        }
    }

    /// Check if there are no more fields.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.current().is_none()
    }

    /// The document text the retained cursor reads, or `None` for an
    /// exhausted list -- no BP work, unlike [`whole_container_cursor`](Self::whole_container_cursor),
    /// so a caller that only needs to know *which* document a list belongs
    /// to can ask before paying for the hop (#3069). A list that was empty
    /// from the start keeps its container's cursor (#3180), so it answers
    /// that container's document, as `whole_container_cursor` answers its
    /// node.
    pub(crate) fn document_text(&self) -> Option<&'a [u8]> {
        self.slot.map(|c| c.text)
    }

    /// The cursor of the object this field list belongs to, but **only**
    /// while the list still stands at that object's first child -- i.e.
    /// while `StandardJson::Object(self)` really is the whole object and not
    /// a partly-consumed tail of it (#2889).
    ///
    /// `JsonFields` retains a child cursor, not the container's own
    /// ([`from_object_cursor`](Self::from_object_cursor) stores
    /// `first_child()`), so the container is one `parent()` hop away. That
    /// hop answers the same node for *every* child, which is exactly why the
    /// first-child check is needed: after an
    /// [`uncons`](Self::uncons) the value in hand is a suffix, and naming the
    /// whole object for it would claim a node identity the value does not
    /// have.
    ///
    /// An object that was empty from the start retains no child to hop from;
    /// its list keeps the object's own cursor tagged instead (#3180), and
    /// that is what this answers for it. `None` for any advanced list -- an
    /// under-report: a caller that needs a node identity simply does without
    /// one, which costs an acceptance, never a wrong answer.
    ///
    /// `pub(crate)`: this exists for `jq::eval`'s bind sites, which have no
    /// cursor of their own to consult, and is not a shape for this type's
    /// public iteration API to grow.
    #[inline]
    pub(crate) fn whole_container_cursor(&self) -> Option<JsonCursor<'a, W>> {
        if let Some(container) = self.empty_container() {
            return Some(container);
        }
        let child = self.current()?;
        let container = child.parent()?;
        (container.first_child()?.bp_pos == child.bp_pos).then_some(container)
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
        let key_cursor = self.current()?;
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
        let key_cursor = self.current()?;

        // Next sibling of key is the value
        let value_cursor = key_cursor.next_sibling()?;

        // The rest starts at the value's next sibling (the next key, if any)
        let rest = JsonFields {
            slot: value_cursor.next_sibling(),
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
            if json_key_is_malformed(&key) {
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
                if key_str.decodes_to(name) {
                    winner = Some((key_str.start(), field, index == 0));
                }
            }
            last_value_cursor = Some(field.value_cursor());
            fields = rest;
            index += 1;
        }
        // #7448: a list the walk stopped on with a child still in hand ends on an
        // unpaired one. Behind a member the trailing-gap check below also
        // refuses it, but the lone child of a one-member `{"a":}`, `{"a"}` or
        // `{:1}` leaves no last value to check, so the lookup answered `null`.
        // O(1): a list that ran out holds no child.
        if fields.ends_unpaired() {
            return Err(fields.malformed_member_error());
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
        self.find_cursor_counted(name).0
    }

    /// [`find_cursor`](Self::find_cursor), plus the number of members the
    /// walk visited (#3913) -- the loop already counts them for the
    /// first-member test, so the count is free. Meaningful only for an `Ok`;
    /// an `Err` reports `0`.
    #[inline]
    pub fn find_cursor_counted(
        &self,
        name: &str,
    ) -> (Result<Option<JsonCursor<'a, W>>, EvalError>, usize)
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
            if json_key_is_malformed(&key) {
                return (Err(fields.malformed_member_error()), 0);
            }
            // Same undecodable-key skip as `find` above (#1247).
            if let StandardJson::String(key) = key {
                if key.decodes_to(name) {
                    winner = Some((key.start(), field.value_cursor(), index == 0));
                }
            }
            last_value_cursor = Some(field.value_cursor());
            fields = rest;
            index += 1;
        }
        // #7448: the unpaired lone child of a one-member object, as in `find`.
        if fields.ends_unpaired() {
            return (Err(fields.malformed_member_error()), 0);
        }
        // STYLE-0013: same reason as `find`'s own exemption -- only
        // `key_start` (captured when a candidate becomes `winner`) is kept,
        // and the check is deferred to the last-write-wins field alone.
        if let Some((key_start, value_cursor, is_first)) = winner {
            let comma_expected = if is_first { None } else { Some(b',') };
            if !preceding_gap_ok(value_cursor.text(), key_start, comma_expected) {
                return (Err(EvalError::malformed_json_text(value_cursor.text())), 0);
            }
            if let Some(value_start) = value_cursor.text_position() {
                if !preceding_gap_ok(value_cursor.text(), value_start, Some(b':')) {
                    return (Err(EvalError::malformed_json_text(value_cursor.text())), 0);
                }
            }
        }
        // #2261: trailing stray comma after the object's real last field
        // (`{"a":1,}`), checked regardless of whether `name` matched
        // anything -- see this function's own doc comment above.
        if let Some(last) = last_value_cursor {
            if !trailing_element_gap_ok(&last, b'}') {
                return (Err(EvalError::malformed_json_text(last.text())), 0);
            }
        }
        (Ok(winner.map(|(_, value_cursor, _)| value_cursor)), index)
    }

    /// [`find_cursor_counted`](Self::find_cursor_counted) for several names in one walk of the
    /// members (#7260): `found[i]` (as long as `names`) is what the lookup of `names[i]`
    /// answers, and an `Err` is what the first of those lookups to fail raises.
    ///
    /// The checks are the one-name walk's, in its order for each name: every key is a string,
    /// each winner's `,`/`:` gaps, and the gap after the last member. Every error among them is
    /// one the others share -- the walk's own key fault, whatever the name, or
    /// [`EvalError::malformed_json_text`] of this document -- so checking every winner before
    /// the trailing gap raises what the names' lookups in turn would have.
    pub fn find_cursors_counted(
        &self,
        names: &[&str],
        found: &mut [Option<JsonCursor<'a, W>>],
    ) -> (Result<(), EvalError>, usize)
    where
        W: Clone,
    {
        debug_assert_eq!(names.len(), found.len());
        if names.len() > MAX_NAMES_PER_WALK {
            return crate::jq::document::find_cursors_one_by_one(self, names, found);
        }
        // STYLE-0013-TAIL: `find_cursor_counted`'s walk for several names, exempt for its reason: the
        // zero-member `{,}` needs the container cursor this walk never receives, and is closed where
        // the array of reads is answered (`comma_field_reads`, as at the `Expr::Field` arm, #2594).
        // (key's own text start, is this field the object's first) of each name's winner so far;
        // its value cursor goes in `found`.
        let mut winners = [(0usize, false); MAX_NAMES_PER_WALK];
        for slot in found.iter_mut() {
            *slot = None;
        }
        let mut fields = *self;
        let mut last_value_cursor: Option<JsonCursor<'a, W>> = None;
        let mut index = 0usize;
        while let Some((field, rest)) = fields.uncons() {
            let key = field.key();
            if json_key_is_malformed(&key) {
                return (Err(fields.malformed_member_error()), 0);
            }
            if let StandardJson::String(key) = key {
                for (i, name) in names.iter().enumerate() {
                    if key.decodes_to(name) {
                        winners[i] = (key.start(), index == 0);
                        found[i] = Some(field.value_cursor());
                    }
                }
            }
            last_value_cursor = Some(field.value_cursor());
            fields = rest;
            index += 1;
        }
        // #7448: the unpaired lone child of a one-member object, as in `find`.
        if fields.ends_unpaired() {
            return (Err(fields.malformed_member_error()), 0);
        }
        // STYLE-0013: as `find_cursor_counted`'s own exemption -- only each winner's `key_start` is
        // kept, and the check is deferred to the last-write-wins field of each name alone.
        for (slot, &(key_start, is_first)) in found.iter().zip(winners.iter()) {
            let Some(value_cursor) = slot else { continue };
            let comma_expected = if is_first { None } else { Some(b',') };
            if !preceding_gap_ok(value_cursor.text(), key_start, comma_expected) {
                return (Err(EvalError::malformed_json_text(value_cursor.text())), 0);
            }
            if let Some(value_start) = value_cursor.text_position() {
                if !preceding_gap_ok(value_cursor.text(), value_start, Some(b':')) {
                    return (Err(EvalError::malformed_json_text(value_cursor.text())), 0);
                }
            }
        }
        if let Some(last) = last_value_cursor {
            if !trailing_element_gap_ok(&last, b'}') {
                return (Err(EvalError::malformed_json_text(last.text())), 0);
            }
        }
        (Ok(()), index)
    }
}

/// The most names [`JsonFields::find_cursors_counted`] looks up in its one walk; a longer list is
/// looked up a name at a time.
const MAX_NAMES_PER_WALK: usize = 8;

/// [`key_is_malformed`](crate::jq::document::key_is_malformed) for a JSON key, without decoding it (#7260): its `decoded_key_str` answers
/// `Ok(None)`, the grammar fault, exactly for a key that is not a string, and a string's decode
/// (`Ok` or the #1247 `Err`) is not that. A field lookup asked it of every key it walked, which
/// decoded each one a second time.
#[inline]
fn json_key_is_malformed<W>(key: &StandardJson<'_, W>) -> bool {
    !matches!(key, StandardJson::String(_))
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
pub struct JsonElements<'a, W = Vec<u64>> {
    /// The current element's cursor, or `None` once exhausted -- or a tagged
    /// empty array's own cursor. Same encoding and same reading rules as
    /// [`JsonFields`]'s slot (#3180).
    slot: Option<JsonCursor<'a, W>>,
}

// Manual Clone/Copy impl since JsonCursor is Copy
impl<W> Clone for JsonElements<'_, W> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<W> Copy for JsonElements<'_, W> {}

/// See [`JsonFields`]'s `Debug` (#3180).
impl<W: core::fmt::Debug> core::fmt::Debug for JsonElements<'_, W> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut shown = f.debug_struct("JsonElements");
        shown.field("element_cursor", &self.current());
        if self.empty_container().is_some() {
            shown.field("empty_container", &true);
        }
        shown.finish()
    }
}

impl<'a, W> JsonElements<'a, W> {
    /// See [`JsonFields::current`].
    #[inline(always)]
    fn current(&self) -> Option<JsonCursor<'a, W>> {
        slot_current(self.slot)
    }

    /// See [`JsonFields::empty_container`].
    #[inline]
    fn empty_container(&self) -> Option<JsonCursor<'a, W>> {
        slot_empty_container(self.slot)
    }
}

impl<'a, W: AsRef<[u64]>> JsonElements<'a, W> {
    /// Create a new JsonElements from an array cursor.
    fn from_array_cursor(array_cursor: JsonCursor<'a, W>) -> Self {
        Self {
            slot: container_slot(array_cursor),
        }
    }

    /// Check if there are no more elements.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.current().is_none()
    }

    /// See [`JsonFields::document_text`].
    pub(crate) fn document_text(&self) -> Option<&'a [u8]> {
        self.slot.map(|c| c.text)
    }

    /// The cursor of the array this element list belongs to, but **only**
    /// while the list still stands at that array's first element (#2889) --
    /// [`JsonFields::whole_container_cursor`]'s twin, with the same
    /// first-child rule, the same reasons for it, the same answer for an
    /// array empty from the start (#3180), and the same `pub(crate)`.
    #[inline]
    pub(crate) fn whole_container_cursor(&self) -> Option<JsonCursor<'a, W>> {
        if let Some(container) = self.empty_container() {
            return Some(container);
        }
        let child = self.current()?;
        let container = child.parent()?;
        (container.first_child()?.bp_pos == child.bp_pos).then_some(container)
    }

    /// Get the first element and the remaining elements.
    ///
    /// Returns `None` if there are no more elements.
    pub fn uncons(&self) -> Option<(StandardJson<'a, W>, Self)> {
        let element_cursor = self.current()?;

        let rest = JsonElements {
            slot: element_cursor.next_sibling(),
        };

        let value = element_cursor.value();
        Some((value, rest))
    }

    /// Get the first element's cursor and the remaining elements.
    ///
    /// This is like `uncons` but returns the cursor instead of the value.
    /// Useful for lazy evaluation where you want to defer calling `value()`.
    pub fn uncons_cursor(&self) -> Option<(JsonCursor<'a, W>, Self)> {
        let element_cursor = self.current()?;

        let rest = JsonElements {
            slot: element_cursor.next_sibling(),
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
        let mut cursor = self.current()?;

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
        // Skip opening quote. The scan for the closing quote already meets every backslash, so
        // it reports whether there was one rather than a second pass searching the same bytes
        // for it (#5594).
        let start = self.start + 1;
        let (end, escaped) = self.find_string_end_and_escape();

        let bytes = &self.text[start..end];

        // Check if we need to decode escapes
        if !escaped {
            // No escapes - can return directly (zero-copy)
            let s = core::str::from_utf8(bytes).map_err(|_| JsonError::InvalidUtf8)?;
            Ok(Cow::Borrowed(s))
        } else {
            // Has escapes - need to decode
            decode_escapes(bytes).map(Cow::Owned)
        }
    }

    /// Whether the string decodes to `name`: `self.as_str().is_ok_and(|k| k == name)`, without
    /// decoding a key that cannot match (#7260).
    ///
    /// A field lookup compares every key of the object, and decoding one means scanning it to
    /// its closing quote and checking it for UTF-8. Up to the first byte where the text and
    /// `name` differ there is no backslash (`name` has none), so the decoded string agrees with
    /// the text there and differs from `name` at that byte, unless the byte is a backslash
    /// that may decode to it. Only that case, and a `name` holding a quote or backslash, decode.
    #[inline]
    pub fn decodes_to(&self, name: &str) -> bool {
        let want = name.as_bytes();
        if want.iter().any(|&b| b == b'"' || b == b'\\') {
            return self.as_str().is_ok_and(|k| k == name);
        }
        let body = self.text.get(self.start + 1..).unwrap_or_default();
        let common = body.iter().zip(want).take_while(|(a, b)| a == b).count();
        match body.get(common) {
            // An escape where the text leaves `name` may still decode to it.
            Some(b'\\') => self.as_str().is_ok_and(|k| k == name),
            // The whole of `name`, then the closing quote (or the end of an unclosed text,
            // which `as_str` reads as the string's end).
            Some(b'"') | None => common == want.len(),
            // The text goes on past `name`, or differs from it at a byte that is itself.
            Some(_) => false,
        }
    }

    fn find_end(&self) -> usize {
        // Include the closing quote, but never run past the text: a string
        // that never closes makes `find_string_end` return `text.len()`, and
        // `+ 1` would then slice one byte past the buffer (#5541). The span
        // ends at end-of-text instead, so the caller's delimiter check sees
        // no closing byte after it and reports the malformed document.
        (self.find_string_end() + 1).min(self.text.len())
    }

    fn find_string_end(&self) -> usize {
        self.find_string_end_and_escape().0
    }

    /// [`find_string_end`](Self::find_string_end), and whether the string holds a backslash.
    fn find_string_end_and_escape(&self) -> (usize, bool) {
        let mut i = self.start + 1; // Skip opening quote
        let mut escaped = false;
        while i < self.text.len() {
            match self.text[i] {
                b'"' => return (i, escaped),
                b'\\' => {
                    escaped = true;
                    i += 2; // Skip escape sequence
                }
                _ => i += 1,
            }
        }
        (self.text.len(), escaped)
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
/// *not* independent: its span has no grammar difference from
/// `nested_number_span`'s own greedy `[0-9.eE+-]*` run (same permissive,
/// malformed-shape-absorbing contract). Since #3167 it reads jq's
/// canonical prefix in one pass and accepts it only where the greedy run
/// would end too, which it asks through `is_number_span_byte` -- the one
/// definition of that class, shared with `nested_number_span` and
/// `strict_number_end` -- rather than calling `nested_number_span` first.
/// Plain backticks, not an intra-doc link: this function is `pub` and
/// `is_jq_canonical_number` is `pub(crate)`, which rustdoc's
/// `private_intra_doc_links` lint rejects under CI's
/// `RUSTDOCFLAGS=-D warnings` -- `cargo doc --no-deps --all-features` has
/// failed on this branch since the link was added.
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
/// `scan_one_json_token` / [`container_end`], in
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

/// Find the end of the object or array that opens at `bytes[start]`.
///
/// Returns the index one past its matching close, or `None` for a `start` that
/// is not a `{` or `[` in `bytes`, or a container that is not closed before the end of `bytes` or holds a string with a raw
/// `U+0000`-`U+001F` byte.
///
/// This is the top-level document splitter's container walk
/// (`find_json_values`, `src/bin/succinctly/jq_runner.rs`). It is structural,
/// not a validator: only this container's own `open`/`close` pair moves the
/// depth (a `]` inside `{...}` is ignored), and string contents are skipped by
/// exactly [`string_literal_end`]'s rule -- the string ends at the first
/// unescaped `"`, a `\` skips the byte after it whatever it is, and a raw
/// control character rejects the whole string (#2878) -- so the two agree on
/// every input, which the tests sweep against the byte-at-a-time walk this
/// replaced.
///
/// **The walk is over a 64-byte mask, not bytes (#4177).** The byte loop paid a
/// three-way compare per byte plus a string-scan entry per string; on a wide or
/// record-shaped document that was ~10 `Ir` per input byte, 15-19% of a `jq -c .`
/// run once #4172 had removed the line counter. Here one block of `"`, `\`,
/// controls and the two brackets is classified once and every hit in it is
/// answered with a `trailing_zeros`; the bytes between hits are never visited,
/// and a short string costs a bit scan of a mask its neighbours already paid
/// for (the `SpecialMask` regime of #3340, extended to the brackets). The state
/// is `in_string`: outside a string `"` opens one and the brackets move `depth`
/// (a stray `\` or whitespace control is ignored, as the byte loop's `_ => {}`
/// did); inside one `"` closes it, `\` moves the cursor past the next byte
/// (carrying into the next block when the `\` is the block's last byte), a
/// control byte returns `None`, and a bracket is content.
///
/// A bracket inside a string is a hit the mask cannot tell apart from one
/// outside, and a string made of the container's own brackets is one loop
/// iteration per byte where `string_literal_end` skipped it in one call (+24%
/// on a 7950X, +60% on an M4 Pro against the byte loop). So the first bracket
/// seen inside a string swaps the block's mask for one without the brackets
/// (`classify_string_64`) until that string ends: ordinary strings never pay for
/// it, a bracket-dense one pays one more classification per block it spans, not
/// one hit per byte. Splitting the mask by state eagerly was measured and
/// rejected: it halved the gain on records, and a lazily narrowed mask kept as
/// a per-hit `cursor` shift (instead of clearing the lowest set bit) made short
/// plain strings 4-6% slower on the M4 Pro.
///
/// A partial final block is copied into a space-padded buffer with its high
/// bits masked off, so nothing past `bytes.len()` is read or reported.
/// `#[inline(never)]` keeps the loop out of its caller's register allocation
/// (#3350); the call is paid once per top-level container, never per byte.
#[inline(never)]
pub fn container_end(bytes: &[u8], start: usize) -> Option<usize> {
    // `start` must hold an opener; anything else (or a `start` past the end)
    // is not a container, so there is no end to report.
    let open = *bytes.get(start)?;
    let close = match open {
        b'{' => b'}',
        b'[' => b']',
        _ => return None,
    };
    let mut depth = 1usize;
    let mut in_string = false;
    // The previous block ended on the `\` of an escape: this block's first
    // byte is the escaped one.
    let mut skip_first = false;
    let mut base = start + 1;
    while base < bytes.len() {
        let tail;
        let (block, len): (&[u8; 64], usize) = match bytes
            .get(base..base + 64)
            .and_then(|s| <&[u8; 64]>::try_from(s).ok())
        {
            Some(block) => (block, 64),
            None => {
                // Fewer than 64 bytes left (and at least one, by the loop guard).
                let rest = &bytes[base..];
                let mut buf = [b' '; 64];
                buf[..rest.len()].copy_from_slice(rest);
                tail = buf;
                (&tail, rest.len())
            }
        };
        let live = if len < 64 {
            (1u64 << len) - 1
        } else {
            u64::MAX
        };
        let all = crate::util::simd::specials::classify_container_64(block, open, close) & live;
        let mut mask = all;
        // `mask` holds `all` minus the brackets for the rest of a string that
        // held one (see below); `all` is restored when that string ends.
        let mut narrowed = false;
        if skip_first {
            mask &= !1;
            skip_first = false;
        }
        while mask != 0 {
            let k = mask.trailing_zeros() as usize;
            mask &= mask - 1;
            let b = block[k];
            if in_string {
                match b {
                    b'"' => {
                        in_string = false;
                        if narrowed {
                            narrowed = false;
                            mask = all & high_bits_from(k + 1);
                        }
                    }
                    b'\\' => {
                        if k == 63 {
                            skip_first = true;
                        } else {
                            mask &= !(1u64 << (k + 1));
                        }
                    }
                    // This container's bracket inside a string: content, not a
                    // decision. Drop the brackets from the rest of the string
                    // so a bracket-dense one costs a hit per byte no more.
                    _ if b >= 0x20 => {
                        narrowed = true;
                        mask = crate::util::simd::specials::classify_string_64(block)
                            & live
                            & high_bits_from(k + 1);
                    }
                    // A raw control character: the whole string is rejected.
                    _ => return None,
                }
            } else if b == b'"' {
                in_string = true;
            } else if b == open {
                depth += 1;
            } else if b == close {
                depth -= 1;
                if depth == 0 {
                    return Some(base + k + 1);
                }
            }
            // else: a stray `\` or a whitespace control byte outside a string.
        }
        base += len;
    }
    None
}

/// A mask of bits `from..64` (empty for `from >= 64`).
#[inline(always)]
fn high_bits_from(from: usize) -> u64 {
    u64::MAX.checked_shl(from as u32).unwrap_or(0)
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
/// `text_range()`, [`nested_number_span`] below).
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
/// `-inf` and `+Infinity` are one span each, while `-nope` is the whole run
/// to the next token boundary, like any number with a byte glued on (#5654). `is_valid_number`/
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
/// recognize them. The same greedy span captures those bytes in a user
/// document too; there they are decoded as an ordinary malformed number,
/// because only an index built over bridge text lets a token decode
/// ([`JsonNumber::bridge_value`], #3034).
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
    while i < text.len() && is_number_span_byte(text[i]) {
        i += 1;
    }
    // Nothing numeric after the (optional) sign: a decNumber special word
    // (`nan`, `sNaN12`, `-Infinity`, `+inf`, #2877)? Only then, and only if
    // the whole word validates -- otherwise it falls to the glued-run arm
    // below, which spans `-nope` / `+x` to the next token boundary (#5654).
    if i == body_start {
        if let Some(end) = special_number_end(text, start) {
            return end;
        }
    }
    // A byte glued to the number that is not a jq token boundary (`0x1`,
    // `12x3`, `0.0x1`) makes the whole run one token to jq's scanner, which
    // then fails to validate it (#5654). The span runs to the end of that
    // token so every decoder sees the malformed whole, not a valid prefix.
    if i < text.len() && !jq_literal_boundary(text[i]) {
        return jq_literal_run_end(text, start);
    }
    i
}

/// Whether `byte` continues [`nested_number_span`]'s greedy number run --
/// `[0-9.eE+-]`, the one definition of that class (#3167). The span's own
/// loop and `JsonCursor::number_glued_junk` ask this. [`strict_number_end`]
/// and [`scan_canonical_number`] ask [`jq_literal_boundary`] instead since
/// #5654: a number ends at a token boundary, not at the first non-number byte.
#[inline(always)]
fn is_number_span_byte(byte: u8) -> bool {
    matches!(byte, b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
}

/// The reason a nested number span that is not a number carries as a
/// [`StandardJson::Error`] (#3222). A route that re-validates the document
/// for its message (`EvalError::malformed_json_text`) names the fault more
/// precisely; this is what the rest print.
const MALFORMED_NUMBER: &str = "invalid numeric literal";

/// Where the token at `start` ends, if it is
/// `-?[0-9]+(\.[0-9]+)?([eE][+-]?[0-9]+)?` and ends exactly where
/// [`nested_number_span`] would end it -- i.e. the next byte is a jq token
/// boundary (or the end), so nothing is glued on (#5654). Such a span is a number to every decoder (RFC 8259
/// plus jq's redundant leading zeros, #1149), so `JsonCursor::number_at`
/// accepts it without taking the greedy span; every other shape, valid or
/// not, answers `None` here and is decided on the whole span.
#[inline]
fn strict_number_end(text: &[u8], start: usize) -> Option<usize> {
    #[inline]
    fn digits(text: &[u8], mut i: usize) -> Option<usize> {
        let from = i;
        while i < text.len() && text[i].is_ascii_digit() {
            i += 1;
        }
        (i > from).then_some(i)
    }
    let mut i = start;
    if text.get(i) == Some(&b'-') {
        i += 1;
    }
    i = digits(text, i)?;
    if text.get(i) == Some(&b'.') {
        i = digits(text, i + 1)?;
    }
    if matches!(text.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(text.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        i = digits(text, i)?;
    }
    // Followed by a jq token boundary (or the end): anything else glued on
    // (`0x1`, `12x3`) makes the whole run one malformed token (#5654).
    match text.get(i) {
        None => Some(i),
        Some(&b) => jq_literal_boundary(b).then_some(i),
    }
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

/// Whether the token starting at `start` is exactly the keyword literal `kw`
/// (`null`/`true`/`false`), returning the run's end on success.
///
/// Same "the *whole* run validates" rule `special_number_end` applies, using
/// the same jq token boundary (`jq_literal_boundary`, the shared definition);
/// reading a valid prefix
/// out of a longer token would materialize a literal where real jq reports a
/// parse error. So `nullx`/`truex`/`falsey` -- a keyword-plus-suffix token --
/// and `nul`/`tru`/`fals` -- a truncated one -- are both not the literal,
/// matching jq 1.7.1's `guess_tag` (`Invalid literal` for either shape,
/// captured live over `[true<suffix>]`). The document is rejected by value
/// decoding instead of any bite-sized prefix being answered (#3035).
///
/// `pub` because the CLI's top-level document splitter
/// (`scan_one_json_token` over `find_literal_end`) must agree on the *same*
/// whole-token rule for bare keywords: with the old alphabetic-only run it
/// split `null1` into `null` + `1` -- a keyword whose run genuinely includes
/// the following digit, passed only because the split happened to break
/// between them (#3035).
///
/// `#[inline]`: this replaced an inlined `starts_with` in `value_at`'s
/// keyword arms, whose cost is charged to every decoded value's fast path.
/// Left as an out-of-line call it inflated `value_at` enough to perturb its
/// own inlining into its many callers, a crate-wide codegen ripple the
/// instruction-count guard caught (a uniform +1-2% Ir on rows that never
/// reach this code, ARM64-only). Inlining keeps the valid-literal arm the
/// same handful of compares it was before.
#[inline]
#[must_use]
pub fn keyword_span(text: &[u8], start: usize, kw: &[u8]) -> Option<usize> {
    // Check the fixed-width window first, then only the one boundary byte
    // that decides whether a longer run (`nullx`) or a truncated one
    // (`nul` at EOF) is a keyword. The old form scanned the whole
    // `jq_literal_run_end` run even when the first `kw.len()` bytes already
    // failed, which is wasted work on `nul`/`nullx` and on valid literals
    // followed by a whitespace run; this reads at most `kw.len() + 1` bytes.
    let end = start.checked_add(kw.len())?;
    if text.get(start..end) != Some(kw) {
        return None;
    }
    match text.get(end) {
        None => Some(end),
        Some(&b) if jq_literal_boundary(b) => Some(end),
        Some(_) => None,
    }
}

/// Whether `byte` ends a jq literal token: whitespace or one of
/// `"` `[` `{` `,` `:` `]` `}` (`jv_parse.c`'s `scan`). One definition,
/// shared by [`jq_literal_run_end`] and [`keyword_span`] so the splitter's
/// token boundary and the decoder's whole-token rule cannot drift apart
/// (#106, #3035).
#[inline]
fn jq_literal_boundary(byte: u8) -> bool {
    byte.is_ascii_whitespace() || matches!(byte, b'"' | b'[' | b'{' | b',' | b':' | b']' | b'}')
}

/// Where the literal token starting at `start` ends under jq 1.7.1's own
/// scanner: `jv_parse.c`'s `scan` accumulates every byte that is not
/// whitespace, `"` or one of `[{,:]}` into one token and hands the whole
/// thing to `check_literal`. So `nan 1` is two tokens, while `nan1.5`,
/// `nan(1)` and `nanx` are each one token that then fails to validate.
fn jq_literal_run_end(text: &[u8], start: usize) -> usize {
    let mut end = start;
    while end < text.len() && !jq_literal_boundary(text[end]) {
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
    /// The span's length, measured once by `JsonCursor::number_at` while it
    /// decides whether the span is a number (#3222), so
    /// [`raw_bytes`](Self::raw_bytes) never rescans it. `u32` so the struct
    /// stays 32 bytes; [`UNMEASURED_SPAN`] (a span that doesn't fit) sends
    /// `raw_bytes` back to the scan.
    len: u32,
    /// Copied from the index's `bridge_tokens` (#3034): whether this span
    /// may be one of the reindex bridge's number tokens.
    bridge: bool,
}

/// [`JsonNumber::len`]'s "not stored" value: a span of `u32::MAX` bytes or
/// more, which `raw_bytes` measures again instead.
const UNMEASURED_SPAN: u32 = u32::MAX;

impl<'a> JsonNumber<'a> {
    /// The byte offset of the number's first character in the document
    /// text. Mirrors [`JsonString::start`] for the same reuse purpose
    /// (#1643, #1677).
    #[inline]
    pub fn start(&self) -> usize {
        self.start
    }

    /// Get the raw bytes of the number.
    #[inline]
    pub fn raw_bytes(&self) -> &'a [u8] {
        // patchcov: coverage tolerate reason="unreachable in practice: UNMEASURED_SPAN only arises when `end - text_pos` in `number_at` overflows u32 -- a single number span >= 4 GiB -- which no realistic (or practically constructible) test document approaches (#3222)"
        let end = if self.len == UNMEASURED_SPAN {
            nested_number_span(self.text, self.start)
        } else {
            self.start + self.len as usize
        };
        // patchcov: coverage end
        &self.text[self.start..end]
    }

    /// Parse as i64.
    pub fn as_i64(&self) -> Result<i64, JsonError> {
        let bytes = self.raw_bytes();
        let s = core::str::from_utf8(bytes).map_err(|_| JsonError::InvalidUtf8)?;
        s.parse()
            .or_else(|_| self.bridge_spelled_int().ok_or(JsonError::InvalidNumber))
    }

    /// The integer a bridge spelling token names (`0x1f` is 31), for an index built over bridge
    /// text; `None` for any other span and for a spelling that is no integer (#2802).
    #[inline]
    pub(crate) fn bridge_spelled_int(&self) -> Option<i64> {
        // #6958: a float whose text reads as an integer (`!!float 1_000`) is no integer.
        if self.raw_bytes().first() == Some(&b'4') && self.bridge_float_spelling().is_some() {
            return None;
        }
        // The integer token first: an explicit `!!int 08` is an integer whatever its text.
        if let Some(spelling) = self.bridge_int_spelling() {
            return crate::yaml::spelled_int_value(&spelling);
        }
        let spelling = self.bridge_spelling()?;
        // #6938: a zero-padded `08` that crossed as a plain spelling is a *float* of a YAML
        // document (#6866), so no integer. Read as one, `.zz = 1 | .p | tag` said `!!int`.
        if crate::yaml::is_zero_padded_with_8_or_9(&spelling) {
            return None;
        }
        crate::yaml::spelled_int_value(&spelling)
    }

    /// The value this span stands for if it is one of the reindex bridge's
    /// number tokens -- NaN (`NAN_SENTINEL`), ±Infinity
    /// (`INFINITY_SENTINEL`/`NEG_INFINITY_SENTINEL`) or a computed finite
    /// float (`crate::json::validate::computed_float_token`) -- **and** it
    /// was read through an index built over bridge text
    /// ([`JsonIndex::build_reindex`]); `None` otherwise.
    ///
    /// The one decoder of those tokens for a document number (#3034). The
    /// tokens are unparseable by `str::parse`, but not unspellable: a user
    /// document can hold the same bytes, and the scanners capture them as one
    /// span there too. Such a span is an ordinary malformed number, read the
    /// way its neighbours (`9e999e998`, `1e0e1`) are, so every reader that
    /// decodes a token has to come through here, where provenance decides.
    #[inline]
    pub fn bridge_value(&self) -> Option<f64> {
        if !self.bridge {
            return None;
        }
        let bytes = self.raw_bytes();
        crate::jq::OwnedValue::bridge_nonfinite_from_bytes(bytes)
            .or_else(|| crate::json::validate::parse_computed_float_token(bytes))
            // #6275: a computed integer is the double it names to every reader that has no
            // integer type of its own; `bridge_computed_int` keeps it an `Int` where one has.
            .or_else(|| crate::json::validate::parse_computed_int_token(bytes).map(|n| n as f64))
    }

    /// The spelling of a document `.inf`/`-.INF`/`.NaN` scalar this span stands for, if it is
    /// the reindex bridge's token for one
    /// ([`nonfinite_literal_token`](crate::json::validate::nonfinite_literal_token)) **and** it
    /// was read through an index built over bridge text; `None` otherwise (#1071, #3034).
    #[inline]
    pub(crate) fn bridge_nonfinite_literal(&self) -> Option<&'static str> {
        if !self.bridge {
            return None;
        }
        crate::json::validate::parse_nonfinite_literal_token(self.raw_bytes())
    }

    /// The source spelling of a YAML scalar this span stands for, if it is the reindex bridge's
    /// token for one
    /// ([`spelling_token`](crate::json::validate::spelling_token)) **and** it was read through
    /// an index built over bridge text; `None` otherwise (#2802, #3034).
    ///
    /// An allocation, unlike [`bridge_nonfinite_literal`](Self::bridge_nonfinite_literal),
    /// whose spellings are a fixed table: a spelling is whatever the scalar was written as.
    /// It is the cold path, reached only for a bridge number that is not a plain one.
    #[inline]
    pub(crate) fn bridge_spelling(&self) -> Option<String> {
        if !self.bridge {
            return None;
        }
        let bytes = self.raw_bytes();
        crate::json::validate::parse_spelling_token(bytes)
            .or_else(|| crate::json::validate::parse_int_spelling_token(bytes))
            .or_else(|| crate::json::validate::parse_float_spelling_token(bytes))
    }

    /// The text of a float the bridge carried under its own token because the text alone reads
    /// as an integer (an underscored `!!float 1_000`, #6958); `None` for any other span. Asked
    /// before [`bridge_spelling`](Self::bridge_spelling), which answers the same text for it.
    #[inline]
    pub(crate) fn bridge_float_spelling(&self) -> Option<String> {
        if !self.bridge {
            return None;
        }
        crate::json::validate::parse_float_spelling_token(self.raw_bytes())
    }

    /// The text of an integer the bridge carried under its own token because the text alone
    /// reads as a float (a zero-padded `08`, #6927); `None` for any other span. Asked before
    /// [`bridge_spelling`](Self::bridge_spelling), which answers the same text for it.
    #[inline]
    pub(crate) fn bridge_int_spelling(&self) -> Option<String> {
        if !self.bridge {
            return None;
        }
        crate::json::validate::parse_int_spelling_token(self.raw_bytes())
    }

    /// The address of the token's first byte in the document text: the
    /// identity jq gives a NaN it parsed (#3309), since every read of one
    /// position answers the same address and no other live token does.
    /// `None` for a reindex bridge token, which stands for a computed value.
    ///
    /// The one definition both keying routes use
    /// (`OwnedValue::from_json_number` and `DocumentValue::number_token_address`),
    /// so a document NaN gets the same instance whichever materializes it.
    #[must_use]
    pub fn token_address(&self) -> Option<usize> {
        if self.bridge_value().is_some() || self.bridge_spelling().is_some() {
            return None;
        }
        Some(self.raw_bytes().as_ptr() as usize)
    }

    /// The text of a bridge number that is a `+`-signed integer past `i64`
    /// (`+99999999999999999999`); `None` for anything else.
    ///
    /// Reading a document, jq stores `+X` as the unsigned `X` (#2877), so
    /// every reader peels the `+`. A reindex bridge never holds a spelling
    /// jq made: it holds a `NumberLiteral`'s stored text, and a YAML document's
    /// `a: +99999999999999999999` is stored with its `+` because yq echoes it
    /// (`tostring`, interpolation, `join`; #4921). The one place both
    /// `DocumentValue::number_literal` and `OwnedValue::from_json_number` keep
    /// it, so the two cannot drift.
    ///
    /// Not gated on the mode: a jq-mode `load()` of a YAML file stores the same
    /// literal, and the two readers must agree on a bridge number (#3069). jq
    /// mode never prints the `+` -- `format_number_jq_compat` peels it -- which
    /// `tests/jq_cli/i4921.rs` pins.
    #[inline]
    pub(crate) fn bridge_signed_overflowed_int(&self) -> Option<&'a str> {
        if !self.bridge {
            return None;
        }
        let bytes = self.raw_bytes();
        if bytes.first() != Some(&b'+') {
            return None;
        }
        core::str::from_utf8(bytes)
            .ok()
            .filter(|text| crate::yaml::is_overflowed_int_literal(text))
    }

    /// [`bridge_value`](Self::bridge_value) restricted to the NaN/infinity
    /// tokens (#472/#1083): `None` for the computed-float token and for
    /// everything read outside the bridge. For the cursor builtins that
    /// special-case a non-finite value before their ordinary parse
    /// (`length`, `isnan`, `isinfinite`, the date builtins).
    #[inline]
    pub fn bridge_nonfinite(&self) -> Option<f64> {
        if !self.bridge {
            return None;
        }
        let bytes = self.raw_bytes();
        // #1071: a document `.inf`/`.NaN` the bridge carried is a literal with text of its own
        // (`.INF | length` is 4), not a computed non-finite number.
        if crate::json::validate::parse_nonfinite_literal_token(bytes).is_some() {
            return None;
        }
        crate::jq::OwnedValue::bridge_nonfinite_from_bytes(bytes)
    }

    /// Whether this number was read from the reindex bridge's text (an owned value written out
    /// to be evaluated again) and not from a document: its text is then the spelling the owned
    /// value carries, or a rendering of a computed one (#3604).
    #[inline]
    pub fn is_bridge(&self) -> bool {
        self.bridge
    }

    /// [`bridge_value`](Self::bridge_value) restricted to the computed-float
    /// token (#2902): `None` for the NaN/infinity tokens and for everything
    /// read outside the bridge.
    #[inline]
    pub fn bridge_computed_float(&self) -> Option<f64> {
        if !self.bridge {
            return None;
        }
        crate::json::validate::parse_computed_float_token(self.raw_bytes())
    }

    /// [`bridge_value`](Self::bridge_value) restricted to the computed-integer
    /// token (#6275): the `i64` it names, `None` for everything else and for
    /// everything read outside the bridge.
    #[inline]
    pub fn bridge_computed_int(&self) -> Option<i64> {
        if !self.bridge {
            return None;
        }
        crate::json::validate::parse_computed_int_token(self.raw_bytes())
    }

    /// Parse as f64.
    ///
    /// Also decodes every spelling the ordinary parse refuses but this
    /// crate's number decoders read as a number: the reindex bridge's tokens
    /// ([`bridge_value`](Self::bridge_value) -- only on a span read through
    /// the bridge's own index, #3034), and decNumber's special values
    /// (#2877: Rust's `f64: FromStr` reads `nan`, `inf`, `infinity` and a
    /// leading `+` case-insensitively, but rejects a signalling NaN `sNaN`
    /// and a NaN payload `nan12`, both of which jq 1.7.1 accepts). Every
    /// cursor-level reader of a document number in both evaluators
    /// (`length`, the math builtins, dates, `isnan`, the generic
    /// materializer's `as_f64` arm, ...) reaches its value through this one
    /// accessor, so the decode lives here rather than being repeated at each
    /// of them. Only consulted once the ordinary parse has already failed,
    /// so a genuine number pays nothing for it.
    ///
    /// The value is the **plain** correctly-rounded parse, the JSON
    /// library's own answer. jq mode's 17-digit literal model (#2936) is
    /// applied by the evaluators' number funnels
    /// (`OwnedValue::from_json_number::<S>`, `document_number_f64::<S>`),
    /// not here: this type has no mode, and every spelling below the
    /// fallback is a non-literal (a bridge token or a decNumber word) whose
    /// value the model cannot change.
    pub fn as_f64(&self) -> Result<f64, JsonError> {
        let bytes = self.raw_bytes();
        let s = core::str::from_utf8(bytes).map_err(|_| JsonError::InvalidUtf8)?;
        s.parse().or_else(|_| {
            self.bridge_value()
                // #2802: the number a spelled integer names; #1356: or a spelled float.
                .or_else(|| self.bridge_spelled_int().map(|n| n as f64))
                .or_else(|| {
                    let spelling = self.bridge_spelling()?;
                    crate::yaml::underscored_float_value(&spelling).or_else(|| {
                        crate::yaml::is_spelled_float(&spelling)
                            .then(|| spelling.parse::<f64>().ok())
                            .flatten()
                    })
                })
                .or_else(|| crate::json::validate::jq_special_number(bytes))
                .ok_or(JsonError::InvalidNumber)
        })
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
    document_token_of, effective_fields_checked, key_hash, key_span_fingerprint,
    repeat_plan_checked, repeat_plan_from_shared, shared_key_hashes_if_any,
    trailing_element_gap_ok, DocumentCursor, DocumentElements, DocumentField, DocumentFields,
    DocumentValue, FieldFate, IndentSpec, JsonConvention, KeyHashes, PositionOf, RepeatPlan,
    RepeatPlanBuilder, StepSpan, PAIRWISE_SPAN_SCAN_LIMIT,
};
use crate::jq::escape::{write_json_body_jq, write_json_body_yq};
use crate::jq::stream::{StreamFailure, StreamResult};
use crate::jq::{
    format_number_jq_compat, is_jq_canonical_number, jq_canonical_number_prefix,
    nesting_depth_exceeded_message, nonfinite_display_string, EvalError, EvalSemantics,
    JqSemantics, NumberRepr, OwnedValue, MAX_VALUE_TREE_DEPTH,
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
/// Catches a missing or doubled delimiter between two real children, and (#5644) a junk byte
/// where the scan stops with nothing before it to excuse it: the first child of a container
/// must follow its `[` or `{`, and a value (`expected` is `:`) must follow its key's closing
/// `"`. Only space, tab, LF and CR are whitespace, as in JSON. A sibling must end on a byte a value
/// can end on (#5650, `sibling_end_ok`).
///
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
            b' ' | b'\t' | b'\n' | b'\r' => i -= 1,
            _ => break, // reached the previous sibling's own content
        }
    }
    // #5644: what the scan stopped on has to be what can sit there. A first child follows its
    // container's `[` or `{`, and a value follows its key's closing `"`; any other byte (`@`,
    // `#`, a control byte, ...) is junk the index read as whitespace, and jq refuses it
    // (`[@"a"]`, `{"a"@:1}`, `{@"a":1}`). After a `,` the stop is the previous sibling's own last
    // byte, which can be anything a value ends in, so only these two are checked.
    let stopped_on_ok = match expected {
        None => i == 0 || matches!(text[i - 1], b'[' | b'{'),
        Some(b':') => i > 0 && text[i - 1] == b'"',
        Some(b',') => i > 0 && sibling_end_ok(&text[..i]),
        Some(_) => true,
    };
    found == expected && stopped_on_ok
}

/// Whether `before`, which ends where a `,` gap begins, ends a value (#5650): the byte a sibling
/// ends on must be able to end one, so `@`, a form feed or any other junk between a sibling and its
/// `,` is refused, as jq refuses it (`[1@,2]`, a form feed after `"a"`). A number that has a bare
/// token glued to it (`1null`, `1e`, `0x1`) is one malformed token to jq's lexer, so a bare token
/// that starts like a number must be one. A token that starts with a letter (`true`, `nan`,
/// `-Infinity`) is left to the value's own decode.
fn sibling_end_ok(before: &[u8]) -> bool {
    let last = before[before.len() - 1];
    // The common siblings end here, in one byte test: a number's last digit, a string, a container.
    // A bare token glued to a number that still ends in a digit (`0x1`, `1_000`) is not looked at:
    // walking every number back would cost the pretty printer 18% on a number-heavy array. The
    // number scanners refuse it where it is read instead (#5654).
    if last.is_ascii_digit() || matches!(last, b'"' | b']' | b'}') {
        return true;
    }
    if !(last.is_ascii_alphabetic() || last == b'.') {
        return false;
    }
    let start = before
        .iter()
        .rposition(|&b| jq_literal_boundary(b))
        .map_or(0, |p| p + 1);
    let token = &before[start..];
    let unsigned = token
        .strip_prefix(b"-")
        .or_else(|| token.strip_prefix(b"+"))
        .unwrap_or(token);
    match unsigned.first() {
        Some(c) if c.is_ascii_digit() || *c == b'.' => {
            number_literal_end(token, 0) == Some(token.len())
        }
        _ => true,
    }
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
/// The byte the scan stops on starts the value, so it must be able to start one (#5644): a
/// letter, digit, quote, bracket, brace or one of `-+.`, or a closing bracket for the
/// empty-value check. Anything else (`@`, `#`, a control byte) is refused.
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
            b' ' | b'\t' | b'\n' | b'\r' => i += 1,
            _ => break,
        }
    }
    // #5644: the value after the `:` starts on the byte the scan stopped on. The index reads a
    // byte that begins no JSON token (`@`, `#`, a control byte, ...) as whitespace and starts the
    // value after it, so `{"a":@1}` would read as `{"a":1}` where jq refuses it. A letter or
    // digit is left to the value's own decode, and a closing bracket to the empty-value check.
    let value_start_ok = i >= text.len()
        || text[i].is_ascii_alphanumeric()
        || matches!(
            text[i],
            b'"' | b'[' | b'{' | b'-' | b'+' | b'.' | b'}' | b']'
        );
    found == Some(b':') && value_start_ok
}

/// The position of the last 1 bit (open parenthesis) strictly before `end` in `words`, whose bit
/// `p` is `words[p / 64] >> (p % 64)`.
fn last_open_before(words: &[u64], end: usize) -> Option<usize> {
    let last = end.checked_sub(1)?;
    let mut word = last / 64;
    let keep = last % 64 + 1;
    let mut bits = words.get(word)?
        & if keep == 64 {
            u64::MAX
        } else {
            (1u64 << keep) - 1
        };
    loop {
        if bits != 0 {
            return Some(word * 64 + 63 - bits.leading_zeros() as usize);
        }
        word = word.checked_sub(1)?;
        bits = words[word];
    }
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
            b' ' | b'\t' | b'\n' | b'\r' => i += 1, // JSON whitespace: no form feed (#5650)
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
/// whose end comes from [`JsonCursor::container_end_bp`] instead (#5654;
/// see [`JsonCursor::last_child_text_end`], which picks between the two).
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
/// This is *half* of the echo decision, not all of it. The other half is
/// the `core::str::from_utf8` the caller runs over the returned span: this
/// scan certifies "canonical **given** valid UTF-8" and leaves the UTF-8
/// question to std's vectorised validator rather than decoding sequences
/// itself (see [`scan_json_string_span`] for the measurement that made that
/// the cheaper split). A test that drives only this function therefore
/// tests only half the gate -- the differential and fuzz tests in this
/// module drive `stream_json` itself for that reason.
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
///   char) must satisfy [`is_jq_canonical_number`], which already answers
///   "would `format_number_jq_compat` echo these exact bytes" (#2206).
///   Since #3167 that is asked in one pass: the rule's own prefix parser,
///   [`jq_canonical_number_prefix`], plus a check that the run ends where
///   the prefix does (see `scan_canonical_number`).
/// - Strings: scanned between the quotes (one specials mask, #3340) against exactly
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
///   equality. That certification is now conditional on the span being
///   valid UTF-8 (see [`scan_json_string_span`]), which only widens where
///   this scan *accepts*: an invalid-UTF-8 key has no decoded form to be
///   confused about, and the caller's own `from_utf8` rejects the whole
///   span before anything is echoed. Two tiers, on the same threshold
///   `src/bin/succinctly/jq_runner.rs`'s own `spans_repeat` splits on
///   (#2919 review; one shared [`PAIRWISE_SPAN_SCAN_LIMIT`] and one shared
///   [`key_span_fingerprint`] since #2608's review, both in
///   `src/jq/document.rs` beside the [`KeyHashes`] the two sites already
///   shared): up to `PAIRWISE_SPAN_SCAN_LIMIT` keys are compared pairwise
///   via a cheap fingerprint with no allocation at all -- most real
///   objects never cross that threshold, and [`KeyHashes::insert`]
///   heap-allocates its table on its very first call, which would
///   otherwise cost every tiny object a real allocation for no reason.
///   Past the threshold `scan_canonical_object_wide` takes over (#3333):
///   each key's [`key_hash`] is only collected as the object is scanned,
///   and [`KeyHashes::has_repeat`] settles them all at the closing `}`
///   through a table sized for exactly that many keys, so nothing is
///   rehashed on the way up. From [`KeyHashes::SATURATING_KEYS`] keys, the
///   width at which a table grown key by key would have turned
///   `saturated()`, the list is settled by `shared_key_hashes_if_any`'s sort instead,
///   as `DistinctKeyCursors` settles an object that wide (`src/jq/document.rs`);
///   until #7293 the scan declined such an object outright, which left its
///   caller to walk and hash every key after the scan had hashed 786,432 of
///   them. A repeat in a wide
///   object is found at its end rather than at the key; the span is
///   declined either way. Either tier's "seen before" answer covers both a
///   genuine duplicate key and a bare 64-bit hash collision between two
///   distinct keys; both get the same conservative answer here (bail, don't
///   try to disambiguate by comparing bytes) since either one means "cannot
///   certify this object's span as canonical", not "definitely not
///   canonical" -- `KeyHashes`'s own doc comment describes the same
///   conservatism for its other callers. Both tiers, the seeding handoff
///   between them, a duplicate placed on either side of the boundary and
///   across it, and the sort past the table's ceiling all have their own
///   sweeps in this module's `mod tests` (`canonical_object_key_tiers_2608`
///   and `canonical_echo_settles_a_saturating_object_by_sort_7293`, #2608
///   review, #7293) -- the corpus below them tops out at six keys, which is why
///   the fuzz generator draws an occasional wide object too.
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
/// edge-byte content per the fuzz-alphabet rule in the testing skill
/// (`.claude/skills/testing/SKILL.md`).
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
#[cfg(test)]
#[must_use]
pub(crate) fn canonical_compact_jq_span_end(bytes: &[u8]) -> Option<usize> {
    canonical_compact_jq_span_end_at_depth(bytes, 0)
}

/// `canonical_compact_jq_span_end` for a value that sits `depth` levels down
/// already (#3909): the scan's nesting ceiling is [`MAX_VALUE_TREE_DEPTH`]
/// counted from the document root, so a caller printing a nested value, or one
/// that owes a tighter ceiling than the printer's, says where it starts. A
/// larger `depth` only narrows what is accepted.
#[must_use]
pub(crate) fn canonical_compact_jq_span_end_at_depth(bytes: &[u8], depth: usize) -> Option<usize> {
    scan_canonical_value(bytes, 0, depth, &mut ScanState::new())
}

/// What [`JsonCursor::canonical_compact_scan`] made of a node (#7310).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CanonicalSpan<'a> {
    /// The node's text, which is already its compact jq render
    /// ([`JsonCursor::canonical_compact_span`]'s `Some`).
    Echo(&'a str),
    /// The node is an object whose every member is canonical and well
    /// formed, whose span is valid UTF-8 (so every key decodes), and these of
    /// whose key hashes ([`raw_key_hash`]) repeat
    /// ([`CanonicalScan::RepeatedKeys`]). Whether a key really does is left
    /// to the keys themselves: the scan compared 64-bit hashes of them.
    RepeatedKeys {
        /// Where the object starts in the document's text.
        start: usize,
        /// The object's text, which the scan certified member by member, so
        /// that each run of members the duplicate-key rule leaves alone is
        /// already its own compact render (#7346).
        span: &'a str,
        /// The hashes more than one key has ([`CanonicalScan::RepeatedKeys`]).
        shared: Vec<u64>,
    },
    /// The node's text, valid UTF-8, which is its compact jq render but for
    /// one object inside it that is what [`RepeatedKeys`](Self::RepeatedKeys)
    /// says of a node ([`CanonicalScan::NestedRepeatedKeys`], #7366).
    NestedRepeatedKeys {
        /// The node's text.
        span: &'a str,
        /// The object inside it, placed within `span`.
        object: Box<NestedRepeat>,
    },
    /// Anything else: the scan declined, the node has no text position, or
    /// the span it accepted is not valid UTF-8.
    Declined,
}

/// The one object inside a scanned value that the canonical scan declined only
/// on its keys' hashes, the value being canonical around it
/// ([`CanonicalScan::NestedRepeatedKeys`], #7366).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NestedRepeat {
    /// Where the object's `{` is, counted from the value's first byte.
    pub(crate) start: usize,
    /// One past the object's closing `}`, counted the same way.
    pub(crate) end: usize,
    /// How many levels below the value the object sits (1 for a member or an
    /// element of the value itself).
    pub(crate) level: usize,
    /// The hashes more than one of its keys has, as
    /// [`CanonicalScan::RepeatedKeys`]'s `shared`.
    pub(crate) shared: Vec<u64>,
}

/// What the canonical scan made of one value (#7310): the end of a span it
/// certifies, or that it declined, keeping apart the one decline a caller can
/// still use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CanonicalScan {
    /// The value's span runs to here and is canonical
    /// ([`canonical_compact_jq_span_end_at_depth`] answers `Some` with it).
    Canonical(usize),
    /// The value is an object whose span runs to `end`, every member of it
    /// is canonical, and some of its own keys' hashes repeat: a repeated key,
    /// or two distinct keys sharing a 64-bit hash. The wide tier settles this
    /// ([`scan_canonical_object_wide`]), the pairwise tier handing a repeat
    /// among the first keys over to it (`pairwise_repeat`, #7355).
    RepeatedKeys {
        /// One past the object's closing `}`.
        end: usize,
        /// The hashes more than one key has, sorted and deduplicated. Each is
        /// a [`key_hash`] of a key's raw span, the bytes between its quotes
        /// with any escape left as written ([`raw_key_hash`]).
        shared: Vec<u64>,
    },
    /// The value's span runs to `end` and is canonical but for one object
    /// inside it, which is what [`RepeatedKeys`](Self::RepeatedKeys) says of a
    /// value (#7366). A second such object, or such an object in a value that
    /// is itself one, is [`Declined`](Self::Declined).
    NestedRepeatedKeys {
        /// One past the value's last byte.
        end: usize,
        /// The object inside it.
        object: Box<NestedRepeat>,
    },
    /// Anything else the scan declines.
    Declined,
}

/// [`canonical_compact_jq_span_end_at_depth`], also saying when the value is
/// an object the scan declined only on its own keys' hashes (#7310).
///
/// The wide tier settles an object's key hashes at its closing `}`, after
/// every member has been certified, so a repeat found there leaves nothing
/// unchecked but the repeat itself. A caller that would otherwise walk the
/// object to hash every key and learn which hashes repeat, and whether it is
/// well formed, has all of that already.
///
/// An object further in that declines the same way is recorded too, and the
/// scan goes on past it (#7366): the rest of the value can still be certified,
/// so that a caller prints the text around that object and only the object by
/// its keys. One such object is recorded per scan; a second one, or the value
/// itself declining after one, declines the value as any other fault does.
#[must_use]
pub(crate) fn canonical_compact_jq_scan_at_depth(bytes: &[u8], depth: usize) -> CanonicalScan {
    let mut scan = ScanState::new();
    scan.value_member_depth = depth + 1;
    let certified = scan_canonical_value(bytes, 0, depth, &mut scan);
    // At most one object recorded a repeat (`record_repeated_keys`), and it is
    // the value itself exactly when its members sit one level below the
    // starting depth: a nested object's sit deeper.
    match (certified, scan.repeated_keys) {
        (Some(end), None) => CanonicalScan::Canonical(end),
        (
            Some(end),
            Some(RepeatedKeys {
                member_depth,
                start,
                end: object_end,
                shared,
            }),
        ) => {
            debug_assert!(member_depth > depth + 1 && start < object_end && object_end <= end);
            // A record no frame placed (a `{` left unknown) or that does not
            // sit inside the value is no use to a printer, which cuts the
            // value's text at it: decline it, as the walk is always sound.
            if start >= object_end || object_end > end || member_depth <= depth + 1 {
                return CanonicalScan::Declined;
            }
            CanonicalScan::NestedRepeatedKeys {
                end,
                object: Box::new(NestedRepeat {
                    start,
                    end: object_end,
                    level: member_depth - (depth + 1),
                    shared,
                }),
            }
        }
        (
            None,
            Some(RepeatedKeys {
                member_depth,
                end,
                shared,
                ..
            }),
        ) if member_depth == depth + 1 => CanonicalScan::RepeatedKeys { end, shared },
        (None, _) => CanonicalScan::Declined,
    }
}

/// The canonical scan's running state: the [`SpecialMask`] its string arm
/// walks, and the one record [`canonical_compact_jq_scan_at_depth`] reads
/// back (#7310): after a decline, for the value itself, and after a
/// certification, for an object further in (#7366).
///
/// `repr(C)` with the mask first, so `&mut scan.specials` is the state's own
/// address: left to order the fields, the compiler put the record first, and
/// offsetting the pointer on every key cost the wide loop 2 instructions per
/// key on a 7950X.
#[repr(C)]
struct ScanState {
    specials: SpecialMask,
    /// Set by [`scan_canonical_object_wide`] when it declines at its closing
    /// `}` only because its keys' hashes repeat (`record_repeated_keys`).
    /// Written at most once per scan: that decline ends the scan for the value
    /// itself, and a second object declining so ends it for one further in
    /// (#7366).
    repeated_keys: Option<RepeatedKeys>,
    /// The depth the members of the value the scan was asked about sit at,
    /// when the caller reads [`ScanState::repeated_keys`] back
    /// ([`canonical_compact_jq_scan_at_depth`]), and 0 (no value's members sit
    /// there) when it does not. Only for such a caller is a repeat among an
    /// object's first keys scanned on to the closing `}` for the record
    /// (`pairwise_repeat`, #7355), and an object further in that declines on
    /// its keys scanned past (#7366); the other callers decline at the
    /// repeated key, as they always did.
    value_member_depth: usize,
}

/// [`ScanState::repeated_keys`]: how deep the members of the object that
/// declined on its keys were scanned (one more than the object itself), where
/// it starts and ends, and which of its key hashes repeat.
struct RepeatedKeys {
    member_depth: usize,
    /// The object's `{`, or [`RepeatedKeys::START_UNKNOWN`] until the frame
    /// that scanned the `{` fills it in (`fill_repeat_start`): the wide tier,
    /// which records the object, does not keep it, since a second position
    /// live across its loop costs every key (#7310).
    start: usize,
    end: usize,
    shared: Vec<u64>,
}

impl RepeatedKeys {
    /// [`RepeatedKeys::start`] before it is known.
    const START_UNKNOWN: usize = usize::MAX;
}

impl ScanState {
    #[inline]
    fn new() -> Self {
        Self {
            specials: SpecialMask::new(),
            repeated_keys: None,
            value_member_depth: 0,
        }
    }
}

/// The hash the canonical scan gives a key (#7310): [`key_hash`] of its raw
/// span, the bytes between its quotes with any escape left as written, or
/// `None` for a key that is not a string token.
///
/// Within one certified object it identifies keys exactly as their decoded
/// spellings do, because the scan only certifies a key spelled the one way
/// jq's writer spells it, so two keys have the same raw span exactly when
/// they decode to the same string. It differs from [`key_hash_of`]'s decoded
/// hash for a key with an escape in it, which is why a caller holding the
/// scan's shared hashes compares keys under this one.
pub(crate) fn raw_key_hash<W: AsRef<[u64]> + Clone>(key: &StandardJson<'_, W>) -> Option<u64> {
    match key {
        StandardJson::String(s) => {
            let (raw, _escaped, _has_del) = s.raw_and_escaped();
            (raw.len() >= 2).then(|| key_hash(&raw[1..raw.len() - 1]))
        }
        _ => None,
    }
}

/// The whole-buffer form of `canonical_compact_jq_span_end`: true iff
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
///
/// `inline(always)` (#3167): inlined into the array and object loops, a
/// scalar element is scanned in the loop itself and only a container pays
/// the recursive call, which was most of the scan's own cost on a
/// numbers-only document. The depth check stays on every element, scalars
/// included, so a scalar at the ceiling still bails exactly where
/// `stream_json_pretty`'s counter would. The string arm stays one
/// out-of-line call ([`scan_json_string_span`]).
#[inline(always)]
fn scan_canonical_value(
    bytes: &[u8],
    pos: usize,
    depth: usize,
    scan: &mut ScanState,
) -> Option<usize> {
    if depth >= MAX_VALUE_TREE_DEPTH {
        return None;
    }
    let byte = *bytes.get(pos)?;
    match byte {
        b'{' => scan_canonical_object(bytes, pos, depth, scan),
        b'[' => scan_canonical_array(bytes, pos, depth, scan),
        b'"' => scan_json_string_span(bytes, pos, &mut scan.specials).map(|(.., end)| end),
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
///
/// "Exactly" is [`keyword_span`]'s whole-token rule, the same one
/// `value_at`/`text_range` apply (#3035): a prefix `nullx` or a truncated
/// `nul` is not the literal, or the echo branch would commit to re-rendering
/// a span the other two scanners reject.
fn scan_canonical_literal(bytes: &[u8], pos: usize, literal: &[u8]) -> Option<usize> {
    keyword_span(bytes, pos, literal)
}

/// The number-token rule from `canonical_compact_jq_span_end`'s own doc
/// comment: the maximal `[-+.eE0-9]*` run ([`nested_number_span`]'s span)
/// is jq's own canonical spelling iff [`is_jq_canonical_number`] says so.
///
/// Answered in one pass (#3167): [`jq_canonical_number_prefix`] reads the
/// canonical prefix straight off the document, and the span is canonical
/// exactly when the byte after that prefix could not extend
/// [`nested_number_span`]'s greedy class. Every byte of the prefix is in
/// that class (digits, `.`, a leading `-`), so if the next byte is not,
/// the greedy span ends where the prefix does and the whole span is
/// canonical; if it is (an exponent, a second `.`, a digit after a lone
/// `0`, a trailing sign), the span runs on past a prefix that is not all
/// of it, which [`is_jq_canonical_number`] rejects. A special word
/// (`-inf`, `nan`) has no integer part, so it fails the prefix either way.
/// `scan_canonical_number_agrees_with_the_two_pass_rule_3167` pins the
/// equivalence against the two-pass form this replaced, which read every
/// number twice: 48 Ir per byte of a numbers-only document on a 7950X,
/// 42% of the whole `-c .` run.
fn scan_canonical_number(bytes: &[u8], pos: usize) -> Option<usize> {
    let end = pos + jq_canonical_number_prefix(&bytes[pos..])?;
    match bytes.get(end) {
        None => Some(end),
        Some(&b) => jq_literal_boundary(b).then_some(end),
    }
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
/// **UTF-8 validity is deliberately not decided here.** Every byte `>= 0x80`
/// is skipped over as literal content (it is not a special, see below), and the single
/// `core::str::from_utf8` `stream_json`'s echo branch already runs over the
/// whole accepted span is what actually rules on it -- a `from_utf8`
/// failure there falls through to the re-render, which produces the real
/// decode diagnostic. That is why the branch cannot treat a `from_utf8`
/// failure as an error of its own (an earlier draft did, and with the
/// in-scanner decode gone it would have turned a document the re-render
/// reports on into a bare formatting error).
///
/// This reverses #2919's reasoning, which put the decode inline precisely
/// to avoid "a second full walk of the string's bytes". The walk is real
/// but it is not the expensive one: measured on a 7950X, the byte-at-a-time
/// loop calling [`decode_code_point`](crate::text::utf8::decode_code_point)
/// cost 11.2 Ir per input byte -- 34% of
/// the whole gate -- while `from_utf8` over the accepted span cost 0.44,
/// because std's validator is vectorised and this loop was not. Moving the
/// work from a scalar per-byte decode to one vectorised pass is a ~25x
/// reduction on that term, not a duplicated cost.
///
/// **The walk itself is over a specials mask, not bytes (#3340).** A byte is
/// *plain* unless it is one of the four the arms above decide on (`"`, `\`,
/// control `< 0x20`, DEL), so the scan asks [`SpecialMask`] for the next one
/// and never visits the bytes between. The mask classifies a 64-byte block
/// once, when a string first reaches it, and every string inside the block is
/// answered from the same word with a shift and a `trailing_zeros`: no vector
/// call is paid per string or per escape, which is what made a SIMD skip
/// inside each string lose on short keys and escape-dense text (#3168). It is
/// lazy on purpose: `bytes` runs to the end of the document even when the
/// value is a small sub-value, and a document of numbers has no strings, so a
/// block is classified only when a string asks for it. The previous
/// byte-at-a-time loop survives in this module's tests as the independent
/// oracle the block walk is swept against.
///
/// Soundness is unchanged: none of `"`, `\`, or a control/DEL byte can ever
/// appear as a byte of a UTF-8 sequence, valid or not (each is `< 0x80` or
/// exactly `0x7F`, and every byte of a multi-byte sequence is `>= 0x80`), so
/// skipping a sequence can never walk past a closing quote, an
/// escape, or a byte the writer would have escaped. What the scan certifies
/// is therefore exactly "canonical **given** the span is valid UTF-8", and
/// the caller supplies the given.
///
/// The library cannot simply assume it: [`JsonIndex::build`] does no UTF-8
/// validation at all, and only the CLI substitutes invalid input ahead of
/// indexing (`utf8_lossy_document`, `src/bin/succinctly/jq_runner.rs`), so a
/// library caller can hand a `JsonCursor` a buffer with any byte in it.
fn scan_json_string_span(
    bytes: &[u8],
    pos: usize,
    specials: &mut SpecialMask,
) -> Option<(usize, usize, usize)> {
    debug_assert_eq!(bytes.get(pos), Some(&b'"'));
    let content_start = pos + 1;
    let mut i = content_start;
    loop {
        // Jump straight to the next byte this arm must decide on (#3340):
        // the plain bytes between are never visited, and no per-string
        // vector entry is paid -- `specials` answers from a 64-byte block it
        // classified once. No special before the end of the buffer is an
        // unterminated string, which is not canonical.
        i = specials.next_special(bytes, i)?;
        match bytes[i] {
            b'"' => return Some((content_start, i, i + 1)),
            b'\\' => i += canonical_escape_len(bytes, i)?,
            // A literal control byte or DEL, which the writer always escapes.
            _ => return None,
        }
    }
}

/// The length of the escape sequence that starts at the `\` at `bytes[i]`
/// (2 for a short escape, 6 for a `\u00XX`), iff it is exactly one
/// [`write_json_body_jq`] would itself emit; `None` otherwise. The escape
/// half of [`scan_json_string_span`]'s accept rule, see its doc comment.
fn canonical_escape_len(bytes: &[u8], i: usize) -> Option<usize> {
    debug_assert_eq!(bytes.get(i), Some(&b'\\'));
    match *bytes.get(i + 1)? {
        b'"' | b'\\' | b'b' | b'f' | b'n' | b'r' | b't' => Some(2),
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
            matches!(cp, 0x00..=0x07 | 0x0B | 0x0E..=0x1F | 0x7F).then_some(6)
        }
        // `/` (never escaped by jq's writer), any other letter, or an
        // unrecognized byte after `\` -- all not canonical.
        _ => None,
    }
}

/// The object-token rule: `{`, then either an immediate `}` or
/// `"key":value` pairs separated by exactly one `,` with no trailing
/// comma, each key checked against [`scan_json_string_span`] and hashed
/// (raw span, undecoded -- see `canonical_compact_jq_span_end`'s own doc
/// comment for why that's sound) to bail on any repeat -- pairwise below
/// [`PAIRWISE_SPAN_SCAN_LIMIT`] keys, through one exactly sized
/// [`KeyHashes::has_repeat`] table per object above it
/// (`scan_canonical_object_wide`). A repeat among the first keys is the table's too when the
/// caller reads the record of one ([`ScanState::value_member_depth`], `pairwise_repeat`, #7355),
/// for the value and, while nothing is recorded yet, for an object further in (#7366).
fn scan_canonical_object(
    bytes: &[u8],
    pos: usize,
    depth: usize,
    scan: &mut ScanState,
) -> Option<usize> {
    debug_assert_eq!(bytes.get(pos), Some(&b'{'));
    let mut i = pos + 1;
    if bytes.get(i) == Some(&b'}') {
        return Some(i + 1);
    }
    // Small-object fast path (#2919 review): up to `PAIRWISE_SPAN_SCAN_LIMIT`
    // keys are compared pairwise via a cheap fingerprint, with no
    // allocation at all. The hash-collecting tier
    // (`scan_canonical_object_wide`) only takes over once an object turns
    // out to have more keys than that; see `PAIRWISE_SPAN_SCAN_LIMIT`'s own
    // doc comment.
    let mut small_spans: [(usize, usize); PAIRWISE_SPAN_SCAN_LIMIT] =
        [(0, 0); PAIRWISE_SPAN_SCAN_LIMIT];
    let mut small_fps: [u64; PAIRWISE_SPAN_SCAN_LIMIT] = [0; PAIRWISE_SPAN_SCAN_LIMIT];
    let mut small_count = 0usize;
    loop {
        if bytes.get(i) != Some(&b'"') {
            return None;
        }
        let (key_start, key_end, after_key) = scan_json_string_span(bytes, i, &mut scan.specials)?;
        let fp = key_span_fingerprint(&bytes[i..after_key]);
        let repeat = (0..small_count).any(|j| {
            let (s, e) = small_spans[j];
            small_fps[j] == fp && bytes[s..e] == bytes[key_start..key_end]
        });
        if repeat {
            // A genuine duplicate key (the spans are compared byte for byte), which no
            // canonical span holds. The rest of the object is scanned by the table tier, so
            // that the keys the repeat shares are recorded for a caller to print from the text
            // (#7355) instead of walking and re-rendering every member.
            return pairwise_repeat(
                bytes,
                pos,
                &small_spans[..small_count],
                (key_start, key_end),
                after_key,
                depth,
                scan,
            );
        }
        if small_count < PAIRWISE_SPAN_SCAN_LIMIT {
            small_fps[small_count] = fp;
            small_spans[small_count] = (key_start, key_end);
            small_count += 1;
        } else {
            // Overflow past the pairwise limit: the rest of the object is
            // the table tier's, seeded with the hashes of what the
            // pairwise scan already collected plus this key (#3333). Whether
            // any two repeat is decided there, once -- including a bare
            // 64-bit hash collision between keys the pairwise scan already
            // proved distinct, which this whole function treats as "bail,
            // don't try to disambiguate" (see `KeyHashes`'s own doc comment
            // on `insert`).
            return pairwise_overflow(
                bytes,
                pos,
                &small_spans[..small_count],
                (key_start, key_end),
                after_key,
                depth,
                scan,
            );
        }
        if bytes.get(after_key) != Some(&b':') {
            return None;
        }
        i = scan_canonical_value(bytes, after_key + 1, depth + 1, scan)?;
        match bytes.get(i) {
            Some(b',') => i += 1,
            Some(b'}') => return Some(i + 1),
            _ => return None,
        }
    }
}

/// The rest of an object [`scan_canonical_object`] found a repeated key in among its first
/// [`PAIRWISE_SPAN_SCAN_LIMIT`] keys, `spans` being the keys before it and `key` the repeat,
/// entered with `after_key` just past it (#7355): [`scan_canonical_object_wide`] from there, with
/// every hash so far. Without it the pairwise tier declined at the key with no record, and every
/// caller walked and re-rendered the whole object, which the same repeat after the pairwise
/// tier did not cost. Out of line and cold, so that the small-object loop keeps no state for it.
#[cold]
#[inline(never)]
fn pairwise_repeat(
    bytes: &[u8],
    pos: usize,
    spans: &[(usize, usize)],
    key: (usize, usize),
    after_key: usize,
    depth: usize,
    scan: &mut ScanState,
) -> Option<usize> {
    // Only a caller that reads the record can use it, and only the first object to decline
    // on its keys: any other scan, and a second such object, is declined at the key as it
    // always was, and not scanned to its end for nothing. An object further in is scanned on
    // as the value is, since the scan then goes on past it (#7366).
    if scan.value_member_depth == 0 || scan.repeated_keys.is_some() {
        return None;
    }
    pairwise_overflow(bytes, pos, spans, key, after_key, depth, scan)
}

/// The rest of an object whose `{` is at `pos`, from the key `key` that [`scan_canonical_object`]
/// handed over to [`scan_canonical_object_wide`] at, `spans` being the keys before it and
/// `after_key` just past it: the key past the pairwise tier (#3333), or one repeating an earlier
/// one (`pairwise_repeat`). The table tier is seeded with every key's [`key_hash`] so far.
///
/// Out of line, so that the pairwise loop's hand-off is a tail call: the table tier does not
/// keep the object's `{`, since a second position live across its loop costs every key (#7310),
/// and a record it makes of the object is given it here, once the tier returns (#7366). With that
/// fill-in after the call in the pairwise loop itself, every object paid 2 instructions more on a
/// 7950X, in the frame the loop had to keep for it.
#[inline(never)]
fn pairwise_overflow(
    bytes: &[u8],
    pos: usize,
    spans: &[(usize, usize)],
    key: (usize, usize),
    after_key: usize,
    depth: usize,
    scan: &mut ScanState,
) -> Option<usize> {
    let mut hashes = Vec::with_capacity(PAIRWISE_SPAN_SCAN_LIMIT * 4);
    for &(s, e) in spans {
        hashes.push(key_hash(&bytes[s..e]));
    }
    hashes.push(key_hash(&bytes[key.0..key.1]));
    let end = scan_canonical_object_wide(bytes, after_key, depth, scan, hashes);
    if scan.repeated_keys.is_some() {
        fill_repeat_start(scan, pos);
    }
    end
}

/// Fills in the `{` at `pos` as the start of the object [`ScanState::repeated_keys`]
/// records, if that is the object whose scan just returned to the frame that scanned its `{`
/// (#7366): an object recorded further in has had its own filled in by then, by its own frame.
#[cold]
#[inline(never)]
fn fill_repeat_start(scan: &mut ScanState, pos: usize) {
    if let Some(record) = &mut scan.repeated_keys {
        if record.start == RepeatedKeys::START_UNKNOWN {
            record.start = pos;
        }
    }
}

/// The rest of an object [`scan_canonical_object`] found wider than
/// [`PAIRWISE_SPAN_SCAN_LIMIT`] keys (#3333), entered with `after_key` just
/// past the key that overflowed the pairwise tier (or, for a caller that reads the record, that
/// repeated an earlier one, `pairwise_repeat`) and `hashes` holding
/// every key's [`key_hash`] up to and including it. Both entries build that list in
/// `pairwise_overflow`.
///
/// Past the pairwise tier the keys' hashes are only *collected*, and one
/// table sized for exactly that many keys settles them at the closing `}`
/// ([`KeyHashes::has_repeat`]). A table grown key by key was 83 Ir per key
/// on a wide object, about half of it rehashing what it had already
/// placed. An object of [`KeyHashes::SATURATING_KEYS`] keys or more, too
/// wide for that table, is settled by [`shared_key_hashes_if_any`] instead, the
/// sort every other batch caller uses past the table's ceiling (#7293).
/// It used to be declined there, which made the scan of its first 786,432
/// keys pure waste: every caller then walks the object, and the walk
/// hashes and sorts every key itself. The list costs 8 bytes per key, as
/// the walk's does.
///
/// A repeat is therefore found at the end of the object rather than at the
/// key. That is the cost of a bail only: the span is declined either way and
/// the re-render that follows is far larger than the rest of one object's
/// scan. An early checkpoint over the first hashes would recover it for a
/// repeat inside them, but charges every no-repeat object just past its width
/// more than it saves, so it was not taken (#4156; `docs/parsing/json.md`,
/// "Rejected: a checkpoint for a front-loaded repeat"). A repeat among the first keys is a
/// different case: the pairwise tier has already found it, so no check is paid for it, and the
/// scan goes on only for a caller that reads the record (#7355).
///
/// A separate function, not a branch in the loop above, so the small-object
/// loop that nearly every object takes keeps no wide-tier state and no
/// per-key test for it.
#[inline(never)]
fn scan_canonical_object_wide(
    bytes: &[u8],
    mut after_key: usize,
    depth: usize,
    scan: &mut ScanState,
    mut hashes: Vec<u64>,
) -> Option<usize> {
    // The members' depth, the one depth the loop needs: also the one the
    // record names, so that no second depth stays live across the loop (with
    // the object's own depth kept for the record, it cost the loop a stack
    // reload per key on a 7950X, #7310).
    let depth = depth + 1;
    loop {
        if bytes.get(after_key) != Some(&b':') {
            return None;
        }
        let mut i = scan_canonical_value(bytes, after_key + 1, depth, scan)?;
        match bytes.get(i) {
            Some(b',') => i += 1,
            Some(b'}') => {
                // Below the table's ceiling the exactly sized table settles the list; at or
                // past it, the sort (#7293). Branched here rather than in a helper so the
                // table's settle stays inlined into this loop, as it was before the sort.
                // A repeat the table finds, and every list past the ceiling,
                // go to the sort, which names the hashes that repeat (#7310).
                if hashes.len() < KeyHashes::SATURATING_KEYS && !KeyHashes::has_repeat(&hashes) {
                    return Some(i + 1);
                }
                return record_repeated_keys(scan, depth, i + 1, sorted_keys_shared(hashes));
            }
            _ => return None,
        }
        if bytes.get(i) != Some(&b'"') {
            return None;
        }
        let (key_start, key_end, next_after_key) =
            scan_json_string_span(bytes, i, &mut scan.specials)?;
        hashes.push(key_hash(&bytes[key_start..key_end]));
        after_key = next_after_key;
    }
}

/// Records an object [`scan_canonical_object_wide`] declines at its closing
/// `}` only because the hashes in `shared` repeat among its keys, its members
/// scanned at `member_depth` and the object ending at `end` (#7310), and
/// declines it; or, when `shared` is empty, certifies it.
///
/// Every member is certified by then, so the record is all a caller needs to
/// skip its own walk of the keys ([`CanonicalScan::RepeatedKeys`]). Out of
/// line and cold, so that the loop's common exit stays as small as it was.
///
/// An object further in than the value the scan was asked about is certified
/// once recorded (#7366): the scan goes on past it, and its caller prints the
/// text around it ([`CanonicalScan::NestedRepeatedKeys`]). Only the first
/// object is recorded, and a scan whose caller does not read the record
/// records none: an object not recorded, a second one or any in such a scan,
/// declines.
#[cold]
#[inline(never)]
fn record_repeated_keys(
    scan: &mut ScanState,
    member_depth: usize,
    end: usize,
    shared: Vec<u64>,
) -> Option<usize> {
    if shared.is_empty() {
        return Some(end);
    }
    if scan.value_member_depth == 0 || scan.repeated_keys.is_some() {
        return None;
    }
    scan.repeated_keys = Some(RepeatedKeys {
        member_depth,
        start: RepeatedKeys::START_UNKNOWN,
        end,
        shared,
    });
    (member_depth > scan.value_member_depth).then_some(end)
}

/// The key hashes that occur more than once, for an object of
/// [`KeyHashes::SATURATING_KEYS`] keys or more (#7293, #7310):
/// [`shared_key_hashes_if_any`]'s sort, since an exactly sized table would outgrow
/// the table's ceiling there and [`KeyHashes::has_repeat`] only answers the
/// conservative `true`. Both are conservative in the same way: two distinct
/// keys sharing a 64-bit hash count as a repeat, which declines the span. The
/// list rather than a yes/no, so that a decline hands the caller the hashes
/// the keys share (#7310). Out of line, so that the sort does not cost
/// `has_repeat` its inlining into `scan_canonical_object_wide`: with both
/// behind one inlined helper the table lost it, and the scan of `wide` 10 MB
/// read 1.8 M more instructions (7950X).
#[inline(never)]
fn sorted_keys_shared(hashes: Vec<u64>) -> Vec<u64> {
    shared_key_hashes_if_any(hashes)
}

/// The array-token rule: `[`, then either an immediate `]` or `value`
/// items separated by exactly one `,` with no trailing comma.
fn scan_canonical_array(
    bytes: &[u8],
    pos: usize,
    depth: usize,
    scan: &mut ScanState,
) -> Option<usize> {
    debug_assert_eq!(bytes.get(pos), Some(&b'['));
    let mut i = pos + 1;
    if bytes.get(i) == Some(&b']') {
        return Some(i + 1);
    }
    loop {
        i = scan_canonical_value(bytes, i, depth + 1, scan)?;
        match bytes.get(i) {
            Some(b',') => i += 1,
            Some(b']') => return Some(i + 1),
            _ => return None,
        }
    }
}

impl<'a, W: AsRef<[u64]> + Clone> DocumentCursor for JsonCursor<'a, W> {
    const PROJECTS_MEMBERS: bool = true;

    /// `JsonElements::uncons_cursor` yields the first child, then each
    /// `next_sibling`: the chain is exactly the siblings.
    const RESUMABLE_ELEMENT_SCAN: bool = true;

    #[inline]
    fn next_element(&self) -> Option<Self> {
        JsonCursor::next_sibling(self)
    }

    const STEPS_BACK: bool = true;

    #[inline]
    fn prev_element(&self) -> Option<Self> {
        JsonCursor::prev_sibling(self)
    }

    /// The balanced-parentheses excess at this node's open: a rank lookup.
    #[inline]
    fn tree_depth(&self) -> Option<usize> {
        self.index.bp().depth(self.bp_pos)
    }

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

    /// A JSON object's owned value holds one entry per distinct key, and its
    /// array one element per child: no merge keys, no wrapper nodes (#3483).
    #[inline]
    fn materializes_members_one_to_one(&self) -> bool {
        true
    }

    fn certified_well_formed(&self, depth: usize) -> bool {
        // The scan counts against the printer's ceiling (384), the walk against its own (256):
        // the walk's `depth` is the scan's `depth` plus the difference (#3909).
        self.canonical_compact_span(
            MAX_VALUE_TREE_DEPTH - crate::jq::eval_generic::MAX_NESTING_DEPTH + depth,
        )
        .is_some()
    }

    fn plain_owned_jq(&self) -> Option<OwnedValue> {
        if !self.is_container() {
            return None;
        }
        let (value, nodes) = crate::jq::parse_plain_at(self.text, self.text_position()?)?;
        // The read holds only where the index nests the container as the text does.
        (self.index.bp().subtree_size(self.bp_pos)? == nodes).then_some(value)
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

    /// The matching close parenthesis: every descendant's `bp_pos` lies
    /// strictly between it and this node's own (#3179).
    #[inline]
    fn subtree_end(&self) -> Option<usize> {
        self.index.bp().find_close(self.bp_pos)
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

    /// `text` is a borrow of the same buffer the index was built from
    /// (#3035; same reasoning as `document_token`'s own doc above).
    #[inline]
    fn doc_text(&self) -> &'a [u8] {
        self.text
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

    /// #5654: see [`DocumentCursor::number_glued_junk`].
    fn number_glued_junk(&self, text_pos: usize) -> bool {
        let text = self.text;
        let mut i = text_pos;
        if matches!(text.get(i), Some(b'-' | b'+')) {
            i += 1;
        }
        let body = i;
        while text.get(i).copied().is_some_and(is_number_span_byte) {
            i += 1;
        }
        i > body
            && text
                .get(i)
                .copied()
                .is_some_and(|b| !jq_literal_boundary(b))
    }

    /// #5683: see [`DocumentCursor::plain_int_at`].
    #[inline]
    fn plain_int_at(&self, text_pos: usize) -> Option<i64> {
        debug_assert_eq!(
            self.text_position(),
            Some(text_pos),
            "plain_int_at reads this cursor's own position"
        );
        let text = self.text;
        let mut i = text_pos;
        let negative = *text.get(i)? == b'-';
        i += usize::from(negative);
        let digits = i;
        while text.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        let len = i - digits;
        // `0` alone, or 1-18 digits with no leading zero: the run `i64` holds exactly, and the
        // spelling the general decode reads as the same integer. `-0` is left to it.
        if len == 0 || len > 18 || (text[digits] == b'0' && (len > 1 || negative)) {
            return None;
        }
        // The token ends here: anything else glued on (`1.5`, `1e3`, `12x`) is not this integer.
        if !matches!(
            text.get(i),
            None | Some(b' ' | b'\t' | b'\n' | b'\r' | b',' | b']' | b'}')
        ) {
            return None;
        }
        let mut value = 0i64;
        for &b in &text[digits..i] {
            value = value * 10 + i64::from(b - b'0');
        }
        Some(if negative { -value } else { value })
    }

    /// #5654: where this container ends, one past its closing bracket, for the trailing-gap
    /// check after a container that is its parent's last child.
    #[inline]
    fn container_text_end(&self) -> Option<usize> {
        self.container_end_bp()
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
        //
        // One scan answers both layouts (#7271 for the indented one, below),
        // and its decline on a wide object's repeated keys hands the
        // re-render the hashes it needs (#7310, below too).
        let scan = if !sort_keys && numbers == JsonConvention::JqCompat {
            self.canonical_compact_scan(0)
        } else {
            CanonicalSpan::Declined
        };
        if indent.is_compact() {
            if let CanonicalSpan::Echo(span) = scan {
                return Ok(out.write_str(span)?);
            }
            // Falls through to the general re-render path below -- no text
            // position was available, the span wasn't certified canonical,
            // or it was but turned out not to be valid UTF-8.
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
        // #7271: an indented, unsorted default render of a certified span is
        // the span itself with jq's whitespace put between its tokens, so it
        // is written by re-indenting the span's bytes (`write_reindented`)
        // rather than by walking the tree, and out a chunk at a time rather
        // than whole. The certificate says the compact render of this span
        // succeeds and is the span byte for byte; the indented render makes
        // the same decisions in the same order and only adds whitespace, so
        // it cannot fail part-way, and nothing is written that a later fault
        // would have had to retract. `-S` (a different walk over decoded
        // keys), other number conventions and every span the scan declines
        // (whitespace inside it, a spelling jq rewrites, a repeated key,
        // invalid UTF-8, a fault) keep the whole-value buffer below, and with
        // it jq's print-nothing on a malformed document. The compact case
        // returned its echo already, above. One decline is written from the
        // text as well (#7346): an object certified member by member and
        // declined only on its keys, alone or inside a value certified around
        // it (#7366), whose pieces are all checked before any is written
        // (`stream_scanned_repeats`).
        if let CanonicalSpan::Echo(span) = scan {
            let mut chunked = ChunkedWrite {
                out,
                buf: String::new(),
            };
            write_reindented(&mut chunked, span, indent.width, indent.unit)?;
            return Ok(chunked.finish()?);
        }
        let mut buf = String::new();
        match (scan, value) {
            // #7310: the scan certified every member of this object and
            // declined only on its keys' hashes, so it is written from its
            // text (#7346), or buffered when it cannot be. See
            // `stream_scanned_repeats`. Kept an arm of this match: as an
            // early `if let` ahead of the buffer, the same call moved the
            // inlined `write_reindented` loop above, and clean pretty `.`
            // read 0.7% to 1.9% more instructions on a 7950X (8.8% more time
            // on `numbers`); as an arm it reads what #7310's did.
            (
                CanonicalSpan::RepeatedKeys {
                    start,
                    span,
                    shared,
                },
                StandardJson::Object(fields),
            ) => {
                let text = RepeatText::whole(span, start);
                return stream_scanned_repeats(out, fields, text, &shared, indent, numbers);
            }
            // #7366: certified but for one object inside it, which declined
            // the same way. See `stream_around_scanned_repeats`.
            (CanonicalSpan::NestedRepeatedKeys { span, object }, value) => {
                return stream_around_scanned_repeats(
                    out, self, value, span, &object, indent, numbers,
                );
            }
            (_, value) => stream_json_pretty(
                &mut buf,
                value,
                0,
                indent.width,
                indent.unit,
                sort_keys,
                numbers,
                0,
            )?,
        }
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

    /// Decodes nothing (#2692). A malformed number is an `Error` value since
    /// #3222 and so truthy here, as every value the index cannot read is;
    /// it raises where something reads it. Until then it was a `Number` the
    /// printer rendered as `null`, and this answered falsy for it under the
    /// output's `JsonConvention` to keep `-e` consistent (#1576).
    #[inline]
    fn is_falsy(&self) -> bool {
        matches!(self.value(), StandardJson::Null | StandardJson::Bool(false))
    }

    /// [`is_falsy`](Self::is_falsy)'s one `value()`, with the token the index
    /// could not read raising instead of reading as truthy (#7449).
    #[inline]
    fn truthiness(&self) -> Result<bool, EvalError> {
        crate::jq::json_truthiness(&self.value())
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

    fn to_owned_values<S: EvalSemantics>(cursors: &[Self]) -> Result<Vec<OwnedValue>, EvalError> {
        cursors
            .iter()
            .map(|cursor| crate::jq::to_owned_standard_json::<S, W>(&cursor.value()))
            .collect()
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
            StandardJson::Number(n) => n.bridge_computed_float(),
            _ => None,
        }
    }

    fn bridge_computed_int(&self) -> Option<i64> {
        match self {
            StandardJson::Number(n) => n.bridge_computed_int(),
            _ => None,
        }
    }

    fn number_token_address(&self) -> Option<usize> {
        match self {
            StandardJson::Number(n) => n.token_address(),
            _ => None,
        }
    }

    fn number_text(&self) -> Option<Cow<'_, str>> {
        match self {
            StandardJson::Number(n) => core::str::from_utf8(n.raw_bytes()).ok().map(Cow::Borrowed),
            _ => None,
        }
    }

    fn valid_number_text(&self) -> Option<Cow<'_, str>> {
        match self {
            // #7315: the grammar pass is the UTF-8 check too (`valid_number_str`).
            StandardJson::Number(n) => {
                crate::json::validate::valid_number_str(n.raw_bytes()).map(Cow::Borrowed)
            }
            _ => None,
        }
    }

    fn number_literal(&self) -> Option<Cow<'_, str>> {
        match self {
            StandardJson::Number(n) => {
                // The ordinary document number: no bridge token to decode, and an RFC 8259
                // spelling. Everything else is `lenient_number_literal`.
                if !n.bridge {
                    if let Some(text) = crate::json::validate::valid_number_str(n.raw_bytes()) {
                        return Some(Cow::Borrowed(text));
                    }
                }
                lenient_number_literal(n)
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

    /// [`key_raw_unescaped`](DocumentValue::key_raw_unescaped) and the span's
    /// end from the one `raw_and_escaped` scan (#3343). An unterminated span
    /// (`raw` not ending in its closing quote) answers `None`, and that guard
    /// is load-bearing: `find_end` stops at the end of the text there, so
    /// `start + raw.len()` would now agree with it, but an unterminated key
    /// must not be answered from this one scan. It takes the two-call path
    /// (#5541).
    #[inline]
    fn key_raw_unescaped_with_end(&self) -> Option<(&[u8], usize)> {
        match self {
            StandardJson::String(s) => {
                let (raw, escaped, _has_del) = s.raw_and_escaped();
                if escaped || raw.len() < 2 || raw[raw.len() - 1] != b'"' {
                    None
                } else {
                    Some((&raw[1..raw.len() - 1], s.start() + raw.len()))
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

/// `DocumentValue::number_literal` of a number that is not an RFC 8259 spelling read off a
/// document: a reindex bridge token, or one of the lenient spellings jq reads.
///
/// Out of line and cold for the reason `JsonCursor::lenient_number_at` is (#7384): the
/// `number_literal` a fold over a document's numbers calls once per element is then the bridge
/// test and one grammar check.
#[cold]
#[inline(never)]
fn lenient_number_literal<'a>(n: &JsonNumber<'a>) -> Option<Cow<'a, str>> {
    // #1071: the bridge's token for a document `.inf`/`.NaN` stands for
    // the spelling the owned value carried.
    if let Some(literal) = n.bridge_nonfinite_literal() {
        return Some(Cow::Borrowed(literal));
    }
    // #2802: so does the token for a scalar written in a non-JSON spelling.
    if let Some(spelling) = n.bridge_spelling() {
        return Some(Cow::Owned(spelling));
    }
    let bytes = n.raw_bytes();
    if let Some(text) = crate::json::validate::valid_number_str(bytes) {
        return Some(Cow::Borrowed(text));
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
    if let Some(text) = n.bridge_signed_overflowed_int() {
        return Some(Cow::Borrowed(text));
    }
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

    #[inline]
    fn find_cursor_counted(&self, name: &str) -> (Result<Option<Self::Cursor>, EvalError>, usize) {
        JsonFields::find_cursor_counted(self, name)
    }

    #[inline]
    fn find_cursors_counted(
        &self,
        names: &[&str],
        found: &mut [Option<Self::Cursor>],
    ) -> (Result<(), EvalError>, usize) {
        JsonFields::find_cursors_counted(self, names, found)
    }

    /// The retained child cursor *is* the next key node (#3913); an exhausted
    /// list, and an empty container's tagged slot (#3180), hold none.
    #[inline]
    fn head_key_cursor(&self) -> Option<Self::Cursor> {
        self.current()
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
        match self.current() {
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

    fn head_id(&self) -> Option<(usize, usize)> {
        self.current().map(|c| (c.node_id(), c.document_token()))
    }

    /// An element's list is its cursor (`slot` untagged), and a cursor is
    /// `(text, index, bp_pos)`, so the node id rebuilds it exactly.
    fn at_head_id(&self, id: usize) -> Option<Self> {
        let cursor = self.current()?.at_node_id(id)?;
        Some(JsonElements { slot: Some(cursor) })
    }

    /// Re-runs the strict validator, mirroring
    /// [`JsonFields::malformed_member_error`]'s own reasoning (#1194) for
    /// the array delimiter class (#1677).
    fn malformed_element_error(&self) -> EvalError {
        match self.current() {
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

/// jq's indented spelling of `span`, a value [`JsonCursor::canonical_compact_span`]
/// certified (#7271): what [`stream_json_pretty`] writes for it with `width` copies of
/// `unit` per level, unsorted, under `JsonConvention::JqCompat`.
///
/// A certified span is exactly the compact render, so the indented render is the same
/// tokens with whitespace between them, and only five bytes decide where it goes, all of
/// them outside strings: an opening bracket starts a new line one level in (an empty
/// container stays `[]`/`{}`), a comma starts a new line at the same level, a closing
/// bracket starts a new line one level out, and a colon becomes `": "`. Strings, numbers
/// and literals are copied as they are, since the certificate says they are already jq's
/// spelling. `pretty_render_of_a_certified_span_is_its_reindentation_7271` checks this
/// against `stream_json_pretty` on generated documents.
fn write_reindented<Out: core::fmt::Write>(
    out: &mut Out,
    span: &str,
    width: usize,
    unit: char,
) -> core::fmt::Result {
    write_reindented_at(out, span, width, unit, 0)
}

/// [`write_reindented`] for text that starts `level` levels in, or that leaves
/// some brackets open (#7366): the text around an object written by other
/// means, whose lines are indented from where it starts, as the whole
/// value's would be at that point; or a value or a comma-separated run of
/// members of an object being written at that level (#7346), the caller
/// having written the line break and indentation before it.
#[inline(always)]
fn write_reindented_at<Out: core::fmt::Write + ?Sized>(
    out: &mut Out,
    span: &str,
    width: usize,
    unit: char,
    level: usize,
) -> core::fmt::Result {
    let bytes = span.as_bytes();
    let mut depth = level;
    let mut copied = 0usize;
    let mut i = 0usize;
    let line = |out: &mut Out, depth: usize| -> core::fmt::Result {
        out.write_char('\n')?;
        write_json_indent(out, depth * width, unit)
    };
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                // A certified string's only escapes are `\x` pairs, so the first unescaped
                // quote ends it.
                i += 1;
                while let Some(at) = bytes[i..].iter().position(|&b| b == b'"' || b == b'\\') {
                    i += at;
                    if bytes[i] == b'"' {
                        break;
                    }
                    i += 2;
                }
                i += 1;
            }
            open @ (b'[' | b'{') => {
                out.write_str(&span[copied..i])?;
                let close = if open == b'[' { b']' } else { b'}' };
                if bytes.get(i + 1) == Some(&close) {
                    out.write_str(if open == b'[' { "[]" } else { "{}" })?;
                    i += 2;
                } else {
                    out.write_char(open as char)?;
                    depth += 1;
                    line(out, depth)?;
                    i += 1;
                }
                copied = i;
            }
            close @ (b']' | b'}') => {
                out.write_str(&span[copied..i])?;
                depth -= 1;
                line(out, depth)?;
                out.write_char(close as char)?;
                i += 1;
                copied = i;
            }
            b',' => {
                out.write_str(&span[copied..i])?;
                out.write_char(',')?;
                line(out, depth)?;
                i += 1;
                copied = i;
            }
            b':' => {
                out.write_str(&span[copied..i])?;
                out.write_str(": ")?;
                i += 1;
                copied = i;
            }
            _ => i += 1,
        }
    }
    out.write_str(&span[copied..])
}

/// The bytes [`ChunkedWrite`] gathers before passing them on: large enough
/// that a render's many small writes reach the real writer as a few large
/// ones, small enough that the output is never held whole (#7271).
const CHUNKED_WRITE_BYTES: usize = 64 * 1024;

/// A writer that passes what it is given on to `out` a chunk of
/// [`CHUNKED_WRITE_BYTES`] at a time (a single write of that size or more goes on by itself,
/// whole, #7356), for a render that is known not to fail
/// and so need not be held back whole (#7271, `JsonCursor::stream_json`).
/// [`ChunkedWrite::finish`] writes what is left.
struct ChunkedWrite<'o, Out: core::fmt::Write> {
    out: &'o mut Out,
    buf: String,
}

impl<Out: core::fmt::Write> ChunkedWrite<'_, Out> {
    fn finish(self) -> core::fmt::Result {
        if self.buf.is_empty() {
            return Ok(());
        }
        self.out.write_str(&self.buf)
    }

    /// Passes the gathered chunk on. Out of line and cold, so that the render's many
    /// write sites inline only the push and the length check, as they do into a `String`.
    #[cold]
    #[inline(never)]
    fn pass_on(&mut self) -> core::fmt::Result {
        self.out.write_str(&self.buf)?;
        self.buf.clear();
        Ok(())
    }

    /// The write that reaches the limit. A write that is itself a chunk's worth goes to `out` as
    /// it is, after the gathered bytes, and is never pushed: gathered, it would have held a 50 MiB
    /// string token a second time (#7356). Any other write joins the buffer and the buffer is
    /// passed on, as every write that reaches the limit always was. Cold for the same reason as
    /// [`Self::pass_on`].
    #[cold]
    #[inline(never)]
    fn pass_on_with(&mut self, s: &str) -> core::fmt::Result {
        if s.len() < CHUNKED_WRITE_BYTES {
            self.buf.push_str(s);
            return self.pass_on();
        }
        if !self.buf.is_empty() {
            self.pass_on()?;
        }
        self.out.write_str(s)
    }
}

impl<Out: core::fmt::Write> core::fmt::Write for ChunkedWrite<'_, Out> {
    #[inline]
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        // The length is checked before the push, so a write of a chunk's worth or more is never
        // gathered: one string token of 50 MiB was held whole here (#7356).
        if self.buf.len() + s.len() >= CHUNKED_WRITE_BYTES {
            return self.pass_on_with(s);
        }
        self.buf.push_str(s);
        Ok(())
    }

    #[inline]
    fn write_char(&mut self, c: char) -> core::fmt::Result {
        self.buf.push(c);
        if self.buf.len() >= CHUNKED_WRITE_BYTES {
            return self.pass_on();
        }
        Ok(())
    }
}

#[cfg(test)]
mod pretty_streaming_tests_7271 {
    use super::*;

    /// Records every write, so a test can tell a render passed on in pieces from one held
    /// back and written whole.
    #[derive(Default)]
    struct Writes(Vec<usize>, String);

    impl core::fmt::Write for Writes {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            self.0.push(s.len());
            self.1.push_str(s);
            Ok(())
        }
    }

    const TWO: IndentSpec = IndentSpec {
        width: 2,
        unit: ' ',
    };

    /// About 200 KB of canonical compact JSON, three chunks' worth of indented output.
    fn certified_doc() -> String {
        let items: Vec<String> = (0..4000)
            .map(|i| {
                format!(
                    r#"{{"id":{i},"n":[1,-2.5,null,true],"s":"é😀\n{}"}}"#,
                    "x".repeat(10)
                )
            })
            .collect();
        format!(r#"{{"xs":[{}]}}"#, items.join(","))
    }

    fn render(json: &[u8], indent: IndentSpec, sort: bool) -> (StreamResult, Writes) {
        let index = JsonIndex::build(json);
        let root = index.root(json);
        let mut out = Writes::default();
        let result = root.stream_json(&mut out, indent, sort, JsonConvention::JqCompat);
        (result, out)
    }

    /// A certified span's indented render reaches the writer in chunks of at most
    /// `CHUNKED_WRITE_BYTES` (plus the write that crossed the line), not as one whole
    /// value, and the text is the one the buffered render writes (`-S` still buffers).
    #[test]
    fn a_certified_span_streams_its_indented_render_in_chunks_7271() {
        let doc = certified_doc();
        assert!(doc.len() > 3 * CHUNKED_WRITE_BYTES / 2);
        let (result, streamed) = render(doc.as_bytes(), TWO, false);
        result.unwrap();
        assert!(streamed.0.len() > 2, "{:?}", streamed.0);
        assert!(streamed.0.iter().all(|&n| n < 2 * CHUNKED_WRITE_BYTES));
        let (result, whole) = render(doc.as_bytes(), TWO, true);
        result.unwrap();
        assert_eq!(whole.0.len(), 1, "-S keeps the whole-value buffer");
        // Every object's keys are already in order, so `-S` renders the same text, whole.
        assert_eq!(streamed.1, whole.1);
        assert!(streamed
            .1
            .starts_with("{\n  \"xs\": [\n    {\n      \"id\": 0,\n      \"n\": [\n        1,"));
        assert!(streamed
            .1
            .ends_with("\"s\": \"é😀\\nxxxxxxxxxx\"\n    }\n  ]\n}"));
    }

    /// One write of a chunk's worth or more goes to the writer as it is, and never into the
    /// buffer: a 50 MiB string token was gathered whole before (#7356). The writes around it keep
    /// their order and the buffer stays below the limit.
    #[test]
    fn a_long_write_bypasses_the_chunk_buffer_7356() {
        let long = "x".repeat(3 * CHUNKED_WRITE_BYTES + 5);
        let mut out = Writes::default();
        let mut chunked = ChunkedWrite {
            out: &mut out,
            buf: String::new(),
        };
        core::fmt::Write::write_str(&mut chunked, "head,").unwrap();
        core::fmt::Write::write_str(&mut chunked, &long).unwrap();
        assert!(
            chunked.buf.capacity() < CHUNKED_WRITE_BYTES,
            "{}",
            chunked.buf.capacity()
        );
        core::fmt::Write::write_str(&mut chunked, ",tail").unwrap();
        chunked.finish().unwrap();
        assert_eq!(out.0, [5, long.len(), 5]);
        assert_eq!(out.1, format!("head,{long},tail"));

        // A write under the limit that reaches it with the buffer's help joins the buffer, and
        // the whole is passed on, as every write that reaches the limit always was.
        let mut out = Writes::default();
        let mut chunked = ChunkedWrite {
            out: &mut out,
            buf: String::new(),
        };
        let part = "y".repeat(CHUNKED_WRITE_BYTES - 3);
        core::fmt::Write::write_str(&mut chunked, &part).unwrap();
        core::fmt::Write::write_str(&mut chunked, "zzzz").unwrap();
        assert_eq!(chunked.buf, "");
        core::fmt::Write::write_str(&mut chunked, "w").unwrap();
        chunked.finish().unwrap();
        assert_eq!(out.0, [part.len() + 4, 1]);

        // The boundaries: a write of exactly the limit with an empty buffer bypasses it; one byte
        // short of it joins the buffer, which a `write_char` then carries to the limit.
        let mut out = Writes::default();
        let mut chunked = ChunkedWrite {
            out: &mut out,
            buf: String::new(),
        };
        let exact = "e".repeat(CHUNKED_WRITE_BYTES);
        core::fmt::Write::write_str(&mut chunked, &exact).unwrap();
        assert!(chunked.buf.is_empty() && chunked.buf.capacity() == 0);
        let short = "s".repeat(CHUNKED_WRITE_BYTES - 1);
        core::fmt::Write::write_str(&mut chunked, &short).unwrap();
        assert_eq!(chunked.buf.len(), short.len());
        core::fmt::Write::write_char(&mut chunked, 'c').unwrap();
        assert_eq!(chunked.buf, "");
        chunked.finish().unwrap();
        assert_eq!(out.0, [exact.len(), CHUNKED_WRITE_BYTES]);
    }

    /// The re-indenter writes what the tree walk writes, for every certified document the
    /// generator draws: nested and empty containers, keys and strings holding the structural
    /// bytes and escaped quotes, every canonical scalar, and each indentation unit and width.
    #[test]
    fn pretty_render_of_a_certified_span_is_its_reindentation_7271() {
        use rand::{RngExt, SeedableRng};
        use rand_chacha::ChaCha8Rng;

        const STRINGS: &[&str] = &[
            "",
            "a",
            "[",
            "]",
            "{",
            "}",
            ",",
            ":",
            "a, b: [c]",
            r#"\""#,
            r"\\",
            r#"\\\""#,
            "é😀",
            r"\n",
            r"\u0000",
            r"\u007f",
            " ",
            "x\u{7f}",
            r"]\u001f,",
            r"\u001f:",
            r"{\u001f",
            r"\u001f}",
        ];
        const SCALARS: &[&str] = &[
            "0", "-1", "42", "3.5", "-0.25", "1e+100", "true", "false", "null",
        ];
        fn value(rng: &mut ChaCha8Rng, depth: u32, out: &mut String) {
            if depth == 0 || rng.random_bool(0.3) {
                if rng.random_bool(0.5) {
                    out.push('"');
                    out.push_str(STRINGS[rng.random_range(0..STRINGS.len())]);
                    out.push('"');
                } else {
                    out.push_str(SCALARS[rng.random_range(0..SCALARS.len())]);
                }
                return;
            }
            let n = rng.random_range(0..5);
            if rng.random_bool(0.5) {
                out.push('[');
                for i in 0..n {
                    if i > 0 {
                        out.push(',');
                    }
                    value(rng, depth - 1, out);
                }
                out.push(']');
            } else {
                out.push('{');
                for i in 0..n {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&format!(
                        r#""k{i}{}":"#,
                        STRINGS[rng.random_range(0..STRINGS.len())]
                    ));
                    value(rng, depth - 1, out);
                }
                out.push('}');
            }
        }

        let mut rng = ChaCha8Rng::seed_from_u64(7271);
        let mut certified = 0;
        for _ in 0..3000 {
            let mut doc = String::new();
            value(&mut rng, 5, &mut doc);
            let index = JsonIndex::build(doc.as_bytes());
            let root = index.root(doc.as_bytes());
            let Some(span) = root.canonical_compact_span(0) else {
                continue;
            };
            certified += 1;
            for indent in [
                TWO,
                IndentSpec {
                    width: 1,
                    unit: ' ',
                },
                IndentSpec {
                    width: 7,
                    unit: ' ',
                },
                IndentSpec {
                    width: 1,
                    unit: '\t',
                },
            ] {
                let mut walked = String::new();
                stream_json_pretty(
                    &mut walked,
                    root.value(),
                    0,
                    indent.width,
                    indent.unit,
                    false,
                    JsonConvention::JqCompat,
                    0,
                )
                .unwrap();
                let mut reindented = String::new();
                write_reindented(&mut reindented, span, indent.width, indent.unit).unwrap();
                assert_eq!(reindented, walked, "{doc}");
            }
        }
        assert!(certified > 1500, "only {certified} documents certified");
    }

    /// Indentation written from a run of the unit is the unit repeated, at every width the
    /// run's length divides unevenly, for spaces, tabs and any other unit.
    #[test]
    fn indent_from_a_run_is_the_unit_repeated_7271() {
        for unit in [' ', '\t', '-'] {
            for n in 0..200 {
                let mut out = String::new();
                write_json_indent(&mut out, n, unit).unwrap();
                assert_eq!(out, unit.to_string().repeat(n), "{n} x {unit:?}");
            }
        }
    }

    /// A fault anywhere, even past the last chunk boundary, makes the scan decline the span,
    /// so the render is held back and nothing at all reaches the writer, as in jq.
    #[test]
    fn a_late_fault_writes_nothing_7271() {
        let doc = certified_doc();
        let late = doc.rfind(r#""id":"#).unwrap() + 5;
        let mut bad = doc.clone().into_bytes();
        bad[late] = b'@';
        let mut trailing = doc.as_bytes()[..doc.len() - 2].to_vec();
        trailing.extend_from_slice(b",]}");
        for json in [bad, trailing] {
            for indent in [
                TWO,
                IndentSpec {
                    width: 1,
                    unit: '\t',
                },
            ] {
                let (result, out) = render(&json, indent, false);
                assert!(result.is_err());
                assert!(out.0.is_empty(), "{:?}", out.0);
            }
        }
    }
}

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
            // Tracks the last element and its position, for the trailing-comma check after the
            // loop (`[1,2,]`): its end is resolved for the last one only (#5654, a container's end
            // is a lookup).
            let mut last_child: Option<(JsonCursor<'_, W>, usize)> = None;
            // Cursor-yielding `uncons_cursor`, not the plain value
            // `Iterator`/`IntoIterator` impl: #1677's missing/doubled
            // `,` check needs each element's own `text_position()`, which
            // only the cursor carries -- `to_owned_at_depth`'s identical
            // array arm (`eval_generic.rs`) is the precedent this mirrors.
            while let Some((elem_cursor, next)) = rest.uncons_cursor() {
                if !first {
                    out.write_char(',')?;
                }
                last_child = None;
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
                    last_child = Some((elem_cursor, pos));
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
            // #1676: a trailing `,` (`[1,2,]`), or (#5654) junk after a container that is the
            // last element.
            if let Some((cursor, gap_start)) = last_child
                .and_then(|(cursor, pos)| cursor.last_child_text_end(pos).map(|end| (cursor, end)))
            {
                if !trailing_gap_ok(cursor.text(), gap_start, b']') {
                    return Err(StreamFailure::Decode(EvalError::malformed_json_text(
                        cursor.text(),
                    )));
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
            // first, which is `-S` (every key, to sort). Every other
            // object is validated by one key-only walk
            // (`repeat_plan_checked`, #1194/#1677/#2261's checks, the same
            // ones the materializing `effective_fields_checked` runs) and
            // then streamed straight off `uncons` in document order. The
            // materialized path used to be unconditional: a 144-byte
            // `DocumentField` per field, 22.9 MB for perf-guard's 2 MB
            // `wide` fixture, and half of the identity print's
            // instructions on a 7950X.
            //
            // #7311: an object that repeats a key under a collapsing mode
            // streams too. Its plan names the few fields the rule changes
            // (a repeated key's first occurrence takes its last value, and
            // its later occurrences go), found by decoding only the keys
            // whose hash another key shares. Materializing such an object
            // instead peaked at 2.6-2.9 GB for `.` over a 100 MB object of
            // 7.1 M keys with one repeat, where the plan peaks at the
            // 260-370 MB the same object takes without it (7950X, M4 Pro).
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
                None
            };
            let plan = if sort_keys {
                None
            } else {
                repeat_plan_checked(&fields, collapse, false).map_err(StreamFailure::Decode)?
            };
            out.write_char('{')?;
            let next_indent = current_indent + indent_spaces;
            let mut first = true;
            match (materialized, plan) {
                (Some(items), _) => {
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
                (None, None) => {
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
                (None, Some(plan)) => stream_json_planned_fields(
                    out,
                    fields,
                    plan,
                    next_indent,
                    indent_spaces,
                    unit,
                    numbers,
                    depth,
                )?,
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

/// The fields of an object that repeats a key, for [`stream_json_pretty`]'s
/// object arm: its streaming walk, asking `plan` what becomes of each field
/// (#7311), never under `-S`.
///
/// Out of line and cold, so that the writer every value recurses through
/// keeps the size it had; see `plan_repeats` (`src/jq/document.rs`) for what
/// keeping it in line cost.
#[cold]
#[inline(never)]
#[allow(clippy::too_many_arguments)] // STYLE-0004: stream_json_pretty's own ambients, which this loop threads through.
fn stream_json_planned_fields<W: AsRef<[u64]> + Clone, Out: core::fmt::Write>(
    out: &mut Out,
    fields: JsonFields<'_, W>,
    mut plan: RepeatPlan<JsonCursor<'_, W>>,
    next_indent: usize,
    indent_spaces: usize,
    unit: char,
    numbers: JsonConvention,
    depth: usize,
) -> StreamResult {
    let mut first = true;
    let mut walk = fields;
    let mut at = 0usize;
    while let Some((field, rest)) = JsonFields::uncons(&walk) {
        walk = rest;
        at += 1;
        let (value, value_cursor) = match plan.fate(at - 1) {
            FieldFate::Unchanged => (field.value(), field.value_cursor()),
            FieldFate::Value(cursor) => (cursor.value(), cursor),
            FieldFate::Dropped => continue,
        };
        stream_json_field(
            out,
            field.key(),
            value,
            &value_cursor,
            first,
            next_indent,
            indent_spaces,
            unit,
            false,
            numbers,
            depth,
        )?;
        first = false;
    }
    Ok(())
}

/// Where an object [`stream_scanned_repeats`] prints sits (#7346, #7366): its
/// own text, and the certified text of the value around it.
#[derive(Clone, Copy, Debug)]
struct RepeatText<'s> {
    /// The object's text, which the scan certified member by member.
    span: &'s str,
    /// Where `span` starts in the document.
    start: usize,
    /// The value's text before the object; empty when the object is the value.
    before: &'s str,
    /// The value's text after the object; empty when the object is the value.
    after: &'s str,
    /// How many levels in the object is, 0 when it is the value.
    level: usize,
}

impl<'s> RepeatText<'s> {
    /// An object that is the whole value being printed.
    fn whole(span: &'s str, start: usize) -> Self {
        Self {
            span,
            start,
            before: "",
            after: "",
            level: 0,
        }
    }
}

/// The unsorted `JqCompat` print, compact or indented, of an object the
/// canonical scan certified member by member and declined only because the
/// hashes in `shared` repeat among its keys (#7310,
/// [`CanonicalSpan::RepeatedKeys`]), with the certified text around it when it
/// sits inside the value being printed (#7366,
/// [`stream_around_scanned_repeats`]), all written from `text` (#7346).
///
/// Such an object is well formed, the scan has hashed every key and found
/// which hashes repeat, and since its span is valid UTF-8 every key decodes:
/// everything the object arm's `repeat_plan_checked` takes a key-only walk
/// for. So the plan is built from the scan's list straight away, and from the
/// object's text ([`text_repeat_plan`], #7350): a byte loop over its members,
/// where walking the index over every field cost about 470 instructions a
/// member over the echo.
///
/// Every member is certified, so each run of members the plan leaves alone is
/// already its own compact render, and a planned field is its first
/// occurrence's key and its last occurrence's value, both certified too. The
/// object is cut into those pieces of its span ([`cut_planned_text`]) and
/// written from them ([`write_planned_piece`]), without walking or rendering
/// the members between the plan's steps: as they stand for `-c`, and
/// re-indented otherwise, through [`ChunkedWrite`], which passes a run of a
/// chunk's worth or more on by itself (#7356), so the output is never held
/// whole. The text around the object is written the same way. The cut is
/// checked whole before anything
/// is written, and after that only the writer can fail, so nothing is written
/// that a decline would have to retract (`evaluate_m2_fast_path`'s fall back
/// to the general path relies on that). Walking and re-rendering every member
/// instead took about 3x the time of the same object without the repeat,
/// which is echoed.
///
/// An object the text walk leaves alone is planned by the fields' walk instead
/// ([`repeat_plan_from_shared`], comparing each key under the scan's own hash,
/// [`raw_key_hash`], and recording where each step lies), and written from its
/// text the same way; one whose plan is too dense to keep its spans, or has no
/// plan, is written by that walk ([`stream_planned_object`]), with the text
/// around it, whole into a buffer, as a declined value is.
///
/// Out of line and cold: it is the rare branch of [`JsonCursor::stream_json`],
/// whose echo arms are the common ones.
#[cold]
#[inline(never)]
fn stream_scanned_repeats<W: AsRef<[u64]> + Clone, Out: core::fmt::Write>(
    out: &mut Out,
    fields: JsonFields<'_, W>,
    text: RepeatText<'_>,
    shared: &[u64],
    indent: IndentSpec,
    numbers: JsonConvention,
) -> StreamResult {
    let RepeatText {
        span,
        start,
        before,
        after,
        level,
    } = text;
    let at = |value: &usize| Some(*value);
    let position_of = match text_repeat_plan(span, start, shared) {
        TextPlan::Plan(plan) => {
            if cut_planned_text(&plan, at, span, start, &mut |_| Ok(())).is_some() {
                return write_planned_text(out, &plan, at, text, indent);
            }
            Some(JsonCursor::text_position as PositionOf<_>) // patchcov: coverage tolerate-line reason="unreachable: a text plan's spans come from the text it cuts (#7350); kept so a mismatch takes the fields' walk"
        }
        // The walk would give its spans up at the same field.
        TextPlan::Dense => None,
        // The walk would find no plan either: the object arm writes it.
        TextPlan::NoPlan => {
            let mut buf = String::new();
            write_around(&mut buf, before, 0, indent)?;
            stream_planned_object(&mut buf, fields, None, indent, numbers, level)?;
            write_around(&mut buf, after, level, indent)?;
            return Ok(out.write_str(&buf)?);
        }
        TextPlan::Walk => Some(JsonCursor::text_position as PositionOf<_>),
    };
    let plan = repeat_plan_from_shared(&fields, shared, raw_key_hash, position_of);
    let at = JsonCursor::text_position;
    match plan {
        Some(plan) if cut_planned_text(&plan, at, span, start, &mut |_| Ok(())).is_some() => {
            write_planned_text(out, &plan, at, text, indent)
        }
        plan => {
            let mut buf = String::new();
            write_around(&mut buf, before, 0, indent)?;
            stream_planned_object(&mut buf, fields, plan, indent, numbers, level)?;
            write_around(&mut buf, after, level, indent)?;
            Ok(out.write_str(&buf)?)
        }
    }
}

/// Writes the object [`stream_scanned_repeats`] prints from its text, with the
/// text around it, by `plan`, whose values are found in the text by `at`
/// (#7346, #7350). The caller has cut the same plan from the same text once
/// already, so the cut cannot fail here and only the writer can.
fn write_planned_text<C: Copy, Out: core::fmt::Write>(
    out: &mut Out,
    plan: &RepeatPlan<C>,
    at: impl Fn(&C) -> Option<usize>,
    text: RepeatText<'_>,
    indent: IndentSpec,
) -> StreamResult {
    let RepeatText {
        span,
        start,
        before,
        after,
        level,
    } = text;
    let mut chunked = ChunkedWrite {
        out,
        buf: String::new(),
    };
    let mut first = true;
    write_around(&mut chunked, before, 0, indent)?;
    let written = cut_planned_text(plan, at, span, start, &mut |piece| {
        write_planned_piece(&mut chunked, piece, &mut first, indent, level)
    })
    .map(|written| {
        written
            .and_then(|()| write_around(&mut chunked, after, level, indent))
            .and_then(|()| chunked.finish())
    });
    // The caller cut the same plan from the same text.
    written
        .unwrap_or(Err(core::fmt::Error))
        .map_err(StreamFailure::from)
}

/// The certified text around an object [`stream_scanned_repeats`] prints
/// (#7366): as it stands for `-c`, and otherwise re-indented from `level`, the
/// level it starts at ([`write_reindented_at`]).
fn write_around<Out: core::fmt::Write + ?Sized>(
    out: &mut Out,
    text: &str,
    level: usize,
    indent: IndentSpec,
) -> core::fmt::Result {
    if text.is_empty() {
        Ok(())
    } else if indent.is_compact() {
        out.write_str(text)
    } else {
        write_reindented_at(out, text, indent.width, indent.unit, level)
    }
}

/// An object [`stream_scanned_repeats`] does not write from its text, written
/// by its plan's walk ([`stream_json_planned_fields`], #7310): most of its
/// fields have a step (`StepSpan::DENSE_FROM`), so its text plan gave up its
/// spans, or its text did not cut. With no plan at all, because the
/// shared hashes are a bare collision between different keys, the object arm
/// writes it, which costs that rare object its key walk.
fn stream_planned_object<W: AsRef<[u64]> + Clone, Out: core::fmt::Write>(
    out: &mut Out,
    fields: JsonFields<'_, W>,
    plan: Option<RepeatPlan<JsonCursor<'_, W>>>,
    indent: IndentSpec,
    numbers: JsonConvention,
    level: usize,
) -> StreamResult {
    let Some(plan) = plan else {
        return stream_json_pretty(
            out,
            StandardJson::Object(fields),
            level * indent.width,
            indent.width,
            indent.unit,
            false,
            numbers,
            level,
        );
    };
    out.write_char('{')?;
    stream_json_planned_fields(
        out,
        fields,
        plan,
        (level + 1) * indent.width,
        indent.width,
        indent.unit,
        numbers,
        level,
    )?;
    if indent.width > 0 {
        out.write_char('\n')?;
        write_json_indent(out, level * indent.width, indent.unit)?;
    }
    Ok(out.write_char('}')?)
}
/// The repeat plan of an object the canonical scan certified member by member
/// and declined only because the hashes in `shared` repeat among its keys
/// ([`CanonicalSpan::RepeatedKeys`]), found from its text `span`, which starts
/// at byte `start` of the document (#7350). Each step's value is where that
/// value starts in the document, and each step's span ([`StepSpan`]) the
/// positions of its key, its value and the next field's key, as
/// [`cut_planned_text`] reads them ([`TextPlan::Plan`]). Otherwise it says why
/// there is none: no key repeats, the shared hashes being a bare collision
/// ([`TextPlan::NoPlan`]); the plan is too dense to keep its spans
/// ([`TextPlan::Dense`]); or the members are long ([`TEXT_WALK_MEMBER_BYTES`])
/// or `span` is not the certified object it should be ([`TextPlan::Walk`]),
/// which leaves the object to the fields' walk, recording the same spans from
/// the index (`repeat_plan_from_shared`'s `position_of`).
///
/// The members are cut as the scan certified them ([`certified_value_end`]):
/// a string at its first unescaped quote, found a word at a time
/// ([`word_special_mask`]), a container by [`container_end`], and a number or
/// a literal at its member's `,`. No cut reads past a budget of
/// [`TEXT_WALK_MEMBER_BYTES`] a member, with 64 members' worth to spare, so an
/// object of long members is handed over after at most that much text, however
/// few its members. Each key is hashed between its quotes as the
/// scan hashed it ([`key_hash`] of the raw span, escapes as written), and only
/// a key whose hash is in `shared` is handed to the duplicate-key rule
/// ([`RepeatPlanBuilder`], the one `repeat_plan_checked` builds with), under
/// its raw spelling. Within one certified object that groups keys exactly as
/// their decoded spellings do ([`raw_key_hash`]): the scan certifies a key only
/// when it is spelled the one way jq's writer spells it, so two keys have the
/// same raw span exactly when they decode to the same string.
///
/// Walking the index instead (`uncons_key` and `uncons` over every member,
/// [`repeat_plan_from_shared`]) cost about 470 instructions a member over the
/// echo on the 100 MB `wide` object, whose members are about 14 bytes of text
/// (M4 Pro).
fn text_repeat_plan(span: &str, start: usize, shared: &[u64]) -> TextPlan {
    text_plan_or_walk(span, start, shared).unwrap_or(TextPlan::Walk)
}

/// What [`text_repeat_plan`] made of an object (#7350).
enum TextPlan {
    /// The plan, its spans found in the text.
    Plan(RepeatPlan<usize>),
    /// No key repeats: the shared hashes are a bare collision between
    /// different keys, which the fields' walk would find too (it groups keys
    /// as this walk does), so the object goes to the object arm.
    NoPlan,
    /// More than half the fields have a step ([`StepSpan::dense`]): the fields'
    /// walk would give up its spans at the same field, so it need not record
    /// them.
    Dense,
    /// No plan from the text: the object is left to the fields' walk, which
    /// records its spans.
    Walk,
}

#[cfg(test)]
impl TextPlan {
    /// The plan, when there is one.
    fn plan(self) -> Option<RepeatPlan<usize>> {
        match self {
            Self::Plan(plan) => Some(plan),
            Self::NoPlan | Self::Dense | Self::Walk => None,
        }
    }
}

/// [`text_repeat_plan`], `None` for [`TextPlan::Walk`]: a cut that failed,
/// because `span` is not a certified object or a member ran past the budget.
fn text_plan_or_walk(span: &str, start: usize, shared: &[u64]) -> Option<TextPlan> {
    let bytes = span.as_bytes();
    let close = bytes.len().checked_sub(1)?;
    if bytes.first() != Some(&b'{') || bytes[close] != b'}' || close == 1 {
        return None;
    }
    // The members, without the closing brace, so that no cut runs past it.
    let text = &bytes[..close];
    let mut plan = RepeatPlanBuilder::default();
    let mut spans: Vec<StepSpan> = Vec::new();
    // Whether the last step waits for the position of the next field's key.
    let mut awaiting_next = false;
    let mut at = 0usize;
    // The first byte of the member being cut: its key's opening quote.
    let mut key = 1usize;
    loop {
        // No cut reads past this: members longer than that on average are
        // cheaper walked, and the walk takes the object.
        let budget = (at + 65).saturating_mul(TEXT_WALK_MEMBER_BYTES);
        let window = &text[..text.len().min(budget)];
        let key_end = certified_string_end(window, key)?;
        if window.get(key_end) != Some(&b':') {
            return None;
        }
        let (name_start, name_end) = (key + 1, key_end - 1);
        if core::mem::take(&mut awaiting_next) {
            // Set only once a step has been pushed.
            spans.last_mut()?.next = start + key;
        }
        let value = key_end + 1;
        let value_end = certified_value_end(window, value)?;
        // A number or literal that ran to the end of the window may run on.
        if value_end == window.len() && window.len() < text.len() {
            return None;
        }
        let hash = key_hash(&text[name_start..name_end]);
        if shared.binary_search(&hash).is_ok() {
            plan.field_hashed(
                at,
                hash,
                Cow::Borrowed(span.get(name_start..name_end)?),
                start + value,
            );
            spans.push(StepSpan {
                key: start + key,
                value: start + value,
                next: StepSpan::LAST,
            });
            awaiting_next = true;
            if StepSpan::dense(spans.len(), at + 1, false) {
                return Some(TextPlan::Dense);
            }
        }
        at += 1;
        match bytes[value_end] {
            b',' => key = value_end + 1,
            _ if value_end == close => break,
            _ => return None,
        }
    }
    if StepSpan::dense(spans.len(), at, true) {
        return Some(TextPlan::Dense);
    }
    Some(plan.finish(spans).map_or(TextPlan::NoPlan, TextPlan::Plan))
}

/// One past the closing quote of the certified string whose opening quote is
/// `text[at]` ([`text_repeat_plan`]): a certified string's only escapes are
/// pairs of a backslash and one byte, so its first unescaped quote ends it,
/// the rule [`write_reindented_at`] cuts by. `None` when `text[at]` is not a
/// quote or the string does not close in `text`.
///
/// The strings this sees are short, since an object of long members is left to
/// the fields' walk ([`TEXT_WALK_MEMBER_BYTES`]), so it searches a word at a
/// time rather than through the scan's block mask ([`scan_json_string_span`]),
/// which read 25 M more instructions than a byte-at-a-time search on the 10 MB
/// repeat row (7950X).
fn certified_string_end(text: &[u8], at: usize) -> Option<usize> {
    if text.get(at) != Some(&b'"') {
        return None;
    }
    let mut i = at + 1;
    loop {
        i = string_special(text, i)?;
        match text[i] {
            b'"' => return Some(i + 1),
            b'\\' => i += 2,
            // A raw control byte, which no certified string holds.
            _ => return None,
        }
    }
}

/// The first `"`, backslash or control byte in `text` at or after `from`,
/// eight bytes at a time by [`word_special_mask`], the scan's own word test
/// ([`certified_string_end`]). A byte at a time cost about 7 instructions a
/// byte, which made the text walk lose to the fields' walk over 256-byte
/// strings (+17% to +29%, both chips).
fn string_special(text: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while let Some(chunk) = text.get(i..i + 8) {
        let word = u64::from_le_bytes(<[u8; 8]>::try_from(chunk).ok()?);
        let hit = word_special_mask(word);
        if hit != 0 {
            return Some(i + (hit.trailing_zeros() / 8) as usize);
        }
        i += 8;
    }
    let rest = text.get(i..)?;
    rest.iter()
        .position(|&b| b == b'"' || b == b'\\' || b < 0x20)
        .map(|at| i + at)
}

/// Where the certified value that starts at `text[at]` ends, `text` being an
/// object's members without its closing `}` ([`text_repeat_plan`]): one past
/// its last byte, which is its member's `,` or the end of `text`. A string is
/// cut by [`certified_string_end`], a container by [`container_end`], and a
/// number or a literal runs to its member's `,`. `None` when the value is
/// empty, does not close in `text`, or a number or literal runs into a byte no
/// member's value holds there.
fn certified_value_end(text: &[u8], at: usize) -> Option<usize> {
    match *text.get(at)? {
        b'"' => certified_string_end(text, at),
        b'{' | b'[' => container_end(text, at),
        _ => {
            let rest = text.get(at..)?;
            match rest
                .iter()
                .position(|&b| matches!(b, b',' | b':' | b'"' | b'[' | b']' | b'{' | b'}'))
            {
                Some(0) => None,
                Some(len) => (rest[len] == b',').then_some(at + len),
                None => Some(text.len()),
            }
        }
    }
}

/// The member length past which [`text_repeat_plan`] leaves an object to the
/// fields' walk: no cut reads past this many bytes a member, with 64 members'
/// worth to spare, so a few huge members are handed over as soon as many long
/// ones are, and never read whole. The text walk reads
/// every byte of a member, about 1.5 instructions a byte for a string
/// ([`string_special`]), and the fields' walk costs about 470 a member
/// whatever its length, so long members are cheaper walked: over members of
/// about 270 bytes (256-byte strings) the text walk read 6% more instructions
/// than the walk (7950X, 512 bytes here), and over 1 KB and 2 KB strings it was
/// 2% to 5% slower (M4 Pro, #7350).
const TEXT_WALK_MEMBER_BYTES: usize = 256;

/// One piece of a planned object's text (#7346, [`cut_planned_text`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlannedPiece<'s> {
    /// The object's opening brace.
    Open,
    /// Members written as they stand: their text, with the commas between
    /// them and none at either end.
    Run(&'s str),
    /// A repeated key's first occurrence, `key` (its quoted text), written with
    /// its last occurrence's `value` (that value's text).
    Field { key: &'s str, value: &'s str },
    /// The object's closing brace.
    Close,
}

/// Cuts an object's certified text `span`, which starts at byte `start` of the
/// document, into the pieces its plan writes (#7346), handing each to `emit`
/// in order: the runs between the plan's steps, and each step's field unless
/// it is dropped. `None`, before `emit` sees anything past where it went
/// wrong, when the plan has no spans or a span is not where a member of
/// `span` would be; a caller that must not write part of an object cuts once
/// with an `emit` that writes nothing first. `Some` with `emit`'s first error
/// otherwise.
///
/// Each step's span gives three positions ([`StepSpan`]), so a field's end is
/// the comma before the next key or the closing brace, with no value read to
/// find it, and a planned value's text runs from its own position to the end
/// of the field it came from, a later step's, found by its position.
fn cut_planned_text<'s, C: Copy>(
    plan: &RepeatPlan<C>,
    value_at: impl Fn(&C) -> Option<usize>,
    span: &'s str,
    start: usize,
    emit: &mut dyn FnMut(PlannedPiece<'s>) -> core::fmt::Result,
) -> Option<core::fmt::Result> {
    let bytes = span.as_bytes();
    let close = span.len().checked_sub(1)?;
    if bytes.first() != Some(&b'{') || bytes.get(close) != Some(&b'}') {
        return None;
    }
    let local = |at: usize| at.checked_sub(start).filter(|&at| at < close);
    // Where the field of `step` ends: the comma before the next key, or the
    // closing brace.
    let field_end = |step: &StepSpan| -> Option<usize> {
        if step.next == StepSpan::LAST {
            return Some(close);
        }
        let end = local(step.next)?.checked_sub(1)?;
        (bytes.get(end) == Some(&b',')).then_some(end)
    };
    let spans = plan.spans();
    let pieces = |emit: &mut dyn FnMut(PlannedPiece<'s>) -> core::fmt::Result| {
        emit(PlannedPiece::Open).ok()?;
        // The first byte of the next member not yet cut.
        let mut next = 1usize;
        for (step, value) in plan.spanned_steps()? {
            let field_start = local(step.key)?;
            if field_start < next {
                return None; // patchcov: coverage tolerate-line reason="defensive: a plan's spans come from the same certified text it cuts, so a field never starts before the cut and always has its colon; kept so a mismatch declines rather than slicing wrong (#7346)"
            }
            if field_start > next {
                // The run ends at the comma before this field.
                if bytes.get(field_start - 1) != Some(&b',') {
                    return None;
                }
                emit(PlannedPiece::Run(span.get(next..field_start - 1)?)).ok()?;
            }
            if let Some(value) = value {
                let colon = local(step.value)?.checked_sub(1)?;
                if colon <= field_start || bytes.get(colon) != Some(&b':') {
                    return None; // patchcov: coverage tolerate-line reason="defensive: a plan's spans come from the same certified text it cuts, so a field never starts before the cut and always has its colon; kept so a mismatch declines rather than slicing wrong (#7346)"
                }
                let at = value_at(&value)?;
                let from = spans.binary_search_by_key(&at, |span| span.value).ok()?;
                let value = span.get(local(at)?..field_end(&spans[from])?)?;
                emit(PlannedPiece::Field {
                    key: span.get(field_start..colon)?,
                    value,
                })
                .ok()?;
            }
            next = match field_end(&step)? {
                end if end == close => close,
                end => end + 1,
            };
        }
        if next < close {
            emit(PlannedPiece::Run(span.get(next..close)?)).ok()?;
        }
        emit(PlannedPiece::Close).ok()?;
        Some(())
    };
    // `emit`'s error stops the cut where it happened; `pieces` answers `None`
    // for it as for a misplaced span, so it is kept apart here.
    let mut failed = Ok(());
    let cut = pieces(&mut |piece| {
        let written = emit(piece);
        if written.is_err() {
            failed = written;
        }
        written
    });
    match (cut, failed) {
        (_, Err(e)) => Some(Err(e)),
        (Some(()), Ok(())) => Some(Ok(())),
        (None, Ok(())) => None,
    }
}

/// Writes one piece of a planned object ([`cut_planned_text`]) as jq prints
/// it, the object being `level` levels in: compact when `indent` is, otherwise
/// with its members indented one level further and its `}` at `level`
/// (#7346). `first` says no member has been written yet. Each piece's text is
/// certified, so a run is already its compact render and is re-indented as
/// [`write_reindented`] re-indents a whole span.
fn write_planned_piece<Out: core::fmt::Write + ?Sized>(
    out: &mut Out,
    piece: PlannedPiece<'_>,
    first: &mut bool,
    indent: IndentSpec,
    level: usize,
) -> core::fmt::Result {
    let text = match piece {
        PlannedPiece::Open => return out.write_char('{'),
        PlannedPiece::Close => {
            if !indent.is_compact() {
                out.write_char('\n')?;
                write_json_indent(out, level * indent.width, indent.unit)?;
            }
            return out.write_char('}');
        }
        PlannedPiece::Run(run) => {
            member_line(out, first, indent, level)?;
            run
        }
        PlannedPiece::Field { key, value } => {
            member_line(out, first, indent, level)?;
            out.write_str(key)?;
            out.write_str(if indent.is_compact() { ":" } else { ": " })?;
            value
        }
    };
    if indent.is_compact() {
        out.write_str(text)
    } else {
        write_reindented_at(out, text, indent.width, indent.unit, level + 1)
    }
}

/// The separator and, when indented, the line a member of a planned object
/// `level` levels in starts with ([`write_planned_piece`]).
fn member_line<Out: core::fmt::Write + ?Sized>(
    out: &mut Out,
    first: &mut bool,
    indent: IndentSpec,
    level: usize,
) -> core::fmt::Result {
    if !core::mem::take(first) {
        out.write_char(',')?;
    }
    if !indent.is_compact() {
        out.write_char('\n')?;
        write_json_indent(out, (level + 1) * indent.width, indent.unit)?;
    }
    Ok(())
}

/// The unsorted `JqCompat` print, compact or indented, of a value the
/// canonical scan certified but for one object inside it, which it declined
/// only because some of its keys' hashes repeat (#7366,
/// [`CanonicalSpan::NestedRepeatedKeys`]): `span`, the text of the value at
/// `cursor`.
///
/// The text before the object and the text after it are already their compact
/// render, as a certified span's is, so they are written as they stand (or
/// re-indented, [`write_reindented_at`]), and the object is written from its
/// text and its plan, as a value that is such an object is
/// ([`stream_scanned_repeats`], #7346). Before #7366, the scan stopped at that
/// object, and every member of every value around it was walked and
/// re-rendered.
///
/// The object is found in the index by where it starts
/// ([`JsonCursor::cursor_at_offset`]). A cursor that is not at that `{`, which
/// a certified span should never give, has the value printed by
/// [`stream_json_pretty`], whole into a buffer, as a declined value is.
///
/// Out of line and cold: it is the rare branch of [`JsonCursor::stream_json`],
/// whose echo arms are the common ones.
#[cold]
#[inline(never)]
fn stream_around_scanned_repeats<W: AsRef<[u64]> + Clone, Out: core::fmt::Write>(
    out: &mut Out,
    cursor: &JsonCursor<'_, W>,
    value: StandardJson<'_, W>,
    span: &str,
    object: &NestedRepeat,
    indent: IndentSpec,
    numbers: JsonConvention,
) -> StreamResult {
    let found = cursor
        .text_position()
        .and_then(|value_start| value_start.checked_add(object.start))
        .and_then(|at| {
            let found = cursor.cursor_at_offset(at)?;
            (found.text_position() == Some(at)).then_some((at, found))
        })
        .and_then(|(at, found)| match found.value() {
            StandardJson::Object(fields) => Some((at, fields)),
            _ => None,
        });
    let (Some((start, fields)), Some(before), Some(object_span), Some(after)) = (
        found,
        span.get(..object.start),
        span.get(object.start..object.end),
        span.get(object.end..),
    ) else {
        debug_assert!(false, "a certified span's recorded object is in the index"); // patchcov: coverage tolerate-line reason="unreachable: a certified span's recorded object is in the index (#7366); kept so a mismatch prints the value whole, as a declined value is (#7346)"
        let mut buf = String::new(); // patchcov: coverage tolerate-line reason="unreachable: a certified span's recorded object is in the index (#7366); kept so a mismatch prints the value whole, as a declined value is (#7346)"
        #[rustfmt::skip]
        stream_json_pretty(&mut buf, value, 0, indent.width, indent.unit, false, numbers, 0)?; // patchcov: coverage tolerate-line reason="unreachable: a certified span's recorded object is in the index (#7366); kept so a mismatch prints the value whole, as a declined value is (#7346)"
        return Ok(out.write_str(&buf)?); // patchcov: coverage tolerate-line reason="unreachable: a certified span's recorded object is in the index (#7366); kept so a mismatch prints the value whole, as a declined value is (#7346)"
    };
    let text = RepeatText {
        span: object_span,
        start,
        before,
        after,
        level: object.level,
    };
    stream_scanned_repeats(out, fields, text, &object.shared, indent, numbers)
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
            match OwnedValue::from_json_number::<JqSemantics>(&n) {
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
                // A NaN spelling (`NaN`, `-nan`) is a parse instance (#3309),
                // whose literal text is not a number spelling: jq prints `null`.
                OwnedValue::NumberLiteral(NumberRepr::Float(f), _) if f.is_nan() => {
                    out.write_str("null")
                }
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
///
/// Written as slices of a run of the unit rather than a character at a time
/// (#7271 review): a writer with work per call, such as the chunked writer of a
/// streamed pretty render, otherwise pays it once per byte of indentation,
/// which on a document 250 levels deep is most of the output.
fn write_json_indent<Out: core::fmt::Write + ?Sized>(
    out: &mut Out,
    spaces: usize,
    unit: char,
) -> core::fmt::Result {
    const SPACES: &str = "                                                                ";
    const TABS: &str = "\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t";
    let run = match unit {
        ' ' => SPACES,
        '\t' => TABS,
        _ => {
            for _ in 0..spaces {
                out.write_char(unit)?;
            }
            return Ok(());
        }
    };
    let mut left = spaces;
    while left > 0 {
        let take = left.min(run.len());
        out.write_str(&run[..take])?;
        left -= take;
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
                    Ok(out.write_str(crate::jq::nonfinite_yaml_spelling(f))?)
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

    // ------------------------------------------------------------------
    // #5650: the byte a sibling ends on, and JSON whitespace only.
    // ------------------------------------------------------------------

    /// `preceding_gap_ok(text, at, Some(b','))` for the `2` of `"<head>,2"`.
    fn comma_gap_ok(head: &[u8]) -> bool {
        // STYLE-0013: a unit test of the shared check itself; there is no field to route through.
        let mut text = head.to_vec();
        text.extend_from_slice(b",2");
        preceding_gap_ok(&text, head.len() + 1, Some(b','))
    }

    #[test]
    fn test_a_sibling_must_end_on_a_byte_a_value_can_end_on_5650() {
        for ok in [
            &b"[1"[..],
            b"[1.",
            b"[.5",
            b"[-0",
            b"[1e5",
            b"[1.5E+3",
            b"[\"a\"",
            b"[[1]",
            b"[{}",
            b"[true",
            b"[false",
            b"[null",
            b"[nan",
            b"[-nan",
            b"[Nan0",
            b"[Infinity",
            b"[-Infinity",
            b"[1 ",
            b"[1\n\t\r ",
        ] {
            assert!(comma_gap_ok(ok), "{:?}", String::from_utf8_lossy(ok));
        }
        for bad in [
            &b"[1@"[..],
            b"[\"a\"@",
            b"[true@",
            b"[[1]@",
            b"[1\x0c",
            b"[\"a\"\x0c",
            b"[1\x00",
            b"[1null",
            b"[1true",
            b"[1false",
            b"[1e",
            b"[-1x",
            b"[.5x",
            b"[1e+",
            b"[1-",
        ] {
            assert!(!comma_gap_ok(bad), "{:?}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn test_a_digit_ending_glued_token_is_left_to_the_decode_5650() {
        // The trade `sibling_end_ok` documents: one byte test per number, so `0x1` is not seen here.
        assert!(comma_gap_ok(b"[0x1"));
        assert!(comma_gap_ok(b"[1_000"));
    }

    #[test]
    fn test_a_form_feed_is_not_whitespace_in_a_trailing_gap_5650() {
        assert!(trailing_gap_ok(b"[1 \n\t\r]", 2, b']'));
        assert!(!trailing_gap_ok(b"[1\x0c]", 2, b']'));
        assert!(!trailing_gap_ok(b"[1@]", 2, b']'));
    }

    /// #5654: the end of a container's last child, which the trailing-gap check starts from, for
    /// each kind of child.
    #[test]
    fn test_last_child_text_end_covers_scalars_containers_and_empties_5654() {
        use crate::json::JsonIndex;
        let text = br#"[1, "ab", [2, [3]], {"k": []}, {}, []]"#;
        let index = JsonIndex::build(text);
        let root = index.root(text);
        let StandardJson::Array(elements) = root.value() else {
            panic!("an array");
        };
        let ends: Vec<Option<usize>> = elements
            .cursor_iter()
            .map(|c| c.text_position().and_then(|s| c.last_child_text_end(s)))
            .collect();
        let end_of = |needle: &[u8]| {
            let at = text
                .windows(needle.len())
                .position(|w| w == needle)
                .expect("present");
            Some(at + needle.len())
        };
        assert_eq!(
            ends,
            [
                Some(2),
                end_of(b"\"ab\""),
                end_of(b"[2, [3]]"),
                end_of(b"{\"k\": []}"),
                end_of(b"{}"),
                Some(text.len() - 1),
            ]
        );
    }

    /// #5683: `plain_int_at` reads the integers the general decode reads as the same number, and
    /// declines every spelling that is not one.
    #[test]
    fn test_plain_int_at_reads_only_plain_integers_5683() {
        use crate::jq::document::DocumentCursor;
        use crate::json::JsonIndex;
        let read = |text: &str| -> Option<i64> {
            let doc = alloc::format!("[{text}]");
            let index = JsonIndex::build(doc.as_bytes());
            let root = index.root(doc.as_bytes());
            let first = root.first_child()?;
            let at = first.text_position()?;
            DocumentCursor::plain_int_at(&first, at)
        };
        for (text, want) in [
            ("0", Some(0)),
            ("7", Some(7)),
            ("-7", Some(-7)),
            ("123456789012345678", Some(123_456_789_012_345_678)),
            ("-123456789012345678", Some(-123_456_789_012_345_678)),
            (" 42 ", Some(42)),
            // declined: left to the general decode
            ("-0", None),
            ("00", None),
            ("007", None),
            ("-07", None),
            ("1234567890123456789", None),
            ("1.0", None),
            ("1.5", None),
            ("1e3", None),
            ("1E3", None),
            ("12x", None),
            ("+1", None),
            ("-", None),
            ("\"1\"", None),
            ("true", None),
        ] {
            assert_eq!(read(text), want, "{text:?}");
        }
        // The last element ends at `]`, a comma-less token end.
        let doc = b"[1,22]";
        let index = JsonIndex::build(doc);
        let last = index
            .root(doc)
            .first_child()
            .unwrap()
            .next_sibling()
            .unwrap();
        let at = last.text_position().unwrap();
        assert_eq!(DocumentCursor::plain_int_at(&last, at), Some(22));
    }

    /// #5683: wherever `plain_int_at` answers, the general decode of the same token is that
    /// integer, spelled the same.
    #[test]
    fn test_plain_int_at_agrees_with_the_general_decode_5683() {
        use crate::jq::document::DocumentCursor;
        use crate::jq::JqSemantics;
        use crate::jq::OwnedValue;
        use crate::json::JsonIndex;
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let digits = b"0123456789";
        let mut answered = 0;
        for _ in 0..5_000 {
            let mut token = String::new();
            if next() % 3 == 0 {
                token.push('-');
            }
            for _ in 0..(next() % 21) {
                token.push(digits[(next() % 10) as usize] as char);
            }
            match next() % 8 {
                0 => token.push_str(".5"),
                1 => token.push_str("e2"),
                2 => token.push('x'),
                _ => {}
            }
            if token.trim_start_matches('-').is_empty() {
                continue;
            }
            let doc = alloc::format!("[{token},0]");
            let index = JsonIndex::build(doc.as_bytes());
            let root = index.root(doc.as_bytes());
            let Some(first) = root.first_child() else {
                continue;
            };
            let Some(at) = first.text_position() else {
                continue;
            };
            if let Some(value) = DocumentCursor::plain_int_at(&first, at) {
                answered += 1;
                let general = OwnedValue::from_number_literal::<JqSemantics>(&token);
                assert_eq!(general.to_json(), value.to_string(), "{token:?}");
            }
        }
        assert!(
            answered > 500,
            "the sweep must reach the reader: {answered}"
        );
    }

    /// Every container's cursor in `text`, in document order.
    fn container_cursors<'a>(index: &'a JsonIndex, text: &'a [u8]) -> Vec<JsonCursor<'a>> {
        let mut out = Vec::new();
        let mut stack = vec![index.root(text)];
        while let Some(c) = stack.pop() {
            if text
                .get(c.text_position().unwrap_or(usize::MAX))
                .is_some_and(|b| matches!(b, b'[' | b'{'))
            {
                out.push(c);
            }
            let mut kids = Vec::new();
            let mut child = c.first_child();
            while let Some(k) = child {
                kids.push(k);
                child = k.next_sibling();
            }
            stack.extend(kids.into_iter().rev());
        }
        out
    }

    /// #5869: `container_end_bp` answers the same from the closer-run memo as from a scan, in
    /// whatever order the containers are asked. A fresh index has no memo, so its answer is the
    /// scan's; the memoed index is asked in document order, in reverse, and interleaved from both
    /// ends, over spines, combs, empty and scalar leaves, junk between closers and a `]` inside
    /// a string.
    #[test]
    fn test_container_end_bp_memo_agrees_with_the_scan_in_any_order_5869() {
        let nest = |n: usize, leaf: &str| format!("{}{leaf}{}", "[".repeat(n), "]".repeat(n));
        let docs = [
            nest(40, "1"),
            nest(40, "[]"),
            nest(40, "{}"),
            nest(150, r#""a]b""#),
            // Closers separated by whitespace and by junk the scan skips.
            "[ [ [ 1 ] ] , [ [ 2 ] @ ] ]".to_string(),
            "[[1@]]".to_string(),
            // A comb: each level has a sibling spine of its own.
            format!(
                "[{}]",
                (0..30)
                    .map(|i| nest(i % 7 + 1, "true"))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            format!(
                r#"{{"a":{},"b":{},"c":[{},{}]}}"#,
                nest(70, "1"),
                nest(3, "[]"),
                nest(5, "null"),
                nest(80, "{}")
            ),
            // Spines under objects, which end in `}`.
            format!("{}1{}", r#"{"k":"#.repeat(90), "}".repeat(90)),
            "[[],[[]],[[],[]]]".to_string(),
        ];
        for doc in &docs {
            let text = doc.as_bytes();
            let n = container_cursors(&JsonIndex::build(text), text).len();
            let expected: Vec<Option<usize>> = (0..n)
                .map(|i| {
                    let fresh = JsonIndex::build(text);
                    container_cursors(&fresh, text)[i].container_end_bp()
                })
                .collect();
            let orders: [Vec<usize>; 4] = [
                (0..n).collect(),
                (0..n).rev().collect(),
                (0..n).flat_map(|i| [i, n - 1 - i]).collect(),
                (0..n).map(|i| (i * 7 + 3) % n).collect(),
            ];
            for order in orders {
                let index = JsonIndex::build(text);
                let cursors = container_cursors(&index, text);
                for i in order {
                    assert_eq!(cursors[i].container_end_bp(), expected[i], "{doc} @ {i}");
                    // Asked twice in a row: the second is answered from the memo it just wrote.
                    assert_eq!(
                        cursors[i].container_end_bp(),
                        expected[i],
                        "{doc} @ {i} again"
                    );
                }
            }
        }
    }

    /// #5869: a walk down a linear nest scans its closers once, not once per level. The count is
    /// of scans, not time, so it fails if the per-level rescan returns, at a depth a debug build
    /// walks in milliseconds.
    #[test]
    fn test_container_end_bp_scans_a_spine_once_5869() {
        const DEPTH: usize = 20_000;
        for leaf in ["1", "[]", r#""x]""#] {
            let doc = format!("{}{leaf}{}", "[".repeat(DEPTH), "]".repeat(DEPTH));
            let text = doc.as_bytes();
            let index = JsonIndex::build(text);
            let cursors = container_cursors(&index, text);
            CLOSE_RUN_SCANS.with(|n| n.set(0));
            for c in &cursors {
                assert!(c.container_end_bp().is_some());
            }
            let scans = CLOSE_RUN_SCANS.with(Cell::get);
            assert!(
                scans <= 2,
                "{leaf}: {scans} scans for {} containers",
                cursors.len()
            );
        }
    }

    /// #5869: the memo holds a bounded number of runs, keeping the newest.
    #[test]
    fn test_close_runs_are_bounded_and_keep_the_newest_5869() {
        let index = JsonIndex::build(b"[]");
        for leaf in 0..CLOSE_RUN_CAPACITY * 3 {
            index.record_close_run(CloseRun {
                leaf,
                first: leaf + 1,
                hi: leaf + 1,
                at: leaf + 1,
                end: 0,
            });
            let runs = index.close_runs.borrow();
            assert!(runs.len() <= CLOSE_RUN_CAPACITY);
            assert_eq!(runs.last().map(|r| r.leaf), Some(leaf));
        }
        assert_eq!(index.close_runs.borrow().len(), CLOSE_RUN_CAPACITY);
    }

    /// #5654: `last_open_before` finds the last 1 bit below `end`, across word boundaries, at a
    /// full-word mask, and after a run of more than a word of zeros (a run of closers).
    #[test]
    fn test_last_open_before_crosses_words_and_masks_5654() {
        // Bit `p` is `words[p / 64] >> (p % 64)`.
        let words = [0b1u64, 0, 0, 1u64 << 63, 0b10];
        assert_eq!(last_open_before(&words, 0), None);
        assert_eq!(last_open_before(&words, 1), Some(0));
        // Strictly before `end`: bit 0 is not before 0, but is before 1 and every later end.
        assert_eq!(last_open_before(&words, 2), Some(0));
        // A run of 191 zeros (words 1 and 2 and the low part of 0) back to bit 0.
        assert_eq!(last_open_before(&words, 3 * 64), Some(0));
        // The top bit of word 3 is position 255; an end at 256 keeps the whole word (`keep == 64`).
        assert_eq!(last_open_before(&words, 256), Some(255));
        assert_eq!(last_open_before(&words, 255), Some(0));
        // Bit 1 of word 4 is position 257.
        assert_eq!(last_open_before(&words, 258), Some(257));
        assert_eq!(last_open_before(&words, 257), Some(255));
        // An end past the stored words finds nothing rather than indexing out of range.
        assert_eq!(last_open_before(&words, 64 * 6), None);
        assert_eq!(last_open_before(&[0u64; 3], 130), None);
    }

    // ------------------------------------------------------------------
    // #3340: the string arm walks `SpecialMask` blocks instead of bytes.
    // ------------------------------------------------------------------

    /// The byte-at-a-time string scan this module used before #3340, kept
    /// verbatim as an *independent* oracle: it shares no code with the block
    /// classifier or `canonical_escape_len`, so a divergence in either shows
    /// up as a disagreement here instead of being copied into both sides.
    fn scan_json_string_span_bytewise_reference(
        bytes: &[u8],
        pos: usize,
    ) -> Option<(usize, usize, usize)> {
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
                // Every other byte -- ASCII, or any byte of a multi-byte UTF-8
                // sequence -- is one byte of literal content. UTF-8 validity is
                // deliberately *not* decided here; see this function's own doc
                // comment.
                _ => i += 1,
            }
        }
    }

    /// Builds one string literal around `body` and checks the block-walking
    /// scan against the byte-loop oracle at every start offset it can sit at
    /// inside a 64-byte block (the padding shifts the opening quote across
    /// every lane and across the block boundary).
    fn assert_string_scan_agrees(body: &[u8]) {
        for pad in 0..130usize {
            let mut doc = vec![b'['; 0];
            doc.extend(core::iter::repeat(b'1').take(pad));
            let pos = doc.len();
            doc.push(b'"');
            doc.extend_from_slice(body);
            doc.push(b'"');
            doc.extend_from_slice(b",1]");
            let expect = scan_json_string_span_bytewise_reference(&doc, pos);
            let got = scan_json_string_span(&doc, pos, &mut SpecialMask::new());
            assert_eq!(got, expect, "pad {pad} body {body:?}");
            // And with no closing quote at all: an unterminated string bails.
            let open = &doc[..doc.len() - 3];
            assert_eq!(
                scan_json_string_span(&open[..open.len() - 1], pos, &mut SpecialMask::new()),
                scan_json_string_span_bytewise_reference(&open[..open.len() - 1], pos),
                "unterminated, pad {pad} body {body:?}"
            );
        }
    }

    /// Plain strings of every length across and past two block boundaries, and
    /// the same with one special of each kind (and one multi-byte character)
    /// planted at every offset: the closing quote, a `\`-escape, a `\u` escape
    /// and a control byte each land on every lane and straddle the boundary at
    /// some offset.
    #[test]
    fn string_scan_agrees_with_the_bytewise_reference_across_block_boundaries_3340() {
        for len in 0..140usize {
            let plain = vec![b'a'; len];
            assert_string_scan_agrees(&plain);
        }
        let units: [&[u8]; 9] = [
            b"\\n",
            b"\\\"",
            b"\\\\",
            b"\\u0001",
            b"\\u007f",
            b"\\u001F",
            b"\\/",
            b"\x01",
            "\u{e9}\u{4e16}".as_bytes(),
        ];
        for unit in units {
            for at in (0..70usize).chain(120..134) {
                let mut body = vec![b'a'; at + 8];
                body.splice(at..at, unit.iter().copied());
                assert_string_scan_agrees(&body);
            }
        }
    }

    /// One mask carried across a whole document: many strings and keys, so
    /// the cached block is reused, left behind and re-based dozens of times at
    /// every offset. The expected answer is built from the byte-loop oracle one
    /// string at a time (a document of `n` strings is canonical iff each
    /// string is), so it shares nothing with the cache under test. The
    /// alphabet only carries valid UTF-8 and no bare quote, so a body's
    /// canonicality is the only thing that can fail the gate.
    #[test]
    fn gate_agrees_with_per_string_oracle_across_a_shared_mask_3340() {
        let alphabet: [&[u8]; 12] = [
            b"a",
            b"bc",
            b" ",
            b"\\n",
            b"\\\"",
            b"\\\\",
            b"\\u0000",
            b"\\u007f",
            b"\\u00e9",
            b"\\x",
            b"\x01",
            "\u{e9}".as_bytes(),
        ];
        let mut seed = 0x0123_4567_89AB_CDEFu64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for round in 0..3000 {
            let count = 1 + (next() % 40) as usize;
            let mut bodies: Vec<Vec<u8>> = Vec::new();
            for _ in 0..count {
                let mut body = Vec::new();
                // Mostly short, so a 64-byte block holds several strings.
                let n = if next() % 4 == 0 {
                    next() % 30
                } else {
                    next() % 6
                };
                for _ in 0..n {
                    body.extend_from_slice(alphabet[(next() >> 20) as usize % alphabet.len()]);
                }
                bodies.push(body);
            }
            let canonical = bodies.iter().all(|body| {
                let mut lit = vec![b'"'];
                lit.extend_from_slice(body);
                lit.push(b'"');
                scan_json_string_span_bytewise_reference(&lit, 0)
                    == Some((1, lit.len() - 1, lit.len()))
            });
            // Array of strings, then an object whose (unique) keys and values are
            // both strings, so key and value scans share the cache.
            let mut array = vec![b'['];
            let mut object = vec![b'{'];
            for (k, body) in bodies.iter().enumerate() {
                if k > 0 {
                    array.push(b',');
                    object.push(b',');
                }
                array.push(b'"');
                array.extend_from_slice(body);
                array.push(b'"');
                object.extend_from_slice(format!("\"k{k}\":\"").as_bytes());
                object.extend_from_slice(body);
                object.push(b'"');
            }
            array.push(b']');
            object.push(b'}');
            for (shape, doc) in [("array", &array), ("object", &object)] {
                let expect = canonical.then_some(doc.len());
                assert_eq!(
                    canonical_compact_jq_span_end(doc),
                    expect,
                    "round {round} {shape}: {:?}",
                    String::from_utf8_lossy(doc)
                );
                // The same document padded in front, shifting every string
                // across a different set of block boundaries.
                let pad = (next() % 70) as usize;
                let mut padded = vec![b'['];
                padded.extend(core::iter::repeat(b'1').take(pad));
                padded.push(b',');
                let value_start = padded.len();
                padded.extend_from_slice(doc);
                padded.push(b']');
                let got = canonical_compact_jq_span_end(&padded[value_start..]);
                assert_eq!(got, expect, "round {round} {shape} pad {pad}");
            }
        }
    }

    /// Escape-dense text -- the shape #3168 lost on -- driven by a seeded
    /// generator rather than a hand-picked list.
    #[test]
    fn string_scan_agrees_with_the_bytewise_reference_on_random_strings_3340() {
        let alphabet: [&[u8]; 14] = [
            b"a",
            b"b",
            b" ",
            b"\\n",
            b"\\t",
            b"\\\"",
            b"\\\\",
            b"\\u0000",
            b"\\u001f",
            b"\\u00e9",
            b"\\x",
            b"\x7f",
            b"\x1f",
            "\u{1f600}".as_bytes(),
        ];
        let mut seed = 0xDEAD_BEEF_CAFE_F00Du64;
        for _ in 0..4000 {
            let mut body = Vec::new();
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let n = (seed % 90) as usize;
            for _ in 0..n {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                body.extend_from_slice(alphabet[(seed >> 20) as usize % alphabet.len()]);
            }
            assert_string_scan_agrees(&body);
        }
    }

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
        assert!(
            DocumentCursor::head_comment_raw(&root).is_empty(),
            "{:?}",
            DocumentCursor::head_comment_raw(&root)
        );
        assert!(
            DocumentCursor::foot_comment_raw(&root).is_empty(),
            "{:?}",
            DocumentCursor::foot_comment_raw(&root)
        );
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

    /// #7425: the length every index holds, at its boundary. A longer text is held when its
    /// counts are within `MAX_COUNT` (#7537).
    #[test]
    #[cfg(target_pointer_width = "64")]
    fn can_index_holds_up_to_u32_max_bytes_7425() {
        assert_eq!(JsonIndex::MAX_TEXT_LEN, 4_294_967_295);
        assert!(JsonIndex::can_index(0));
        assert!(JsonIndex::can_index(u32::MAX as usize));
        assert!(!JsonIndex::can_index(u32::MAX as usize + 1));
        assert!(!JsonIndex::can_index(usize::MAX));
    }

    /// #7537: the ceiling is the count of interest bits, not the length, so a length past
    /// `u32::MAX` with few set bits is held. `ib_len` is a plain parameter, so this needs no 4 GiB
    /// allocation.
    #[test]
    #[cfg(target_pointer_width = "64")]
    fn from_parts_holds_a_length_past_u32_max_with_few_interest_bits_7537() {
        let index = JsonIndex::from_parts(vec![1u64], u32::MAX as usize + 1, vec![0b01], 2);
        assert_eq!(index.ib_len(), u32::MAX as usize + 1);
        assert_eq!(index.ib_select1(0), Some(0));
        assert_eq!(index.ib_select1_sequential(0), Some(0));
    }

    /// #7537: the count ceiling at its boundary, `u32::MAX` set bits held and one more refused.
    /// 512 MiB of words, so it runs with `large-tests` only.
    #[test]
    #[cfg(all(feature = "large-tests", target_pointer_width = "64"))]
    fn the_interest_bit_count_ceiling_is_u32_max_7537() {
        let mut words = vec![u64::MAX; (u32::MAX as usize).div_ceil(64)];
        assert!(ones_exceed_max_count(&words));
        *words.last_mut().unwrap() >>= 1;
        assert!(!ones_exceed_max_count(&words));
    }

    /// #7537: the line/column answers for a text a line index cannot hold are the line index's
    /// own, over every offset (and past the end) of texts with each kind of line break.
    #[test]
    fn the_scanned_line_columns_are_the_line_index_answers_7537() {
        let texts: [&[u8]; 9] = [
            b"",
            b"a",
            b"\n",
            b"a\nb",
            b"a\r\nb\rc\n",
            b"\r\n\r\n",
            b"{\n  \"a\": [1,\r\n 2]\n}\n",
            b"\r\r\n\n\rx",
            b"x\n\n\ny",
        ];
        for text in texts {
            let lines = crate::text::LineIndex::build(text);
            for offset in 0..text.len() + 3 {
                assert_eq!(
                    scan_line_column(text, offset),
                    lines.to_line_column(offset),
                    "{text:?} at {offset}"
                );
            }
            for line in 0..6 {
                for column in 0..text.len() + 3 {
                    assert_eq!(
                        scan_offset(text, line, column),
                        lines.to_offset(line, column),
                        "{text:?} at {line}:{column}"
                    );
                }
            }
        }
    }

    /// `from_parts`'s success path -- the loading-from-serialized-parts
    /// route (mmap, `SimpleJsonIndex::from_parts`'s own sibling test in
    /// `simple_light.rs`) -- reconstructs an index that navigates
    /// identically to the one it was extracted from. `bridge_tokens` isn't
    /// serialized (`from_parts` always sets it `false`, matching `build`):
    /// deliberate, since the only route to a `true` one is
    /// `JsonIndex::build_reindex` over freshly-generated bridge text, never
    /// data loaded back from a store (#3034).
    #[test]
    fn test_from_parts_round_trips_an_existing_index() {
        let json = br#"{"data":[1,2,3]}"#;
        let index = JsonIndex::build(json);

        let ib = index.ib().to_vec();
        let ib_len = index.ib_len();
        let bp = index.bp().words().to_vec();
        let bp_len = index.bp().len();

        let reconstructed = JsonIndex::from_parts(ib, ib_len, bp, bp_len);

        let orig_root = index.root(json);
        let new_root = reconstructed.root(json);
        assert_eq!(
            format!("{:?}", orig_root.value()),
            format!("{:?}", new_root.value())
        );
        let orig_child = orig_root.first_child().expect("object has a field");
        let new_child = new_root.first_child().expect("object has a field");
        assert_eq!(
            format!("{:?}", orig_child.value()),
            format!("{:?}", new_child.value())
        );
    }

    /// `Debug` is hand-written (not derived) so `seq_hint`, a navigation
    /// cache, can be left out -- its own doc comment explains why a derived
    /// impl would make two cursors that reached the same node by different
    /// routes print differently. Exercised directly since nothing else in
    /// the suite formats a `JsonIndex`.
    #[test]
    fn test_json_index_debug_omits_seq_hint_includes_bridge_tokens() {
        let json = br#"{"a":1}"#;
        let index = JsonIndex::build(json);
        let debug = format!("{index:?}");
        assert!(debug.contains("bridge_tokens: false"), "{debug}");
        assert!(!debug.contains("seq_hint"), "{debug}");

        let reindexed = JsonIndex::build_reindex(json);
        assert!(
            format!("{reindexed:?}").contains("bridge_tokens: true"),
            "{reindexed:?}"
        );
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

    /// The whole safety argument for `canonical_compact_jq_span_end`, run
    /// against a hand-picked corpus covering every rule in its own doc
    /// comment, split into three groups asserted differently -- and then
    /// swept a fourth time, all three groups together, through
    /// `stream_json`'s actual echo branch, which is the only assertion here
    /// that covers the *whole* gate rather than one of its halves:
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
    /// - **Invalid UTF-8** entries the predicate *accepts*, because the
    ///   scan certifies "canonical given valid UTF-8" and the gate's own
    ///   `from_utf8` supplies the given (#2608 follow-up; before that the
    ///   scan decoded every multi-byte sequence itself, at 34% of the
    ///   gate's cost, and these sat in the non-canonical group). What has
    ///   to hold for them is a property of the gate, not of the predicate,
    ///   so it is the cross-group sweep below that asserts it.
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

        // The third group: structurally canonical, but not valid UTF-8. The
        // predicate accepts these (#2608 follow-up moved the UTF-8 decision
        // out of the scan and onto the caller's single `from_utf8`), and
        // the gate still declines them -- which is the assertion that
        // matters, and the one the loop over all three groups makes.
        let utf8_rejected: Vec<Vec<u8>> =
            vec![invalid_utf8_bare_continuation, invalid_utf8_truncated_lead];

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
        for doc in &utf8_rejected {
            assert!(
                is_canonical_compact_jq_span(doc),
                "expected structurally canonical: {:?} (as text: {})",
                doc,
                String::from_utf8_lossy(doc)
            );
            assert!(
                core::str::from_utf8(doc).is_err(),
                "sanity: this group is the invalid-UTF-8 one: {doc:?}"
            );
        }
        for doc in canonical
            .iter()
            .chain(non_canonical.iter())
            .chain(utf8_rejected.iter())
        {
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

    /// #3167: the one-pass number arm accepts exactly what the two-pass
    /// rule it replaced accepted, and ends the span in the same place: the
    /// greedy [`nested_number_span`], then [`is_jq_canonical_number`] over
    /// it. Every token of up to four bytes over the greedy class plus the
    /// digits that exercise the prefix rules (`0`, `1`, `9`), followed by
    /// each of the 256 byte values and by end of input; five-byte tokens
    /// against the class bytes and a delimiter; the `0.000000x` spellings
    /// decNumber prints in scientific notation. Each accepted token is then
    /// checked inside an array and as an object value too, through the
    /// whole gate, so the loops' inlined dispatch is covered.
    #[test]
    fn scan_canonical_number_agrees_with_the_two_pass_rule_3167() {
        fn two_pass(bytes: &[u8], pos: usize) -> Option<usize> {
            let end = nested_number_span(bytes, pos);
            (end != pos && is_jq_canonical_number(&bytes[pos..end])).then_some(end)
        }
        const ALPHABET: &[u8] = b"019.eE+-";
        let mut by_len: Vec<Vec<Vec<u8>>> = vec![vec![Vec::new()]];
        for len in 1..=5 {
            let next = by_len[len - 1]
                .iter()
                .flat_map(|t| {
                    ALPHABET.iter().map(move |&b| {
                        let mut u = t.clone();
                        u.push(b);
                        u
                    })
                })
                .collect();
            by_len.push(next);
        }
        let all_bytes: Vec<Vec<u8>> = (0..=255u8).map(|b| vec![b]).collect();
        let short_suffixes: Vec<Vec<u8>> =
            ALPHABET.iter().chain(b",]} x").map(|&b| vec![b]).collect();
        let mut cases: Vec<(Vec<u8>, &[Vec<u8>])> = Vec::new();
        for tokens in &by_len[1..=4] {
            for t in tokens {
                cases.push((t.clone(), &all_bytes));
            }
        }
        for t in &by_len[5] {
            cases.push((t.clone(), &short_suffixes));
        }
        for spelling in [
            "0.000001",
            "0.0000001",
            "0.0000010",
            "0.000000",
            "0.0000000",
            "-0.0000001",
            "1.0000001",
            "10.0000001",
            "-inf",
            "nan",
            "-nan",
            "+1",
            "01",
            "-01",
            "1.2.3",
        ] {
            cases.push((spelling.as_bytes().to_vec(), &all_bytes));
        }
        let (mut checked, mut accepted) = (0usize, 0usize);
        for (token, suffixes) in &cases {
            // Only a token whose first byte routes to the number arm.
            if !matches!(token.first(), Some(b'-' | b'0'..=b'9')) {
                continue;
            }
            for suffix in suffixes.iter().map(Vec::as_slice).chain([&b""[..]]) {
                let doc: Vec<u8> = [token.as_slice(), suffix].concat();
                let want = two_pass(&doc, 0);
                assert_eq!(
                    scan_canonical_number(&doc, 0),
                    want,
                    "{:?}",
                    String::from_utf8_lossy(&doc)
                );
                checked += 1;
                accepted += usize::from(want.is_some());
            }
            // In container context, through the whole gate: the token is a
            // canonical element exactly when the two-pass rule ends it at
            // the closing bracket.
            for (open, close) in [(&b"["[..], &b"]"[..]), (b"{\"k\":", b"}"), (b"[0,", b",1]")] {
                let doc: Vec<u8> = [open, token.as_slice(), close].concat();
                let at = open.len();
                let element_ok = two_pass(&doc, at) == Some(at + token.len());
                assert_eq!(
                    canonical_compact_jq_span_end(&doc) == Some(doc.len()),
                    element_ok,
                    "{:?}",
                    String::from_utf8_lossy(&doc)
                );
            }
        }
        // The matrix must reach both answers, or it proves nothing.
        assert!(
            checked > 500_000 && accepted > 1_000,
            "{checked} {accepted}"
        );
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

        // A keyword's own grammar is the whole-token rule (#3035), so
        // `truexyz` is *not* a canonical `true` followed by garbage -- jq
        // reports `Invalid literal` for it. The echo must decline and the
        // re-render must error, so both halves of the safety argument agree
        // on `None`: the gate may only ever be a faster spelling of the
        // re-render, never a looser one.
        for doc in [&b"truexyz"[..], &b"falsezzz"[..], &b"nullx"[..]] {
            assert_eq!(
                render_compact_jq_through_gate(doc).as_deref(),
                None,
                "gate decline on malformed keyword {:?}",
                String::from_utf8_lossy(doc)
            );
            assert_eq!(
                render_compact_jq_by_rerender(doc).as_deref(),
                None,
                "re-render decline on malformed keyword {:?}",
                String::from_utf8_lossy(doc)
            );
        }
    }

    /// A canonical compact object of `keys` fields, `"k0000":0` through
    /// `"k{keys-1}":{keys-1}` in order -- except that when `duplicate` is
    /// `Some((first, repeat))`, field `repeat` carries field `first`'s
    /// *key* (and its own value), making the object carry exactly one
    /// duplicate key at a chosen pair of positions.
    ///
    /// Every key is the same length and starts with the same byte, so
    /// `key_span_fingerprint` separates them on their last content byte
    /// alone and two keys ten apart collide -- which is deliberate: it is
    /// the pairwise tier's span-comparison fallback, not its fingerprint
    /// compare, that has to settle those, and a corpus of keys that all
    /// differed in length would never reach it.
    fn wide_canonical_object(keys: usize, duplicate: Option<(usize, usize)>) -> Vec<u8> {
        let mut out = String::from("{");
        for i in 0..keys {
            if i > 0 {
                out.push(',');
            }
            let named = match duplicate {
                Some((first, repeat)) if i == repeat => first,
                _ => i,
            };
            out.push_str(&format!("\"k{named:04}\":{i}"));
        }
        out.push('}');
        out.into_bytes()
    }

    /// What jq prints for [`wide_canonical_object`]'s output: identical to
    /// it when there is no duplicate, and otherwise the collapse -- the
    /// repeated key keeps its *first* position and takes its *last* value,
    /// and the later field disappears.
    fn wide_object_expectation(keys: usize, duplicate: Option<(usize, usize)>) -> String {
        let mut out = String::from("{");
        let mut first_field = true;
        for i in 0..keys {
            if duplicate.is_some_and(|(_, repeat)| i == repeat) {
                continue;
            }
            if !first_field {
                out.push(',');
            }
            first_field = false;
            let value = match duplicate {
                Some((first, repeat)) if i == first => repeat,
                _ => i,
            };
            out.push_str(&format!("\"k{i:04}\":{value}"));
        }
        out.push('}');
        out
    }

    /// `scan_canonical_object`'s duplicate-key check has *two* tiers
    /// (#2608 review): the first `PAIRWISE_SPAN_SCAN_LIMIT` keys are
    /// compared pairwise with no allocation, and the key that overflows
    /// that limit hands the rest of the object to
    /// `scan_canonical_object_wide`, which collects every later key's hash
    /// for one exactly sized table (#3333). Nothing else in this module's tests reaches the second
    /// tier -- the corpus above tops out at six keys and the fuzz
    /// generator's objects are narrow -- so the seeding, the table tier
    /// and the *cross-tier* duplicate detection went unexercised.
    ///
    /// Both sides of the boundary, in both directions:
    /// - **Distinct keys**, at 17 (one past the limit, the seeding case),
    ///   20 and 40 keys: the whole span still certifies, and the
    ///   ground-truth re-render still reproduces it byte for byte.
    /// - **One duplicate**, placed at every interesting pair of positions:
    ///   `(16, 17)` is the key that *seeds* the table repeated by the
    ///   first key to use it; `(1, 17)` and `(1, 20)` are a pairwise-tier
    ///   key found again by the table; `(17, 18)` and `(17, 20)` are both
    ///   halves inside the table tier. Every one must be *declined* --
    ///   echoing a span whose object carries a duplicate key would print
    ///   a field jq collapses away -- and the gate must then produce the
    ///   collapse the re-render produces.
    ///
    /// The duplicate expectations are spelled out rather than compared
    /// only against the re-render: `assert_eq!(gate, rerender)` alone
    /// would pass just as happily if *both* wrongly echoed the duplicate.
    #[test]
    fn canonical_object_key_tiers_2608() {
        for keys in [17usize, 20, 40, 300] {
            let doc = wide_canonical_object(keys, None);
            assert!(
                is_canonical_compact_jq_span(&doc),
                "expected canonical at {keys} keys: {}",
                String::from_utf8_lossy(&doc)
            );
            assert_eq!(
                render_compact_jq_by_rerender(&doc).as_deref(),
                Some(doc.as_slice()),
                "expected round-trip at {keys} keys"
            );
            assert_eq!(
                render_compact_jq_through_gate(&doc),
                Some(wide_object_expectation(keys, None)),
                "gate echo at {keys} keys"
            );
        }

        for (keys, first, repeat) in [
            (20usize, 16usize, 17usize),
            (20, 1, 17),
            (24, 1, 20),
            (20, 17, 18),
            (24, 17, 20),
            // #3333: the table tier settles at the closing `}`, so a repeat
            // must be found when it is the object's *last* key, whether
            // it repeats a pairwise-tier key or a table-tier one, and
            // however wide the object has grown by then.
            (20, 0, 19),
            (20, 16, 19),
            (300, 0, 299),
            (300, 17, 299),
            (300, 150, 151),
        ] {
            let duplicate = Some((first, repeat));
            let doc = wide_canonical_object(keys, duplicate);
            assert!(
                !is_canonical_compact_jq_span(&doc),
                "a duplicate key at ({first}, {repeat}) of {keys} must not certify: {}",
                String::from_utf8_lossy(&doc)
            );
            let want = wide_object_expectation(keys, duplicate);
            assert_eq!(
                render_compact_jq_through_gate(&doc),
                Some(want.clone()),
                "gate must re-render the collapse at ({first}, {repeat}) of {keys}"
            );
            assert_eq!(
                render_compact_jq_by_rerender(&doc)
                    .map(|b| String::from_utf8_lossy(&b).into_owned()),
                Some(want),
                "re-render at ({first}, {repeat}) of {keys}"
            );
        }
    }

    /// The table tier's width ceiling (#2608 review, #3333, #7293):
    /// [`KeyHashes`] stops growing at its own ceiling, so from
    /// `KeyHashes::SATURATING_KEYS` keys `scan_canonical_object_wide` settles
    /// its list by sorting it (`shared_key_hashes_if_any`) rather than through a
    /// table. Until #7293 it declined there, so every caller re-walked an
    /// object it had already hashed 786,432 keys of.
    ///
    /// The width is not negotiable -- `KeyHashes::MAX_SLOTS * 3 / 4`
    /// exactly, the point `src/jq/document.rs`'s own
    /// `key_hashes_stops_growing_at_the_ceiling_1588` pins -- so this
    /// document is ~9 MB and this is deliberately the only test here that
    /// builds one. Both sides of the ceiling certify a no-repeat object, a
    /// repeat on either side is still found (the last key repeating the
    /// first, so the sort must compare the two ends of the list), and the
    /// gate prints the document either way, because the re-render of a
    /// canonical span *is* that span.
    #[test]
    fn canonical_echo_settles_a_saturating_object_by_sort_7293() {
        // `KeyHashes::MAX_SLOTS` (`1 << 20`) * 3/4: the key count at which
        // `insert` would next have doubled a table already at its ceiling,
        // so `saturated()` first answers `true`. Spelled out here as an
        // oracle independent of the constant the scan uses, and checked
        // against it on the next line.
        const SATURATES_AT: usize = 786_432;
        assert_eq!(KeyHashes::SATURATING_KEYS, SATURATES_AT);

        fn object(keys: usize, last: Option<&str>) -> Vec<u8> {
            let mut doc = String::with_capacity(keys * 13);
            doc.push('{');
            for i in 0..keys {
                if i > 0 {
                    doc.push(',');
                }
                match last {
                    Some(key) if i == keys - 1 => doc.push_str(&format!("\"{key}\":1")),
                    _ => doc.push_str(&format!("\"k{i:06}\":0")),
                }
            }
            doc.push('}');
            doc.into_bytes()
        }

        for keys in [SATURATES_AT - 1, SATURATES_AT, SATURATES_AT + 1] {
            let doc = object(keys, None);
            assert!(
                is_canonical_compact_jq_span(&doc),
                "{keys} distinct keys certify on either side of the ceiling"
            );
            assert_eq!(
                render_compact_jq_through_gate(&doc),
                Some(String::from_utf8_lossy(&doc).into_owned()),
                "the re-render of a canonical span is the span ({keys} keys)"
            );
            let repeat = object(keys, Some("k000000"));
            assert!(
                !is_canonical_compact_jq_span(&repeat),
                "a last key repeating the first declines at {keys} keys"
            );
            // #7310: and says it declined on the keys alone, on either side
            // of the ceiling (the table below it, the sort from it).
            assert_eq!(
                canonical_compact_jq_scan_at_depth(&repeat, 0),
                CanonicalScan::RepeatedKeys {
                    end: repeat.len(),
                    shared: vec![key_hash(b"k000000")],
                },
                "a repeat at {keys} keys is reported as one"
            );
            // And is printed as jq collapses it: the first key takes the last
            // one's value, and the last goes (#7310, the plan built from the
            // scan's hashes on either side of the ceiling).
            let mut want = String::from_utf8(object(keys - 1, None)).unwrap();
            want.replace_range(.."{\"k000000\":0".len(), "{\"k000000\":1");
            assert_eq!(
                render_compact_jq_through_gate(&repeat),
                Some(want),
                "the collapse of a repeat at {keys} keys"
            );
        }
    }

    /// #7310: [`canonical_compact_jq_scan_at_depth`] reports
    /// [`CanonicalScan::RepeatedKeys`] for exactly the value that is a wide
    /// object declining on its own keys' hashes, and nothing else the scan
    /// declines, and hands back exactly the hashes that repeat. A wrong
    /// `RepeatedKeys` would send a malformed object past the walk that refuses
    /// it, so every neighbour of the one shape that earns it is held to
    /// `Declined`: a fault before or after the repeated key, a fault inside a
    /// later value, and an object that never closes. A repeat in a nested
    /// object is [`CanonicalScan::NestedRepeatedKeys`] when the value is
    /// canonical around it (#7366), and every neighbour of that is `Declined`
    /// too: a second such object, the value's own repeat after one, a fault
    /// anywhere else.
    #[test]
    fn canonical_scan_reports_only_its_own_keys_repeat_7310() {
        let repeated = |doc: &str, depth: usize| match canonical_compact_jq_scan_at_depth(
            doc.as_bytes(),
            depth,
        ) {
            CanonicalScan::RepeatedKeys { end, shared } => Some((end, shared)),
            _ => None,
        };
        let repeat = wide_canonical_object(40, Some((3, 30)));
        let repeat = String::from_utf8(repeat).unwrap();
        let want = Some((repeat.len(), vec![key_hash(b"k0003")]));
        assert_eq!(repeated(&repeat, 0), want);
        // The starting depth is the caller's, and the record is compared to it.
        assert_eq!(repeated(&repeat, 7), want);
        // Two repeated keys, one of them three times: each hash once, sorted.
        let doc = wide_canonical_object(40, Some((3, 30)))
            .into_iter()
            .map(char::from)
            .collect::<String>()
            .replacen("\"k0031\"", "\"k0017\"", 1)
            .replacen("\"k0032\"", "\"k0017\"", 1);
        let mut both = vec![key_hash(b"k0003"), key_hash(b"k0017")];
        both.sort_unstable();
        assert_eq!(repeated(&doc, 0), Some((doc.len(), both)));
        // An escaped key is hashed as written, escape and all.
        let escaped = repeat.replace("\"k0003\"", "\"k\\n3\"");
        assert_eq!(
            repeated(&escaped, 0),
            Some((escaped.len(), vec![key_hash(b"k\\n3")]))
        );
        // A canonical object is still `Canonical`.
        let clean = String::from_utf8(wide_canonical_object(40, None)).unwrap();
        assert_eq!(
            canonical_compact_jq_scan_at_depth(clean.as_bytes(), 0),
            CanonicalScan::Canonical(clean.len())
        );
        // The whole span is the object's even with bytes after it.
        let trailing = format!("{repeat} 1");
        assert_eq!(repeated(&trailing, 0), want);
        // A repeat among the first `PAIRWISE_SPAN_SCAN_LIMIT` keys is the table tier's from the
        // repeated key on, so it is recorded the same (#7355): in an object of two, ten and forty
        // keys, the first key repeated by the second, by the last and by one past the pairwise tier.
        for (keys, first, again) in [
            (2, 0, 1),
            (10, 2, 7),
            (40, 2, 7),
            (PAIRWISE_SPAN_SCAN_LIMIT, 0, PAIRWISE_SPAN_SCAN_LIMIT - 1),
            (PAIRWISE_SPAN_SCAN_LIMIT + 3, 4, PAIRWISE_SPAN_SCAN_LIMIT),
        ] {
            let doc = String::from_utf8(wide_canonical_object(keys, Some((first, again)))).unwrap();
            let key = alloc::format!("k{first:04}");
            assert_eq!(
                repeated(&doc, 0),
                Some((doc.len(), vec![key_hash(key.as_bytes())])),
                "{doc}"
            );
            // A fault after the repeat still declines the span with no record.
            let faulty = doc.replacen(&alloc::format!(":{}", keys - 1), ":1e3", 1);
            assert_eq!(
                canonical_compact_jq_scan_at_depth(faulty.as_bytes(), 0),
                CanonicalScan::Declined,
                "{faulty}"
            );
        }

        // A repeat in an object further in is that object's record, and the
        // scan goes on past it to certify the rest of the value (#7366): the
        // object found by the wide tier and by the pairwise one (`early`), as an
        // element, a member, deeper, and with a wide value after it.
        let early = String::from_utf8(wide_canonical_object(10, Some((2, 7)))).unwrap();
        let nested_record = |doc: &str, depth: usize| match canonical_compact_jq_scan_at_depth(
            doc.as_bytes(),
            depth,
        ) {
            CanonicalScan::NestedRepeatedKeys { end, object } => Some((end, *object)),
            _ => None,
        };
        let wide_tail = String::from_utf8(wide_canonical_object(40, None)).unwrap();
        for (object, shared) in [(&repeat, key_hash(b"k0003")), (&early, key_hash(b"k0002"))] {
            for (before, after, level) in [
                ("[", "]", 1),
                ("{\"a\":", "}", 1),
                ("[1,", ",2]", 1),
                ("{\"x\":[true,{\"y\":", "}],\"z\":null}", 3),
                ("[[", alloc::format!("],{wide_tail}]").as_str(), 2),
            ] {
                let doc = format!("{before}{object}{after}");
                let want = NestedRepeat {
                    start: before.len(),
                    end: before.len() + object.len(),
                    level,
                    shared: vec![shared],
                };
                assert_eq!(
                    nested_record(&doc, 0),
                    Some((doc.len(), want.clone())),
                    "{doc}"
                );
                // The level is counted from the value, whatever depth it starts at.
                assert_eq!(nested_record(&doc, 5), Some((doc.len(), want)), "{doc}");
                // The scans that do not read the record decline at the object.
                assert_eq!(canonical_compact_jq_span_end(doc.as_bytes()), None, "{doc}");
            }
        }
        // One such object is recorded, and its value certified around it; a
        // second, or the value itself repeating a key after one, or any later
        // fault, declines the value. So does an earlier one, which ends the scan.
        let declined_around = [
            format!("[{repeat},{early}]"),
            format!("[{early},{early}]"),
            format!("[{early},[{repeat}]]"),
            format!("{{\"a\":{early},\"a\":1}}"),
            format!("{{\"a\":{early},\"b\":1,\"a\":1}}"),
            format!("[{early},1e3]"),
            format!("[{early},1 ]"),
            format!("[1e3,{early}]"),
            format!("[{early}"),
        ];
        for doc in &declined_around {
            assert_eq!(
                canonical_compact_jq_scan_at_depth(doc.as_bytes(), 0),
                CanonicalScan::Declined,
                "{doc}"
            );
        }
        // An object of a wide tier recorded inside it is what declines the
        // outer one, which is no record of its own.
        let wide_around = String::from_utf8(wide_canonical_object(40, None))
            .unwrap()
            .replacen(":7,", &alloc::format!(":{early},"), 1);
        let Some((end, object)) = nested_record(&wide_around, 0) else {
            panic!("{wide_around}");
        };
        assert_eq!(end, wide_around.len());
        assert_eq!(&wide_around[object.start..object.end], early);

        let declined = [
            // A fault ahead of the closing brace ends the scan first: in a
            // value before the repeat, after it, in a key, a stray space.
            repeat.replacen(":1,", ":1e3,", 1),
            repeat.replacen(":31,", ":1e3,", 1),
            repeat.replacen(":31,", ":[1 ],", 1),
            repeat.replacen("\"k0035\"", "\"k\\/35\"", 1),
            repeat.replacen(",\"k0039\"", ", \"k0039\"", 1),
            // Never closes, and so never settles its hashes.
            repeat[..repeat.len() - 1].to_owned(),
            format!("{},", &repeat[..repeat.len() - 1]),
        ];
        for doc in &declined {
            assert_eq!(
                canonical_compact_jq_scan_at_depth(doc.as_bytes(), 0),
                CanonicalScan::Declined,
                "{doc}"
            );
        }
        // A nested object's repeat is `RepeatedKeys` only when it is the value
        // asked about: from its parent it is the text around it (#7366).
        let nested = format!("{{\"a\":{repeat}}}");
        let index = JsonIndex::build(nested.as_bytes());
        let inner = index.root(nested.as_bytes()).first_child().unwrap();
        let inner = inner.next_sibling().unwrap();
        assert_eq!(
            inner.canonical_compact_scan(1),
            CanonicalSpan::RepeatedKeys {
                start: 5,
                span: &nested[5..nested.len() - 1],
                shared: vec![key_hash(b"k0003")],
            }
        );
        assert_eq!(
            index.root(nested.as_bytes()).canonical_compact_scan(0),
            CanonicalSpan::NestedRepeatedKeys {
                span: &nested,
                object: Box::new(NestedRepeat {
                    start: 5,
                    end: nested.len() - 1,
                    level: 1,
                    shared: vec![key_hash(b"k0003")],
                }),
            }
        );
        let element = format!("[0,{nested}]");
        let index = JsonIndex::build(element.as_bytes());
        let second = index.root(element.as_bytes()).first_child().unwrap();
        let second = second.next_sibling().unwrap();
        assert!(matches!(
            second.canonical_compact_scan(1),
            CanonicalSpan::NestedRepeatedKeys { span, .. } if span == nested
        ));
        // A node that is not the document finds the object from where the node
        // starts: printed from its text, it prints what the re-render does.
        for indent in [
            IndentSpec::COMPACT,
            IndentSpec {
                width: 2,
                unit: ' ',
            },
        ] {
            let mut printed = String::new();
            second
                .stream_json(&mut printed, indent, false, JsonConvention::JqCompat)
                .unwrap();
            let mut rerender = String::new();
            stream_json_pretty(
                &mut rerender,
                second.value(),
                0,
                indent.width,
                indent.unit,
                false,
                JsonConvention::JqCompat,
                0,
            )
            .unwrap();
            assert_eq!(printed, rerender, "{indent:?}");
        }
        // A repeat among the first keys is recorded for a scan that reads the record
        // (above), and not for one that does not (#7355).
        assert_eq!(canonical_compact_jq_span_end(early.as_bytes()), None);
        assert!(!is_canonical_compact_jq_span(early.as_bytes()));
        // The scan leaves UTF-8 to its caller, and the cursor declines an
        // object that is not: a key that will not decode needs the walk that
        // compares every key.
        let mut invalid = repeat.clone().into_bytes();
        let at = invalid.windows(7).position(|w| w == b"\"k0005\"").unwrap() + 3;
        invalid[at] = 0xFF;
        assert!(matches!(
            canonical_compact_jq_scan_at_depth(&invalid, 0),
            CanonicalScan::RepeatedKeys { .. }
        ));
        let index = JsonIndex::build(&invalid);
        assert_eq!(
            index.root(&invalid).canonical_compact_scan(0),
            CanonicalSpan::Declined
        );
    }

    /// #7366: an object printed by [`stream_scanned_repeats`] at a level below
    /// the value, which the plan finds nothing to collapse in (a shared hash no
    /// two of its keys have, as a bare collision leaves it), is printed by the
    /// object arm at that level: members one level further in, `}` at it.
    #[test]
    fn scanned_repeats_without_a_plan_print_at_their_level_7366() {
        let doc = "{\"a\":1,\"b\":[2,{\"c\":null}],\"d\":{}}";
        let index = JsonIndex::build(doc.as_bytes());
        let root = index.root(doc.as_bytes());
        let StandardJson::Object(fields) = root.value() else {
            panic!("{doc}");
        };
        let shared = [key_hash(b"b")];
        assert!(matches!(
            text_repeat_plan(doc, 0, &shared),
            TextPlan::NoPlan
        ));
        assert!(repeat_plan_from_shared(&fields, &shared, raw_key_hash, None).is_none());
        for (indent, level) in [
            (IndentSpec::COMPACT, 2),
            (
                IndentSpec {
                    width: 2,
                    unit: ' ',
                },
                2,
            ),
            (
                IndentSpec {
                    width: 1,
                    unit: '\t',
                },
                3,
            ),
        ] {
            let mut printed = String::new();
            let text = RepeatText {
                level,
                ..RepeatText::whole(doc, 0)
            };
            stream_scanned_repeats(
                &mut printed,
                fields,
                text,
                &shared,
                indent,
                JsonConvention::JqCompat,
            )
            .unwrap();
            let mut rerender = String::new();
            stream_json_pretty(
                &mut rerender,
                root.value(),
                level * indent.width,
                indent.width,
                indent.unit,
                false,
                JsonConvention::JqCompat,
                level,
            )
            .unwrap();
            assert_eq!(printed, rerender, "{indent:?} {level}");
            if indent.width > 0 {
                let close = alloc::format!(
                    "\n{}}}",
                    indent.unit.to_string().repeat(level * indent.width)
                );
                assert!(printed.ends_with(&close), "{printed:?}");
            }
        }
    }

    /// #7310: `stream_json` prints an object the scan declined on its keys
    /// from a repeat plan built on the scan's own hashes, and that must print
    /// what the re-render (`stream_json_pretty`, whose plan takes a key walk of
    /// its own) prints, compact and indented. The key alphabet has escaped keys
    /// in it, whose scan hash differs from the walk's decoded one: comparing
    /// them under the wrong hash would leave a repeat uncollapsed. Seeded wide objects over a key alphabet small enough to repeat
    /// often, with values that are themselves canonical or not, nested
    /// objects that repeat their own keys, escaped and multi-byte keys, and
    /// the odd fault: every outcome of the scan comes up, and each document
    /// is held to the re-render whatever the scan said.
    #[test]
    fn repeated_keys_print_as_the_rerender_does_7310() {
        use rand::{RngExt, SeedableRng};
        use rand_chacha::ChaCha8Rng;

        const KEYS: &[&str] = &[
            "a",
            "b",
            "k0",
            "k1",
            "\\n",
            "\\u0001",
            "\\\"q",
            "\u{e9}",
            "\u{1f600}",
            "x y",
        ];
        // The first `CANONICAL` are jq's own spelling (jq 1.7.1 keeps `1.0`
        // and `1.50` as written); the rest it rewrites, or repeat a key.
        const CANONICAL: usize = 12;
        const VALUES: &[&str] = &[
            "0",
            "-1",
            "1.5",
            "1.0",
            "null",
            "true",
            "\"s\"",
            "\"\u{e9}\\n\"",
            "[]",
            "{}",
            "[1,2]",
            "{\"b\":[{\"c\":null}]}",
            "1e3",
            "\"\\u00e9\"",
            "[1, 2]",
            "{\"a\":1,\"a\":2}",
        ];

        // A key comes from the small alphabet a quarter of the time, among the first
        // `PAIRWISE_SPAN_SCAN_LIMIT` too, where a repeat is the pairwise tier's (#7355), and a
        // root object may be as narrow as two keys. A
        // value is a fault (`VALUES`' non-canonical tail) only in a `dirty`
        // document, so that most documents get as far as the closing brace.
        fn object(rng: &mut ChaCha8Rng, depth: usize, dirty: bool, out: &mut String) {
            let fields = if depth == 0 {
                rng.random_range(2..60)
            } else {
                rng.random_range(0..24)
            };
            out.push('{');
            for i in 0..fields {
                if i > 0 {
                    out.push(',');
                }
                if rng.random_range(0..4) > 0 {
                    out.push_str(&format!("\"d{i}\":"));
                } else {
                    out.push_str(&format!("\"{}\":", KEYS[rng.random_range(0..KEYS.len())]));
                }
                if depth < 2 && rng.random_range(0..8) == 0 {
                    object(rng, depth + 1, dirty, out);
                } else {
                    let values = if dirty { VALUES.len() } else { CANONICAL };
                    out.push_str(VALUES[rng.random_range(0..values)]);
                }
            }
            out.push('}');
        }

        let indents = [
            IndentSpec::COMPACT,
            IndentSpec {
                width: 2,
                unit: ' ',
            },
            IndentSpec {
                width: 1,
                unit: '\t',
            },
        ];
        let mut rng = ChaCha8Rng::seed_from_u64(7310);
        let mut repeated = 0;
        let mut around = 0;
        let mut from_text = 0;
        for _ in 0..4000 {
            let mut doc = String::new();
            let dirty = rng.random_range(0..4) == 0;
            // The object is the document half the time, and otherwise inside one, so
            // that its repeat is a nested object's and printed around (#7366): after a wide
            // run of canonical values, in an object, or beside a second object of its own.
            // #7346: or it is an array's second element, scanned on its own, so that the
            // value printed starts past the document's first byte.
            let mut second = false;
            match rng.random_range(0..7) {
                0..=2 => object(&mut rng, 0, dirty, &mut doc),
                3 => {
                    doc.push_str("[0,\"\u{e9}\",{\"x\":[1,{}]},");
                    object(&mut rng, 0, dirty, &mut doc);
                    doc.push_str(",[[]]]");
                }
                4 => {
                    doc.push_str("{\"data\":");
                    object(&mut rng, 0, dirty, &mut doc);
                    doc.push_str(",\"n\":1.0}");
                }
                5 => {
                    doc.push('[');
                    object(&mut rng, 0, dirty, &mut doc);
                    doc.push(',');
                    object(&mut rng, 1, dirty, &mut doc);
                    doc.push(']');
                }
                _ => {
                    doc.push_str("[1,");
                    object(&mut rng, 0, dirty, &mut doc);
                    doc.push(']');
                    second = true;
                }
            }
            let index = JsonIndex::build(doc.as_bytes());
            let root = index.root(doc.as_bytes());
            let root = if second {
                root.first_child().unwrap().next_sibling().unwrap()
            } else {
                root
            };
            match root.canonical_compact_scan(0) {
                CanonicalSpan::RepeatedKeys {
                    start,
                    span,
                    shared,
                } => {
                    repeated += 1;
                    // #7346: and is written from its text when its plan keeps
                    // spans (unless more than half the fields have a step).
                    let StandardJson::Object(fields) = root.value() else {
                        panic!("not an object"); // patchcov: coverage tolerate-line reason="the failure message for the assertion this #7346 test exists to make"
                    };
                    let mut plan = text_repeat_plan(span, start, &shared).plan();
                    // #7350: the plan found from the text is the fields'
                    // walk's, field by field, unless it was too dense to keep,
                    // and the walk that records spans finds the same spans.
                    let walked = repeat_plan_from_shared(&fields, &shared, raw_key_hash, None);
                    match (plan.as_mut(), walked) {
                        (Some(plan), Some(walked)) => {
                            assert_eq!(
                                text_plan_fates(plan, &fields),
                                walked_plan_fates(walked, &fields),
                                "{doc}"
                            );
                            let spanned = repeat_plan_from_shared(
                                &fields,
                                &shared,
                                raw_key_hash,
                                Some(JsonCursor::text_position),
                            )
                            .unwrap();
                            assert_eq!(spanned.spans(), plan.spans(), "{doc}");
                        }
                        (Some(_), None) => panic!("only the text found a plan: {doc}"), // patchcov: coverage tolerate-line reason="the failure message for the assertion this #7350 test exists to make"
                        // Too dense to keep spans is the one reason to have
                        // no plan from the text when the walk finds one.
                        #[rustfmt::skip]
                        (None, walked) => assert!(walked.into_iter().all(|w| fates_are_dense(&walked_plan_fates(w, &fields))), "{doc}"), // patchcov: coverage tolerate-line reason="the sweep's objects are never dense or a bare collision; dense_repeats_print_by_the_walk_7346 and scanned_repeats_without_a_plan_print_at_their_level_7366 cover both (#7350)"
                    }
                    from_text += usize::from(
                        plan.is_some_and(|plan| planned_pieces(&plan, span, start).is_some()),
                    );
                }
                CanonicalSpan::NestedRepeatedKeys { .. } => around += 1,
                _ => {}
            }
            for indent in indents {
                let mut gate = String::new();
                let gate = root
                    .stream_json(&mut gate, indent, false, JsonConvention::JqCompat)
                    .map(|()| gate);
                let mut rerender = String::new();
                let rerender = stream_json_pretty(
                    &mut rerender,
                    root.value(),
                    0,
                    indent.width,
                    indent.unit,
                    false,
                    JsonConvention::JqCompat,
                    0,
                )
                .map(|()| rerender);
                assert_eq!(gate.ok(), rerender.ok(), "{indent:?} {doc}");
            }
        }
        // The sweep has to reach the new branch often enough to mean anything.
        assert!(repeated > 100, "only {repeated} documents declined on keys");
        assert!(
            around > 100,
            "only {around} documents printed around an object"
        );
        assert!(
            from_text > 100,
            "only {from_text} documents printed from text"
        );
    }

    /// What a repeat plan makes of each of an object's fields, in order:
    /// `None` for a field written as it stands, `Some(None)` for one dropped,
    /// and `Some(Some(at))` for one written with the value that starts at byte
    /// `at` of the document (#7350).
    type Fates = Vec<Option<Option<usize>>>;

    /// How many fields `fields` has.
    fn field_count<W: AsRef<[u64]> + Clone>(fields: &JsonFields<'_, W>) -> usize {
        core::iter::successors(Some(*fields), |f| f.uncons().map(|(_, rest)| rest)).count() - 1
    }

    /// Whether a plan with these fates gives up its spans ([`StepSpan::dense`]):
    /// during the walk, at some step, or at its end.
    fn fates_are_dense(fates: &Fates) -> bool {
        let mut steps = 0usize;
        for (at, fate) in fates.iter().enumerate() {
            if fate.is_some() {
                steps += 1;
                if StepSpan::dense(steps, at + 1, false) {
                    return true;
                }
            }
        }
        StepSpan::dense(steps, fates.len(), true)
    }

    /// [`Fates`] of the plan [`text_repeat_plan`] found for `fields`.
    fn text_plan_fates<W: AsRef<[u64]> + Clone>(
        plan: &mut RepeatPlan<usize>,
        fields: &JsonFields<'_, W>,
    ) -> Fates {
        (0..field_count(fields))
            .map(|at| match plan.fate(at) {
                FieldFate::Unchanged => None,
                FieldFate::Dropped => Some(None),
                FieldFate::Value(value) => Some(Some(value)),
            })
            .collect()
    }

    /// [`Fates`] of the plan the fields' walk found for `fields`
    /// ([`repeat_plan_from_shared`]).
    fn walked_plan_fates<W: AsRef<[u64]> + Clone>(
        mut plan: RepeatPlan<JsonCursor<'_, W>>,
        fields: &JsonFields<'_, W>,
    ) -> Fates {
        (0..field_count(fields))
            .map(|at| match plan.fate(at) {
                FieldFate::Unchanged => None,
                FieldFate::Dropped => Some(None),
                FieldFate::Value(cursor) => Some(Some(cursor.text_position().unwrap())),
            })
            .collect()
    }

    /// #7350: [`text_repeat_plan`] cuts a certified object's members with a
    /// byte loop, so a member whose value hides a `,`, `}`, `]`, `"` or `:`
    /// inside a string or a nested container must still be one member, and an
    /// escaped key must be compared under the scan's raw hash and grouped by
    /// its raw spelling. Each object here is certified and declined on its
    /// keys, at the root and one level in, and its text plan is the fields'
    /// walk's, field by field, with each step's three positions where the
    /// index puts them. Text that is not a certified object has no plan.
    #[test]
    #[allow(clippy::literal_string_with_formatting_args)] // JSON text, not a format string.
    fn text_repeat_plan_cuts_members_as_the_walk_does_7350() {
        let head = (0..40)
            .map(|i| format!("\"k{i:04}\":{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let tails = [
            r#""a":"x,}\":{","b":1,"a":2"#,
            r#""a\n":[1,{"a\n":"]"}],"a":{},"a\n":"\\""#,
            r#""\u0000":null,"é":"é","\u0000":[[],[[]]],"é":true"#,
            r#""k0003":{"k0003":{"x":[",",{"}":"{"}]}},"z":[],"k0003":-1.5"#,
        ]
        .map(String::from);
        // Strings, of a key and of a value, long enough to cross several
        // 64-byte blocks, full of commas and escaped quotes, with an escape at
        // every offset against a block's end.
        let long = (0..64).map(|pad| {
            let text = format!("{}{}", "p".repeat(pad), r#",\"x\\"#.repeat(40));
            format!(r#""{text}":1,"a":"{text}","{text}":[2],"a":3"#)
        });
        for tail in tails.into_iter().chain(long) {
            for nested in [false, true] {
                let object = format!("{{{head},{tail}}}");
                let doc = if nested {
                    format!("[1,{object}]")
                } else {
                    object.clone()
                };
                let index = JsonIndex::build(doc.as_bytes());
                let mut root = index.root(doc.as_bytes());
                if nested {
                    root = root.first_child().unwrap().next_sibling().unwrap();
                }
                let CanonicalSpan::RepeatedKeys {
                    start,
                    span,
                    shared,
                } = root.canonical_compact_scan(0)
                else {
                    panic!("the scan declines on the keys alone: {doc}"); // patchcov: coverage tolerate-line reason="the failure message for the assertion this #7350 test exists to make"
                };
                assert_eq!((start, span), (usize::from(nested) * 3, object.as_str()));
                let StandardJson::Object(fields) = root.value() else {
                    panic!("an object"); // patchcov: coverage tolerate-line reason="the failure message for the assertion this #7350 test exists to make"
                };
                let mut plan = text_repeat_plan(span, start, &shared).plan().expect(&doc);
                let walked = repeat_plan_from_shared(&fields, &shared, raw_key_hash, None).unwrap();
                assert_eq!(
                    text_plan_fates(&mut plan, &fields),
                    walked_plan_fates(walked, &fields),
                    "{doc}"
                );
                // Each step's key, value and next key, from the index.
                let mut want = Vec::new();
                let mut walk = fields;
                let mut stepped = false;
                while let Some((key, key_cursor, rest)) = walk.uncons_key() {
                    let key_at = key_cursor.text_position().unwrap();
                    if core::mem::take(&mut stepped) {
                        let last: &mut StepSpan = want.last_mut().unwrap();
                        last.next = key_at;
                    }
                    if raw_key_hash(&key).is_some_and(|h| shared.binary_search(&h).is_ok()) {
                        let (field, _) = walk.uncons().unwrap();
                        want.push(StepSpan {
                            key: key_at,
                            value: field.value_cursor().text_position().unwrap(),
                            next: StepSpan::LAST,
                        });
                        stepped = true;
                    }
                    walk = rest;
                }
                assert_eq!(plan.spans(), want.as_slice(), "{doc}");
                let cuts = [&span[1..], &span[..span.len() - 1], &span[..span.len() - 2]];
                for cut in cuts {
                    assert!(
                        text_repeat_plan(cut, start, &shared).plan().is_none(),
                        "{cut}"
                    );
                }
                // Without the shared hashes nothing is compared, so nothing repeats.
                assert!(text_repeat_plan(span, start, &[]).plan().is_none(), "{doc}");
            }
        }
        // Ten keys that occur once, so that two `a`s are not dense and only
        // the fault can leave the plan out.
        let pad = (0..10)
            .map(|i| format!("\"u{i}\":{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let shared = [key_hash(b"a")];
        let well_formed = format!("{{{pad},\"a\":1,\"a\":2}}");
        assert!(text_repeat_plan(&well_formed, 0, &shared).plan().is_some());
        for fault in [
            "\"a\"",
            "\"a\":",
            "\"a\":1,",
            "\"a\":1],\"a\":2",
            "\"a\":1:\"b\":2,\"a\":3",
            "\"a\":1\"b\",\"a\":2",
            "\"a\":1{},\"a\":2",
            "\"a\":[1,\"a\":2",
            "\"a\":\"1,\"a\":2",
            "\"a\":1,\"a\":2,1",
            "a:1,\"a\":2",
            "\"a\":\"x\u{1}y\",\"a\":2",
            "\"a\":,\"a\":2",
        ] {
            let text = format!("{{{pad},{fault}}}");
            assert!(
                text_repeat_plan(&text, 0, &shared).plan().is_none(),
                "{text}"
            );
        }
        // A number that runs past the reading budget is handed over, not cut
        // at the budget's edge.
        let long = format!("{{{pad},\"a\":{},\"a\":2}}", "1".repeat(40_000));
        assert!(matches!(
            text_repeat_plan(&long, 0, &shared),
            TextPlan::Walk
        ));
        for text in ["{}", "[]", "{\"a\":1,\"a\":2}"] {
            assert!(
                text_repeat_plan(text, 0, &shared).plan().is_none(),
                "{text}"
            );
        }
    }

    /// #7350: an object whose members average more than
    /// [`TEXT_WALK_MEMBER_BYTES`] has no text plan, and is printed from its
    /// text by the fields' walk's plan, which records where each step lies
    /// (#7346): the same spans the text walk finds for short members. Either
    /// way it prints what the re-render prints.
    #[test]
    fn long_members_take_the_fields_walk_7350() {
        // Few members, long enough that no 64-member average is ever taken:
        // the budget hands them over all the same.
        for (keys, len) in [
            (200, 2000),
            (200, 400),
            (200, 100),
            (3, 100_000),
            (30, 2000),
        ] {
            let mut fields: Vec<String> = (0..keys)
                .map(|i| format!("\"k{i}\":\"{}{i}\"", "x".repeat(len)))
                .collect();
            fields.push(format!("\"k0\":[{}]", "1,".repeat(len / 2) + "1"));
            let doc = format!("{{{}}}", fields.join(","));
            let index = JsonIndex::build(doc.as_bytes());
            let root = index.root(doc.as_bytes());
            let CanonicalSpan::RepeatedKeys {
                start,
                span,
                shared,
            } = root.canonical_compact_scan(0)
            else {
                panic!("the scan declines on the keys alone"); // patchcov: coverage tolerate-line reason="the failure message for the assertion this #7350 test exists to make"
            };
            let StandardJson::Object(fields) = root.value() else {
                panic!("an object"); // patchcov: coverage tolerate-line reason="the failure message for the assertion this #7350 test exists to make"
            };
            let walked = repeat_plan_from_shared(
                &fields,
                &shared,
                raw_key_hash,
                Some(JsonCursor::text_position),
            )
            .unwrap();
            let text = text_repeat_plan(span, start, &shared).plan();
            assert_eq!(text.is_none(), len > TEXT_WALK_MEMBER_BYTES, "{len}");
            if let Some(text) = text {
                assert_eq!(text.spans(), walked.spans());
            }
            assert!(walked.spanned_steps().is_some());
            for indent in [
                IndentSpec::COMPACT,
                IndentSpec {
                    width: 2,
                    unit: ' ',
                },
            ] {
                let mut printed = String::new();
                root.stream_json(&mut printed, indent, false, JsonConvention::JqCompat)
                    .unwrap();
                let mut rerender = String::new();
                stream_json_pretty(
                    &mut rerender,
                    root.value(),
                    0,
                    indent.width,
                    indent.unit,
                    false,
                    JsonConvention::JqCompat,
                    0,
                )
                .unwrap();
                assert_eq!(printed, rerender, "{len}");
            }
        }
    }

    /// #7350: [`string_special`]'s word-at-a-time search finds what a
    /// byte-at-a-time one does, from every start, over seeded text dense and
    /// sparse in quotes, backslashes, control bytes and bytes one bit away from
    /// them.
    #[test]
    fn string_special_matches_the_byte_search_7350() {
        use rand::{RngExt, SeedableRng};
        use rand_chacha::ChaCha8Rng;
        const BYTES: &[u8] = b"\"\\x#]\x22\x5d\x5c\xa2\xdc\x23\x5e\x00\x1f\x20\xff";
        let mut rng = ChaCha8Rng::seed_from_u64(7350);
        for len in 0..80 {
            for _ in 0..20 {
                let sparse = rng.random_range(0..2) == 0;
                let text: Vec<u8> = (0..len)
                    .map(|_| {
                        if sparse && rng.random_range(0..12) > 0 {
                            b'x'
                        } else {
                            BYTES[rng.random_range(0..BYTES.len())]
                        }
                    })
                    .collect();
                for from in 0..=len + 1 {
                    let want = text
                        .get(from..)
                        .and_then(|rest| {
                            rest.iter()
                                .position(|&b| b == b'"' || b == b'\\' || b < 0x20)
                        })
                        .map(|at| from + at);
                    assert_eq!(string_special(&text, from), want, "{text:?} {from}");
                }
            }
        }
    }

    /// [`cut_planned_text`]'s pieces, collected (#7346).
    fn planned_pieces<'s>(
        plan: &RepeatPlan<usize>,
        span: &'s str,
        start: usize,
    ) -> Option<Vec<PlannedPiece<'s>>> {
        let mut pieces = Vec::new();
        cut_planned_text(plan, |v| Some(*v), span, start, &mut |piece| {
            pieces.push(piece);
            Ok(())
        })?
        .expect("collecting pieces does not fail");
        Some(pieces)
    }

    /// #7346: a root object the scan declined only on its keys is cut into
    /// the runs its plan leaves alone and its planned fields, each a slice of
    /// its text, and printed from them: compact, and indented from the
    /// members' level, the repeated key in its first place with its last
    /// value. A plan whose spans are not where the object's members are (here,
    /// the wrong start) cuts nothing, and `stream_scanned_repeats` then writes
    /// nothing and leaves the object to the walk.
    #[test]
    fn repeated_keys_print_from_their_text_7346() {
        let doc = String::from_utf8(wide_canonical_object(40, Some((3, 30)))).unwrap();
        let doc = doc.replacen(":30,", ":[30,{\"v\":[]}],", 1);
        let index = JsonIndex::build(doc.as_bytes());
        let root = index.root(doc.as_bytes());
        let CanonicalSpan::RepeatedKeys {
            start,
            span,
            shared,
        } = root.canonical_compact_scan(0)
        else {
            panic!("the scan declines on the keys alone"); // patchcov: coverage tolerate-line reason="the failure message for the assertion this #7346 test exists to make"
        };
        assert_eq!((start, span), (0, doc.as_str()));
        let StandardJson::Object(fields) = root.value() else {
            panic!("an object"); // patchcov: coverage tolerate-line reason="the failure message for the assertion this #7346 test exists to make"
        };
        let plan = text_repeat_plan(span, start, &shared).plan().unwrap();
        let run = |from: usize, to: usize| {
            (from..to)
                .map(|i| format!("\"k{i:04}\":{i}"))
                .collect::<Vec<_>>()
                .join(",")
        };
        let (head, middle, tail) = (run(0, 3), run(4, 30), run(31, 40));
        assert_eq!(
            planned_pieces(&plan, span, 0),
            Some(vec![
                PlannedPiece::Open,
                PlannedPiece::Run(&head),
                PlannedPiece::Field {
                    key: "\"k0003\"",
                    value: "[30,{\"v\":[]}]",
                },
                PlannedPiece::Run(&middle),
                PlannedPiece::Run(&tail),
                PlannedPiece::Close,
            ])
        );
        assert_eq!(planned_pieces(&plan, span, 1), None);
        assert_eq!(planned_pieces(&plan, &span[1..], 0), None);
        for indent in [
            IndentSpec::COMPACT,
            IndentSpec {
                width: 2,
                unit: ' ',
            },
        ] {
            // A misplaced span is written by the plan's walk instead, as the
            // object arm writes it.
            let mut walked = String::new();
            let text = RepeatText::whole(span, 1);
            stream_scanned_repeats(
                &mut walked,
                fields,
                text,
                &shared,
                indent,
                JsonConvention::JqCompat,
            )
            .unwrap();
            let mut rerender = String::new();
            stream_json_pretty(
                &mut rerender,
                root.value(),
                0,
                indent.width,
                indent.unit,
                false,
                JsonConvention::JqCompat,
                0,
            )
            .unwrap();
            assert_eq!(walked, rerender);
        }

        let print = |indent| {
            let mut out = String::new();
            root.stream_json(&mut out, indent, false, JsonConvention::JqCompat)
                .unwrap();
            out
        };
        assert_eq!(
            print(IndentSpec::COMPACT),
            format!("{{{head},\"k0003\":[30,{{\"v\":[]}}],{middle},{tail}}}")
        );
        let pretty = print(IndentSpec {
            width: 2,
            unit: ' ',
        });
        let lines = pretty.lines().collect::<Vec<_>>();
        // 39 fields, the braces, and five more lines for the nested value.
        assert_eq!(lines.len(), 39 + 2 + 5);
        assert_eq!(&lines[..3], ["{", "  \"k0000\": 0,", "  \"k0001\": 1,"]);
        assert_eq!(
            &lines[4..11],
            [
                "  \"k0003\": [",
                "    30,",
                "    {",
                "      \"v\": []",
                "    }",
                "  ],",
                "  \"k0004\": 4,",
            ]
        );
        assert_eq!(&lines[lines.len() - 2..], ["  \"k0039\": 39", "}"]);
    }

    /// #7346: a writer that fails part-way stops the text print with its
    /// error, where the cut stopped, rather than reading as a misplaced span
    /// (which would send the object to the walk and print it a second time).
    #[test]
    fn a_failing_writer_stops_the_text_print_7346() {
        /// Fails once it has been given `budget` bytes.
        struct Failing(usize);
        impl core::fmt::Write for Failing {
            fn write_str(&mut self, s: &str) -> core::fmt::Result {
                self.0 = self.0.checked_sub(s.len()).ok_or(core::fmt::Error)?;
                Ok(())
            }
        }
        // Several chunks long, so that the writer fails part-way through the
        // cut: [`ChunkedWrite`] passes the first chunk on, then fails the next.
        let doc = String::from_utf8(wide_canonical_object(20_000, Some((3, 19_990)))).unwrap();
        assert!(doc.len() > 3 * CHUNKED_WRITE_BYTES);
        let index = JsonIndex::build(doc.as_bytes());
        let root = index.root(doc.as_bytes());
        let CanonicalSpan::RepeatedKeys {
            start,
            span,
            shared,
        } = root.canonical_compact_scan(0)
        else {
            panic!("the scan declines on the keys alone"); // patchcov: coverage tolerate-line reason="the failure message for the assertion this #7346 test exists to make"
        };
        let StandardJson::Object(fields) = root.value() else {
            panic!("an object"); // patchcov: coverage tolerate-line reason="the failure message for the assertion this #7346 test exists to make"
        };
        let plan = text_repeat_plan(span, start, &shared).plan().unwrap();
        // The cut stops at the planned field and answers the writer's error.
        let mut seen = Vec::new();
        let cut = cut_planned_text(&plan, |v| Some(*v), span, start, &mut |piece| {
            seen.push(piece);
            match piece {
                PlannedPiece::Field { .. } => Err(core::fmt::Error),
                _ => Ok(()),
            }
        });
        assert_eq!(cut, Some(Err(core::fmt::Error)));
        assert!(matches!(seen.last(), Some(PlannedPiece::Field { .. })));
        assert_eq!(seen.len(), 3, "the brace, the first run and the field");
        for indent in [
            IndentSpec::COMPACT,
            IndentSpec {
                width: 2,
                unit: ' ',
            },
        ] {
            let failed = stream_scanned_repeats(
                &mut Failing(CHUNKED_WRITE_BYTES + CHUNKED_WRITE_BYTES / 2),
                fields,
                RepeatText::whole(span, start),
                &shared,
                indent,
                JsonConvention::JqCompat,
            );
            assert!(matches!(failed, Err(StreamFailure::Fmt)), "{indent:?}");
        }
    }

    /// #7346: a plan where more than half the fields have a step keeps no
    /// spans (`StepSpan::DENSE_FROM`), so the object is written by the plan's
    /// walk, as the re-render writes it; below that share the plan keeps them,
    /// and the object is printed from its text.
    #[test]
    fn dense_repeats_print_by_the_walk_7346() {
        let plan_of = |doc: &str, want_spans: bool| {
            let index = JsonIndex::build(doc.as_bytes());
            let root = index.root(doc.as_bytes());
            let CanonicalSpan::RepeatedKeys {
                start,
                span,
                shared,
            } = root.canonical_compact_scan(0)
            else {
                panic!("the scan declines on the keys alone"); // patchcov: coverage tolerate-line reason="the failure message for the assertion this #7346 test exists to make"
            };
            let StandardJson::Object(fields) = root.value() else {
                panic!("an object"); // patchcov: coverage tolerate-line reason="the failure message for the assertion this #7346 test exists to make"
            };
            assert!(repeat_plan_from_shared(&fields, &shared, raw_key_hash, None).is_some());
            // The walk that records spans gives them up on the same objects.
            let spanned = repeat_plan_from_shared(
                &fields,
                &shared,
                raw_key_hash,
                Some(JsonCursor::text_position),
            )
            .unwrap();
            assert_eq!(
                spanned.spanned_steps().is_some(),
                want_spans,
                "{}",
                doc.len()
            );
            let walked = repeat_plan_from_shared(&fields, &shared, raw_key_hash, None).unwrap();
            assert_eq!(
                fates_are_dense(&walked_plan_fates(walked, &fields)),
                !want_spans,
                "{}",
                doc.len()
            );
            // #7350: a dense object says so, so the walk does not record spans.
            match text_repeat_plan(span, start, &shared) {
                TextPlan::Plan(plan) => {
                    assert!(want_spans, "{}", doc.len());
                    assert_eq!(plan.spans(), spanned.spans());
                }
                TextPlan::Dense => assert!(!want_spans, "{}", doc.len()),
                TextPlan::NoPlan | TextPlan::Walk => panic!("{}", doc.len()), // patchcov: coverage tolerate-line reason="the failure message for the assertion this #7350 test exists to make"
            }
            for indent in [
                IndentSpec::COMPACT,
                IndentSpec {
                    width: 2,
                    unit: ' ',
                },
            ] {
                let mut printed = String::new();
                root.stream_json(&mut printed, indent, false, JsonConvention::JqCompat)
                    .unwrap();
                let mut rerender = String::new();
                stream_json_pretty(
                    &mut rerender,
                    root.value(),
                    0,
                    indent.width,
                    indent.unit,
                    false,
                    JsonConvention::JqCompat,
                    0,
                )
                .unwrap();
                assert_eq!(printed, rerender);
            }
        };
        // `unique` keys, `repeats` fields of one key, then `after` more keys.
        let object = |unique: usize, repeats: usize, after: usize| {
            let mut fields: Vec<String> = (0..unique).map(|i| format!("\"u{i}\":{i}")).collect();
            fields.extend((0..repeats).map(|i| format!("\"x\":{i}")));
            fields.extend((0..after).map(|i| format!("\"v{i}\":{i}")));
            format!("{{{}}}", fields.join(","))
        };
        let floor = StepSpan::DENSE_FROM;
        // Over half the fields: given up during the walk once past the floor,
        // and at its end under it.
        plan_of(&object(20, floor + 10, 0), false);
        plan_of(&object(20, 3000, 0), false);
        // Under half: spans, past the floor and under it.
        plan_of(&object(floor + 20, floor + 10, 0), true);
        plan_of(&object(600, 500, 0), true);
        // Repeats clustered in the first fields, under the floor, of an object
        // they are a small share of: the walk's running share is past half
        // while it is among them, and the spans are kept.
        plan_of(&object(20, 3000, 30_000), true);
    }

    /// #7346: the compact text print passes each long run on in one write,
    /// where gathering it would hold most of the object (100 MB more at the
    /// peak of `-c .` over the 100 MB `wide` object, before #7356 let a long
    /// write through [`ChunkedWrite`]); the indented one is passed on a chunk at
    /// a time.
    /// The last key repeats the first, so one run holds nearly all of it.
    #[test]
    fn repeated_keys_text_print_write_sizes_7346() {
        /// Every write's length, and the text.
        #[derive(Default)]
        struct Writes(Vec<usize>, String);
        impl core::fmt::Write for Writes {
            fn write_str(&mut self, s: &str) -> core::fmt::Result {
                self.0.push(s.len());
                self.1.push_str(s);
                Ok(())
            }
        }
        let keys = 20_000;
        let doc = String::from_utf8(wide_canonical_object(keys, Some((0, keys - 1)))).unwrap();
        assert!(doc.len() > 2 * CHUNKED_WRITE_BYTES);
        let index = JsonIndex::build(doc.as_bytes());
        let root = index.root(doc.as_bytes());
        assert!(matches!(
            root.canonical_compact_scan(0),
            CanonicalSpan::RepeatedKeys { .. }
        ));
        let mut compact = Writes::default();
        root.stream_json(
            &mut compact,
            IndentSpec::COMPACT,
            false,
            JsonConvention::JqCompat,
        )
        .unwrap();
        assert_eq!(
            compact.1,
            wide_object_expectation(keys, Some((0, keys - 1)))
        );
        assert!(
            compact.0[0] < CHUNKED_WRITE_BYTES,
            "the brace and the planned first field go on before the run, not with it"
        );
        assert!(compact.0.iter().any(|&n| n > 2 * CHUNKED_WRITE_BYTES));
        let mut pretty = Writes::default();
        let two = IndentSpec {
            width: 2,
            unit: ' ',
        };
        root.stream_json(&mut pretty, two, false, JsonConvention::JqCompat)
            .unwrap();
        assert!(pretty.0.len() > 2);
        assert!(pretty.0.iter().all(|&n| n < 2 * CHUNKED_WRITE_BYTES));

        // #7366's nested object is written from its text too, with the text
        // around it, where the walk would have buffered the whole value for
        // one write.
        let nested = format!("[0,{{\"a\":{doc}}},[1]]");
        let index = JsonIndex::build(nested.as_bytes());
        let root = index.root(nested.as_bytes());
        assert!(matches!(
            root.canonical_compact_scan(0),
            CanonicalSpan::NestedRepeatedKeys { .. }
        ));
        for indent in [IndentSpec::COMPACT, two] {
            let mut printed = Writes::default();
            root.stream_json(&mut printed, indent, false, JsonConvention::JqCompat)
                .unwrap();
            let mut rerender = String::new();
            stream_json_pretty(
                &mut rerender,
                root.value(),
                0,
                indent.width,
                indent.unit,
                false,
                JsonConvention::JqCompat,
                0,
            )
            .unwrap();
            assert_eq!(printed.1, rerender, "{indent:?}");
            assert!(printed.0.len() > 2, "{indent:?}: {:?}", printed.0.len());
            assert_eq!(
                printed.0.iter().any(|&n| n > 2 * CHUNKED_WRITE_BYTES),
                indent.is_compact(),
                "{indent:?}: only `-c` has a run of more than a chunk"
            );
        }
    }

    /// Same load-bearing direction as the corpus test above --
    /// `is_canonical_compact_jq_span(doc) == true ==>` `doc` round-trips
    /// through the re-render unchanged -- swept over multi-byte-UTF-8-and-
    /// edge-byte documents a seeded generator builds, rather than a
    /// hand-picked list (the fuzz-alphabet rule in the testing skill: a fuzzer
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
    ///
    /// Two properties, asserted separately because they are not the same
    /// claim (#2608 follow-up). The predicate's round-trip direction is
    /// conditioned on the document being valid UTF-8, since the scan now
    /// certifies "canonical *given* valid UTF-8" and leaves the rest to the
    /// gate's own `from_utf8`. The gate's own output-equals-the-re-render
    /// property is asserted for *every* generated document, valid UTF-8 or
    /// not -- that is the property the echo actually has to have, it
    /// subsumes the first, and the generator's raw-byte ingredients cover
    /// every family `from_utf8` rejects (bare continuation, truncated lead,
    /// overlong, UTF-8-encoded surrogate, past U+10FFFF, `0xFF`). It is
    /// also what drives the branch's `debug_assert_eq!` that the scan and
    /// `text_range` agree on where the value ends.
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
        // Every family `core::str::from_utf8` rejects, since #2608's scanner
        // no longer decodes sequences itself and the gate's single
        // `from_utf8` is now the only thing standing between an
        // invalid-UTF-8 span and a verbatim echo.
        const RAW_BYTE_INGREDIENTS: &[&[u8]] = &[
            &[0x80],                   // bare continuation byte
            &[0xC2],                   // truncated 2-byte lead
            &[0xE0, 0xA0],             // truncated 3-byte lead
            &[0xFF],                   // never legal anywhere in UTF-8
            &[0xE0, 0x80],             // overlong 3-byte prefix
            &[0xC0, 0xAF],             // overlong encoding of `/`
            &[0xED, 0xA0, 0x80],       // UTF-8-encoded lone surrogate (D800)
            &[0xF5, 0x80, 0x80, 0x80], // past U+10FFFF
        ];

        /// How many of `STRING_INGREDIENTS`'s entries, from the front,
        /// are canonical string *content* -- the list above is ordered
        /// canonical-first on purpose, and the sweep below pins that
        /// boundary rather than trusting it. `gen_canonical_string` draws
        /// from this prefix so that a wide object can actually certify.
        const CANONICAL_STRINGS: usize = 17;

        /// The same, for `NUMBER_INGREDIENTS` below.
        const CANONICAL_NUMBERS: usize = 11;

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

        /// `wide` counts the objects this call tree drew wider than
        /// `PAIRWISE_SPAN_SCAN_LIMIT`, so the sweep below can assert it
        /// actually reached `scan_canonical_object`'s table tier instead
        /// of taking it on trust (the testing skill: a fuzz route's weight is
        /// part of the claim).
        fn gen_value(rng: &mut ChaCha8Rng, depth: u32, out: &mut Vec<u8>, wide: &mut usize) {
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
                gen_object(rng, depth, out, wide);
            } else {
                gen_array(rng, depth, out, wide);
            }
        }

        fn gen_array(rng: &mut ChaCha8Rng, depth: u32, out: &mut Vec<u8>, wide: &mut usize) {
            out.push(b'[');
            let n = rng.random_range(0..4);
            for i in 0..n {
                if i > 0 {
                    out.push(b',');
                }
                gen_value(rng, depth - 1, out, wide);
            }
            out.push(b']');
        }

        /// One canonical string, drawn from the canonical prefix of
        /// `STRING_INGREDIENTS` and opened with a fixed-width `f000`
        /// stamp so that two calls with different `stamp`s are never
        /// byte-equal by accident (#2608 review).
        ///
        /// No ingredient begins with a digit, so the stamp cannot be
        /// absorbed into a neighbour's content and distinct stamps always
        /// give distinct spans. That matters for the keys of a wide
        /// object: a quarter of the draws here are the empty content, so
        /// an unstamped 20-key object would almost always repeat a key
        /// and decline, turning the second-tier *accept* sweep vacuous.
        fn gen_canonical_string(rng: &mut ChaCha8Rng, stamp: usize, out: &mut Vec<u8>) {
            out.push(b'"');
            out.extend_from_slice(format!("f{stamp:03}").as_bytes());
            let picks = rng.random_range(0..4);
            for _ in 0..picks {
                out.extend_from_slice(STRING_INGREDIENTS[rng.random_range(0..CANONICAL_STRINGS)]);
            }
            out.push(b'"');
        }

        /// One canonical scalar value, deliberately never a container:
        /// the point of the wide-object arm below is the key tiers, and
        /// recursing at that width would multiply the document size at
        /// every level of nesting.
        fn gen_canonical_scalar(rng: &mut ChaCha8Rng, out: &mut Vec<u8>) {
            match rng.random_range(0..5) {
                0 => gen_canonical_string(rng, 0, out),
                1 => out.extend_from_slice(b"true"),
                2 => out.extend_from_slice(b"false"),
                3 => out.extend_from_slice(b"null"),
                _ => out.extend_from_slice(
                    NUMBER_INGREDIENTS[rng.random_range(0..CANONICAL_NUMBERS)].as_bytes(),
                ),
            }
        }

        fn gen_object(rng: &mut ChaCha8Rng, depth: u32, out: &mut Vec<u8>, wide: &mut usize) {
            out.push(b'{');
            // Mostly narrow, but one object in seven is wider than
            // `PAIRWISE_SPAN_SCAN_LIMIT` (#2608 review). Without this the
            // generator never left `scan_canonical_object`'s pairwise
            // tier, so neither the `KeyHashes` seeding nor the table tier
            // -- nor a duplicate straddling the boundary, which the
            // `dup_key` draw below now reaches on its own -- was ever
            // swept here. Kept rare, and drawn just past the limit rather
            // than far past it, because the document grows multiplicatively
            // with the width at every level of nesting.
            let widened = rng.random_bool(0.15);
            if widened {
                *wide += 1;
            }
            let n = if widened {
                rng.random_range(17..24)
            } else {
                rng.random_range(0..4)
            };
            // **Width alone does not reach the second tier.** The scan
            // stops at the first key or value it cannot certify, and a
            // field drawn from the full ingredient lists is canonical only
            // about 60% of the time -- so a 20-field object built that way
            // survives to its 17th key roughly once in 30,000, and neither
            // the `KeyHashes` seeding nor the table ever runs. Most wide
            // objects therefore draw canonical-only content; the rest are
            // left alone so the decline path is swept at width too. (The
            // testing skill's rule that a fuzz route's weight is part of the
            // claim: the assertions at the end of this test pin both.)
            let canonical_content = widened && rng.random_bool(0.8);
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
                    if canonical_content {
                        gen_canonical_string(rng, i, out);
                    } else {
                        gen_string(rng, out);
                    }
                    if i == 0 {
                        key_0 = out[key_start..].to_vec();
                    }
                }
                out.push(b':');
                if canonical_content {
                    gen_canonical_scalar(rng, out);
                } else {
                    gen_value(rng, depth - 1, out, wide);
                }
            }
            out.push(b'}');
        }

        // The canonical-prefix boundary both `CANONICAL_*` constants name,
        // pinned rather than assumed: reorder either ingredient list and
        // this fails here instead of silently making every wide object
        // decline again (and the whole second-tier sweep vacuous).
        for (k, ingredient) in STRING_INGREDIENTS.iter().enumerate() {
            assert_eq!(
                is_canonical_compact_jq_span(&json_string_token(ingredient)),
                k < CANONICAL_STRINGS,
                "string ingredient {k} is on the wrong side of CANONICAL_STRINGS: {ingredient:?}"
            );
        }
        for (k, ingredient) in NUMBER_INGREDIENTS.iter().enumerate() {
            assert_eq!(
                is_canonical_compact_jq_span(ingredient.as_bytes()),
                k < CANONICAL_NUMBERS,
                "number ingredient {k} is on the wrong side of CANONICAL_NUMBERS: {ingredient}"
            );
        }

        let mut mismatches = Vec::new();
        let mut gate_mismatches = Vec::new();
        let mut canonical_count = 0usize;
        let mut wide_documents = 0usize;
        let mut canonical_wide_documents = 0usize;
        const ITERATIONS: u64 = 2000;
        for seed in 0..ITERATIONS {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let mut doc = Vec::new();
            let mut wide = 0usize;
            gen_value(&mut rng, 3, &mut doc, &mut wide);
            if wide > 0 {
                wide_documents += 1;
            }

            let checker_says_canonical = is_canonical_compact_jq_span(&doc);
            let rerendered = render_compact_jq_by_rerender(&doc);
            let actually_round_trips = rerendered.as_deref() == Some(doc.as_slice());
            // `&& is_ok()`: the scan certifies "canonical *given* valid
            // UTF-8" and the gate's own `from_utf8` supplies the given
            // (#2608 follow-up), so the predicate alone makes no claim
            // about an invalid-UTF-8 document -- the gate check just below
            // is what covers those, and covers them more strongly.
            if checker_says_canonical && core::str::from_utf8(&doc).is_ok() {
                canonical_count += 1;
                if wide > 0 {
                    canonical_wide_documents += 1;
                }
                // One direction only -- see this test's own doc comment
                // for why the converse is not asserted here.
                if !actually_round_trips {
                    mismatches.push((seed, doc.clone()));
                }
            }
            // The whole echo *branch*, not just the predicate: it must
            // produce exactly what the fall-through re-render would, on
            // every generated document regardless of which side of the
            // gate it lands -- including every invalid-UTF-8 one, where
            // "the gate declines and the re-render answers" is the entire
            // contract. This is also what drives the branch's
            // `debug_assert_eq!` that the scan's end agrees with
            // `text_range`'s over the whole sweep (#2608 follow-up).
            if render_compact_jq_through_gate(&doc)
                != rerendered.map(|b| String::from_utf8_lossy(&b).into_owned())
            {
                gate_mismatches.push((seed, doc.clone()));
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
        assert!(
            gate_mismatches.is_empty(),
            "{} of {ITERATIONS} generated documents rendered differently through `stream_json`'s echo branch than through the re-render; first few: {:#?}",
            gate_mismatches.len(),
            gate_mismatches
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
        // The same sanity check for the *second* key tier, which the
        // narrow generator never reached (#2608 review): a document has to
        // carry an object wider than `PAIRWISE_SPAN_SCAN_LIMIT` before the
        // `KeyHashes` seeding and table paths run at all, and a document
        // that is also *certified* before either path can have said yes.
        assert!(
            wide_documents > ITERATIONS as usize / 20,
            "generator produced too few wide objects to exercise the table tier: {wide_documents}/{ITERATIONS}"
        );
        assert!(
            canonical_wide_documents > ITERATIONS as usize / 100,
            "too few generated documents both carried a wide object and were certified canonical -- the table tier's *accept* path is the one a decline can never exercise: {canonical_wide_documents}/{ITERATIONS}"
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
        assert!(!out.is_empty(), "{out:?}");
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
    /// #7449: a cursor's truthiness is `is_falsy`'s answer negated for every
    /// value the index reads, a container and a string with a bad escape
    /// included, and a decode failure for a token it could not read.
    #[test]
    fn test_cursor_truthiness_raises_only_for_an_unreadable_token_7449() {
        let json: &[u8] = br#"[null,false,true,0,"",[tru],{"a":nul},"\x",tru,nulll,x,1.2.3,-,1e]"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);
        let mut child = root.first_child();
        let mut verdicts = Vec::new();
        while let Some(c) = child {
            let verdict = c.truthiness();
            if let Ok(truthy) = verdict {
                assert_eq!(truthy, !c.is_falsy(), "{:?}", c.value());
            }
            verdicts.push(verdict.ok());
            child = c.next_sibling();
        }
        let readable = [Some(false), Some(false), Some(true), Some(true), Some(true)];
        let containers_and_strings = [Some(true), Some(true), Some(true)];
        let unreadable = [None; 6];
        let want: Vec<Option<bool>> = readable
            .into_iter()
            .chain(containers_and_strings)
            .chain(unreadable)
            .collect();
        assert_eq!(verdicts, want);
    }

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

    /// #7448: the three name walks raise on a one-member object whose lone
    /// child has no partner, for the name it holds and for any other. There
    /// is no last value behind it for the trailing-gap check to read, so this
    /// answered `Ok(None)`.
    #[test]
    fn test_find_walks_reject_a_lone_unpaired_child_7448() {
        for json in [
            br#"{"a":}"#.as_slice(),
            br#"{"a"}"#.as_slice(),
            br#"{"a":@}"#.as_slice(),
            br#"{"a":,}"#.as_slice(),
            br"{:1}".as_slice(),
        ] {
            let index = JsonIndex::build(json);
            let root = index.root(json);
            let StandardJson::Object(fields) = root.value() else {
                panic!("{json:?}: expected object");
            };
            assert!(fields.ends_unpaired(), "{json:?}");
            for name in ["a", "b"] {
                let find = fields.find(name).map(|v| v.is_some());
                let cursor = fields.find_cursor_counted(name).0.map(|c| c.is_some());
                let mut found = [None];
                let several = fields.find_cursors_counted(&[name], &mut found).0;
                for (walk, err) in [
                    ("find", find.err()),
                    ("find_cursor_counted", cursor.err()),
                    ("find_cursors_counted", several.err()),
                ] {
                    let err = err.unwrap_or_else(|| panic!("{json:?} {walk}({name}) answered"));
                    assert!(
                        err.message.contains("Invalid JSON text"),
                        "{json:?} {walk}: {}",
                        err.message
                    );
                }
            }
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

    /// #3035: `keyword_span` applies jq's *whole-token* rule (the run up to
    /// jq's literal boundary must be exactly the keyword), not
    /// longest-prefix acceptance. The boundary set is pinned by the same
    /// live sweep that built [`jq_literal_run_end`]'s class: `[true<suffix>]`
    /// against /usr/bin/jq 1.7.1 -- whitespace, `"`, `,`, `[`, `{`, `]`,
    /// `}`, `:` end the token, every other byte (digits, letters, all
    /// punctuation, NUL) continues it into one `Invalid literal` token.
    #[test]
    fn test_keyword_span_is_whole_token_not_prefix_3035() {
        // Exact token, followed by a jq terminator: the run is exactly the
        // keyword, so the span is the keyword's length.
        for (kw, term) in [
            (&b"true"[..], b' '),
            (&b"true"[..], b']'),
            (&b"true"[..], b'\n'),
            (&b"false"[..], b','),
            (&b"null"[..], b'}'),
            (&b"null"[..], b':'),
        ] {
            let doc = [kw, &[term][..]].concat();
            assert_eq!(
                keyword_span(&doc, 0, kw),
                Some(kw.len()),
                "{:?} + terminator {:?}",
                String::from_utf8_lossy(kw),
                term
            );
        }
        // Prefix-plus-suffix (`nullx`) and truncated (`nul`) tokens are not
        // the literal -- jq reports `Invalid literal` for both shapes.
        for doc in [
            &b"nullx"[..],
            &b"truex"[..],
            &b"falsey"[..],
            &b"null1"[..],
            &b"true1"[..],
            &b"false0"[..],
            &b"nul"[..],
            &b"tru"[..],
            &b"fals"[..],
            &b"nully"[..],
            &b"true!"[..],
            &b"false-"[..],
            &b"null+1"[..],
            &b"null.e"[..],
        ] {
            let kw: &[u8] = match doc[0] {
                b't' => b"true",
                b'f' => b"false",
                _ => b"null",
            };
            assert_eq!(
                keyword_span(doc, 0, kw),
                None,
                "{:?} must not resolve to the keyword",
                String::from_utf8_lossy(doc)
            );
        }
    }

    /// #3035: the nested-keyword decoders (`value_at`, and `text_range`
    /// through them) surface a decode error for a malformed keyword token
    /// instead of materializing the valid prefix out of it. The error value
    /// is how the evaluator's `type`/`map(type)`/`.[] | type` arms now raise
    /// the malformed-document error rather than answering `"error"`; the
    /// `None` `text_range` is what the pretty/compact writers decline.
    #[test]
    fn test_keyword_decoding_requires_the_whole_token_3035() {
        for doc in [
            &b"[nullx]"[..],
            &b"[truex]"[..],
            &b"[falsey]"[..],
            &b"[nul]"[..],
            &b"[tru]"[..],
            &b"[fals]"[..],
            &b"[null1]"[..],
        ] {
            let index = JsonIndex::build(doc);
            let root = index.root(doc);
            let elem = root.first_child().expect("array element cursor");
            assert!(
                matches!(elem.value(), StandardJson::Error(_)),
                "{:?} must decode to an error, not a bite-sized literal",
                String::from_utf8_lossy(doc)
            );
            assert_eq!(
                elem.text_range(),
                None,
                "{:?} must not report a raw span",
                String::from_utf8_lossy(doc)
            );
        }
    }

    /// #3035 (guard that decoy literals stay intact): a *valid* keyword must
    /// still decode and report its span, and a genuine `nan` still routes to
    /// the number path (`special_number_end`'s #2877 ordering kept `null`
    /// first) rather than being caught by the new keyword rule.
    #[test]
    fn test_keyword_decoding_keeps_valid_literals_and_nan_3035() {
        for doc in [&b"[null]"[..], &b"[true]"[..], &b"[false]"[..]] {
            let index = JsonIndex::build(doc);
            let root = index.root(doc);
            let elem = root.first_child().unwrap();
            assert!(!matches!(elem.value(), StandardJson::Error(_)));
            assert!(elem.text_range().is_some());
        }
        // `nan` shares its first byte with `null`; it must stay a number.
        let index = JsonIndex::build(b"[nan]");
        let root = index.root(b"[nan]");
        let elem = root.first_child().unwrap();
        assert!(matches!(elem.value(), StandardJson::Number(_)));
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

    /// The byte-at-a-time container walk `container_end` replaced (#4177),
    /// verbatim apart from the call it makes: the independent oracle the block
    /// walk is swept against.
    fn container_end_bytewise_reference(bytes: &[u8], pos: usize) -> Option<usize> {
        let open = bytes[pos];
        let close = if open == b'{' { b'}' } else { b']' };
        let mut depth = 1;
        let mut i = pos + 1;
        while i < bytes.len() && depth > 0 {
            match bytes[i] {
                b'"' => {
                    i = string_literal_end(bytes, i)?;
                    continue;
                }
                c if c == open => depth += 1,
                c if c == close => depth -= 1,
                _ => {}
            }
            i += 1;
        }
        if depth == 0 {
            Some(i)
        } else {
            None
        }
    }

    /// A string made of the container's own brackets (every byte a hit in the
    /// first single-mask version, #4177) ends where its closing quote does.
    #[test]
    fn test_container_end_ignores_brackets_inside_strings_4177() {
        for n in [1usize, 31, 32, 33, 63, 64, 65, 200] {
            let obj = format!("{{\"a\":\"{}\"}}", "{}".repeat(n));
            let arr = format!("[\"{}\", \"]\"]", "[]".repeat(n));
            for doc in [obj, arr] {
                let end = doc.len();
                let mut padded = doc.clone().into_bytes();
                padded.extend_from_slice(b"}]}]");
                assert_eq!(container_end(&padded, 0), Some(end), "{doc}");
                assert_eq!(container_end_bytewise_reference(&padded, 0), Some(end));
            }
        }
    }

    #[test]
    fn test_container_end_refuses_a_start_that_is_not_an_opener_4177() {
        assert_eq!(container_end(b"", 0), None);
        assert_eq!(container_end(b"{}", 2), None);
        assert_eq!(container_end(b"a]", 0), None);
        assert_eq!(container_end(b"\"{}\"", 0), None);
    }

    #[test]
    fn test_container_end_known_shapes_4177() {
        let cases: &[(&[u8], Option<usize>)] = &[
            (b"{}", Some(2)),
            (b"[]", Some(2)),
            (b"{} trailing", Some(2)),
            (br#"{"a":1}"#, Some(7)),
            (br#"{"a":{"b":[1,2]}}x"#, Some(17)),
            // Only this container's own pair moves the depth.
            (b"{]}", Some(3)),
            (b"[}]", Some(3)),
            (b"{{}", None),
            (b"[[]", None),
            // Brackets inside a string do not count.
            (br#"{"a":"}"}"#, Some(9)),
            (br#"["]"]"#, Some(5)),
            // An escaped quote does not end the string; an escaped backslash does not escape.
            (br#"{"a":"\"}"}"#, Some(11)),
            (br#"{"a":"\\"}"#, Some(10)),
            // A byte after `\` is skipped whatever it is -- even a control byte.
            (b"{\"a\":\"\\\t\"}", Some(10)),
            // A raw control character rejects the string, and so the container.
            (b"{\"a\":\"\t\"}", None),
            (b"{\"a\":\"\n\"}", None),
            // DEL is accepted, a control byte outside a string is whitespace.
            (b"{\"a\":\"\x7f\"}", Some(9)),
            (b"{\n\t\r\"a\" :\n 1}", Some(13)),
            (b"{\"a\":1\x00}", Some(8)),
            // Unterminated string / container / dangling backslash.
            (br#"{"a":"b"#, None),
            (br#"{"a":1"#, None),
            (br#"{"a":"b\"#, None),
            (b"{", None),
        ];
        for (doc, want) in cases {
            assert_eq!(
                container_end(doc, 0),
                *want,
                "{:?}",
                String::from_utf8_lossy(doc)
            );
            assert_eq!(
                container_end_bytewise_reference(doc, 0),
                *want,
                "reference disagrees with the pinned answer for {:?}",
                String::from_utf8_lossy(doc)
            );
        }
    }

    /// The block edges are the off-by-one sites: a `\` as the last byte of a
    /// block (its skip carries into the next), a string spanning blocks, a close
    /// on the last byte of a block and on the first of the next, and a partial
    /// tail.
    #[test]
    fn test_container_end_block_edges_4177() {
        for pad in 0..140usize {
            for body in [
                &br#"{"k":"v"}"#[..],
                br#"{"k":"a\"b"}"#,
                br#"{"k":"\\"}"#,
                br#"{"k":"}\\}"}"#,
                b"{\"k\":\"\t\"}",
                br#"{"k":[1,{"a":"]"}]}"#,
                b"[]",
                br#"["\\"]"#,
            ] {
                // Leading filler is whitespace inside a container so the walk
                // starts at byte 0 and the interesting bytes land at `pad`.
                let open = body[0];
                let mut doc = vec![open];
                doc.extend(core::iter::repeat_n(b' ', pad));
                doc.extend_from_slice(&body[1..]);
                for extra in [0usize, 1, 63, 64, 65] {
                    let mut d = doc.clone();
                    d.extend(core::iter::repeat_n(b'x', extra));
                    assert_eq!(
                        container_end(&d, 0),
                        container_end_bytewise_reference(&d, 0),
                        "pad {pad} extra {extra} body {:?}",
                        String::from_utf8_lossy(body)
                    );
                    // Truncated at every length near the container's end and
                    // inside the filler after it: the unterminated and
                    // partial-tail answers.
                    for cut in doc.len().saturating_sub(3).max(1)..=d.len() {
                        assert_eq!(
                            container_end(&d[..cut], 0),
                            container_end_bytewise_reference(&d[..cut], 0),
                            "cut {cut} pad {pad} extra {extra} body {:?}",
                            String::from_utf8_lossy(body)
                        );
                    }
                }
            }
        }
    }

    /// Seeded random documents over an alphabet made of the walk's own
    /// decision bytes, started at a random offset (so containers begin at every
    /// alignment) and inside a prefix that is itself arbitrary.
    #[test]
    fn test_container_end_matches_bytewise_reference_randomised_4177() {
        const ALPHABET: &[u8] = b"{}[]\"\\ab :,\t\n\x7f\x01\xc3\xa9";
        let mut state = 0x4177_4177_4177_4177u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for round in 0..40_000 {
            let lead = (next() % 70) as usize;
            let len = (next() % 220) as usize;
            let mut doc: Vec<u8> = (0..lead).map(|_| b'a' + (next() % 26) as u8).collect();
            doc.push(if next() % 2 == 0 { b'{' } else { b'[' });
            // Sparse rounds are mostly plain bytes (long runs between hits),
            // dense ones are all decision bytes.
            let sparse = round % 2 == 0;
            for _ in 0..len {
                let r = next();
                doc.push(if sparse && r % 6 != 0 {
                    b'a'
                } else {
                    ALPHABET[(r >> 8) as usize % ALPHABET.len()]
                });
            }
            assert_eq!(
                container_end(&doc, lead),
                container_end_bytewise_reference(&doc, lead),
                "round {round}: {doc:?} from {lead}"
            );
        }
    }

    /// A well-formed nested document with a long tail after it: the walk must
    /// stop at the matching close, not scan on, and report the same end the
    /// byte loop did.
    #[test]
    fn test_container_end_stops_at_the_matching_close_4177() {
        let mut doc =
            br#"{"users":[{"name":"a\"b","tags":["x","}"],"n":1.5},{"name":"c"}],"ok":true}"#
                .to_vec();
        let end = doc.len();
        doc.extend_from_slice(br#"{"next":"}}}}"}"#);
        assert_eq!(container_end(&doc, 0), Some(end));
        assert_eq!(container_end_bytewise_reference(&doc, 0), Some(end));
        assert_eq!(container_end(&doc, end), Some(doc.len()));
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
            // Bridge text: the only index the token decodes under (#3034).
            let index = JsonIndex::build_reindex(bytes);
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

    /// #7315: JSON's `valid_number_text` override answers what the trait default would: the raw
    /// span when it is an RFC 8259 number, `None` for every other span (lenient spellings, bridge
    /// tokens, bytes that are not UTF-8) and every non-number. The two definitions are kept apart
    /// only so JSON can skip a UTF-8 pass; this is the test that they agree.
    #[test]
    fn valid_number_text_is_number_text_filtered_by_is_valid_number_7315() {
        let mut docs: Vec<Vec<u8>> = [
            "0",
            "-0",
            "1.0",
            "-1.5e-3",
            "1e1000",
            "-1E+400",
            "123456789012345678901234",
            "0.30000000000000004",
            "007",
            "-007.5",
            ".5",
            "-.5",
            "5.",
            "1e",
            "1.e5",
            "+1",
            "0x10",
            "\"12\"",
            "null",
            "true",
            "[]",
            "{}",
        ]
        .iter()
        .map(|t| format!("[{t}]").into_bytes())
        .collect();
        docs.push(
            format!(
                "[{}]",
                crate::json::validate::computed_float_token(0.1 + 0.2)
            )
            .into_bytes(),
        );
        docs.push(b"[1\xff]".to_vec());
        docs.push(b"[\xff1]".to_vec());
        docs.push(b"[-\xc3\xa9]".to_vec());
        let (mut valid, mut refused_numbers) = (0usize, 0usize);
        for bytes in &docs {
            for index in [JsonIndex::build(bytes), JsonIndex::build_reindex(bytes)] {
                let root = index.root(bytes);
                let StandardJson::Array(items) = root.value() else {
                    panic!("expected an array: {:?}", String::from_utf8_lossy(bytes));
                };
                for value in items {
                    let expected = value
                        .number_text()
                        .filter(|text| crate::json::validate::is_valid_number(text.as_bytes()));
                    assert_eq!(
                        value.valid_number_text(),
                        expected,
                        "{:?}",
                        String::from_utf8_lossy(bytes)
                    );
                    if expected.is_some() {
                        valid += 1;
                    } else if value.number_text().is_some() {
                        refused_numbers += 1;
                    }
                }
            }
        }
        // Both answers are exercised: valid spans, and number spans the grammar refuses.
        assert!(
            valid >= 14 && refused_numbers >= 8,
            "{valid} valid, {refused_numbers} refused"
        );
    }

    /// #6275: a computed integer's token is one `Number` span the bridge's index decodes to the
    /// `i64` it names (`bridge_computed_int`) and to that value as a double (`bridge_value`,
    /// `as_f64`), while `as_i64` and `number_literal()` refuse it. A document holding the same
    /// bytes reads none of it, and a float token is no integer.
    #[test]
    fn json_number_decodes_the_computed_int_token_6275() {
        for n in [
            0i64,
            1,
            -1,
            7,
            -7,
            1000,
            1 << 53,
            -(1 << 53),
            i64::MAX,
            i64::MIN,
        ] {
            let token = crate::json::validate::computed_int_token(n);
            let json = format!("[{token}]");
            let bytes = json.as_bytes();
            assert_eq!(nested_number_span(bytes, 1), 1 + token.len());
            let index = JsonIndex::build_reindex(bytes);
            let root = index.root(bytes);
            let StandardJson::Array(mut items) = root.value() else {
                panic!("expected an array");
            };
            let StandardJson::Number(number) = items.next().expect("one element") else {
                panic!("expected a number");
            };
            assert_eq!(number.raw_bytes(), token.as_bytes());
            assert_eq!(number.bridge_computed_int(), Some(n), "{token}");
            assert_eq!(number.bridge_computed_float(), None, "{token}");
            assert_eq!(number.bridge_value(), Some(n as f64), "{token}");
            assert_eq!(number.as_f64(), Ok(n as f64), "{token}");
            assert_eq!(number.as_i64(), Err(JsonError::InvalidNumber), "{token}");
            let value: StandardJson<'_, Vec<u64>> = StandardJson::Number(number);
            assert_eq!(value.number_literal(), None, "{token}");
            assert_eq!(
                DocumentValue::bridge_computed_int(&value),
                Some(n),
                "{token}"
            );

            // The same bytes in a user document are no number at all (#3034).
            let document = JsonIndex::build(bytes);
            let root = document.root(bytes);
            let StandardJson::Array(mut items) = root.value() else {
                panic!("expected an array");
            };
            assert!(
                !matches!(items.next(), Some(StandardJson::Number(_))),
                "{token}"
            );
        }
        // A computed float's token is no integer, whatever it spells.
        let token = crate::json::validate::computed_float_token(2.0);
        let json = format!("[{token}]");
        let bytes = json.as_bytes();
        let index = JsonIndex::build_reindex(bytes);
        let root = index.root(bytes);
        let StandardJson::Array(mut items) = root.value() else {
            panic!("expected an array");
        };
        let StandardJson::Number(number) = items.next().expect("one element") else {
            panic!("expected a number");
        };
        assert_eq!(number.bridge_computed_int(), None);
        assert_eq!(number.bridge_computed_float(), Some(2.0));
    }

    /// #2802: the spelling token is one `Number` span the bridge's index decodes to its text
    /// (`bridge_spelling`, `number_literal`), which has no numeric value and no document
    /// address; a document spelling the same bytes decodes nothing and is no number (#3034).
    #[test]
    fn json_number_decodes_the_spelling_token_2802() {
        for spelling in ["~", "True", "0x1f", "+2.0", ".5", "007", "日本"] {
            let token = crate::json::validate::spelling_token(spelling);
            let json = format!("[{token}]");
            let bytes = json.as_bytes();
            assert_eq!(nested_number_span(bytes, 1), 1 + token.len());
            let index = JsonIndex::build_reindex(bytes);
            let root = index.root(bytes);
            let StandardJson::Array(mut items) = root.value() else {
                panic!("expected an array");
            };
            let StandardJson::Number(n) = items.next().expect("one element") else {
                panic!("expected a number");
            };
            assert_eq!(n.bridge_spelling().as_deref(), Some(spelling), "{token}");
            assert_eq!(n.bridge_value(), None, "{token}");
            assert_eq!(n.bridge_nonfinite(), None, "{token}");
            assert_eq!(n.bridge_nonfinite_literal(), None, "{token}");
            assert_eq!(n.token_address(), None, "{token}");
            // The number a spelled integer or float names, and nothing for a spelling that is no
            // number (#2802, #1356).
            let named = match crate::yaml::resolve_plain(spelling) {
                crate::yaml::ResolvedScalar::Int(i) => Some(i),
                _ => None,
            };
            let named_float = match spelling {
                "+2.0" => Some(2.0),
                ".5" => Some(0.5),
                _ => named.map(|i| i as f64),
            };
            assert_eq!(n.as_i64().ok(), named, "{token}");
            assert_eq!(n.as_f64().ok(), named_float, "{token}");
            let number: StandardJson<'_, Vec<u64>> = StandardJson::Number(n);
            assert_eq!(
                number.number_literal().as_deref(),
                Some(spelling),
                "{token}"
            );

            let user_index = JsonIndex::build(bytes);
            assert!(
                matches!(
                    user_index
                        .root(bytes)
                        .first_child()
                        .expect("one child")
                        .value(),
                    StandardJson::Error(MALFORMED_NUMBER)
                ),
                "{token} as a document"
            );
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
            ("nan0", None, None),
            ("sNaN0", None, None),
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
                        cursor.value() // patchcov: coverage tolerate-line reason="unreachable in a passing suite by design -- the failure message for the assertion this #2877 test exists to make"
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
                    child.value() // patchcov: coverage tolerate-line reason="unreachable in a passing suite by design -- the failure message for the assertion this #2877 test exists to make"
                );
                assert_eq!(child.text_range(), None, "{doc}: text_range");
            }
        }
        // `-` before a word is a run jq reads as one token (`-nope`, `+x`),
        // so the span is the whole run, not the sign alone (#5654); it is
        // not a number either way, so every decoder refuses it.
        assert_eq!(nested_number_span(b"[-nope]", 1), 6);
        assert_eq!(nested_number_span(b"[+x]", 1), 3);
        // A number with a byte glued on is one token too (#5654).
        assert_eq!(nested_number_span(b"[0x1,2]", 1), 4);
        assert_eq!(nested_number_span(b"[12x3]", 1), 5);
        assert_eq!(nested_number_span(b"[0.0x1]", 1), 6);
        assert_eq!(strict_number_end(b"0x1,", 0), None);
        assert_eq!(strict_number_end(b"12,", 0), Some(2));
        assert_eq!(strict_number_end(b"12", 0), Some(2));
        assert_eq!(scan_canonical_number(b"12x3", 0), None);
        assert_eq!(scan_canonical_number(b"12 ", 0), Some(2));
        // ... while a valid signed word is the whole word.
        assert_eq!(nested_number_span(b"[-inf]", 1), 5);
        assert_eq!(nested_number_span(b"[+nan]", 1), 5);
        assert_eq!(nested_number_span(b"[sNaN0,1]", 1), 6);
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
            (b"NaN0,", 4),
            (b"sNaN0]", 5),
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
            // Bridge text: the only index the token decodes under (#3034).
            let index = JsonIndex::build_reindex(bytes);
            let root = index.root(bytes);
            let StandardJson::Number(n) = root.first_child().expect("one child").value() else {
                panic!("expected a number"); // patchcov: coverage tolerate-line reason="unreachable in a passing suite by design -- the failure message for the assertion this #2877 test exists to make"
            };
            match expected {
                None => assert!(n.as_f64().is_ok_and(f64::is_nan), "{token}"),
                Some(v) => assert_eq!(n.as_f64(), Ok(v), "{token}"),
            }
        }
    }

    /// #3034: the bridge's number tokens decode only under the index built
    /// over bridge text. The same bytes in a user document are an ordinary
    /// malformed span, read like their neighbour spellings (`9e999e998`,
    /// `1e0e1`): since #3222 that is an `Error` value, not a `Number` any
    /// reader could take a value out of.
    #[test]
    fn bridge_tokens_decode_only_under_the_bridge_index_3034() {
        let tokens = [
            crate::jq::OwnedValue::Float(f64::NAN).to_json_input_bridge(),
            crate::jq::OwnedValue::Float(f64::INFINITY).to_json_input_bridge(),
            crate::jq::OwnedValue::Float(f64::NEG_INFINITY).to_json_input_bridge(),
            crate::json::validate::computed_float_token(1.0),
            crate::json::validate::computed_float_token(-2.5),
        ];
        for token in &tokens {
            let json = format!("[{token}]");
            let bytes = json.as_bytes();
            let bridge_index = JsonIndex::build_reindex(bytes);
            let value = bridge_index
                .root(bytes)
                .first_child()
                .expect("one child")
                .value();
            let StandardJson::Number(n) = value else {
                panic!("expected a number"); // patchcov: coverage tolerate-line reason="failure message for the assertion this #3034 test exists to make"
            };
            assert!(
                n.bridge_value().is_some(),
                "{token} must decode as bridge text"
            );
            assert!(n.as_f64().is_ok(), "{token} must decode as bridge text");
            let user_index = JsonIndex::build(bytes);
            assert!(
                matches!(
                    user_index
                        .root(bytes)
                        .first_child()
                        .expect("one child")
                        .value(),
                    StandardJson::Error(MALFORMED_NUMBER)
                ),
                "{token} in a user document is a malformed number"
            );
        }
        // Not everything outside `[0-9.eE+-]` is a token: decNumber's words
        // are user text and read the same under either index (#2877).
        for word in ["nan", "-Infinity"] {
            let json = format!("[{word}]");
            let bytes = json.as_bytes();
            for index in [JsonIndex::build(bytes), JsonIndex::build_reindex(bytes)] {
                let StandardJson::Number(n) =
                    index.root(bytes).first_child().expect("one child").value()
                else {
                    panic!("expected a number"); // patchcov: coverage tolerate-line reason="failure message for the assertion this #3034 test exists to make"
                };
                assert!(n.as_f64().is_ok(), "{word}");
                assert_eq!(n.bridge_value(), None, "{word}");
            }
        }
    }

    /// #3222: a nested span that is not a number jq reads is an `Error`
    /// value, so every route that reads it raises, rather than a `Number`
    /// the funnels read as `null` and `type` as `"number"` (#966). Its
    /// siblings are untouched, and every lenient spelling jq accepts is
    /// still a `Number`.
    #[test]
    fn malformed_nested_number_is_an_error_value_3222() {
        for span in [
            "1.2.3",
            "1ee5",
            "1-2",
            "9e999e998",
            "+1.2.3",
            "-",
            "1e",
            "1.5e+",
        ] {
            for json in [format!("[{span},2]"), format!("{{\"a\":{span},\"b\":2}}")] {
                let bytes = json.as_bytes();
                let index = JsonIndex::build(bytes);
                let first = index.root(bytes).first_child().expect("a child");
                let value = if json.starts_with('{') {
                    first.next_sibling().expect("a value").value()
                } else {
                    first.value()
                };
                assert!(
                    matches!(value, StandardJson::Error(MALFORMED_NUMBER)),
                    "{json}: {value:?}"
                );
            }
        }
        for span in [
            "1",
            "-0",
            "007",
            "1.500",
            ".5",
            "-.5",
            "1.",
            "1.e5",
            "+1",
            "+.5",
            "nan",
            "-Infinity",
        ] {
            let json = format!("[{span},2]");
            let bytes = json.as_bytes();
            let index = JsonIndex::build(bytes);
            let value = index.root(bytes).first_child().expect("a child").value();
            assert!(
                matches!(value, StandardJson::Number(_)),
                "{json}: {value:?}"
            );
        }
    }

    /// #3222: `strict_number_end` is `JsonCursor::number_at`'s shortcut,
    /// taken without the greedy span. It may only ever accept a span the
    /// full decision also accepts, and the end it reports must be the one
    /// the greedy span finds, since `raw_bytes` trusts it. Every string over
    /// the greedy class up to five bytes, followed by a delimiter.
    #[test]
    fn strict_number_fast_path_implies_the_full_decision_3222() {
        const CLASS: &[u8] = b"019.eE+-";
        let mut frontier: Vec<Vec<u8>> = vec![Vec::new()];
        for _ in 0..5 {
            let mut next = Vec::new();
            for prefix in &frontier {
                for &b in CLASS {
                    let mut span = prefix.clone();
                    span.push(b);
                    let mut text = span.clone();
                    text.push(b',');
                    if let Some(end) = strict_number_end(&text, 0) {
                        assert_eq!(end, span.len(), "{:?}", String::from_utf8_lossy(&span));
                        assert_eq!(
                            nested_number_span(&text, 0),
                            span.len(),
                            "{:?}",
                            String::from_utf8_lossy(&span) // patchcov: coverage tolerate-line reason="unreachable in a passing suite by design -- this is a panic-message format argument for the #3222 sweep's own assertion, only evaluated if the assert's own condition is false (#3222)"
                        );
                        assert!(
                            crate::json::validate::number_span_decodes(&span),
                            "{:?}",
                            String::from_utf8_lossy(&span) // patchcov: coverage tolerate-line reason="unreachable in a passing suite by design -- see the assert_eq! format argument above, same sweep (#3222)"
                        );
                    }
                    next.push(span);
                }
            }
            frontier = next;
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
            cursors.len() // patchcov: coverage tolerate-line reason="unreachable in a passing suite by design -- this is a panic-message format argument for the #2072 pin itself, only evaluated if the assert's own condition is false (#2072)"
        );

        for c in &cursors {
            let id = c.node_id();
            let back = c
                .at_node_id(id)
                .expect("a node's own id always names a node in its own document");
            assert!(
                back.same_node(c),
                "id {id} re-resolved to bp {} instead of {}",
                back.node_id(), // patchcov: coverage tolerate-line reason="unreachable in a passing suite by design -- this is a panic-message format argument for the #2072 pin itself, only evaluated if the assert's own condition is false (#2072)"
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
                c.node_id() // patchcov: coverage tolerate-line reason="unreachable in a passing suite by design -- this is a panic-message format argument for the #2072 pin itself, only evaluated if the assert's own condition is false (#2072)"
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

    /// #3180: a list opened on an *empty* container names that container
    /// through `whole_container_cursor` -- the tagged slot -- while every
    /// navigation reader still sees an empty list. A list that is merely
    /// *exhausted* or partly consumed names nothing: it is a suffix, not the
    /// container.
    #[cfg(target_pointer_width = "64")]
    #[test]
    fn whole_container_cursor_names_an_empty_container_3180() {
        fn fields(v: StandardJson<'_>) -> JsonFields<'_> {
            match v {
                StandardJson::Object(f) => f,
                _ => panic!("not an object"), // patchcov: coverage tolerate-line reason="unreachable in a passing suite: every caller passes an object (#3180)"
            }
        }
        fn elements(v: StandardJson<'_>) -> JsonElements<'_> {
            match v {
                StandardJson::Array(e) => e,
                _ => panic!("not an array"), // patchcov: coverage tolerate-line reason="unreachable in a passing suite: every caller passes an array (#3180)"
            }
        }

        // Empty roots.
        let json: &[u8] = b"{}";
        let index = JsonIndex::build(json);
        let root = index.root(json);
        let f = fields(root.value());
        assert_eq!(
            f.whole_container_cursor().map(|c| c.bp_position()),
            Some(root.bp_position())
        );
        assert!(f.is_empty());
        assert!(f.uncons().is_none());
        assert!(!f.ends_unpaired());
        assert!(f.unpaired_tail().is_none());
        assert!(matches!(f.find("a"), Ok(None)));
        assert_eq!(f.find_cursor("a").map(|c| c.is_some()), Ok(false));
        assert_eq!(f.count(), 0);
        assert_eq!(f.document_text(), Some(json));

        let json: &[u8] = b"[]";
        let index = JsonIndex::build(json);
        let root = index.root(json);
        let e = elements(root.value());
        assert_eq!(
            e.whole_container_cursor().map(|c| c.bp_position()),
            Some(root.bp_position())
        );
        assert!(e.is_empty());
        assert!(e.uncons().is_none());
        assert!(e.uncons_cursor().is_none());
        assert!(e.get(0).is_none());
        assert!(e.get_fast(0).is_none());
        assert_eq!(e.count(), 0);
        assert_eq!(e.document_text(), Some(json));

        // Nested empty containers: the node named is the child's, not the root's.
        let json: &[u8] = br#"{"a":{},"b":[]}"#;
        let index = JsonIndex::build(json);
        let root = index.root(json);
        let top = fields(root.value());
        let a = top.find_cursor("a").unwrap().unwrap();
        let b = top.find_cursor("b").unwrap().unwrap();
        assert_ne!(a.bp_position(), root.bp_position());
        let fa = fields(a.value());
        assert_eq!(
            fa.whole_container_cursor().map(|c| c.bp_position()),
            Some(a.bp_position())
        );
        assert!(fa.is_empty());
        let eb = elements(b.value());
        assert_eq!(
            eb.whole_container_cursor().map(|c| c.bp_position()),
            Some(b.bp_position())
        );
        assert!(eb.is_empty());
        // The non-empty root still answers through the first-child hop.
        assert_eq!(
            top.whole_container_cursor().map(|c| c.bp_position()),
            Some(root.bp_position())
        );

        // Exhausted and partly consumed lists are suffixes: no node.
        let json: &[u8] = br#"{"a":1}"#;
        let index = JsonIndex::build(json);
        let (_, rest) = fields(index.root(json).value()).uncons().unwrap();
        assert!(rest.is_empty());
        assert!(rest.whole_container_cursor().is_none());
        assert!(rest.document_text().is_none());
        let json: &[u8] = b"[1,2]";
        let index = JsonIndex::build(json);
        let (_, rest) = elements(index.root(json).value()).uncons().unwrap();
        assert!(!rest.is_empty());
        assert!(rest.whole_container_cursor().is_none());
        let (_, rest) = rest.uncons().unwrap();
        assert!(rest.is_empty());
        assert!(rest.whole_container_cursor().is_none());
    }

    /// #3180: the empty-container tag never shows in `{:?}` output -- the
    /// derived impl would have printed the tagged `bp_pos` as a huge number.
    #[cfg(target_pointer_width = "64")]
    #[test]
    fn empty_container_debug_hides_the_tag_3180() {
        let tag = EMPTY_CONTAINER_TAG.to_string();
        for json in [&b"{}"[..], b"[]", b"{\"a\":1}", b"[1]"] {
            let index = JsonIndex::build(json);
            let shown = format!("{:?}", index.root(json).value());
            assert!(!shown.contains(&tag), "{shown}"); // patchcov: coverage tolerate-line reason="unreachable in a passing suite by design -- panic-message format argument (#3180)"
            let empty = json.len() == 2;
            assert_eq!(shown.contains("empty_container"), empty, "{shown}");
        }
    }

    /// #3180: the tag lives in `bp_pos`'s top bit precisely so the list
    /// types do not grow -- they are `Copy` and on the hottest JSON
    /// iteration path.
    #[cfg(target_pointer_width = "64")]
    #[test]
    fn list_types_stay_one_cursor_wide_3180() {
        use core::mem::size_of;
        assert_eq!(size_of::<JsonCursor<'_>>(), 32);
        assert_eq!(size_of::<Option<JsonCursor<'_>>>(), 32);
        assert_eq!(size_of::<JsonFields<'_>>(), 32);
        assert_eq!(size_of::<JsonElements<'_>>(), 32);
        assert_eq!(size_of::<StandardJson<'_>>(), 40);
    }

    /// `key_raw_unescaped_with_end` (#3343) answers the span and where it
    /// ends from one scan, and only where the two-call path would agree:
    /// an unterminated span has no closing quote to end at (`find_end` stops at
    /// the text's end, #5541, so the two paths would agree by accident) and an
    /// escaped one is not its own decoded key, so both decline.
    #[test]
    fn key_raw_unescaped_with_end_declines_what_the_two_calls_would_split_3343() {
        use crate::jq::document::DocumentValue;
        fn key(text: &[u8]) -> StandardJson<'_> {
            StandardJson::String(JsonString { text, start: 1 })
        }

        let closed = key(br#"{"abc":1}"#);
        assert_eq!(
            closed.key_raw_unescaped_with_end(),
            Some((&b"abc"[..], 6)),
            "ends one past the closing quote"
        );
        assert_eq!(closed.text_end(), Some(6));

        let open = key(br#"{"abc"#);
        assert!(
            open.key_raw_unescaped().is_some(),
            "the old raw-span probe still hashes an unterminated span"
        );
        assert_eq!(open.key_raw_unescaped_with_end(), None);

        assert_eq!(key(br#"{"a\nb":1}"#).key_raw_unescaped_with_end(), None);
        assert_eq!(
            key(br#"{"":1}"#).key_raw_unescaped_with_end(),
            Some((&b""[..], 3))
        );
    }

    /// #5541: a string that never closes ends at the end of the text, not one
    /// byte past it -- `raw_bytes` sliced `text[start..len + 1]` and panicked.
    /// Both ways of leaving the string unclosed are covered: running out of
    /// bytes, and a trailing backslash whose escaped byte is the one missing.
    #[test]
    fn unterminated_string_span_stops_at_end_of_text_5541() {
        for text in [
            &br#"["abc"#[..],
            &br#"["abc\"#[..],
            &br#"["abc\\"#[..],
            &b"["[..],
        ] {
            let s = JsonString { text, start: 1 };
            assert_eq!(s.end(), text.len(), "{:?}", core::str::from_utf8(text));
            assert_eq!(s.raw_bytes(), &text[1..]);
        }
        let closed = JsonString {
            text: br#"["abc"]"#,
            start: 1,
        };
        assert_eq!(closed.end(), 6);
        assert_eq!(closed.raw_bytes(), br#""abc""#);
    }

    /// #7260: the JSON lookups' own malformed-key test answers what the shared one does, for keys
    /// that decode, keys that do not (#1247), and keys the grammar never allowed (#1995).
    #[test]
    fn json_key_is_malformed_agrees_with_the_shared_predicate_7260() {
        use crate::jq::document::key_is_malformed;
        for text in [
            &br#"{"a":1}"#[..],
            br#"{"":1}"#,
            br#"{"a\u0062":1}"#,
            br#"{"\ud800":1}"#,
            br#"{"a\q":1}"#,
            b"{\"a\xff\":1}",
            br#"{"a":1,123:2}"#,
            br#"{"a":1,true:2}"#,
            br#"{"a":1,null:2}"#,
            br#"{"a":1,[1]:2}"#,
            br#"{"a":1,{}:2}"#,
        ] {
            let index = JsonIndex::build(text);
            let root = index.root(text);
            let StandardJson::Object(mut fields) = root.value() else {
                panic!("an object: {:?}", String::from_utf8_lossy(text));
            };
            while let Some((field, rest)) = fields.uncons() {
                let key = field.key();
                assert_eq!(
                    json_key_is_malformed(&key),
                    key_is_malformed(&key),
                    "{:?}",
                    String::from_utf8_lossy(text)
                );
                fields = rest;
            }
        }
    }

    /// #7260: `decodes_to` answers what decoding the key and comparing it answers, for every
    /// pairing of these key texts (escapes, invalid UTF-8 and escapes, unclosed strings, a
    /// multi-byte character split where the two differ) with these names.
    #[test]
    fn decodes_to_agrees_with_decoding_the_key_7260() {
        let keys: [&[u8]; 22] = [
            br#""""#,
            br#""a""#,
            br#""ab""#,
            br#""abc""#,
            br#""a\u0062""#,
            br#""\u0061b""#,
            br#""a\"b""#,
            br#""a\\b""#,
            br#""a\qb""#,
            br#""\ud800""#,
            "\"\u{e9}\"".as_bytes(),
            "\"\u{e8}\"".as_bytes(),
            br#""\u00e9""#,
            b"\"a\xffb\"",
            b"\"ab\xff\"",
            b"\"\xc3\"",
            br#""ab"#,
            br#""a"#,
            br#"""#,
            br#""a\"#,
            br#""ab\n""#,
            br#""name": 1"#,
        ];
        let names = [
            "", "a", "ab", "abc", "a\"b", "a\\b", "\u{e9}", "\u{e8}", "ab\n", "name", "b",
        ];
        for key in keys {
            let mut text = alloc::vec![b'{'];
            text.extend_from_slice(key);
            let s = JsonString {
                text: &text,
                start: 1,
            };
            for name in names {
                assert_eq!(
                    s.decodes_to(name),
                    s.as_str().is_ok_and(|k| k == name),
                    "key {:?}, name {name:?}",
                    String::from_utf8_lossy(key)
                );
            }
        }
    }

    /// #4217: `build_reindex_scalar_from` overwrites the buffers it is given.
    /// Parts full of stale bits, longer and shorter than the index being
    /// built, must give the index `build_reindex_scalar` makes -- interest
    /// bits, rank at every position, parentheses -- and `take_scalar_parts`
    /// must return the same buffers (not copies) for the next build.
    #[test]
    fn scalar_index_built_from_dirty_parts_matches_a_fresh_one_4217() {
        for len in [1usize, 2, 63, 64, 65, 128, 129, 1000] {
            for stale_words in [0usize, 1, 3, 40] {
                let dirty = ScalarIndexParts {
                    ib: alloc::vec![u64::MAX; stale_words],
                    ib_rank: alloc::vec![u32::MAX; stale_words + 5],
                    bp_words: alloc::vec![u64::MAX; stale_words],
                };
                let reused = JsonIndex::build_reindex_scalar_from(len, dirty);
                let fresh = JsonIndex::build_reindex_scalar(len);
                assert_eq!(
                    reused.ib(),
                    fresh.ib(),
                    "IB, len {len}, stale {stale_words}"
                );
                assert_eq!(reused.ib_len(), fresh.ib_len());
                for p in 0..=len + 64 {
                    assert_eq!(reused.ib_rank1(p), fresh.ib_rank1(p), "ib_rank1({p})");
                }
                assert_eq!(reused.bp().words(), fresh.bp().words());
                assert_eq!(reused.bp().len(), fresh.bp().len());
                assert!(format!("{reused:?}").contains("bridge_tokens: true"));
            }
        }

        let mut index = JsonIndex::build_reindex_scalar(200);
        let (ib_ptr, rank_ptr) = (index.ib().as_ptr(), index.ib_rank.as_ptr());
        let parts = index.take_scalar_parts();
        let again = JsonIndex::build_reindex_scalar_from(150, parts);
        assert_eq!(again.ib().as_ptr(), ib_ptr, "the IB buffer is reused");
        assert_eq!(
            again.ib_rank.as_ptr(),
            rank_ptr,
            "the rank buffer is reused"
        );
    }
}
