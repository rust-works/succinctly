//! Benchmark comparing PFSM (table-based) vs SIMD implementations for JSON semi-indexing.
//!
//! PFSM (Parallel Finite State Machine) uses lookup tables for state transitions.
//! SIMD implementations use vectorized instructions (AVX2/NEON) for parallel processing.
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::fs;
use std::path::PathBuf;

fn discover_json_files() -> Vec<(String, PathBuf, u64)> {
    let base_dir = PathBuf::from("data/bench/generated");
    let mut files = Vec::new();

    if !base_dir.exists() {
        eprintln!(
            "Warning: {} not found. Run: cargo run --features cli -- json generate-suite",
            base_dir.display()
        );
        return files;
    }

    // Focus on specific sizes and patterns
    let target_files = [
        "comprehensive/10kb.json",
        "comprehensive/100kb.json",
        "comprehensive/1mb.json",
        "users/10kb.json",
        "nested/10kb.json",
        "arrays/10kb.json",
    ];

    for file in target_files {
        let path = base_dir.join(file);
        if let Ok(metadata) = fs::metadata(&path) {
            let pattern = path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");
            let size = path
                .file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");
            let name = format!("{}/{}", pattern, size);
            files.push((name, path, metadata.len()));
        }
    }

    files
}

fn bench_pfsm_vs_simd(c: &mut Criterion) {
    let files = discover_json_files();

    if files.is_empty() {
        eprintln!("No JSON files found for benchmarking.");
        return;
    }

    for (name, path, file_size) in files {
        let json = match fs::read(&path) {
            Ok(data) => Box::leak(data.into_boxed_slice()),
            Err(e) => {
                eprintln!("Failed to read {}: {}", path.display(), e);
                continue;
            }
        };

        let mut group = c.benchmark_group("pfsm_vs_simd");
        group.throughput(Throughput::Bytes(file_size));

        // Benchmark PFSM (table-based state machine)
        group.bench_with_input(BenchmarkId::new("PFSM", &name), json, |b, json| {
            b.iter(|| succinctly::json::standard::build_semi_index(black_box(json)));
        });

        // Benchmark AVX2 SIMD if available
        #[cfg(target_arch = "x86_64")]
        if is_x86_feature_detected!("avx2") {
            group.bench_with_input(BenchmarkId::new("AVX2", &name), json, |b, json| {
                b.iter(|| succinctly::json::simd::avx2::build_semi_index_standard(black_box(json)));
            });
        }

        // Benchmark AVX2 PFSM shuffle composition (experimental)
        #[cfg(target_arch = "x86_64")]
        if is_x86_feature_detected!("avx2") {
            group.bench_with_input(BenchmarkId::new("AVX2-PFSM", &name), json, |b, json| {
                b.iter(|| {
                    succinctly::json::simd::avx2_pfsm::build_semi_index_standard(black_box(json))
                });
            });
        }

        // Benchmark NEON SIMD on ARM
        #[cfg(target_arch = "aarch64")]
        group.bench_with_input(BenchmarkId::new("NEON", &name), json, |b, json| {
            b.iter(|| succinctly::json::simd::neon::build_semi_index_standard(black_box(json)));
        });

        group.finish();
    }
}

criterion_group!(benches, bench_pfsm_vs_simd);
criterion_main!(benches);
