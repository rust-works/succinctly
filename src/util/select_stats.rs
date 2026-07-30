//! Scan-length instrumentation for the select word-scan loops (#40).
//!
//! Issue #40 asks whether the word-scan inside `select` is worth vectorising
//! (SIMD popcount several words, prefix-sum, skip to the crossing word). That
//! only pays off if the scans are actually *long*: SIMD setup cost cannot be
//! recovered when the loop typically exits after one or two words. Four prior
//! SIMD proposals in this crate (P2.8, P3, P5, P8) were rejected for exactly
//! that reason — a micro-benchmark win over input shapes real workloads do not
//! contain.
//!
//! So before writing any kernel, we measure: how many words does each scan
//! site actually traverse on real inputs? This module records that
//! distribution.
//!
//! # Zero cost when disabled
//!
//! The five instrumented scan loops call [`record`] only under
//! `#[cfg(feature = "select-stats")]`, so in normal builds the hot loops carry
//! no counter at all. This module itself is always compiled (it needs `std`
//! for its thread-local storage), which keeps its own logic covered by the
//! ordinary test run rather than only under a feature CI does not enable.
//!
//! # Usage
//!
//! ```text
//! cargo run --release --features cli,bench-runner,select-stats -- \
//!     dev select-stats --data-dir data/bench/corpus
//! ```

use core::cell::RefCell;

/// The select word-scan sites instrumented for #40.
///
/// Each variant is one of the five near-identical scalar scan loops found in
/// the crate. Recording them separately is the point of the exercise: they
/// differ enormously in heat, and a distribution pooled across all five would
/// hide that.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Site {
    /// `AdvancePositions::get_sequential` — per node on the yq streaming path.
    /// The only site expected to be hot.
    YamlAdvance,
    /// `WithSelect::select1` — YAML `find_bp_at_text_pos`, one call per locate.
    BpWithSelect,
    /// `BitVec::select1` — newline / line-start lookup in locate.
    BitVec,
    /// `EliasFano::select1` — library-only; no in-crate consumer.
    EliasFano,
    /// `SimpleJsonIndex::ib_select1` — unindexed O(n) scan; no in-crate consumer.
    SimpleJson,
    /// `JsonIndex::ib_select1_from` — hot (per value materialised), but *not* a
    /// word scan: gallop + binary search over the precomputed `ib_rank` array.
    /// Recorded in probes, not words, so the two kinds of cost can be compared.
    JsonIbSelectFrom,
    /// `YamlIndex::ib_select1_from` — the YAML twin of [`Site::JsonIbSelectFrom`].
    YamlIbSelectFrom,
}

impl Site {
    /// Total number of instrumented sites.
    pub const COUNT: usize = 7;

    /// Dense index into the per-site histogram array.
    #[inline]
    pub fn index(self) -> usize {
        match self {
            Self::YamlAdvance => 0,
            Self::BpWithSelect => 1,
            Self::BitVec => 2,
            Self::EliasFano => 3,
            Self::SimpleJson => 4,
            Self::JsonIbSelectFrom => 5,
            Self::YamlIbSelectFrom => 6,
        }
    }

    /// All sites, in histogram order.
    pub fn all() -> [Self; Self::COUNT] {
        [
            Self::YamlAdvance,
            Self::BpWithSelect,
            Self::BitVec,
            Self::EliasFano,
            Self::SimpleJson,
            Self::JsonIbSelectFrom,
            Self::YamlIbSelectFrom,
        ]
    }

