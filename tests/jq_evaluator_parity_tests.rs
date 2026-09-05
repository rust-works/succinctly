//! Evaluator-parity tests: the CLI uses the generic evaluator
//! (`src/jq/eval_generic.rs`) while the library's `jq::eval` entry point uses
//! the full evaluator (`src/jq/eval.rs`). For builtins implemented in both,
//! the two must agree; where they don't, that drift is a bug (#157/#161/#162).
//!
//! Each case renders both evaluators' outputs to JSON and compares them. Cases
//! that currently AGREE are asserted equal (locking them in). Cases that
//! currently DIVERGE are pinned with `assert_ne!` plus the observed outputs, so
//! the fix is forced to update them and no NEW drift slips in silently.
//!
//! A third axis lives at the bottom of this file (#2416 phase 0): inside
//! `eval_generic.rs`, a path-context pipe is answered either by the #2061
//! cursor walk or by the materializing bridge, and those two must agree too.
//! `eval_generic::PathContextRoute` makes both routes drivable from here.

use succinctly::jq::eval_generic;
use succinctly::jq::{eval, parse, EvalSemantics, Expr, JqSemantics, QueryResult, YqSemantics};
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
        .expect("materializes")
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

// ---------------------------------------------------------------------------
// Format functions (#124)
//
// Formats are pure functions of the value, so the two evaluators must agree on
// every one. The generic evaluator has no `Expr::Format` arm, so today it
// reaches them only via the catch-all's `to_json` + `JsonIndex::build`
// round-trip. These cases pin current behaviour so giving it a direct arm can
// be proven output-neutral.
// ---------------------------------------------------------------------------

/// Formats that accept scalars, over every scalar shape.
#[test]
fn test_parity_formats_scalar_inputs() {
    for filter in ["@text", "@json", "@uri", "@html", "@sh", "@yaml", "@props"] {
        for json in [
            b"null".as_slice(),
            b"true",
            b"false",
            b"42",
            b"-1.5",
            br#""hello world/?&=x""#,
            br#""<a href=\"x\">&y</a>""#,
            br#""it's""#,
            br#""""#,
        ] {
            assert_parity(json, filter);
        }
    }
}

/// Array-shaped formats, including the quoting/escaping edge cases.
#[test]
fn test_parity_formats_array_inputs() {
    for json in [
        br#"["a","b,c",1,true,null]"#.as_slice(),
        br#"["q\"uote","tab\there","nl\nhere","back\\slash"]"#,
        br#"["it's",-2.5]"#,
        br"[]",
    ] {
        for filter in [
            "@csv",
            "@tsv",
            r#"@dsv("|")"#,
            r#"@dsv(",")"#,
            "@sh",
            "@json",
            "@text",
            "@yaml",
            "@props",
        ] {
            assert_parity(json, filter);
        }
    }
}

/// Formats that only accept strings.
#[test]
fn test_parity_formats_string_only() {
    for filter in ["@base64", "@base64d", "@urid"] {
        for json in [
            br#""hello""#.as_slice(),
            br#""aGVsbG8=""#,
            br#""%C3%A9""#,
            br#""a%2Fb%20c""#,
            br#""100%""#,
            br#""""#,
        ] {
            assert_parity(json, filter);
        }
    }
}

