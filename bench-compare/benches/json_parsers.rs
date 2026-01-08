//! Benchmark comparing succinctly against other Rust JSON parsers.
//!
//! Compares:
//! - serde_json: Standard DOM parser
//! - simd-json: SIMD-accelerated parser (simdjson port)
//! - sonic-rs: SIMD + arena-based parser
//! - succinctly: Semi-index with balanced parentheses
//!
//! Run from the bench-compare directory:
//! ```bash
//! cargo bench --bench json_parsers
//! ```

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use sonic_rs::{JsonContainerTrait, JsonValueTrait};
use succinctly::json::light::{JsonCursor, JsonIndex, StandardJson};

/// Test file paths (relative to workspace root)
/// Note: 1GB excluded - too large for criterion's minimum sample requirements
const TEST_FILES: &[(&str, &str)] = &[
    ("1kb", "../data/bench/generated/comprehensive/1kb.json"),
    ("10kb", "../data/bench/generated/comprehensive/10kb.json"),
    ("100kb", "../data/bench/generated/comprehensive/100kb.json"),
    ("1mb", "../data/bench/generated/comprehensive/1mb.json"),
    ("10mb", "../data/bench/generated/comprehensive/10mb.json"),
    ("100mb", "../data/bench/generated/comprehensive/100mb.json"),
];

/// Load test file or return None
fn load_test_file(path: &str) -> Option<Vec<u8>> {
    let path = std::path::Path::new(path);
    if !path.exists() {
        eprintln!("Skipping benchmark: {} not found", path.display());
        eprintln!("Generate with: cd .. && cargo run --release --features cli -- json generate 10mb -o data/bench/generated/comprehensive/10mb.json");
        return None;
    }
    Some(std::fs::read(path).expect("Failed to read test file"))
}

// ============================================================================
// Traversal functions for each parser
// ============================================================================

/// Count nodes using serde_json
fn count_serde(v: &serde_json::Value) -> usize {
    match v {
        serde_json::Value::Array(arr) => 1 + arr.iter().map(count_serde).sum::<usize>(),
        serde_json::Value::Object(obj) => 1 + obj.values().map(count_serde).sum::<usize>(),
        _ => 1,
    }
}

/// Count nodes using simd-json (borrowed value)
fn count_simd_json(v: &simd_json::BorrowedValue) -> usize {
    match v {
        simd_json::BorrowedValue::Array(arr) => 1 + arr.iter().map(count_simd_json).sum::<usize>(),
        simd_json::BorrowedValue::Object(obj) => {
            1 + obj.values().map(count_simd_json).sum::<usize>()
        }
        _ => 1,
    }
}

/// Count nodes using sonic-rs
fn count_sonic(v: &sonic_rs::Value) -> usize {
    if v.is_array() {
        1 + v.as_array().unwrap().iter().map(count_sonic).sum::<usize>()
    } else if v.is_object() {
        1 + v
            .as_object()
            .unwrap()
            .iter()
            .map(|(_, v)| count_sonic(v))
            .sum::<usize>()
    } else {
        1
    }
}

/// Count nodes using succinctly with value() - calls text_position on every node
fn count_succinctly_value(v: StandardJson) -> usize {
    match v {
        StandardJson::Array(elements) => {
            let mut count = 1;
            for elem in elements {
                count += count_succinctly_value(elem);
            }
            count
        }
        StandardJson::Object(entries) => {
            let mut count = 1;
            for entry in entries {
                count += count_succinctly_value(entry.value());
            }
            count
        }
        _ => 1,
    }
}

/// Count nodes using succinctly with children() - BP-only, no text_position
fn count_succinctly_fast(cursor: JsonCursor) -> usize {
    let mut count = 1;
    for child in cursor.children() {
        count += count_succinctly_fast(child);
    }
    count
}

// ============================================================================
// Benchmarks
// ============================================================================

