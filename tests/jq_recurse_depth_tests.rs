//! Correctness-at-depth pinning for `..`/`recurse` path resolution (#661).
//!
//! De-risk step for #626 (threading a `Cow<'a, OwnedValue>` lifetime through
//! `PathBranch`/`resolve_node` in `src/jq/eval.rs` to kill the O(subtree)
//! clone-per-node cost in `push_recursive_branches`/`resolve_recurse`).
//! Every existing recurse/`..` golden fixture nests three levels deep at
//! most, so nothing today would catch a lifetime bug that only corrupts or
//! truncates output once recursion goes deeper than a handful of frames.
//! This drives both code paths #626 touches to depth 300 and asserts their
//! exact, independently-derived output.
//!
//! Depth 300 is chosen deliberately beyond what real jq's own JSON parser
//! accepts (jq-1.7.1 raises "Exceeds depth limit for parsing" around depth
//! ~128 on this document shape), so this is a pure internal correctness pin
//! against a programmatically-derived expected value, not an oracle
//! comparison — there is no jq to compare against at this depth.
//!
//! No timing assertion: correctness only. See `benches/jq_recurse_depth_bench.rs`
//! for the depth-scaling *timing* benchmark this issue also adds.
//!
//! Run with: cargo test --test jq_recurse_depth_tests

use succinctly::jq::{eval, parse, JqSemantics, OwnedValue, QueryResult};
use succinctly::json::JsonIndex;

/// Deep enough to exercise many recursion frames; see module doc for why this
/// exceeds jq's own parser depth limit.
const DEPTH: usize = 300;

/// `{"k":{"k":...{}...}}`, `depth` levels of `"k"` nesting, terminating in `{}`.
fn linear_nest(depth: usize) -> String {
    format!("{}{{}}{}", "{\"k\":".repeat(depth), "}".repeat(depth))
}

/// The paths `path(..)`/`path(recurse(...))` visit on `linear_nest(depth)`,
/// derived independently of any evaluator: the root (`[]`), then one more
/// `"k"` per level down to the innermost `{}`.
fn expected_paths(depth: usize) -> Vec<String> {
    (0..=depth)
        .map(|i| format!("[{}]", vec!["\"k\""; i].join(",")))
        .collect()
}

/// Run `filter` against `json` through the library's full evaluator and
/// render each output path with `OwnedValue::to_json`, matching the
/// `expected_paths` string shape above.
fn run_paths(json: &str, filter: &str) -> Vec<String> {
    let bytes = json.as_bytes();
    let index = JsonIndex::build(bytes);
    let cursor = index.root(bytes);
    let expr = parse(filter).expect("parse failed");
    let result: QueryResult<Vec<u64>> = eval::<Vec<u64>, JqSemantics>(&expr, cursor);
    assert!(
        !result.is_error(),
        "`{filter}` errored on the depth-{DEPTH} document: {result:?}"
    );
    result
        .collect_owned()
        .iter()
        .map(OwnedValue::to_json)
        .collect()
}

/// Bare `..` — the `push_recursive_branches` path.
#[test]
fn recursive_descent_correct_at_depth() {
    let json = linear_nest(DEPTH);
    assert_eq!(run_paths(&json, "path(..)"), expected_paths(DEPTH));
}

/// Bare `recurse` — shares `push_recursive_branches` with `..` (per the
/// dispatch comment in `resolve_node`), pinned separately in case a future
/// refactor splits the two.
#[test]
fn bare_recurse_correct_at_depth() {
    let json = linear_nest(DEPTH);
    assert_eq!(run_paths(&json, "path(recurse)"), expected_paths(DEPTH));
}

/// `recurse(f; cond)` — the `resolve_recurse` BFS-queue path. `cond` stops
/// exactly at the `{}` leaf (its `.k` is `null`, not an object), producing
/// the same path set as the bare form on this document.
#[test]
fn parameterized_recurse_correct_at_depth() {
    let json = linear_nest(DEPTH);
    assert_eq!(
        run_paths(&json, r#"path(recurse(.k; type=="object"))"#),
        expected_paths(DEPTH)
    );
}
