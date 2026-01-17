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

/// Generate long double-quoted strings (stress test for SIMD string scanning).
fn generate_long_quoted_strings(count: usize, string_len: usize) -> Vec<u8> {
    let mut yaml = Vec::with_capacity(count * (string_len + 20));
    let long_string: String = "x".repeat(string_len);
    for i in 0..count {
        yaml.extend_from_slice(format!("key{}: \"{}\"\n", i, long_string).as_bytes());
    }
    yaml
}

/// Generate long single-quoted strings (stress test for SIMD string scanning).
fn generate_long_single_quoted_strings(count: usize, string_len: usize) -> Vec<u8> {
    let mut yaml = Vec::with_capacity(count * (string_len + 20));
    let long_string: String = "y".repeat(string_len);
    for i in 0..count {
        yaml.extend_from_slice(format!("key{}: '{}'\n", i, long_string).as_bytes());
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

/// Generate block scalars (literal | style) for SIMD optimization testing.
fn generate_block_scalars(count: usize, lines_per_block: usize) -> Vec<u8> {
    let mut yaml = Vec::with_capacity(count * lines_per_block * 50);
    for i in 0..count {
        yaml.extend_from_slice(format!("block{}: |\n", i).as_bytes());
        for j in 0..lines_per_block {
            yaml.extend_from_slice(
                format!("  This is line {} of block scalar {}\n", j, i).as_bytes(),
            );
        }
    }
    yaml
}

/// Generate long block scalars (stress test for SIMD block scanning).
fn generate_long_block_scalars(count: usize, lines_per_block: usize) -> Vec<u8> {
    let mut yaml = Vec::with_capacity(count * lines_per_block * 100);
    let long_line = "x".repeat(80);
    for i in 0..count {
        yaml.extend_from_slice(format!("content{}: |\n", i).as_bytes());
        for _ in 0..lines_per_block {
            yaml.extend_from_slice(format!("  {}\n", long_line).as_bytes());
        }
    }
    yaml
}

/// Generate anchor-heavy YAML (for P4 SIMD anchor name parsing optimization).
/// Simulates Kubernetes manifests with many anchors and aliases.
fn generate_anchor_heavy(pairs: usize) -> Vec<u8> {
    let mut yaml = Vec::with_capacity(pairs * 100);

    // Define anchors with increasingly long names (simulating real K8s patterns)
    for i in 0..pairs {
        if i % 3 == 0 {
            // Long anchor names (32+ chars) - where SIMD wins big
            yaml.extend_from_slice(
                format!(
                    "common_configuration_setting_{}: &config_anchor_very_long_name_{} value{}\n",
                    i, i, i
                )
                .as_bytes(),
            );
        } else if i % 3 == 1 {
            // Medium anchor names (16-24 chars)
            yaml.extend_from_slice(
                format!("setting_{}: &anchor_medium_{} val{}\n", i, i, i).as_bytes(),
            );
        } else {
            // Reference previous anchors with aliases
            let ref_idx = i.saturating_sub(3);
            if ref_idx % 3 == 0 {
                yaml.extend_from_slice(
                    format!("ref_{}: *config_anchor_very_long_name_{}\n", i, ref_idx).as_bytes(),
                );
            } else {
                yaml.extend_from_slice(
                    format!("ref_{}: *anchor_medium_{}\n", i, ref_idx).as_bytes(),
                );
            }
        }
    }
    yaml
}

/// Generate Kubernetes-like YAML with realistic anchor patterns.
fn generate_k8s_like(resources: usize) -> Vec<u8> {
    let mut yaml = Vec::with_capacity(resources * 500);

    // Common configuration anchors
    yaml.extend_from_slice(b"common: &default_resource_limits\n");
    yaml.extend_from_slice(b"  cpu: \"1000m\"\n");
    yaml.extend_from_slice(b"  memory: \"512Mi\"\n");
    yaml.extend_from_slice(b"labels: &common_labels_for_deployment\n");
    yaml.extend_from_slice(b"  app: myapp\n");
    yaml.extend_from_slice(b"  environment: production\n");
    yaml.extend_from_slice(b"---\n");

    // Generate resources referencing anchors
    for i in 0..resources {
        yaml.extend_from_slice(format!("resource_{}:\n", i).as_bytes());
        yaml.extend_from_slice(b"  metadata:\n");
        yaml.extend_from_slice(b"    labels: *common_labels_for_deployment\n");
        yaml.extend_from_slice(b"  spec:\n");
        yaml.extend_from_slice(b"    resources:\n");
        yaml.extend_from_slice(b"      limits: *default_resource_limits\n");
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

/// Benchmark long strings to stress SIMD string scanning.
fn bench_long_strings(c: &mut Criterion) {
    let mut group = c.benchmark_group("yaml/long_strings");

    // Test with various string lengths: 64, 256, 1024, 4096 bytes
    for &string_len in &[64, 256, 1024, 4096] {
        let count = 100; // 100 entries each

        // Double-quoted long strings
        let double_yaml = generate_long_quoted_strings(count, string_len);
        group.throughput(Throughput::Bytes(double_yaml.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("double", format!("{}b", string_len)),
            &double_yaml,
            |b, yaml| {
                b.iter(|| {
                    let index = YamlIndex::build(black_box(yaml)).unwrap();
                    black_box(index)
                })
            },
        );

        // Single-quoted long strings
        let single_yaml = generate_long_single_quoted_strings(count, string_len);
        group.throughput(Throughput::Bytes(single_yaml.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("single", format!("{}b", string_len)),
            &single_yaml,
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

/// Benchmark block scalars to measure SIMD block scanning optimization.
fn bench_block_scalars(c: &mut Criterion) {
    let mut group = c.benchmark_group("yaml/block_scalars");

    // Test with various block sizes
    for &(blocks, lines) in &[(10, 10), (50, 50), (100, 100), (10, 1000)] {
        let label = format!("{}x{}lines", blocks, lines);
        let yaml = generate_block_scalars(blocks, lines);
        group.throughput(Throughput::Bytes(yaml.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(&label), &yaml, |b, yaml| {
            b.iter(|| {
                let index = YamlIndex::build(black_box(yaml)).unwrap();
                black_box(index)
            })
        });
    }

    // Test long block scalars
    for &(blocks, lines) in &[(10, 100), (50, 100), (100, 100)] {
        let label = format!("long_{}x{}lines", blocks, lines);
        let yaml = generate_long_block_scalars(blocks, lines);
        group.throughput(Throughput::Bytes(yaml.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(&label), &yaml, |b, yaml| {
            b.iter(|| {
                let index = YamlIndex::build(black_box(yaml)).unwrap();
                black_box(index)
            })
        });
    }

    group.finish();
}

fn bench_anchors(c: &mut Criterion) {
    let mut group = c.benchmark_group("yaml/anchors");

    // Test anchor-heavy YAML with various counts
    for &count in &[10, 100, 1000, 5000] {
        let yaml = generate_anchor_heavy(count);
        group.throughput(Throughput::Bytes(yaml.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(count), &yaml, |b, yaml| {
            b.iter(|| {
                let index = YamlIndex::build(black_box(yaml)).unwrap();
                black_box(index)
            })
        });
    }

    // Test Kubernetes-like patterns
    for &resources in &[10, 50, 100] {
        let label = format!("k8s_{}", resources);
        let yaml = generate_k8s_like(resources);
        group.throughput(Throughput::Bytes(yaml.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(&label), &yaml, |b, yaml| {
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
    bench_long_strings,
    bench_large_files,
    bench_block_scalars,
    bench_anchors,
);
criterion_main!(benches);
