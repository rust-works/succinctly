//! Scaling coverage for jq's write-path builtins (#829).
//!
//! Every existing jq benchmark in this crate (`jq_comparison`, the query
//! shapes in `src/bin/succinctly/jq_bench.rs`) covers read-only queries only
//! (`.`, `keys_unsorted`, `map`, `select`, `length`, `.[]`, `first`/`last`).
//! None of them exercise `=`, `|=`, `del(...)`, or `path(...)` -- the
//! write-path builtins that go through `eval_assign`/`eval_update`/
//! `builtin_del` and, for a computed/dynamic path, `resolve_dynamic_indexes`/
//! `resolve_node`.
//!
//! That gap let a real regression through review undetected: an earlier
//! version of #682's `recurse(f)` path-tracking fix accidentally broadened
//! `needs_path_prepass` globally, turning `del(.foo[])`/`path(.foo[])` on a
//! large array into O(n^2)-or-worse (~1500x slower at 50,000 elements,
//! timing out past 100,000) -- caught only by manual timing during code
//! review, since nothing in CI or the benchmark suite touched this shape.
//!
//! This file makes **no timing assertion** -- like `jq_generic_index_bench`,
//! it is a Criterion benchmark for manual before/after comparison (interleaved
//! per `docs/guides/benchmarking.md#ab-benchmarking-method`), not a CI gate.
//! `Throughput::Elements` is set per group so `cargo bench`'s reported
//! ns/element makes an O(n) vs O(n^2) regression visible directly in the
//! summary table, without needing to eyeball the raw per-size timings.
//!
//! Run with:
//! ```bash
//! cargo bench --bench jq_write_path_bench
//! ```

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;
use succinctly::jq::{eval, parse, JqSemantics};
use succinctly::json::JsonIndex;

const SIZES: &[usize] = &[1_000, 10_000, 100_000];

/// `{"foo": [0, 1, ..., n-1]}`.
fn array_doc(n: usize) -> Vec<u8> {
    let mut out = String::from(r#"{"foo":["#);
    for i in 0..n {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&i.to_string());
    }
    out.push_str("]}");
    out.into_bytes()
}

/// `{"foo": {"k0": 0, "k1": 1, ..., "k<n-1>": n-1}}` -- same element count
/// as [`array_doc`], but keyed, so `del(.foo[])`'s object arm (a distinct
/// code path from its array arm) gets its own scaling coverage.
fn object_doc(n: usize) -> Vec<u8> {
    let mut out = String::from(r#"{"foo":{"#);
    for i in 0..n {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(r#""k{i}":{i}"#));
    }
    out.push_str("}}");
    out.into_bytes()
}

/// `{"items": [{"foo": [0, ..., n-1]}, {"foo": [0, ..., n-1]}]}` -- for the
/// computed-key-alongside-trailing-iterate shape (`.items[(0,1)].foo[]`),
/// the one combination the issue calls out as still routed through the
/// multi-branch resolver even after #682's narrowed fix.
fn computed_key_doc(n: usize) -> Vec<u8> {
    let one = array_doc(n);
    let one = core::str::from_utf8(&one).unwrap();
    format!(r#"{{"items":[{one},{one}]}}"#).into_bytes()
}

fn run(expr_src: &str, json: &[u8]) -> usize {
    let expr = parse(expr_src).unwrap_or_else(|e| panic!("{expr_src:?} must parse: {e}"));
    let index = JsonIndex::build(json);
    let cursor = index.root(json);
    let result = eval::<Vec<u64>, JqSemantics>(&expr, cursor);
    assert!(!result.is_error(), "{expr_src:?} must not error");
    result.collect_owned().len()
}

fn bench_del_array(c: &mut Criterion) {
    let mut group = c.benchmark_group("jq_write_path_del_array");
    for &n in SIZES {
        let json = array_doc(n);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &json, |b, json| {
            b.iter(|| black_box(run("del(.foo[])", black_box(json))));
        });
    }
    group.finish();
}

fn bench_del_object(c: &mut Criterion) {
    let mut group = c.benchmark_group("jq_write_path_del_object");
    for &n in SIZES {
        let json = object_doc(n);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &json, |b, json| {
            b.iter(|| black_box(run("del(.foo[])", black_box(json))));
        });
    }
    group.finish();
}

fn bench_assign_array(c: &mut Criterion) {
    let mut group = c.benchmark_group("jq_write_path_assign_array");
    for &n in SIZES {
        let json = array_doc(n);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &json, |b, json| {
            b.iter(|| black_box(run(".foo[] = 0", black_box(json))));
        });
    }
    group.finish();
}

fn bench_update_array(c: &mut Criterion) {
    let mut group = c.benchmark_group("jq_write_path_update_array");
    for &n in SIZES {
        let json = array_doc(n);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &json, |b, json| {
            b.iter(|| black_box(run(".foo[] |= . + 1", black_box(json))));
        });
    }
    group.finish();
}

fn bench_path_trailing_iterate(c: &mut Criterion) {
    let mut group = c.benchmark_group("jq_write_path_path_array");
    for &n in SIZES {
        let json = array_doc(n);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &json, |b, json| {
            b.iter(|| black_box(run("[path(.foo[])]", black_box(json))));
        });
    }
    group.finish();
}

fn bench_computed_key_with_trailing_iterate(c: &mut Criterion) {
    let mut group = c.benchmark_group("jq_write_path_computed_key_trailing_iterate");
    // Much smaller sizes than the other groups: this shape is NOT the
    // trailing-bare-iterate case #682 already fixed -- a computed key
    // (`(0,1)`) ahead of the trailing `.foo[]` still routes through the
    // multi-branch resolver's O(n^2) path (confirmed while writing this
    // benchmark: doubling `n` roughly quadruples wall time, e.g. 2000 ->
    // 4000 elements total went 0.27s -> 1.08s; filed as #888). The sizes
    // here are deliberately small enough to finish in reasonable time
    // under that *existing* bug, not chosen for statistical smoothness --
    // once #888 lands, raise them to match the other groups.
    for &n in &[200usize, 1_000, 2_000] {
        let json = computed_key_doc(n);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &json, |b, json| {
            b.iter(|| black_box(run("[path(.items[(0,1)].foo[])]", black_box(json))));
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_del_array,
    bench_del_object,
    bench_assign_array,
    bench_update_array,
    bench_path_trailing_iterate,
    bench_computed_key_with_trailing_iterate,
);
criterion_main!(benches);
