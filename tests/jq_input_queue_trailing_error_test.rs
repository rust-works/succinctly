//! #2961: the `input`/`inputs` queue can end in a parse error after its
//! documents, delivered exactly once and followed by an exhausted stream.
//!
//! Drives the public seam directly (`seed_remaining_inputs_with_error`,
//! `pop_input`, `pop_remaining_input`, `current_input_location`), which the CLI
//! reaches only through `jq_runner.rs`. Kept to its own test binary because
//! seeding is thread-local and stays active for the rest of the thread's life
//! (see `jq_eval_using_input_queue_gate_test.rs`).

#![cfg(feature = "std")]

use std::cell::RefCell;
use std::rc::Rc;

use succinctly::jq::{
    count_reads_past_end, current_input_location, pop_input, pop_remaining_input,
    seed_remaining_inputs_from, seed_remaining_inputs_with_error,
    seed_remaining_inputs_with_errors, set_input_read_hook, EvalError, InputPop, OwnedValue,
    ProducedInput,
};

#[test]
fn a_trailing_parse_error_is_delivered_once_after_the_documents_2961() {
    seed_remaining_inputs_with_error(
        vec![(OwnedValue::Int(1), 0, 0), (OwnedValue::Int(2), 0, 0)],
        Some((0, 0)),
        Some((EvalError::new("Invalid JSON text"), (0, 1))),
    );
    count_reads_past_end();

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
    // The marker names where the parser stopped. The reads after it go on to
    // the end of the stream (#4303): the first names the seeded end of input,
    // and every later one is jq's `<unknown>`. jq 1.7.1 on `1\n2 }\n\n\n`
    // names line 2 for the error's read, line 4 for the next, then
    // `<unknown>`.
    assert_eq!(current_input_location(), Some((0, 1)));
    assert!(matches!(pop_input(), InputPop::Exhausted));
    assert_eq!(current_input_location(), Some((0, 0)));
    assert!(matches!(pop_input(), InputPop::Exhausted));
    assert_eq!(current_input_location(), None);

    // Without the count (a route that did not record the stream's end), every
    // read past the end leaves the seeded end of input.
    seed_remaining_inputs_with_error(
        vec![],
        Some((0, 7)),
        Some((EvalError::new("Invalid JSON text"), (0, 1))),
    );
    assert!(matches!(pop_input(), InputPop::ParseError(_)));
    assert!(matches!(pop_input(), InputPop::Exhausted));
    assert!(matches!(pop_input(), InputPop::Exhausted));
    assert_eq!(current_input_location(), Some((0, 7)));

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

/// #4311: parse errors among the documents are each delivered once, by the
/// read after the documents before them, and the documents jq read after
/// resuming follow; two errors may come back to back, and one may end the
/// stream.
#[test]
fn parse_errors_among_the_documents_are_delivered_in_order_4311() {
    let error = |message: &str| EvalError::new(message);
    seed_remaining_inputs_with_errors(
        vec![
            (OwnedValue::Int(1), 0, 1),
            (OwnedValue::Int(2), 0, 4),
            (OwnedValue::Int(3), 0, 5),
        ],
        Some((0, 6)),
        vec![
            (1, error("first"), (0, 2)),
            (1, error("second"), (0, 3)),
            (3, error("last"), (0, 6)),
        ],
    );
    let mut reads = Vec::new();
    for _ in 0..7 {
        reads.push(match pop_input() {
            InputPop::Document(OwnedValue::Int(n)) => format!("{n}@{:?}", current_input_location()),
            InputPop::Document(_) => unreachable!("only integers are seeded"), // patchcov: coverage tolerate-line reason="unreachable in a passing suite: reports a failed test invariant (#3673)"
            InputPop::ParseError(e) => format!("{}@{:?}", e.message, current_input_location()),
            InputPop::Exhausted => "end".to_string(),
        });
    }
    assert_eq!(
        reads,
        [
            "1@Some((0, 1))",
            "first@Some((0, 2))",
            "second@Some((0, 3))",
            "2@Some((0, 4))",
            "3@Some((0, 5))",
            "last@Some((0, 6))",
            "end",
        ]
    );
}

/// #6475: every read tells the hook where it reached -- the source of the document or parse error it
/// returns, and `None` once it finds the stream at its end -- before it returns, so a driver can say
/// what a file it could not open stood in the way of at the read that gets there.
#[test]
fn a_read_tells_the_hook_which_source_it_reached_6475() {
    let seen = Rc::new(RefCell::new(Vec::new()));
    seed_remaining_inputs_with_errors(
        vec![(OwnedValue::Int(1), 0, 1), (OwnedValue::Int(2), 1, 2)],
        Some((1, 3)),
        vec![(1, EvalError::new("e"), (0, 9))],
    );
    let sink = Rc::clone(&seen);
    set_input_read_hook(move |source| sink.borrow_mut().push(source));
    for _ in 0..5 {
        let _ = pop_input();
    }
    // The document, the parse error after it, the document of the next source, then the end, which
    // every read past it reports again.
    assert_eq!(*seen.borrow(), [Some(0), Some(0), Some(1), None, None]);
}

/// The same for a stream the CLI produces as it is read, and a seed clears the hook.
#[test]
fn a_producer_read_tells_the_hook_and_a_seed_clears_it_6475() {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut left = vec![(OwnedValue::Int(1), 0u32, 1u32), (OwnedValue::Int(2), 2, 2)];
    left.reverse();
    seed_remaining_inputs_from(Box::new(move || match left.pop() {
        Some((value, source, line)) => ProducedInput::Document(value, source, line),
        None => ProducedInput::End(Some((2, 3))),
    }));
    let sink = Rc::clone(&seen);
    set_input_read_hook(move |source| sink.borrow_mut().push(source));
    for _ in 0..3 {
        let _ = pop_input();
    }
    assert_eq!(*seen.borrow(), [Some(0), Some(2), None]);

    seed_remaining_inputs_with_errors(vec![(OwnedValue::Int(3), 0, 1)], None, Vec::new());
    let _ = pop_input();
    assert_eq!(seen.borrow().len(), 3, "the seed cleared the hook");
}
