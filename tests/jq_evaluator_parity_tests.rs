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
use succinctly::jq::{eval, parse, Expr, JqSemantics, QueryResult};
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
    // `null | length == 0` in jq, so `null` answers "not found" like `[]`
    // rather than erroring (#420); the round trip must preserve that too.
    assert_parity(br"null", "bsearch(1)");
    // A NaN needle is never found in a NaN-free sorted haystack -- NaN
    // orders as less than every number, so `compare_values` never answers
    // `Equal` for it (#421).
    assert_parity(br"[1,2,3]", "bsearch(nan)");
}

#[test]
fn test_nan_ordering_parity_421() {
    // jq treats NaN as strictly less than every number, including another
    // NaN. `f64::partial_cmp` returns `None` for any NaN comparison, and
    // both evaluators used to paper over that in incompatible, both-wrong
    // ways: the full evaluator folded it to `Equal` (NaN compared equal to
    // everything); the generic (CLI) evaluator's `<`/`<=`/`>`/`>=` fast path
    // folded the resulting `None` to `false` (NaN compared less than
    // nothing). Every expected value below is pinned against jq-1.7.1-apple.
    for (filter, expected) in [
        ("nan < 1", "true"),
        ("nan > 1", "false"),
        ("nan <= 1", "true"),
        ("nan >= 1", "false"),
        ("1 < nan", "false"),
        ("1 > nan", "true"),
        ("nan < nan", "true"),
        ("nan <= nan", "true"),
        ("nan >= nan", "false"),
        ("nan > nan", "false"),
        ("nan == nan", "false"),
    ] {
        assert_eq!(
            as_strs(&full_outputs(b"null", filter)),
            [expected],
            "full evaluator disagrees with jq for `{filter}`"
        );
        assert_parity(b"null", filter);
    }
}

#[test]
fn test_nan_container_ordering_parity_421() {
    // NaN's ordering rule reaches every container builtin that sorts.
    // `sort`/`unique`/`group_by` have no dedicated fast path in the generic
    // (CLI) evaluator -- like `bsearch` (#384), they fall through its JSON
    // round-trip fallback into the full evaluator, so `assert_parity` here
    // pins that round trip rather than a second implementation. Every
    // expected value is pinned against jq-1.7.1-apple.
    for (filter, expected) in [
        ("[3,nan,1] | sort", "[null,1,3]"),
        ("[1,nan] | min", "null"),
        ("[nan,1] | min", "null"),
        ("[1,nan] | max", "1"),
        ("[nan,1] | max", "1"),
        // A single NaN in the array needs no dedup/grouping decision against
        // another NaN, so this one is unaffected by the separate defect below.
        ("[nan,1,2] | group_by(.)", "[[null],[1],[2]]"),
    ] {
        assert_eq!(
            as_strs(&full_outputs(b"null", filter)),
            [expected],
            "full evaluator disagrees with jq for `{filter}`"
        );
        assert_parity(b"null", filter);
    }
}

