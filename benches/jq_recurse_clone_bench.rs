//! Depth-scaling benchmark for `push_recursive_branches`/`resolve_recurse`'s
//! per-node clone cost (#668), split out of #675.
//!
//! `jq_recurse_depth_bench`'s query, `.. | .[.k]?`, does **not** reach
//! `push_recursive_branches`/`resolve_recurse` (`src/jq/eval.rs`): bare `..`
//! in value position dispatches to `eval_recursive_descent`/
//! `collect_recursive`, which clones a cheap `StandardJson` cursor per node,
//! not the materialized `OwnedValue` — see that benchmark's own corrected
//! docstring. `push_recursive_branches`/`resolve_recurse` are reachable only
//! through `resolve_node`, which `resolve_dynamic_indexes` calls solely when
//! `needs_path_prepass` is true — true for `..`/`recurse` themselves, so only
//! a *path-context* use (`path(..)`, `path(recurse(...))`, `=`, `|=`,
//! `del()`) reaches them (#668's own description of its four callers).
//!
//! **Why `del(..)`, not `path(..)`.** `path(..)`, the more obvious trigger,
//! turns out to be a poor isolator: `builtin_path` calls
//! `resolve_dynamic_indexes` (which drives `push_recursive_branches`) and
//! *then* re-walks every one of the `depth + 1` resolved paths from the
//! document root via `walk_path`/`step_into`, each step of which clones the
//! value reached so far. That second pass was measured (`temp_probe`-style,
//! not checked in) to cost **~250x** `push_recursive_branches`'s own share at
//! depth 400 (1.9s vs 7-8ms) and scales worse than quadratically — so a
//! `path(..)` benchmark would be dominated by `walk_path`, not by the
//! function #668 targets, near-invisible to that fix, and would repeat
//! exactly the mistake #675 was filed to correct in the first place.
//! `del(..)` instead reaches `resolve_del_path_branches` and then applies
//! `delete_at_path`/`DeleteTrie` directly to the already-resolved static
//! paths — no second walk of the original tree — so its cost tracks
//! `push_recursive_branches`'s own share closely.
//!
//! #701 found the depth-400 point in the list below panicking
//! (`nesting depth exceeds limit of 384`, from `to_owned_at_depth`,
//! unrelated to this file's own machinery — it fires before path-tracking
//! even starts) — invisible until now because `cargo bench` isn't run by
//! `cargo test`/CI. This file's depths are now derived from
//! `MAX_VALUE_TREE_DEPTH` so they can't silently exceed it again; the old
//! "measured... at depth 400" figures above were never actually
//! reproducible as checked in and should not be trusted.
//!
//! `del(..)` also has no computed index anywhere in the query, so — like
//! `path(..)` — it can never route through `eval_index_expr`/
//! `eval_slice_bound` (#626/#670's already-fixed target).
//!
//! On the linear-nesting document below (one child per level, depth `d`),
//! `push_recursive_branches` visits `d + 1` nodes pre-order and clones the
//! full subtree rooted at each: the node at depth `i` clones a subtree of
//! size `O(d - i)`. Summed over all nodes, that's `O(d^2)` — the signature
//! #668 targets. `resolve_dynamic_indexes`, `del(..)`'s sole consumer of this
//! machinery, discards every branch's cloned value once it has read the
//! branch's path components (`delete_at_path` only needs the path, not the
//! value that was sitting there), so that clone cost is pure waste — exactly
//! the waste #668 describes.
//!
//! **#1651 update.** #701 made `push_recursive_branches`'s *path* half O(1)
//! per node (an `Rc<PathPrefix>` cons-list replacing a `Vec<Expr>` clone),
//! but this benchmark's growth exponent stayed k≈1.94 regardless — because
//! `del(..)`'s own consumer, not `push_recursive_branches`, still flattened
//! that O(1)-to-construct chain back down to an owned `Vec<Expr>`
//! (`resolve_dynamic_indexes`'s `assemble`) and then *again* to
//! `Vec<DeleteStep>` (`builtin_del`'s `flatten_delete_path`) — once per
//! resolved branch, unconditionally, even though this fixture's own guard
//! above proves the result is always `null` and neither flattened form is
//! ever inspected. `del()` now resolves through `resolve_del_path_branches`,
//! which reports the document root as `DelPaths::Root` (true for `..`/bare
//! `recurse`/`recurse(f)`/`recurse(f;cond)` unconditionally, since each
//! emits self before recursing into children) and skips both flattens. This
//! benchmark now exercises only `push_recursive_branches`'s own O(d) branch
//! construction (still real work — every node's path is still resolved) —
//! its growth exponent should read ~k≈1 post-fix.
//!
//! **#1690 update.** A **filtered** recurse whose match set excludes the
//! root (`del(.. | select(cond))` where `cond` rejects `.`) never reaches
//! `DelPaths::Root`, so #1651 left it paying both flattens in full. #1690
//! removed them for every multi-path `del()` by merging the resolved paths
//! into a `DeleteTrie` rather than flattening each branch independently. It
//! is deliberately still not covered *here* — that shape's own cost is
//! dominated by a different term (`select` re-serializing each branch's
//! value through `to_json_for_reindex_at_depth`), which is exactly the
//! "poor isolator" problem this file's header is about; see
//! `benches/jq_write_path_bench.rs`'s
//! `jq_write_path_del_shared_prefix_width` group for the isolating
//! fixture.
//!
//! This file makes **no timing assertion** — it is a Criterion benchmark for
//! manual before/after comparison, not a CI gate. Run it interleaved
//! before/after #668's fix lands, per the A/B method in
//! `docs/guides/benchmarking.md#ab-benchmarking-method`, and record the
//! resulting table in that issue/PR rather than here.
//!
//! Run with:
//! ```bash
//! cargo bench --bench jq_recurse_clone_bench
//! ```

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;
use succinctly::jq::{eval, parse, JqSemantics, OwnedValue, QueryResult, MAX_VALUE_TREE_DEPTH};
use succinctly::json::JsonIndex;

