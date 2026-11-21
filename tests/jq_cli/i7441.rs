//! #7441: `halt_error(x)` with a non-number `x` raises `<type> (<input>) halt_error/1: number required`.
//!
//! jq's `f_halt_error` raises `type_error(input, "halt_error/1: number required")`, so the message
//! names the INPUT value (a jq quirk), previewed the way every `type_error` is: the compact JSON
//! cut to 10 bytes and `...` after a cut. Every argument that is not a number gives it, the same
//! whatever it is, and a `try` catches it. `nan` as an argument is a number, and as the input it
//! prints as `number (null)`. Every row captured from `/usr/bin/jq` 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

/// `(input as JSON, jq's message)`.
const INPUTS: &[(&str, &str)] = &[
    ("null", "null (null) halt_error/1: number required"),
    ("\"in\"", "string (\"in\") halt_error/1: number required"),
    ("1", "number (1) halt_error/1: number required"),
    ("1.5", "number (1.5) halt_error/1: number required"),
    ("true", "boolean (true) halt_error/1: number required"),
    ("[]", "array ([]) halt_error/1: number required"),
    ("[1,2]", "array ([1,2]) halt_error/1: number required"),
    ("{}", "object ({}) halt_error/1: number required"),
    (
        "{\"a\":1}",
        "object ({\"a\":1}) halt_error/1: number required",
    ),
    ("\"\"", "string (\"\") halt_error/1: number required"),
    (
        "\"a long string that is longer than the preview limit allows\"",
        "string (\"a long str...) halt_error/1: number required",
    ),
    (
        "{\"a\":\"a long string that is longer than the preview\",\"b\":[1,2,3]}",
        "object ({\"a\":\"a lon...) halt_error/1: number required",
    ),
    (
        "\"é€😀 and more text here\"",
        "string (\"é€😀 ...) halt_error/1: number required",
    ),
    (
        "[[[[[1]]]]]",
        "array ([[[[[1]]]]]) halt_error/1: number required",
    ),
    ("nan", "number (null) halt_error/1: number required"),
];

/// The arguments that are not numbers; the first one to fail is the one reported, so a generator
/// of them (`"a",2`) fails at `"a"`.
const ARGUMENTS: &[&str] = &[
    "\"x\"",
    "null",
    "true",
    "false",
    "[1]",
    "[]",
    "{\"a\":1}",
    "{}",
    "\"a\",2",
];

/// Caught with `try`, on a `-n` literal and on a document read from stdin (two routes).
#[test]
fn test_7441_message_names_the_input_caught() -> Result<()> {
    for &(input, message) in INPUTS {
        let want = format!("{}\n", serde_json::to_string(message)?);
        for arg in ARGUMENTS {
            let filter = format!("try halt_error({arg}) catch .");
            let (stdout, stderr, code) = run_jq_full(&["-c", &filter], Some(input))?;
            assert_eq!(
                (stdout.as_str(), stderr.as_str(), code),
                (want.as_str(), "", 0),
                "{input} | {filter} (stdin)"
            );
            let literal = format!("{input} | {filter}");
            let (stdout, stderr, code) = run_jq_full(&["-nc", &literal], None)?;
            assert_eq!(
                (stdout.as_str(), stderr.as_str(), code),
                (want.as_str(), "", 0),
                "{literal} (-n)"
            );
        }
    }
    Ok(())
}

/// Uncaught: the message on stderr with its input position, exit 5, nothing on stdout. A `-n`
/// run has no position (`<unknown>`).
#[test]
fn test_7441_message_names_the_input_uncaught() -> Result<()> {
    for &(input, message) in INPUTS {
        for arg in ARGUMENTS {
            let filter = format!("halt_error({arg})");
            // The newline ends the input's line, which is what the `:1` counts (`echo`'s).
            let (stdout, stderr, code) = run_jq_full(&[&filter], Some(&format!("{input}\n")))?;
            assert_eq!(
                (stdout.as_str(), stderr.as_str(), code),
                (
                    "",
                    format!("jq: error (at <stdin>:1): {message}\n").as_str(),
                    5
                ),
                "{input} | {filter} (stdin)"
            );
            let literal = format!("{input} | {filter}");
            let (stdout, stderr, code) = run_jq_full(&["-n", &literal], None)?;
            assert_eq!(
                (stdout.as_str(), stderr.as_str(), code),
                (
                    "",
                    format!("jq: error (at <unknown>): {message}\n").as_str(),
                    5
                ),
                "{literal} (-n)"
            );
        }
    }
    Ok(())
}

/// `?` swallows it like any error, `//` sees it as empty, and an argument that yields a number
/// first (`1, "x"`) halts before the bad one is read.
#[test]
fn test_7441_optional_and_alternative_and_generator_argument() -> Result<()> {
    let rows: &[(&str, &str, i32)] = &[
        (r#""in" | [halt_error("x")?]"#, "[]\n", 0),
        (r#""in" | (halt_error("x"))? // 7"#, "7\n", 0),
        (r#""in" | halt_error("x")? , 1"#, "1\n", 0),
        (
            r#"[1] | map(try halt_error("x") catch .)"#,
            "[\"number (1) halt_error/1: number required\"]\n",
            0,
        ),
        (
            r#""in" | try (halt_error("x")) catch (.|length)"#,
            "43\n",
            0,
        ),
        (
            r#"try ("in" | halt_error("x")) catch ."#,
            "\"string (\\\"in\\\") halt_error/1: number required\"\n",
            0,
        ),
    ];
    for &(filter, want_out, want_code) in rows {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (want_out, "", want_code),
            "{filter}"
        );
    }
    // `halt_error(1, "x")` halts on the first code, with the input as the message.
    let (stdout, stderr, code) = run_jq_full(&["-n", r#""m" | halt_error(1, "x")"#], None)?;
    assert_eq!((stdout.as_str(), stderr.as_str(), code), ("", "m", 1));
    Ok(())
}
