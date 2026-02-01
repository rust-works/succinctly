//! Criterion benchmarks for full JSON index construction and pattern comparison.
//!
//! Measures performance of:
//! - Full JsonIndex construction (includes BP RangeMin) on files up to 100MB
//! - Pattern comparison at fixed 10MB size for fairness testing
//!
//! Run with:
//! ```bash
//! cargo bench --bench json_simd_full
//! ```

mod json_simd_common;

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use json_simd_common::{discover_json_files, filter_by_max_size};
use succinctly::json::JsonIndex;

/// Benchmark full JsonIndex construction (includes BP RangeMin).
fn bench_full_index(c: &mut Criterion) {
    let all_files = discover_json_files();

    if all_files.is_empty() {
        return;
    }

    // Limit to files <= 100MB
    let files = filter_by_max_size(all_files, 100 * 1024 * 1024);

    let mut group = c.benchmark_group("json_full_index");
    group.sample_size(10);

    for (name, path, file_size) in &files {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => continue,
        };

        group.throughput(Throughput::Bytes(*file_size));

        group.bench_with_input(BenchmarkId::new("JsonIndex", name), &bytes, |b, bytes| {
            b.iter(|| JsonIndex::build(black_box(bytes)))
        });
    }

    group.finish();
}

/// Benchmark comparing patterns at a fixed size (10MB) for fairness testing.
#[cfg(target_arch = "x86_64")]
fn bench_pattern_comparison(c: &mut Criterion) {
    let all_files = discover_json_files();

    // Filter to only 10mb files from each pattern
    let files: Vec<_> = all_files
        .into_iter()
        .filter(|(name, _, _)| name.ends_with("/10mb"))
        .collect();

    if files.is_empty() {
        return;
    }

    let mut group = c.benchmark_group("pattern_comparison_10mb");
    group.sample_size(10);

    for (name, path, file_size) in &files {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => continue,
        };

        // Extract just the pattern name for cleaner output
        let pattern = name.split('/').next().unwrap_or(name);

        group.throughput(Throughput::Bytes(*file_size));

        if is_x86_feature_detected!("avx2") {
            group.bench_with_input(BenchmarkId::new("AVX2", pattern), &bytes, |b, bytes| {
                b.iter(|| succinctly::json::simd::avx2::build_semi_index_standard(black_box(bytes)))
            });
        }

        group.bench_with_input(BenchmarkId::new("PFSM", pattern), &bytes, |b, bytes| {
            b.iter(|| succinctly::json::standard::build_semi_index(black_box(bytes)))
        });
    }

    group.finish();
}

/// Benchmark comparing patterns at a fixed size (10MB) for fairness testing (aarch64).
#[cfg(target_arch = "aarch64")]
fn bench_pattern_comparison(c: &mut Criterion) {
    let all_files = discover_json_files();

    // Filter to only 10mb files from each pattern
    let files: Vec<_> = all_files
        .into_iter()
        .filter(|(name, _, _)| name.ends_with("/10mb"))
        .collect();

    if files.is_empty() {
        return;
    }

    let mut group = c.benchmark_group("pattern_comparison_10mb");
    group.sample_size(10);

    for (name, path, file_size) in &files {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => continue,
        };

        // Extract just the pattern name for cleaner output
        let pattern = name.split('/').next().unwrap_or(name);

        group.throughput(Throughput::Bytes(*file_size));

        group.bench_with_input(BenchmarkId::new("NEON", pattern), &bytes, |b, bytes| {
            b.iter(|| succinctly::json::simd::neon::build_semi_index_standard(black_box(bytes)))
        });

        group.bench_with_input(BenchmarkId::new("PFSM", pattern), &bytes, |b, bytes| {
            b.iter(|| succinctly::json::standard::build_semi_index(black_box(bytes)))
        });
    }

    group.finish();
}

/// Benchmark comparing patterns at a fixed size (10MB) for fairness testing (other architectures).
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
fn bench_pattern_comparison(c: &mut Criterion) {
    let all_files = discover_json_files();

    // Filter to only 10mb files from each pattern
    let files: Vec<_> = all_files
        .into_iter()
        .filter(|(name, _, _)| name.ends_with("/10mb"))
        .collect();

    if files.is_empty() {
        return;
    }

    let mut group = c.benchmark_group("pattern_comparison_10mb");
    group.sample_size(10);

    for (name, path, file_size) in &files {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => continue,
        };

        let pattern = name.split('/').next().unwrap_or(name);

        group.throughput(Throughput::Bytes(*file_size));

        group.bench_with_input(BenchmarkId::new("PFSM", pattern), &bytes, |b, bytes| {
            b.iter(|| succinctly::json::standard::build_semi_index(black_box(bytes)))
        });
    }

    group.finish();
}

criterion_group!(benches, bench_full_index, bench_pattern_comparison);

criterion_main!(benches);
