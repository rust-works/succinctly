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

/// #488: NaN is "no array element" only where a number addresses an element.
///
/// The key kind is otherwise dispatched before the container (see above), and
/// NaN is the exception that has to look: on a container a number cannot index
/// at all, the failure is the ordinary indexing one, and jq's message says
/// nothing about NaN.
#[test]
fn test_nan_key_names_the_container_it_cannot_index() {
    // An array — and the null a write would build into one — is where NaN's own
    // complaint belongs.
    for input in ["[1,2,3]", "null"] {
        check(
            input,
            ".[nan] = 5",
            Outcome::error("Cannot set array element at NaN index"),
        );
    }

    // Anywhere else it is the message `.[0] = 5` gets on the same document.
    check(
        r#"{"a":1}"#,
        ".[nan] = 5",
        Outcome::error("Cannot index object with number"),
    );
    check(
        r#"{"a":1}"#,
        ".[0] = 5",
        Outcome::error("Cannot index object with number"),
    );
    check(
        r#""s""#,
        ".[nan] = 5",
        Outcome::error("Cannot index string with number"),
    );
    check(
        "true",
        ".[nan] = 5",
        Outcome::error("Cannot index boolean with number"),
    );

    // Through the other writers that resolve a path, not just `=`.
    check(
        r#"{"a":1}"#,
        ".[nan] |= 5",
        Outcome::error("Cannot index object with number"),
    );
    check(
        r#"{"a":1}"#,
        "del(.[nan])",
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

/// `?` does not cover the *target* either — jq's `gen_index_opt(obj, key)`
/// makes one opcode optional and compiles both halves normally.
///
/// The folded spelling `"str" | .a[0]?` has always raised here, because `.a`
/// and `[0]` are separate chain elements and only the second is wrapped. When
/// the computed-key path passed its `optional` down into the target as well,
/// `"str" | .a[1+1]?` — the same query, differing only in whether the key
/// happens to fold to a constant — silently returned nothing instead. Every
/// case below is written in both spellings for exactly that reason.
///
/// A `Paren` target hides this: `(.a)[…]` resets the flag on the way down, so
/// `(error("boom"))[…]?` raised even while the bare-field form did not.
#[test]
fn test_optional_does_not_reach_the_target() {
    for filter in [".a[0]?", ".a[1+1]?", ".a[length]?"] {
        check(
            r#""str""#,
            filter,
            Outcome::error(r#"Cannot index string with string "a""#),
        );
    }

    // Deeper in the chain, where the failing element is not the chain head.
    for filter in [".a.b[0]?", ".a.b[1+1]?", ".a.b[length]?"] {
        check(
            r#"{"a":"s"}"#,
            filter,
            Outcome::error(r#"Cannot index string with string "b""#),
        );
    }

    // A target that raises outright rather than by being unindexable. This is
    // the parenthesised spelling that kept working throughout, and is here as
    // the contrast that made the bare-field bug invisible.
    check(
        "null",
        r#"(error("boom"))[("a","b")]?"#,
        Outcome::error("boom"),
    );

    // What `?` *does* still suppress, so the fix is not simply "`?` no longer
    // does anything": the indexing itself, once the target has been produced.
    check(r#"{"a":"s"}"#, ".a[length]?", Outcome::values(&[]));
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
    // "Empty" has two spellings inside the evaluators — a `None` result and a
    // present-but-zero-length one — and only the second reaches the explicit
    // length check. An inner computed index that prunes every key produces the
    // second: `1` is indexable by neither `"p"` nor `"q"`, so `?` prunes both
    // and hands the outer bracket a stream that exists and is empty.
    check(r#"{"x":1}"#, r#".[.x[("p","q")]?]"#, Outcome::values(&[]));
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

/// The two ways a resolved branch is pruned rather than propagated, neither of
/// which the cases above reach.
///
/// Both are the failure to *index* that `?` covers — the half #413 deliberately
/// left suppressed — but they arrive at different points in the resolver, and a
/// key kind that is valid on its own reaches only the second.
#[test]
fn test_optional_prunes_an_index_failure_and_an_unresolvable_target() {
    // The key's *kind* fits (a string names a field, a number an element), so the
    // component is built — and only then does applying it to the container fail.
    // The suppressed-key cases elsewhere in this file fail one step earlier, when
    // no component can be named at all (`.[null]?`).
    for input in [
        r#"{"a":"str","k":"x"}"#, // string key, string container
        r#"{"a":true,"k":"x"}"#,  // string key, boolean container
        r#"{"a":{},"k":0}"#,      // number key, object container
    ] {
        check(input, ".a[.k]? = 5", Outcome::values(&[input]));
    }

    // A `?`-wrapped node that is not `E[K]` is resolved by the blanket arm, which
    // prunes when the node itself cannot resolve. Reaching it needs a computed key
    // *elsewhere* in the path — without one the whole pre-pass short-circuits and
    // the walkers see the `?` directly. `1+1`/`length` are keys that do not touch
    // the input, so the target is what fails: `.a` on a string.
    for filter in [".a?[1+1] = 5", ".a?[length] = 5"] {
        check(r#""str""#, filter, Outcome::values(&[r#""str""#]));
    }
    // The contrast: with the target resolvable, the same filter reports the
    // indexing failure that the missing `?` on the bracket no longer covers.
    check(
        r#"{"a":1}"#,
        ".a?[length] = 5",
        Outcome::error("Cannot index number with number"),
    );
}

/// #413: `?` on an assignment path suppresses only the failure to *index* —
/// exactly as it does in value position (`test_optional_does_not_reach_the_target`
/// above). It does not cover a failure raised while evaluating the key itself,
/// nor one raised while evaluating the target: `.[.k]?` on a string still
/// raises "Cannot index string with string \"k\"" for `.k`, and NaN still
/// refuses to name an *array* element, `?` or not.
///
/// The key is evaluated before the target here, as it is in value position, so
/// which of the two a message blames — and whether the target runs at all — is
/// part of what these cases pin.
#[test]
fn test_optional_does_not_swallow_a_key_or_target_evaluation_error() {
    check(
        r#""str""#,
        ".[.k]? = 5",
        Outcome::error(r#"Cannot index string with string "k""#),
    );
    // The key is evaluated against the value *threaded to that position*, not
    // the document root, so the same failure shows up after a pipe stage too.
    check(
        r#"{"a":"str","k":"x"}"#,
        ".a | .[.k]? = 5",
        Outcome::error(r#"Cannot index string with string "k""#),
    );
    // A target evaluation failure is likewise not covered by the trailing `?` —
    // but the key is evaluated first, so on `5` it is `.k` that raises and the
    // `.a` the message names is the key, not the target that never ran.
    check(
        "5",
        ".a[.k]? = 9",
        Outcome::error(r#"Cannot index number with string "k""#),
    );
    // Keys-first is visible in the other direction too: an empty key stream
    // short-circuits before the target, so the target's failure never happens.
    // (`test_empty_key_stream_short_circuits` pins the same rule in value
    // position.)
    check("5", ".a[empty]? = 9", Outcome::values(&["5"]));
    check("5", ".a[empty] = 9", Outcome::values(&["5"]));
    // NaN denotes no array element, so it still errors under `?` even though a
    // plain type mismatch (tested above) does not. null counts as an array here
    // exactly as it does for a write: `null | .[0] = 5` builds one.
    check(
        "[1,2,3]",
        ".[nan]? = 5",
        Outcome::error("Cannot set array element at NaN index"),
    );
    check(
        "null",
        ".[nan]? = 5",
        Outcome::error("Cannot set array element at NaN index"),
    );
    // On a container a number cannot address at all, though, the failure is the
    // ordinary `Cannot index object with number` — which is exactly what `?`
    // suppresses, so these leave the document alone rather than raising.
    check(
        r#"{"a":1}"#,
        ".[nan]? = 5",
        Outcome::values(&[r#"{"a":1}"#]),
    );
    check(r#""str""#, ".[nan]? = 5", Outcome::values(&[r#""str""#]));
    // Per key, not per filter: the NaN prunes its own branch and the string key
    // beside it still assigns.
    check(
        r#"{"a":0}"#,
        r#".[("a",nan)]? = 1"#,
        Outcome::values(&[r#"{"a":1}"#]),
    );
    // What `?` *does* still suppress: a genuine failure to index, once the
    // target and key have both evaluated without error.
    check(
        r#"{"a":1}"#,
        ".a[.k]? = 5",
        Outcome::values(&[r#"{"a":1}"#]),
    );
}

/// The rule of #413 belongs to the path *resolver*, so it has to hold for every
/// filter that resolves a path — not just `=`, which is where the bug was found.
/// `|=`, the compound forms, `del` and `path` all reach `resolve_dynamic_indexes`
/// through their own entry points.
#[test]
fn test_optional_key_error_reaches_every_path_consuming_form() {
    for filter in [
        ".[.k]? = 5",
        ".[.k]? |= 5",
        ".[.k]? += 1",
        ".[.k]? //= 5",
        "del(.[.k]?)",
        "[path(.[.k]?)]",
    ] {
        check(
            r#""str""#,
            filter,
            Outcome::error(r#"Cannot index string with string "k""#),
        );
    }
    // And the NaN-on-an-object case stays suppressed through the same forms:
    // the writes are no-ops rather than errors.
    check(
        r#"{"a":1}"#,
        ".[nan]? |= 5",
        Outcome::values(&[r#"{"a":1}"#]),
    );
    check(
        r#"{"a":1}"#,
        "del(.[nan]?)",
        Outcome::values(&[r#"{"a":1}"#]),
    );
    // And a suppressed key that resolves to no path at all is *no output*, not
    // the root path — the divergence #489 fixed. It was pinned here as `[[]]`
    // while it lasted, because it needed no key to show (`{"a":1} |
    // [path(empty)]` read `[[]]` too) and this case is where it was visible.
    check(r#"{"a":1}"#, "[path(.[nan]?)]", Outcome::values(&["[]"]));
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

/// Components *after* the last computed key stay verbatim, including ones that
/// fan out.
///
/// The resolver stops expanding once no computed key remains to its right, so
/// `.[.k].b[]` resolves `.k` and then copies `.b` and `[]` through untouched —
/// one path with an `Iterate` in it, not one path per element. It still has to
/// thread a *value* through that tail, in case an enclosing key indexes it, and
/// a fanning-out component has no single value to thread: the walk stops at
/// null there rather than picking an arbitrary element.
///
/// Both directions matter. `[]` over two elements is the many-valued case; `[]`
/// over an empty array is the no-valued one, and reaches the same stop.
#[test]
fn test_components_after_the_last_computed_key_may_fan_out() {
    check(
        r#"{"k":"a","a":{"b":[1,2]}}"#,
        ".[.k].b[] = 9",
        Outcome::values(&[r#"{"k":"a","a":{"b":[9,9]}}"#]),
    );
    check(
        r#"{"k":"a","a":{"b":[1,2]}}"#,
        ".[.k].b[] |= .*10",
        Outcome::values(&[r#"{"k":"a","a":{"b":[10,20]}}"#]),
    );
    check(
        r#"{"k":"a","a":{"b":[1,2]}}"#,
        "del(.[.k].b[])",
        Outcome::values(&[r#"{"k":"a","a":{"b":[]}}"#]),
    );
    // One `Iterate` component, expanded by the tracker rather than the
    // resolver — which is why it is still two paths.
    check(
        r#"{"k":"a","a":{"b":[1,2]}}"#,
        "[path(.[.k].b[])]",
        Outcome::values(&[r#"[["a","b",0],["a","b",1]]"#]),
    );
    // Nothing to iterate: same stop, no write, no error.
    check(
        r#"{"k":"a","a":{"b":[]}}"#,
        ".[.k].b[] = 9",
        Outcome::values(&[r#"{"k":"a","a":{"b":[]}}"#]),
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
    // Iterating a scalar, reported in jq's own words (#538).
    check(
        "1",
        r#"(.[] | .[("x","y")]) = 9"#,
        Outcome::error("Cannot iterate over number (1)"),
    );
    // `range(3)` is a multi-output component with no path-tracking arm (#412
    // covers `..`, `recurse` and the typeof filters — see
    // `test_multi_output_path_components_fan_out` — but not arbitrary
    // generators), so it is still refused rather than silently applied to one
    // branch. jq itself refuses too, in its own words (`Invalid path
    // expression near attempt to access element "x" of 0`).
    check(
        r#"{"a":1}"#,
        r#"(range(3) | .[("x","y")]) = 9"#,
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

/// #412: a computed key after `..`, `recurse` or a typeof filter (`objects`,
/// `select`, ...) now resolves per branch instead of being refused outright —
/// `resolve_node` names the Field/Index chain reaching *each* of the many
/// values, the same job it already did for `.[]`.
///
/// Every expectation here was captured from real jq 1.7.1.
#[test]
fn test_multi_output_path_components_fan_out() {
    // The exact repro from #412.
    check(
        r#"{"k":"a","a":{"k":"a","a":1}}"#,
        r"[path(.. | objects | .[.k]?)]",
        Outcome::values(&[r#"[["a"],["a","a"]]"#]),
    );

    let doc = r#"{"x":{"k":"v","v":1},"y":{"k":"w","w":2}}"#;
    check(
        doc,
        r"[path(.. | objects | .[.k]?)]",
        Outcome::values(&[r#"[["x","v"],["y","w"]]"#]),
    );
    check(
        doc,
        r"(.. | objects | .[.k]?) = 99",
        Outcome::values(&[r#"{"x":{"k":"v","v":99},"y":{"k":"w","w":99}}"#]),
    );
    check(
        doc,
        r"(.. | objects | .[.k]?) |= (. + 1)",
        Outcome::values(&[r#"{"x":{"k":"v","v":2},"y":{"k":"w","w":3}}"#]),
    );
    check(
        doc,
        r"del(.. | objects | .[.k]?)",
        Outcome::values(&[r#"{"x":{"k":"v"},"y":{"k":"w"}}"#]),
    );

    // `recurse` (BFS order) and `select(f)` generalise the same way as `..`
    // and `objects`.
    check(
        doc,
        r"[path(recurse | objects | .[.k]?)]",
        Outcome::values(&[r#"[["x","v"],["y","w"]]"#]),
    );
    check(
        doc,
        r#"[path(.. | select(type == "object") | .[.k]?)]"#,
        Outcome::values(&[r#"[["x","v"],["y","w"]]"#]),
    );

    // `..` still fails loudly, in jq's own words, when the computed key
    // reaches a value that cannot be indexed by it — the resolver fans out
    // and lets the ordinary indexing error surface per branch.
    check(
        r#"{"a":1}"#,
        r#"(.. | .[("x","y")]) = 9"#,
        Outcome::error(r#"Cannot index number with string "x""#),
    );
}

/// Every *writer* after a multi-output prefix, not just `path()`.
///
/// `path()` reads a resolved path; `=`, `|=`, `+=` and `del()` walk it, through
/// three functions `path()` never touches (`get_path_mut`, `update_path`,
/// `delete_at_path`). Checking only `path()` is what let a resolver emitting
/// `Optional(Field("x")) | Optional(Field("v"))` — which those three matched
/// against `Field`/`Index`/`Iterate`, missed, and handled by acting at the
/// *wrapper's* position with the rest of the path dropped — read correctly
/// while `del(recurse | objects | .[.k]?)` deleted the whole `.x` and
/// `|=` overwrote it. So each prefix is pinned against all five.
///
/// `.[]?` carries the same wrapper and was wrong the same way before #412 —
/// it is here because it is the case that shows the defect was never about
/// `recurse`.
#[test]
fn test_every_writer_agrees_after_a_multi_output_prefix() {
    let doc = r#"{"x":{"k":"v","v":1}}"#;

    for prefix in [
        ".. | objects",
        "recurse | objects",
        "recurse(.[]?) | objects",
    ] {
        check(
            doc,
            &format!("[path({prefix} | .[.k]?)]"),
            Outcome::values(&[r#"[["x","v"]]"#]),
        );
        check(
            doc,
            &format!("({prefix} | .[.k]?) = 7"),
            Outcome::values(&[r#"{"x":{"k":"v","v":7}}"#]),
        );
        check(
            doc,
            &format!("({prefix} | .[.k]?) |= 7"),
            Outcome::values(&[r#"{"x":{"k":"v","v":7}}"#]),
        );
        check(
            doc,
            &format!("({prefix} | .[.k]?) += 7"),
            Outcome::values(&[r#"{"x":{"k":"v","v":8}}"#]),
        );
        check(
            doc,
            &format!("del({prefix} | .[.k]?)"),
            Outcome::values(&[r#"{"x":{"k":"v"}}"#]),
        );
    }

    // The `?`-wrapped spelling of `.[]`, whose components reach the walkers
    // wrapped even without any of the #412 arms in play.
    check(
        doc,
        r"[path(.[]? | .[.k]?)]",
        Outcome::values(&[r#"[["x","v"]]"#]),
    );
    check(
        doc,
        r"(.[]? | .[.k]?) = 7",
        Outcome::values(&[r#"{"x":{"k":"v","v":7}}"#]),
    );
    check(
        doc,
        r"(.[]? | .[.k]?) |= 7",
        Outcome::values(&[r#"{"x":{"k":"v","v":7}}"#]),
    );
    check(
        doc,
        r"del(.[]? | .[.k]?)",
        Outcome::values(&[r#"{"x":{"k":"v"}}"#]),
    );
}

/// `recurse(f)` for an `f` that never stops producing.
///
/// `f` is arbitrary, so nothing guarantees progress: `.a?` reads `null` from
/// `null` forever. `builtin_recurse_f` is bounded because it does not queue a
/// null child, and `resolve_recurse` has to make the same choice — without it
/// the queue runs to its 10,000-item cutoff with the path prefix one component
/// longer each round, which is quadratic. Measured at 9 GB resident and 5s of
/// CPU for this 18-byte document; it is a bound, not a preference.
///
/// jq does not terminate here at all (it recurses until it cannot allocate),
/// so the expectation is `builtin_recurse_f`'s, deliberately.
#[test]
fn test_recurse_over_a_null_producing_filter_terminates() {
    let doc = r#"{"k":"a","a":null}"#;
    // Both sides visit the root and stop: `.a?` yields null, which ends that
    // line of descent rather than seeding another round.
    check(
        doc,
        r"[recurse(.a?)]",
        Outcome::values(&[r#"[{"k":"a","a":null}]"#]),
    );
    check(
        doc,
        r"[path(recurse(.a?) | objects | .[.k]?)]",
        Outcome::values(&[r#"[["a"]]"#]),
    );
}

/// The three parameterised `recurse` spellings, which
/// `test_multi_output_path_components_fan_out` exercises only in its bare
/// `recurse` form. Bare `recurse` is `..` and shares its resolver; these do
/// not, because `f` is arbitrary — `resolve_recurse` re-implements
/// `builtin_recurse_f`/`builtin_recurse_cond`'s queue in order to thread path
/// components alongside each value, so the thing worth pinning is that it
/// still visits what those two visit.
///
/// It does not follow them in *every* respect, and the difference is
/// deliberate: when `f` yields an array those two descend into its elements
/// (an artefact of collapsing a stream into one array), where jq and the
/// resolver both stop at the array. See `resolve_recurse`'s own note.
#[test]
fn test_recurse_variants_fan_out_like_their_value_paths() {
    let doc = r#"{"x":{"k":"v","v":1},"y":{"k":"w","w":2}}"#;
    // `recurse(f)` and `recurse(f; cond)` with a cond that keeps everything
    // both reduce to the bare `recurse` above.
    check(
        doc,
        r"[path(recurse(.[]?) | objects | .[.k]?)]",
        Outcome::values(&[r#"[["x","v"],["y","w"]]"#]),
    );
    check(
        doc,
        r"[path(recurse(.[]?; true) | objects | .[.k]?)]",
        Outcome::values(&[r#"[["x","v"],["y","w"]]"#]),
    );

    // A cond that actually prunes: `.v` and `.w` are scalars, so recursion
    // stops at them and only the two objects' own keys are reached. Without
    // the cond being honoured this would also walk into the scalars.
    check(
        r#"{"k":"v","v":1,"deep":{"k":"w","w":2}}"#,
        r#"[path(recurse(.[]?; type == "object") | objects | .[.k]?)]"#,
        Outcome::values(&[r#"[["v"],["deep","w"]]"#]),
    );

    // `recurse_down` is a succinctly-only alias for `recurse` (jq 1.7.1 has no
    // such builtin), so it is pinned against `recurse` rather than against jq.
    check(
        doc,
        r"[path(recurse_down | objects | .[.k]?)]",
        Outcome::values(&[r#"[["x","v"],["y","w"]]"#]),
    );

    // A cond that *errors* on some node prunes it, rather than propagating —
    // `.k` on the string `"v"` cannot be indexed. jq instead raises `Cannot
    // index string with string "k"`. This mirrors `builtin_recurse_cond`'s own
    // `Err(_) => false`, i.e. the divergence is in the value path and predates
    // #412; what is pinned here is that the path side agrees with it, because
    // a resolver that propagated would make `path(f)` disagree with `f`.
    check(
        r#"{"k":"v","v":1,"deep":{"k":"w","w":2}}"#,
        r#"[path(recurse(.[]?; .k != "w") | objects | .[.k]?)]"#,
        Outcome::values(&[r#"[["v"]]"#]),
    );
}

/// Each arm of `type_filter_matches`, which decides whether a typeof filter
/// keeps a path branch or prunes it. A miswired arm (`booleans` matching
/// numbers, `scalars` matching arrays) is invisible in the happy cases above,
/// where only `objects` is exercised.
///
/// The key is `("p","q")` — two keys of the *same* kind, and neither derived
/// from the value being indexed — so each branch either resolves both keys or
/// neither, and the filter's keep/prune decision is the only thing under test.
#[test]
fn test_typeof_filters_decide_which_path_branches_survive() {
    let doc = r#"{"arr":[10,20],"obj":{"p":1},"s":"str","n":7,"b":true,"u":null}"#;

    // The filters that keep an indexable branch: the root and `.obj` are the
    // two objects, and a string key into null is legal (it reads as null), so
    // `.u` is what survives `scalars`/`nulls`.
    for filter in ["objects", "values", "iterables"] {
        check(
            doc,
            &format!(r#"[path(.. | {filter} | .[("p","q")]?)]"#),
            Outcome::values(&[r#"[["p"],["q"],["obj","p"],["obj","q"]]"#]),
        );
    }
    for filter in ["scalars", "nulls"] {
        check(
            doc,
            &format!(r#"[path(.. | {filter} | .[("p","q")]?)]"#),
            Outcome::values(&[r#"[["u","p"],["u","q"]]"#]),
        );
    }

    // The remaining arms keep only branches a string key *cannot* index, so
    // they emit no path at all — which on its own would also be the reading if
    // the arm wrongly pruned everything. Dropping the `?` distinguishes the
    // two: the error names the type the arm let through, so it is evidence the
    // branch was kept. (`nulls` is absent here because assignment through null
    // is a separate pre-existing gap: jq autovivifies `{"u":null} | .u.p = 9`
    // to `{"u":{"p":9}}`, succinctly reports `Cannot index null with string`.)
    for (filter, type_name) in [
        ("arrays", "array"),
        ("numbers", "number"),
        ("strings", "string"),
        ("booleans", "boolean"),
    ] {
        check(
            doc,
            &format!(r#"(.. | {filter} | .[("p","q")]) = 9"#),
            Outcome::error(&format!(r#"Cannot index {type_name} with string "p""#)),
        );
    }
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
    // Deleting repeated indexes deletes once, and every index is removed
    // simultaneously, so an earlier removal cannot shift a later one.
    check(
        "[10,20,30,40]",
        "del(.[(0,2)])",
        Outcome::values(&["[20,40]"]),
    );
    check("[1,2,3]", "del(.[(0,0)])", Outcome::values(&["[2,3]"]));
}

/// #424: a negative computed index has to resolve against the length the
/// array had *before* any deletion here, not the length after an earlier
/// sibling was already removed.
///
/// `del(.[(-1,-2)])` on `[10,20,30,40]` used to delete `-1` (`40`) first,
/// shortening the array to length 3, so `-2` then counted back from *that*
/// and took `20` instead of `30` — giving `[10,30]` where jq gives `[10,20]`.
/// Reversing the argument order didn't help, because `-1` and `-2` are
/// counted from the opposite end to a non-negative index, so no ordering of
/// one-at-a-time deletions is correct; every index has to be resolved
/// against the same, original length and removed in one pass.
#[test]
fn test_del_with_negative_computed_indexes_resolves_against_original_length() {
    check(
        "[10,20,30,40]",
        "del(.[(-1,-2)])",
        Outcome::values(&["[10,20]"]),
    );
    // Order-insensitive, same as the non-negative case above.
    check(
        "[10,20,30,40]",
        "del(.[(-2,-1)])",
        Outcome::values(&["[10,20]"]),
    );
    // A mixed sign pair that isn't order-insensitive by coincidence.
    check(
        "[10,20,30,40]",
        "del(.[(1,-1)])",
        Outcome::values(&["[10,30]"]),
    );
    // Two independent computed-index positions in one chain: each inner
    // array gets its own simultaneous removal, resolved against its own
    // (not the outer array's) length.
    check(
        "[[10,20,30],[40,50,60,70]]",
        "del(.[(0,1)][(-1,-2)])",
        Outcome::values(&["[[10],[40,50]]"]),
    );
}

/// #477: an out-of-range array index used to raise in `del()`, where jq's
/// `delpaths` — which `del` is defined in terms of, `def del(f):
/// delpaths([path(f)]);` — silently skips a path that names nothing, `?` or
/// not. `delete_keys` already matched that (its doc comment: "silently drops
/// an index that names nothing"); `del`'s own path-walking code, which can't
/// just call `delete_keys` directly since it works on path *expressions*
/// rather than resolved paths, raised instead.
#[test]
fn test_del_on_out_of_range_index_is_a_silent_noop() {
    // Positive index past the end.
    check("[1,2]", "del(.[5])", Outcome::values(&["[1,2]"]));
    // Negative index still negative after counting back from the end.
    check("[1,2]", "del(.[-5])", Outcome::values(&["[1,2]"]));
    // `?` makes no difference — jq never errors on this in the first place.
    check("[1,2]", "del(.[5]?)", Outcome::values(&["[1,2]"]));
    check("[1,2]", "del(.[-5]?)", Outcome::values(&["[1,2]"]));
    // The rest of the path is skipped along with it, rather than walked
    // against the `null` an in-range read would have produced.
    check("[1,2]", "del(.[5].a)", Outcome::values(&["[1,2]"]));
    check(
        "[[1,2],[3,4]]",
        "del(.[5][0])",
        Outcome::values(&["[[1,2],[3,4]]"]),
    );
}

/// Same no-op behavior through the grouped/computed-key deletion path added
/// for #424 — both when the out-of-range index shares a sibling group with an
/// in-range one (`delete_expr_array_paths`'s per-key check), and when every
/// sibling is out of range and the index only ever reaches the terminal
/// `delete_keys` call.
#[test]
fn test_del_on_out_of_range_computed_index_is_a_silent_noop() {
    check("[1,2]", "del(.[(0,5)])", Outcome::values(&["[2]"]));
    check("[1,2]", "del(.[(5,6)])", Outcome::values(&["[1,2]"]));
    check(
        "[[1,2],[3,4]]",
        "del(.[(0,5)][0])",
        Outcome::values(&["[[2],[3,4]]"]),
    );
}

/// Grouped deletion (added above for #424) assumed every sibling path
/// reaching the same depth shared the exact same shape — all `Field`, all
/// `Index`, or all `Iterate` — and dispatched once on the first path's shape
/// alone. That assumption breaks against `null`: `null` accepts a string
/// key, a numeric key, or `.[]` without erroring (`null | .a`, `null | .[0]`,
/// and `null | .[]` are all `null`), so `.[("a",0)]` resolved against a null
/// target yields one `Field("a")` path and one `Index(0)` path at the same
/// position. That used to panic (`unreachable!()`); #424's fix stopped the
/// panic but still raised the same error the existing single-path `del(.a)`
/// on `null` raised at the time. #476 closed that remaining divergence:
/// `delete_expr_object_paths`/`delete_expr_array_paths` now give `null` the
/// same unconditional no-op exemption `delete_keys` already gave it, so all
/// three cases below now agree with jq's silent no-op.
#[test]
fn test_del_computed_index_against_null_is_a_no_op() {
    check("null", r#"del(.[("a",0)])"#, Outcome::values(&["null"]));
    // Same shape mismatch, opposite generation order — still order-independent
    // (fields are always resolved before indices).
    check("null", r#"del(.[(0,"a")])"#, Outcome::values(&["null"]));
    // Nested under a field rather than at the top of the path.
    check(
        r#"{"x":null}"#,
        r#"del(.x[("a",0)])"#,
        Outcome::values(&[r#"{"x":null}"#]),
    );
    // The no-op is unconditional — a `?` on one sibling doesn't matter when
    // *every* sibling reaches the same tolerant `null`, unlike the genuine
    // wrong-type case in
    // `test_del_container_type_error_is_not_masked_by_an_earlier_optional_sibling`
    // below.
    check(
        "null",
        r#"del(.a?, .[("b","c")])"#,
        Outcome::values(&["null"]),
    );
    check(
        "null",
        r#"del(.[("b","c")], .a?)"#,
        Outcome::values(&["null"]),
    );
}

/// The same array index can be named by more than one sibling path in one
/// `del(...)` argument, each with its own `?`: `del(.[(0,5)].a, .[5]?.a)`
/// names index 5 once without `?` (via the computed key `(0,5)`) and once
/// with it. Grouping by index used to keep only whichever occurrence's
/// `optional` flag was pushed first, so whether the shared, out-of-range
/// index 5 raised depended on which side of the comma `.[5]?` was written
/// on. One optional occurrence has to cover every other occurrence of the
/// same index, regardless of order.
#[test]
fn test_del_merges_optional_across_duplicate_indexes_order_independently() {
    check(
        r#"[{"a":1},{"a":2}]"#,
        r"del(.[(0,5)].a, .[5]?.a)",
        Outcome::values(&[r#"[{},{"a":2}]"#]),
    );
    check(
        r#"[{"a":1},{"a":2}]"#,
        r"del(.[5]?.a, .[(0,5)].a)",
        Outcome::values(&[r#"[{},{"a":2}]"#]),
    );
}

/// A non-object (or non-array) container fails every sibling path
/// identically, so an optional sibling must not mask a non-optional one's
/// error just because it happens to resolve first. `.a?` on `5` succeeds
/// silently, but `.[("b","c")]` (no `?`) reaching the same `5` still has
/// to raise — whichever order they're written in.
///
/// `null` used to be the example container here, but #476 gave `null` an
/// unconditional no-op exemption (see `test_del_computed_index_against_null_is_a_no_op`
/// above) — `null | del(.a?, .[("b","c")])` is now `null` regardless of the
/// optional mix, so it no longer demonstrates this masking property. `5` is
/// a genuine wrong type and still raises, both orderings, keeping the
/// original intent of this test.
#[test]
fn test_del_container_type_error_is_not_masked_by_an_earlier_optional_sibling() {
    check(
        "5",
        r#"del(.a?, .[("b","c")])"#,
        Outcome::error(r#"Cannot index number with string "b""#),
    );
    check(
        "5",
        r#"del(.[("b","c")], .a?)"#,
        Outcome::error(r#"Cannot index number with string "b""#),
    );
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

/// #475: a top-level `Comma` with every branch static (no computed key at
/// all, e.g. `del(.a, .b)`) used to be handed to the single-path walkers
/// verbatim, none of which has a `Comma` arm, so it always fell into the
/// "cannot use expression as delete target" catch-all — unconditionally, for
/// the most basic multi-target `del()` form there is.
#[test]
fn test_del_with_purely_static_comma_paths() {
    check(r#"{"a":1,"b":2}"#, "del(.a, .b)", Outcome::values(&["{}"]));
    check("[1,2,3]", "del(.[0], .[2])", Outcome::values(&["[2]"]));
}

/// #505: a comma target mixing bare `.` (identity, which flattens to zero
/// path components) with any other path (one or more components) panicked —
/// `delete_expr_paths_at`'s leaf check only compared `start` against
/// `paths[0].len()`, so whichever ordering put `.` second either tripped a
/// `debug_assert_eq!` (identity first) or indexed past the end of `.`'s empty
/// component slice (identity elsewhere). An exhausted sibling deletes the
/// whole subtree, which subsumes any other sibling's deletion within it, so
/// the correct result is `null` regardless of order — matching jq.
#[test]
fn test_del_with_comma_mixing_identity_and_other_paths() {
    check(
        r#"{"a":1,"x":"a"}"#,
        "del(.[.x], .)",
        Outcome::values(&["null"]),
    );
    check(
        r#"{"a":1,"x":"a"}"#,
        "del(., .[.x])",
        Outcome::values(&["null"]),
    );
    check(r#"{"a":1,"b":2}"#, "del(., .a)", Outcome::values(&["null"]));
    check(r#"{"a":1,"b":2}"#, "del(., .)", Outcome::values(&["null"]));
}

/// #527: two comma siblings continuing through the same *missing* field
/// (`.a.b.c` and `.a.b.d`, both through `.a.b`) put `delete_expr_object_paths`
/// in front of a field name the object doesn't have, and it raised
/// succinctly's own `field 'b' not found`. jq reads a missing key as `null`
/// and keeps walking the rest of the path against it, so the *tail* decides:
/// a `Field`/`Index`/`Slice` tail is a no-op, an `[]` tail still raises.
#[test]
fn test_del_through_a_missing_intermediate_field_walks_the_rest_against_null() {
    // The issue's own reproduction.
    check(
        r#"{"a":{"x":1}}"#,
        "del(.a.b.c, .a.b.d)",
        Outcome::values(&[r#"{"a":{"x":1}}"#]),
    );
    // A missing sibling must not cancel a present one beside it, in either
    // order, and must not materialise the key it walked through.
    check(
        r#"{"a":{"x":1,"y":2}}"#,
        "del(.a.b.c, .a.x)",
        Outcome::values(&[r#"{"a":{"y":2}}"#]),
    );
    check(
        r#"{"a":{"x":1,"y":2}}"#,
        "del(.a.x, .a.b.c)",
        Outcome::values(&[r#"{"a":{"y":2}}"#]),
    );
    // Terminal and continuing at the same name: deleting `.a` outright
    // subsumes the walk through it.
    check(
        r#"{"a":{"x":1},"z":2}"#,
        "del(.a, .a.b.c)",
        Outcome::values(&[r#"{"z":2}"#]),
    );
    // The grouping is per container, so the same holds one array element
    // down.
    check(
        r#"{"a":[{"z":1}]}"#,
        "del(.a[0].b.c, .a[0].b.d)",
        Outcome::values(&[r#"{"a":[{"z":1}]}"#]),
    );
    // An `[]` tail on the synthesised `null` still raises, and does so even
    // when a no-op sibling shares the same missing field.
    check(
        r#"{"a":{"x":1}}"#,
        "del(.a.b[], .a.b.c)",
        Outcome::error("Cannot iterate over null (null)"),
    );
    // The `[]` can sit further down the tail than the very next step, and
    // the walk still has to reach it.
    check(
        r#"{"a":[{"x":1},{"x":2}]}"#,
        "del(.a[].b.c[], .a[].x)",
        Outcome::error("Cannot iterate over null (null)"),
    );
    // Same rule where the dead end is an explicit `null` rather than an
    // absent key: #476's exemption covers that one step, not the tail.
    check(
        r#"{"a":null}"#,
        "del(.a.b[], .a.c[])",
        Outcome::error("Cannot iterate over null (null)"),
    );
    check(
        r#"{"a":null}"#,
        "del(.a[0][], .a[1][])",
        Outcome::error("Cannot iterate over null (null)"),
    );
    // ... while the tails `null` does tolerate stay no-ops.
    check(
        r#"{"a":null}"#,
        "del(.a.b.c, .a.d.e)",
        Outcome::values(&[r#"{"a":null}"#]),
    );
    // Regression guard: a field that *is* present but cannot be indexed is a
    // genuine error, raised by the `resolve_node` read pre-pass before these
    // walkers ever see the value. Treating a missing key as `null` must not
    // extend to a wrong-typed one.
    check(
        r#"{"a":{"b":5}}"#,
        "del(.a.b.c, .a.b.d)",
        Outcome::error(r#"Cannot index number with string "c""#),
    );
    // ... while an explicitly `null` intermediate stays a no-op (#476).
    check(
        r#"{"a":{"b":null}}"#,
        "del(.a.b.c, .a.b.d)",
        Outcome::values(&[r#"{"a":{"b":null}}"#]),
    );
}

/// #475 follow-up: fixing the purely-static comma case above reaches a
/// second surface that was unexercised until now — `delete_expr_paths_at`
/// routing `Slice` into the same `indices` bucket as a plain `Index` (added
/// for #366 / #492), previously only reachable through a computed key. These
/// pin the three filters the issue's follow-up comment called out: two
/// disjoint-range unions and one case where a slice and an index name the
/// same element, so it's one deletion rather than two.
#[test]
fn test_del_with_static_comma_of_slices_and_indexes() {
    check(
        "[1,2,3,4]",
        "del(.[0:2], .[1:3])",
        Outcome::values(&["[4]"]),
    );
    check("[1,2,3,4]", "del(.[0], .[1:3])", Outcome::values(&["[4]"]));
    check(
        "[1,2,3,4]",
        "del(.[1], .[1:2])",
        Outcome::values(&["[1,3,4]"]),
    );
}

/// #475 follow-up: on a container that cannot be sliced or indexed, jq
/// reports the *first* sibling's key type rather than one fixed sentence —
/// swapping which of an index and a slice comes first in the comma changes
/// "with number" to "with object". An all-slice batch on a string reports
/// "Cannot delete fields from", the message a whole-container deletion
/// raises rather than an index/key-type mismatch.
#[test]
fn test_del_static_comma_type_error_reports_the_first_sibling() {
    check(
        "5",
        "del(.[0], .[1:2])",
        Outcome::error("Cannot index number with number"),
    );
    check(
        "5",
        "del(.[1:2], .[0])",
        Outcome::error("Cannot index number with object"),
    );
    check(
        r#""hi""#,
        "del(.[0:1], .[1:2])",
        Outcome::error("Cannot delete fields from string"),
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
