//! #1827: end-to-end behavioral equivalence between an integer-spelled and
//! a float-spelled static path component (`Expr::Index`/`Expr::Slice`,
//! `src/jq/expr.rs`).
//!
//! `.[1]` folds to `Expr::Index { idx: 1, key: None }`; `.[1.0]` (same
//! integer value, float spelling) folds to the same variant with `key`
//! populated, so `path()` can report the component's own source spelling
//! (#1088, #1326). The two must behave identically for navigation —
//! reading, `del`, `setpath`/`=`/`|=`, `getpath` — with `path()`'s own
//! component rendering the only place they're allowed to differ.
//!
//! This is the regression class #1088/#1326 already hit once. Each spelling
//! used to be its *own* variant (`Expr::IndexNumber` beside `Expr::Index`,
//! `Expr::SliceNumber` beside `Expr::Slice`), so every site had to spell
//! out both members of the pair — and `delete_expr_array_paths`'s
//! error-construction match had no `IndexNumber` arm at all from #1088
//! until #1326 added one as a byproduct, so a float-spelled index reaching
//! it would have panicked via `unreachable!()`.
//!
//! #1401 folded the keys onto the surviving variants, which makes a missing
//! *sibling* arm unrepresentable — one variant per family, so the compiler
//! sees every site. What this file still covers is the property that
//! motivated the pairs in the first place, and that no type can enforce:
//! that the two spellings actually reach the same behavior end to end,
//! through the full evaluator, at every dispatch site a real filter can
//! drive. A site that keyed off the spelling rather than `idx` would still
//! be caught here.
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

/// `del` with more than one target routes through `delete_expr_array_paths`
/// (`src/jq/eval.rs`), a *different* function from single-target `del`'s
/// fast path -- confirmed by local experimentation that this test suite's
/// single-target cases above do not reach it at all (reverting its
/// `ArrayStep`-classification match's or-pattern arms locally left every
/// test above green; only a multi-target `del` here panicked with
/// `unreachable!("delete_expr_paths_at only dispatches Index/Slice paths
/// here")`).
///
/// This is a *different* match inside the same function from the one
/// #1088/#1326's own history is about (`delete_expr_array_paths`'s
/// *error-construction* match, for a non-array target) -- that one has no
/// real parsed-filter reproduction at all, per its own existing direct-call
/// regression tests' doc comments
/// (`test_delete_expr_array_paths_reports_index_number_like_plain_index_1326`/
/// `test_delete_expr_array_paths_reports_slice_number_like_plain_slice_1827`
/// in `src/jq/eval.rs`, both of which explicitly note a parsed
/// `del(.[2.0])`-shaped query exits through a different, higher-level
/// dispatch first). This test still earns its place: the `ArrayStep` match
/// is a real, independent dispatch site among the ~30 the issue is about,
/// and CLI-reachable, unlike the error-construction one.
#[test]
fn test_multi_target_del_reaches_array_step_dispatch_1827() {
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