    /// Human-readable name used in the report.
    pub fn name(self) -> &'static str {
        match self {
            Self::YamlAdvance => "yaml/advance_positions get_sequential",
            Self::BpWithSelect => "trees/bp WithSelect::select1",
            Self::BitVec => "bits/bitvec BitVec::select1",
            Self::EliasFano => "bits/elias_fano EliasFano::select1",
            Self::SimpleJson => "json/simple_light ib_select1",
            Self::JsonIbSelectFrom => "json/light ib_select1_from",
            Self::YamlIbSelectFrom => "yaml/index ib_select1_from",
        }
    }

    /// Whether this site is a word scan (the shape #40 proposes vectorising)
    /// or a rank-array search (where the popcount step is already precomputed).
    #[inline]
    pub fn is_word_scan(self) -> bool {
        !matches!(self, Self::JsonIbSelectFrom | Self::YamlIbSelectFrom)
    }

    /// Unit of the recorded quantity: words popcounted, or `ib_rank` probes.
    pub fn unit(self) -> &'static str {
        if self.is_word_scan() {
            "words"
        } else {
            "probes"
        }
    }
}

/// Number of exact histogram buckets: scan lengths `0..EXACT_BUCKETS` are
/// counted individually. Anything longer lands in the overflow bucket.
///
/// Sized well past the decision boundary rather than close to it. An earlier
/// 64-bucket version saturated: real YAML scans reach 296 words, so p90 pinned
/// to the ceiling and the distribution's shape was invisible. 512 is cheap
/// (4 KiB per site) and leaves headroom above the observed maximum.
pub const EXACT_BUCKETS: usize = 512;

/// Per-site scan-length histogram.
#[derive(Clone)]
pub struct Histogram {
    /// `buckets[n]` counts scans that traversed exactly `n` words.
    buckets: [u64; EXACT_BUCKETS],
    /// Scans that traversed `EXACT_BUCKETS` or more words.
    overflow: u64,
    /// Longest scan observed (exact, even when it lands in overflow).
    max: u64,
    /// Sum of all scan lengths, for the mean.
    total_words: u64,
    /// Number of scans recorded.
    calls: u64,
}

impl Default for Histogram {
    fn default() -> Self {
        Self {
            buckets: [0; EXACT_BUCKETS],
            overflow: 0,
            max: 0,
            total_words: 0,
            calls: 0,
        }
    }
}

impl Histogram {
    /// Record one scan that traversed `words` words.
    #[inline]
    fn record(&mut self, words: usize) {
        let w = words as u64;
        self.calls += 1;
        self.total_words += w;
        if w > self.max {
            self.max = w;
        }
        if words < EXACT_BUCKETS {
            self.buckets[words] += 1;
        } else {
            self.overflow += 1;
        }
    }

    /// Number of scans recorded.
    pub fn calls(&self) -> u64 {
        self.calls
    }

    /// Longest scan observed, in words.
    pub fn max(&self) -> u64 {
        self.max
    }

    /// Mean scan length in words, or `None` if nothing was recorded.
    pub fn mean(&self) -> Option<f64> {
        (self.calls > 0).then(|| self.total_words as f64 / self.calls as f64)
    }

    /// Nearest-rank percentile of the scan length, in words.
    ///
    /// Returns `None` when nothing was recorded. A result of
    /// `EXACT_BUCKETS` means "at least `EXACT_BUCKETS`" — the overflow bucket
    /// does not retain exact lengths, only [`Self::max`] does.
    pub fn percentile(&self, p: f64) -> Option<u64> {
        if self.calls == 0 {
            return None;
        }
        // Nearest-rank: smallest value whose cumulative count reaches ceil(p% * n).
        let rank = ((p / 100.0) * self.calls as f64).ceil().max(1.0) as u64;
        let mut cumulative = 0u64;
        for (words, &count) in self.buckets.iter().enumerate() {
            cumulative += count;
            if cumulative >= rank {
                return Some(words as u64);
            }
        }
        Some(EXACT_BUCKETS as u64)
    }

    /// Fraction of scans that traversed strictly fewer than `words` words.
    ///
    /// Counts *calls*, so it answers "how often is a scan too short for a SIMD
    /// block to help?".
    pub fn fraction_below(&self, words: usize) -> Option<f64> {
        if self.calls == 0 {
            return None;
        }
        let below: u64 = self.buckets[..words.min(EXACT_BUCKETS)].iter().sum();
        Some(below as f64 / self.calls as f64)
    }

