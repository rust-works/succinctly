//! Criterion benchmarks for Elias-Fano encoding.
//!
//! Compares EliasFano against Vec<u32> for different access patterns:
//! - Sequential iteration (cursor-based)
//! - Random access
//! - Skip-by-small (advance_by with small k)
//! - Seek operations
//!
//! Run with: cargo bench --bench elias_fano

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use succinctly::bits::EliasFano;

/// Generate realistic bp_to_text-like data: monotone positions with varying gaps.
fn generate_bp_to_text(n: usize, seed: u64) -> Vec<u32> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut values = Vec::with_capacity(n);
    let mut pos = 0u32;

    for _ in 0..n {
        values.push(pos);
        // Varying gaps: small for scalars (10-30), larger for containers (50-150)
        let gap = if rng.gen_bool(0.8) {
            rng.gen_range(10..30) // 80% small gaps (scalar values)
        } else {
            rng.gen_range(50..150) // 20% larger gaps (containers)
        };
        pos = pos.saturating_add(gap);
    }

    values
}

/// Generate random query indices.
fn generate_queries(count: usize, max: usize, seed: u64) -> Vec<usize> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    (0..count).map(|_| rng.gen_range(0..max)).collect()
}

fn bench_construction(c: &mut Criterion) {
    let mut group = c.benchmark_group("elias_fano/construction");

    for n in [10_000, 100_000, 1_000_000] {
        let values = generate_bp_to_text(n, 42);

        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(
            BenchmarkId::new("build", format!("{}K", n / 1000)),
            &values,
            |b, values| b.iter(|| EliasFano::build(black_box(values))),
        );
    }

    group.finish();
}

fn bench_sequential(c: &mut Criterion) {
    let mut group = c.benchmark_group("elias_fano/sequential");

    for n in [10_000, 100_000, 1_000_000] {
        let values = generate_bp_to_text(n, 42);
        let ef = EliasFano::build(&values);

        group.throughput(Throughput::Elements(n as u64));

        // EliasFano cursor iteration
        group.bench_with_input(
            BenchmarkId::new("ef_cursor", format!("{}K", n / 1000)),
            &ef,
            |b, ef| {
                b.iter(|| {
                    let mut cursor = ef.cursor();
                    let mut sum = 0u64;
                    while let Some(v) = cursor.current() {
                        sum += v as u64;
                        cursor.advance_one();
                    }
                    black_box(sum)
                })
            },
        );

        // Vec<u32> iteration (baseline)
        group.bench_with_input(
            BenchmarkId::new("vec_iter", format!("{}K", n / 1000)),
            &values,
            |b, values| {
                b.iter(|| {
                    let mut sum = 0u64;
                    for &v in values {
                        sum += v as u64;
                    }
                    black_box(sum)
                })
            },
        );
    }

    group.finish();
}

fn bench_random_access(c: &mut Criterion) {
    let mut group = c.benchmark_group("elias_fano/random_access");

    for n in [10_000, 100_000, 1_000_000] {
        let values = generate_bp_to_text(n, 42);
        let ef = EliasFano::build(&values);
        let queries = generate_queries(10_000, n, 123);

        group.throughput(Throughput::Elements(queries.len() as u64));

        // EliasFano random get
        group.bench_with_input(
            BenchmarkId::new("ef_get", format!("{}K", n / 1000)),
            &(&ef, &queries),
            |b, (ef, queries)| {
                b.iter(|| {
                    let mut sum = 0u64;
                    for &i in queries.iter() {
                        sum += ef.get(black_box(i)).unwrap() as u64;
                    }
                    black_box(sum)
                })
            },
        );

        // Vec<u32> random access (baseline)
        group.bench_with_input(
            BenchmarkId::new("vec_index", format!("{}K", n / 1000)),
            &(&values, &queries),
            |b, (values, queries)| {
                b.iter(|| {
                    let mut sum = 0u64;
                    for &i in queries.iter() {
                        sum += values[black_box(i)] as u64;
                    }
                    black_box(sum)
                })
            },
        );
    }

    group.finish();
}

fn bench_skip_by_small(c: &mut Criterion) {
    let mut group = c.benchmark_group("elias_fano/skip_by_small");

    for n in [10_000, 100_000, 1_000_000] {
        let values = generate_bp_to_text(n, 42);
        let ef = EliasFano::build(&values);

        // EliasFano advance_by (simulating next_sibling navigation)
        group.bench_with_input(
            BenchmarkId::new("ef_advance_by", format!("{}K", n / 1000)),
            &ef,
            |b, ef| {
                b.iter(|| {
                    let mut cursor = ef.cursor();
                    let mut sum = 0u64;
                    let mut skip = 2usize;
                    while let Some(v) = cursor.current() {
                        sum += v as u64;
                        cursor.advance_by(skip);
                        skip = (skip % 7) + 2; // Vary skip 2-8
                    }
                    black_box(sum)
                })
            },
        );

        // Vec<u32> skip (baseline)
        group.bench_with_input(
            BenchmarkId::new("vec_skip", format!("{}K", n / 1000)),
            &values,
            |b, values| {
                b.iter(|| {
                    let mut sum = 0u64;
                    let mut i = 0;
                    let mut skip = 2usize;
                    while i < values.len() {
                        sum += values[i] as u64;
                        i += skip;
                        skip = (skip % 7) + 2;
                    }
                    black_box(sum)
                })
            },
        );
    }

    group.finish();
}

