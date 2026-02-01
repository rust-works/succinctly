//! Criterion benchmarks for JSON simple cursor operations.
//!
//! Measures performance of cursor-only (no BP RangeMin) semi-index construction
//! on real JSON files from data/bench/generated/.
//!
//! This benchmark tests simpler cursor implementations on files up to 10MB.
//!
//! Run with:
//! ```bash
//! cargo bench --bench json_simd_cursor
//! ```

mod json_simd_common;

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use json_simd_common::{discover_json_files, filter_by_max_size};

/// Benchmark simple cursor on all discovered files (x86_64).
#[cfg(target_arch = "x86_64")]
fn bench_simple_cursor(c: &mut Criterion) {
    let all_files = discover_json_files();

    if all_files.is_empty() {
        return;
    }

    // Limit to files <= 10MB for simple cursor benchmarks
    let files = filter_by_max_size(all_files, 10 * 1024 * 1024);

    let mut group = c.benchmark_group("json_simple_cursor");
    group.sample_size(10);

    for (name, path, file_size) in &files {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => continue,
        };

        group.throughput(Throughput::Bytes(*file_size));

        // AVX2 (if available)
        if is_x86_feature_detected!("avx2") {
            group.bench_with_input(BenchmarkId::new("AVX2", name), &bytes, |b, bytes| {
                b.iter(|| succinctly::json::simd::avx2::build_semi_index_simple(black_box(bytes)))
            });
        }

        // SSE2 (always available)
        group.bench_with_input(BenchmarkId::new("SSE2", name), &bytes, |b, bytes| {
            b.iter(|| succinctly::json::simd::x86::build_semi_index_simple(black_box(bytes)))
        });

        // Scalar (byte-by-byte simple cursor)
        group.bench_with_input(BenchmarkId::new("Scalar", name), &bytes, |b, bytes| {
            b.iter(|| succinctly::json::simple::build_semi_index(black_box(bytes)))
        });
    }

    group.finish();
}

/// Benchmark simple cursor on all discovered files (aarch64).
#[cfg(target_arch = "aarch64")]
fn bench_simple_cursor(c: &mut Criterion) {
    let all_files = discover_json_files();

    if all_files.is_empty() {
        return;
    }

    // Limit to files <= 10MB for simple cursor benchmarks
    let files = filter_by_max_size(all_files, 10 * 1024 * 1024);

    let mut group = c.benchmark_group("json_simple_cursor");
    group.sample_size(10);

    for (name, path, file_size) in &files {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => continue,
        };

        group.throughput(Throughput::Bytes(*file_size));

        // NEON simple cursor
        group.bench_with_input(BenchmarkId::new("NEON", name), &bytes, |b, bytes| {
            b.iter(|| succinctly::json::simd::neon::build_semi_index_simple(black_box(bytes)))
        });

        // Scalar (byte-by-byte simple cursor)
        group.bench_with_input(BenchmarkId::new("Scalar", name), &bytes, |b, bytes| {
            b.iter(|| succinctly::json::simple::build_semi_index(black_box(bytes)))
        });
    }

    group.finish();
}

/// Benchmark simple cursor on all discovered files (other architectures).
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
fn bench_simple_cursor(c: &mut Criterion) {
    let all_files = discover_json_files();

    if all_files.is_empty() {
        return;
    }

    let files = filter_by_max_size(all_files, 10 * 1024 * 1024);

    let mut group = c.benchmark_group("json_simple_cursor");
    group.sample_size(10);

    for (name, path, file_size) in &files {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => continue,
        };

        group.throughput(Throughput::Bytes(*file_size));

        // Scalar (byte-by-byte simple cursor)
        group.bench_with_input(BenchmarkId::new("Scalar", name), &bytes, |b, bytes| {
            b.iter(|| succinctly::json::simple::build_semi_index(black_box(bytes)))
        });
    }

    group.finish();
}

criterion_group!(benches, bench_simple_cursor);

criterion_main!(benches);
