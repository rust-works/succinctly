//! #5243: a path target whose index key only computes a value is static, so it stays on the
//! streaming walkers instead of the path-mode interpreter (which copied a `reduce`'s accumulator
//! on every step, quadratic in the input).

use anyhow::Result;

use crate::run_jq_full;

/// A value-only index key, on both routes (stdin and `-n` with `input`), as jq 1.7.1 answers
/// (captured live). jq runs the key as a subexpression on the target's *input*, so `.a[.k]` reads
/// the root's `.k` and `path(.b[length - 1])` takes the root's length.
#[test]
fn test_value_only_index_key_answers_as_jq_5243() -> Result<()> {
    let records = r#"[{"name":"a","score":1},{"name":"b","score":2},{"name":"a","score":3}]"#;
    for (input, filter, want) in [
        (
            records,
            "reduce .[] as $r ({}; .[$r.name] = $r.score)",
            r#"{"a":3,"b":2}"#,
        ),
        (
            records,
            "reduce .[] as $r ({}; .[$r.name] += $r.score)",
            r#"{"a":4,"b":2}"#,
        ),
        (
            r#"{"a":{"x":1,"y":2},"k":"x"}"#,
            ".a[.k] |= . + 10",
            r#"{"a":{"x":11,"y":2},"k":"x"}"#,
        ),
        (
            r#"{"a":[1,2,3],"i":1}"#,
            "del(.a[.i, .i + 1])",
            r#"{"a":[1],"i":1}"#,
        ),
        (r#"{"b":[10,20,30]}"#, "path(.b[length - 1])", r#"["b",0]"#),
        // A NaN key: the walker carries it as jq does (#5245).
        ("[1,2]", "path(.[nan])", "[null]"),
        // A key holding an assignment of its own, static or not.
        (
            r#"{"a":1}"#,
            ".[(.a = 7) | .a | tostring] = 2",
            r#"{"a":1,"7":2}"#,
        ),
        (
            r#"{"a":1}"#,
            ".[(.a |= . + 1) | .a | tostring] = 2",
            r#"{"a":1,"2":2}"#,
        ),
        (
            r#"{"a":1}"#,
            ".[(.a += 3) | .a | tostring] = 2",
            r#"{"a":1,"4":2}"#,
        ),
        (
            r#"{"a":1}"#,
            ".[(.z //= \"q\") | .z] = 2",
            r#"{"a":1,"q":2}"#,
        ),
        (r#"{"a":1}"#, "[path(.[(.b = \"a\") | .b])]", r#"[["a"]]"#),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(input))?;
        assert_eq!((stdout.trim_end(), code), (want, 0), "`{filter}`: {stderr}");
        let (stdout, stderr, code) =
            run_jq_full(&["-n", "-c", &format!("input | {filter}")], Some(input))?;
        assert_eq!(
            (stdout.trim_end(), code),
            (want, 0),
            "-n `{filter}`: {stderr}"
        );
    }
    Ok(())
}

/// A key that calls a function keeps the target dynamic, so it runs on the path-mode interpreter,
/// with jq's answer (the holdout refuses every dynamic target).
#[test]
fn test_function_call_in_an_index_key_runs_on_the_interpreter_5243() -> Result<()> {
    let filter = "def f: 9; .[(.a = f) | .a | tostring] = 2";
    let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(r#"{"a":1}"#))?;
    assert_eq!(
        (stdout.trim_end(), code),
        (r#"{"a":1,"9":2}"#, 0),
        "`{filter}`: {stderr}"
    );
    Ok(())
}

/// A succinctly extension in an index key keeps the target dynamic, so the program is refused
/// at compile time, as before #5243: the walker evaluates a key off the cursor and would read
/// the wrong node (`.a[line | tostring]` wrote key `"0"` on line 3).
#[test]
fn test_extension_in_an_index_key_is_still_refused_5243() -> Result<()> {
    for (filter, name) in [(".a[line | tostring] = 9", "line"), (".[key] = 1", "key")] {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some("\n\n{\"a\":{\"3\":1}}"))?;
        assert_eq!((stdout.as_str(), code), ("", 3), "`{filter}`: {stderr}");
        assert!(
            stderr.contains(&format!("{name} is not supported in a program")),
            "`{filter}`: {stderr}"
        );
    }
    Ok(())
}
