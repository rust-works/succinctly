//! Crossover micro-benchmark for the `select` word scan (#40).
//!
//! Answers one question: from what scan distance does a block kernel beat the
//! per-word scalar loop? Everything is parameterised by **scan distance in
//! words**, because that is the quantity `succinctly dev select-stats` measures
//! on real inputs — so the two measurements compose directly into a decision
//! instead of needing a judgement call to connect them.
//!
//! Three variants are compared at each distance:
//!
//! * `scalar`   — the per-word loop every call site used before.
//! * `dispatch` — `scan_select`, which picks NEON / AVX2 / portable at runtime.
//! * `portable` — the same block structure with a plain-Rust block popcount,
//!   which shows how much (if anything) the intrinsics add over what LLVM
//!   already auto-vectorises. If this matches `dispatch`, the intrinsics are
//!   not carrying their weight and should go.
//!
//! Run with:
//! ```bash
//! cargo bench --bench select_scan_micro
//! ```

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::hint::black_box;
use succinctly::bits::{block_popcount_portable, scan_select, scan_select_scalar, BLOCK};

/// Scan distances in words, spanning the range the corpus actually produces:
/// p50 ≈ 3, p90 ≈ 232, max ≈ 296 at the hot YAML site.
const DISTANCES: &[usize] = &[1, 2, 4, 8, 16, 32, 64, 128, 256];

/// Bits set per word.
///
/// Long scans arise where interest bits are *sparse* — a long YAML scalar puts
/// many bitmap words between consecutive structural positions — so `1` is the
/// realistic case for the tail this optimisation targets. `32` is included as
/// a dense control.
const DENSITIES: &[u32] = &[1, 32];

/// Build words with exactly `ones_per_word` bits set in each, spread out so no
/// word is trivially all-ones.
fn words_with_density(count: usize, ones_per_word: u32) -> Vec<u64> {
    let stride = 64 / ones_per_word.max(1);
    let word = (0..ones_per_word).fold(0u64, |acc, i| acc | (1u64 << (i * stride)));
    vec![word; count]
}

/// Block-structured scan using the portable block popcount, mirroring
/// `scan_select`'s shape so the comparison isolates the kernel.
fn scan_portable(words: &[u64], start_word: usize, remaining: usize) -> Option<(usize, usize)> {
    if start_word >= words.len() {
        return None;
    }
    let mut rem = remaining;
    let mut idx = start_word;

    while idx + BLOCK <= words.len() {
        let total = block_popcount_portable(&words[idx..idx + BLOCK]);
        if total > rem {
            break;
        }
        rem -= total;
        idx += BLOCK;
    }

    for (off, &w) in words[idx..].iter().enumerate() {
        let pop = w.count_ones() as usize;
        if pop > rem {
            return Some((idx + off, rem));
        }
        rem -= pop;
    }
    None
}

fn bench_scan(c: &mut Criterion) {
    for &ones_per_word in DENSITIES {
        let mut group = c.benchmark_group(format!("select_scan/density_{ones_per_word}"));

        for &distance in DISTANCES {
            // Enough words to cover the scan plus a block of slack.
            let words = words_with_density(distance + BLOCK + 2, ones_per_word);
            // Target the first set bit of the word `distance` words along, so
            // the scan traverses exactly `distance` words before crossing.
            let k = distance * ones_per_word as usize;

            // Guard the premise: if these disagree the benchmark is measuring
            // two different things and any speedup it reports is meaningless.
            assert_eq!(
                scan_select(&words, 0, k),
                scan_select_scalar(&words, 0, k),
                "dispatch/scalar disagree at distance={distance} density={ones_per_word}"
            );
            assert_eq!(
                scan_portable(&words, 0, k),
                scan_select_scalar(&words, 0, k),
                "portable/scalar disagree at distance={distance} density={ones_per_word}"
            );

            group.bench_with_input(BenchmarkId::new("scalar", distance), &distance, |b, _| {
                b.iter(|| scan_select_scalar(black_box(&words), 0, black_box(k)));
            });
            group.bench_with_input(BenchmarkId::new("dispatch", distance), &distance, |b, _| {
                b.iter(|| scan_select(black_box(&words), 0, black_box(k)));
            });
            group.bench_with_input(BenchmarkId::new("portable", distance), &distance, |b, _| {
                b.iter(|| scan_portable(black_box(&words), 0, black_box(k)));
            });
        }

        group.finish();
    }
}

/// The measured corpus distribution, replayed as a mixed workload.
///
/// Per-distance numbers can mislead when a distribution is bimodal: a kernel
/// that wins hugely on long scans and loses slightly on short ones may still be
/// a net win, or not, depending on the mix. This benchmark applies the real mix
/// — roughly half trivial scans, the rest spread into a long tail — so the
/// aggregate is measured rather than inferred.
fn bench_corpus_mix(c: &mut Criterion) {
    // Approximates the hot YAML site: p50 ≈ 3, p90 ≈ 232, max ≈ 296,
    // ~50% of calls below 4 words.
    let mix: &[usize] = &[
        1, 1, 2, 2, 3, 3, 3, 1, 2, 3, // the short half
        8, 14, 20, 45, 96, 160, 232, 246, 288, 296, // the long tail
    ];
    let ones_per_word = 1u32;
    let longest = *mix.iter().max().unwrap();
    let words = words_with_density(longest + BLOCK + 2, ones_per_word);
    let targets: Vec<usize> = mix.iter().map(|d| d * ones_per_word as usize).collect();

    let mut group = c.benchmark_group("select_scan/corpus_mix");

    group.bench_function("scalar", |b| {
        b.iter(|| {
            for &k in &targets {
                black_box(scan_select_scalar(black_box(&words), 0, k));
            }
        });
    });
    group.bench_function("dispatch", |b| {
        b.iter(|| {
            for &k in &targets {
                black_box(scan_select(black_box(&words), 0, k));
            }
        });
    });
    group.bench_function("portable", |b| {
        b.iter(|| {
            for &k in &targets {
                black_box(scan_portable(black_box(&words), 0, k));
            }
        });
    });

    group.finish();
}

criterion_group!(benches, bench_scan, bench_corpus_mix);
criterion_main!(benches);
