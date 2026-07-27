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
fn test_numeric_equality_parity_156() {
    // `OwnedValue`'s equality is now numeric-aware (#156), so both evaluators
    // agree that 1 and 1.0 are the same number -- and agree for the same
    // reason. Before the fix the generic path already answered `[2,3]` for
    // `. - [1]`, but only by accident: `eval_on_owned` round-trips the value
    // through `to_json()` (eval_generic.rs), which renders `Float(1.0)` as `1`
    // and erased the distinction the full evaluator was still honouring.
    for (json, filter) in [
        (b"null".as_slice(), "1 == 1.0"),
        (b"null", "1 != 1.0"),
        (b"null", "1 == 1.5"),
        (b"null", "nan == nan"),
        (b"null", "[1] == [1.0]"),
        (b"null", r#"{"a":1} == {"a":1.0}"#),
        (br"[1.0,2,3]", ". - [1]"),
        (br"[1,2,3]", "contains([1.0])"),
        (br"[2,1,3]", "index(1.0)"),
        (br"[1,2,1.0]", "indices(1)"),
    ] {
        assert_parity(json, filter);
    }
    // Pin the shared answer against real jq, so parity can't agree on a wrong
    // one (the failure mode this file's header calls out).
    assert_eq!(as_strs(&full_outputs(b"null", "1 == 1.0")), ["true"]);
    assert_eq!(as_strs(&full_outputs(br"[1.0,2,3]", ". - [1]")), ["[2,3]"]);
    assert_eq!(as_strs(&full_outputs(b"null", "nan == nan")), ["false"]);
}

#[test]
fn test_stream_operator_parity_160() {
    // `//`, `and` and `or` are generators over their operands' streams, not
    // scalar operators over the first output of each (#160). The generic (CLI)
    // evaluator delegates all three back into the full evaluator, so the fix
    // has to land in both at once -- this pins that it did.
    //
    // Every expectation is pinned against real jq-1.7.1 first, so parity cannot
    // lock in an agreed-upon wrong answer (this file's header failure mode).
    for (filter, expected) in [
        (r#"(false,1,null,2) // "backup""#, ["1", "2"].as_slice()),
        ("false // (null,7)", &["null", "7"]),
        ("(null,false) // (null,5) // 6", &["5"]),
        ("empty // 9", &["9"]),
        ("(true,false) and (true,false)", &["true", "false", "false"]),
        ("(true,false) or (true,false)", &["true", "true", "false"]),
        ("(false,true) and (1,2)", &["false", "true", "true"]),
        (r#"false and error("x")"#, &["false"]),
        (r#"true or error("x")"#, &["true"]),
    ] {
        assert_eq!(
            as_strs(&full_outputs(b"null", filter)),
            expected,
            "full evaluator disagrees with jq for `{filter}`"
        );
        assert_parity(b"null", filter);
    }
}

#[test]
fn test_multi_output_condition_in_select_parity_160() {
    // `and`/`or` can now hand `select` a multi-output condition, where the two
    // evaluators disagree: `builtin_select` (eval.rs) tests the first output,
    // while eval_generic's `Builtin::Select` treats any multi-output condition
    // as truthy outright. jq fans the condition out instead, emitting the input
    // once per truthy output -- so both are wrong, in different ways.
    //
    // That drift predates #160; #160 only widened what can reach it. Pinned
    // here rather than fixed, because fixing `select` is the separate
    // follow-up. jq's answer for the filter below is no output at all, which
    // is what the full evaluator happens to give.
    assert_eq!(
        as_strs(&full_outputs(b"1", "[(false,false) and true]")),
        ["[false,false]"]
    );
    assert_divergence(b"1", "select((false,false) and true)", &[], &["1"]);
}

#[test]
fn test_out_of_bounds_index_parity_307() {
    // jq: indexing an array out of bounds (positive or negative) yields `null`.
    // Both evaluators now agree; the generic (CLI) path previously erred -- #307.
    for filter in [".[5]", ".[-5]", ".[100]"] {
        assert_parity(br"[1,2,3]", filter);
    }
    // The `?` variant also yields null (no error for `?` to suppress).
    assert_parity(br"[1,2,3]", ".[10]?");
}

#[test]
fn test_bsearch_parity_384() {
    // `bsearch` lives only in the full evaluator; the generic (CLI) path
    // reaches it through the fallback that re-renders the input as JSON and
    // hands it to `full_eval`. These pin that the round trip preserves the
    // answer -- including the negative insertion point, which the fallback
    // would have to carry back as a number rather than the object `bsearch`
    // returned before #384.
    for filter in ["bsearch(3)", "bsearch(5)", "bsearch(0)"] {
        assert_parity(br"[1,2,3,4]", filter);
    }
    // Containers exercise the recursive comparator across the round trip.
    assert_parity(br"[[1],[2],[3]]", "bsearch([2])");
    assert_parity(br"[[1],[2],[3]]", "bsearch([9])");
    assert_parity(br#"[{"a":1},{"a":3}]"#, r#"bsearch({"a":2})"#);
    assert_parity(br"[]", "bsearch(1)");
}

#[test]
fn test_parity_delpaths_398() {
    // `delpaths` sorts its path list and deletes by grouped prefix, so the
    // caller's order is immaterial and a repeat deletes once -- #398. Only
    // `src/jq/eval.rs` implements it; `eval_generic` has no `DelPaths` arm and
    // round-trips through JSON to the full evaluator, so most of these confirm
    // that fallback rather than a second implementation.
    for filter in [
        "delpaths([[0],[2]])",
        "delpaths([[2],[0]])",
        "delpaths([[0],[0]])",
        "delpaths([[-1],[-2]])",
        "delpaths([[3],[-1]])",
        "delpaths([[0],[0,1]])",
        "delpaths([[]])",
    ] {
        assert_parity(br"[10,20,30,40]", filter);
    }
    // Not a tautology: the round trip is where object key order could be lost,
    // and this is the case that would show it.
    assert_parity(
        br#"{"a":{"x":1,"y":2},"b":3,"c":4}"#,
        r#"delpaths([["a","x"],["b"]])"#,
    );
}