/// Benchmark parse/index time only (no traversal)
fn bench_parse_only(c: &mut Criterion) {
    for (name, path) in TEST_FILES {
        let Some(bytes) = load_test_file(path) else {
            continue;
        };
        let file_size = bytes.len() as u64;

        let mut group = c.benchmark_group(format!("parse_only/{}", name));
        group.throughput(Throughput::Bytes(file_size));
        let sample_size = match *name {
            "1kb" | "10kb" | "100kb" => 100,
            "1mb" => 50,
            "10mb" => 20,
            "100mb" => 10,
            "1gb" => 5,
            _ => 20,
        };
        group.sample_size(sample_size);

        // serde_json
        group.bench_function("serde_json", |b| {
            b.iter(|| {
                let v: serde_json::Value = serde_json::from_slice(black_box(&bytes)).unwrap();
                black_box(v)
            })
        });

        // simd-json (needs mutable copy, use owned to avoid lifetime issues)
        group.bench_function("simd_json", |b| {
            b.iter(|| {
                let mut bytes_copy = bytes.clone();
                let v: simd_json::OwnedValue =
                    simd_json::to_owned_value(black_box(&mut bytes_copy)).unwrap();
                black_box(v)
            })
        });

        // sonic-rs
        group.bench_function("sonic_rs", |b| {
            b.iter(|| {
                let v: sonic_rs::Value = sonic_rs::from_slice(black_box(&bytes)).unwrap();
                black_box(v)
            })
        });

        // succinctly (index only)
        group.bench_function("succinctly", |b| {
            b.iter(|| {
                let index = JsonIndex::build(black_box(&bytes));
                black_box(index)
            })
        });

        group.finish();
    }
}

/// Benchmark full pipeline: parse + traverse all nodes
fn bench_parse_and_traverse(c: &mut Criterion) {
    for (name, path) in TEST_FILES {
        let Some(bytes) = load_test_file(path) else {
            continue;
        };
        let file_size = bytes.len() as u64;

        let mut group = c.benchmark_group(format!("parse_traverse/{}", name));
        group.throughput(Throughput::Bytes(file_size));
        let sample_size = match *name {
            "1kb" | "10kb" | "100kb" => 100,
            "1mb" => 50,
            "10mb" => 20,
            "100mb" => 10,
            "1gb" => 5,
            _ => 20,
        };
        group.sample_size(sample_size);

        // serde_json: parse + traverse
        group.bench_function("serde_json", |b| {
            b.iter(|| {
                let v: serde_json::Value = serde_json::from_slice(black_box(&bytes)).unwrap();
                count_serde(&v)
            })
        });

        // simd-json: parse + traverse
        group.bench_function("simd_json", |b| {
            b.iter(|| {
                let mut bytes_copy = bytes.clone();
                let v: simd_json::BorrowedValue =
                    simd_json::to_borrowed_value(black_box(&mut bytes_copy)).unwrap();
                count_simd_json(&v)
            })
        });

        // sonic-rs: parse + traverse
        group.bench_function("sonic_rs", |b| {
            b.iter(|| {
                let v: sonic_rs::Value = sonic_rs::from_slice(black_box(&bytes)).unwrap();
                count_sonic(&v)
            })
        });

        // succinctly with value() - older API, calls text_position on every node
        group.bench_function("succinctly_value", |b| {
            b.iter(|| {
                let index = JsonIndex::build(black_box(&bytes));
                let root = index.root(&bytes);
                count_succinctly_value(root.value())
            })
        });

        // succinctly with children() - optimized, BP-only traversal
        group.bench_function("succinctly_fast", |b| {
            b.iter(|| {
                let index = JsonIndex::build(black_box(&bytes));
                let root = index.root(&bytes);
                count_succinctly_fast(root)
            })
        });

        group.finish();
    }
}

