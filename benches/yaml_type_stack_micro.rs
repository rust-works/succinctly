//! Micro-benchmark for type stack operations.
//!
//! This benchmark isolates the performance difference between:
//! 1. Original: type_stack.last() + Option comparison
//! 2. Optimized: cached current_type direct comparison
//!
//! Run with:
//! ```bash
//! cargo bench --bench yaml_type_stack_micro
//! ```

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use succinctly::yaml::YamlIndex;

/// Generate YAML with heavy type checking (nested mappings and sequences).
/// This stresses the type_stack operations since each container needs type checks.
fn generate_deeply_nested_yaml(depth: usize) -> Vec<u8> {
    let mut yaml = Vec::new();

    // Create alternating mapping/sequence nesting
    for d in 0..depth {
        let indent = "  ".repeat(d);
        if d % 2 == 0 {
            // Mapping
            yaml.extend_from_slice(format!("{}key{}:\n", indent, d).as_bytes());
        } else {
            // Sequence
            yaml.extend_from_slice(format!("{}- item{}:\n", indent, d).as_bytes());
        }
    }

    // Add leaf value
    let indent = "  ".repeat(depth);
    yaml.extend_from_slice(format!("{}value: leaf\n", indent).as_bytes());

    yaml
}

/// Generate YAML with many sibling containers at same level.
/// This creates many type_stack.last() checks when deciding to open new containers.
fn generate_wide_structure(width: usize) -> Vec<u8> {
    let mut yaml = Vec::new();

    // Create many mapping entries at root level
    for i in 0..width / 2 {
        yaml.extend_from_slice(format!("key{}:\n", i).as_bytes());
        yaml.extend_from_slice(format!("  - item{}\n", i).as_bytes());
    }

    yaml
}

/// Generate mixed structure with many container transitions.
/// Each transition requires type checks.
fn generate_mixed_containers(pairs: usize) -> Vec<u8> {
    let mut yaml = Vec::new();

    for i in 0..pairs {
        if i % 3 == 0 {
            // Mapping with sequence value
            yaml.extend_from_slice(format!("key{}:\n", i).as_bytes());
            yaml.extend_from_slice(b"  - a\n  - b\n");
        } else if i % 3 == 1 {
            // Sequence item with nested mapping
            yaml.extend_from_slice(b"- nested:\n    val: x\n");
        } else {
            // Simple mapping
            yaml.extend_from_slice(format!("key{}: value\n", i).as_bytes());
        }
    }

    yaml
}

fn bench_deeply_nested(c: &mut Criterion) {
    let mut group = c.benchmark_group("yaml_type_stack/deeply_nested");

    for &depth in &[10, 20, 50, 100] {
        let yaml = generate_deeply_nested_yaml(depth);
        group.throughput(Throughput::Elements(depth as u64));
        group.bench_with_input(BenchmarkId::from_parameter(depth), &yaml, |b, yaml| {
            b.iter(|| {
                let index = YamlIndex::build(black_box(yaml)).unwrap();
                black_box(index)
            })
        });
    }

    group.finish();
}

fn bench_wide_structure(c: &mut Criterion) {
    let mut group = c.benchmark_group("yaml_type_stack/wide");

    for &width in &[100, 500, 1000, 5000] {
        let yaml = generate_wide_structure(width);
        group.throughput(Throughput::Elements(width as u64));
        group.bench_with_input(BenchmarkId::from_parameter(width), &yaml, |b, yaml| {
            b.iter(|| {
                let index = YamlIndex::build(black_box(yaml)).unwrap();
                black_box(index)
            })
        });
    }

    group.finish();
}

fn bench_mixed_containers(c: &mut Criterion) {
    let mut group = c.benchmark_group("yaml_type_stack/mixed");

    for &pairs in &[100, 500, 1000, 5000] {
        let yaml = generate_mixed_containers(pairs);
        group.throughput(Throughput::Elements(pairs as u64));
        group.bench_with_input(BenchmarkId::from_parameter(pairs), &yaml, |b, yaml| {
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
    bench_deeply_nested,
    bench_wide_structure,
    bench_mixed_containers,
);
criterion_main!(benches);