/// A format that rejects its input must reject it identically in both.
///
/// `({"a":1}, "@uri")`, `({"a":1}, "@html")`, and `(b"42", "@base64")` used
/// to live in this list; #1096 fixed `@uri`/`@html`/`@base64` to
/// JSON-stringify a container, and `@base64` to convert any other scalar to
/// its display string too, before formatting (matching real jq: `42 |
/// jq -c '@base64'` => `"NDI="`), so none of the three error on these
/// inputs anymore. See `test_format_uri_container_jq_mode_1096`,
/// `test_format_html_container_jq_mode_1096`, and
/// `test_format_base64_scalar_conversion_1096` (`src/jq/eval.rs`) for their
/// corrected coverage instead. `{"a":1} | @sh` stays: `@sh` was never in
/// #1096's scope and still rejects a container.
#[test]
fn test_parity_formats_type_errors() {
    for (json, filter) in [
        (br#"{"a":1}"#.as_slice(), "@sh"),
        (b"42", "@csv"),
        (b"42", "@tsv"),
        (b"42", "@urid"),
        (br#""notanarray""#, "@csv"),
    ] {
        assert_parity(json, filter);
    }
}

/// The shape that matters most for #124: a *constructed* value piped into a
/// format routes through `eval_on_owned`, not `eval_single`, so it needs its
/// own coverage.
#[test]
fn test_parity_formats_piped_from_constructed() {
    let json = br#"{"a":"x y","b":2,"c":"q\"uote"}"#;
    for filter in [
        r"[.a,.b] | @csv",
        r"[.a,.c] | @csv",
        r"[.a,.b] | @tsv",
        r#"[.a,.b] | @dsv(";")"#,
        r"[.a,.b] | @json",
        r"[.a,.b] | @sh",
        r"[.a,.b] | @text",
        r".a | @uri",
        r".a | @html",
        r".c | @sh",
    ] {
        assert_parity(json, filter);
    }
}

/// Formats applied per-element across an iteration -- the CLI's common shape.
#[test]
fn test_parity_formats_over_iteration() {
    let json = br#"{"u":[{"n":"a b","v":1},{"n":"c&d","v":2}]}"#;
    for filter in [
        ".u[] | .n | @uri",
        ".u[] | .n | @html",
        ".u[] | [.n,.v] | @csv",
        ".u[] | [.n,.v] | @tsv",
        "[.u[].n] | @csv",
        ".u[] | @json",
    ] {
        assert_parity(json, filter);
    }
}

/// Non-finite floats exercise `numeric_display_string`/`owned_to_yaml`/
/// `props_value_to_string` (eval.rs), which already rendered `"inf"`/`".nan"`/
/// etc. consistently before #124 -- both evaluators agreed on these even when
/// the generic evaluator had no direct `Expr::Format` arm, since its catch-all
/// round-trip goes through `to_json_for_reindex` (preserving, #561), not plain
/// `to_json` (nulling). #124's direct arm doesn't change that outcome, only
/// how cheaply it's reached.
///
/// jq mode's own spelling changed under #1075 (`"inf"`/`"-inf"`, Rust's bare
/// `f64::Display`, to jq's actual `DBL_MAX`-text substitution) -- yq mode's
/// (`.inf`/`.nan`) is unaffected.
///
/// The pinned full-evaluator values below are asserted explicitly so parity
/// can't silently re-agree on a *new* wrong answer.
///
/// Note neither evaluator fully matches real jq here even after #1075: jq
/// 1.7.1 preserves an overflowed literal's own source text for *either*
/// sign when it's the input document itself (as every case below is) --
/// `1e400 | tostring` -> `"1E+400"`, `-1e400 | tostring` -> `"-1E+400"` --
/// rather than substituting `DBL_MAX` text for either. (A leading `-`
/// typed inside the *filter* text instead, rather than the document, is a
/// different, unrelated case: jq's own grammar treats that as unary
/// negation on the positive literal, degrading fidelity -- not what any
/// case here exercises.) That literal-preservation gap is a separate
/// pre-existing issue (#1083), out of scope for both #124 and #1075 -- see
/// `numeric_display_string`'s own doc comment (`src/jq/eval.rs`) for why.
#[test]
fn test_formats_non_finite_parity_124() {
    for (json, filter, expected) in [
        (b"1e400".as_slice(), "@text", r#""1.7976931348623157e+308""#),
        (b"-1e400", "@text", r#""-1.7976931348623157e+308""#),
        (b"1e400", "@uri", r#""1.7976931348623157e%2B308""#),
        (b"1e400", "@html", r#""1.7976931348623157e+308""#),
        (b"1e400", "@yaml", r#"".inf""#),
        (b"1e400", "@props", r#"".inf""#),
        (b"[1e400]", "@csv", r#""1.7976931348623157e+308""#),
        (b"[1e400]", "@tsv", r#""1.7976931348623157e+308""#),
        (b"[1e400]", "@sh", r#""1.7976931348623157e+308""#),
    ] {
        assert_eq!(
            as_strs(&full_outputs(json, filter)),
            [expected],
            "full evaluator output changed for `{filter}`"
        );
        assert_parity(json, filter);
    }
}

/// `@json` is immune: `format_json` calls `to_json`, which nulls non-finites
/// itself, so both evaluators already agree.
#[test]
fn test_formats_non_finite_json_parity() {
    assert_parity(b"1e400", "@json");
    assert_parity(b"[1e400]", "@json");
}

/// When the format's input is a *constructed* value, the full evaluator also
/// round-trips it through JSON (`eval_owned_pipe`/`eval_owned_input`), and that
/// round-trip is likewise `to_json_for_reindex`-based rather than nulling
/// (#561), so both evaluators already agree on the non-finite rendering here
/// too. `eval_on_owned`'s `Format` fast path (#124) formats these directly with
/// no guard, for the same reason `eval_single`'s arm needs none.
#[test]
fn test_formats_non_finite_owned_pipe_parity() {
    assert_parity(b"[1e400]", "[.[]] | @csv");
    assert_parity(b"[1e400]", "[.[]] | @tsv");
    assert_parity(b"[1e400]", "[.[]] | @sh");
    assert_parity(b"1e400", "[.] | @csv");
}

/// `?` on a format applied to a *constructed* value (arithmetic, array/object
/// construction, ...) must still suppress a type-mismatch error, producing no
/// output at all -- verified against jq 1.7.1 (`jq -n '(1+1 | @csv)?'` and
/// the `@tsv`/`@base64d`/`@urid` siblings all produce nothing, exit 0;
/// `@dsv` has no jq equivalent to check directly, since it's this crate's
/// own extension, but shares `format_csv`'s exact suppression code path).
///
/// `@base64` is deliberately *not* in this list (unlike before #1096): real
/// jq converts a scalar to its display string before base64-encoding it, so
/// `jq -n '(1+1 | @base64)?'` genuinely produces `"Mg=="`, not nothing --
/// re-verified live rather than trusted from the pre-#1096 version of this
/// comment, which had never actually checked `@base64` against real jq and
/// merely inherited succinctly's own (then-incorrect) error-on-scalar
/// behavior. See `test_format_base64_constructed_scalar_no_longer_suppressed_1096`
/// below for that corrected case.
///
/// Before #693, `format_csv`/`format_tsv`/`format_dsv`/`format_base64d`/
/// `format_urid`'s own `_ if optional => Ok(String::new())`
/// arm was the *only* thing that made `?` work here -- there was no separate
/// `Expr::Optional`-catches-`Error` fallback for this path, so a suppressed
/// format fell back to the empty *string* `""` (a real output) rather than
/// true suppression (no output), because `format_owned` returns `Result<
/// String, EvalError>` with no way to spell "no result". That workaround is
/// still in place (a constructed value still reaches the format via
/// `eval_on_owned`'s fast path (#124), forwarding the real ambient
/// `optional`) but is now moot for the bare `(EXPR | @format)?` shape tested
/// here: `Expr::Optional`'s new catch-after-the-fact dispatch evaluates the
/// pipe with `optional` forced to `false`, so the format leaf raises a real
/// `Error` instead of self-suppressing to `""`, and that real error is what
/// gets caught -- correctly producing no output, matching jq. The old
/// pinned `""` was never jq-verified and was wrong.
#[test]
fn test_formats_optional_owned_type_error_parity_124() {
    for (json, filter) in [
        (b"null".as_slice(), "(1+1 | @csv)?"),
        (b"null", "(1+1 | @tsv)?"),
        (b"null", r#"(1+1 | @dsv("|"))?"#),
        (b"null", "(1+1 | @base64d)?"),
        (b"null", "(1+1 | @urid)?"),
    ] {
        assert_eq!(
            full_outputs(json, filter),
            Vec::<String>::new(),
            "full evaluator output changed for `{filter}`"
        );
        assert_parity(json, filter);
    }
}

/// Counterpart to `test_formats_optional_owned_type_error_parity_124` above:
/// `@base64` on a *constructed* scalar no longer errors at all (#1096), so
/// `?` has nothing to suppress and the pipe produces its real output --
/// verified against real jq 1.7.1 (`jq -n '(1+1 | @base64)?'` => `"Mg=="`,
/// exit 0), not just the two evaluators agreeing with each other.
#[test]
fn test_format_base64_constructed_scalar_no_longer_suppressed_1096() {
    assert_eq!(
        as_strs(&full_outputs(b"null", "(1+1 | @base64)?")),
        ["\"Mg==\""]
    );
    assert_parity(b"null", "(1+1 | @base64)?");
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
    // `and`/`or` can hand `select`/`if` a multi-output condition. Before
    // #378, the two evaluators disagreed about it: `builtin_select`/`eval_if`
    // (eval.rs) tested only the condition's first output, while eval_generic's
    // `Builtin::Select` treated any multi-output condition as truthy outright.
    // jq fans the condition out instead, running the body once per output --
    // #378 makes both evaluators do that, so this is now `assert_parity`
    // rather than the `assert_divergence` it was pinned as.
    //
    // Every expectation is pinned against jq-1.7.1 first, so parity can't lock
    // in an agreed-upon wrong answer (this file's header failure mode).
    assert_eq!(
        as_strs(&full_outputs(b"1", "[(false,false) and true]")),
        ["[false,false]"]
    );
    assert_eq!(
        as_strs(&full_outputs(b"1", "select((false,false) and true)")),
        Vec::<&str>::new(),
        "full evaluator disagrees with jq"
    );
    assert_parity(b"1", "select((false,false) and true)");

    assert_eq!(
        as_strs(&full_outputs(b"1", "select((true,false) and true)")),
        ["1"],
        "full evaluator disagrees with jq"
    );
    assert_parity(b"1", "select((true,false) and true)");

    assert_eq!(
        as_strs(&full_outputs(
            b"null",
            r#"if (true,false) then "a" else "b" end"#
        )),
        [r#""a""#, r#""b""#],
        "full evaluator disagrees with jq"
    );
    assert_parity(b"null", r#"if (true,false) then "a" else "b" end"#);
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
        // another NaN, so this one needed no fix from #472's NaN-survival
        // work below.
        ("[nan,1,2] | group_by(.)", "[[null],[1],[2]]"),
        // These three used to collapse distinct NaNs into a single `null`
        // (`[null]` / `[null,1]` / `[[null,null],[1]]`): a freshly
        // constructed array is materialized through JSON text on its way to
        // `unique`/`group_by` (JSON has no NaN literal), which turned each
        // NaN into a genuine `Null` *before* `compare_values` ever ran, and
        // two real `Null`s legitimately compare `Equal`. #472 preserves NaN
        // through that round trip, so each NaN now stays distinct, exactly
        // like jq's own `nan != nan`.
        ("[nan,nan] | unique", "[null,null]"),
        ("[nan,1,nan] | unique", "[null,null,1]"),
        ("[nan,nan,1] | group_by(.)", "[[null],[null],[1]]"),
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
        generic.collect_owned().expect("materializes").is_empty(),
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
    for (json, filter) in [
        (b"1".as_slice(), r#"contains("a")?"#),
        (b"1", "bsearch(9)?"),
    ] {
        let expr = parse(filter).expect("parse failed");
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
    // from the `to_json`-per-output loop above. Real jq requires the full
    // 8-element array (weekday and yearday included) — a 6-element array
    // errors in real jq too (#760), so this uses 8 elements to keep
    // exercising the NumberLiteral-Float arm this test targets without
    // relying on the since-fixed 6-element leniency.
    assert_eq!(
        as_strs(&full_outputs(
            br"[2020.0,0,1,0,0,0,3,0]",
            r#"strftime("%Y-%m-%d")"#
        )),
        ["\"2020-01-01\""]
    );
    assert_parity(br"[2020.0,0,1,0,0,0,3,0]", r#"strftime("%Y-%m-%d")"#);
}

/// #1556: `range`'s bound resolution moved from a bespoke triple-nested
/// `stream_outputs` loop to `each_range` (a native `eval_each` lazy arm),
/// with `eval_range` itself becoming a thin wrapper over it. `eval_generic.rs`
/// has no `Expr::Range` handling of its own and always bridges into
/// `eval.rs` for this builtin, so this pins that relationship: a future
/// native `eval_generic.rs` arm for `Range` cannot silently diverge in the
/// *values* it produces without this test catching it (laziness itself --
/// which document `input` pops -- is a CLI-level, not a values-only,
/// concern, and is covered by `tests/jq_cli_tests.rs` instead).
#[test]
fn test_parity_range_multi_output_bounds_1556() {
    for (json, filter, expected) in [
        (br"null".as_slice(), "range((1,2))", "[0,0,1]"),
        (br"null", "range((0,1);(2,3))", "[0,1,0,1,2,1,1,2]"),
        (br"null", "range(0;6;(2,3))", "[0,2,4,0,3]"),
        (
            br"null",
            "range((0,1);(2,3);(1,2))",
            "[0,1,0,0,1,2,0,2,1,1,1,2,1]",
        ),
    ] {
        let full = full_outputs(json, &format!("[{filter}]"));
        assert_eq!(
            as_strs(&full),
            [expected],
            "full evaluator disagrees with jq for `{filter}`"
        );
        assert_parity(json, &format!("[{filter}]"));
    }
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

#[test]
fn test_object_construction_product_parity_354() {
    // Object construction is a generator: an entry whose key or value yields n
    // outputs multiplies the objects produced (#354). Both evaluators route
    // `Expr::Object` through eval.rs -- eval_generic has no Object arm -- so
    // parity alone cannot catch this bug. Every expectation below is therefore
    // pinned against real jq-1.7.1 first, then locked in by assert_parity.
    for (json, filter, expected) in [
        // The LAST entry varies fastest; within an entry the key varies slower
        // than the value. A transposed product would order these differently.
        (
            b"null".as_slice(),
            "{a: (1,2), b: (3,4)}",
            vec![
                r#"{"a":1,"b":3}"#,
                r#"{"a":1,"b":4}"#,
                r#"{"a":2,"b":3}"#,
                r#"{"a":2,"b":4}"#,
            ],
        ),
        // Multi-output keys are a product too, not an error.
        (
            b"null",
            r#"{("x","y"): (1,2)}"#,
            vec![r#"{"x":1}"#, r#"{"x":2}"#, r#"{"y":1}"#, r#"{"y":2}"#],
        ),
        // Borrowed (document-derived) multi-output values, not just literals.
        (
            br#"{"x":9,"y":8}"#,
            "{k: .[]}",
            vec![r#"{"k":9}"#, r#"{"k":8}"#],
        ),
        // An entry with zero outputs empties the whole product.
        (b"null", "{a: empty, b: 1}", vec![]),
        // Duplicate keys: last value wins, first position kept.
        (b"null", "{a:1, b:2, a:3}", vec![r#"{"a":3,"b":2}"#]),
        // A multi-output object as the RHS of a pipe. The full evaluator reaches
        // this via eval_owned_pipe, which used to fold the outputs into a single
        // array while the generic path kept them -- so this case is what keeps
        // the two evaluators honest after #354.
        (
            b"null",
            r#"{"p":1} | {a: (2,3)}"#,
            vec![r#"{"a":2}"#, r#"{"a":3}"#],
        ),
    ] {
        let full = full_outputs(json, filter);
        assert_eq!(
            as_strs(&full),
            expected,
            "full evaluator disagrees with jq for `{filter}`"
        );
        assert_parity(json, filter);
    }
}

/// `path`/`parent`/`parent(n)`/`key` (yq's path-context builtins) only get
/// their `current_path` threaded through `eval.rs`'s `eval_pipe`, reached
/// when `needs_path_context` sees one of them anywhere in an `Expr::Pipe`
/// list (#554). The generic (CLI/cursor) evaluator has its own independent
/// `Expr::Pipe` handling with no path-accumulator of any kind, and used to
/// bridge only the bare trailing builtin to the full evaluator once a
/// preceding stage collapsed to a plain value -- discarding the very pipe
/// structure `needs_path_context` needs to see. That silently returned the
/// root-level defaults (`[]`/`{}`/null) for any of these builtins appearing
/// anywhere but the first pipe stage. Every case here is wrong (mismatched
/// with `full_outputs`) before the generic evaluator's `Expr::Pipe` arm also
/// bridges the *whole* pipe once `needs_path_context` fires.
#[test]
fn test_path_context_builtins_survive_pipe_parity_554() {
    for (json, filter, expected) in [
        // The issue's own repro cases.
        (br#"{"a":1}"#.as_slice(), ".a | path", r#"["a"]"#),
        (br#"{"a":1}"#, ".a | parent", r#"{"a":1}"#),
        (br#"{"a":1}"#, ".a | key", r#""a""#),
        (br#"{"a":{"b":1}}"#, ".a.b | parent", r#"{"b":1}"#),
        // `parent(n)`: n=1 matches bare `parent`; n=2 climbs one further.
        (
            br#"{"a":{"b":{"c":1}}}"#,
            ".a.b.c | parent(1)",
            r#"{"c":1}"#,
        ),
        (
            br#"{"a":{"b":{"c":1}}}"#,
            ".a.b.c | parent(2)",
            r#"{"b":{"c":1}}"#,
        ),
        // A stage after the path-context builtin keeps consuming its output.
        (br#"{"a":1}"#, ".a | path | .[0]", r#""a""#),
        // A multi-output preceding stage (iteration): `key` reports each
        // element's own key/index, not just the first.
        (br"[10,20,30]", ".[] | key", "0"),
        (br#"{"x":1,"y":2}"#, ".[] | key", r#""x""#),
        // Bare, first-stage usage was already correct -- locked in here
        // too so a future change can't silently regress it.
        (br#"{"a":1}"#, "path", "[]"),
    ] {
        assert_parity(json, filter);
        let generic = generic_outputs(json, filter);
        assert_eq!(
            as_strs(&generic)[0],
            expected,
            "unexpected CLI (generic evaluator) output for `{filter}` on `{}`",
            String::from_utf8_lossy(json)
        );
    }
}

/// #1739: `Builtin::Has`'s new native arm in `eval_generic.rs` must agree
/// with `eval.rs`'s pre-existing `builtin_has`/`has_one_key` for every case
/// its own doc comment claims to cover -- a single-valued key against an
/// object, array, or `null` receiver. Expected values are jq-1.7.1-verified
/// (see this issue and #909's own array-key doc comment).
#[test]
fn test_parity_has_native_arm_1739() {
    for (json, filter, expected) in [
        (br#"{"a":1,"b":2}"#.as_slice(), r#"has("a")"#, "true"),
        (br#"{"a":1,"b":2}"#, r#"has("z")"#, "false"),
        (br"[1,2,3]", "has(1)", "true"),
        (br"[1,2,3]", "has(3)", "false"),
        (br"[1,2,3]", "has(-1)", "false"),
        (br"null", r#"has("a")"#, "false"),
        (br"[1,2,3]", "has(nan)", "false"),
        // A computed key sourced from the document itself, not a literal.
        (br#"{"a":1,"k":"a"}"#, "has(.k)", "true"),
        // A sibling key that fails to decode must not stop `has` from
        // answering about an unrelated, well-formed key (#1642 precedent --
        // `find`/`find_cursor` already skip it silently, same as `.foo`).
        (
            b"{\"\xff\xfe\": 1, \"a\": 2}".as_slice(),
            r#"has("a")"#,
            "true",
        ),
        // A repeated key: presence alone doesn't depend on which occurrence
        // `find` resolves to.
        (br#"{"a":1,"a":2}"#, r#"has("a")"#, "true"),
    ] {
        assert_eq!(
            as_strs(&full_outputs(json, filter)),
            [expected],
            "full evaluator disagrees with jq for `{filter}` on `{}`",
            String::from_utf8_lossy(json)
        );
        assert_parity(json, filter);
    }

    // A generator key still fans out one output per key (jq's rule, #1279)
    // -- this shape isn't handled by the new native arm and must keep
    // falling back to the existing round-trip path.
    assert_parity(br#"{"a":1,"b":2}"#, r#"[has(("a","z"))]"#);
}

/// #2293: `eval.rs`'s `has_one_key`/`builtin_length`/`builtin_keys` had the
/// same missing #1677/#2261 trailing-comma/malformed-member gap checks that
/// `eval_generic.rs`'s own siblings already closed (#2291) -- unreachable
/// from either shipped CLI today (both route through `eval_generic.rs`
/// exclusively, and `succinctly yq`'s `eval.rs` call site only ever
/// re-serializes an already-validated `OwnedValue`), but a real gap for a
/// library consumer of `jq::eval*` building a cursor directly from raw,
/// unvalidated bytes -- exactly what `full_outputs`/`assert_error_parity`
/// do here, bypassing the CLI's own upfront checks entirely. Every input
/// mirrors an existing `eval_generic.rs`-side CLI repro (`jq_cli_tests.rs`'s
/// `test_jq_cursor_transparent_fast_paths_reject_trailing_comma_2261`/
/// `test_jq_len_checked_sibling_paths_reject_trailing_comma_2261`), so both
/// evaluators must now raise with the same message.
///
/// Deliberately excludes `{"a":1,} | length` (jq mode, `collapse: true`):
/// found live while writing this test that `effective_len_checked`'s
/// `census` path never calls `trailing_gap_ok` at all, so *both*
/// evaluators already agree today -- on the wrong answer (`1`, not a
/// raise; confirmed against `/usr/bin/jq` 1.7.1, which raises `Expected
/// another key-value pair`). That's a real, live, CLI-reachable bug in
/// the shared `document.rs` helper both evaluators call, not an eval.rs/
/// eval_generic.rs parity drift -- out of this fix's scope; filed as
/// #2307.
///
/// `has(nan)` pins round-2 review's own finding: the array arm's first
/// draft called `numeric_key_to_array_index` *before* `len_checked`,
/// short-circuiting to `Bool(false)` on `None` (a NaN key in jq mode)
/// without ever running the gap check -- `eval_generic.rs`'s own
/// `eval_has_one_key` calls `len_checked` unconditionally first. Fixed to
/// match that order exactly.
#[test]
fn test_parity_eval_rs_trailing_comma_sibling_gaps_2293() {
    for (json, filter) in [
        (br"[1,]".as_slice(), "length"),
        (br"[1,2,3,]", "length"),
        (br"[1,2,3,]", "keys"),
        (br"[1,2,3,]", "keys_unsorted"),
        (br"[1,2,3,]", "has(0)"),
        (br"[1,2,3,]", "has(nan)"),
        (br#"{"a":1,}"#, r#"has("a")"#),
    ] {
        assert_error_parity(json, filter);
    }
}

/// #2313: `eval.rs`'s `builtin_keys` hardcoded `collapse: false`, no
/// `S: EvalSemantics` parameter to derive a mode-correct value from --
/// disagreeing with `eval_generic.rs`'s own `Builtin::Keys`/
/// `Builtin::KeysUnsorted`, which already collapse via
/// `S::COLLAPSE_DUPLICATE_KEYS` (`true` in jq mode, matching real jq
/// 1.7.1: `{"a":1,"a":2} | keys_unsorted` is `["a"]`, confirmed live).
/// `full_outputs` used `JqSemantics`, so this drift was live for any
/// library consumer of `jq::eval*` on jq-mode duplicate-key input, not
/// merely a defensive gap.
#[test]
fn test_parity_builtin_keys_collapses_duplicate_keys_2313() {
    assert_parity(br#"{"a":1,"a":2}"#, "keys");
    assert_parity(br#"{"a":1,"a":2}"#, "keys_unsorted");
    assert_parity(br#"{"a":1,"a":2,"b":3,"a":4}"#, "keys_unsorted");
}

/// `assert_parity` compares only *successful* outputs: `collect_owned`'s
/// `Error(_) => vec![]` arm swallows the error, so two evaluators that raise
/// different messages -- or one that raises where the other doesn't -- both
/// render as `[]` and compare equal (code review, #1909).
///
/// This asserts on the error side instead, so a filter that must raise is
/// actually pinned to raising, with the same message, on both evaluators.
fn assert_error_parity(json: &[u8], filter: &str) {
    let index = JsonIndex::build(json);
    let expr = parse(filter).expect("parse failed");

    let full_cursor = index.root(json);
    let full: QueryResult<Vec<u64>> = eval::<Vec<u64>, JqSemantics>(&expr, full_cursor);
    let full_err = match full {
        QueryResult::Error(e) => e.message,
        other => panic!(
            "full evaluator did not raise for `{filter}` on `{}`: {:?}",
            String::from_utf8_lossy(json),
            other.collect_owned()
        ),
    };

    let generic_cursor = index.root(json);
    let generic = eval_generic::eval_with_cursor(&expr, generic_cursor);
    let generic_err = match generic {
        eval_generic::GenericResult::Error(e) => e.message,
        other => panic!(
            "generic evaluator did not raise for `{filter}` on `{}`: {other:?}",
            String::from_utf8_lossy(json)
        ),
    };

    assert_eq!(
        full_err,
        generic_err,
        "evaluator error drift for `{filter}` on `{}`",
        String::from_utf8_lossy(json)
    );
}

/// #1909: `Builtin::Path`'s new arm in `eval_generic.rs` calls
/// `eval::builtin_path_on_owned` directly instead of routing through
/// `eval_on_owned`'s serialize + `JsonIndex::build` + re-enter bridge (which
/// landed back in `eval::builtin_path` only for *it* to materialize the same
/// document a second time). Same evaluator, same input tree — so every
/// output must be unchanged, including the multi-output and partial-output
/// shapes where a later path errors after earlier ones already resolved.
#[test]
fn test_parity_path_builtin_bypasses_reindex_bridge_1909() {
    for (json, filter) in [
        (br#"{"a":{"b":1},"c":[1,2,3]}"#.as_slice(), "path(.)"),
        (br#"{"a":{"b":1},"c":[1,2,3]}"#, "path(.a.b)"),
        (br#"{"a":{"b":1},"c":[1,2,3]}"#, "[path(..)]"),
        (br#"{"a":{"b":1},"c":[1,2,3]}"#, "[path(.[])]"),
        (br#"{"a":{"b":1},"c":[1,2,3]}"#, "[path(.a,.c)]"),
        (br#"{"a":{"b":1},"c":[1,2,3]}"#, r#"path(.["a"])"#),
        (br#"{"a":{"b":1},"c":[1,2,3]}"#, "path(.c[0])"),
        (br#"{"a":{"b":1},"c":[1,2,3]}"#, "path(.c[-1])"),
        (br#"{"a":{"b":1},"c":[1,2,3]}"#, "path(.c[0:2])"),
        (br#"{"a":{"b":1},"c":[1,2,3]}"#, "[path(first(.[]))]"),
        (br#"{"a":{"b":1},"c":[1,2,3]}"#, "[path(limit(2;.[]))]"),
        (br#"{"a":{"b":1},"c":[1,2,3]}"#, "[path(select(true))]"),
        (
            br#"{"a":{"b":1},"c":[1,2,3]}"#,
            "[path(if true then .a else .c end)]",
        ),
        (br#"{"a":{"b":1},"c":[1,2,3]}"#, "[path(getpath([\"a\"]))]"),
        // A dynamic index resolved against the document itself.
        (br#"{"k":"a","a":1}"#, "path(.[.k])"),
        (br"null", "path(.a.b)"),
        (br"null", "path(.)"),
        // A repeated key, which the bypass must resolve the same way the
        // bridge did (`IndexMap`-collapsed, since both sides walk the same
        // already-materialized tree).
        (br#"{"a":1,"a":2}"#, "[path(.[])]"),
        // Documents whose numbers straddle the `reindex_bridge_is_identity`
        // guard: a float-free tree takes the bypass, a float-carrying one
        // falls back to the bridge. Both must agree with the full evaluator.
        (br#"{"a":{"b":1}}"#, "[path(..)]"),
        (br#"{"a":{"b":3.5},"c":1e18}"#, "[path(..)]"),
        (br#"{"a":10000000000000000000.0}"#, "[path(..)]"),
        // A genuine `QueryResult::Partial`: `.a.b` resolves and is emitted,
        // then `.c.d` errors. Deliberately *unwrapped* -- `[path(...)]`
        // raises with no output at all, so it would compare `[] == []` here
        // and leave `query_result_to_generic`'s `Partial` arm uncovered on
        // the bypass (code review).
        (br#"{"a":{"b":1},"c":1}"#, "path(.a.b, .c.d)"),
        (br#"{"a":1}"#, r#"try path(.a[0]) catch "E""#),
    ] {
        assert_parity(json, filter);
    }

    // Raising shapes, asserted on the error message rather than through
    // `assert_parity`, which cannot see them (see `assert_error_parity`).
    for (json, filter) in [
        (br#"{"a":1}"#.as_slice(), "path(.a[0])"),
        (br#"{"a":1}"#, "path(.a.b)"),
        (br"[1,2,3]", r#"path(.["a"])"#),
        (br#""s""#, "path(.a)"),
        (br#"{"a":1}"#, "path(.a[0:2])"),
    ] {
        assert_error_parity(json, filter);
    }
}

/// The `Expr::Pipe` half of the same #1909 change: a pipe containing
/// `key`/`parent`/`file_index`/`path` bridges into `eval.rs`'s path-context
/// evaluator directly rather than re-entering through `eval_pipe`'s own
/// identical `needs_path_context` gate. `key`/`parent` are succinctly
/// extensions with no jq oracle, so evaluator parity is the check available.
#[test]
fn test_parity_pipe_path_context_bypasses_reindex_bridge_1909() {
    for (json, filter) in [
        (br#"{"a":{"b":1},"c":[1,2,3]}"#.as_slice(), ".a | path(.b)"),
        (br#"{"a":{"b":1},"c":[1,2,3]}"#, "[.a | path(.)]"),
        (br#"{"a":{"b":1},"c":[1,2,3]}"#, ".a | key"),
        (br#"{"a":{"b":1},"c":[1,2,3]}"#, ".a.b | key"),
        (br#"{"a":{"b":1},"c":[1,2,3]}"#, ".a | parent"),
        (br#"{"a":{"b":1},"c":[1,2,3]}"#, ".a.b | parent(1)"),
        (br#"{"a":{"b":1},"c":[1,2,3]}"#, "[.[] | key]"),
        (br#"{"a":{"b":1},"c":[1,2,3]}"#, "[.c[] | key]"),
        (br#"{"a":{"b":1},"c":[1,2,3]}"#, "[.. | key]"),
        (
            br#"{"a":{"b":1},"c":[1,2,3]}"#,
            r#"[.[] | select(key == "a")]"#,
        ),
        (br#"{"a":{"b":1},"c":[1,2,3]}"#, ".a | . as $x | key"),
        // Float-carrying documents fall back to the bridge rather than the
        // bypass; float-free ones take it. Both must agree.
        (br#"{"a":{"b":3.5}}"#, ".a | parent"),
        (br#"{"a":{"b":1}}"#, ".a | parent"),
        (br#"{"a":10000000000000000000.0}"#, ".a | parent"),
    ] {
        assert_parity(json, filter);
    }
}

/// `key` at the document root: the two evaluators now *diverge*, and the
/// generic one is the side that matches the reference.
///
/// Real yq prints nothing for `key` at the root (#2421, captured live from
/// v4.53.3), and #2416 phase 2's cursor walk follows it, so the CLI route
/// yields no output. `eval.rs`'s eager path-context evaluator still answers
/// `null` -- its `Key` handler's comment says "(yq behavior)", which is
/// false against the pinned binary -- and is #2421's remaining half until
/// that function retires. Pinned in both directions so the migration that
/// deletes the eager arm has to move this row into the parity table above,
/// and nothing can quietly re-teach the walk to print `null`.
#[test]
fn test_parity_key_at_root_diverges_2421() {
    // Since the library entry point routes every path-context query to the
    // generic evaluator (spine 2416, phase 3), both sides answer as the
    // reference does -- nothing -- and the eager evaluator's `null`/`{}` is
    // no longer reachable from either. The rows stay as the pin that this
    // does not regress.
    for (json, filter) in [
        (br"null".as_slice(), ". | key"),
        (br#"{"a":1}"#, "parent"),
        (br#"{"a":1}"#, "key"),
    ] {
        let full = full_outputs(json, filter);
        let generic = generic_outputs(json, filter);
        assert!(full.is_empty(), "eval.rs moved for `{filter}`: {full:?}");
        assert!(
            generic.is_empty(),
            "eval_generic.rs moved for `{filter}`: {generic:?}"
        );
    }
}

/// #1519: a `?//`-alternatives bind under a short-circuiting consumer re-runs
/// once per alternative, because jq's `?//` catches *any* escaping break --
/// including the one its own `builtin.jq` macros raise -- and reads it as
/// "this alternative failed, try the next".
///
/// succinctly's builtins are native Rust and signal satisfaction as
/// `Demand::Stop`/`Flow::Stopped` instead, so the retry lives in
/// `each_pattern_alternatives` (`src/jq/eval.rs`) and
/// `each_pattern_alternatives_generic` (`src/jq/eval_generic.rs`) -- two
/// independent copies of the same loop, plus separate copies of the reshaped
/// terminal sinks (`each_take_first`/`each_take_first_generic`,
/// `each_take_nth`/`nth_with_n_generic`). These rows exist so neither copy can
/// be changed without the other.
///
/// Note `first(...)`/`last(...)` are additionally intercepted by
/// `eval_generic.rs`'s own native routing before `eval.rs`'s `eval_each` ever
/// sees them (#1461), so the `first` rows here exercise a genuinely different
/// code path in each evaluator rather than the same one twice.
#[test]
fn test_parity_pattern_alternatives_under_short_circuit_1519() {
    let null = b"null";
    for filter in [
        // One answer per alternative, per consumer.
        "[isempty(1 as $x ?// $y | 5)]",
        "[isempty(1 as $x ?// $y ?// $z | 5)]",
        "[first(1 as $x ?// $y | 5, 6)]",
        "[limit(1; 1 as $x ?// $y | 5)]",
        "[nth(1; 1 as $x ?// $y | 5, 6)]",
        "[any(1 as $x ?// $y | true; .)]",
        "[all(1 as $x ?// $y | false; .)]",
        // The counter-continuation and next-pattern rules.
        "[limit(2; 1 as $x ?// $y | 5,6)]",
        r#"[first([1] as [$x] ?// $x | ($x|tostring), "z")]"#,
        r#"[first((1,2) as $x ?// $y | $x, "z")]"#,
        r#"[first(1 as $a ?// $b | (2 as $c ?// $d | 9), "z")]"#,
        // A retried alternative that exhausts still reaches the builtin's own
        // exhaustion answer.
        r#"[isempty([1] as [$x] ?// $x | if ($x|type)=="number" then 9 else empty end)]"#,
        r"[any([1] as [$x] ?// $x | ($x == 1); .)]",
        // Shapes that must NOT retry, so a too-eager fix drifts one evaluator.
        "[isempty(1 as $x ?// $y | empty)]",
        "[first(1 as $x ?// $y | empty)]",
        "[limit(0; 1 as $x ?// $y | 5)]",
        "[1 as $x ?// $y | 5]",
        "[label $out | (1 as $x ?// $y | 5), break $out]",
        "[1 as $x ?// $y | label $in | (5, break $in)]",
    ] {
        assert_parity(null, filter);
    }
}

/// #1519, cursor-backed twin of the row above: the generic evaluator keeps a
/// single `first(...)` item cursor-backed (`generic_item_to_result`) but has to
/// route a multi-item `?//` retry through the batch adapter instead, so the two
/// spellings are genuinely different code. Run against a real document rather
/// than `null` so the cursor path is actually taken.
#[test]
fn test_parity_pattern_alternatives_short_circuit_over_document_1519() {
    let json = br#"{"a": [1, 2], "b": {"k": "v"}}"#;
    for filter in [
        "[first(.a as [$p] ?// $p | $p, 9)]",
        "[first(.b as {k:$p} ?// $p | $p, 9)]",
        "[first(.a as {k:$p} ?// [$p,$q] | [$p,$q], 9)]",
        "[isempty(.a as [$p] ?// $p | $p)]",
        "[limit(1; .a as [$p] ?// $p | $p)]",
        "[nth(1; .a as [$p] ?// $p | $p, 7)]",
        "[.a as [$p] ?// $p | $p]",
    ] {
        assert_parity(json, filter);
    }
}

/// #2312: `eval.rs`'s `get_element_at_index`/`count_elements` (negative-index
/// resolution, `.foo[-1]`) had the same missing #1677/#2261 trailing-comma
/// gap check `eval_generic.rs`'s own `Expr::Index` arm already closed
/// (#2261) -- unreachable from either shipped CLI today (same reachability
/// analysis as #2293/#2311's sibling fixes in this file: `succinctly jq`
/// never calls into `eval.rs`, and `succinctly yq`'s one call site always
/// hands it an already-materialized value), but a real gap for a library
/// consumer of `jq::eval*` building a cursor directly from raw,
/// unvalidated bytes. Both evaluators already agree (`eval_generic.rs` was
/// already correct), so `assert_error_parity` pins the fix.
#[test]
fn test_parity_negative_index_trailing_comma_2312() {
    assert_error_parity(br"[1,2,3,]", ".[-1]");
}

/// #2312 review: a first draft of this fix only checked the negative-index
/// branch, leaving a *positive* index (`.[0]`) on the same malformed array
/// silently unchecked -- found live by code review. Pins both directions,
/// matching `eval_generic.rs`'s own `Expr::Index` arm, which already
/// checks unconditionally regardless of sign.
#[test]
fn test_parity_positive_index_trailing_comma_2312() {
    assert_error_parity(br"[1,2,3,]", ".[0]");
}

/// #2163: `foreach`/`reduce`'s INIT-fork re-entry divergence (real jq
/// re-evaluates SOURCE against a synthetic `null` ambient document for every
/// INIT fork after the first; succinctly re-evaluates SOURCE identically for
/// every fork instead — see `docs/compliance/jq/limitations.md`'s "INIT-fork
/// re-entry" entry and `eval.rs`'s
/// `test_foreach_reduce_init_fork_source_reads_ambient_input_2163`). Both
/// front ends here (`full_outputs`/`generic_outputs`) ultimately dispatch to
/// the same shared `eval_reduce_with_values`/`eval_foreach_with_values` core,
/// so this does not double-verify two independently-coded evaluators — it
/// pins that each front end's own input-collection step still feeds that
/// shared core the same way, so a future change to either front end's
/// dispatch can't quietly stop reaching it.
#[test]
fn test_parity_foreach_reduce_init_fork_source_reads_ambient_input_2163() {
    for filter in [
        "[foreach (.a) as $k ((0,.c); $k)]",
        "[reduce (.a) as $k ((0,.c); $k)]",
    ] {
        assert_parity(br#"{"a":1,"c":2}"#, filter);
    }
}

/// #2440: both front ends must evaluate INIT *before* the first SOURCE pull,
/// so INIT's own error outranks a SOURCE that errors on its first output.
/// Captured from /usr/bin/jq 1.7.1 on `{"a":1}`: `foreach (1, error("in"))
/// as $x (error("init"); .)` reports `init`, and `reduce`'s twin does too;
/// succinctly reported `in` from both front ends before the fix.
///
/// The two front ends share `eval_foreach_with_values`/
/// `eval_reduce_with_values` (see the #2163 test above for what that does and
/// does not prove), but the ordering being pinned here is *each front end's
/// own*: the fix moved the INIT evaluation out of the callers' hand-rolled
/// prologues and into a `source` closure the shared core decides whether to
/// call at all, so a front end that reverted to evaluating SOURCE eagerly
/// would fail here while still compiling.
#[test]
fn test_parity_fold_init_evaluated_before_source_2440() {
    for filter in [
        r#"foreach (1, error("in")) as $x (error("init"); .)"#,
        r#"reduce (1, error("in")) as $x (error("init"); .)"#,
        r#"foreach error("in") as $x (error("init"); .)"#,
        r#"reduce error("in") as $x (error("init"); .)"#,
    ] {
        assert_error_parity(br#"{"a":1}"#, filter);
        // Parity alone would be satisfied by both front ends reporting
        // `in`, which is exactly the pre-fix behaviour -- so name the
        // message jq gives.
        let index = JsonIndex::build(br#"{"a":1}"#);
        let expr = parse(filter).expect("parse failed");
        let full: QueryResult<Vec<u64>> =
            eval::<Vec<u64>, JqSemantics>(&expr, index.root(br#"{"a":1}"#));
        match full {
            QueryResult::Error(e) => assert_eq!(e.message, "init", "`{filter}`"),
            other => panic!("`{filter}` did not raise: {:?}", other.collect_owned()),
        }
    }
    // A zero-output INIT never pulls SOURCE at all, in either front end:
    // jq 1.7.1's `[foreach error("in") as $x (empty; .)]` is `[]`, exit 0.
    for filter in [
        r#"[foreach error("in") as $x (empty; .)]"#,
        r#"[reduce error("in") as $x (empty; .)]"#,
        r#"[foreach (1, error("in")) as $x (empty; .)]"#,
        r#"[reduce (1, error("in")) as $x (empty; .)]"#,
    ] {
        assert_parity(br#"{"a":1}"#, filter);
        assert_eq!(
            as_strs(&generic_outputs(br#"{"a":1}"#, filter)),
            ["[]"],
            "`{filter}` must not reach SOURCE at all"
        );
    }
}

// ---------------------------------------------------------------------------
// #2416 phase 0, axis three: the cursor walk vs the materializing bridge.
//
// The two axes above compare `eval.rs` with `eval_generic.rs`. Inside
// `eval_generic.rs` there is a *third* pair that has to agree and had nothing
// asserting it: a pipe stage that `needs_path_context` is answered either by
// `try_path_context_cursor_walk` (#2061, straight from cursors) or, for any
// shape the walk does not model, by the materializing bridge
// (`to_owned_with_cursor` + `eval::eval_pipe_with_path_context`).
//
// #2416 migrates the eager evaluator's arms into the walk one at a time, and
// the fallback is *meant* to be output-identical — which is exactly why
// nothing in the suite could see #2061's own lost optimization; patch coverage
// caught it instead. `PathContextRoute` makes the two routes drivable from
// here so "identical" becomes an assertion.
//
// The walk is genuinely exercised by these cases, not merely offered: with the
// route site instrumented, the two shape tables below reach 332 path-context
// evaluations per route, and the walk answers 246 of them. The other 86 are
// shapes `path_context_is_cursor_walkable` still refuses -- the migration's
// own backlog. A test that only ever reached the bridge would pass while
// proving nothing (#1599's lesson).
// ---------------------------------------------------------------------------

/// Outputs of the generic evaluator with the path-context route pinned.
fn route_outputs<S: EvalSemantics>(
    json: &[u8],
    filter: &str,
    route: eval_generic::PathContextRoute,
) -> Vec<String> {
    let index = JsonIndex::build(json);
    let cursor = index.root(json);
    let expr = parse(filter).expect("parse failed");
    eval_generic::eval_with_cursor_using_route::<S, _>(&expr, cursor, route)
        .collect_owned()
        .expect("materializes")
        .iter()
        .map(succinctly::jq::OwnedValue::to_json)
        .collect()
}

/// Assert the cursor walk and the materializing bridge agree, in both modes.
///
/// Both modes on purpose: `key`/`parent`/`path` are yq builtins that
/// `succinctly jq` also accepts as an extension, and the walk is shared, so a
/// mode-specific regression in one route would otherwise only be visible in
/// half the surface (#2226's lesson about a shared site with a per-mode gate).
fn assert_walk_bridge_parity(json: &[u8], filter: &str) {
    for (mode, walk, bridge) in [
        (
            "jq",
            route_outputs::<JqSemantics>(
                json,
                filter,
                eval_generic::PathContextRoute::WalkThenBridge,
            ),
            route_outputs::<JqSemantics>(json, filter, eval_generic::PathContextRoute::BridgeOnly),
        ),
        (
            "yq",
            route_outputs::<YqSemantics>(
                json,
                filter,
                eval_generic::PathContextRoute::WalkThenBridge,
            ),
            route_outputs::<YqSemantics>(json, filter, eval_generic::PathContextRoute::BridgeOnly),
        ),
    ] {
        assert_eq!(
            walk,
            bridge,
            "path-context route drift ({mode} mode) for `{filter}` on `{}`:\n  \
             walk   = {walk:?}\n  bridge = {bridge:?}",
            String::from_utf8_lossy(json)
        );
    }
}

/// Every path-context shape the walk models today, plus a ring of shapes just
/// outside it, over documents that exercise object keys, array indices,
/// nesting and duplicate mapping keys.
///
/// A shape the walk refuses is not a failure here — the bridge answers it and
/// both routes agree trivially. That is the point of the one-directional gate
/// #2416 phase 2 relies on: an un-migrated arm is a missed optimization, never
/// a wrong answer. This test is what keeps that claim true as arms migrate.
#[test]
fn test_walk_vs_bridge_path_context_parity_2416() {
    let docs: [&[u8]; 5] = [
        br#"{"a":{"b":1,"c":2},"d":[10,20]}"#,
        br#"{"a":{"b":{"e":1}},"d":[[1],[2]]}"#,
        br#"{"a":1,"a":2,"d":[10,20]}"#,
        br#"{"a":{"b":1,"b":2},"d":[10,20]}"#,
        br#"[{"k":1},{"k":2}]"#,
    ];
    for doc in docs {
        for filter in [
            // Leaves at the root, where the walk currently defers.
            "key",
            "parent",
            "path",
            // Navigation prefixes the walk models.
            ".a | key",
            ".a | path",
            ".a | parent",
            ".a[] | key",
            ".a[] | path",
            ".a[] | parent",
            ".d[] | key",
            ".d[] | path",
            ".[] | key",
            "[.[] | key]",
            "[.a[] | key]",
            "[.a[] | path]",
            ".a.b | path",
            ".a.b | parent | key",
            ".a[] | parent(1) | key",
            ".a[] | parent(0) | key",
            "[.a[] | (key, path)]",
            // Stages past the leaf, which fold through the ordinary evaluator.
            ".a | key | tostring",
            "[.a[] | key] | length",
            "[.d[] | path] | flatten",
            // Shapes outside the walkable set: the bridge answers, and both
            // routes must still agree.
            ".a[] | select(true) | key",
            "{\"k\": key}",
            "[.a[] | {\"k\": key}]",
            "first(.a[] | key)",
            "[limit(1; .a[] | key)]",
            "[foreach (.a[] | key) as $k ([]; . + [$k])] | last",
            "[reduce (.a[] | key) as $k ([]; . + [$k])]",
            "try (.a[] | key) catch \"e\"",
            "(.a[] | key)?",
            "label $o | (.a[] | key, break $o)",
            ".a[] | key as $k | $k",
            "[.. | path]",
            // #2416 step 2: heads that can land on an absent node. Some of
            // these miss on some of the documents above and not on others,
            // which is the point -- the absent route walks the head as
            // positions and has to answer both kinds from the same pipe.
            ".a.zz | key",
            ".a.zz | path",
            ".a.zz | parent",
            ".a.zz.yy | key",
            ".a.zz.yy | path",
            ".a.zz | select(key == \"zz\")",
            ".a.zz | select(true) | key",
            ".a.zz | [key, path]",
            ".a.zz | (key, path)",
            ".a.zz | key | tostring",
            ".a.zz | file_index",
            ".a.zz | if key == \"zz\" then 1 else 2 end",
            ".d[9] | key",
            ".d[9] | path",
            ".d[9] | select(key == 9)",
            ".zz.yy | path",
            ".a.zz | limit(1; key)",
            ".a.zz | first(key)",
            ".a.zz | try (key) catch \"e\"",
            ".a.zz.yy | [path, parent]",
        ] {
            assert_walk_bridge_parity(doc, filter);
        }
    }
}

/// Float-carrying documents through both routes.
///
/// Until #2416 phase 2, `try_path_context_cursor_walk` deferred to the bridge
/// whenever an emitted value failed `reindex_bridge_is_identity`, so the walk
/// could never disagree with the bridge's `to_json_for_reindex` re-spelling
/// of a bare float. Phase 2 deleted that fallback: the walk emits the cursor
/// itself, and the CLI streams a whole-walk pipe from the cursor's own text
/// (`m2_gate.rs`), which is where the document's spelling now survives --
/// `test_path_context_bypass_keeps_yq_float_fidelity_1909` in
/// `tests/yq_cli_tests.rs` pins that. The re-spelling was the bridge's, not
/// the reference's: yq v4.53.3 preserves the source spelling in every probed
/// case (#2419).
///
/// Here both routes are read back through `collect_owned`, which materializes
/// a cursor with the same value-level conversion the bridge used, so the two
/// still agree on these documents; the assertion is that phase 2 changed the
/// *route*, not the value the library hands back.
#[test]
fn test_walk_vs_bridge_float_respelling_guard_2419() {
    for doc in [
        br#"{"a":{"b":1.50,"c":1e3},"d":[1e20,20]}"#.as_slice(),
        br#"{"a":{"b":100000000000000000000,"c":2},"d":[10,20]}"#.as_slice(),
        br#"{"a":{"b":10000000000000000000.0},"d":[1]}"#.as_slice(),
    ] {
        for filter in [
            ".a | key",
            ".a[] | key",
            ".a[] | path",
            ".a.b | parent",
            ".a | .b | parent | .b | tostring",
            "[.a[] | parent]",
            "[.d[] | parent]",
        ] {
            assert_walk_bridge_parity(doc, filter);
        }
    }
}

// ---------------------------------------------------------------------------
// #2374 item 3: a table-driven parity test over the target-Partial-prefix
// yq-gate family, landed here as part of #2416 phase 0.
//
// The pattern is `if S::TAG == EvalTag::Yq { discard } else { keep the
// target's own already-produced prefix and fold it in }`, hand-copied at 8+
// call sites across `eval.rs` and `eval_generic.rs` with no shared definition
// and no test that the copies agree. It has already shipped wrong once:
// `eval_generic::eval_slice_bound` had no yq gate at all until #2351 caught it
// as a live bug.
//
// The table below enumerates the indexing-like constructs #2374 names and
// pins every one against output captured live on 2026-09-05 from the pinned
// oracles (/usr/bin/jq 1.7.1 and Homebrew yq v4.53.3) on this exact document.
// A ninth construct added without the gate then fails one shared test instead
// of shipping silently.
//
// `(.a, error("e"))` is the target: it produces one value and then raises, so
// the whole family sees a `Partial`. jq streams the prefix and exits 5; yq
// discards it and exits 1.
// ---------------------------------------------------------------------------

/// `(outputs of eval.rs, outputs of eval_generic.rs)` for one filter under `S`.
///
/// Kept separate rather than asserted equal inside the helper: the two
/// path-context sites in this family are a *known* divergence, and a helper
/// that collapsed the pair could not express it.
fn both_evaluator_outputs<S: EvalSemantics>(
    json: &[u8],
    filter: &str,
) -> (Vec<String>, Vec<String>) {
    let index = JsonIndex::build(json);
    let expr = parse(filter).expect("parse failed");

    let full: QueryResult<Vec<u64>> = eval::<Vec<u64>, S>(&expr, index.root(json));
    let full = full
        .collect_owned()
        .iter()
        .map(succinctly::jq::OwnedValue::to_json)
        .collect();

    let generic = eval_generic::eval_with_cursor_using::<S, _>(&expr, index.root(json))
        .collect_owned()
        .expect("materializes")
        .iter()
        .map(succinctly::jq::OwnedValue::to_json)
        .collect();

    (full, generic)
}

/// The document every row below was captured against.
const YQ_GATE_DOC: &[u8] = br#"{"a":{"b":1,"c":2},"d":[10,20]}"#;

#[test]
fn test_target_partial_prefix_yq_gate_family_2374() {
    // (construct, filter, jq oracle outputs, yq oracle outputs)
    //
    // Every `yq` column is empty and that is the whole point: real yq drops
    // the prefix the target already produced. A site that forgets the gate
    // shows up here as a non-empty yq column.
    let rows: &[(&str, &str, &[&str], &[&str])] = &[
        ("index/field", r#"(.a, error("e")).b"#, &["1"], &[]),
        ("index/dynamic", r#"(.a, error("e")) | .["b"]"#, &["1"], &[]),
        ("index/number", r#"(.d, error("e"))[1]"#, &["20"], &[]),
        (
            "index/negative",
            r#"(.d, error("e")) | .[-1]"#,
            &["20"],
            &[],
        ),
        ("slice/both", r#"(.d, error("e"))[0:1]"#, &["[10]"], &[]),
        (
            "slice/open-end",
            r#"(.d, error("e")) | .[1:]"#,
            &["[20]"],
            &[],
        ),
        (
            "slice/open-start",
            r#"(.d, error("e")) | .[:1]"#,
            &["[10]"],
            &[],
        ),
        ("iterate", r#"(.a, error("e"))[]"#, &["1", "2"], &[]),
        ("keys", r#"(.a, error("e")) | keys"#, &[r#"["b","c"]"#], &[]),
        (
            "keys_unsorted",
            r#"(.a, error("e")) | keys_unsorted"#,
            &[r#"["b","c"]"#],
            &[],
        ),
        ("has", r#"(.a, error("e")) | has("b")"#, &["true"], &[]),
        (
            "to_entries",
            r#"(.a, error("e")) | to_entries"#,
            &[r#"[{"key":"b","value":1},{"key":"c","value":2}]"#],
            &[],
        ),
        ("length", r#"(.a, error("e")) | length"#, &["2"], &[]),
        ("del", r#"(.a, error("e")) | del(.b)"#, &[r#"{"c":2}"#], &[]),
    ];

    for (construct, filter, jq_expected, yq_expected) in rows {
        for (mode, expected, (full, generic)) in [
            (
                "jq",
                *jq_expected,
                both_evaluator_outputs::<JqSemantics>(YQ_GATE_DOC, filter),
            ),
            (
                "yq",
                *yq_expected,
                both_evaluator_outputs::<YqSemantics>(YQ_GATE_DOC, filter),
            ),
        ] {
            assert_eq!(
                full, expected,
                "[{construct}] eval.rs disagrees with the {mode} oracle for `{filter}`"
            );
            assert_eq!(
                generic, expected,
                "[{construct}] eval_generic.rs disagrees with the {mode} oracle for `{filter}`"
            );
        }
    }
}

/// The two path-context members of the same family — and they are missing the
/// gate, which is exactly the recurrence #2374 predicted.
///
/// `eval_index_expr_with_path_context` and `eval_slice_expr_with_path_context`
/// keep the target's already-produced prefix in yq mode, where their
/// plain-read siblings (pinned as passing in the table above) discard it. Real
/// yq emits nothing for every row here; succinctly emits the value.
///
/// Captured live on 2026-09-05 from yq v4.53.3. Pinned as a divergence rather
/// than fixed: these are arms of `eval_stage_with_path_context`, the function
/// #2416 exists to retire, so the fix belongs to the migration that deletes
/// them — and #2416's own hygiene rule says a PR adding an arm there has to
/// argue why it is not a migration.
///
/// jq mode has no oracle for these at all: jq 1.7.1 rejects `key`, `path` and
/// `parent` at compile time (`key/0 is not defined`, exit 3), so they are
/// succinctly extensions there and only yq mode can be checked.
#[test]
fn test_target_partial_prefix_path_context_sites_miss_the_yq_gate_2374() {
    // (construct, filter, succinctly's current yq-mode output)
    let rows: &[(&str, &str, &[&str])] = &[
        (
            "index/field + key",
            r#"(.a, error("e")).b | key"#,
            &[r#""b""#],
        ),
        (
            "index/field + path",
            r#"(.a, error("e")).b | path"#,
            &[r#"["a","b"]"#],
        ),
        (
            "index/field + parent",
            r#"(.a, error("e")).b | parent"#,
            &[r#"{"b":1,"c":2}"#],
        ),
        ("index/number + key", r#"(.d, error("e"))[0] | key"#, &["0"]),
        (
            "index/number + path",
            r#"(.d, error("e"))[0] | path"#,
            &[r#"["d",0]"#],
        ),
        (
            "slice + path",
            r#"(.d, error("e"))[0:1] | path"#,
            &[r#"["d",{"start":0,"end":1}]"#],
        ),
    ];

    // Real yq emits nothing for every row: the target's prefix is discarded
    // before the index/slice ever runs.
    let oracle: Vec<String> = Vec::new();

    for (construct, filter, current) in rows {
        let (full, generic) = both_evaluator_outputs::<YqSemantics>(YQ_GATE_DOC, filter);
        assert_eq!(
            full, *current,
            "[{construct}] eval.rs moved for `{filter}` — if this is the #2416 \
             migration landing, drop the row and add it to the passing table above"
        );
        assert_eq!(
            generic, *current,
            "[{construct}] eval_generic.rs moved for `{filter}`"
        );
        assert_ne!(
            full, oracle,
            "[{construct}] `{filter}` now matches real yq — nice; move the row \
             into test_target_partial_prefix_yq_gate_family_2374"
        );
    }
}
