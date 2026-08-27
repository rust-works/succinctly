//! Depth-scaling benchmark for `collect_leaf_paths`'s per-node prefix-rebuild
//! cost (#1657), isolating one of the four sibling functions #1657 covers
//! (`collect_paths`/`collect_leaf_paths`/`collect_tostream_events`/
//! `collect_stream`, `src/jq/eval.rs`) from the others.
//!
//! Reuses `jq_recurse_clone_bench.rs`'s `linear_nest` fixture verbatim:
//! `{"k": {"k": ... {} ...}}`, `depth` levels of `"k"` nesting, terminating
//! in an empty object. `[leaf_paths]` (not `[paths]`) is the isolator here,
//! deliberately, not just an arbitrary pick between the two options #1657
//! names:
//!
//! - `[leaf_paths]` on this fixture visits `depth` nodes along the chain but
//!   records exactly **one** output — the single leaf (the innermost `{}`,
//!   a leaf under this crate's tree-structural definition, see
//!   `collect_leaf_paths`'s doc comment and #771) at path length `depth`.
//!   Total *necessary* output size is `O(depth)`. Before #1657, each of the
//!   `depth` descents still rebuilt the whole current-depth prefix via
//!   `current_path.to_vec()` before extending it by one component — an
//!   `O(depth)` rebuild repeated at every level, `O(depth^2)` summed, even
//!   though only one path is ever recorded. Push/recurse/pop reduces that
//!   traversal to `O(1)` per level (`O(depth)` total), leaving only the
//!   single unavoidable `O(depth)` clone at the leaf — `O(depth)` overall,
//!   i.e. linear.
//! - `[paths]` on the same fixture is a poor isolator by contrast: `paths`
//!   records **every** node's path, not just leaves, so a `depth`-level
//!   chain with one child per level yields `depth` outputs of lengths
//!   `1..=depth` — `O(depth^2)` of *necessary* output regardless of the
//!   traversal fix (writing that many result elements can't be done in less
//!   than the time it takes to write them). A `[paths]` benchmark on a plain
//!   chain would still show quadratic growth after the fix, just with a
//!   smaller constant, and would not demonstrate the asymptotic improvement
//!   #1657's acceptance criteria call for.
//!
//! This file makes **no timing assertion** — it is a Criterion benchmark for
//! manual before/after comparison, not a CI gate. Run it interleaved
//! before/after #1657's fix lands, per the A/B method in
//! `docs/guides/benchmarking.md#ab-benchmarking-method`, and record the
//! resulting table in that issue/PR rather than here.
//!
//! Run with:
//! ```bash
//! cargo bench --bench jq_leaf_paths_clone_bench
//! ```

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;
use succinctly::jq::{eval, parse, JqSemantics, OwnedValue, QueryResult, MAX_VALUE_TREE_DEPTH};
use succinctly::json::JsonIndex;

/// `{"k": {"k": ... {} ... }}`, `depth` levels of `"k"` nesting, no other
/// fields — identical to `jq_recurse_clone_bench.rs`'s fixture of the same
/// name, duplicated here rather than shared so this file stays a
/// self-contained, independently runnable benchmark like its siblings.
fn linear_nest(depth: usize) -> Vec<u8> {
    let open = "{\"k\":".repeat(depth);
    let close = "}".repeat(depth);
    format!("{open}{{}}{close}").into_bytes()
}

fn bench_leaf_paths_clone_depth(c: &mut Criterion) {
    let mut group = c.benchmark_group("jq_leaf_paths_clone_depth");
    // Same depth points as `jq_recurse_clone_bench.rs` for cross-benchmark
    // comparability; the top depth is derived from `MAX_VALUE_TREE_DEPTH`
    // (384) rather than hardcoded, since `collect_leaf_paths` panics past it
    // (#1025).
    for &depth in &[100usize, 200, 300, MAX_VALUE_TREE_DEPTH - 9] {
        let json = linear_nest(depth);
        let index = JsonIndex::build(&json);
        let expr = parse("[leaf_paths]").expect("filter parses");
        // Guard the premise: exactly one leaf path, of length `depth`,
        // pointing at the innermost `{}` -- a different shape would mean
        // this fixture stopped isolating the single-output case the module
        // doc above depends on.
        let cursor = index.root(&json);
        let probe: QueryResult<Vec<u64>> = eval::<Vec<u64>, JqSemantics>(&expr, cursor);
        match probe {
            QueryResult::Owned(OwnedValue::Array(paths)) => {
                assert_eq!(
                    paths.len(),
                    1,
                    "depth {depth} fixture must yield exactly one leaf path"
                );
                match &paths[0] {
                    OwnedValue::Array(components) => assert_eq!(
                        components.len(),
                        depth,
                        "depth {depth} fixture's leaf path must have length {depth}"
                    ),
                    other => panic!("depth {depth} fixture: expected an array path, got {other:?}"),
                }
            }
            other => panic!("depth {depth} fixture: expected a single-array result, got {other:?}"),
        }

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

criterion_group!(benches, bench_leaf_paths_clone_depth);
criterion_main!(benches);