    /// Fraction of *total words scanned* that occurs in scans of at least
    /// `words` words.
    ///
    /// This is the number the #40 decision actually turns on, and it differs
    /// sharply from [`Self::fraction_below`] when the distribution is bimodal.
    /// A workload where most *calls* are 1–2 words but most *work* happens in
    /// rare 300-word scans is a good SIMD target: the vector path handles the
    /// long tail that dominates runtime, while a scalar prologue keeps the
    /// short calls free. Counting calls alone would wrongly reject it.
    ///
    /// Samples in the overflow bucket contribute their exact length to the
    /// numerator via `total_words`, so the result stays correct above the
    /// bucket ceiling.
    pub fn work_fraction_at_or_above(&self, words: usize) -> Option<f64> {
        if self.calls == 0 || self.total_words == 0 {
            return None;
        }
        let below_words: u64 = self.buckets[..words.min(EXACT_BUCKETS)]
            .iter()
            .enumerate()
            .map(|(len, &count)| len as u64 * count)
            .sum();
        Some((self.total_words - below_words) as f64 / self.total_words as f64)
    }

    /// Total words popcounted across all recorded scans.
    pub fn total_words(&self) -> u64 {
        self.total_words
    }

    /// Merge another histogram into this one.
    pub fn merge(&mut self, other: &Self) {
        for (dst, src) in self.buckets.iter_mut().zip(other.buckets.iter()) {
            *dst += src;
        }
        self.overflow += other.overflow;
        self.total_words += other.total_words;
        self.calls += other.calls;
        if other.max > self.max {
            self.max = other.max;
        }
    }
}

thread_local! {
    /// Per-thread, per-site histograms.
    ///
    /// Thread-local rather than a global mutex so instrumentation cannot
    /// perturb the very timings a follow-up benchmark would measure. The
    /// reporting command is single-threaded, so no cross-thread merge is
    /// needed.
    static HISTOGRAMS: RefCell<[Histogram; Site::COUNT]> =
        RefCell::new(core::array::from_fn(|_| Histogram::default()));
}

/// Record that a scan at `site` traversed `words` words.
///
/// Call sites guard this with `#[cfg(feature = "select-stats")]` so the hot
/// loops are untouched in normal builds.
#[inline]
pub fn record(site: Site, words: usize) {
    HISTOGRAMS.with(|h| {
        if let Ok(mut h) = h.try_borrow_mut() {
            h[site.index()].record(words);
        }
    });
}

/// Take a snapshot of the current thread's histogram for `site`.
pub fn snapshot(site: Site) -> Histogram {
    HISTOGRAMS.with(|h| h.borrow()[site.index()].clone())
}

