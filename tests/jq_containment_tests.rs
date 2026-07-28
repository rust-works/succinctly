//! `contains`/`inside` conformance for mismatched operand kinds (#358).
//!
//! jq raises an error when the two operands' kinds cannot be compared;
//! succinctly used to answer `false`. Two subtleties decide the shape of the
//! check:
//!
//! 1. Only the outermost pair of operands is screened, so a mismatch nested
//!    inside a container stays `false`.
//! 2. The screen is on jq's *kind*, not its type name. `Int`/`Float` are one
//!    kind (`number`) and so never mismatch — but `true` and `false` are two
//!    kinds that share the name `boolean`, so `true | contains(false)` errors
//!    with a message calling both operands `boolean`. See `jq_kind` in
//!    `src/jq/eval.rs`.
//!
//! # Oracle provenance
//!
//! Every expectation below is the observed output of jq 1.7.1, captured with:
//!
//! ```text
//! echo 1                | jq -c 'contains("a")'
//! echo 1                | jq -c 'inside([1])'
//! echo '[1,"a"]'        | jq -c 'contains(["a",2])'
//! echo true             | jq -c 'contains(false)'   # distinct kinds, one name
//! echo '"abcdefghijkl"' | jq -c 'contains(1)'      # 14-byte dump, kept whole
//! echo '"abcdefghijklm"'| jq -c 'contains(1)'      # 15 bytes, truncated
//! printf '"a\302\205b"' | jq -c 'contains(1)'      # C1 control, passed through raw
//! printf '"x\010y"'      | jq -c 'contains(1)'      # backspace, jq's short form
//! printf '"x\177y"'      | jq -c 'contains(1)'      # DEL, escaped as \u007f
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
//!
//! # Merged with #356
//!
//! #356 landed second and paid the merge: `EvalError::containment_check` and the
//! truncation now exist once, in `src/jq/error.rs`, where `dump_truncated` is
//! the streaming preview this file's C1 case pins. The kind helpers
//! (`jq_kind`/`sort_rank`) stay in `src/jq/eval.rs` — they classify values
//! rather than word messages. `tests/data/jq-error-known-divergences.txt` does
//! not list the containment rows, because #358 fixed them before that manifest
//! arrived.

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
    // --- the two boolean kinds that share one name ------------------------
    // jq's `jv_kind` splits `JV_KIND_TRUE` from `JV_KIND_FALSE` and screens on
    // the kind, so a mixed pair errors even though `jv_kind_name` calls both
    // `boolean`. A `type_name`-based screen answers `false` here instead.
    (
        br"true",
        r"contains(false)",
        Expect::Error("boolean (true) and boolean (false) cannot have their containment checked"),
    ),
    (
        br"false",
        r"contains(true)",
        Expect::Error("boolean (false) and boolean (true) cannot have their containment checked"),
    ),
    (
        br"true",
        r"inside(false)",
        Expect::Error("boolean (false) and boolean (true) cannot have their containment checked"),
    ),
    (
        br"false",
        r"inside(true)",
        Expect::Error("boolean (true) and boolean (false) cannot have their containment checked"),
    ),
    // A matched pair is a plain comparison, and nested it is plain `false`.
    (br"true", r"contains(true)", Expect::Values(&["true"])),
    (br"false", r"contains(false)", Expect::Values(&["true"])),
    (br"true", r"inside(true)", Expect::Values(&["true"])),
    (br"[false]", r"contains([true])", Expect::Values(&["false"])),
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
    // The preview escapes exactly as jq does — #385 made that one writer
    // (`write_json_body_jq`) rather than three near-copies, so these three cases
    // pin all of `dump_truncated`'s escaping, not just the C1 row #358 needed.
    //
    // A C1 control (U+0085, the two bytes C2 85) is passed through raw. This is
    // the row that used to fail: `char::is_control()` is true for C1, so every
    // writer branching on it escaped a character jq leaves alone.
    (
        "\"a\u{85}b\"".as_bytes(),
        r"contains(1)",
        Expect::Error("string (\"a\u{85}b\") and number (1) cannot have their containment checked"),
    ),
    // Backspace takes jq's short form. yq writes it as the long \u0008, and
    // #358 previewed through yq's writer, so this row is what keeps the preview
    // on the jq one.
    (
        b"\"x\x08y\"",
        r"contains(1)",
        Expect::Error(r#"string ("x\by") and number (1) cannot have their containment checked"#),
    ),
    // DEL is escaped, though it is not below 0x20 — which is why the predicate
    // is `< 0x20 || == 0x7f` and not the bare `< 0x20` #385 first proposed.
    (
        b"\"x\x7fy\"",
        r"contains(1)",
        Expect::Error(
            r#"string ("x\u007fy") and number (1) cannot have their containment checked"#,
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
/// the parser catches up (#367).
///
/// Both evaluators agree here: the generic evaluator's fallback to the full
/// evaluator for builtins it doesn't implement itself now threads `optional`
/// through (`src/jq/eval_generic.rs`, `eval_on_owned`/`eval_on_many_owned`), so
/// an optional-wrapped builtin it delegates is suppressed the same way as the
/// full evaluator (#386, previously pinned as open drift here).
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

        let generic = eval_generic::eval_with_cursor(&expr, index.root(json));
        assert!(
            !generic.is_error(),
            "generic evaluator: optional {filter} should be suppressed"
        );
        assert!(
            generic.collect_owned().is_empty(),
            "generic evaluator: optional {filter} should yield nothing"
        );
    }
}

/// A number in the containment-check preview now reads back exactly as jq
/// prints it, because `OwnedValue::NumberLiteral` (#387) carries the source
/// literal through `EvalError::containment_check`'s `OwnedValue::to_json`
/// preview. Was pinned as `number_previews_are_canonicalised`, documenting the
/// opposite (canonicalised) output; see `dump_truncated`'s doc comment in
/// `src/jq/error.rs` for the sibling fix in the truncated-dump preview.
#[test]
fn number_previews_match_jq() {
    // jq: number (1E+100) and string ("a") cannot have their containment checked
    assert_eq!(
        full_outcome(br"1e100", r#"contains("a")"#),
        Err(
            r#"number (1E+100) and string ("a") cannot have their containment checked"#.to_string()
        )
    );

    // jq: number (1.0) and string ("a") cannot have their containment checked
    assert_eq!(
        full_outcome(br"1.0", r#"contains("a")"#),
        Err(r#"number (1.0) and string ("a") cannot have their containment checked"#.to_string())
    );

    // Integers that need no canonicalising are unaffected, which is why every
    // case in `CASES` agrees with jq exactly.
    assert_eq!(
        full_outcome(br"42", r#"contains("a")"#),
        Err(r#"number (42) and string ("a") cannot have their containment checked"#.to_string())
    );
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
