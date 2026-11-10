//! #6867: `-o=json` of a float that is `+Inf`, `-Inf` or `NaN` -- computed, or a document
//! `.inf`/`.nan` scalar -- fails as yq's JSON encoder does instead of printing `null`.
//!
//! yq v4.53.3 refuses to marshal either and wraps the failure once per node from the number up to
//! the root, the number's own node included (`1 / 0` once, `[1 / 0]` twice, `{"a": [0 / 0]}`
//! three times). A computed number fails in `encoding/json` (`json: unsupported value: +Inf`); a
//! document scalar fails earlier, parsing its own text (`strconv.ParseFloat: parsing ".inf":
//! invalid syntax`). `-r` on a bare scalar skips the encoder and prints the number's own text.
//!
//! Every row below was captured from yq v4.53.3. Where the number sits inside a document the
//! cursor route streams, the part of that result before the number is already written when the
//! failure comes (yq writes none of it), which `limitations.md` records, so those rows pin the
//! diagnostic and the exit code and leave stdout alone, except for the results before it.

use crate::run_yq_stdin_with_stderr;

const WRAP: &str = "json: error calling MarshalJSON for type *yqlib.CandidateNode: ";

/// The first stderr line yq prints: `levels` wraps, then the number's own failure.
fn failure(levels: usize, cause: &str) -> String {
    format!("Error: {}{cause}", WRAP.repeat(levels))
}

fn parse_float(text: &str) -> String {
    format!("strconv.ParseFloat: parsing \"{text}\": invalid syntax")
}

fn run(filter: &str, input: &str, args: &[&str]) -> (String, String, i32) {
    run_yq_stdin_with_stderr(filter, input, args).expect("run succinctly yq")
}

/// (filter, stdout, wraps, cause), run with `-n -o=json`.
const COMPUTED: &[(&str, &str, usize, &str)] = &[
    ("1 / 0", "", 1, "json: unsupported value: +Inf"),
    ("0 / 0", "", 1, "json: unsupported value: NaN"),
    ("1.5 % 0", "", 1, "json: unsupported value: NaN"),
    ("[1 / 0]", "", 2, "json: unsupported value: +Inf"),
    ("[1, (-1 / 0)]", "", 2, "json: unsupported value: -Inf"),
    (
        "{\"a\": {\"b\": [0 / 0]}}",
        "",
        4,
        "json: unsupported value: NaN",
    ),
    // Results before the failing one are written, the failing one and the rest are not.
    ("1, (1 / 0), 2", "1\n", 1, "json: unsupported value: +Inf"),
];

#[test]
fn a_computed_non_finite_float_fails_as_yqs_encoder_does_6867() {
    for (filter, stdout, levels, cause) in COMPUTED {
        for compact in [false, true] {
            let args: &[&str] = if compact {
                &["-n", "-o=json", "-I0"]
            } else {
                &["-n", "-o=json"]
            };
            let (out, err, code) = run(filter, "", args);
            assert_eq!(out, *stdout, "{filter} {args:?}: stdout");
            assert_eq!(
                err.lines().next().unwrap_or(""),
                failure(*levels, cause),
                "{filter} {args:?}: stderr"
            );
            assert_eq!(code, 1, "{filter} {args:?}: exit code");
        }
    }
}

#[test]
fn a_computed_non_finite_float_from_a_document_fails_alike_6867() {
    let input = "a: 1\nb: [2, 3]\n";
    for (filter, levels, cause) in [
        (".a / 0", 1, "json: unsupported value: +Inf"),
        (".b[] / 0", 1, "json: unsupported value: +Inf"),
        ("[.a / 0]", 2, "json: unsupported value: +Inf"),
        (".b | map(. / 0)", 2, "json: unsupported value: +Inf"),
    ] {
        let (out, err, code) = run(filter, input, &["-o=json"]);
        assert_eq!(out, "", "{filter}: stdout");
        assert_eq!(
            err.lines().next().unwrap_or(""),
            failure(levels, cause),
            "{filter}: stderr"
        );
        assert_eq!(code, 1, "{filter}: exit code");
    }
}

#[test]
fn raw_output_prints_a_non_finite_scalars_own_text_6867() {
    for (filter, text) in [("1 / 0", "+Inf\n"), ("0 / 0", "NaN\n")] {
        let (out, err, code) = run(filter, "", &["-n", "-r", "-o=json"]);
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (text, "", 0),
            "{filter}"
        );
    }
    // A container is still encoded, so `-r` does not rescue it.
    let (out, err, code) = run("[1 / 0]", "", &["-n", "-r", "-o=json"]);
    assert_eq!(out, "");
    assert_eq!(
        err.lines().next().unwrap_or(""),
        failure(2, "json: unsupported value: +Inf")
    );
    assert_eq!(code, 1);
    // A document scalar prints the spelling it was written with.
    for (input, text) in [
        ("a: .inf\n", ".inf\n"),
        ("a: -.INF\n", "-.INF\n"),
        ("a: .NaN\n", ".NaN\n"),
    ] {
        let (out, err, code) = run(".a", input, &["-r", "-o=json"]);
        assert_eq!((out.as_str(), err.as_str(), code), (text, "", 0), "{input}");
    }
}