/// Reset the current thread's histograms.
pub fn reset() {
    HISTOGRAMS.with(|h| {
        *h.borrow_mut() = core::array::from_fn(|_| Histogram::default());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_histogram_reports_nothing() {
        let h = Histogram::default();
        assert_eq!(h.calls(), 0);
        assert_eq!(h.max(), 0);
        assert_eq!(h.mean(), None);
        assert_eq!(h.percentile(50.0), None);
        assert_eq!(h.fraction_below(4), None);
    }

    #[test]
    fn percentile_uses_nearest_rank() {
        let mut h = Histogram::default();
        // Ten scans: 1,1,1,1,1,2,2,2,3,9
        for _ in 0..5 {
            h.record(1);
        }
        for _ in 0..3 {
            h.record(2);
        }
        h.record(3);
        h.record(9);

        assert_eq!(h.calls(), 10);
        assert_eq!(h.max(), 9);
        assert_eq!(h.percentile(50.0), Some(1));
        assert_eq!(h.percentile(90.0), Some(3));
        assert_eq!(h.percentile(100.0), Some(9));
        // mean = (5*1 + 3*2 + 3 + 9) / 10 = 23/10
        assert!((h.mean().unwrap() - 2.3).abs() < 1e-9);
    }

    #[test]
    fn fraction_below_counts_strictly_shorter_scans() {
        let mut h = Histogram::default();
        h.record(0);
        h.record(1);
        h.record(4);
        h.record(10);
        // 0 and 1 are below 4; 4 itself is not.
        assert!((h.fraction_below(4).unwrap() - 0.5).abs() < 1e-9);
        assert!((h.fraction_below(1).unwrap() - 0.25).abs() < 1e-9);
    }

    #[test]
    fn work_fraction_weights_by_words_not_calls() {
        let mut h = Histogram::default();
        // Bimodal: many trivial scans, one long one. By call count the short
        // scans dominate; by work the long scan does. #40 turns on the latter.
        for _ in 0..99 {
            h.record(1);
        }
        h.record(901);

        assert_eq!(h.total_words(), 99 + 901);
        // 99% of calls are below 4 words ...
        assert!((h.fraction_below(4).unwrap() - 0.99).abs() < 1e-9);
        // ... but 90.1% of the work is in scans of 4+ words.
        assert!((h.work_fraction_at_or_above(4).unwrap() - 0.901).abs() < 1e-9);
    }

    #[test]
    fn work_fraction_stays_correct_above_the_bucket_ceiling() {
        let mut h = Histogram::default();
        h.record(1);
        // Lands in overflow, but still contributes its exact length to the
        // total, so the work share must not silently drop it.
        h.record(EXACT_BUCKETS + 99);

        let total = 1 + (EXACT_BUCKETS as u64 + 99);
        assert_eq!(h.total_words(), total);
        let expected = (total - 1) as f64 / total as f64;
        assert!((h.work_fraction_at_or_above(4).unwrap() - expected).abs() < 1e-9);
    }

    #[test]
    fn work_fraction_is_none_without_samples() {
        assert_eq!(Histogram::default().work_fraction_at_or_above(4), None);
        // A histogram of only zero-length scans has no work to attribute.
        let mut zeros = Histogram::default();
        zeros.record(0);
        assert_eq!(zeros.work_fraction_at_or_above(4), None);
    }

    #[test]
    fn long_scans_land_in_overflow_but_keep_exact_max() {
        let mut h = Histogram::default();
        h.record(1);
        h.record(EXACT_BUCKETS + 500);
        assert_eq!(h.calls(), 2);
        assert_eq!(h.max(), (EXACT_BUCKETS + 500) as u64);
        // The overflowing sample cannot be resolved beyond the bucket ceiling.
        assert_eq!(h.percentile(100.0), Some(EXACT_BUCKETS as u64));
    }

    #[test]
    fn merge_combines_counts_and_max() {
        let mut a = Histogram::default();
        a.record(1);
        a.record(2);
        let mut b = Histogram::default();
        b.record(2);
        b.record(70);

        a.merge(&b);
        assert_eq!(a.calls(), 4);
        assert_eq!(a.max(), 70);
        assert_eq!(a.percentile(50.0), Some(2));
    }

    #[test]
    fn record_and_snapshot_roundtrip() {
        reset();
        record(Site::YamlAdvance, 3);
        record(Site::YamlAdvance, 5);
        record(Site::BitVec, 1);

        let yaml = snapshot(Site::YamlAdvance);
        assert_eq!(yaml.calls(), 2);
        assert_eq!(yaml.max(), 5);

        let bitvec = snapshot(Site::BitVec);
        assert_eq!(bitvec.calls(), 1);

        reset();
        assert_eq!(snapshot(Site::YamlAdvance).calls(), 0);
    }

    #[test]
    fn sites_have_dense_distinct_indices() {
        let all = Site::all();
        assert_eq!(all.len(), Site::COUNT);
        for (i, site) in all.iter().enumerate() {
            assert_eq!(site.index(), i, "{}", site.name());
        }
    }
}
