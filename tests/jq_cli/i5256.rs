//! #5256: the path-mode interpreter writes a path's value into the document in place, so a
//! `reduce` whose target stays dynamic no longer copies its accumulator on every step. The
//! writes wait for the paths to finish; these pin that nothing else moved -- the answers, and
//! which error comes first when a write and the path generator both raise.

use anyhow::Result;

use crate::run_jq_full;

/// Dynamic targets (`getpath` is not a path the streaming walkers take) on the interpreter, as
/// jq 1.7.1 answers them (captured live): several paths, a path twice, a value generator, an
/// empty update, nested paths in either order, and a fold of them.
#[test]
fn test_dynamic_target_writes_answer_as_jq_5256() -> Result<()> {
    for (input, filter, want) in [
        (
            "[1,2,3,4]",
            "(getpath([0]), getpath([1])) |= . * 2",
            "[2,4,3,4]",
        ),
        (
            "[1,2,3,4]",
            "(getpath([0]), getpath([9])) += 1",
            "[2,2,3,4,null,null,null,null,null,1]",
        ),
        ("[1,2,3]", "(getpath([0]), getpath([0])) += 1", "[3,2,3]"),
        ("[1,2,3]", "(getpath([0]), getpath([0])) = 7", "[7,2,3]"),
        (
            "[1,2,3]",
            "[(getpath([0]), getpath([1])) |= (., .+10)]",
            "[[1,2,3]]",
        ),
        (
            "[1,2,3]",
            "(getpath([0]), getpath([1]), getpath([2])) |= empty",
            "[]",
        ),
        (
            r#"{"a":[1,2]}"#,
            r#"[(getpath(["a",0])) = (.a[], 5)]"#,
            r#"[{"a":[1,2]},{"a":[2,2]},{"a":[5,2]}]"#,
        ),
        (
            r#"{"a":[1,2]}"#,
            r#"[(getpath(["a",0])) += (1, 2)]"#,
            r#"[{"a":[2,2]},{"a":[3,2]}]"#,
        ),
        (r#"{"a":[1,2]}"#, r#"[(getpath(["a",0])) += empty]"#, "[]"),
        (
            r#"{"a":[1,2]}"#,
            r#"[limit(1; (getpath(["a",0])) = (1, error("late")))]"#,
            r#"[{"a":[1,2]}]"#,
        ),
        (
            r#"{"a":[1,2]}"#,
            r#"(getpath(["a",0]), getpath(["a",1])) = ."#,
            r#"{"a":[{"a":[1,2]},{"a":[1,2]}]}"#,
        ),
        (
            r#"{"a":[null,2]}"#,
            r#"(getpath(["a",0]), getpath(["a",1])) //= 3"#,
            r#"{"a":[3,2]}"#,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r#"(getpath(["a"]), getpath(["a","b"])) |= (if type == "object" then {"b":5} else . + 1 end)"#,
            r#"{"a":{"b":6}}"#,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r#"(getpath(["a","b"]), getpath(["a"])) = 3"#,
            r#"{"a":3}"#,
        ),
        // A value that is not there: no output, from `=` and from `+=`.
        (r#"{"a":1}"#, r#"(getpath(["b"])) = (.a.c?)"#, ""),
        (r#"{"a":1}"#, r#"(getpath(["b"])) += (.a.c?)"#, ""),
        (
            r#"{"a":[1,2]}"#,
            r#"reduce range(3) as $i (.; (getpath(["a",$i]), getpath(["b"])) |= ($i))"#,
            r#"{"a":[0,1,2],"b":2}"#,
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(input))?;
        assert_eq!((stdout.trim_end(), code), (want, 0), "`{filter}`: {stderr}");
    }
    Ok(())
}

/// A path the generator raises after earlier ones were written: the earlier write's own error
/// comes first, as in jq (the writes are held until the paths are done, so the interpreter
/// raises the write's error ahead of the generator's); a write that succeeds leaves the
/// generator's error to be raised.
#[test]
fn test_write_error_comes_before_a_later_path_error_5256() -> Result<()> {
    for (input, filter, want_stderr) in [
        (
            "[]",
            r#"(.[-5], error("x")) = 1"#,
            "Out of bounds negative array index",
        ),
        (
            "[]",
            r#"(.[-5], error("x")) |= 1"#,
            "Out of bounds negative array index",
        ),
        ("[1,2]", r#"(.[0], error("x")) += 9"#, "x"),
        (
            r#"{"a":[1,2]}"#,
            r#"(getpath(["a",0]), getpath(["a","x"]), error("z")) = 5"#,
            r#"Cannot index array with string "x""#,
        ),
        (
            r#"{"a":[1,2]}"#,
            r#"(getpath(["a",0]), getpath(["a",1]), error("z")) |= 5"#,
            "z",
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(input))?;
        assert_eq!((stdout.as_str(), code), ("", 5), "`{filter}`: {stderr}");
        assert!(
            stderr.trim_end().ends_with(want_stderr),
            "`{filter}`: {stderr}"
        );
    }
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"try ((.[0], error("x")) = 9) catch ."#],
        Some("[1,2]"),
    )?;
    assert_eq!((stdout.trim_end(), code), (r#""x""#, 0), "{stderr}");
    Ok(())
}

/// The writes are held back for at most `WRITE_LOOKAHEAD` (4,096) paths; around that bound the
/// answer and the first error are still jq's (captured live): the paths on either side of it, a
/// write that fails inside the flush or after it, and a path error after the flush.
#[test]
fn test_write_lookahead_bound_answers_as_jq_5256() -> Result<()> {
    for (n, want) in [
        (4095, "8386560"),
        (4096, "8390656"),
        (4097, "8394753"),
        (5000, "12502500"),
    ] {
        let filter = format!("[range({n})] | ((if true then .[] else empty end) |= . + 1) | add");
        let (stdout, stderr, code) = run_jq_full(&["-n", "-c", &filter], None)?;
        assert_eq!((stdout.trim_end(), code), (want, 0), "`{filter}`: {stderr}");
    }
    for (filter, want_stderr) in [
        (
            r#"[range(5000)] | ((.[-10000], (if true then .[] else empty end), error("x")) = 1)"#,
            "Out of bounds negative array index",
        ),
        (
            r#"[range(5000)] | (((if true then .[] else empty end), error("x")) = 1)"#,
            "x",
        ),
        (
            "[range(5000)] | (((if true then .[] else empty end), .[-10000]) = 1)",
            "Out of bounds negative array index",
        ),
        (
            r#"[range(5000)] | (((if true then .[] else empty end), .[-10000], error("x")) |= 1)"#,
            "Out of bounds negative array index",
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-n", "-c", filter], None)?;
        assert_eq!((stdout.as_str(), code), ("", 5), "`{filter}`: {stderr}");
        assert!(
            stderr.trim_end().ends_with(want_stderr),
            "`{filter}`: {stderr}"
        );
    }
    Ok(())
}
