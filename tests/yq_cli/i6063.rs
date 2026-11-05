//! #6063: `succinctly yq`'s `--argjson` reads a JSON number written with a bare trailing dot
//! (`5.`) the way yq's own JSON reader reads it (`printf '{"n":5.}' | yq -p json .` is
//! `{"n":5}`, v4.53.3). #6052 made the jq side read it; this flag is yq's own extension, so the
//! reference it follows is the `-p json` reader, which the same rows are compared against.

use crate::run_yq_stdin;

/// `(token, what --argjson and the -p json reader both print)`, each captured from yq v4.53.3's
/// `-p json` reader (`printf '{"n":<token>}' | yq -p json -o json -I0 .`).
const ROWS: &[(&str, &str)] = &[
    ("5.", r#"{"n":5}"#),
    ("-5.", r#"{"n":-5}"#),
    ("0.", r#"{"n":0}"#),
    ("-0.", r#"{"n":0}"#),
    ("007.", r#"{"n":7}"#),
    ("12345678901234567.", r#"{"n":12345678901234568}"#),
    ("123456789012345678901.", r#"{"n":1.2345678901234568e+20}"#),
    ("5.e0", r#"{"n":5}"#),
    ("1.e5", r#"{"n":100000}"#),
    ("-9223372036854775808.", r#"{"n":-9223372036854775808}"#),
    // Go's saturating `f == float64(int64(f))`: 2^63 saturates to `i64::MAX`, 1e19 stays a float.
    ("9223372036854775808.", r#"{"n":9223372036854775807}"#),
    ("9223372036854775807.", r#"{"n":9223372036854775807}"#),
    ("10000000000000000000.", r#"{"n":1e+19}"#),
];

#[test]
fn argjson_bare_trailing_dot_reads_as_the_json_reader_does_6063() {
    for (token, expected) in ROWS {
        let (output, code) = run_yq_stdin(
            ".n = $n",
            "{}",
            &["--argjson", "n", token, "-o=json", "-I=0"],
        )
        .unwrap();
        assert_eq!(code, 0, "--argjson n {token}: {output:?}");
        assert_eq!(output.trim(), *expected, "--argjson n {token}");

        let json = format!(r#"{{"n":{token}}}"#);
        let (output, code) = run_yq_stdin(".", &json, &["-p", "json", "-o=json", "-I=0"]).unwrap();
        assert_eq!(code, 0, "-p json {json}: {output:?}");
        assert_eq!(output.trim(), *expected, "-p json {json}");
    }
}

#[test]
fn argjson_bare_trailing_dot_nested_and_yaml_output_6063() {
    for (value, expected) in [
        ("[1., 2.]", "[1,2]"),
        ("+5.", "5"),
        (r#"{"a":5.,"b":[-0.]}"#, r#"{"a":5,"b":[0]}"#),
    ] {
        let (output, code) =
            run_yq_stdin("$x", "a: 1", &["--argjson", "x", value, "-o=json", "-I=0"]).unwrap();
        assert_eq!(code, 0, "--argjson x {value}: {output:?}");
        assert_eq!(output.trim(), expected, "--argjson x {value}");
    }
    // The integer prints as an integer in YAML, not `5.0`.
    let (output, code) = run_yq_stdin(".n = $n", "{}", &["--argjson", "n", "5."]).unwrap();
    assert_eq!((output.as_str(), code), ("n: 5\n", 0));
}

/// A fraction that is not whole stays a float, and a dot with no integer part is still no
/// number (jq refuses it too).
#[test]
fn argjson_dot_without_an_integer_part_or_with_a_second_dot_stays_refused_6063() {
    let (output, code) = run_yq_stdin(
        ".n = $n",
        "{}",
        &["--argjson", "n", ".5", "-o=json", "-I=0"],
    )
    .unwrap();
    assert_eq!((output.trim(), code), (r#"{"n":0.5}"#, 0));
    for value in [".", "-.", "1.5.", "1..", "1.x", "[1.x]"] {
        let (output, code) = run_yq_stdin("$x", "a: 1", &["--argjson", "x", value]).unwrap();
        assert_ne!(code, 0, "--argjson x {value} must be rejected: {output:?}");
        assert_eq!(output, "", "--argjson x {value}");
    }
}

/// The document reader shares the materializer: `--input-format json` with `--slurp` read a
/// bare trailing dot as the integer too.
#[test]
fn json_input_bare_trailing_dot_is_the_integer_under_slurp_and_eval_all_6063() {
    for (filter, flag) in [(".[0].n", "--slurp"), (".n", "--eval-all")] {
        let (output, code) = run_yq_stdin(
            filter,
            r#"{"n":5.}"#,
            &["--input-format", "json", flag, "-o=json", "-I=0"],
        )
        .unwrap();
        assert_eq!((output.trim(), code), ("5", 0), "{flag}");
    }
    let (output, code) = run_yq_stdin(
        ".[0]",
        "[1., -0., 2.5]",
        &["--input-format", "json", "--slurp", "-o=json", "-I=0"],
    )
    .unwrap();
    assert_eq!((output.trim(), code), ("[1,0,2.5]", 0));
}
