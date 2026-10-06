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

/// `QueryResult` of `filter` over `json`, through the public entry on a
/// thread of an explicit stack size.
///
/// The native path walkers need ~1.4 MiB at 383 levels on a debug build
/// (`MAX_PATH_WALK_DEPTH`'s doc comment has the table), so a default 2 MiB
/// test thread leaves them only a ~1.4x margin. 8 MiB keeps these tests off
/// that edge on any platform's frame sizes: they pin the ceiling, not the
/// margin.
fn run_on_big_stack(json: String, filter: String) -> Result<Vec<String>, (bool, String)> {
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let bytes = json.as_bytes();
            let index = JsonIndex::build(bytes);
            let expr = parse(&filter).expect("parse failed");
            let result: QueryResult<Vec<u64>> =
                eval::<Vec<u64>, JqSemantics>(&expr, index.root(bytes));
            match result {
                QueryResult::Error(e) => Err((e.is_decode_failure(), e.to_string())),
                other => Ok(other
                    .collect_owned_checked::<JqSemantics>()
                    .expect("collects")
                    .iter()
                    .map(OwnedValue::to_json)
                    .collect()),
            }
        })
        .expect("spawns")
        .join()
        .expect("must not overflow the stack")
}

/// #3457/#3429: the path family reports a document nested past the walkers'
/// ceiling as an ordinary `QueryResult::Error`, through the public entry
/// with no `catch_unwind` net and not as a panic. The error is
/// decode-failure-tagged, so a `try` in the running filter cannot swallow it
/// (a plain error would let `try ... catch` turn a stack-safety guard into an
/// ordinary answer).
///
/// The native walkers (`paths`, `leaf_paths`, `.. | path`) stop at
/// `MAX_PATH_WALK_DEPTH` (384); `path(..)` materializes the document first and
/// stops at `MAX_NESTING_DEPTH` (256).
#[test]
fn test_public_eval_path_family_over_depth_is_a_clean_error_3457() {
    for (filter, depths, limit) in [
        ("[paths] | length", &[384, 1000][..], 384),
        ("[leaf_paths] | length", &[384, 1000][..], 384),
        ("[.. | path] | length", &[384, 1000][..], 384),
        (
            r#"try ([paths] | length) catch "swallowed""#,
            &[384, 1000][..],
            384,
        ),
        ("[path(..)] | length", &[256, 300, 384, 1000][..], 256),
    ] {
        for &depth in depths {
            let outcome = run_on_big_stack(nested_arrays(depth), filter.to_string());
            let (decode_failure, message) =
                outcome.expect_err(&format!("{filter} @ {depth}: expected an error"));
            assert!(decode_failure, "{filter} @ {depth}: {message}");
            assert_eq!(
                message,
                format!("nesting depth exceeds limit of {limit}"),
                "{filter} @ {depth}"
            );
        }
    }
}

/// Companion to the test above: just under the ceiling the same queries
/// answer -- for the native walkers that includes the 256-383 band the
/// materializers' ceiling used to refuse (#3429).
#[test]
fn test_public_eval_path_family_under_depth_answers_3457() {
    for depth in [200, 255, 256, 300, 383] {
        for (filter, expected) in [
            ("[paths] | length", depth),
            ("[leaf_paths] | length", 1),
            ("[.. | path] | length", depth + 1),
        ] {
            assert_eq!(
                run_on_big_stack(nested_arrays(depth), filter.to_string()),
                Ok(vec![expected.to_string()]),
                "{filter} @ {depth}"
            );
        }
    }
    // The materializing form is still bounded by the lower ceiling.
    assert_eq!(
        run_on_big_stack(nested_arrays(255), "[path(..)] | length".to_string()),
        Ok(vec!["256".to_string()])
    );
}

/// #3429: a static chain (`path(.k.k...k)`) is bounded by the same ceiling:
/// the guard runs before each stage's step, so 384 components are the most it
/// takes and the 385th refuses as a decode failure -- not a native stack
/// overflow.
#[test]
fn test_public_eval_static_path_chain_ceiling_3429() {
    let doc = format!("{}{{}}{}", "{\"k\":".repeat(400), "}".repeat(400));
    let chain = |n: usize| format!("path({}) | length", ".k".repeat(n));
    assert_eq!(
        run_on_big_stack(doc.clone(), chain(384)),
        Ok(vec!["384".to_string()])
    );
    let (decode_failure, message) = run_on_big_stack(doc, chain(385)).expect_err("refuses");
    assert!(decode_failure, "{message}");
    assert_eq!(message, "nesting depth exceeds limit of 384");
}

/// #3457: `..`/`recurse` over a document far deeper than any native stack
/// could hold a recursion of runs on an ordinary small thread and answers,
/// as it does in the CLI. The walker was a native recursion per level, so the
/// library entry -- which, unlike the CLI, has no large evaluation thread --
/// aborted the process with a stack overflow at a few thousand levels.
#[test]
fn test_public_eval_recurse_over_very_deep_document_does_not_overflow_the_stack_3457() {
    const DEPTH: usize = 100_000;
    // A 256 KiB stack holds a few hundred recursive frames of the old walker,
    // not 100,000 of anything.
    let outcome = std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            let json = nested_arrays(DEPTH);
            let bytes = json.as_bytes();
            let index = JsonIndex::build(bytes);
            [
                "[..] | length",
                "[recurse] | length",
                "last(..)",
                "first(..) | type",
            ]
            .map(|filter| {
                let expr = parse(filter).expect("parse failed");
                let result: QueryResult<Vec<u64>> =
                    eval::<Vec<u64>, JqSemantics>(&expr, index.root(bytes));
                result
                    .collect_owned_checked::<JqSemantics>()
                    .map(|vs| vs.iter().map(OwnedValue::to_json).collect::<Vec<_>>())
                    .map_err(|e| e.to_string())
            })
        })
        .expect("spawns")
        .join()
        .expect("must not overflow the stack");
    assert_eq!(outcome[0], Ok(vec![(DEPTH + 1).to_string()]));
    assert_eq!(outcome[1], Ok(vec![(DEPTH + 1).to_string()]));
    assert_eq!(outcome[2], Ok(vec!["1".to_string()]));
    assert_eq!(outcome[3], Ok(vec![r#""array""#.to_string()]));
}

/// #3457: which `QueryResult` variant carries an answer, as `eval`'s docs
/// state it -- bare `.` is the cursor, any other filter that yields a value
/// the document holds is `One` (even the input itself), and a computed value
/// is `Owned`.
#[test]
fn test_public_eval_result_variant_contract_3457() {
    let json = br#"{"a":1}"#;
    let index = JsonIndex::build(json);
    let run = |filter: &str| -> QueryResult<Vec<u64>> {
        eval::<Vec<u64>, JqSemantics>(&parse(filter).expect("parse failed"), index.root(json))
    };
    assert!(matches!(run("."), QueryResult::OneCursor(_)));
    assert!(matches!(run(". | ."), QueryResult::One(_)));
    assert!(matches!(run("first(.)"), QueryResult::One(_)));
    assert!(matches!(run(".a"), QueryResult::One(_)));
    assert!(matches!(run(".a + 1"), QueryResult::Owned(_)));
    assert!(matches!(
        run(".missing"),
        QueryResult::Owned(OwnedValue::Null)
    ));
    assert!(matches!(run(".[] | select(. > 5)"), QueryResult::None));
}
