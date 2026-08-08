//! Benchmark `LineIndex::to_line_column` against the pre-#228 dense-`BitVec`
//! implementation it replaced (issue #543, closing ADR-0012's query-time gap).
//!
//! `to_line_column` moved from O(1) rank + a sampled select to an O(log lines)
//! binary search of sampled selects (`EliasFano::predecessor`), with a
//! one-entry `Cell` cache added later (`bc877b54`) so a monotone walk — the
//! access pattern the `line`/`column` jq/yq builtins produce — resolves in
//! amortised O(1) instead. Two benchmarks cover both paths as they exist
//! today:
//!
//! - `sequential_forward`: offsets in increasing order, exercising the cache.
//! - `random`: shuffled offsets, defeating the cache and forcing the full
//!   binary search every call.
//!
//! `DenseLineIndexV1` reconstructs the removed `BitVec`-backed
//! `to_line_column` verbatim from commit `48dcdb59` (`rank1(offset+1)` +
//! `select1(line-2)`), so "before" and "after" run in the same binary and
//! process rather than needing a cross-commit rebuild against a since-removed
//! API — the same in-file A/B style `benches/json_pipeline.rs` already uses
//! for `JsonIndexV2`/`bench_v1_vs_v2_text_position`.
//!
//! Run with: cargo bench --bench line_index

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;
use std::hint::black_box;
use succinctly::text::LineIndex;
use succinctly::{BitVec, RankSelect};

/// Pre-#228 dense `to_line_column`: one bit per input byte, set immediately
/// after every LF/CRLF/CR, read via O(1) `rank1` + a `select1` sampled every
/// 256 ones. Reconstructed from the code removed in commit `48dcdb59`
/// (`src/json/light.rs`) — not part of the public API, benchmark-only.
struct DenseLineIndexV1 {
    newlines: BitVec,
}

impl DenseLineIndexV1 {
    fn build(text: &[u8]) -> Self {
        if text.is_empty() {
            return Self {
                newlines: BitVec::new(),
            };
        }

        let mut bits = vec![0u64; text.len().div_ceil(64)];
        let mut i = 0;
        while i < text.len() {
            match text[i] {
                b'\n' => {
                    let next = i + 1;
                    if next < text.len() {
                        bits[next / 64] |= 1 << (next % 64);
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
                        bits[next / 64] |= 1 << (next % 64);
                    }
                    i = next;
                }
                _ => i += 1,
            }
        }

        Self {
            newlines: BitVec::from_words(bits, text.len()),
        }
    }

    fn to_line_column(&self, offset: usize) -> (usize, usize) {
        if self.newlines.is_empty() {
            return (1, offset + 1);
        }

        let markers_before_or_at = self.newlines.rank1(offset + 1);
        let line = 1 + markers_before_or_at;
        let line_start = if line == 1 {
            0
        } else {
            self.newlines.select1(line - 2).unwrap_or(0)
        };

        (line, offset - line_start + 1)
    }
}

/// Synthetic text with `n` lines at realistic lengths
/// (corpus-shape.md: ~20-78 bytes/line; 20 here matches the corpus median).
fn generate_text(n: usize, avg_line_len: usize, seed: u64) -> Vec<u8> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut text = Vec::new();
    for i in 0..n {
        if i > 0 {
            text.push(b'\n');
        }
        let len = avg_line_len.saturating_sub(4) + rng.random_range(0..8);
        for _ in 0..len {
            text.push(b'a' + rng.random_range(0..26));
        }
    }
    text
}

/// Offset of every line start, in increasing order — the `.[] | line` access
/// pattern that visits one node per line walking a document top to bottom.
/// Crucially the gap between consecutive queries is always ~1 line
/// regardless of `text`'s total size, so this stays within
/// `LineIndex::FORWARD_WALK_CAP` at every scale; an earlier version of this
/// benchmark instead spread a fixed query count across the whole text, which
/// made the per-query line-gap (and so the number of cache-miss binary
/// searches) grow with file size — an artifact of the query set, not of
/// `LineIndex` itself.
fn generate_offsets_sequential(text: &[u8]) -> Vec<usize> {
    let mut starts = vec![0usize];
    for (i, &b) in text.iter().enumerate() {
        if b == b'\n' && i + 1 < text.len() {
            starts.push(i + 1);
        }
    }
    starts
}

