use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use std::fs;
use succinctly::json::{pfsm, pfsm_optimized, pfsm_tables::PfsmState, standard, BitWriter};

fn benchmark_pfsm_vs_standard(c: &mut Criterion) {
    // Test with real benchmark files
    let test_files = [
        ("1kb", "data/bench/generated/comprehensive/1kb.json"),
        ("10kb", "data/bench/generated/comprehensive/10kb.json"),
        ("100kb", "data/bench/generated/comprehensive/100kb.json"),
        ("1mb", "data/bench/generated/comprehensive/1mb.json"),
    ];

    for (name, path) in test_files {
        let json = match fs::read(path) {
            Ok(data) => Box::leak(data.into_boxed_slice()),
            Err(_) => {
                eprintln!(
                    "Warning: {} not found, skipping. Run: cargo run --features cli -- json generate-suite",
                    path
                );
                continue;
            }
        };

        let mut group = c.benchmark_group(format!("pfsm_comparison/{}", name));
        group.throughput(Throughput::Bytes(json.len() as u64));

        // Benchmark PFSM (scalar, two-pass)
        group.bench_function("pfsm_scalar", |b| {
            b.iter(|| {
                let mut ib = BitWriter::new();
                let mut bp = BitWriter::new();
                pfsm::pfsm_process_chunk(black_box(json), PfsmState::InJson, &mut ib, &mut bp);
                (ib.finish(), bp.finish())
            });
        });

        // Benchmark PFSM optimized (single-pass, no allocation)
        group.bench_function("pfsm_optimized", |b| {
            b.iter(|| {
                let mut ib = BitWriter::new();
                let mut bp = BitWriter::new();
                pfsm_optimized::pfsm_process_chunk_optimized(
                    black_box(json),
                    PfsmState::InJson,
                    &mut ib,
                    &mut bp,
                );
                (ib.finish(), bp.finish())
            });
        });

        // Benchmark standard scalar implementation
        group.bench_function("standard_scalar", |b| {
            b.iter(|| standard::build_semi_index(black_box(json)));
        });

        // Benchmark AVX2 if available
        #[cfg(target_arch = "x86_64")]
        if is_x86_feature_detected!("avx2") {
            group.bench_function("standard_avx2", |b| {
                b.iter(|| succinctly::json::simd::avx2::build_semi_index_standard(black_box(json)));
            });
        }

        group.finish();
    }
}

criterion_group!(benches, benchmark_pfsm_vs_standard);
criterion_main!(benches);
