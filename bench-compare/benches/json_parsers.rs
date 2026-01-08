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
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use succinctly::json::light::{JsonCursor, JsonIndex, StandardJson};

// ============================================================================
// Peak Memory Tracking Allocator
// ============================================================================

struct PeakAllocator {
    current: AtomicUsize,
    peak: AtomicUsize,
}

impl PeakAllocator {
    const fn new() -> Self {
        Self {
            current: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        }
    }

    fn reset(&self) {
        self.current.store(0, Ordering::SeqCst);
        self.peak.store(0, Ordering::SeqCst);
    }

    fn peak(&self) -> usize {
        self.peak.load(Ordering::SeqCst)
    }
}

unsafe impl GlobalAlloc for PeakAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let old = self.current.fetch_add(layout.size(), Ordering::SeqCst);
            let new = old + layout.size();
            // Update peak if this is a new high
            let mut peak = self.peak.load(Ordering::SeqCst);
            while new > peak {
                match self
                    .peak
                    .compare_exchange_weak(peak, new, Ordering::SeqCst, Ordering::SeqCst)
                {
                    Ok(_) => break,
                    Err(p) => peak = p,
                }
            }
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.current.fetch_sub(layout.size(), Ordering::SeqCst);
        unsafe { System.dealloc(ptr, layout) };
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            let old_size = layout.size();
            if new_size > old_size {
                let diff = new_size - old_size;
                let old = self.current.fetch_add(diff, Ordering::SeqCst);
                let new = old + diff;
                let mut peak = self.peak.load(Ordering::SeqCst);
                while new > peak {
                    match self.peak.compare_exchange_weak(
                        peak,
                        new,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    ) {
                        Ok(_) => break,
                        Err(p) => peak = p,
                    }
                }
            } else {
                self.current
                    .fetch_sub(old_size - new_size, Ordering::SeqCst);
            }
        }
        new_ptr
    }
}

#[global_allocator]
static ALLOCATOR: PeakAllocator = PeakAllocator::new();

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

/// Count nodes in simd-json OwnedValue
fn count_simd_owned(v: &simd_json::OwnedValue) -> usize {
    match v {
        simd_json::OwnedValue::Array(arr) => 1 + arr.iter().map(count_simd_owned).sum::<usize>(),
        simd_json::OwnedValue::Object(obj) => 1 + obj.values().map(count_simd_owned).sum::<usize>(),
        _ => 1,
    }
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
        let simd_value: simd_json::OwnedValue = simd_json::to_owned_value(&mut simd_bytes).unwrap();

        let sonic_value: sonic_rs::Value = sonic_rs::from_slice(&bytes).unwrap();

        let succ_index = JsonIndex::build(&bytes);

        let mut group = c.benchmark_group(format!("traverse_only/{}", name));
        group.throughput(Throughput::Bytes(file_size));
        let sample_size = match *name {
            "1kb" | "10kb" | "100kb" => 100,
            "1mb" => 50,
            "10mb" => 20,
            "100mb" => 10,
            _ => 20,
        };
        group.sample_size(sample_size);

        // serde_json traverse
        group.bench_function("serde_json", |b| {
            b.iter(|| count_serde(black_box(&serde_value)))
        });

        // simd-json traverse (owned value)
        group.bench_function("simd_json", |b| {
            b.iter(|| count_simd_owned(black_box(&simd_value)))
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

/// Measure actual peak memory usage for each parser
fn measure_peak_memory(c: &mut Criterion) {
    let mut group = c.benchmark_group("peak_memory");
    group.sample_size(10);

    for (name, path) in TEST_FILES {
        let Some(bytes) = load_test_file(path) else {
            continue;
        };

        let json_size = bytes.len();
        let json_kb = json_size as f64 / 1024.0;
        let json_mb = json_size as f64 / 1024.0 / 1024.0;

        println!("\n{:=^70}", "");
        if json_mb >= 1.0 {
            println!(" Peak Memory Usage: {} ({:.2} MB) ", name, json_mb);
        } else {
            println!(" Peak Memory Usage: {} ({:.2} KB) ", name, json_kb);
        }
        println!("{:=^70}", "");

        // Measure serde_json
        ALLOCATOR.reset();
        let _serde_value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let serde_peak = ALLOCATOR.peak();
        drop(_serde_value);

        // Measure simd-json
        ALLOCATOR.reset();
        let mut simd_bytes = bytes.clone();
        let _simd_value: simd_json::OwnedValue =
            simd_json::to_owned_value(&mut simd_bytes).unwrap();
        let simd_peak = ALLOCATOR.peak();
        drop(_simd_value);
        drop(simd_bytes);

        // Measure sonic-rs
        ALLOCATOR.reset();
        let _sonic_value: sonic_rs::Value = sonic_rs::from_slice(&bytes).unwrap();
        let sonic_peak = ALLOCATOR.peak();
        drop(_sonic_value);

        // Measure succinctly
        ALLOCATOR.reset();
        let _succ_index = JsonIndex::build(&bytes);
        let succ_peak = ALLOCATOR.peak();
        drop(_succ_index);

        // Print results
        println!(
            "\n{:<20} {:>15} {:>12} {:>12}",
            "Parser", "Peak Memory", "Overhead", "vs JSON"
        );
        println!("{:-<20} {:-<15} {:-<12} {:-<12}", "", "", "", "");

        let print_row = |parser: &str, peak: usize| {
            let overhead_pct = peak as f64 / json_size as f64 * 100.0;
            let ratio = peak as f64 / json_size as f64;
            if peak >= 1024 * 1024 {
                println!(
                    "{:<20} {:>12.2} MB {:>10.1}% {:>11.2}x",
                    parser,
                    peak as f64 / 1024.0 / 1024.0,
                    overhead_pct,
                    ratio
                );
            } else {
                println!(
                    "{:<20} {:>12.2} KB {:>10.1}% {:>11.2}x",
                    parser,
                    peak as f64 / 1024.0,
                    overhead_pct,
                    ratio
                );
            }
        };

        print_row("serde_json", serde_peak);
        print_row("simd-json", simd_peak);
        print_row("sonic-rs", sonic_peak);
        print_row("succinctly", succ_peak);
        if json_mb >= 1.0 {
            println!(
                "{:<20} {:>12.2} MB {:>12} {:>12}",
                "(original JSON)", json_mb, "---", "1.00x"
            );
        } else {
            println!(
                "{:<20} {:>12.2} KB {:>12} {:>12}",
                "(original JSON)", json_kb, "---", "1.00x"
            );
        }

        // Memory efficiency comparison
        println!("\nMemory efficiency (smaller is better):");
        if succ_peak > 0 {
            println!(
                "  succinctly uses {:.1}x LESS memory than serde_json",
                serde_peak as f64 / succ_peak as f64
            );
            println!(
                "  succinctly uses {:.1}x LESS memory than simd-json",
                simd_peak as f64 / succ_peak as f64
            );
            println!(
                "  succinctly uses {:.1}x LESS memory than sonic-rs",
                sonic_peak as f64 / succ_peak as f64
            );
        }

        // Dummy benchmark to satisfy criterion
        group.bench_function(format!("{}_measure", name), |b| {
            b.iter(|| black_box(succ_peak))
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_parse_only,
    bench_parse_and_traverse,
    bench_traverse_only,
    measure_peak_memory,
);
criterion_main!(benches);
