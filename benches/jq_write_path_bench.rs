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
//! summary table, without needing to eyeball the raw per-size timings. Each
//! group's premise is checked once per size, outside the timed loop -- the
//! same "guard the premise" idiom `jq_generic_index_bench` uses -- so a
//! future bug that silently turns a write into a no-op (which would
//! otherwise show up as a misleading *speedup*, per this repo's own
//! benchmarking discipline: "gate on output identity before believing any
//! timing") fails loudly here instead.
//!
//! `parse`/`JsonIndex::build` run once per size, outside `b.iter`, matching
//! `jq_generic_index_bench`'s convention -- only `index.root` + `eval` are
//! timed, so the reported cost isolates the write-path evaluator itself
//! rather than being diluted by a constant per-sample index-build cost.
//!
//! Run with:
//! ```bash
//! cargo bench --bench jq_write_path_bench
//! ```

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;
use succinctly::jq::{eval, parse, Expr, JqSemantics, OwnedValue, QueryResult};
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

/// Evaluate `expr` against `json`, asserting it produced exactly one
/// non-error output (every shape benchmarked here is a single-document
/// write or a `[path(...)]`-collected array, never a fan-out), and return
/// that output for the caller's own content check.
fn eval_one(expr: &Expr, json: &[u8]) -> OwnedValue {
    let index = JsonIndex::build(json);
    let cursor = index.root(json);
    let result = eval::<Vec<u64>, JqSemantics>(expr, cursor);
    match result {
        QueryResult::Owned(v) => v,
        other => panic!("expected exactly one non-error output, got {other:?}"),
    }
}

fn bench_del_array(c: &mut Criterion) {
    let mut group = c.benchmark_group("jq_write_path_del_array");
    let expr = parse("del(.foo[])").expect("must parse");
    for &n in SIZES {
        let json = array_doc(n);

        // Guard the premise: `del(.foo[])` must actually empty the array,
        // not silently no-op (which would otherwise look like a speedup).
        let OwnedValue::Object(doc) = eval_one(&expr, &json) else {
            panic!("n={n}: del(.foo[]) must produce an object");
        };
        assert_eq!(
            doc.get("foo"),
            Some(&OwnedValue::Array(Vec::new())),
            "n={n}: del(.foo[]) must empty the array"
        );

        let index = JsonIndex::build(&json);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &json, |b, json| {
            b.iter(|| {
                let cursor = index.root(black_box(json));
                black_box(eval::<Vec<u64>, JqSemantics>(&expr, cursor))
            });
        });
    }
    group.finish();
}

fn bench_del_object(c: &mut Criterion) {
    let mut group = c.benchmark_group("jq_write_path_del_object");
    let expr = parse("del(.foo[])").expect("must parse");
    for &n in SIZES {
        let json = object_doc(n);

        let OwnedValue::Object(doc) = eval_one(&expr, &json) else {
            panic!("n={n}: del(.foo[]) must produce an object");
        };
        let OwnedValue::Object(emptied) = doc.get("foo").expect("foo field") else {
            panic!("n={n}: .foo must still be an object");
        };
        assert!(
            emptied.is_empty(),
            "n={n}: del(.foo[]) must empty the object"
        );

        let index = JsonIndex::build(&json);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &json, |b, json| {
            b.iter(|| {
                let cursor = index.root(black_box(json));
                black_box(eval::<Vec<u64>, JqSemantics>(&expr, cursor))
            });
        });
    }
    group.finish();
}

