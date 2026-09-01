//! #1504's `input_queue_is_active` + `uses_input_builtins` gate was added to
//! *both* `eval_generic::eval_using` and `eval_generic::eval_with_cursor_using`,
//! but `succinctly jq`/`succinctly yq` only ever call the cursor-preserving
//! half (`eval_with_cursor_using`, via `jq_runner.rs`/`yq_runner.rs`) -- so no
//! CLI-driven integration test can reach `eval_using`'s own copy of the gate.
//! It is reachable only through the library's non-cursor `eval`/`eval_using`
//! entry point, which is what this file drives directly.
//!
//! Seeding the input queue (`seed_remaining_inputs`) is thread-local and,
//! once seeded, `input_queue_is_active()` stays true for the rest of that
//! thread's life -- kept to its own test binary (this file compiles to a
//! separate process) so it can't leak into any other test file's shared
//! process or test-thread pool.
//!
//! `seed_remaining_inputs` itself is `#[cfg(feature = "std")]` (`src/jq/mod.rs`),
//! so this whole file compiles to nothing under `--no-default-features` (#2083).

#![cfg(feature = "std")]

use succinctly::jq::eval_generic::eval_using;
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
        .collect_owned()
        .unwrap()
        .iter()
        .map(OwnedValue::to_json)
        .collect();

    assert_eq!(outputs, vec![r#"{"a":1}"#.to_string(), "42".to_string()]);
}
