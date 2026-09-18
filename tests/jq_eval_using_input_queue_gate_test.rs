//! #1504's `input_queue_is_active` + `uses_input_builtins` gate was added to
//! *both* `eval_generic::eval_using` and `eval_generic::eval_with_cursor_using`,
//! but `succinctly jq`/`succinctly yq`'s own routing (`jq_runner.rs`'s
//! `can_use_lazy_path`) excludes every program that uses `input`/`inputs`
//! from the lazy/generic path entirely, sending it to the eager evaluator
//! instead -- so no CLI-driven integration test can reach *either* copy of
//! the gate. Both are reachable only through the library's own generic
//! entry points, which is what this file drives directly.
//!
//! Seeding the input queue (`seed_remaining_inputs`) is thread-local and,
//! once seeded, `input_queue_is_active()` stays true for the rest of that
//! thread's life -- kept to its own test binary (this file compiles to a
//! separate process) so it can't leak into any other test file's shared
//! process or test-thread pool.

#![cfg(feature = "std")]

use succinctly::jq::eval_generic::{eval_using, eval_with_cursor_using};
use succinctly::jq::{parse, seed_remaining_inputs, JqSemantics, OwnedValue};
use succinctly::json::JsonIndex;

#[test]
fn test_eval_using_interleaves_input_with_top_level_comma_1504() {
    let json = br#"{"a":1}"#;
    let index = JsonIndex::build(json);
    let cursor = index.root(json);
    let value = cursor.value();

    seed_remaining_inputs(vec![(OwnedValue::Int(42), 0, 1)], None);

    let expr = parse("(., input)").expect("parse failed");
    let result = eval_using::<JqSemantics, _>(&expr, value);
    let outputs: Vec<String> = result
        .collect_owned::<JqSemantics>()
        .unwrap()
        .iter()
        .map(OwnedValue::to_json)
        .collect();

    assert_eq!(outputs, vec![r#"{"a":1}"#.to_string(), "42".to_string()]);
}

/// #3122: the cursor-preserving twin of the test above -- same gate, same
/// bridge shape (`Reentry::Against(RootWitness::of(Some(&cursor)))` rather
/// than `eval_using`'s cursor-less `Reentry::REBUILT`), reached only through
/// `eval_with_cursor_using` directly, for the same "the CLI never sends this
/// route an input/inputs program" reason the module doc above gives.
#[test]
fn test_eval_with_cursor_using_interleaves_input_with_top_level_comma_3122() {
    let json = br#"{"a":1}"#;
    let index = JsonIndex::build(json);

    seed_remaining_inputs(vec![(OwnedValue::Int(42), 0, 1)], None);

    let expr = parse("(., input)").expect("parse failed");
    let result = eval_with_cursor_using::<JqSemantics, _>(&expr, index.root(json));
    let outputs: Vec<String> = result
        .collect_owned::<JqSemantics>()
        .unwrap()
        .iter()
        .map(OwnedValue::to_json)
        .collect();

    assert_eq!(outputs, vec![r#"{"a":1}"#.to_string(), "42".to_string()]);
}