fn bench_assign_array(c: &mut Criterion) {
    let mut group = c.benchmark_group("jq_write_path_assign_array");
    let expr = parse(".foo[] = 0").expect("must parse");
    for &n in SIZES {
        let json = array_doc(n);

        // Guard the premise: every element must become 0, not just the
        // first (a common off-by-one for a trailing-iterate write path).
        let OwnedValue::Object(doc) = eval_one(&expr, &json) else {
            panic!("n={n}: .foo[] = 0 must produce an object");
        };
        let OwnedValue::Array(elements) = doc.get("foo").expect("foo field") else {
            panic!("n={n}: .foo must still be an array");
        };
        assert_eq!(
            elements.len(),
            n,
            "n={n}: .foo[] = 0 must keep every element"
        );
        assert!(
            elements.iter().all(|v| *v == OwnedValue::Int(0)),
            "n={n}: .foo[] = 0 must set every element to 0"
        );

        let index = JsonIndex::build(&json);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &json, |b, json| {
            b.iter(|| {
                let cursor = index.root(black_box(json));
                black_box(eval::<Vec<u64>, JqSemantics>(&expr, cursor))
            });
        });
    }
    group.finish();
}

fn bench_update_array(c: &mut Criterion) {
    let mut group = c.benchmark_group("jq_write_path_update_array");
    let expr = parse(".foo[] |= . + 1").expect("must parse");
    for &n in SIZES {
        let json = array_doc(n);

        // Guard the premise: every element must be incremented by exactly
        // one, preserving order -- not just non-erroring.
        let OwnedValue::Object(doc) = eval_one(&expr, &json) else {
            panic!("n={n}: .foo[] |= . + 1 must produce an object");
        };
        let OwnedValue::Array(updated) = doc.get("foo").expect("foo field") else {
            panic!("n={n}: .foo must still be an array");
        };
        assert_eq!(
            updated.len(),
            n,
            "n={n}: .foo[] |= . + 1 must keep every element"
        );
        for (i, v) in updated.iter().enumerate() {
            assert_eq!(
                *v,
                OwnedValue::Int(i as i64 + 1),
                "n={n}: element {i} must be incremented by one"
            );
        }

        let index = JsonIndex::build(&json);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &json, |b, json| {
            b.iter(|| {
                let cursor = index.root(black_box(json));
                black_box(eval::<Vec<u64>, JqSemantics>(&expr, cursor))
            });
        });
    }
    group.finish();
}

fn bench_path_trailing_iterate(c: &mut Criterion) {
    let mut group = c.benchmark_group("jq_write_path_path_array");
    let expr = parse("[path(.foo[])]").expect("must parse");
    for &n in SIZES {
        let json = array_doc(n);

        // Guard the premise: exactly one path per element, in order.
        let OwnedValue::Array(paths) = eval_one(&expr, &json) else {
            panic!("n={n}: [path(.foo[])] must produce an array");
        };
        assert_eq!(paths.len(), n, "n={n}: must report one path per element");
        for (i, p) in paths.iter().enumerate() {
            let expected = OwnedValue::Array(vec![
                OwnedValue::String("foo".into()),
                OwnedValue::Int(i as i64),
            ]);
            assert_eq!(*p, expected, "n={n}: path {i} mismatch");
        }

        let index = JsonIndex::build(&json);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &json, |b, json| {
            b.iter(|| {
                let cursor = index.root(black_box(json));
                black_box(eval::<Vec<u64>, JqSemantics>(&expr, cursor))
            });
        });
    }
    group.finish();
}

fn bench_computed_key_with_trailing_iterate(c: &mut Criterion) {
    let mut group = c.benchmark_group("jq_write_path_computed_key_trailing_iterate");
    let expr = parse("[path(.items[(0,1)].foo[])]").expect("must parse");
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

        // Guard the premise: `(0,1)` fans out over both `.items` entries,
        // each contributing its own n-element `.foo[]` -- 2n paths total.
        let OwnedValue::Array(paths) = eval_one(&expr, &json) else {
            panic!("n={n}: query must produce an array");
        };
        assert_eq!(
            paths.len(),
            2 * n,
            "n={n}: must report 2n paths (both branches)"
        );

        let index = JsonIndex::build(&json);
        // `(0,1)` touches both n-element `.foo` arrays, so the real work
        // done is 2n elements, not n -- matches `paths.len()`'s guard above.
        group.throughput(Throughput::Elements(2 * n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &json, |b, json| {
            b.iter(|| {
                let cursor = index.root(black_box(json));
                black_box(eval::<Vec<u64>, JqSemantics>(&expr, cursor))
            });
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
