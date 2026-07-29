//! Cross-type numeric equality in the full jq evaluator (`src/jq/eval.rs`).
//!
//! `OwnedValue` stores JSON numbers as two variants, `Int(i64)` and
//! `Float(f64)`. Equality used to come from `#[derive(PartialEq)]`, which
//! compares the *representation*, so `1 == 1.0` was `false` (jq: `true`) and
//! every builtin routed through equality — array `-`, `contains`, `inside`,
//! `index`, `indices`, `rindex` — inherited the divergence. Meanwhile `unique`
//! sorts with an ordering-based comparison and already agreed with jq, so the
//! two notions of "same number" contradicted each other inside one evaluator.
//!
//! #156 replaced the derive with a hand-written numeric-aware `PartialEq` in
//! `src/jq/value.rs`, which fixes all of those at once. Every expectation below
//! is pinned against jq-1.7.1 (the version in
//! `tests/data/jq-golden/JQ_VERSION`), so this suite cannot lock in a wrong
//! answer that merely happens to be self-consistent.

use succinctly::jq::{eval, parse, JqSemantics, QueryResult};
use succinctly::json::JsonIndex;

/// Render every output of the full evaluator as a compact JSON string.
fn full_outputs(json: &[u8], filter: &str) -> Vec<String> {
    let index = JsonIndex::build(json);
    let cursor = index.root(json);
    let expr = parse(filter).expect("parse failed");
    let result: QueryResult<Vec<u64>> = eval::<Vec<u64>, JqSemantics>(&expr, cursor);
    result
        .collect_owned()
        .iter()
        .map(succinctly::jq::OwnedValue::to_json)
        .collect()
}

/// Convenience: assert the filter produces exactly one output and return it.
fn one(json: &[u8], filter: &str) -> String {
    let outs = full_outputs(json, filter);
    assert_eq!(
        outs.len(),
        1,
        "expected exactly one output for `{filter}`, got {outs:?}"
    );
    outs.into_iter().next().unwrap()
}

#[test]
fn test_eq_same_representation_holds() {
    // Same representation compares equal in both jq and succinctly.
    assert_eq!(one(b"1", "1 == 1"), "true");
    assert_eq!(one(b"1", "1.0 == 1.0"), "true");
    assert_eq!(one(b"1", "2 == 1"), "false");
    assert_eq!(one(b"1", "1 != 2"), "true");
}

#[test]
fn test_eq_int_vs_float() {
    // jq: `1 == 1.0` is true, `1 != 1.0` is false (#156).
    assert_eq!(one(b"1", "1 == 1.0"), "true");
    assert_eq!(one(b"1", "1 != 1.0"), "false");
    // Only *equal* numbers are conflated -- this is not "any two numbers".
    assert_eq!(one(b"1", "1 == 1.5"), "false");
    assert_eq!(one(b"1", "1 != 1.5"), "true");
}

#[test]
fn test_eq_is_numeric_not_cross_type() {
    // jq conflates the two number representations and nothing else.
    assert_eq!(one(b"1", r#"1 == "1""#), "false");
    assert_eq!(one(b"1", "1 == true"), "false");
    assert_eq!(one(b"1", "0 == false"), "false");
    assert_eq!(one(b"1", "0 == null"), "false");
}

#[test]
fn test_eq_nan_is_never_equal() {
    // jq: `nan == nan` is false. Equality is therefore NOT
    // `compare_values(..) == Equal` -- `compare_values` orders NaN as
    // strictly `Less` than every number, including another NaN (#421).
    assert_eq!(one(b"1", "nan == nan"), "false");
    assert_eq!(one(b"1", "nan != nan"), "true");
    // Infinities, by contrast, do compare equal to themselves.
    assert_eq!(one(b"1", "infinite == infinite"), "true");
}

#[test]
fn test_eq_signed_zero() {
    // jq: `-0.0 == 0` is true.
    assert_eq!(one(b"1", "-0.0 == 0"), "true");
    assert_eq!(one(b"1", "-0.0 == 0.0"), "true");
}

#[test]
fn test_eq_recurses_through_containers() {
    // jq: `[1] == [1.0]` and `{"a":1} == {"a":1.0}` are both true, because
    // Vec/IndexMap inherit their equality from the element type.
    assert_eq!(one(b"1", "[1] == [1.0]"), "true");
    assert_eq!(one(b"1", r#"{"a":1} == {"a":1.0}"#), "true");
    assert_eq!(one(b"1", "[1] == [1.5]"), "false");
}

#[test]
fn test_unique_dedups_int_and_float() {
    // `unique` sorts with the ordering-based comparison, which treats 1 and
    // 1.0 as equal -> a single element. This always matched jq; it is asserted
    // here because it is the invariant `==` used to contradict.
    assert_eq!(one(br"[1,1.0]", "unique"), "[1]");
}

#[test]
fn test_difference_int_float() {
    // jq: `[1.0,2,3] - [1]` is `[2,3]` -- numeric equality removes the 1.0.
    // Array subtraction filters with `Vec::contains`, so it rides on the same
    // `PartialEq` as `==`.
    assert_eq!(one(br"[1.0,2,3]", ". - [1]"), "[2,3]");
    assert_eq!(one(br"[1,2,3]", ". - [1.0]"), "[2,3]");
}

#[test]
fn test_contains_int_float() {
    // jq: `[1,2,3] | contains([1.0])` is true.
    assert_eq!(one(br"[1,2,3]", "contains([1.0])"), "true");
    assert_eq!(one(br"[1.0,2,3]", "contains([1])"), "true");
    assert_eq!(one(br"[1,2,3]", "contains([1.5])"), "false");
}

#[test]
fn test_inside_int_float() {
    // `inside` is `contains` with the arguments swapped, and shares the impl.
    assert_eq!(one(br"[1.0]", "inside([1,2,3])"), "true");
}

#[test]
fn test_index_int_float() {
    // jq: `[2,1,3] | index(1.0)` is 1.
    assert_eq!(one(br"[2,1,3]", "index(1.0)"), "1");
    assert_eq!(one(br"[2,1.0,3]", "index(1)"), "1");
    assert_eq!(one(br"[2,1,3]", "index(1.5)"), "null");
}

#[test]
fn test_indices_and_rindex_int_float() {
    // `indices` and `rindex` walk the same equality.
    assert_eq!(one(br"[1,2,1.0]", "indices(1)"), "[0,2]");
    assert_eq!(one(br"[1,2,1.0]", "rindex(1)"), "2");
}
