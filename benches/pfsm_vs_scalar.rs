//! Benchmark comparing PFSM (table-based) vs true scalar implementations.
//!
//! Both use the standard cursor (4-state, marks value starts).
//! - PFSM: Uses 256-entry lookup tables for state transitions
//! - Scalar: Byte-by-byte processing with explicit state machine
use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use std::fs;
use succinctly::json::standard;

fn benchmark_pfsm_vs_scalar(c: &mut Criterion) {
    let test_files = [
        ("1kb", "data/bench/generated/comprehensive/1kb.json"),
        ("10kb", "data/bench/generated/comprehensive/10kb.json"),
        ("100kb", "data/bench/generated/comprehensive/100kb.json"),
    ];

    for (name, path) in test_files {
        let json = match fs::read(path) {
            Ok(data) => Box::leak(data.into_boxed_slice()),
            Err(_) => {
                eprintln!("Warning: {path} not found, skipping");
                continue;
            }
        };

        let mut group = c.benchmark_group(format!("pfsm_vs_scalar/{name}"));
        group.throughput(Throughput::Bytes(json.len() as u64));

        // PFSM: table-based state machine
        group.bench_function("PFSM", |b| {
            b.iter(|| standard::build_semi_index(black_box(json)));
        });

        // Scalar: byte-by-byte fallback
        group.bench_function("Scalar", |b| {
            b.iter(|| standard::build_semi_index_scalar(black_box(json)));
        });

        group.finish();
    }
}

criterion_group!(benches, benchmark_pfsm_vs_scalar);
criterion_main!(benches);