/// Benchmark traverse only (parsing done beforehand)
fn bench_traverse_only(c: &mut Criterion) {
    for (name, path) in TEST_FILES {
        let Some(bytes) = load_test_file(path) else {
            continue;
        };
        let file_size = bytes.len() as u64;

        // Pre-parse everything
        let serde_value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        let mut simd_bytes = bytes.clone();
        let simd_value: simd_json::OwnedValue =
            simd_json::to_owned_value(&mut simd_bytes).unwrap();

        let sonic_value: sonic_rs::Value = sonic_rs::from_slice(&bytes).unwrap();

        let succ_index = JsonIndex::build(&bytes);

        let mut group = c.benchmark_group(format!("traverse_only/{}", name));
        group.throughput(Throughput::Bytes(file_size));
        let sample_size = match *name {
            "1kb" | "10kb" | "100kb" => 100,
            "1mb" => 50,
            "10mb" => 20,
            "100mb" => 10,
            "1gb" => 5,
            _ => 20,
        };
        group.sample_size(sample_size);

        // serde_json traverse
        group.bench_function("serde_json", |b| {
            b.iter(|| count_serde(black_box(&serde_value)))
        });

        // simd-json traverse (owned value)
        group.bench_function("simd_json", |b| {
            b.iter(|| {
                fn count_owned(v: &simd_json::OwnedValue) -> usize {
                    match v {
                        simd_json::OwnedValue::Array(arr) => {
                            1 + arr.iter().map(count_owned).sum::<usize>()
                        }
                        simd_json::OwnedValue::Object(obj) => {
                            1 + obj.values().map(count_owned).sum::<usize>()
                        }
                        _ => 1,
                    }
                }
                count_owned(black_box(&simd_value))
            })
        });

        // sonic-rs traverse
        group.bench_function("sonic_rs", |b| {
            b.iter(|| count_sonic(black_box(&sonic_value)))
        });

        // succinctly traverse with value() (calls text_position)
        group.bench_function("succinctly_value", |b| {
            b.iter(|| {
                let root = succ_index.root(black_box(&bytes));
                count_succinctly_value(root.value())
            })
        });

        // succinctly traverse with children() (BP-only)
        group.bench_function("succinctly_fast", |b| {
            b.iter(|| {
                let root = succ_index.root(black_box(&bytes));
                count_succinctly_fast(root)
            })
        });

        group.finish();
    }
}

