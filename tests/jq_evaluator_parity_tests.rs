//! Evaluator-parity tests: the CLI uses the generic evaluator
//! (`src/jq/eval_generic.rs`) while the library's `jq::eval` entry point uses
//! the full evaluator (`src/jq/eval.rs`). For builtins implemented in both,
//! the two must agree; where they don't, that drift is a bug (#157/#161/#162).
//!
//! Each case renders both evaluators' outputs to JSON and compares them. Cases
//! that currently AGREE are asserted equal (locking them in). Cases that
//! currently DIVERGE are pinned with `assert_ne!` plus the observed outputs, so
//! the fix is forced to update them and no NEW drift slips in silently.

use succinctly::jq::eval_generic;
use succinctly::jq::{eval, parse, JqSemantics, QueryResult};
use succinctly::json::JsonIndex;

/// Outputs of the full evaluator (`src/jq/eval.rs`).
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

/// Outputs of the generic evaluator (`src/jq/eval_generic.rs`, the CLI path).
fn generic_outputs(json: &[u8], filter: &str) -> Vec<String> {
    let index = JsonIndex::build(json);
    let cursor = index.root(json);
    let expr = parse(filter).expect("parse failed");
    let result = eval_generic::eval_with_cursor(&expr, cursor);
    result
        .collect_owned()
        .iter()
        .map(succinctly::jq::OwnedValue::to_json)
        .collect()
}

fn as_strs(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}

/// Assert both evaluators produce identical output for `filter` on `json`.
fn assert_parity(json: &[u8], filter: &str) {
    let full = full_outputs(json, filter);
    let generic = generic_outputs(json, filter);
    assert_eq!(
        full,
        generic,
        "evaluator drift for `{filter}` on `{}`:\n  full   = {full:?}\n  generic= {generic:?}",
        String::from_utf8_lossy(json)
    );
}

/// Assert the two evaluators currently DISAGREE, pinning both observed outputs.
/// When the referenced fix aligns them, the `assert_ne!` fails, forcing whoever
/// lands the fix to convert this into `assert_parity`.
fn assert_divergence(json: &[u8], filter: &str, full_expected: &[&str], generic_expected: &[&str]) {
    let full = full_outputs(json, filter);
    let generic = generic_outputs(json, filter);
    assert_eq!(
        as_strs(&full),
        full_expected,
        "full evaluator output changed for `{filter}`"
    );
    assert_eq!(
        as_strs(&generic),
        generic_expected,
        "generic evaluator output changed for `{filter}`"
    );
    assert_ne!(
        full, generic,
        "evaluators now AGREE for `{filter}` -- convert to assert_parity"
    );
}

#[test]
fn test_parity_values_builtin() {
    // `values` drops null inputs.
    assert_parity(br"[1,null,2,null,3]", "[.[] | values]");
    assert_parity(br#"{"a":1,"b":null,"c":3}"#, "[.[] | values]");
}

#[test]
fn test_parity_first_last() {
    assert_parity(br"[10,20,30]", "first(.[])");
    assert_parity(br"[10,20,30]", "last(.[])");
    assert_parity(br"[10,20,30]", "first");
    assert_parity(br"[10,20,30]", "last");
}

#[test]
fn test_parity_first_last_empty() {
    assert_parity(br"[]", "first(.[])");
    assert_parity(br"[]", "last(.[])");
}

#[test]
fn test_parity_values_bare_is_identity_on_non_null() {
    // jq: `values` == `select(. != null)` -- identity on any non-null input,
    // including scalars and whole containers; null yields no output (#161).
    assert_parity(b"1", "values");
    assert_parity(br#""abc""#, "values");
    assert_parity(b"true", "values");
    assert_parity(br#"{"a":1,"b":null}"#, "values");
    assert_parity(br"[1,null,2]", "values");
    assert_parity(b"null", "values");
}

#[test]
fn test_parity_first_last_bare_on_empty_and_null() {
    // jq: `first` == `.[0]` and `last` == `.[-1]`, so `[]` and `null` inputs
    // yield null rather than erroring (#161).
    assert_parity(br"[]", "first");
    assert_parity(br"[]", "last");
    assert_parity(b"null", "first");
    assert_parity(b"null", "last");
}

#[test]
fn test_parity_length_of_i64_min() {
    // -2^63 has no i64 absolute value; both evaluators must agree on the
    // f64 fallback instead of panicking in debug builds (#161).
    assert_parity(b"-9223372036854775808", "length");
}

#[test]
fn test_object_ordering_parity_162() {
    // jq compares objects by [sorted keys] first, then by [values in key
    // order]. Fixed by #162 in BOTH evaluators (eval_generic was missing the
    // Object arm; eval.rs interleaved key and value comparison). Every
    // expected value below is pinned against real jq, so the parity assertion
    // can't lock in an agreed-upon wrong answer.
    for (filter, expected) in [
        (r#"{"a":1} < {"a":2}"#, "true"),
        (r#"{"a":2} > {"a":1}"#, "true"),
        (r#"{"a":1} < {"b":1}"#, "true"),
        (r#"{"a":1,"b":2} < {"a":1,"b":3}"#, "true"),
        // Key arrays decide before any values: ["a","b"] < ["a","c"] even
        // though the value at the shared key "a" compares Greater.
        (r#"{"a":2,"b":1} < {"a":1,"c":9}"#, "true"),
        // Insertion order is irrelevant; these objects are equal.
        (r#"{"b":1,"a":2} <= {"a":2,"b":1}"#, "true"),
        (r#"{"a":1} >= {"a":1}"#, "true"),
        // A key array that is a strict prefix compares Less.
        (r#"{"a":1} < {"a":1,"b":2}"#, "true"),
        (r#"{"a":1,"b":2} < {"a":1}"#, "false"),
    ] {
        let full = full_outputs(b"null", filter);
        assert_eq!(
            as_strs(&full),
            [expected],
            "full evaluator disagrees with jq for `{filter}`"
        );
        assert_parity(b"null", filter);
    }
}

#[test]
fn test_out_of_bounds_index_diverges_161_162() {
    // jq: indexing an array out of bounds (positive or negative) yields `null`.
    // The FULL evaluator matches; the generic (CLI) path yields NO output --
    // bug #161/#162.
    for filter in [".[5]", ".[-5]", ".[100]"] {
        assert_divergence(br"[1,2,3]", filter, &["null"], &[]);
    }
}
