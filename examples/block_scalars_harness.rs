//! Standalone, fixed-shot `YamlIndex::build` harness for a single `yaml/block_scalars`
//! shape from `benches/yaml_bench.rs`, for use under `valgrind --tool=cachegrind` or
//! `perf stat -e instructions,cycles` (issue #595).
//!
//! Criterion's adaptive, timing-based iteration count doesn't work under instrumentation
//! that slows execution 20-50x — the two binaries being compared would end up running a
//! different number of iterations, making total instruction counts incomparable. This
//! harness runs a fixed iteration count instead.
//!
//! Run with:
//! ```bash
//! cargo build --release --example block_scalars_harness
//! valgrind --tool=cachegrind ./target/release/examples/block_scalars_harness long_100x100lines 50
//! ```

use std::env;
use succinctly::yaml::YamlIndex;

/// Mirrors `generate_block_scalars` in `benches/yaml_bench.rs`.
fn generate_block_scalars(count: usize, lines_per_block: usize) -> Vec<u8> {
    let mut yaml = Vec::with_capacity(count * lines_per_block * 50);
    for i in 0..count {
        yaml.extend_from_slice(format!("block{i}: |\n").as_bytes());
        for j in 0..lines_per_block {
            yaml.extend_from_slice(format!("  This is line {j} of block scalar {i}\n").as_bytes());
        }
    }
    yaml
}

/// Mirrors `generate_long_block_scalars` in `benches/yaml_bench.rs`.
fn generate_long_block_scalars(count: usize, lines_per_block: usize) -> Vec<u8> {
    let mut yaml = Vec::with_capacity(count * lines_per_block * 100);
    let long_line = "x".repeat(80);
    for i in 0..count {
        yaml.extend_from_slice(format!("content{i}: |\n").as_bytes());
        for _ in 0..lines_per_block {
            yaml.extend_from_slice(format!("  {long_line}\n").as_bytes());
        }
    }
    yaml
}

fn shape(name: &str) -> Vec<u8> {
    match name {
        "10x10lines" => generate_block_scalars(10, 10),
        "50x50lines" => generate_block_scalars(50, 50),
        "100x100lines" => generate_block_scalars(100, 100),
        "10x1000lines" => generate_block_scalars(10, 1000),
        "long_10x100lines" => generate_long_block_scalars(10, 100),
        "long_50x100lines" => generate_long_block_scalars(50, 100),
        "long_100x100lines" => generate_long_block_scalars(100, 100),
        other => {
            eprintln!("unknown shape {other:?} - see yaml/block_scalars in benches/yaml_bench.rs");
            std::process::exit(1);
        }
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let shape_name = args.get(1).map_or("long_100x100lines", String::as_str);
    let iterations: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(50);

    let yaml = shape(shape_name);

    // Cheap structural fingerprint, summed across iterations, for the output-identity
    // gate (docs/guides/benchmarking.md rule 4): if the "before" and "after" binaries
    // build different index shapes, the checksum diverges even though nothing here
    // depends on wall-clock time. Deliberately avoids `select_heap_size()` — it doesn't
    // exist on the pre-CS-Poppy (`WithSelect`) commit this harness must also compile
    // against unmodified.
    let mut checksum: u64 = 0;
    for _ in 0..iterations {
        let index = YamlIndex::build(std::hint::black_box(yaml.as_slice())).unwrap();
        checksum = checksum
            .wrapping_add(index.bp().len() as u64)
            .wrapping_add(index.bp().total_ones() as u64)
            .wrapping_add(index.bp().words().len() as u64)
            .wrapping_add(index.ib_len() as u64)
            .wrapping_add(index.ty_len() as u64);
        std::hint::black_box(&index);
    }

    println!(
        "shape={shape_name} doc_bytes={} iterations={iterations} checksum={checksum}",
        yaml.len()
    );
}
