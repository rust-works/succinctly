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
    result.collect_owned().iter().map(|v| v.to_json()).collect()
}

/// Outputs of the generic evaluator (`src/jq/eval_generic.rs`, the CLI path).
fn generic_outputs(json: &[u8], filter: &str) -> Vec<String> {
    let index = JsonIndex::build(json);
    let cursor = index.root(json);
    let expr = parse(filter).expect("parse failed");
    let result = eval_generic::eval_with_cursor(&expr, cursor);
    result.collect_owned().iter().map(|v| v.to_json()).collect()
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
    assert_parity(br#"[1,null,2,null,3]"#, "[.[] | values]");
    assert_parity(br#"{"a":1,"b":null,"c":3}"#, "[.[] | values]");
}

#[test]
fn test_parity_first_last() {
    assert_parity(br#"[10,20,30]"#, "first(.[])");
    assert_parity(br#"[10,20,30]"#, "last(.[])");
    assert_parity(br#"[10,20,30]"#, "first");
    assert_parity(br#"[10,20,30]"#, "last");
}

#[test]
fn test_parity_first_last_empty() {
    assert_parity(br#"[]"#, "first(.[])");
    assert_parity(br#"[]"#, "last(.[])");
}

#[test]
fn test_object_ordering_diverges_157() {
    // jq compares objects by [sorted keys], then by [values in key order].
    // Every case below is `true` in jq, which the FULL evaluator matches; the
    // generic (CLI) path always returns `false` -- bug #157.
    for filter in [
        r#"{"a":1} < {"a":2}"#,
        r#"{"a":2} > {"a":1}"#,
        r#"{"a":1} < {"b":1}"#,
        r#"{"a":1,"b":2} < {"a":1,"b":3}"#,
    ] {
        assert_divergence(b"null", filter, &["true"], &["false"]);
    }
}

#[test]
fn test_out_of_bounds_index_diverges_161_162() {
    // jq: indexing an array out of bounds (positive or negative) yields `null`.
    // The FULL evaluator matches; the generic (CLI) path yields NO output --
    // bug #161/#162.
    for filter in [".[5]", ".[-5]", ".[100]"] {
        assert_divergence(br#"[1,2,3]"#, filter, &["null"], &[]);
    }
}