/// `{"k": {"k": ... {} ... }}`, `depth` levels of `"k"` nesting, no other
/// fields — nothing here needs a computed index to reach, unlike
/// `jq_recurse_depth_bench`'s `pad` sibling (added there only to give
/// `.[.k]?` a non-nesting object to fail an index into).
fn linear_nest(depth: usize) -> Vec<u8> {
    let open = "{\"k\":".repeat(depth);
    let close = "}".repeat(depth);
    format!("{open}{{}}{close}").into_bytes()
}

fn bench_recurse_clone_depth(c: &mut Criterion) {
    let mut group = c.benchmark_group("jq_recurse_clone_depth");
    // Matches #661/#626's depths (100/200/300) where possible so results
    // stay comparable across this benchmark family; the top depth is derived
    // from `MAX_VALUE_TREE_DEPTH` (384) rather than hardcoded 400, which
    // panics past the limit (#701 — see the module doc above).
    for &depth in &[100usize, 200, 300, MAX_VALUE_TREE_DEPTH - 9] {
        let json = linear_nest(depth);
        let index = JsonIndex::build(&json);
        let expr = parse("del(..)").expect("filter parses");
        // Guard the premise: deleting every node including the root
        // collapses the whole document to `null` — confirmed against real
        // jq (`jq 'del(..)'`) — regardless of depth. A different result
        // would mean this fixture stopped exercising the full `d + 1`-node
        // fan-out `push_recursive_branches` walks.
        let cursor = index.root(&json);
        let probe: QueryResult<Vec<u64>> = eval::<Vec<u64>, JqSemantics>(&expr, cursor);
        assert!(
            matches!(probe, QueryResult::Owned(OwnedValue::Null)),
            "depth {depth} fixture must delete down to null"
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

criterion_group!(benches, bench_recurse_clone_depth);
criterion_main!(benches);
