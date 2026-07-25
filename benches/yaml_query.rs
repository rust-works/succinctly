//! YAML **query**-path benchmark (#40).
//!
//! `yaml_bench` measures index *building*; every one of its cases calls
//! `YamlIndex::build` and stops. Nothing in the suite measured the traversal
//! that follows, so a change to the query path — such as the `select` word scan
//! this benchmark exists to check — had no end-to-end coverage at all.
//!
//! Each case builds the index once, outside the timed region, then walks every
//! node in document order calling `text_position()`. That call is the sole
//! entry point to `AdvancePositions::get`, which owns the hottest `select` scan
//! in the crate, so this measures the affected path with none of the process
//! startup and 10 MB of output I/O that swamp it when timing the `yq` CLI.
//!
//! Scalar length is the axis that matters. Interest bits mark structural
//! positions, so the byte gap between consecutive scalars *is* the scan length:
//! short values keep scans to a word or two, long ones push them into the tail
//! where a block kernel pays off. The cases below span that range deliberately.
//!
//! Run with:
//! ```bash
//! cargo bench --bench yaml_query
//! ```

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;
use succinctly::yaml::{YamlCursor, YamlIndex, YamlValue};

/// A mapping of `count` keys whose values are scalars of `value_len` bytes.
fn mapping_with_scalars(count: usize, value_len: usize) -> Vec<u8> {
    let mut out = String::new();
    for i in 0..count {
        let value = "x".repeat(value_len);
        out.push_str(&format!("key_{i}: {value}\n"));
    }
    out.into_bytes()
}

/// A sequence of `count` small mappings — the Kubernetes/CI manifest shape,
/// where structure is dense and scalars are short.
fn sequence_of_records(count: usize) -> Vec<u8> {
    let mut out = String::new();
    out.push_str("items:\n");
    for i in 0..count {
        out.push_str(&format!(
            "  - name: item_{i}\n    id: {i}\n    enabled: true\n"
        ));
    }
    out.into_bytes()
}

/// Nested mappings `depth` levels deep, `width` keys per level.
fn nested(depth: usize, width: usize) -> Vec<u8> {
    fn go(out: &mut String, depth: usize, width: usize, indent: usize) {
        if depth == 0 {
            return;
        }
        for i in 0..width {
            let pad = "  ".repeat(indent);
            if depth == 1 {
                out.push_str(&format!("{pad}leaf_{i}: value_{i}\n"));
            } else {
                out.push_str(&format!("{pad}node_{i}:\n"));
                go(out, depth - 1, width, indent + 1);
            }
        }
    }
    let mut out = String::new();
    go(&mut out, depth, width, 0);
    out.into_bytes()
}

/// Walk every node in document order, materialising each text position.
///
/// Returns the number of nodes visited so the optimiser cannot discard the
/// traversal, and so a mismatch between runs would be visible.
fn walk(cur: YamlCursor<'_, Vec<u64>>) -> usize {
    let mut visited = 1;
    black_box(cur.text_position());

    match cur.value() {
        YamlValue::Mapping(fields) => {
            for field in fields {
                visited += walk(field.value_cursor());
            }
        }
        YamlValue::Sequence(mut elements) => {
            while let Some((child, rest)) = elements.uncons_cursor() {
                visited += walk(child);
                elements = rest;
            }
        }
        // Aliases are not followed: the target is visited at its definition.
        YamlValue::Alias { .. } | YamlValue::String(_) | YamlValue::Null | YamlValue::Error(_) => {}
    }
    visited
}

/// Walk from the document root, which YAML models as a sequence of documents.
fn walk_root(index: &YamlIndex<Vec<u64>>, text: &[u8]) -> usize {
    let root = index.root(text);
    match root.value() {
        YamlValue::Sequence(mut docs) => {
            let mut visited = 0;
            while let Some((doc, rest)) = docs.uncons_cursor() {
                visited += walk(doc);
                docs = rest;
            }
            visited
        }
        _ => walk(root),
    }
}

fn bench_case(c: &mut Criterion, group_name: &str, cases: &[(String, Vec<u8>)]) {
    let mut group = c.benchmark_group(group_name);
    for (label, yaml) in cases {
        let index = YamlIndex::build(yaml).expect("benchmark fixture must parse");
        // Guard the premise: a fixture that produced no nodes would report a
        // meaningless "speedup".
        assert!(walk_root(&index, yaml) > 1, "{label} traversed nothing");

        group.throughput(Throughput::Bytes(yaml.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(label), yaml, |b, yaml| {
            b.iter(|| black_box(walk_root(&index, black_box(yaml))));
        });
    }
    group.finish();
}

/// Scalar length sweep — the axis that controls scan length.
fn bench_scalar_length(c: &mut Criterion) {
    let cases: Vec<(String, Vec<u8>)> = [8usize, 32, 128, 512, 2048]
        .iter()
        .map(|&len| (format!("value_{len}b"), mapping_with_scalars(2000, len)))
        .collect();
    bench_case(c, "yaml_query/scalar_length", &cases);
}

/// Realistic document shapes.
fn bench_shapes(c: &mut Criterion) {
    let cases = vec![
        ("records_1k".to_string(), sequence_of_records(1000)),
        ("records_10k".to_string(), sequence_of_records(10_000)),
        ("nested_d6_w4".to_string(), nested(6, 4)),
        ("nested_d4_w8".to_string(), nested(4, 8)),
    ];
    bench_case(c, "yaml_query/shapes", &cases);
}

criterion_group!(benches, bench_scalar_length, bench_shapes);
criterion_main!(benches);
