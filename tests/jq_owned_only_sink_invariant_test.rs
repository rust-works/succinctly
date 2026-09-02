//! Drives the three sinks whose protocol `Item::into_owned_from_owned_producer`
//! documents as owned-only (#2025), so that method's `debug_assert!` actually
//! runs against each of them in a debug test build.
//!
//! The subject of these tests is the assertion, not the outputs -- every output
//! asserted here is already covered elsewhere. What is *not* covered elsewhere
//! is the claim the method's own doc comment makes: that `binary_fanout_each`,
//! `each_paths_filter` and `each_inputs` push nothing but `Item::Owned`. That
//! claim was re-derived by hand when #2025 was closed, and nothing but review
//! stops a later edit from pushing a borrowed item into one of these sinks --
//! at which point an undecodable string would silently materialize as `""`
//! (#1746's bug shape) instead of raising. With this file in place, that edit
//! fails here first.
//!
//! Driven through the library's own `eval` entry point rather than the CLI:
//! `binary_fanout_core` is `eval.rs`'s eager collecting wrapper, and reaching
//! it from filter text alone depends on which arms `eval_generic` currently
//! handles natively -- a detail that has moved before (#607, #1607) and is not
//! what these tests are pinning.
//!
//! Kept to its own test binary for `seed_remaining_inputs`'s sake: seeding is
//! thread-local and, once seeded, `input_queue_is_active()` stays true for the
//! rest of that thread's life, so it must not leak into another file's shared
//! process (same reasoning as `jq_eval_using_input_queue_gate_test.rs`).

#![cfg(feature = "std")]

use succinctly::jq::{eval, parse, seed_remaining_inputs, JqSemantics, OwnedValue};
use succinctly::json::JsonIndex;

fn eval_to_json(json: &[u8], filter: &str) -> Vec<String> {
    let index = JsonIndex::build(json);
    let cursor = index.root(json);
    let expr = parse(filter).expect("parse failed");
    eval::<_, JqSemantics>(&expr, cursor)
        .collect_owned()
        .iter()
        .map(OwnedValue::to_json)
        .collect()
}

/// `binary_fanout_core`'s sink, fed by `binary_fanout_each`'s single
/// `sink(Item::Owned(v))` -- `combine`'s own result. Two-by-two so the sink is
/// pushed to more than once, and jq's right-outer/left-inner order is visible.
#[test]
fn test_binary_fanout_core_sink_carries_only_owned_items_2025() {
    assert_eq!(
        eval_to_json(br"null", "(1,2) + (10,20)"),
        vec!["11", "12", "21", "22"],
    );
}

/// `builtin_paths_filter`'s sink, fed by `each_paths_filter`'s single
/// `sink(Item::Owned(path.clone()))`. A nested match so the pushed path array
/// has more than one component.
#[test]
fn test_builtin_paths_filter_sink_carries_only_owned_items_2025() {
    assert_eq!(
        eval_to_json(
            br#"{"a": 1, "b": {"c": 2}, "d": "x"}"#,
            r#"paths(type == "number")"#
        ),
        vec![r#"["a"]"#, r#"["b","c"]"#],
    );
}

/// `builtin_inputs`'s sink, fed by `each_inputs`'s single
/// `sink(Item::Owned(doc))`. Two queued documents so the sink is pushed to more
/// than once; `[inputs]` (not `inputs`) to reach the eager collecting face.
#[test]
fn test_builtin_inputs_sink_carries_only_owned_items_2025() {
    seed_remaining_inputs(
        vec![(OwnedValue::Int(42), 0, 1), (OwnedValue::Int(43), 0, 2)],
        None,
    );

    assert_eq!(eval_to_json(br#"{"a":1}"#, "[inputs]"), vec!["[42,43]"]);
}
