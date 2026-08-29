//! #1827: end-to-end behavioral equivalence between `Expr::Index`/`Expr::Slice`
//! and their float-spelling-preserving siblings `Expr::IndexNumber`/
//! `Expr::SliceNumber` (`src/jq/expr.rs`).
//!
//! `.[1]` folds to `Expr::Index(1)`; `.[1.0]` (same integer value, float
//! spelling) folds to `Expr::IndexNumber { idx: 1, .. }` instead, so
//! `path()` can report the component's own source spelling (#1088, #1326).
//! The two must behave identically for navigation — reading, `del`,
//! `setpath`/`=`/`|=`, `getpath` — with `path()`'s own component rendering
//! the only place they're allowed to differ.
//!
//! This is the regression class #1088/#1326 already hit once:
//! `delete_expr_array_paths`'s error-construction match had no
//! `Expr::IndexNumber` arm at all from #1088 until #1326 added one as a
//! byproduct, so a float-spelled index reaching it would have panicked via
//! `unreachable!()`. `src/jq/eval.rs` has ~30 sites that pattern-match on
//! this pair (or its `Slice`/`SliceNumber` sibling) — most via the shared
//! predicates or an `Expr::Index(idx) | Expr::IndexNumber { idx, .. }`
//! or-pattern, but nothing enforces that a *new* site does so, or that a
//! *third* paired variant joining either family gets taught to every one of
//! them. Running real filters through the full evaluator, as this file
//! does, is what would have caught the #1088/#1326 gap: a missing arm
//! panics or errors differently, not just "the wrong predicate returns
//! false".
//!
//! Every expectation below is pinned against jq-1.7.1 (the version in
//! `tests/data/jq-golden/JQ_VERSION`).

use succinctly::jq::{eval, parse, JqSemantics, QueryResult};
use succinctly::json::JsonIndex;

/// Render every output of the full evaluator as a compact JSON string.
fn full_outputs(json: &[u8], filter: &str) -> Vec<String> {
    let index = JsonIndex::build(json);
    let cursor = index.root(json);
    let expr = parse(filter).expect("parse failed");
    let result: QueryResult<Vec<u64>> = eval::<Vec<u64>, JqSemantics>(&expr, cursor);
    result
        .collect_owned()
        .iter()
        .map(succinctly::jq::OwnedValue::to_json)
        .collect()
}

/// Convenience: assert the filter produces exactly one output and return it.
fn one(json: &[u8], filter: &str) -> String {
    let outs = full_outputs(json, filter);
    assert_eq!(
        outs.len(),
        1,
        "expected exactly one output for `{filter}`, got {outs:?}"
    );
    outs.into_iter().next().unwrap()
}

const ARRAY: &[u8] = b"[10,20,30,40]";

#[test]
fn test_index_number_navigation_matches_plain_index_1827() {
    for filter in [".[1]", ".[1.0]", ".[1e0]"] {
        assert_eq!(one(ARRAY, filter), "20", "read: {filter}");
        assert_eq!(
            one(ARRAY, &format!("del({filter})")),
            "[10,30,40]",
            "del: {filter}"
        );
        assert_eq!(
            one(
                ARRAY,
                &format!("getpath([{}])", &filter[2..filter.len() - 1])
            ),
            "20",
            "getpath: {filter}"
        );
        assert_eq!(
            one(ARRAY, &format!("{filter} = 99")),
            "[10,99,30,40]",
            "=: {filter}"
        );
        assert_eq!(
            one(ARRAY, &format!("{filter} |= . + 1")),
            "[10,21,30,40]",
            "|=: {filter}"
        );
    }
}

/// #1827's own trigger case: `del` with more than one target routes through
/// `delete_expr_paths_at`/`delete_expr_array_paths` (`src/jq/eval.rs`), a
/// *different* function from single-target `del`'s fast path -- the one
/// whose `Expr::IndexNumber`/`Expr::SliceNumber` match arms were genuinely
/// missing from #1088 until #1326 added them. Confirmed this reaches (and
/// previously would have panicked in) that exact function by reverting its
/// or-pattern arms to plain `Expr::Index`/`Expr::Slice` locally and
/// re-running this test, which then failed with
/// `unreachable!("delete_expr_paths_at only dispatches Index/Slice paths
/// here")` -- the single-target cases above did not reproduce that panic.
#[test]
fn test_multi_target_del_reaches_index_number_dispatch_1827() {
    for filter in [
        "del(.[1.0], .[3])",
        "del(.[1], .[3])",
        "del(.[1.0:2], .[3])",
    ] {
        assert_eq!(one(ARRAY, filter), "[10,30]", "{filter}");
    }
}

#[test]
fn test_index_number_path_rendering_preserves_spelling_1827() {
    // Only `path()`'s own component rendering is allowed to differ --
    // everything else above must be byte-identical.
    assert_eq!(one(ARRAY, "path(.[1])"), "[1]");
    assert_eq!(one(ARRAY, "path(.[1.0])"), "[1.0]");
    // `1e0` renders identically to `1` once formatted, so unlike `1.0` it
    // does not carry a distinct spelling to preserve here.
    assert_eq!(one(ARRAY, "path(.[1e0])"), "[1]");
}

#[test]
fn test_slice_number_navigation_matches_plain_slice_1827() {
    for filter in [".[1:3]", ".[1.0:3]", ".[1:3.0]"] {
        assert_eq!(one(ARRAY, filter), "[20,30]", "read: {filter}");
        assert_eq!(
            one(ARRAY, &format!("del({filter})")),
            "[10,40]",
            "del: {filter}"
        );
        assert_eq!(
            one(ARRAY, &format!("{filter} = [99]")),
            "[10,99,40]",
            "=: {filter}"
        );
    }
    // getpath's own path-component shape is independent of either bound's
    // spelling, so a single representative case here (not a loop) is
    // enough to confirm the shared component-parsing path still resolves.
    assert_eq!(one(ARRAY, r#"getpath([{"start":1,"end":3}])"#), "[20,30]");
}

#[test]
fn test_slice_number_path_rendering_preserves_spelling_1827() {
    assert_eq!(one(ARRAY, "path(.[1:3])"), r#"[{"start":1,"end":3}]"#);
    assert_eq!(one(ARRAY, "path(.[1.0:3])"), r#"[{"start":1.0,"end":3}]"#);
    assert_eq!(one(ARRAY, "path(.[1:3.0])"), r#"[{"start":1,"end":3.0}]"#);
}