/// Offsets in random order, defeating `LineIndex`'s forward-walk cache.
fn generate_offsets_random(text: &[u8], count: usize, seed: u64) -> Vec<usize> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    if text.is_empty() {
        return vec![0; count];
    }
    (0..count)
        .map(|_| rng.random_range(0..text.len()))
        .collect()
}

/// Cross-check `DenseLineIndexV1` against `LineIndex` before trusting either
/// side's numbers — bench files aren't covered by `cargo test`.
fn assert_v1_matches_line_index(text: &[u8]) {
    let v1 = DenseLineIndexV1::build(text);
    let index = LineIndex::build(text);
    let step = (text.len() / 200).max(1);
    for offset in (0..text.len()).step_by(step) {
        assert_eq!(
            v1.to_line_column(offset),
            index.to_line_column(offset),
            "DenseLineIndexV1/LineIndex disagree at offset {offset}"
        );
    }
}

/// Line counts spanning the real corpus (corpus-shape.md), up to its largest
/// file (34,169 lines, `pretty/3d-ribbon.json`).
const LINE_COUNTS: [usize; 4] = [1_000, 10_000, 34_169, 100_000];

fn bench_sequential_forward(c: &mut Criterion) {
    let mut group = c.benchmark_group("line_index/sequential_forward");

    for n in LINE_COUNTS {
        let text = generate_text(n, 20, 42);
        assert_v1_matches_line_index(&text);
        let offsets = generate_offsets_sequential(&text);

        let index = LineIndex::build(&text);
        let v1 = DenseLineIndexV1::build(&text);

        group.throughput(Throughput::Elements(offsets.len() as u64));

        group.bench_with_input(
            BenchmarkId::new("line_index", format!("{n}_lines")),
            &(&index, &offsets),
            |b, (index, offsets)| {
                b.iter(|| {
                    let mut sum = 0usize;
                    for &off in *offsets {
                        let (l, c) = index.to_line_column(black_box(off));
                        sum += l + c;
                    }
                    black_box(sum)
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("v1_dense_bitvec", format!("{n}_lines")),
            &(&v1, &offsets),
            |b, (v1, offsets)| {
                b.iter(|| {
                    let mut sum = 0usize;
                    for &off in *offsets {
                        let (l, c) = v1.to_line_column(black_box(off));
                        sum += l + c;
                    }
                    black_box(sum)
                });
            },
        );
    }

    group.finish();
}

fn bench_random(c: &mut Criterion) {
    let mut group = c.benchmark_group("line_index/random");

    for n in LINE_COUNTS {
        let text = generate_text(n, 20, 42);
        let offsets = generate_offsets_random(&text, 10_000, 123);

        let index = LineIndex::build(&text);
        let v1 = DenseLineIndexV1::build(&text);

        group.throughput(Throughput::Elements(offsets.len() as u64));

        group.bench_with_input(
            BenchmarkId::new("line_index", format!("{n}_lines")),
            &(&index, &offsets),
            |b, (index, offsets)| {
                b.iter(|| {
                    let mut sum = 0usize;
                    for &off in *offsets {
                        let (l, c) = index.to_line_column(black_box(off));
                        sum += l + c;
                    }
                    black_box(sum)
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("v1_dense_bitvec", format!("{n}_lines")),
            &(&v1, &offsets),
            |b, (v1, offsets)| {
                b.iter(|| {
                    let mut sum = 0usize;
                    for &off in *offsets {
                        let (l, c) = v1.to_line_column(black_box(off));
                        sum += l + c;
                    }
                    black_box(sum)
                });
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_sequential_forward, bench_random);
criterion_main!(benches);
