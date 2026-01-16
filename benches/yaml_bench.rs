//! Criterion benchmarks for YAML parsing performance.
//!
//! Establishes baseline throughput measurements for the YAML parser
//! before SIMD optimizations.
//!
//! Run with:
//! ```bash
//! cargo bench --bench yaml_bench
//! ```

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use succinctly::yaml::YamlIndex;

// ============================================================================
// YAML Generator Functions (Phase 1: Block-style only)
// ============================================================================

/// Generate simple key-value mappings.
/// Example: `key0: value0\nkey1: value1\n...`
fn generate_simple_kv(pairs: usize) -> Vec<u8> {
    let mut yaml = Vec::with_capacity(pairs * 20);
    for i in 0..pairs {
        yaml.extend_from_slice(format!("key{}: value{}\n", i, i).as_bytes());
    }
    yaml
}

/// Generate nested mapping structure.
/// Creates a tree with specified depth and width at each level.
fn generate_nested(depth: usize, width: usize) -> Vec<u8> {
    let mut yaml = Vec::new();
    generate_nested_recursive(&mut yaml, depth, width, 0);
    yaml
}

fn generate_nested_recursive(yaml: &mut Vec<u8>, depth: usize, width: usize, indent: usize) {
    let indent_str = "  ".repeat(indent);
    if depth == 0 {
        yaml.extend_from_slice(format!("{}value: leaf\n", indent_str).as_bytes());
        return;
    }
    for i in 0..width {
        yaml.extend_from_slice(format!("{}level{}_{}: \n", indent_str, depth, i).as_bytes());
        generate_nested_recursive(yaml, depth - 1, width, indent + 1);
    }
}

/// Generate a sequence with simple items.
/// Example: `- item0\n- item1\n...`
fn generate_sequence(items: usize) -> Vec<u8> {
    let mut yaml = Vec::with_capacity(items * 12);
    for i in 0..items {
        yaml.extend_from_slice(format!("- item{}\n", i).as_bytes());
    }
    yaml
}

/// Generate double-quoted strings.
fn generate_quoted_strings(count: usize) -> Vec<u8> {
    let mut yaml = Vec::with_capacity(count * 40);
    for i in 0..count {
        yaml.extend_from_slice(
            format!("key{}: \"This is a quoted string with value {}\"\n", i, i).as_bytes(),
        );
    }
    yaml
}

/// Generate single-quoted strings.
fn generate_single_quoted_strings(count: usize) -> Vec<u8> {
    let mut yaml = Vec::with_capacity(count * 40);
    for i in 0..count {
        yaml.extend_from_slice(
            format!("key{}: 'This is a single-quoted string {}'\n", i, i).as_bytes(),
        );
    }
    yaml
}

/// Generate a mixed YAML structure approximating target size.
/// Uses simple key-value pairs to ensure valid YAML.
fn generate_mixed_yaml(target_size: usize) -> Vec<u8> {
    let mut yaml = Vec::with_capacity(target_size + 200);

    // Simple repeating structure
    let mut i = 0;
    while yaml.len() < target_size {
        yaml.extend_from_slice(format!("key{}: value{}\n", i, i).as_bytes());
        i += 1;
    }

    yaml
}

// ============================================================================
// Benchmark Groups
// ============================================================================

fn bench_simple_kv(c: &mut Criterion) {
    let mut group = c.benchmark_group("yaml/simple_kv");

    for &count in &[10, 100, 1000, 10000] {
        let yaml = generate_simple_kv(count);
        group.throughput(Throughput::Bytes(yaml.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(count), &yaml, |b, yaml| {
            b.iter(|| {
                let index = YamlIndex::build(black_box(yaml)).unwrap();
                black_box(index)
            })
        });
    }

    group.finish();
}

fn bench_nested(c: &mut Criterion) {
    let mut group = c.benchmark_group("yaml/nested");

    // Various depth/width combinations
    for &(depth, width) in &[(3, 3), (5, 2), (10, 2), (3, 5)] {
        let yaml = generate_nested(depth, width);
        let label = format!("d{}_w{}", depth, width);
        group.throughput(Throughput::Bytes(yaml.len() as u64));
        group.bench_with_input(BenchmarkId::new("depth_width", &label), &yaml, |b, yaml| {
            b.iter(|| {
                let index = YamlIndex::build(black_box(yaml)).unwrap();
                black_box(index)
            })
        });
    }

    group.finish();
}

fn bench_sequences(c: &mut Criterion) {
    let mut group = c.benchmark_group("yaml/sequences");

    for &count in &[10, 100, 1000, 10000] {
        let yaml = generate_sequence(count);
        group.throughput(Throughput::Bytes(yaml.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(count), &yaml, |b, yaml| {
            b.iter(|| {
                let index = YamlIndex::build(black_box(yaml)).unwrap();
                black_box(index)
            })
        });
    }

    group.finish();
}

fn bench_quoted_strings(c: &mut Criterion) {
    let mut group = c.benchmark_group("yaml/quoted");

    for &count in &[10, 100, 1000] {
        let double_quoted = generate_quoted_strings(count);
        group.throughput(Throughput::Bytes(double_quoted.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("double", count),
            &double_quoted,
            |b, yaml| {
                b.iter(|| {
                    let index = YamlIndex::build(black_box(yaml)).unwrap();
                    black_box(index)
                })
            },
        );

        let single_quoted = generate_single_quoted_strings(count);
        group.throughput(Throughput::Bytes(single_quoted.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("single", count),
            &single_quoted,
            |b, yaml| {
                b.iter(|| {
                    let index = YamlIndex::build(black_box(yaml)).unwrap();
                    black_box(index)
                })
            },
        );
    }

    group.finish();
}

fn bench_large_files(c: &mut Criterion) {
    let mut group = c.benchmark_group("yaml/large");

    // Test various sizes: 1KB, 10KB, 100KB, 1MB
    for &(name, size) in &[
        ("1kb", 1_000),
        ("10kb", 10_000),
        ("100kb", 100_000),
        ("1mb", 1_000_000),
    ] {
        let yaml = generate_mixed_yaml(size);
        group.throughput(Throughput::Bytes(yaml.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(name), &yaml, |b, yaml| {
            b.iter(|| {
                let index = YamlIndex::build(black_box(yaml)).unwrap();
                black_box(index)
            })
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_simple_kv,
    bench_nested,
    bench_sequences,
    bench_quoted_strings,
    bench_large_files,
);
criterion_main!(benches);
