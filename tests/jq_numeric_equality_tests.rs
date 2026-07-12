//! Characterization tests for cross-type numeric equality in the full jq
//! evaluator (`src/jq/eval.rs`).
//!
//! `==`/`!=` are implemented with `OwnedValue`'s *derived* `PartialEq`, which
//! distinguishes the `Int` and `Float` representations — so `1 == 1.0` is
//! currently `false`, diverging from jq (which compares numbers by value).
//! Several builtins route through `==`-style comparison (`contains`, `index`,
//! array `-`, ...) and inherit the divergence, while `unique` uses an
//! ordering-based comparison and already agrees with jq. Worse, the array `-`
//! builtin diverges between the two evaluators too (the generic/CLI path treats
//! 1 and 1.0 as equal, the full evaluator does not). That inconsistency is
//! exactly the hazard #156 tracks.
//!
//! These tests lock in the CURRENT behavior and document jq's answer inline.
//! When #156 lands, the `*_diverges_156` assertions must be flipped to jq's
//! semantics — they will fail loudly at that point, which is the point.

use succinctly::jq::{eval, parse, JqSemantics, QueryResult};
use succinctly::json::JsonIndex;

/// Render every output of the full evaluator as a compact JSON string.
fn full_outputs(json: &[u8], filter: &str) -> Vec<String> {
    let index = JsonIndex::build(json);
    let cursor = index.root(json);
    let expr = parse(filter).expect("parse failed");
    let result: QueryResult<Vec<u64>> = eval::<Vec<u64>, JqSemantics>(&expr, cursor);
    result.collect_owned().iter().map(|v| v.to_json()).collect()
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
fn test_eq_int_vs_float_diverges_156() {
    // jq: `1 == 1.0` is `true`; succinctly currently yields `false`.
    assert_eq!(one(b"1", "1 == 1.0"), "false");
    // jq: `1 != 1.0` is `false`; succinctly currently yields `true`.
    assert_eq!(one(b"1", "1 != 1.0"), "true");
}

#[test]
fn test_unique_dedups_int_and_float() {
    // `unique` sorts with the ordering-based comparison, which treats 1 and
    // 1.0 as equal -> a single element. This already matches jq.
    assert_eq!(one(br#"[1,1.0]"#, "unique"), "[1]");
}

#[test]
fn test_difference_int_float_diverges_156() {
    // jq: `[1.0,2,3] - [1]` is `[2,3]` (numeric equality removes 1.0).
    // The FULL evaluator compares Int(1) != Float(1.0), so 1.0 is NOT removed.
    // This ALSO diverges from the generic/CLI path (which yields `[2,3]`) — see
    // the evaluator-parity tests, which pin that drift explicitly.
    assert_eq!(one(br#"[1.0,2,3]"#, ". - [1]"), "[1,2,3]");
}

#[test]
fn test_contains_int_float_diverges_156() {
    // jq: `[1,2,3] | contains([1.0])` is `true`; succinctly currently `false`.
    assert_eq!(one(br#"[1,2,3]"#, "contains([1.0])"), "false");
}

#[test]
fn test_index_int_float_diverges_156() {
    // jq: `[2,1,3] | index(1.0)` is `1`; succinctly currently `null`.
    assert_eq!(one(br#"[2,1,3]"#, "index(1.0)"), "null");
}
