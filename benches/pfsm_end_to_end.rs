///! End-to-end benchmark: PFSM vs existing implementations across all JSON patterns
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::fs;
use std::path::PathBuf;
use succinctly::json::{pfsm, pfsm_tables::PfsmState, BitWriter};

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

fn bench_end_to_end(c: &mut Criterion) {
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

        let mut group = c.benchmark_group("pfsm_end_to_end");
        group.throughput(Throughput::Bytes(file_size));

        // Benchmark PFSM (with BMI2/AVX2 auto-dispatch)
        group.bench_with_input(BenchmarkId::new("pfsm_auto", &name), json, |b, json| {
            b.iter(|| {
                let mut ib = BitWriter::new();
                let mut bp = BitWriter::new();
                pfsm::pfsm_process_chunk(black_box(json), PfsmState::InJson, &mut ib, &mut bp);
                (ib.finish(), bp.finish())
            });
        });

        // Benchmark standard scalar
        group.bench_with_input(
            BenchmarkId::new("standard_scalar", &name),
            json,
            |b, json| {
                b.iter(|| succinctly::json::standard::build_semi_index(black_box(json)));
            },
        );

        // Benchmark AVX2 if available
        #[cfg(target_arch = "x86_64")]
        if is_x86_feature_detected!("avx2") {
            group.bench_with_input(BenchmarkId::new("standard_avx2", &name), json, |b, json| {
                b.iter(|| succinctly::json::simd::avx2::build_semi_index_standard(black_box(json)));
            });
        }

        group.finish();
    }
}

criterion_group!(benches, bench_end_to_end);
criterion_main!(benches);
