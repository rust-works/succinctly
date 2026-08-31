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

/// [`computed_key_doc`]'s object-valued twin: `{"items": [{"foo": {"k0": 0,
/// ...}}, {...}]}`. `del(.items[(0,1)].foo[])` over this routes the same
/// per-element siblings into the delete walk's object arm rather than its
/// array arm -- which until #1690 merged them into one `DeleteTrie` were
/// two separate copies of the same logic, quadratic in the same way
/// (#1301). `bench_del_object` does
/// not cover it -- without a computed key, `del(.foo[])` takes
/// `builtin_del`'s single-path route and never reaches the grouping at all.
fn computed_key_object_doc(n: usize) -> Vec<u8> {
    let one = object_doc(n);
    let one = core::str::from_utf8(&one).unwrap();
    format!(r#"{{"items":[{one},{one}]}}"#).into_bytes()
}

/// `{"foo": [{"a": 0, "b": 0, "c": 0}, ...]}` -- for the comma-through-iterate
/// shape (`del(.foo[].a, .foo[].b)`), which needs no computed key at all to
/// reach the multi-path delete walker: a top-level comma already routes there
/// (#475), and the `[]` ahead of it still enumerates one sibling per element.
fn comma_through_iterate_doc(n: usize) -> Vec<u8> {
    let mut out = String::from(r#"{"foo":["#);
    for i in 0..n {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(r#"{{"a":{i},"b":{i},"c":{i}}}"#));
    }
    out.push_str("]}");
    out.into_bytes()
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
    // This shape is NOT the trailing-bare-iterate case #682 already fixed --
    // a computed key (`(0,1)`) ahead of the trailing `.foo[]` used to route
    // through the multi-branch resolver's O(n^2) path (doubling `n` roughly
    // quadrupled wall time, e.g. 2000 -> 4000 elements total went 0.27s ->
    // 1.08s; filed as #888, fixed by stripping a bare trailing iterate out
    // of `resolve_dynamic_indexes`'s fan-out before it reaches
    // `resolve_seq`, and re-attaching it as a single literal component per
    // branch afterward). Same `SIZES` as the other groups now that this is
    // linear.
    //
    // `path()` only. The same deferral is unsound for `del`/`=`/`|=` (see
    // `resolve_dynamic_indexes`'s doc comment), so the `del` group below
    // still carries the quadratic -- which is why it needs its own capped
    // sizes rather than sharing these.
    for &n in SIZES {
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

/// The `del` half of the shape above, which #888 did **not** make linear and
/// #1301 since has.
///
/// #888's own table reported `del`/`=`/`|=` as flat here, but it stopped at
/// 16,000 elements; `del`'s quadratic term did not dominate until roughly
/// 50,000. The cause was **not** the trailing-iterate deferral `path()` got
/// -- widening the computed key from 2 branches to 512 while holding the
/// resolved-path count at 80,000 took `del` from 890ms to 95ms, which a
/// deferral-shaped cost cannot do -- but `delete_expr_array_paths`'
/// own sibling grouping, which scanned an insertion-ordered `Vec` linearly
/// per sibling. Interleaved A/B, Apple M-series, release, total elements = 2n:
///
/// | 2n      | before  | after  | path  |
/// |---------|---------|--------|-------|
/// | 20,000  |  0.07s  | 0.02s  | 0.03s |
/// | 100,000 |  1.34s  | 0.09s  | 0.10s |
/// | 200,000 |  5.92s  | 0.20s  | 0.20s |
/// | 400,000 | 22.03s  | 0.38s  | 0.72s |
///
/// So this shares the other groups' `SIZES` now, where it used to be capped
/// at 1,000/5,000/10,000 to finish in reasonable time under the bug.
fn bench_del_computed_key_with_trailing_iterate(c: &mut Criterion) {
    let mut group = c.benchmark_group("jq_write_path_del_computed_key_trailing_iterate");
    let expr = parse("del(.items[(0,1)].foo[])").expect("must parse");
    for &n in SIZES {
        let json = computed_key_doc(n);

        // Guard the premise: both `.items` entries must come back emptied,
        // so a future bug that turns this into a partial or no-op write
        // fails here instead of reading as a speedup.
        let OwnedValue::Object(doc) = eval_one(&expr, &json) else {
            panic!("n={n}: query must produce an object");
        };
        let Some(OwnedValue::Array(items)) = doc.get("items") else {
            panic!("n={n}: `.items` must survive as an array");
        };
        assert_eq!(items.len(), 2, "n={n}: both branches must survive");
        for (i, item) in items.iter().enumerate() {
            let OwnedValue::Object(item) = item else {
                panic!("n={n}: `.items[{i}]` must stay an object");
            };
            assert_eq!(
                item.get("foo"),
                Some(&OwnedValue::Array(Vec::new())),
                "n={n}: `.items[{i}].foo` must be emptied"
            );
        }

        let index = JsonIndex::build(&json);
        // Both n-element `.foo` arrays are touched, matching
        // `bench_computed_key_with_trailing_iterate`'s own accounting.
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

/// [`bench_del_computed_key_with_trailing_iterate`]'s object-tail twin,
/// covering `delete_expr_object_paths`' half of #1301's fix. Interleaved
/// A/B, Apple M-series, release, total elements = 2n: 20,000 0.18s -> 0.03s;
/// 80,000 2.02s -> 0.12s.
fn bench_del_computed_key_with_trailing_iterate_object(c: &mut Criterion) {
    let mut group = c.benchmark_group("jq_write_path_del_computed_key_trailing_iterate_object");
    let expr = parse("del(.items[(0,1)].foo[])").expect("must parse");
    for &n in SIZES {
        let json = computed_key_object_doc(n);

        // Guard the premise, as the array twin does: both `.items` entries
        // must come back with an emptied object, so a bug that turns this
        // into a no-op fails here instead of reading as a speedup.
        let OwnedValue::Object(doc) = eval_one(&expr, &json) else {
            panic!("n={n}: query must produce an object");
        };
        let Some(OwnedValue::Array(items)) = doc.get("items") else {
            panic!("n={n}: `.items` must survive as an array");
        };
        assert_eq!(items.len(), 2, "n={n}: both branches must survive");
        for (i, item) in items.iter().enumerate() {
            let OwnedValue::Object(item) = item else {
                panic!("n={n}: `.items[{i}]` must stay an object");
            };
            let Some(OwnedValue::Object(entries)) = item.get("foo") else {
                panic!("n={n}: `.items[{i}].foo` must stay an object");
            };
            assert!(
                entries.is_empty(),
                "n={n}: `.items[{i}].foo` must be emptied"
            );
        }

        let index = JsonIndex::build(&json);
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

/// The shape that reaches #1301's grouping without any computed key.
///
/// `del(.foo[].a, .foo[].b)` is an ordinary two-path comma, but the multi-path
/// walker it routes through saw one `Index(i)` sibling per element all the
/// same, and `delete_expr_array_paths`' *group* loop -- a different loop from
/// the terminal one the computed-key groups above exercise -- scanned them
/// linearly. Interleaved A/B, Apple M-series, release: 10,000 0.09s -> 0.04s;
/// 20,000 0.28s -> 0.09s; 40,000 0.96s -> 0.17s (before, x3.0 and x3.5 per
/// doubling; after, x1.95 and x1.92).
fn bench_del_comma_through_iterate(c: &mut Criterion) {
    let mut group = c.benchmark_group("jq_write_path_del_comma_through_iterate");
    let expr = parse("del(.foo[].a, .foo[].b)").expect("must parse");
    for &n in SIZES {
        let json = comma_through_iterate_doc(n);

        // Guard the premise: every element must lose `a` and `b` and keep `c`.
        let OwnedValue::Object(doc) = eval_one(&expr, &json) else {
            panic!("n={n}: query must produce an object");
        };
        let Some(OwnedValue::Array(items)) = doc.get("foo") else {
            panic!("n={n}: `.foo` must survive as an array");
        };
        assert_eq!(items.len(), n, "n={n}: no element may be removed");
        for (i, item) in items.iter().enumerate() {
            let OwnedValue::Object(entries) = item else {
                panic!("n={n}: `.foo[{i}]` must stay an object");
            };
            assert_eq!(entries.len(), 1, "n={n}: `.foo[{i}]` must keep exactly `c`");
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

// =============================================================================
// #1690: del() over a match set that grows with depth
// =============================================================================

/// Depths for the two #1690 groups. `MAX_NESTING_DEPTH` is 256, so the curve
/// is fitted over 30..240 rather than further out.
const DEPTHS: &[usize] = &[30, 60, 120, 240];

/// A `d`-deep `{"c": ...}` chain terminating in a `d`-element array:
/// `{"c":{"c":...{"c":[0,1,...,d-1]}}}`.
///
/// Both the shared prefix depth *and* the leaf fan-out scale with `d`, which
/// is what makes this shape discriminate a truly-shared trie (O(d)) from a
/// naively-flattened-per-branch one (O(d^2)). A "broom" — a fixed-size match
/// set under a deep prefix — measures **linear** however the flatten is done,
/// because the branch *count* is what multiplies the per-branch cost.
fn deep_chain_doc(d: usize) -> Vec<u8> {
    let mut out = String::new();
    for _ in 0..d {
        out.push_str(r#"{"c":"#);
    }
    out.push('[');
    for i in 0..d {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&i.to_string());
    }
    out.push(']');
    for _ in 0..d {
        out.push('}');
    }
    out.into_bytes()
}

/// [`deep_chain_doc`] at a *fixed* depth of 240 with a `k`-element leaf array,
/// so only the branch count varies.
fn wide_leaf_doc(k: usize) -> Vec<u8> {
    let mut out = String::new();
    for _ in 0..FIXED_DEPTH {
        out.push_str(r#"{"c":"#);
    }
    out.push('[');
    for i in 0..k {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&i.to_string());
    }
    out.push(']');
    for _ in 0..FIXED_DEPTH {
        out.push('}');
    }
    out.into_bytes()
}

const FIXED_DEPTH: usize = 240;
const WIDTHS: &[usize] = &[250, 500, 1_000, 2_000, 4_000];

/// Walk `d` `.c` steps into a [`deep_chain_doc`] result and return the array
/// found there, so each group can check its own premise.
fn leaf_array(mut value: &OwnedValue, d: usize) -> &OwnedValue {
    for _ in 0..d {
        let OwnedValue::Object(entries) = value else {
            panic!("expected an object at every chain level");
        };
        value = entries.get("c").expect("chain key `c` must survive");
    }
    value
}

/// `del(.. | select(type == "number"))` over [`deep_chain_doc`] — the shape
/// #1690 is about: a *filtered* recursive descent whose match set excludes
/// the document root, so #1651's root short-circuit never fires and every
/// resolved branch used to pay its own O(depth) flatten.
///
/// Note what this group does **not** isolate. Evaluating `select(...)` per
/// resolved branch inside `resolve_node` re-serializes that branch's value
/// through `to_json_for_reindex_at_depth`, which is O(subtree) per branch and
/// so O(d^2) in its own right — a separate term that dominates this shape and
/// is tracked separately from #1690. Use
/// `jq_write_path_del_shared_prefix_width` below to see the path-flatten term
/// on its own.
fn bench_del_filtered_descent_depth(c: &mut Criterion) {
    let mut group = c.benchmark_group("jq_write_path_del_filtered_descent_depth");
    let expr = parse(r#"del(.. | select(type == "number"))"#).expect("must parse");
    for &d in DEPTHS {
        let json = deep_chain_doc(d);

        // Guard the premise: every number must actually be gone, and the
        // chain itself must survive (a silent no-op would read as a speedup).
        let out = eval_one(&expr, &json);
        let OwnedValue::Array(items) = leaf_array(&out, d) else {
            panic!("d={d}: the leaf must stay an array");
        };
        assert!(items.is_empty(), "d={d}: every leaf number must be deleted");

        let index = JsonIndex::build(&json);
        group.throughput(Throughput::Elements(d as u64));
        group.bench_with_input(BenchmarkId::from_parameter(d), &json, |b, json| {
            b.iter(|| {
                let cursor = index.root(black_box(json));
                black_box(eval::<Vec<u64>, JqSemantics>(&expr, cursor))
            });
        });
    }
    group.finish();
}

/// The same computed-key delete as `bench_del_shared_prefix_width` below, but
/// with the branch count tied to the depth instead of pinned — a `D`-deep
/// chain ending in a `D`-element array, `del(.c...c[range(D)])`.
///
/// This is the shape #1690's acceptance criteria describe (both the shared
/// prefix depth *and* the leaf fan-out scale with `D`) reached the way that
/// actually exposes the term #1690 fixes: through a computed key rather than
/// a filter, so no `select` runs per branch to re-serialize that branch's
/// value. `bench_del_filtered_descent_depth` above is the same scaling shape
/// written the way the issue spells it, and is dominated by that
/// serialization instead — the two together are what show which term is
/// which.
fn bench_del_shared_prefix_depth(c: &mut Criterion) {
    let mut group = c.benchmark_group("jq_write_path_del_shared_prefix_depth");
    for &d in DEPTHS {
        let json = deep_chain_doc(d);
        let expr = parse(&format!("del({}[range({d})])", ".c".repeat(d))).expect("must parse");

        let out = eval_one(&expr, &json);
        let OwnedValue::Array(items) = leaf_array(&out, d) else {
            panic!("d={d}: the leaf must stay an array");
        };
        assert!(
            items.is_empty(),
            "d={d}: every leaf element must be deleted"
        );

        let index = JsonIndex::build(&json);
        group.throughput(Throughput::Elements(d as u64));
        group.bench_with_input(BenchmarkId::from_parameter(d), &json, |b, json| {
            b.iter(|| {
                let cursor = index.root(black_box(json));
                black_box(eval::<Vec<u64>, JqSemantics>(&expr, cursor))
            });
        });
    }
    group.finish();
}

/// The same shared-deep-prefix delete with the depth pinned at
/// [`FIXED_DEPTH`], reached by a computed key rather than a filter, so only
/// the branch count varies.
///
/// This is the group that isolates #1690's own term. Every branch shares the
/// full 240-step prefix and differs only in its final index, the computed key
/// is evaluated once against the (small) leaf array, and no `select` runs per
/// branch — so anything growing with `k` here is charged *per branch*, which
/// is exactly what the pre-#1690 per-branch flatten was and what the trie
/// removes. Reported per element, an O(depth)-per-branch cost shows up as a
/// flat-but-large ns/element and a shared-prefix one as a falling curve.
fn bench_del_shared_prefix_width(c: &mut Criterion) {
    let mut group = c.benchmark_group("jq_write_path_del_shared_prefix_width");
    let prefix = ".c".repeat(FIXED_DEPTH);
    for &k in WIDTHS {
        let json = wide_leaf_doc(k);
        let expr = parse(&format!("del({prefix}[range({k})])")).expect("must parse");

        let out = eval_one(&expr, &json);
        let OwnedValue::Array(items) = leaf_array(&out, FIXED_DEPTH) else {
            panic!("k={k}: the leaf must stay an array");
        };
        assert!(
            items.is_empty(),
            "k={k}: every leaf element must be deleted"
        );

        let index = JsonIndex::build(&json);
        group.throughput(Throughput::Elements(k as u64));
        group.bench_with_input(BenchmarkId::from_parameter(k), &json, |b, json| {
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
    bench_del_computed_key_with_trailing_iterate,
    bench_del_computed_key_with_trailing_iterate_object,
    bench_del_comma_through_iterate,
    bench_del_filtered_descent_depth,
    bench_del_shared_prefix_depth,
    bench_del_shared_prefix_width,
);
criterion_main!(benches);
