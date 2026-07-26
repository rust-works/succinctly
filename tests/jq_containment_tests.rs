//! `contains`/`inside` conformance for mismatched operand types (#358).
//!
//! jq raises an error when the two operands' types cannot be compared;
//! succinctly used to answer `false`. The subtlety is *where* the check applies:
//! only the outermost pair of operands is screened, so a mismatch nested inside
//! a container stays `false`, and `Int`/`Float` are one type (`number`) and so
//! never mismatch.
//!
//! # Oracle provenance
//!
//! Every expectation below is the observed output of jq 1.7.1, captured with:
//!
//! ```text
//! echo 1                | jq -c 'contains("a")'
//! echo 1                | jq -c 'inside([1])'
//! echo '[1,"a"]'        | jq -c 'contains(["a",2])'
//! echo '"abcdefghijkl"' | jq -c 'contains(1)'      # 14-byte dump, kept whole
//! echo '"abcdefghijklm"'| jq -c 'contains(1)'      # 15 bytes, truncated
//! ```
//!
//! These are hand-transcribed rather than generated, so the suite is hermetic —
//! it needs no `jq` binary. Do not "fix" an expectation to match succinctly;
//! re-run the probe against jq first.
//!
//! # Why both evaluators
//!
//! The library entry point uses the full evaluator (`src/jq/eval.rs`) while the
//! CLI (`sjq`, `syq`) uses the generic one (`src/jq/eval_generic.rs`), which has
//! no `contains` arm of its own and delegates to the full evaluator. Running
//! every case through both is what proves the CLI path errors too — see
//! `tests/jq_evaluator_parity_tests.rs` for the general form of this drift risk.

use succinctly::jq::{eval, eval_generic, parse, JqSemantics, OwnedValue, QueryResult};
use succinctly::json::JsonIndex;

/// What jq does with a case: either it prints values, or it raises a message.
#[derive(Debug, PartialEq, Eq)]
enum Expect {
    /// Output values, rendered as compact JSON.
    Values(&'static [&'static str]),
    /// The raised error message.
    Error(&'static str),
}

/// Outcome of the full evaluator (`src/jq/eval.rs`), the library entry point.
fn full_outcome(json: &[u8], filter: &str) -> Result<Vec<String>, String> {
    let index = JsonIndex::build(json);
    let cursor = index.root(json);
    let expr = parse(filter).expect("parse failed");
    match eval::<Vec<u64>, JqSemantics>(&expr, cursor) {
        QueryResult::Error(e) => Err(e.message),
        other => Ok(other
            .collect_owned()
            .iter()
            .map(OwnedValue::to_json)
            .collect()),
    }
}

/// Outcome of the generic evaluator (`src/jq/eval_generic.rs`), the CLI path.
fn generic_outcome(json: &[u8], filter: &str) -> Result<Vec<String>, String> {
    let index = JsonIndex::build(json);
    let cursor = index.root(json);
    let expr = parse(filter).expect("parse failed");
    let result = eval_generic::eval_with_cursor(&expr, cursor);
    if let Some(e) = result.error() {
        return Err(e.message.clone());
    }
    Ok(result
        .collect_owned()
        .iter()
        .map(OwnedValue::to_json)
        .collect())
}

