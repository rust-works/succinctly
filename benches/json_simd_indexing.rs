//! Criterion benchmarks for JSON SIMD indexing operations.
//!
//! Measures performance of different SIMD implementations (AVX2, SSE4.2, SSE2, NEON, PFSM)
//! on real JSON files from data/bench/generated/.
//!
//! This is the heaviest json_simd benchmark - tests all SIMD variants on files up to 100MB.
//!
//! Run with:
//! ```bash
//! cargo bench --bench json_simd_indexing
//! ```

mod json_simd_common;

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use json_simd_common::{discover_json_files, filter_by_max_size};

/// Benchmark JSON indexing throughput on all discovered files (x86_64).
#[cfg(target_arch = "x86_64")]
fn bench_json_indexing(c: &mut Criterion) {
    let all_files = discover_json_files();

    if all_files.is_empty() {
        eprintln!("No JSON files found for benchmarking.");
        return;
    }

    // Limit to files <= 100MB for reasonable benchmark times
    let files = filter_by_max_size(all_files, 100 * 1024 * 1024);

    let mut group = c.benchmark_group("json_indexing");

    // Configure for larger files - use fewer samples
    group.sample_size(10);

    for (name, path, file_size) in &files {
        // Read file contents
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("Failed to read {}: {}", path.display(), e);
                continue;
            }
        };

        group.throughput(Throughput::Bytes(*file_size));

        // AVX2 (if available) - fastest on x86_64
        if is_x86_feature_detected!("avx2") {
            group.bench_with_input(BenchmarkId::new("AVX2", name), &bytes, |b, bytes| {
                b.iter(|| succinctly::json::simd::avx2::build_semi_index_standard(black_box(bytes)))
            });
        }

        // SSE4.2 (if available)
        if is_x86_feature_detected!("sse4.2") {
            group.bench_with_input(BenchmarkId::new("SSE4.2", name), &bytes, |b, bytes| {
                b.iter(|| {
                    succinctly::json::simd::sse42::build_semi_index_standard(black_box(bytes))
                })
            });
        }

        // SSE2 (always available on x86_64)
        group.bench_with_input(BenchmarkId::new("SSE2", name), &bytes, |b, bytes| {
            b.iter(|| succinctly::json::simd::x86::build_semi_index_standard(black_box(bytes)))
        });

        // PFSM (table-based state machine) baseline
        group.bench_with_input(BenchmarkId::new("PFSM", name), &bytes, |b, bytes| {
            b.iter(|| succinctly::json::standard::build_semi_index(black_box(bytes)))
        });
    }

    group.finish();
}

/// Benchmark JSON indexing throughput on all discovered files (aarch64).
#[cfg(target_arch = "aarch64")]
fn bench_json_indexing(c: &mut Criterion) {
    use succinctly::json::JsonIndex;

    let all_files = discover_json_files();

    if all_files.is_empty() {
        eprintln!("No JSON files found for benchmarking.");
        return;
    }

    let files = filter_by_max_size(all_files, 100 * 1024 * 1024);

    let mut group = c.benchmark_group("json_indexing");
    group.sample_size(10);

    for (name, path, file_size) in &files {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => continue,
        };

        group.throughput(Throughput::Bytes(*file_size));

        // NEON (always available on aarch64)
        group.bench_with_input(BenchmarkId::new("NEON", name), &bytes, |b, bytes| {
            b.iter(|| succinctly::json::simd::neon::build_semi_index_standard(black_box(bytes)))
        });

        // PFSM (table-based state machine) baseline
        group.bench_with_input(BenchmarkId::new("PFSM", name), &bytes, |b, bytes| {
            b.iter(|| succinctly::json::standard::build_semi_index(black_box(bytes)))
        });

        // Full index
        group.bench_with_input(BenchmarkId::new("JsonIndex", name), &bytes, |b, bytes| {
            b.iter(|| JsonIndex::build(black_box(bytes)))
        });
    }

    group.finish();
}

/// Benchmark JSON indexing throughput on all discovered files (other architectures).
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
fn bench_json_indexing(c: &mut Criterion) {
    use succinctly::json::JsonIndex;

    let all_files = discover_json_files();
    let files = filter_by_max_size(all_files, 100 * 1024 * 1024);

    let mut group = c.benchmark_group("json_indexing");
    group.sample_size(10);

    for (name, path, file_size) in &files {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => continue,
        };

        group.throughput(Throughput::Bytes(*file_size));

        group.bench_with_input(BenchmarkId::new("PFSM", name), &bytes, |b, bytes| {
            b.iter(|| succinctly::json::standard::build_semi_index(black_box(bytes)))
        });

        group.bench_with_input(BenchmarkId::new("JsonIndex", name), &bytes, |b, bytes| {
            b.iter(|| JsonIndex::build(black_box(bytes)))
        });
    }

    group.finish();
}

criterion_group!(benches, bench_json_indexing);

criterion_main!(benches);