/// Measure memory overhead for each parser (prints comparison table)
fn measure_memory_overhead(c: &mut Criterion) {
    // This benchmark prints memory statistics rather than measuring time
    let mut group = c.benchmark_group("memory_overhead");
    group.sample_size(10);

    for (name, path) in TEST_FILES {
        let Some(bytes) = load_test_file(path) else {
            continue;
        };

        let json_size = bytes.len();
        let json_mb = json_size as f64 / 1024.0 / 1024.0;

        println!("\n{:=^60}", "");
        println!(" Memory Overhead Analysis: {} ({:.2} MB) ", name, json_mb);
        println!("{:=^60}", "");

        // ===== serde_json =====
        let serde_value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let serde_node_count = count_serde(&serde_value);
        // serde_json::Value is 32 bytes per node on 64-bit (enum discriminant + 24 bytes data)
        // Plus string data is owned (copied), plus hashmap overhead for objects
        let serde_base = serde_node_count * 32;
        // Estimate string storage: ~30% of JSON is typically string content
        let serde_strings = (json_size as f64 * 0.3) as usize;
        // HashMap overhead: ~48 bytes per entry for BTreeMap internals
        let serde_map_overhead = serde_node_count / 4 * 48; // rough estimate
        let serde_total = serde_base + serde_strings + serde_map_overhead;

        // ===== simd-json =====
        let mut simd_bytes = bytes.clone();
        let simd_value: simd_json::OwnedValue = simd_json::to_owned_value(&mut simd_bytes).unwrap();
        // simd-json OwnedValue uses halfbrown HashMap (more efficient)
        // Node size similar to serde, but uses arena-like allocation
        let simd_node_count = count_simd_owned(&simd_value);
        let simd_base = simd_node_count * 32;
        let simd_strings = (json_size as f64 * 0.3) as usize;
        let simd_map_overhead = simd_node_count / 4 * 32; // halfbrown is more efficient
        let simd_total = simd_base + simd_strings + simd_map_overhead;

        // ===== sonic-rs =====
        let sonic_value: sonic_rs::Value = sonic_rs::from_slice(&bytes).unwrap();
        let sonic_node_count = count_sonic(&sonic_value);
        // sonic-rs uses arena allocation with 24-byte nodes + separate string storage
        let sonic_base = sonic_node_count * 24;
        let sonic_strings = (json_size as f64 * 0.3) as usize;
        let sonic_total = sonic_base + sonic_strings;

        // ===== succinctly =====
        let succ_index = JsonIndex::build(&bytes);
        // IB: 1 bit per byte of input (rounded to u64 words)
        let ib_bits = json_size;
        let ib_bytes = ib_bits.div_ceil(64) * 8;

        // BP: 2 bits per structural character (open + close parens)
        // Plus RangeMin structure for navigation
        let bp_len = succ_index.bp().len();
        let bp_words = bp_len.div_ceil(64);
        let bp_bytes = bp_words * 8;

        // RangeMin L0: 2 bytes per word (min_excess + cum_excess as i8)
        let rm_l0_bytes = bp_words * 2;
        // RangeMin L1: 4 bytes per 32 words
        let rm_l1_bytes = bp_words.div_ceil(32) * 4;
        // RangeMin L2: 4 bytes per 1024 words
        let rm_l2_bytes = bp_words.div_ceil(1024) * 4;
        let rm_total = rm_l0_bytes + rm_l1_bytes + rm_l2_bytes;

        // IB rank directory for select1 (cumulative popcount per word)
        let ib_rank_bytes = ib_bits.div_ceil(64) * 4; // u32 per word

        let succ_total = ib_bytes + ib_rank_bytes + bp_bytes + rm_total;

        // Print results
        println!("\n{:<20} {:>12} {:>12} {:>10}", "Parser", "Est. Size", "Overhead", "Ratio");
        println!("{:-<20} {:-<12} {:-<12} {:-<10}", "", "", "", "");

        let print_row = |name: &str, size: usize| {
            let mb = size as f64 / 1024.0 / 1024.0;
            let overhead = size as f64 / json_size as f64 * 100.0;
            let ratio = size as f64 / json_size as f64;
            println!("{:<20} {:>10.2} MB {:>10.1}% {:>10.2}x", name, mb, overhead, ratio);
        };

        print_row("serde_json", serde_total);
        print_row("simd-json", simd_total);
        print_row("sonic-rs", sonic_total);
        print_row("succinctly", succ_total);
        println!("{:<20} {:>10.2} MB {:>10}  {:>10}", "(original JSON)", json_mb, "---", "1.00x");

        // Detailed succinctly breakdown
        println!("\nSuccinctly index breakdown:");
        println!("  IB (interest bits):     {:>8.2} MB ({:.1}%)",
            ib_bytes as f64 / 1024.0 / 1024.0,
            ib_bytes as f64 / json_size as f64 * 100.0);
        println!("  IB rank directory:      {:>8.2} MB ({:.1}%)",
            ib_rank_bytes as f64 / 1024.0 / 1024.0,
            ib_rank_bytes as f64 / json_size as f64 * 100.0);
        println!("  BP (balanced parens):   {:>8.2} MB ({:.1}%)",
            bp_bytes as f64 / 1024.0 / 1024.0,
            bp_bytes as f64 / json_size as f64 * 100.0);
        println!("  RangeMin structure:     {:>8.2} MB ({:.1}%)",
            rm_total as f64 / 1024.0 / 1024.0,
            rm_total as f64 / json_size as f64 * 100.0);
        println!("  --------------------------------");
        println!("  Total index overhead:   {:>8.2} MB ({:.1}%)",
            succ_total as f64 / 1024.0 / 1024.0,
            succ_total as f64 / json_size as f64 * 100.0);

        // Memory efficiency comparison
        println!("\nMemory efficiency (smaller is better):");
        println!("  succinctly uses {:.1}x LESS memory than serde_json",
            serde_total as f64 / succ_total as f64);
        println!("  succinctly uses {:.1}x LESS memory than simd-json",
            simd_total as f64 / succ_total as f64);
        println!("  succinctly uses {:.1}x LESS memory than sonic-rs",
            sonic_total as f64 / succ_total as f64);

        // Dummy benchmark to satisfy criterion
        group.bench_function(format!("{}_measure", name), |b| {
            b.iter(|| black_box(succ_total))
        });
    }

    group.finish();
}

/// Count nodes in simd-json OwnedValue
fn count_simd_owned(v: &simd_json::OwnedValue) -> usize {
    match v {
        simd_json::OwnedValue::Array(arr) => 1 + arr.iter().map(count_simd_owned).sum::<usize>(),
        simd_json::OwnedValue::Object(obj) => {
            1 + obj.values().map(count_simd_owned).sum::<usize>()
        }
        _ => 1,
    }
}

criterion_group!(
    benches,
    bench_parse_only,
    bench_parse_and_traverse,
    bench_traverse_only,
    measure_memory_overhead,
);
criterion_main!(benches);
