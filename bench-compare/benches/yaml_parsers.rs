//! Benchmark comparing succinctly YAML against serde_yaml.
//!
//! Compares:
//! - serde_yaml: Standard YAML parser (based on yaml-rust2)
//! - succinctly: Semi-index with balanced parentheses
//!
//! Run from the bench-compare directory:
//! ```bash
//! cargo bench --bench yaml_parsers
//! ```

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use succinctly::yaml::{YamlIndex, YamlValue};

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

/// Test file paths (relative to bench-compare directory)
const TEST_FILES: &[(&str, &str)] = &[
    ("1kb", "../data/bench/generated/yaml/comprehensive/1kb.yaml"),
    ("10kb", "../data/bench/generated/yaml/comprehensive/10kb.yaml"),
    (
        "100kb",
        "../data/bench/generated/yaml/comprehensive/100kb.yaml",
    ),
    ("1mb", "../data/bench/generated/yaml/comprehensive/1mb.yaml"),
    (
        "10mb",
        "../data/bench/generated/yaml/comprehensive/10mb.yaml",
    ),
];

/// Load test file or return None
fn load_test_file(path: &str) -> Option<Vec<u8>> {
    let path = std::path::Path::new(path);
    if !path.exists() {
        eprintln!("Skipping benchmark: {} not found", path.display());
        eprintln!("Generate with: cd .. && cargo run --release --features cli -- yaml generate comprehensive -o data/bench/generated/yaml/comprehensive/");
        return None;
    }
    Some(std::fs::read(path).expect("Failed to read test file"))
}

// ============================================================================
// Traversal functions for each parser
// ============================================================================

/// Count nodes using serde_yaml
fn count_serde_yaml(v: &serde_yaml::Value) -> usize {
    match v {
        serde_yaml::Value::Sequence(arr) => 1 + arr.iter().map(count_serde_yaml).sum::<usize>(),
        serde_yaml::Value::Mapping(obj) => 1 + obj.values().map(count_serde_yaml).sum::<usize>(),
        _ => 1,
    }
}