/// (input, filter, wraps, the scalar's text).
const SOURCE: &[(&str, &str, usize, &str)] = &[
    ("a: .inf\n", ".a", 1, ".inf"),
    ("a: .inf\nb: .nan\n", ".", 2, ".inf"),
    ("a: +.inf\n", ".a", 1, "+.inf"),
    ("a: -.INF\n", ".", 2, "-.INF"),
    ("a: [1, .NaN]\n", ".", 3, ".NaN"),
    ("a: [1, .NaN]\n", ".a", 2, ".NaN"),
    ("a: {b: {c: .inf}}\n", ".a", 3, ".inf"),
    ("a: {b: {c: .inf}}\n", ".a.b", 2, ".inf"),
    ("a: {b: {c: .inf}}\n", ".a.b.c", 1, ".inf"),
    ("a: !!float .inf\n", ".", 2, ".inf"),
    ("a: !!float .inf\n", ".a", 1, ".inf"),
    // A sequence the program builds is a node of its own.
    ("a: [.inf, 1]\n", "[.a]", 3, ".inf"),
    ("a: [.inf, 1]\n", "[.a[]]", 2, ".inf"),
    ("a: [.inf, 1]\n", ".a | [.]", 3, ".inf"),
    ("a: [.inf, 1]\n", "{\"x\": .a}", 3, ".inf"),
    // An alias and its target are one node.
    ("a: &x .inf\nb: *x\n", ".b", 1, ".inf"),
    ("a: &x .inf\nb: *x\n", ".", 2, ".inf"),
    ("a: &x [.inf]\nb: *x\n", ".b", 2, ".inf"),
    ("a: &x [.inf]\nb: *x\n", ".", 3, ".inf"),
];

#[test]
fn a_document_non_finite_scalar_fails_as_yqs_encoder_does_6867() {
    for (input, filter, levels, text) in SOURCE {
        for args in [&["-o=json"][..], &["-o=json", "-I0"], &["-o=json", "-P"]] {
            let (_, err, code) = run(filter, input, args);
            assert_eq!(
                err.lines().next().unwrap_or(""),
                failure(*levels, &parse_float(text)),
                "{input:?} {filter} {args:?}: stderr"
            );
            assert_eq!(code, 1, "{input:?} {filter} {args:?}: exit code");
        }
    }
}

#[test]
fn the_results_before_a_failing_document_scalar_are_written_6867() {
    // `.[]`: the `1` is a result of its own and is out before the `.inf` fails.
    let (out, err, code) = run(".[]", "b: 1\na: .inf\n", &["-o=json"]);
    assert_eq!(out, "1\n");
    assert_eq!(
        err.lines().next().unwrap_or(""),
        failure(1, &parse_float(".inf"))
    );
    assert_eq!(code, 1);
    // Two documents: the first is whole, the second is the one that fails.
    let (out, err, code) = run(".", "b: 1\n---\na: .inf\n", &["-o=json"]);
    assert!(out.starts_with("{\n  \"b\": 1\n}\n"), "stdout: {out:?}");
    assert!(!out.contains("null"), "stdout: {out:?}");
    assert_eq!(
        err.lines().next().unwrap_or(""),
        failure(2, &parse_float(".inf"))
    );
    assert_eq!(code, 1);
}

#[test]
fn what_is_not_a_non_finite_float_is_unchanged_6867() {
    // Quoted, an unknown spelling and the YAML output stay as they were: text, and a float.
    for (input, args, expected) in [
        (
            "a: \".inf\"\n",
            &["-o=json", "-I0"][..],
            "{\"a\":\".inf\"}\n",
        ),
        ("a: '.nan'\n", &["-o=json", "-I0"], "{\"a\":\".nan\"}\n"),
        ("a: .infx\n", &["-o=json", "-I0"], "{\"a\":\".infx\"}\n"),
        ("a: .inf\n", &["-o=yaml"], "a: .inf\n"),
        ("a: 1e3\n", &["-o=json", "-I0"], "{\"a\":1e3}\n"),
    ] {
        let (out, err, code) = run(".", input, args);
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (expected, "", 0),
            "{input:?}"
        );
    }
}
