//! #2961: the `input`/`inputs` queue can end in a parse error after its
//! documents, delivered exactly once and followed by an exhausted stream.
//!
//! Drives the public seam directly (`seed_remaining_inputs_with_error`,
//! `pop_input`, `pop_remaining_input`, `current_input_location`), which the CLI
//! reaches only through `jq_runner.rs`. Kept to its own test binary because
//! seeding is thread-local and stays active for the rest of the thread's life
//! (see `jq_eval_using_input_queue_gate_test.rs`).

#![cfg(feature = "std")]

use succinctly::jq::{
    current_input_location, pop_input, pop_remaining_input, seed_remaining_inputs_with_error,
    EvalError, InputPop, OwnedValue,
};

#[test]
fn a_trailing_parse_error_is_delivered_once_after_the_documents_2961() {
    seed_remaining_inputs_with_error(
        vec![(OwnedValue::Int(1), 0, 0), (OwnedValue::Int(2), 0, 0)],
        Some((0, 0)),
        Some((EvalError::new("Invalid JSON text"), (0, 1))),
    );

    assert!(matches!(
        pop_input(),
        InputPop::Document(OwnedValue::Int(1))
    ));
    assert!(matches!(
        pop_input(),
        InputPop::Document(OwnedValue::Int(2))
    ));
    match pop_input() {
        InputPop::ParseError(e) => assert_eq!(e.message, "Invalid JSON text"),
        _ => panic!("the parse error follows the documents"),
    }
    // The marker names where the parser stopped, and stays there.
    assert_eq!(current_input_location(), Some((0, 1)));
    assert!(matches!(pop_input(), InputPop::Exhausted));
    assert!(matches!(pop_input(), InputPop::Exhausted));
    assert_eq!(current_input_location(), Some((0, 1)));

    // The `Option`-shaped pop reads a trailing error as the end of the
    // stream, and consumes it.
    seed_remaining_inputs_with_error(
        vec![(OwnedValue::Int(3), 0, 0)],
        None,
        Some((EvalError::new("Invalid JSON text"), (0, 0))),
    );
    assert_eq!(pop_remaining_input(), Some(OwnedValue::Int(3)));
    assert_eq!(pop_remaining_input(), None);
    assert!(matches!(pop_input(), InputPop::Exhausted));
}