/// `(input, filter, what jq does)`.
const CASES: &[(&[u8], &str, Expect)] = &[
    // --- mismatched types: the divergence #358 fixed -----------------------
    (
        br"1",
        r#"contains("a")"#,
        Expect::Error(r#"number (1) and string ("a") cannot have their containment checked"#),
    ),
    (
        br"1",
        r"inside([1])",
        // `inside` swaps the operands, so its argument leads the message.
        Expect::Error("array ([1]) and number (1) cannot have their containment checked"),
    ),
    (
        br"true",
        r"contains(1)",
        Expect::Error("boolean (true) and number (1) cannot have their containment checked"),
    ),
    (
        br"null",
        r"contains(1)",
        Expect::Error("null (null) and number (1) cannot have their containment checked"),
    ),
    (
        br#""a""#,
        r"contains(null)",
        Expect::Error(r#"string ("a") and null (null) cannot have their containment checked"#),
    ),
    (
        br#"["aaaaaaaaaaaaaaaaaaaaaaa"]"#,
        r#"contains("a")"#,
        Expect::Error(
            r#"array (["aaaaaaaaa...) and string ("a") cannot have their containment checked"#,
        ),
    ),
    // --- the preview's truncation boundary: jq's `char buf[15]` ------------
    (
        br#""abcdefghijkl""#, // dump is exactly 14 bytes: kept whole
        r"contains(1)",
        Expect::Error(
            r#"string ("abcdefghijkl") and number (1) cannot have their containment checked"#,
        ),
    ),
    (
        br#""abcdefghijklm""#, // 15 bytes: cut to 11, plus `...`
        r"contains(1)",
        Expect::Error(
            r#"string ("abcdefghij...) and number (1) cannot have their containment checked"#,
        ),
    ),
    (
        br#"{"aaa":1,"bbb":2,"ccc":3,"ddd":4}"#,
        r"contains(1)",
        Expect::Error(
            r#"object ({"aaa":1,"b...) and number (1) cannot have their containment checked"#,
        ),
    ),
    // (`?` suppression is covered by `optional_suppresses_the_error` below —
    //  the surface syntax `contains("a")?` does not parse yet.)
    // --- what must NOT change --------------------------------------------
    // A mismatch nested inside a container is plain false, not an error.
    (
        br#"[1,"a"]"#,
        r#"contains(["a",2])"#,
        Expect::Values(&["false"]),
    ),
    (
        br#"[{"a":1}]"#,
        r#"contains([{"a":"x"}])"#,
        Expect::Values(&["false"]),
    ),
    // Int and Float are one type, so these compare numerically.
    (br"[1,2,3]", r"contains([1.0])", Expect::Values(&["true"])),
    (br"[1.0,2,3]", r"contains([1])", Expect::Values(&["true"])),
    (br"[1.0]", r"inside([1,2,3])", Expect::Values(&["true"])),
    (br"1", r"contains(1.0)", Expect::Values(&["true"])),
    // Matching types that simply do not contain each other.
    (br"[1]", r"contains([2])", Expect::Values(&["false"])),
    (br"1", r"contains(2)", Expect::Values(&["false"])),
    (br#""ab""#, r#"contains("a")"#, Expect::Values(&["true"])),
    (
        br#"{"a":1}"#,
        r#"contains({"a":1})"#,
        Expect::Values(&["true"]),
    ),
];

#[test]
fn containment_matches_jq_in_the_full_evaluator() {
    for (input, filter, expect) in CASES {
        let actual = full_outcome(input, filter);
        assert_outcome("full", input, filter, expect, &actual);
    }
}

#[test]
fn containment_matches_jq_in_the_generic_evaluator() {
    for (input, filter, expect) in CASES {
        let actual = generic_outcome(input, filter);
        assert_outcome("generic", input, filter, expect, &actual);
    }
}

/// An optional expression swallows the error and yields nothing, as `?` does for
/// every other error — jq prints nothing for `1 | contains("a")?`.
///
/// The expression is built with [`Expr::optional`] rather than parsed, because
/// succinctly's parser rejects a postfix `?` after *any* function call
/// (`contains("a")?`, `has("a")?`, even `(contains("a"))?` — "unexpected
/// character '?'"), where jq accepts all three. That is a parser gap unrelated to
/// containment; going through the builder still exercises the evaluators' real
/// `Expr::Optional` path, so this starts covering the surface syntax the moment
/// the parser catches up.
///
/// Two gaps therefore sit behind this test, both pre-existing and both invisible
/// until `contains` started erroring at all:
///
/// 1. the parser cannot express `contains("a")?`;
/// 2. the generic evaluator drops the flag: its catch-all arm re-enters the full
///    evaluator with a fresh `optional = false`
///    (`src/jq/eval_generic.rs`, the `_ =>` fallback), so an optional-wrapped
///    builtin it does not implement itself loses its optionality.
///
/// Gap 2 is pinned below rather than fixed, so the divergence is on record and a
/// fix is forced to update this test — the convention
/// `tests/jq_evaluator_parity_tests.rs` uses for evaluator drift.
#[test]
fn optional_suppresses_the_error() {
    for filter in [r#"contains("a")"#, r"inside([1])"] {
        let expr = parse(filter).expect("parse failed").optional();
        let json: &[u8] = br"1";
        let index = JsonIndex::build(json);

        let full: QueryResult<Vec<u64>> = eval::<Vec<u64>, JqSemantics>(&expr, index.root(json));
        assert!(
            !full.is_error(),
            "full evaluator: optional {filter} should be suppressed"
        );
        assert!(
            full.collect_owned().is_empty(),
            "full evaluator: optional {filter} should yield nothing"
        );

        // Gap 2, pinned: the CLI path still raises. Flip this to the assertions
        // above once the fallback threads `optional` through.
        let generic = eval_generic::eval_with_cursor(&expr, index.root(json));
        assert!(
            generic.is_error(),
            "generic evaluator: optional {filter} unexpectedly suppressed — \
             the fallback now threads `optional`, so tighten this test"
        );
    }
}

/// `catch` binds the raised message as a string, so an uncaught error and a
/// caught one have to carry the same text (#158 made this observable).
#[test]
fn containment_error_is_catchable_as_a_string() {
    let caught = full_outcome(br"1", r#"try contains("a") catch ."#).expect("catch swallows it");
    assert_eq!(
        caught,
        vec![
            r#""number (1) and string (\"a\") cannot have their containment checked""#.to_string()
        ]
    );
}

fn assert_outcome(
    evaluator: &str,
    input: &[u8],
    filter: &str,
    expect: &Expect,
    actual: &Result<Vec<String>, String>,
) {
    let input = String::from_utf8_lossy(input);
    match (expect, actual) {
        (Expect::Error(want), Err(got)) => assert_eq!(
            got, want,
            "{evaluator} evaluator: {input} | {filter} raised the wrong message"
        ),
        (Expect::Values(want), Ok(got)) => assert_eq!(
            got, want,
            "{evaluator} evaluator: {input} | {filter} produced the wrong output"
        ),
        (Expect::Error(want), Ok(got)) => panic!(
            "{evaluator} evaluator: {input} | {filter} should have raised\n  \
             {want}\nbut produced {got:?}"
        ),
        (Expect::Values(want), Err(got)) => panic!(
            "{evaluator} evaluator: {input} | {filter} should have produced \
             {want:?}\nbut raised\n  {got}"
        ),
    }
}
