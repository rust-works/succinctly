//! Computed keys in index brackets (#360), pinned on *both* evaluators.
//!
//! `.[K]` for a non-constant `K` is implemented twice — once in the full
//! evaluator (`src/jq/eval.rs`, the library `jq::eval` entry point) and once in
//! the generic evaluator (`src/jq/eval_generic.rs`, which the `jq`/`yq` CLIs
//! use). The two share no code on this path, so every case here runs through
//! both and asserts they agree; see `jq_evaluator_parity_tests.rs` for why that
//! drift is worth a test of its own.
//!
//! The golden fixtures in `tests/data/jq-golden/` cover the happy paths through
//! stdout comparison. This file covers what stdout cannot see: the *error*
//! messages, the empty-stream cases, and the shape of the result when a target
//! or key stream is owned, borrowed, multi-output or absent.
//!
//! Every expectation below was captured from real jq 1.7.1 first, and the
//! handful that deliberately differ say so at the case.

use succinctly::jq::eval_generic;
use succinctly::jq::{eval, parse, JqSemantics, OwnedValue, QueryResult};
use succinctly::json::JsonIndex;

/// What an evaluator did: a stream of JSON-rendered outputs, or the error it
/// raised. `collect_owned` maps an error to an empty stream, so the two must be
/// distinguished before rendering or every error case would silently pass.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Values(Vec<String>),
    Error(String),
}

impl Outcome {
    fn values(vs: &[&str]) -> Self {
        Self::Values(vs.iter().map(|s| (*s).to_string()).collect())
    }

    fn error(message: &str) -> Self {
        Self::Error(message.to_string())
    }
}

/// Run `filter` through the full evaluator (`src/jq/eval.rs`).
fn full(json: &[u8], filter: &str) -> Outcome {
    let index = JsonIndex::build(json);
    let cursor = index.root(json);
    let expr = parse(filter).expect("parse failed");
    let result: QueryResult<Vec<u64>> = eval::<Vec<u64>, JqSemantics>(&expr, cursor);
    match result {
        QueryResult::Error(e) => Outcome::Error(e.message),
        other => Outcome::Values(
            other
                .collect_owned()
                .iter()
                .map(OwnedValue::to_json)
                .collect(),
        ),
    }
}

/// Run `filter` through the generic evaluator (`src/jq/eval_generic.rs`).
fn generic(json: &[u8], filter: &str) -> Outcome {
    let index = JsonIndex::build(json);
    let cursor = index.root(json);
    let expr = parse(filter).expect("parse failed");
    match eval_generic::eval_with_cursor(&expr, cursor) {
        eval_generic::GenericResult::Error(e) => Outcome::Error(e.message),
        other => Outcome::Values(
            other
                .collect_owned()
                .iter()
                .map(OwnedValue::to_json)
                .collect(),
        ),
    }
}

/// Assert both evaluators produce `expected` for `filter` on `json`.
fn check(json: &str, filter: &str, expected: Outcome) {
    let full = full(json.as_bytes(), filter);
    let generic = generic(json.as_bytes(), filter);
    assert_eq!(full, expected, "full evaluator: `{filter}` on `{json}`");
    assert_eq!(
        generic, expected,
        "generic evaluator: `{filter}` on `{json}`"
    );
}

// =============================================================================
// Reading through a computed key
// =============================================================================

