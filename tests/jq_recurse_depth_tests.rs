//! Correctness-at-depth pinning for `..`/`recurse` path resolution through
//! the public `eval()` API (#661).
//!
//! Originally a de-risk step for #626 (threading a `Cow<'a, OwnedValue>`
//! lifetime through `PathBranch`/`resolve_node` in `src/jq/eval.rs` to kill
//! the O(subtree) clone-per-node cost in
//! `resolve_recursive_descent_sink`/`resolve_recurse_sink`), driving both
//! code paths to depth 300 through this file's own `eval()` calls.
//!
//! #3266 routes every `path(f)` from `eval()` onto the generic evaluator
//! instead (closing a cross-evaluator inconsistency where `eval()` raised on
//! an unreadable sibling that the CLI's own evaluator already navigated
//! past), so `eval()`'s own `path(...)` calls no longer reach
//! `resolve_recursive_descent_sink`/`resolve_recurse_sink` at all -- this
//! file's depth-300 pin moved to `src/jq/eval.rs`'s own test module, calling
//! `eval_full` directly to keep testing #626's actual code unaffected by the
//! routing change. What's left here instead pins the *public* API's new,
//! CLI-consistent capacity: the generic evaluator's own path walker recurses
//! natively and caps out at `eval_generic::MAX_NESTING_DEPTH` (256) for
//! native-stack safety -- confirmed live, depth 255 succeeds and 256 raises
//! `nesting depth exceeds limit of 256`, identically through the CLI and
//! `eval()`. DEPTH here is kept safely under that ceiling.
//!
//! Every existing recurse/`..` golden fixture nests three levels deep at
//! most, so nothing else would catch a lifetime bug that only corrupts or
//! truncates output once recursion goes deeper than a handful of frames --
//! 200 is still two orders of magnitude past that, while leaving headroom
//! under the 256 ceiling.
//!
//! No timing assertion: correctness only. See `benches/jq_recurse_depth_bench.rs`
//! for the depth-scaling *timing* benchmark this issue also adds.
//!
//! Run with: cargo test --test jq_recurse_depth_tests

use succinctly::jq::{eval, parse, JqSemantics, OwnedValue, QueryResult};
use succinctly::json::JsonIndex;

/// Deep enough to exercise many recursion frames while staying under the
/// generic evaluator's 256-deep native-recursion ceiling (see module doc).
const DEPTH: usize = 200;

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

/// Run `filter` against `json` through the library's public `eval()` entry
/// and render each output path with `OwnedValue::to_json`, matching the
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
        .collect_owned::<JqSemantics>()
        .iter()
        .map(OwnedValue::to_json)
        .collect()
}

/// Bare `..` through the public `eval()` entry, now the generic evaluator's
/// `path_walk_generic`.
#[test]
fn recursive_descent_correct_at_depth() {
    let json = linear_nest(DEPTH);
    assert_eq!(run_paths(&json, "path(..)"), expected_paths(DEPTH));
}

/// Bare `recurse` — same generic-evaluator route as `..` above.
#[test]
fn bare_recurse_correct_at_depth() {
    let json = linear_nest(DEPTH);
    assert_eq!(run_paths(&json, "path(recurse)"), expected_paths(DEPTH));
}

/// `recurse(f; cond)`. `cond` stops exactly at the `{}` leaf (its `.k` is
/// `null`, not an object), producing the same path set as the bare form on
/// this document.
#[test]
fn parameterized_recurse_correct_at_depth() {
    let json = linear_nest(DEPTH);
    assert_eq!(
        run_paths(&json, r#"path(recurse(.k; type=="object"))"#),
        expected_paths(DEPTH)
    );
}
