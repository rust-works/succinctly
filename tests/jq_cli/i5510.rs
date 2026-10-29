//! #5510: jq records each constant non-string object key as its own compile error and goes on
//! parsing, so a program with several prints them all and ends `jq: N compile errors`, where
//! `succinctly jq` stopped at the first and said `1 compile error`. Every row was captured from
//! jq 1.7.1 (`/usr/bin/jq -n`): the `jq: error:` lines in order, and the trailer. The echoed
//! source line under each differs only in trailing padding, and the wording of a *syntax* error
//! after the key errors is succinctly's own (both recorded in `limitations.md`), so only the
//! first line of that one is compared.

use anyhow::Result;

use crate::run_jq_full;

/// The `jq: error: ...` lines of `stderr` and its last line.
fn report(stderr: &str) -> (Vec<&str>, &str) {
    let errors = stderr
        .lines()
        .filter(|line| line.starts_with("jq: error: "))
        .collect();
    (errors, stderr.lines().last().unwrap_or(""))
}

#[test]
fn test_every_constant_non_string_key_is_reported_and_counted_5510() -> Result<()> {
    for (filter, errors, trailer) in [
        (
            "{(0):1,(1):2}",
            &[
                "jq: error: Cannot use number (0) as object key at <top-level>, line 1:",
                "jq: error: Cannot use number (1) as object key at <top-level>, line 1:",
            ][..],
            "jq: 2 compile errors",
        ),
        (
            "{(0):1,\"a\":2,(1):3}",
            &[
                "jq: error: Cannot use number (0) as object key at <top-level>, line 1:",
                "jq: error: Cannot use number (1) as object key at <top-level>, line 1:",
            ][..],
            "jq: 2 compile errors",
        ),
        (
            "{(0):1,(null):2} | {(true):3}",
            &[
                "jq: error: Cannot use number (0) as object key at <top-level>, line 1:",
                "jq: error: Cannot use null (null) as object key at <top-level>, line 1:",
                "jq: error: Cannot use boolean (true) as object key at <top-level>, line 1:",
            ][..],
            "jq: 3 compile errors",
        ),
        // A destructuring pattern takes the same check.
        (
            ". as {(0):$a,(1):$b} | 1",
            &[
                "jq: error: Cannot use number (0) as object key at <top-level>, line 1:",
                "jq: error: Cannot use number (1) as object key at <top-level>, line 1:",
            ][..],
            "jq: 2 compile errors",
        ),
        // Wherever they sit in the program: a definition, a branch, an interpolation, a
        // nested object, a `reduce` update.
        (
            "def f: {(0):1}; {(1):2}",
            &[
                "jq: error: Cannot use number (0) as object key at <top-level>, line 1:",
                "jq: error: Cannot use number (1) as object key at <top-level>, line 1:",
            ][..],
            "jq: 2 compile errors",
        ),
        (
            "if true then {(0):1} else {(1):2} end",
            &[
                "jq: error: Cannot use number (0) as object key at <top-level>, line 1:",
                "jq: error: Cannot use number (1) as object key at <top-level>, line 1:",
            ][..],
            "jq: 2 compile errors",
        ),
        (
            "\"\\({(0):1}) \\({(1):2})\"",
            &[
                "jq: error: Cannot use number (0) as object key at <top-level>, line 1:",
                "jq: error: Cannot use number (1) as object key at <top-level>, line 1:",
            ][..],
            "jq: 2 compile errors",
        ),
        (
            "{a:{(0):1},b:{(1):2}}",
            &[
                "jq: error: Cannot use number (0) as object key at <top-level>, line 1:",
                "jq: error: Cannot use number (1) as object key at <top-level>, line 1:",
            ][..],
            "jq: 2 compile errors",
        ),
        (
            "reduce range(3) as $i ({}; {(0):$i}) | {(1):2}",
            &[
                "jq: error: Cannot use number (0) as object key at <top-level>, line 1:",
                "jq: error: Cannot use number (1) as object key at <top-level>, line 1:",
            ][..],
            "jq: 2 compile errors",
        ),
        // One is still one.
        (
            "{(0):1}",
            &["jq: error: Cannot use number (0) as object key at <top-level>, line 1:"][..],
            "jq: 1 compile error",
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-n", filter], None)?;
        assert_eq!((stdout.as_str(), code), ("", 3), "{filter}");
        assert_eq!(report(&stderr), (errors.to_vec(), trailer), "{filter}");
    }
    Ok(())
}

#[test]
fn test_a_syntax_error_after_a_key_error_is_reported_with_it_5510() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-n", "{(0):1} )"], None)?;
    assert_eq!((stdout.as_str(), code), ("", 3));
    let (errors, trailer) = report(&stderr);
    assert_eq!(
        errors.first(),
        Some(&"jq: error: Cannot use number (0) as object key at <top-level>, line 1:")
    );
    assert_eq!(errors.len(), 2, "{stderr}");
    assert_eq!(trailer, "jq: 2 compile errors");
    Ok(())
}

#[test]
fn test_string_keys_and_computed_keys_still_compile_5510() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-n", "-c", "{(\"a\"):1,(\"b\"):2}"], None)?;
    assert_eq!((stdout.as_str(), code), ("{\"a\":1,\"b\":2}\n", 0));
    Ok(())
}
