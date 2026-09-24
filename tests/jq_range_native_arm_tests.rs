//! #2698: `each_range_generic`'s two branches the parser cannot reach.
//!
//! The parser desugars `range(n)` to `range(0; n)` (every `Expr::Range` it
//! builds carries `to: Some(..)`), so the `to == None` arm is unreachable
//! from any query text. But `Expr` is public, so a library caller can build
//! one directly -- which is what these tests do, rather than marking the arm
//! tolerated and leaving its behaviour unasserted.

use succinctly::jq::{eval_generic, Expr, JqSemantics, Literal};
use succinctly::json::JsonIndex;

fn run(expr: &Expr, doc: &str) -> Vec<String> {
    let bytes = doc.as_bytes();
    let index = JsonIndex::build(bytes);
    let cursor = index.root(bytes);
    eval_generic::eval_with_cursor(expr, cursor)
        .collect_owned::<JqSemantics>()
        .expect("evaluates")
        .iter()
        .map(succinctly::jq::OwnedValue::to_json)
        .collect()
}

/// Same route as `run`, but from real filter text -- `each_range_generic`,
/// not `eval.rs`'s own `each_range`, is what `succinctly jq`/`succinctly yq`
/// actually call (both CLI runners import from `eval_generic`, not `eval`).
fn run_filter(filter: &str, doc: &str) -> Vec<String> {
    run(&succinctly::jq::parse(filter).unwrap(), doc)
}

fn range_to_none(from: Literal) -> Expr {
    Expr::Range {
        from: Box::new(Expr::Literal(from)),
        to: None,
        step: None,
    }
}

/// A hand-built `Range { to: None }` behaves as `range(0; n)`, exactly as
/// `eval.rs`'s own `each_range` treats that shape -- integer path.
#[test]
fn range_with_no_upper_bound_counts_from_zero_int_2698() {
    let expr = Expr::Array(Box::new(range_to_none(Literal::Int(3))));
    assert_eq!(run(&expr, "null"), vec!["[0,1,2]"]);
}

/// Same arm, float path: `range(2.5)` as a bare bound is `range(0.0; 2.5)`.
#[test]
fn range_with_no_upper_bound_counts_from_zero_float_2698() {
    let expr = Expr::Array(Box::new(range_to_none(Literal::Float(2.5))));
    assert_eq!(run(&expr, "null"), vec!["[0,1,2]"]);
}

/// #3102, via `each_range_generic`: `eval.rs`'s own unit tests
/// (`test_range_3_arg_nan_bound_full_matrix_3102`) cover this same fix
/// through `eval.rs`'s `each_range` (the `query!` macro's route), which
/// neither CLI runner actually calls -- `jq_runner.rs`/`yq_runner.rs` both
/// dispatch through `eval_generic`. Confirmed against `/usr/bin/jq` 1.7.1.
#[test]
fn range_3_arg_nan_bound_full_matrix_via_generic_evaluator_3102() {
    assert_eq!(
        run_filter("[limit(3; range(0; nan; -1))]", "null"),
        vec!["[0,-1,-2]"]
    );
    assert_eq!(
        run_filter("[limit(3; range(nan; -3; -1))]", "null"),
        vec!["[]"]
    );
    assert_eq!(
        run_filter("[limit(3; range(nan; 3; 1))]", "null"),
        vec!["[null,null,null]"]
    );
}

/// #3227, via `each_range_generic` -- same rationale as #3102's sibling test
/// above: a NaN *step* routes into the descending arm (confirmed against
/// `/usr/bin/jq` 1.7.1), not neither arm.
#[test]
fn range_3_arg_nan_step_matches_total_order_via_generic_evaluator_3227() {
    assert_eq!(
        run_filter("[limit(3; range(5; 0; nan))]", "null"),
        vec!["[5]"]
    );
    assert_eq!(
        run_filter("[limit(3; range(0; 5; nan))]", "null"),
        vec!["[]"]
    );
    assert_eq!(
        run_filter("[limit(3; range(0; -5; nan))]", "null"),
        vec!["[0]"]
    );
}
