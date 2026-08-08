//! Depth-scaling benchmark for `..`/`recurse` path resolution (#661).
//!
//! De-risk step for #626, validating PR #670's fix: `eval_index_expr`/
//! `eval_slice_bound` (`src/jq/eval.rs`) used to materialize a computed
//! index/bound's full subtree before checking whether it was even usable. On
//! a linear-nesting document that materialization is O(subtree), so the
//! total cost across all nodes is O(depth^2). This reproduces #626's own
//! synthetic document and query so a before/after run of that fix produces
//! numbers directly comparable to the table in that issue — #670 took the
//! depth-400 case from 11.7ms to 93µs.
//!
//! **This benchmark does not cover #668.** It was originally written
//! attributing the O(depth^2) cost to `push_recursive_branches`/
//! `resolve_recurse` (the `..`/`recurse` fan-out itself) rather than to
//! `eval_index_expr`'s computed-key handling — plausible since the query
//! below combines both, but wrong: PR #670 fixed `eval_index_expr` only, and
//! that alone reproduced the full speedup, so the fan-out was never the
//! bottleneck this benchmark measures. `push_recursive_branches`/
//! `resolve_recurse` do still clone the value at every visited node
//! (`eval.rs:8595`, `8659`, `8664`), independently of this benchmark's
//! query — that's #668, and #675 tracks adding a benchmark that isolates it
//! (a query with no computed index, so it can't route through
//! `eval_index_expr`).
//!
//! The document — `{"k": {"k": ... "pad": {"a":{},"b":{},"c":{}}}}` — and
//! the query — `.. | .[.k]?` — are chosen so the query never errors and
//! never produces output: every `.k` lookup either finds the next nesting
//! level (an object, so `.[<object>]` fails and `?` suppresses it) or misses
//! (null, same suppression). That isolates the computed-index cost #626
//! targets from any output-formatting cost.
//!
//! This file makes **no timing assertion** — it is a Criterion benchmark for
//! manual before/after comparison, not a CI gate. Run it interleaved
//! before/after a relevant fix lands, per the A/B method in
//! `docs/guides/benchmarking.md#ab-benchmarking-method`, and record the
//! resulting table in that issue/PR rather than here.
//!
//! Run with:
//! ```bash
//! cargo bench --bench jq_recurse_depth_bench
//! ```

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;
use succinctly::jq::{eval, parse, JqSemantics, QueryResult};
use succinctly::json::JsonIndex;

/// `{"k": {"k": ... "pad": {"a":{},"b":{},"c":{}}}}`, `depth` levels of `"k"`
/// nesting — #626's synthetic linear-nesting document, verbatim.
fn linear_nest_with_pad(depth: usize) -> Vec<u8> {
    let open = "{\"k\":".repeat(depth);
    let close = "}".repeat(depth);
    format!("{open}{{\"pad\":{{\"a\":{{}},\"b\":{{}},\"c\":{{}}}}}}{close}").into_bytes()
}

fn bench_recurse_depth(c: &mut Criterion) {
    let mut group = c.benchmark_group("jq_recurse_depth");
    // #626's own table uses these four depths — matched here so a
    // before/after run is directly comparable to it.
    for &depth in &[100usize, 200, 300, 400] {
        let json = linear_nest_with_pad(depth);
        let index = JsonIndex::build(&json);
        let expr = parse(".. | .[.k]?").expect("filter parses");
        // Guard the premise: a fixture that errored or produced output would
        // no longer be isolating the clone cost this benchmark targets.
        let cursor = index.root(&json);
        let probe: QueryResult<Vec<u64>> = eval::<Vec<u64>, JqSemantics>(&expr, cursor);
        assert!(!probe.is_error(), "depth {depth} fixture must not error");
        assert!(
            probe.collect_owned().is_empty(),
            "depth {depth} fixture must produce no output"
        );

        group.throughput(Throughput::Elements(depth as u64));
        group.bench_with_input(BenchmarkId::from_parameter(depth), &json, |b, json| {
            b.iter(|| {
                let cursor = index.root(black_box(json));
                let result: QueryResult<Vec<u64>> = eval::<Vec<u64>, JqSemantics>(&expr, cursor);
                black_box(result.collect_owned().len())
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_recurse_depth);
criterion_main!(benches);
