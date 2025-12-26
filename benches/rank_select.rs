//! Criterion benchmarks for rank/select operations.

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use succinctly::{BitVec, RankSelect};

/// Generate a bitvector with specified size and density.
fn generate_bitvec(size: usize, density: f64, seed: u64) -> BitVec {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let word_count = (size + 63) / 64;
    let mut words = Vec::with_capacity(word_count);

    let threshold = (density * u64::MAX as f64) as u64;
    for _ in 0..word_count {
        let mut word = 0u64;
        for bit in 0..64 {
            if rng.r#gen::<u64>() < threshold {
                word |= 1 << bit;
            }
        }
        words.push(word);
    }

    BitVec::from_words(words, size)
}

/// Generate random query positions.
fn generate_queries(count: usize, max: usize, seed: u64) -> Vec<usize> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    (0..count).map(|_| rng.gen_range(0..max)).collect()
}

fn bench_rank(c: &mut Criterion) {
    let mut group = c.benchmark_group("rank1");

    for size in [1_000_000, 10_000_000] {
        for density in [0.01, 0.1, 0.5, 0.9] {
            let bv = generate_bitvec(size, density, 42);
            let queries = generate_queries(10000, size, 123);

            group.bench_with_input(
                BenchmarkId::new(
                    format!("{:.0}M/{:.0}%", size as f64 / 1e6, density * 100.0),
                    "",
                ),
                &(&bv, &queries),
                |b, (bv, queries)| {
                    b.iter(|| {
                        let mut sum = 0usize;
                        for &q in queries.iter() {
                            sum += bv.rank1(black_box(q));
                        }
                        sum
                    })
                },
            );
        }
    }
    group.finish();
}

fn bench_select(c: &mut Criterion) {
    let mut group = c.benchmark_group("select1");

    for size in [1_000_000, 10_000_000] {
        for density in [0.1, 0.5, 0.9] {
            let bv = generate_bitvec(size, density, 42);
            let ones = bv.count_ones();
            if ones == 0 {
                continue;
            }
            let queries = generate_queries(10000, ones, 123);

            group.bench_with_input(
                BenchmarkId::new(
                    format!("{:.0}M/{:.0}%", size as f64 / 1e6, density * 100.0),
                    "",
                ),
                &(&bv, &queries),
                |b, (bv, queries)| {
                    b.iter(|| {
                        let mut sum = 0usize;
                        for &q in queries.iter() {
                            if let Some(pos) = bv.select1(black_box(q)) {
                                sum += pos;
                            }
                        }
                        sum
                    })
                },
            );
        }
    }
    group.finish();
}

fn bench_construction(c: &mut Criterion) {
    let mut group = c.benchmark_group("construction");

    for size in [1_000_000, 10_000_000] {
        let words: Vec<u64> = (0..(size + 63) / 64)
            .map(|i| i as u64 * 0x1234_5678_9ABC_DEF0)
            .collect();

        group.bench_with_input(
            BenchmarkId::new(format!("{:.0}M", size as f64 / 1e6), ""),
            &words,
            |b, words| b.iter(|| BitVec::from_words(black_box(words.clone()), size)),
        );
    }
    group.finish();
}

fn bench_select_in_word(c: &mut Criterion) {
    use succinctly::select_in_word;

    let mut group = c.benchmark_group("select_in_word");

    // Test different word patterns
    let patterns = [
        ("sparse", 0x0001_0001_0001_0001u64),
        ("dense", 0xFFFF_FFFF_FFFF_FFFFu64),
        ("alternating", 0xAAAA_AAAA_AAAA_AAAAu64),
    ];

    for (name, word) in patterns {
        let pop = word.count_ones();
        group.bench_with_input(BenchmarkId::new(name, ""), &word, |b, &word| {
            b.iter(|| {
                let mut sum = 0u32;
                for k in 0..pop {
                    sum += select_in_word(black_box(word), k);
                }
                sum
            })
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_rank,
    bench_select,
    bench_construction,
    bench_select_in_word
);
criterion_main!(benches);
