//! #2627: a library embedder calling `succinctly::jq::eval` directly (no
//! CLI, no `catch_unwind`) must never see a raw panic on a >256-deep
//! document, even for a filter whose own operand doesn't need path context.
//!
//! `eval::eval`'s own top-level gate (`needs_path_context`) routes a whole
//! expression through `eval_generic` only when *some* part of it needs path
//! context (`key`/`parent`/`path`/...). A bare `. and true`/`. + 1` on its
//! own therefore never reaches `eval_generic`'s internal materialization
//! through the public API -- but a pipe that combines a path-context need
//! with an otherwise-ungated sub-expression does, and once inside
//! `eval_generic`, that sub-expression's own `needs_path_context` gate can
//! independently be `false`, sending it to the same wildcard/ambient bridge
//! `tests/jq_cli_tests.rs`'s `test_boolean_wildcard_bridge_over_depth_
//! document_reports_cleanly_not_panic_2627` pins at the CLI layer (protected
//! there by `catch_unwind`, #1793).
//!
//! #2627's fix made this a clean `Err` rather than a panic; #2692 then
//! removed the ambient-validation walk `and`/`or` routed through entirely
//! (see `src/jq/eval_generic.rs`'s `eval_boolean_generic` and
//! `docs/adrs/adr-0018.md`), so `. and true` now reads only `.`'s
//! truthiness -- `is_falsy`, which never descends into a container's
//! children -- and answers `true` regardless of nesting depth. This file
//! confirms that corollary through the *public* library entry point (no
//! `catch_unwind` net, unlike the CLI layer), mirroring
//! `test_boolean_and_reads_truthiness_without_tripping_depth_guard_2692`
//! (`src/jq/eval_generic.rs`) and the CLI-level
//! `test_truthiness_probes_do_not_trip_the_depth_guard_2692`
//! (`tests/jq_cli_tests.rs`).
//!
//! `(. and true)`, not `(true and true)`: a fully closed sub-expression (no
//! operand reads `.` at all) no longer reaches `eval_boolean_generic`'s
//! document argument in the first place -- #2173's
//! `walk::reads_ambient_value` gate (landed the same day as #2627's fix)
//! substitutes `null` for a closed term instead of the real document. `.`
//! keeps the sub-expression's own `needs_path_context` false (still no
//! `key`/`parent`/`path` operand) while making it read the ambient value,
//! which is what keeps the real (over-deep) document in the loop and lets
//! this test exercise the depth guard's *absence* rather than a
//! substituted `Null`.
//!
//! Confirmed live before #2627's fix that this exact shape panicked with no
//! `catch_unwind` to save it (the underlying `assert_nesting_depth` `panic!`
//! in `src/jq/eval_generic.rs`).

use succinctly::jq::{eval, parse, JqSemantics, OwnedValue, QueryResult};
use succinctly::json::JsonIndex;

/// `[[[...1...]]]`, `depth` levels of array nesting wrapping a single `1` --
/// mirrors `jq_recurse_depth_tests.rs`'s own `linear_nest` and `jq_cli_tests
/// .rs`'s `nested_arrays`.
fn nested_arrays(depth: usize) -> String {
    format!("{}1{}", "[".repeat(depth), "]".repeat(depth))
}

/// `key?` forces `eval::eval`'s own gate to route the whole pipe through
/// `eval_generic` (confirmed by `needs_path_context`'s `Expr::Builtin(Builtin
/// ::Key) => true` arm); `(. and true)` has no path-context operand of its
/// own, so once inside `eval_generic` it independently falls to
/// `eval_boolean_generic`, while still reading the ambient value (see the
/// module doc comment above). `key` on a root array (not an object) errors,
/// so `?` swallows that branch's output entirely, leaving `(. and true)`'s
/// `true` as the comma expression's only result.
const FILTER: &str = "key?, (. and true)";

#[test]
fn test_public_eval_over_depth_document_reads_truthiness_without_tripping_depth_guard_2692() {
    let json = nested_arrays(300);
    let bytes = json.as_bytes();
    let index = JsonIndex::build(bytes);
    let cursor = index.root(bytes);
    let expr = parse(FILTER).expect("parse failed");

    let result: QueryResult<Vec<u64>> = eval::<Vec<u64>, JqSemantics>(&expr, cursor);
    match result {
        QueryResult::Owned(OwnedValue::Bool(true)) => {}
        other => panic!("expected Owned(true), got: {other:?}"),
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
