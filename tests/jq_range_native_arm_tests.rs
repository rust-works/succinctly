//! #2698: `each_range_generic`'s two branches the parser cannot reach.
//!
//! The parser desugars `range(n)` to `range(0; n)` (every `Expr::Range` it
//! builds carries `to: Some(..)`), so the `to == None` arm is unreachable
//! from any query text. But `Expr` is public, so a library caller can build
//! one directly -- which is what these tests do, rather than marking the arm
//! tolerated and leaving its behaviour unasserted.

use succinctly::jq::{eval_generic, Expr, Literal};
use succinctly::json::JsonIndex;

fn run(expr: &Expr, doc: &str) -> Vec<String> {
    let bytes = doc.as_bytes();
    let index = JsonIndex::build(bytes);
    let cursor = index.root(bytes);
    eval_generic::eval_with_cursor(expr, cursor)
        .collect_owned()
        .expect("evaluates")
        .iter()
        .map(succinctly::jq::OwnedValue::to_json)
        .collect()
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