#[test]
fn test_string_key_reads_object() {
    // A key navigated out of the document arrives borrowed, one output.
    check(r#"{"k":"a","a":1}"#, ".[.k]", Outcome::values(&["1"]));
    // Missing key is null, exactly as `.missing` is.
    check(r#"{"k":"nope","a":1}"#, ".[.k]", Outcome::values(&["null"]));
    // A multi-output key arrives borrowed too, and fans out.
    check(
        r#"{"ks":["a","b"],"a":1,"b":2}"#,
        ".[.ks[]]",
        Outcome::values(&["1", "2"]),
    );
}

#[test]
fn test_numeric_key_reads_array() {
    check("[10,20,30]", ".[(-1,0)]", Outcome::values(&["30", "10"]));
    // Out of bounds, either end, is null rather than an error (#307).
    check("[1,2,3]", ".[(5,-9)]", Outcome::values(&["null", "null"]));
}

#[test]
fn test_null_input_passes_through_for_a_valid_key_kind() {
    // `null | .[K]` is null for a string or number key — the same rule `.foo`
    // and `.[0]` follow — but a key of any other kind still errors (below).
    check(
        "null",
        r#".[("a","b")]"#,
        Outcome::values(&["null", "null"]),
    );
    check("null", ".[(0,1)]", Outcome::values(&["null", "null"]));
}

#[test]
fn test_wrong_container_for_key_kind_errors() {
    // The key kind is dispatched before the container is inspected, so the
    // message names both — jq's wording, not a generic "expected object, got".
    check(
        "1",
        r#".[("a","b")]"#,
        Outcome::error(r#"Cannot index number with string "a""#),
    );
    check(
        r#"{"a":1}"#,
        ".[(0,1)]",
        Outcome::error("Cannot index object with number"),
    );
}

#[test]
fn test_key_of_an_unindexable_kind_errors_even_on_a_container() {
    check(
        r#"{"a":1}"#,
        ".[(null,true)]",
        Outcome::error("Cannot index object with null"),
    );
}

#[test]
fn test_optional_suppresses_the_indexing_error_only() {
    // `?` covers the indexing, so a bad container or a bad key kind yields no
    // output instead of an error...
    check("1", r#".[("a","b")]?"#, Outcome::values(&[]));
    check(r#"{"a":1}"#, ".[(0,1)]?", Outcome::values(&[]));
    check(r#"{"a":1}"#, ".[(null,true)]?", Outcome::values(&[]));
    // ...but it does not cover the key expression: jq's `.[error("boom")]?`
    // still raises `boom`, and so does this.
    check("null", r#".[error("boom")]"#, Outcome::error("boom"));
}

#[test]
fn test_empty_key_stream_short_circuits() {
    // An empty key stream must not evaluate the target at all — jq compiles
    // `E[K]` as `K as $k | E | .[$k]`, so `(error("boom"))[empty]` raises
    // nothing.
    check("null", ".[empty]", Outcome::values(&[]));
    check("null", r#"(error("boom"))[empty]"#, Outcome::values(&[]));
    // A key expression that iterates an empty array is empty the same way.
    check(r#"{"ks":[]}"#, ".[.ks[]]", Outcome::values(&[]));
}

#[test]
fn test_key_is_outer_and_target_inner() {
    // Every target is walked for the first key before the second key starts:
    // 1, 2, null, null — not 1, null, 2, null.
    check(
        r#"[{"a":1},{"a":2}]"#,
        r#".[][("a","b")]"#,
        Outcome::values(&["1", "2", "null", "null"]),
    );
}

#[test]
fn test_target_stream_edge_cases() {
    // No target: no output, and no error.
    check("null", r#"empty[("a","b")]"#, Outcome::values(&[]));
    // A target that errors propagates, once the key stream is known non-empty.
    check(
        "null",
        r#"(error("boom"))[("a","b")]"#,
        Outcome::error("boom"),
    );
    // `break` unwinds through either half.
    check("null", "label $out | .[break $out]", Outcome::values(&[]));
    check(
        "null",
        r#"label $out | (break $out)[("a","b")]"#,
        Outcome::values(&[]),
    );
}

#[test]
fn test_mixed_borrowed_and_owned_results() {
    // A found key yields a borrowed node; a missing one yields an owned null.
    // Both orders have to produce the same stream, which is only true if the
    // accumulator converts what it has already collected when the first owned
    // result shows up.
    check(
        r#"{"a":1}"#,
        r#".[("missing","a")]"#,
        Outcome::values(&["null", "1"]),
    );
    check(
        r#"{"a":1}"#,
        r#".[("a","missing")]"#,
        Outcome::values(&["1", "null"]),
    );
}

// =============================================================================
// Owned targets — a left side that computed rather than navigated
// =============================================================================
//
// `getpath(...)` returns a materialized value, so these exercise the owned
// branch of the same rules. A collection literal would be the obvious way to
// write them; it does not parse here — see
// `test_collection_literal_as_a_target_is_a_parse_error`.

#[test]
fn test_owned_target_follows_the_same_rules() {
    let doc = r#"{"a":[10,20,30]}"#;
    check(
        doc,
        r#"getpath(["a"])[(-1,0)]"#,
        Outcome::values(&["30", "10"]),
    );
    // Two outputs from an owned target stay a stream, rather than collapsing.
    check(
        doc,
        r#"getpath(["a"])[(0,1)]"#,
        Outcome::values(&["10", "20"]),
    );
    // NaN is a number, so the container check still applies, but there is no
    // element: null, not an error and not element zero.
    check(
        doc,
        r#"getpath(["a"])[(nan,0)]"#,
        Outcome::values(&["null", "10"]),
    );
    check(
        doc,
        r#"getpath(["a"])[(5,-9)]"#,
        Outcome::values(&["null", "null"]),
    );
    // A single output stays a single output rather than becoming a one-element
    // stream.
    check(doc, r#"getpath(["a"])[(1+1)]"#, Outcome::values(&["30"]));
}

#[test]
fn test_owned_null_target_passes_through() {
    check(
        "{}",
        r#"getpath(["m"])[("x","y")]"#,
        Outcome::values(&["null", "null"]),
    );
    check(
        "{}",
        r#"getpath(["m"])[(0,1)]"#,
        Outcome::values(&["null", "null"]),
    );
}

#[test]
fn test_owned_target_of_the_wrong_kind_errors() {
    check(
        r#"{"a":1}"#,
        "(.a|tostring)[(null,true)]",
        Outcome::error("Cannot index string with null"),
    );
    check(
        r#"{"a":1}"#,
        "(.a|tostring)[(null,true)]?",
        Outcome::values(&[]),
    );
    // `?` on an owned *array* target suppresses the same way.
    check(
        r#"{"a":[10,20,30]}"#,
        r#"getpath(["a"])[(null,true)]?"#,
        Outcome::values(&[]),
    );
}

#[test]
fn test_collection_literal_as_a_target_is_a_parse_error() {
    // DIVERGENCE from jq 1.7.1, and independent of #360 — a constant key is
    // rejected just the same. jq reads `[1,2,3][0]` as `1` and `{"a":1}["a"]`
    // as `1`; here the postfix chain never attaches to a collection literal, so
    // both are refused before evaluation. This is why every owned target above
    // is spelled `getpath(...)`.
    for src in ["[1,2,3][0]", r#"{"a":1}["a"]"#, "[1,2,3][(0,2)]"] {
        assert!(parse(src).is_err(), "`{src}` unexpectedly parsed");
    }
}

// =============================================================================
// Path contexts: assignment, update, del, path()
// =============================================================================

#[test]
fn test_assignment_applies_every_resolved_key() {
    // Keys resolve against the *original* document, so the second key here is
    // read from the untouched `.x` rather than from what the first write left.
    check(
        r#"{"a":"x","x":"y"}"#,
        ".[.a, .x] = 1",
        Outcome::values(&[r#"{"a":"x","x":1,"y":1}"#]),
    );
    check(
        r#"{"a":1}"#,
        r#".[("a","b")]? = 5"#,
        Outcome::values(&[r#"{"a":5,"b":5}"#]),
    );
}

#[test]
fn test_assignment_key_must_denote_a_component() {
    // Reading `.[null]` and writing through it differ: the read errors because
    // null is not a key kind, and so does the write, with the same message.
    check(
        r#"{"a":1}"#,
        ".[null] = 5",
        Outcome::error("Cannot index object with null"),
    );
    check(
        r#"{"a":1}"#,
        ".[null] |= 5",
        Outcome::error("Cannot index object with null"),
    );
    // A key that errors while being computed propagates out of the pre-pass.
    check(
        r#"{"a":1}"#,
        r#".[error("boom")] = 5"#,
        Outcome::error("boom"),
    );
    // DIVERGENCE from jq 1.7.1: a `break` in a key is silent there, unwinding
    // to the label and emitting nothing. The path pre-pass resolves keys with a
    // plain `Result`, which has no way to carry a label out, so it reports the
    // unwind rather than mapping it to "no paths" — the alternative would make
    // `break` in a key indistinguishable from an empty key stream, which is a
    // *successful* no-op assignment.
    check(
        r#"{"a":1}"#,
        "label $out | (.[break $out] = 5)",
        Outcome::error("break $out not in label"),
    );
}

#[test]
fn test_optional_prunes_a_path_that_cannot_resolve() {
    // `?` on the path expression prunes the branch, so a key with no component
    // leaves the document untouched rather than erroring — contrast
    // `.[null] = 5` above.
    check(
        r#"{"a":1}"#,
        ".[null]? = 5",
        Outcome::values(&[r#"{"a":1}"#]),
    );
    // A branch under `?` that resolves to *several* components has to be
    // re-assembled as a chain, not left as a bare component list.
    check(
        r#"{"a":{"x":1}}"#,
        r#".a[("x","y")]? = 5"#,
        Outcome::values(&[r#"{"a":{"x":5,"y":5}}"#]),
    );
}

#[test]
fn test_assignment_to_an_out_of_range_index_errors() {
    // DIVERGENCE from jq 1.7.1, and one that predates computed keys: jq pads
    // the array, giving `[1,2,3,null,null,9]`. Pinned rather than fixed so the
    // day it changes is visible; the computed-key work only wrapped this
    // `set_path` call in a per-path loop, it did not introduce the error.
    check(
        "[1,2,3]",
        ".[5] = 9",
        Outcome::error("index 5 out of bounds (length 3)"),
    );
}

#[test]
fn test_path_expression_shapes_around_a_computed_key() {
    // Parenthesised and piped path prefixes are spliced into flat components,
    // so the key still sees the value reaching *its* position.
    check(
        r#"{"a":{"x":1}}"#,
        r#"((.a) | .[("x","y")]) = 5"#,
        Outcome::values(&[r#"{"a":{"x":5,"y":5}}"#]),
    );
    // A comma at a path position resolves each branch independently, and a
    // branch without any computed key is copied through untouched.
    check(
        r#"{"a":{"b":{"x":1}},"c":{"x":2}}"#,
        r#"((.a|.b), .c[("x","y")]) = 5"#,
        Outcome::values(&[r#"{"a":{"b":5},"c":{"x":5,"y":5}}"#]),
    );
    // `.[]` before a computed key expands to one concrete component per
    // element, because each continues with its own key.
    check(
        r#"{"p":{"x":1}}"#,
        r#"(.[] | .[("x","y")]) = 9"#,
        Outcome::values(&[r#"{"p":{"x":9,"y":9}}"#]),
    );
    // A branch that produces no value prunes rather than erroring.
    check(
        r#"{"a":1}"#,
        r#"(empty | .[("x","y")]) = 9"#,
        Outcome::values(&[r#"{"a":1}"#]),
    );
}

/// A computed key is checked against the container it will actually index, not
/// against the value the chain started from.
///
/// The resolver keeps the static components of a path verbatim and threads a
/// value alongside them; when it skipped that threading, `.a.b[.k]` checked a
/// numeric key against the document *root* — an object — and reported `Cannot
/// index object with number` for a filter jq assigns without complaint.
///
/// A string key hides this: looking one up in the wrong object yields null
/// rather than an error, so every case here uses a numeric key into an array,
/// which is the combination that fails loudly.
#[test]
fn test_computed_key_sees_the_container_it_indexes() {
    // One static component ahead of the key, in each path context.
    let doc = r#"{"a":{"b":[10,20]},"k":1}"#;
    check(
        doc,
        ".a.b[.k] = 99",
        Outcome::values(&[r#"{"a":{"b":[10,99]},"k":1}"#]),
    );
    check(
        doc,
        ".a.b[.k] |= .+1",
        Outcome::values(&[r#"{"a":{"b":[10,21]},"k":1}"#]),
    );
    check(
        doc,
        "del(.a.b[.k])",
        Outcome::values(&[r#"{"a":{"b":[10]},"k":1}"#]),
    );
    check(
        doc,
        "[path(.a.b[.k])]",
        Outcome::values(&[r#"[["a","b",1]]"#]),
    );

    // The components *after* the last computed key are kept verbatim rather
    // than resolved, so they are the second place the value has to be threaded:
    // here `.j` applies to `.a[.k].b`, not to `.a[.k]`.
    check(
        r#"{"a":{"x":{"b":[10,20]}},"k":"x","j":1}"#,
        ".a[.k].b[.j] = 99",
        Outcome::values(&[r#"{"a":{"x":{"b":[10,99]}},"k":"x","j":1}"#]),
    );

    // A decoy at the root under the same name the key resolves to. If the
    // container is taken from the root, the second key indexes the string
    // "decoy" and errors; the plain version above passes either way, because
    // looking "x" up in a root that lacks it yields null, which accepts any
    // key kind.
    check(
        r#"{"a":{"b":{"x":[10,20]}},"x":"decoy","k":"x","j":1}"#,
        ".a.b[.k][.j] = 99",
        Outcome::values(&[r#"{"a":{"b":{"x":[10,99]}},"x":"decoy","k":"x","j":1}"#]),
    );
}

#[test]
fn test_unsupported_path_prefixes_report_rather_than_misfire() {
    // Iterating a scalar. jq says `Cannot iterate over number (1)`; the wording
    // differs, the rejection does not.
    check(
        "1",
        r#"(.[] | .[("x","y")]) = 9"#,
        Outcome::error("expected array or object, got number"),
    );
    // A multi-output component that is not `.[]` cannot be expanded into
    // concrete path components, so it is refused rather than silently applied
    // to one branch. jq resolves `..` and then fails on the first scalar it
    // reaches (`Cannot index number with string "x"`).
    check(
        r#"{"a":1}"#,
        r#"(.. | .[("x","y")]) = 9"#,
        Outcome::error("Cannot use a computed index after a multi-output path component"),
    );
    // `. = 5` replaces the root, so the sibling branch then indexes a number,
    // and reports it as jq does.
    check(
        r#"{"a":{"x":1}}"#,
        r#"(., .a[("x","y")]) = 5"#,
        Outcome::error(r#"Cannot index number with string "a""#),
    );
}

#[test]
fn test_del_resolves_and_orders_computed_keys() {
    check(
        r#"{"a":1,"b":2}"#,
        r#"del(.[("a","b")])"#,
        Outcome::values(&["{}"]),
    );
    // A path that continues past the computed key, and one wrapped in `?`,
    // both have to survive the deletion ordering pass.
    check(
        r#"{"a":{"x":1},"b":{"x":2}}"#,
        r#"del(.[("a","b")].x)"#,
        Outcome::values(&[r#"{"a":{},"b":{}}"#]),
    );
    check(
        r#"{"a":1,"b":2}"#,
        r#"del(.[("a","b")]?)"#,
        Outcome::values(&["{}"]),
    );
    // Deleting repeated indexes deletes once, and right-to-left, so an earlier
    // removal cannot shift a later one.
    check(
        "[10,20,30,40]",
        "del(.[(0,2)])",
        Outcome::values(&["[20,40]"]),
    );
    check("[1,2,3]", "del(.[(0,0)])", Outcome::values(&["[2,3]"]));
}

#[test]
fn test_del_and_path_report_a_bad_computed_key() {
    check(
        r#"{"a":1}"#,
        "del(.[null])",
        Outcome::error("Cannot index object with null"),
    );
    check(
        r#"{"a":1}"#,
        "path(.[null])",
        Outcome::error("Cannot index object with null"),
    );
    // The key resolves, but the value it lands on cannot be indexed further.
    check(
        r#"{"a":5}"#,
        r#"del(.a[("x","y")])"#,
        Outcome::error(r#"Cannot index number with string "x""#),
    );
    // A static path that cannot apply still errors from the deletion loop,
    // with the same sentence a computed key would give.
    check(
        "[1,2,3]",
        "del(.foo)",
        Outcome::error(r#"Cannot index array with string "foo""#),
    );
}

#[test]
fn test_path_of_a_computed_key_emits_one_path_per_key() {
    check(
        r#"{"a":1,"b":2}"#,
        r#"path(.[("a","b")])"#,
        Outcome::values(&[r#"["a"]"#, r#"["b"]"#]),
    );
}

// =============================================================================
// A computed key inside other expression forms
// =============================================================================

#[test]
fn test_computed_key_survives_function_expansion() {
    // The walkers that inline a `def` body and substitute its parameters have
    // to descend into both halves of an `IndexExpr`, or the key silently stays
    // un-substituted.
    check(
        r#"{"a":1,"b":2}"#,
        r#"def f: [.[("a","b")]]; f"#,
        Outcome::values(&["[1,2]"]),
    );
    check(
        r#"{"a":1,"b":2}"#,
        r#"def g(p): [.[(p,p)]]; g("a")"#,
        Outcome::values(&["[1,1]"]),
    );
}

#[test]
fn test_computed_key_as_a_pipe_stage() {
    // A computed index as a pipe element is walked by the path-context check
    // that decides how the rest of the pipe is evaluated, so both halves of the
    // node have to be reachable from that walk.
    check(
        r#"{"a":"xx","b":"yyy"}"#,
        r#".[("a","b")] | length"#,
        Outcome::values(&["2", "3"]),
    );
}

#[test]
fn test_bracket_after_a_dot_indexes_the_chain() {
    // `.a.[0]` is jq's older spelling of `.a[0]`, still accepted; the dot
    // before the bracket is what routes it through a different parser branch.
    check(r#"{"a":[10,20]}"#, ".a.[0]", Outcome::values(&["10"]));
    check(
        r#"{"a":[10,20],"k":1}"#,
        ".a.[.k]",
        Outcome::values(&["20"]),
    );
    check(
        r#"{"a":[10,20]}"#,
        r#".a.[("0")]"#,
        Outcome::error(r#"Cannot index array with string "0""#),
    );
}

// =============================================================================
// Static forms that share the computed-key helpers
// =============================================================================

#[test]
fn test_static_field_and_index_errors_match_the_computed_ones() {
    // `.foo` and `.[0]` report the same message a computed key of that kind
    // would, which is the point of sharing the lookup bodies.
    check(
        "[1,2,3]",
        ".foo",
        Outcome::error(r#"Cannot index array with string "foo""#),
    );
    check(
        r#"{"a":1}"#,
        ".[0]",
        Outcome::error("Cannot index object with number"),
    );
    // Index on null passes through; `?` on a non-array suppresses.
    check("null", ".[0]", Outcome::values(&["null"]));
    check("1", ".[0]?", Outcome::values(&[]));
}

// =============================================================================
// Cursor-valued targets (generic evaluator only)
// =============================================================================

#[test]
fn test_cursor_target_is_indexed_in_place() {
    // `at_offset` hands back a cursor rather than a value, which only the
    // generic (CLI) evaluator can produce — the library `eval` entry point has
    // no cursor to navigate from and reports that. Indexing it must still work.
    // Collecting into `[...]` would re-enter without the cursor, so the target
    // is indexed directly and the two outputs are compared as a stream.
    let doc = r#"{"a":1,"b":2}"#;
    assert_eq!(
        generic(doc.as_bytes(), r#"at_offset(0)[("a","b")]"#),
        Outcome::values(&["1", "2"])
    );
    assert_eq!(
        full(doc.as_bytes(), r#"at_offset(0)[("a","b")]"#),
        Outcome::error("at_offset requires document cursor context")
    );
}
