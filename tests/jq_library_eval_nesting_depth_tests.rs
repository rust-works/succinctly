//! #2627: a library embedder calling `succinctly::jq::eval` directly (no
//! CLI, no `catch_unwind`) must never see a raw panic on a >256-deep
//! document, even for a filter that reads no path context at all.
//!
//! `eval::eval`'s own top-level gate (`needs_path_context`) routes a whole
//! expression through `eval_generic` only when *some* part of it needs path
//! context (`key`/`parent`/`path`/...). A bare `true and true`/`1+1` on its
//! own therefore never reaches `eval_generic`'s internal materialization
//! through the public API -- but a pipe that combines a path-context need
//! with an otherwise-ungated sub-expression does, and once inside
//! `eval_generic`, that sub-expression's own `needs_path_context` gate can
//! independently be `false`, sending it to the same wildcard/ambient bridge
//! `tests/jq_cli_tests.rs`'s `test_boolean_wildcard_bridge_over_depth_
//! document_reports_cleanly_not_panic_2627` pins at the CLI layer (protected
//! there by `catch_unwind`, #1793). This file confirms the same document
//! and filter shape, reached through the *public* library entry point with
//! no such net, returns a clean `Err` instead of aborting the process.
//!
//! Confirmed live before the fix that this exact shape panicked with no
//! `catch_unwind` to save it (the underlying `assert_nesting_depth` `panic!`
//! in `src/jq/eval_generic.rs`).

use succinctly::jq::{eval, parse, JqSemantics, QueryResult};
use succinctly::json::JsonIndex;

/// `[[[...1...]]]`, `depth` levels of array nesting wrapping a single `1` --
/// mirrors `jq_recurse_depth_tests.rs`'s own `linear_nest` and `jq_cli_tests
/// .rs`'s `nested_arrays`.
fn nested_arrays(depth: usize) -> String {
    format!("{}1{}", "[".repeat(depth), "]".repeat(depth))
}

/// `key?` forces `eval::eval`'s own gate to route the whole pipe through
/// `eval_generic` (confirmed by `needs_path_context`'s `Expr::Builtin(Builtin
/// ::Key) => true` arm); `(true and true)` has no path-context operand of
/// its own, so once inside `eval_generic` it independently falls to the
/// wildcard/ambient bridge this issue is about.
const FILTER: &str = "key?, (true and true)";

#[test]
fn test_public_eval_over_depth_document_returns_error_not_panic_2627() {
    let json = nested_arrays(300);
    let bytes = json.as_bytes();
    let index = JsonIndex::build(bytes);
    let cursor = index.root(bytes);
    let expr = parse(FILTER).expect("parse failed");

    let result: QueryResult<Vec<u64>> = eval::<Vec<u64>, JqSemantics>(&expr, cursor);
    match result {
        QueryResult::Error(e) => {
            assert!(e.is_decode_failure(), "expected decode failure, got: {e:?}");
            assert!(
                e.message.contains("nesting depth exceeds limit of 256"),
                "message: {}",
                e.message
            );
        }
        other => panic!("expected a decode failure, got: {other:?}"),
    }
}

/// Companion: the same filter on a document well under the limit must still
/// evaluate normally through the public API.
#[test]
fn test_public_eval_accepts_depth_under_limit_2627() {
    let json = nested_arrays(100);
    let bytes = json.as_bytes();
    let index = JsonIndex::build(bytes);
    let cursor = index.root(bytes);
    let expr = parse(FILTER).expect("parse failed");

    let result: QueryResult<Vec<u64>> = eval::<Vec<u64>, JqSemantics>(&expr, cursor);
    assert!(!result.is_error(), "unexpected error: {result:?}");
}
