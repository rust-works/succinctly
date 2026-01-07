/// Benchmark the default `build_semi_index` which now uses PFSM optimized.
use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use std::fs;
use succinctly::json::standard;

fn benchmark_standard_semi_index(c: &mut Criterion) {
    let test_files = [
        ("1kb", "data/bench/generated/comprehensive/1kb.json"),
        ("10kb", "data/bench/generated/comprehensive/10kb.json"),
        ("100kb", "data/bench/generated/comprehensive/100kb.json"),
    ];

    for (name, path) in test_files {
        let json = match fs::read(path) {
            Ok(data) => Box::leak(data.into_boxed_slice()),
            Err(_) => {
                eprintln!("Warning: {} not found, skipping", path);
                continue;
            }
        };

        let mut group = c.benchmark_group(format!("standard_cursor/{}", name));
        group.throughput(Throughput::Bytes(json.len() as u64));

        // Benchmark default implementation (now PFSM optimized)
        group.bench_function("build_semi_index", |b| {
            b.iter(|| standard::build_semi_index(black_box(json)));
        });

        // Benchmark scalar fallback for comparison
        group.bench_function("build_semi_index_scalar", |b| {
            b.iter(|| standard::build_semi_index_scalar(black_box(json)));
        });

        group.finish();
    }
}

criterion_group!(benches, benchmark_standard_semi_index);
criterion_main!(benches);
