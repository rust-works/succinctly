//! Benchmarks for JSON validation (RFC 8259).
//!
//! This benchmark validates all generated JSON test files to measure
//! validation throughput across different patterns and sizes.
//!
//! Run with:
//! ```bash
//! cargo bench --bench json_validate_bench
//! ```

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::fs;
use std::hint::black_box;
use std::path::Path;
use succinctly::json::validate;

/// Test file patterns available in data/bench/generated/
const PATTERNS: &[&str] = &[
    "arrays",
    "comprehensive",
    "literals",
    "mixed",
    "nested",
    "numbers",
    "pathological",
    "strings",
    "unicode",
    "users",
];

/// Test file sizes
const SIZES: &[&str] = &["1kb", "10kb", "100kb", "1mb", "10mb"];

/// Base directory for generated files
const BASE_DIR: &str = "data/bench/generated";

/// Load a test file, returning None if it doesn't exist
fn load_file(pattern: &str, size: &str) -> Option<Vec<u8>> {
    let path = format!("{BASE_DIR}/{pattern}/{size}.json");
    let path = Path::new(&path);
    if !path.exists() {
        return None;
    }
    fs::read(path).ok()
}

/// Benchmark validation across all patterns for a given size
fn bench_validate_by_size(c: &mut Criterion) {
    // Group benchmarks by size
    for size in SIZES {
        let mut group = c.benchmark_group(format!("validate_{size}"));

        for pattern in PATTERNS {
            let Some(bytes) = load_file(pattern, size) else {
                continue;
            };

            group.throughput(Throughput::Bytes(bytes.len() as u64));

            group.bench_with_input(BenchmarkId::from_parameter(pattern), &bytes, |b, bytes| {
                b.iter(|| {
                    let result = validate::validate(black_box(bytes));
                    black_box(result)
                });
            });
        }

        group.finish();
    }
}

/// Benchmark validation across all sizes for a given pattern
fn bench_validate_by_pattern(c: &mut Criterion) {
    // Focus on comprehensive pattern as it tests all JSON features
    let pattern = "comprehensive";
    let mut group = c.benchmark_group(format!("validate_{pattern}"));

    for size in SIZES {
        let Some(bytes) = load_file(pattern, size) else {
            continue;
        };

        group.throughput(Throughput::Bytes(bytes.len() as u64));

        group.bench_with_input(BenchmarkId::from_parameter(size), &bytes, |b, bytes| {
            b.iter(|| {
                let result = validate::validate(black_box(bytes));
                black_box(result)
            });
        });
    }

    group.finish();
}

/// Benchmark validation of the largest files (10mb) to measure sustained throughput
fn bench_validate_large_files(c: &mut Criterion) {
    let mut group = c.benchmark_group("validate_10mb");
    group.sample_size(10); // Fewer samples for large files

    for pattern in PATTERNS {
        let Some(bytes) = load_file(pattern, "10mb") else {
            continue;
        };

        group.throughput(Throughput::Bytes(bytes.len() as u64));

        group.bench_with_input(BenchmarkId::from_parameter(pattern), &bytes, |b, bytes| {
            b.iter(|| {
                let result = validate::validate(black_box(bytes));
                black_box(result)
            });
        });
    }

    group.finish();
}

/// Verify all generated files pass validation (not a benchmark, but useful for testing)
fn verify_all_files_valid(c: &mut Criterion) {
    let mut group = c.benchmark_group("validate_verify_all");
    group.sample_size(10);

    // Collect all valid files
    let mut all_files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut total_bytes = 0u64;

    for pattern in PATTERNS {
        for size in SIZES {
            if let Some(bytes) = load_file(pattern, size) {
                total_bytes += bytes.len() as u64;
                all_files.push((format!("{pattern}/{size}"), bytes));
            }
        }
    }

    if all_files.is_empty() {
        eprintln!("No test files found. Run: succinctly json generate-suite");
        return;
    }

    group.throughput(Throughput::Bytes(total_bytes));

    group.bench_function("all_files", |b| {
        b.iter(|| {
            let mut valid_count = 0;
            for (name, bytes) in &all_files {
                match validate::validate(black_box(bytes)) {
                    Ok(()) => valid_count += 1,
                    Err(e) => panic!("Validation failed for {name}: {e:?}"),
                }
            }
            valid_count
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_validate_by_size,
    bench_validate_by_pattern,
    bench_validate_large_files,
    verify_all_files_valid,
);
criterion_main!(benches);
