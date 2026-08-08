//! Depth-scaling benchmark for `..`/`recurse` path resolution (#661).
//!
//! De-risk step for #626: `push_recursive_branches` (bare `..`/`recurse`) and
//! `resolve_recurse` (`recurse(f)`/`recurse(f;cond)`), both in
//! `src/jq/eval.rs`, clone the value at every visited node to build that
//! node's `PathBranch`. On a linear-nesting document that clone is
//! O(subtree), so the total cost across all nodes is O(depth^2). This
//! reproduces #626's own synthetic document and query so a before/after run
//! of that fix produces numbers directly comparable to the table in that
//! issue.
//!
//! The document — `{"k": {"k": ... "pad": {"a":{},"b":{},"c":{}}}}` — and
//! the query — `.. | .[.k]?` — are chosen so the query never errors and
//! never produces output: every `.k` lookup either finds the next nesting
//! level (an object, so `.[<object>]` fails and `?` suppresses it) or misses
//! (null, same suppression). That isolates the fan-out/per-node clone cost
//! #626 targets from any output-formatting cost.
//!
//! This file makes **no timing assertion** — it is a Criterion benchmark for
//! manual before/after comparison, not a CI gate. Run it interleaved
//! before/after #626 lands, per the A/B method in
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