#[test]
fn test_nan_container_ordering_known_divergence_421() {
    // jq keeps NaN a real NaN internally and only turns it into `null` at
    // print time, so `[nan,nan] | unique` keeps both (jq: `[null,null]`).
    // Here, a freshly-constructed array is materialized through JSON text on
    // its way to `unique`/`group_by` (JSON has no NaN literal), which turns
    // each NaN into a genuine `Null` *before* `compare_values` ever runs --
    // and two real `Null`s legitimately compare `Equal`, so they collapse.
    //
    // This is the separate, pre-existing defect #421 calls out ("nan does
    // not survive as a number") -- not a comparator bug, and out of scope for
    // this fix. Pinning the current (wrong, but internally consistent
    // between both evaluators) answer here so a fix for that defect has a
    // failing test to flip, rather than silently dropping coverage.
    for (filter, current_answer) in [
        ("[nan,nan] | unique", "[null]"),
        ("[nan,1,nan] | unique", "[null,1]"),
        ("[nan,nan,1] | group_by(.)", "[[null,null],[1]]"),
    ] {
        assert_eq!(
            as_strs(&full_outputs(b"null", filter)),
            [current_answer],
            "full evaluator answer changed for `{filter}` -- if this now matches jq \
             (`[null,null]` / `[null,null,1]` / `[[null],[null],[1]]` respectively), \
             the separate NaN-materialization defect is fixed: update this test's \
             expectation and move the case into test_nan_container_ordering_parity_421"
        );
        assert_parity(b"null", filter);
    }
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

/// Assert both evaluators produce identical, non-error, empty output for an
/// optional-wrapped `expr` -- the shared assertion `?` collapses to once the
/// error it would have raised is suppressed.
fn assert_optional_parity_suppressed(json: &[u8], expr: &Expr) {
    let index = JsonIndex::build(json);

    let full: QueryResult<Vec<u64>> = eval::<Vec<u64>, JqSemantics>(expr, index.root(json));
    assert!(
        !full.is_error(),
        "full evaluator: {expr:?} should be suppressed"
    );
    assert!(
        full.collect_owned().is_empty(),
        "full evaluator: {expr:?} should yield nothing"
    );

    let generic = eval_generic::eval_with_cursor(expr, index.root(json));
    assert!(
        !generic.is_error(),
        "generic evaluator: {expr:?} should be suppressed"
    );
    assert!(
        generic.collect_owned().is_empty(),
        "generic evaluator: {expr:?} should yield nothing"
    );
}

#[test]
fn test_optional_builtin_fallback_parity_386() {
    // `eval_generic`'s builtin dispatch handles a handful of builtins itself
    // and sends the rest to the full evaluator via `eval_on_owned`. That
    // fallback used to rebuild a bare `Expr::Builtin`, dropping the `optional`
    // flag it was called with -- so `builtin?` raised through the CLI path
    // even though the full evaluator suppressed it (#386). `bsearch` and
    // `contains` both live only in the full evaluator, so both reach this
    // fallback (`src/jq/eval_generic.rs`, `eval_builtin`'s `_ =>` arm).
    //
    // `?` can't be parsed after a call (#367), so the expression is built
    // with `Expr::optional` rather than parsed -- same as
    // `jq_containment_tests.rs::optional_suppresses_the_error`.
    for (json, filter) in [(b"1".as_slice(), r#"contains("a")"#), (b"1", "bsearch(9)")] {
        let expr = parse(filter).expect("parse failed").optional();
        assert_optional_parity_suppressed(json, &expr);
    }
}

#[test]
fn test_optional_pipe_fallback_no_longer_raises_386() {
    // A second, related fallback site: once a pipe stage has round-tripped
    // through `eval_on_owned` and produced an owned intermediate value,
    // continuing the pipe from that owned value used the same JSON round trip
    // (`src/jq/eval_generic.rs`, the `GenericResult::Owned`/`ManyOwned` arms
    // of `Expr::Pipe`) and dropped `optional` the same way (#386).
    //
    // `contains(["a"])` on `["ab"]` succeeds (true), round-tripping through
    // the fallback into an owned boolean. Piping that boolean into
    // `contains("x")` errors (containment is undefined for booleans) -- before
    // the fix that error escaped even though the whole pipe is wrapped
    // optional; now it's suppressed.
    //
    // This does NOT use `assert_optional_parity_suppressed`: the full
    // evaluator's own owned-pipe continuation (`eval_owned_expr` in
    // `src/jq/eval.rs`) collapses a suppressed `None` into `null` rather than
    // "no output" -- a pre-existing, unrelated quirk that exists to give
    // `reduce`/`foreach` a single value per step. So `full` yields `null` here
    // while `generic` yields nothing; both are correctly non-error, which is
    // all #386 is about, so only that is asserted.
    let expr = Expr::Pipe(vec![
        parse(r#"contains(["a"])"#).expect("parse failed"),
        parse(r#"contains("x")"#).expect("parse failed"),
    ])
    .optional();
    let json: &[u8] = br#"["ab"]"#;
    let index = JsonIndex::build(json);

    let full: QueryResult<Vec<u64>> = eval::<Vec<u64>, JqSemantics>(&expr, index.root(json));
    assert!(
        !full.is_error(),
        "full evaluator: optional pipe should be suppressed, not raise"
    );

    let generic = eval_generic::eval_with_cursor(&expr, index.root(json));
    assert!(
        !generic.is_error(),
        "generic evaluator: optional pipe should be suppressed, not raise"
    );
}

#[test]
fn test_parity_number_literal_preservation_387() {
    // `tostring`/`tojson`/`@json`/string interpolation on a document number
    // used to lose the source literal and re-render Rust's own `f64::Display`
    // (`1e100` -> a 101-digit integer). `tostring` is implemented directly in
    // both evaluators (`eval.rs::builtin_tostring`,
    // `eval_generic.rs::Builtin::ToString`), so this is exactly the kind of
    // two-implementation drift this file exists to catch (#387).
    //
    // Every expectation is pinned against jq-1.7.1 first, so parity can't lock
    // in an agreed-upon wrong answer.
    for (json, filter, expected) in [
        (b"1e100".as_slice(), "tostring", "1E+100"),
        (b"1.0", "tostring", "1.0"),
        (b"-0.0", "tostring", "-0.0"),
        (b"1e-7", "tostring", "1E-7"),
        (b"1e100", "tojson", "1E+100"),
        (b"1.0", "tojson", "1.0"),
        (b"1e100", r#""\(.)""#, "1E+100"),
    ] {
        assert_eq!(
            as_strs(&full_outputs(json, filter)),
            [format!("\"{expected}\"")],
            "full evaluator disagrees with jq for `{filter}` on `{}`",
            String::from_utf8_lossy(json)
        );
        assert_parity(json, filter);
    }

    // A computed number (post-arithmetic) is a fresh value, not a passthrough,
    // so it drops the literal and both evaluators still agree with each other
    // -- this only pins parity, not a specific jq-matching spelling (that gap
    // is pre-existing and unrelated to #387; see CLAUDE.md's own notes).
    assert_parity(b"1e100", "(. + 0) | tostring");

    // The streaming identity path was already correct before #387 and must
    // stay that way -- `-0.0` in particular is the case the original report
    // used to show identity was fine while `tostring` was not.
    assert_parity(b"-0.0", ".");
}

#[test]
fn test_parity_number_literal_reaches_numeric_arg_builtins_387() {
    // #387 made every document number materialize as `OwnedValue::NumberLiteral`
    // instead of plain `Int`/`Float`. A handful of builtins in `eval.rs` matched
    // their numeric *argument* against `OwnedValue::Int(_)` only (not the new
    // variant), so a document-sourced argument -- a field, an array element, a
    // bound variable -- fell through to their "not a number" error arm even
    // though the value plainly was one. A filter literal (`limit(2; ...)`)
    // never hit this, which is why it went unnoticed: only indirection through
    // data did. Every expectation here is pinned against jq-1.7.1 first.
    for (json, filter, expected) in [
        (br#"{"n":2}"#.as_slice(), "[limit(.n; range(10))]", "[0,1]"),
        (br"[10,20,30,1]", "nth(.[3])", "20"),
        (br"[1,[9,[2,3]]]", "flatten(.[0])", "[1,9,[2,3]]"),
        (br"[1,2,3]", "has(.[0])", "true"),
        (br"[99,1]", "getpath([.[1]])", "1"),
        (br"[1,2]", "[combinations(.[0])]|length", "2"),
        (
            br#"{"y":1,"x":1}"#,
            ". as $o | atan2($o.y; $o.x)",
            "0.7853981633974483",
        ),
    ] {
        assert_eq!(
            as_strs(&full_outputs(json, filter)),
            [expected],
            "full evaluator disagrees with jq for `{filter}` on `{}`",
            String::from_utf8_lossy(json)
        );
        assert_parity(json, filter);
    }
}

#[test]
fn test_parity_number_literal_ordering_agrees_with_equality_387() {
    // `compare_values`'s first cut at a `NumberLiteral` ordering arm tried an
    // exact `i64` comparison before falling back to `f64`, while `==`
    // (`OwnedValue::PartialEq`) always widens a mixed pair to `f64`. Above
    // 2^53 the two representations of "the same number" disagree about
    // whether an `i64` round-trips through `f64` exactly, so `==` and `>`
    // could both report `true` for the same pair -- e.g. `sort`/`unique`
    // disagreeing with `==` about whether two values are the same number.
    // This is an internal-consistency property, not a jq-parity one: this
    // crate already documents (`OwnedValue`'s `PartialEq` doc comment) that it
    // widens to `f64` here where jq 1.7 keeps full decimal precision, so `==`
    // itself already diverges from jq for this pair -- what must not diverge
    // is `==` from `>`/`<`/`sort` about the *same* values.
    let json = br"[9007199254740993, 9007199254740992.0]";
    for filter in [".[0] == .[1]", ".[0] > .[1]", ".[0] < .[1]"] {
        assert_parity(json, filter);
    }
    assert_eq!(as_strs(&full_outputs(json, ".[0] == .[1]")), ["true"]);
    assert_eq!(as_strs(&full_outputs(json, ".[0] > .[1]")), ["false"]);
    assert_eq!(as_strs(&full_outputs(json, ".[0] < .[1]")), ["false"]);
}

#[test]
fn test_parity_number_literal_reaches_more_numeric_arg_builtins_387() {
    // A second batch of builtins that, like
    // `test_parity_number_literal_reaches_numeric_arg_builtins_387`, match a
    // numeric *argument* (not the primary input) against `OwnedValue::Int`/
    // `Float` and needed a `NumberLiteral` arm added alongside: in()'s
    // negative-index check, range()'s bounds, setpath's index (reached via
    // `[]=`), mktime/strftime's broken-down-time array elements,
    // combinations(n), pick/omit's index lists, tonumber's already-numeric
    // passthrough, and @sh's numeric formatting.
    //
    // Every argument below is deliberately sourced by *direct* indexing
    // (`.field`, `.[idx]`) rather than through `as $var`/`reduce` binding:
    // variable binding round-trips a value through `owned_to_expr`, whose own
    // doc comment says a bound `NumberLiteral` "degrades to its plain parsed
    // form" (`Expr::Literal` has no source-text slot) -- so a `$var`-sourced
    // argument would exercise the already-covered plain Int/Float arm
    // instead of the new one. Every expectation is pinned against jq-1.7.1
    // (or, for the yq-only pick/omit, against this crate's own hermetic
    // yq-golden fixtures) first.
    for (json, filter, expected) in [
        // `in()` (not `has()` -- a separate, near-duplicate implementation)
        // shares `has()`'s "jq: negative indices are never in range" rule.
        // Both key representations are needed: llvm-cov instruments each side
        // of the `OwnedValue::Int(idx) | OwnedValue::NumberLiteral(..)`
        // or-pattern as its own region, so a `NumberLiteral`-only key (the
        // #387-added arm) leaves the pre-existing plain-`Int` arm looking
        // uncovered on the same source line.
        (br"null".as_slice(), "(-1) | in([1,2,3])", "false"),
        (br"[1,2,3,-1]", ".[3] | in([1,2,3,-1])", "false"),
        (br#"{"a":0,"b":3}"#, "[range(.a; .b)]", "[0,1,2]"),
        // `setpath(path; value)` -- not the `[]=` assignment operator, which
        // resolves indices through a separate `resolve_dynamic_indexes` path
        // that doesn't share this match -- with both an Int- and
        // Float-repr'd `NumberLiteral` index.
        (br"[10,20,30,1]", "setpath([.[-1]]; 99)", "[10,99,30,1]"),
        (br"[10,20,30,1.7]", "setpath([.[-1]]; 99)", "[10,99,30,1.7]"),
        (br"[2020.0,0,1,0,0,0]", "mktime", "1577836800"),
        (br"[1,2,3,2]", "[combinations(.[-1])] | length", "16"),
        // Both an Int- and a Float-repr'd `NumberLiteral` index.
        (br"[10,20,30,1]", "pick([.[-1]])", "[20]"),
        (br"[10,20,30,1]", "omit([.[-1]])", "[10,30,1]"),
        (br"[10,20,30,1.0]", "pick([.[-1]])", "[20]"),
        (br"[10,20,30,1.0]", "omit([.[-1]])", "[10,30,1.0]"),
        (br"1e100", "tonumber", "1E+100"),
        (br"1e2", "@sh", "\"1E+2\""),
    ] {
        assert_eq!(
            as_strs(&full_outputs(json, filter)),
            [expected],
            "full evaluator disagrees with jq for `{filter}` on `{}`",
            String::from_utf8_lossy(json)
        );
        assert_parity(json, filter);
    }

    // strftime returns a raw (unquoted) string, so it's checked separately
    // from the `to_json`-per-output loop above.
    assert_eq!(
        as_strs(&full_outputs(
            br"[2020.0,0,1,0,0,0]",
            r#"strftime("%Y-%m-%d")"#
        )),
        ["\"2020-01-01\""]
    );
    assert_parity(br"[2020.0,0,1,0,0,0]", r#"strftime("%Y-%m-%d")"#);
}

/// A slice is a path component (#366), so it reaches `path()`, `getpath`,
/// `setpath`, `delpaths`, `=`, `|=` and `del()`. The CLI drives the generic
/// evaluator, which has no `Expr::Slice` arm of its own and round-trips to the
/// full one — this pins that the hand-off keeps every one of those in step.
///
/// Each expectation is jq-1.7.1's, read off the pinned binary.
#[test]
fn slice_path_component_agrees_across_evaluators() {
    for (json, filter, expected) in [
        // `path()` yields ONE component carrying the bounds as written.
        (
            br"[1,2,3]".as_slice(),
            "path(.[1:2])",
            r#"[{"start":1,"end":2}]"#,
        ),
        (br"[1,2,3]", "path(.[-2:-1])", r#"[{"start":-2,"end":-1}]"#),
        (br"[1,2,3]", "path(.[1:])", r#"[{"start":1,"end":null}]"#),
        (br"[1,2,3]", "path(.[1:2][0])", r#"[{"start":1,"end":2},0]"#),
        // …and it round-trips back through the consumers.
        (br"[1,2,3]", "getpath(path(.[1:2]))", "[2]"),
        (
            br"[1,2,3]",
            r#"setpath(path(.[1:2]); ["z"])"#,
            r#"[1,"z",3]"#,
        ),
        (br"[1,2,3]", "delpaths([path(.[1:2])])", "[1,3]"),
        // Reading a descriptor: array, string, and the whole-container bounds.
        (br"[1,2,3]", r#"getpath([{"start":1,"end":2}])"#, "[2]"),
        (
            br#""abcdef""#,
            r#"getpath([{"start":1,"end":2}])"#,
            r#""b""#,
        ),
        (
            br"[1,2,3]",
            r#"getpath([{"start":null,"end":null}])"#,
            "[1,2,3]",
        ),
        // Writing splices, and the range clamps rather than refusing.
        (
            br"[1,2,3]",
            r#"setpath([{"start":1,"end":2}]; ["x","y"])"#,
            r#"[1,"x","y",3]"#,
        ),
        (
            br"[1,2,3]",
            r#"setpath([{"start":5,"end":9}]; ["x"])"#,
            r#"[1,2,3,"x"]"#,
        ),
        (
            br"[1,2,3]",
            r#"setpath([{"start":2,"end":1}]; ["x"])"#,
            r#"[1,2,"x",3]"#,
        ),
        (
            br"null",
            r#"setpath([{"start":1,"end":2}]; ["x"])"#,
            r#"["x"]"#,
        ),
        // The assignment operators, including a slice mid-chain.
        (br"[1,2,3]", r#".[1:2] = ["x"]"#, r#"[1,"x",3]"#),
        (br"[1,2,3]", r#".[1:2] |= . + ["q"]"#, r#"[1,2,"q",3]"#),
        (br"[1,2,3]", r#".[0:2] += ["x"]"#, r#"[1,2,"x",3]"#),
        (br"[1,2,3,4]", ".[1:3][] = 9", "[1,9,9,4]"),
        (
            br#"{"a":[1,{"b":5}]}"#,
            ".a[1:2][0].b = 9",
            r#"{"a":[1,{"b":9}]}"#,
        ),
        // Deleting: through a slice, and the single-batch union of ranges.
        (br"[1,2,3]", "del(.[1:2])", "[1,3]"),
        (br"[1,2,3]", "del(.[5:9])", "[1,2,3]"),
        (br"[1,[2],[3]]", "del(.[1:3][0])", "[1,[3]]"),
        (
            br"[1,2,3,4]",
            r#"delpaths([[{"start":0,"end":2}],[{"start":1,"end":3}]])"#,
            "[4]",
        ),
        (
            br"[1,2,3,4]",
            r#"delpaths([[1],[{"start":1,"end":2}]])"#,
            "[1,3,4]",
        ),
        // An object pattern to the string searches is the slice, not a search.
        (br#""abcabc""#, r#"indices({"start":1,"end":2})"#, r#""b""#),
    ] {
        assert_eq!(
            as_strs(&full_outputs(json, filter)),
            [expected],
            "full evaluator disagrees with jq for `{filter}` on `{}`",
            String::from_utf8_lossy(json)
        );
        assert_parity(json, filter);
    }
}