fn bench_seek(c: &mut Criterion) {
    let mut group = c.benchmark_group("elias_fano/seek");

    for n in [10_000, 100_000, 1_000_000] {
        let values = generate_bp_to_text(n, 42);
        let ef = EliasFano::build(&values);
        // Generate seek targets that jump forward by varying amounts
        let seek_targets: Vec<usize> = (0..1000).map(|i| ((i * 73) % n).min(n - 1)).collect();

        group.throughput(Throughput::Elements(seek_targets.len() as u64));

        // EliasFano seek (uses select samples)
        group.bench_with_input(
            BenchmarkId::new("ef_seek", format!("{}K", n / 1000)),
            &(&ef, &seek_targets),
            |b, (ef, targets)| {
                b.iter(|| {
                    let mut sum = 0u64;
                    for &target in targets.iter() {
                        let mut cursor = ef.cursor();
                        if let Some(v) = cursor.seek(black_box(target)) {
                            sum += v as u64;
                        }
                    }
                    black_box(sum)
                })
            },
        );

        // Vec<u32> direct index (baseline)
        group.bench_with_input(
            BenchmarkId::new("vec_index", format!("{}K", n / 1000)),
            &(&values, &seek_targets),
            |b, (values, targets)| {
                b.iter(|| {
                    let mut sum = 0u64;
                    for &target in targets.iter() {
                        sum += values[black_box(target)] as u64;
                    }
                    black_box(sum)
                })
            },
        );
    }

    group.finish();
}

fn bench_cursor_from(c: &mut Criterion) {
    let mut group = c.benchmark_group("elias_fano/cursor_from");

    for n in [10_000, 100_000, 1_000_000] {
        let values = generate_bp_to_text(n, 42);
        let ef = EliasFano::build(&values);
        // Random starting positions (simulating entering subtrees at arbitrary bp indices)
        let start_positions = generate_queries(1000, n, 456);

        group.throughput(Throughput::Elements(start_positions.len() as u64));

        // EliasFano cursor_from (creates cursor at arbitrary position)
        group.bench_with_input(
            BenchmarkId::new("ef_cursor_from", format!("{}K", n / 1000)),
            &(&ef, &start_positions),
            |b, (ef, positions)| {
                b.iter(|| {
                    let mut sum = 0u64;
                    for &pos in positions.iter() {
                        let cursor = ef.cursor_from(black_box(pos));
                        if let Some(v) = cursor.current() {
                            sum += v as u64;
                        }
                    }
                    black_box(sum)
                })
            },
        );

        // EliasFano cursor_from + short iteration (typical use: enter subtree, iterate children)
        group.bench_with_input(
            BenchmarkId::new("ef_cursor_from_iter5", format!("{}K", n / 1000)),
            &(&ef, &start_positions),
            |b, (ef, positions)| {
                b.iter(|| {
                    let mut sum = 0u64;
                    for &pos in positions.iter() {
                        let mut cursor = ef.cursor_from(black_box(pos));
                        // Iterate 5 elements (typical: small container children)
                        for _ in 0..5 {
                            if let Some(v) = cursor.current() {
                                sum += v as u64;
                                cursor.advance_one();
                            } else {
                                break;
                            }
                        }
                    }
                    black_box(sum)
                })
            },
        );

        // Vec<u32> equivalent (baseline)
        group.bench_with_input(
            BenchmarkId::new("vec_index", format!("{}K", n / 1000)),
            &(&values, &start_positions),
            |b, (values, positions)| {
                b.iter(|| {
                    let mut sum = 0u64;
                    for &pos in positions.iter() {
                        sum += values[black_box(pos)] as u64;
                    }
                    black_box(sum)
                })
            },
        );

        // Vec<u32> index + short iteration (baseline)
        group.bench_with_input(
            BenchmarkId::new("vec_index_iter5", format!("{}K", n / 1000)),
            &(&values, &start_positions),
            |b, (values, positions)| {
                b.iter(|| {
                    let mut sum = 0u64;
                    for &pos in positions.iter() {
                        let start = black_box(pos);
                        for i in 0..5 {
                            if start + i < values.len() {
                                sum += values[start + i] as u64;
                            } else {
                                break;
                            }
                        }
                    }
                    black_box(sum)
                })
            },
        );
    }

    group.finish();
}

fn bench_memory(c: &mut Criterion) {
    let mut group = c.benchmark_group("elias_fano/memory");

    for n in [10_000, 100_000, 1_000_000] {
        let values = generate_bp_to_text(n, 42);
        let ef = EliasFano::build(&values);

        let vec_size = values.len() * 4;
        let ef_size = ef.heap_size();
        let compression = vec_size as f64 / ef_size as f64;

        // Just report sizes (no actual benchmark, just use a trivial operation)
        group.bench_with_input(
            BenchmarkId::new(
                format!(
                    "{}K (Vec={}KB, EF={}KB, {:.2}x)",
                    n / 1000,
                    vec_size / 1024,
                    ef_size / 1024,
                    compression
                ),
                "",
            ),
            &ef,
            |b, ef| b.iter(|| black_box(ef.len())),
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_construction,
    bench_sequential,
    bench_random_access,
    bench_skip_by_small,
    bench_seek,
    bench_cursor_from,
    bench_memory,
);
criterion_main!(benches);