/// Count nodes using succinctly
fn count_succinctly<W: AsRef<[u64]>>(v: YamlValue<'_, W>) -> usize {
    match v {
        YamlValue::Sequence(elements) => {
            let mut count = 1;
            for elem in elements {
                count += count_succinctly(elem);
            }
            count
        }
        YamlValue::Mapping(fields) => {
            let mut count = 1;
            for field in fields {
                count += count_succinctly(field.key());
                count += count_succinctly(field.value());
            }
            count
        }
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

        let mut group = c.benchmark_group(format!("yaml_parse_only/{}", name));
        group.throughput(Throughput::Bytes(file_size));
        let sample_size = match *name {
            "1kb" | "10kb" | "100kb" => 100,
            "1mb" => 50,
            "10mb" => 20,
            _ => 20,
        };
        group.sample_size(sample_size);

        // serde_yaml
        group.bench_function("serde_yaml", |b| {
            b.iter(|| {
                let v: serde_yaml::Value = serde_yaml::from_slice(black_box(&bytes)).unwrap();
                black_box(v)
            })
        });

        // succinctly (index only)
        group.bench_function("succinctly", |b| {
            b.iter(|| {
                let index = YamlIndex::build(black_box(&bytes)).unwrap();
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

        let mut group = c.benchmark_group(format!("yaml_parse_traverse/{}", name));
        group.throughput(Throughput::Bytes(file_size));
        let sample_size = match *name {
            "1kb" | "10kb" | "100kb" => 100,
            "1mb" => 50,
            "10mb" => 20,
            _ => 20,
        };
        group.sample_size(sample_size);

        // serde_yaml: parse + traverse
        group.bench_function("serde_yaml", |b| {
            b.iter(|| {
                let v: serde_yaml::Value = serde_yaml::from_slice(black_box(&bytes)).unwrap();
                count_serde_yaml(&v)
            })
        });

        // succinctly: parse + traverse
        group.bench_function("succinctly", |b| {
            b.iter(|| {
                let index = YamlIndex::build(black_box(&bytes)).unwrap();
                let root = index.root(&bytes);
                count_succinctly(root.value())
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
        let serde_value: serde_yaml::Value = serde_yaml::from_slice(&bytes).unwrap();
        let succ_index = YamlIndex::build(&bytes).unwrap();

        let mut group = c.benchmark_group(format!("yaml_traverse_only/{}", name));
        group.throughput(Throughput::Bytes(file_size));
        let sample_size = match *name {
            "1kb" | "10kb" | "100kb" => 100,
            "1mb" => 50,
            "10mb" => 20,
            _ => 20,
        };
        group.sample_size(sample_size);

        // serde_yaml traverse
        group.bench_function("serde_yaml", |b| {
            b.iter(|| count_serde_yaml(black_box(&serde_value)))
        });

        // succinctly traverse
        group.bench_function("succinctly", |b| {
            b.iter(|| {
                let root = succ_index.root(black_box(&bytes));
                count_succinctly(root.value())
            })
        });

        group.finish();
    }
}

/// Benchmark YAML to JSON conversion
fn bench_yaml_to_json(c: &mut Criterion) {
    for (name, path) in TEST_FILES {
        let Some(bytes) = load_test_file(path) else {
            continue;
        };
        let file_size = bytes.len() as u64;

        let mut group = c.benchmark_group(format!("yaml_to_json/{}", name));
        group.throughput(Throughput::Bytes(file_size));
        let sample_size = match *name {
            "1kb" | "10kb" | "100kb" => 100,
            "1mb" => 50,
            "10mb" => 20,
            _ => 20,
        };
        group.sample_size(sample_size);

        // serde_yaml -> serde_json (via Value conversion)
        group.bench_function("serde_yaml", |b| {
            b.iter(|| {
                let yaml_val: serde_yaml::Value = serde_yaml::from_slice(black_box(&bytes)).unwrap();
                let json_str = serde_json::to_string(&yaml_val).unwrap();
                black_box(json_str)
            })
        });

        // succinctly direct to_json
        group.bench_function("succinctly", |b| {
            b.iter(|| {
                let index = YamlIndex::build(black_box(&bytes)).unwrap();
                let root = index.root(&bytes);
                let json_str = root.to_json_document();
                black_box(json_str)
            })
        });

        group.finish();
    }
}

/// Measure actual peak memory usage for each parser
fn measure_peak_memory(c: &mut Criterion) {
    let mut group = c.benchmark_group("yaml_peak_memory");
    group.sample_size(10);

    for (name, path) in TEST_FILES {
        let Some(bytes) = load_test_file(path) else {
            continue;
        };

        let yaml_size = bytes.len();
        let yaml_kb = yaml_size as f64 / 1024.0;
        let yaml_mb = yaml_size as f64 / 1024.0 / 1024.0;

        println!("\n{:=^70}", "");
        if yaml_mb >= 1.0 {
            println!(" YAML Peak Memory Usage: {} ({:.2} MB) ", name, yaml_mb);
        } else {
            println!(" YAML Peak Memory Usage: {} ({:.2} KB) ", name, yaml_kb);
        }
        println!("{:=^70}", "");

        // Measure serde_yaml
        ALLOCATOR.reset();
        let _serde_value: serde_yaml::Value = serde_yaml::from_slice(&bytes).unwrap();
        let serde_peak = ALLOCATOR.peak();
        drop(_serde_value);

        // Measure succinctly
        ALLOCATOR.reset();
        let _succ_index = YamlIndex::build(&bytes).unwrap();
        let succ_peak = ALLOCATOR.peak();
        drop(_succ_index);

        // Print results
        println!(
            "\n{:<20} {:>15} {:>12} {:>12}",
            "Parser", "Peak Memory", "Overhead", "vs YAML"
        );
        println!("{:-<20} {:-<15} {:-<12} {:-<12}", "", "", "", "");

        let print_row = |parser: &str, peak: usize| {
            let overhead_pct = peak as f64 / yaml_size as f64 * 100.0;
            let ratio = peak as f64 / yaml_size as f64;
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

        print_row("serde_yaml", serde_peak);
        print_row("succinctly", succ_peak);
        if yaml_mb >= 1.0 {
            println!(
                "{:<20} {:>12.2} MB {:>12} {:>12}",
                "(original YAML)", yaml_mb, "---", "1.00x"
            );
        } else {
            println!(
                "{:<20} {:>12.2} KB {:>12} {:>12}",
                "(original YAML)", yaml_kb, "---", "1.00x"
            );
        }

        // Memory efficiency comparison
        println!("\nMemory efficiency (smaller is better):");
        if succ_peak > 0 && serde_peak > 0 {
            let ratio = serde_peak as f64 / succ_peak as f64;
            if ratio > 1.0 {
                println!(
                    "  succinctly uses {:.1}x LESS memory than serde_yaml",
                    ratio
                );
            } else {
                println!(
                    "  serde_yaml uses {:.1}x LESS memory than succinctly",
                    1.0 / ratio
                );
            }
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
    bench_yaml_to_json,
    measure_peak_memory,
);
criterion_main!(benches);
