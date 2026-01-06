//! Criterion benchmarks for JSON SIMD parsing operations.
//!
//! Measures performance of different SIMD implementations on real JSON files
//! from data/bench/generated/.
//!
//! Run with:
//! ```bash
//! cargo bench --bench json_simd
//! ```

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use std::path::PathBuf;

/// Discover all JSON files in data/bench/generated/ directory.
/// Returns a sorted list of (display_name, path, file_size) tuples.
fn discover_json_files() -> Vec<(String, PathBuf, u64)> {
    let base_dir = PathBuf::from("data/bench/generated");
    let mut files = Vec::new();

    if !base_dir.exists() {
        eprintln!(
            "Warning: {} does not exist. Run `cargo run --features cli -- json generate-suite` first.",
            base_dir.display()
        );
        return files;
    }

    // Iterate through pattern directories
    if let Ok(patterns) = std::fs::read_dir(&base_dir) {
        for pattern_entry in patterns.flatten() {
            let pattern_path = pattern_entry.path();
            if !pattern_path.is_dir() {
                continue;
            }

            let pattern_name = pattern_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");

            // Iterate through size files in each pattern directory
            if let Ok(size_files) = std::fs::read_dir(&pattern_path) {
                for file_entry in size_files.flatten() {
                    let file_path = file_entry.path();
                    if file_path.extension().is_some_and(|e| e == "json") {
                        let size_name = file_path
                            .file_stem()
                            .and_then(|n| n.to_str())
                            .unwrap_or("unknown");

                        let file_size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);

                        // Create display name: pattern/size (e.g., "comprehensive/10mb")
                        let display_name = format!("{}/{}", pattern_name, size_name);
                        files.push((display_name, file_path, file_size));
                    }
                }
            }
        }
    }

    // Sort by pattern name, then by file size
    files.sort_by(|a, b| {
        let (a_pattern, a_size) = a.0.split_once('/').unwrap_or((&a.0, ""));
        let (b_pattern, b_size) = b.0.split_once('/').unwrap_or((&b.0, ""));

        a_pattern.cmp(b_pattern).then_with(|| {
            // Parse sizes for proper numerical ordering
            parse_size_order(a_size).cmp(&parse_size_order(b_size))
        })
    });

    files
}

/// Parse size string to a comparable number for ordering.
fn parse_size_order(s: &str) -> u64 {
    let s = s.to_lowercase();
    if s.ends_with("gb") {
        s.trim_end_matches("gb")
            .parse::<u64>()
            .unwrap_or(0)
            .saturating_mul(1024 * 1024 * 1024)
    } else if s.ends_with("mb") {
        s.trim_end_matches("mb")
            .parse::<u64>()
            .unwrap_or(0)
            .saturating_mul(1024 * 1024)
    } else if s.ends_with("kb") {
        s.trim_end_matches("kb")
            .parse::<u64>()
            .unwrap_or(0)
            .saturating_mul(1024)
    } else {
        s.parse::<u64>().unwrap_or(0)
    }
}

/// Filter files by maximum size (skip files larger than this).
fn filter_by_max_size(
    files: Vec<(String, PathBuf, u64)>,
    max_bytes: u64,
) -> Vec<(String, PathBuf, u64)> {
    files
        .into_iter()
        .filter(|(_, _, size)| *size <= max_bytes)
        .collect()
}

/// Benchmark JSON indexing throughput on all discovered files.
#[cfg(target_arch = "x86_64")]
fn bench_json_files(c: &mut Criterion) {
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

        // AVX-512 (if available) - fastest on Intel Ice Lake+ and AMD Zen 4+
        if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512bw") {
            group.bench_with_input(BenchmarkId::new("AVX512", name), &bytes, |b, bytes| {
                b.iter(|| {
                    succinctly::json::simd::avx512::build_semi_index_standard(black_box(bytes))
                })
            });
        }

        // AVX2 (if available) - fastest on older x86_64
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

        // Scalar baseline
        group.bench_with_input(BenchmarkId::new("Scalar", name), &bytes, |b, bytes| {
            b.iter(|| succinctly::json::standard::build_semi_index(black_box(bytes)))
        });
    }

    group.finish();
}

/// Benchmark simple cursor on all discovered files (x86_64).
#[cfg(target_arch = "x86_64")]
fn bench_simple_cursor_files(c: &mut Criterion) {
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

        // AVX-512 (if available)
        if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512bw") {
            group.bench_with_input(BenchmarkId::new("AVX512", name), &bytes, |b, bytes| {
                b.iter(|| {
                    succinctly::json::simd::avx512::build_semi_index_simple(black_box(bytes))
                })
            });
        }

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

        // Scalar
        group.bench_with_input(BenchmarkId::new("Scalar", name), &bytes, |b, bytes| {
            b.iter(|| succinctly::json::simple::build_semi_index(black_box(bytes)))
        });
    }

    group.finish();
}

/// Benchmark full JsonIndex construction (includes BP RangeMin).
#[cfg(target_arch = "x86_64")]
fn bench_full_index(c: &mut Criterion) {
    use succinctly::json::JsonIndex;

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

/// Benchmark comparing patterns at a fixed size (10MB).
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

        group.bench_with_input(BenchmarkId::new("Scalar", pattern), &bytes, |b, bytes| {
            b.iter(|| succinctly::json::standard::build_semi_index(black_box(bytes)))
        });
    }

    group.finish();
}

// ARM/aarch64 benchmarks
#[cfg(target_arch = "aarch64")]
fn bench_json_files(c: &mut Criterion) {
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

        // Scalar baseline
        group.bench_with_input(BenchmarkId::new("Scalar", name), &bytes, |b, bytes| {
            b.iter(|| succinctly::json::standard::build_semi_index(black_box(bytes)))
        });

        // Full index
        group.bench_with_input(BenchmarkId::new("JsonIndex", name), &bytes, |b, bytes| {
            b.iter(|| JsonIndex::build(black_box(bytes)))
        });
    }

    group.finish();
}

#[cfg(target_arch = "aarch64")]
fn bench_simple_cursor_files(_c: &mut Criterion) {
    // Placeholder - can be expanded
}

#[cfg(target_arch = "aarch64")]
fn bench_full_index(_c: &mut Criterion) {
    // Already covered in bench_json_files for aarch64
}

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

        group.bench_with_input(BenchmarkId::new("Scalar", pattern), &bytes, |b, bytes| {
            b.iter(|| succinctly::json::standard::build_semi_index(black_box(bytes)))
        });
    }

    group.finish();
}

// Fallback for other architectures
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
fn bench_json_files(c: &mut Criterion) {
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

        group.bench_with_input(BenchmarkId::new("Scalar", name), &bytes, |b, bytes| {
            b.iter(|| succinctly::json::standard::build_semi_index(black_box(bytes)))
        });

        group.bench_with_input(BenchmarkId::new("JsonIndex", name), &bytes, |b, bytes| {
            b.iter(|| JsonIndex::build(black_box(bytes)))
        });
    }

    group.finish();
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
fn bench_simple_cursor_files(_c: &mut Criterion) {}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
fn bench_full_index(_c: &mut Criterion) {}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
fn bench_pattern_comparison(_c: &mut Criterion) {}

criterion_group!(
    benches,
    bench_json_files,
    bench_simple_cursor_files,
    bench_full_index,
    bench_pattern_comparison,
);

criterion_main!(benches);
