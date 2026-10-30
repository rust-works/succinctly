//! #5233: no jq-mode program with a dynamic path target runs on the shipped resolver any
//! more. One that also holds a construct the path-mode interpreter does not run (the
//! succinctly extensions `key`, `file_index`, `parent`, bare `path` and the cursor-metadata
//! builtins) is refused, naming it, and a dynamic path program over a document the
//! interpreter cannot materialize whole raises the decode error. jq 1.7.1 rejects every one
//! of these programs at compile time (`key/0 is not defined`, ...), and both documents when
//! it parses them.
//!
//! The `unshared-containers` holdout refuses every such program with its own message, so these
//! pin the shipped shape only.
#![cfg(not(feature = "unshared-containers"))]

use anyhow::Result;

use crate::{names_fault, run_jq_full};

/// A dynamic path target beside an extension builtin, anywhere in the program, is refused
/// with the builtin's name, once, at jq's compile-error status (3), before any input is read:
/// jq rejects the extension at compile time too. These rows used to reach the shipped route's
/// path-context evaluator (#2289, #2642, #2974, #3036, #3134, #3369).
#[test]
fn a_dynamic_path_target_beside_an_extension_builtin_is_refused_5233() -> Result<()> {
    for (filter, input, name) in [
        (r#".a | (try path(1) catch "x"), key"#, r#"{"a":1}"#, "key"),
        (r#"path(@base64 "\(key)")"#, r#"{"foo":1}"#, "key"),
        (
            ".foo | . as $x | (parent, ($x.a = 9))",
            r#"{"foo":{"a":1},"b":2}"#,
            "parent",
        ),
        (
            r#".a[] | path(@base64 "\(parent(1)|tostring)")"#,
            r#"{"a":{"foo":1}}"#,
            "parent",
        ),
        (
            ".foo | . as $x | sort | (path | empty), ($x[0] = 9)",
            r#"{"foo":[{"a":1}]}"#,
            "path",
        ),
        (
            "at_offset(11) as $x | del(.a | $x)",
            r#"{"a":1,"a":1}"#,
            "at_offset",
        ),
        (
            "file_index, path(.a as $x | .b)",
            r#"{"a":1}"#,
            "file_index",
        ),
        ("line, path(.a as $x | .b)", r#"{"a":1}"#, "line"),
    ] {
        // Two documents: the refusal is the program's, so it is reported once.
        let (stdout, stderr, code) =
            run_jq_full(&["-c", filter], Some(&format!("{input} {input}")))?;
        assert_eq!((stdout.as_str(), code), ("", 3), "`{filter}`: {stderr}");
        assert_eq!(
            stderr,
            format!(
                "jq: error: {name} is not supported in a program whose path expression does more \
                 than navigate\njq: 1 compile error\n"
            ),
            "`{filter}`"
        );
    }
    // With no input at all, as jq's compile error does.
    let (stdout, _, code) = run_jq_full(&["-c", "key, path(.a as $x | .b)"], Some(""))?;
    assert_eq!((stdout.as_str(), code), ("", 3));
    Ok(())
}

/// The same builtins beside path targets that only navigate stay on the shipped route and
/// answer as before.
#[test]
fn an_extension_builtin_beside_a_static_path_target_still_answers_5233() -> Result<()> {
    for (filter, input, want) in [
        (
            "(.a[] | select(. > 1)) |= (parent | length)",
            r#"{"a":[1,2,3]}"#,
            "{\"a\":[1,3,3]}\n",
        ),
        (".a | path(.b), key", r#"{"a":{"b":1}}"#, "[\"b\"]\n\"a\"\n"),
        ("del(.a), line", r#"{"a":1}"#, "{}\n1\n"),
        // #5243: a comma-valued index key only computes a value, so the target is
        // static (the answers `main` pinned before #5223).
        (".a[(0,1)] |= key", r#"{"a":[1,2,3]}"#, "{\"a\":[0,1,3]}\n"),
        (
            ".a[(0,1)] |= parent",
            r#"{"a":[1,2,3]}"#,
            "{\"a\":[[1,2,3],[1,2,3],3]}\n",
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(input))?;
        assert_eq!((stdout.as_str(), code), (want, 0), "`{filter}`: {stderr}");
    }
    Ok(())
}

/// A dynamic path program over a document the interpreter cannot materialize raises the decode
/// error instead of answering from the parts it reads: a lone surrogate, which jq rejects at
/// parse time, and a scalar at the 257th level, which jq's parser accepts and the materializer
/// refuses (#5497: a document past jq's parse-depth limit is refused before any filter runs). A
/// program with no dynamic target still reads only what it navigates (#4412).
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq's `{k:.}`, not a format argument
fn a_dynamic_path_program_over_an_undecodable_document_raises_5233() -> Result<()> {
    let lone = r#"{"a":"\ud800","d":5}"#;
    let deep = format!("{}1{}", "[".repeat(256), "]".repeat(256));
    for (filter, input, message) in [
        (
            "[.d | path(if . then . else empty end)]",
            lone,
            "invalid unicode escape sequence",
        ),
        (
            ". as $y | [path($y)]",
            deep.as_str(),
            "nesting depth exceeds limit of 256",
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(input))?;
        assert_eq!((stdout.as_str(), code), ("", 5), "`{filter}`: {stderr}");
        assert!(names_fault(&stderr, message), "`{filter}`: {stderr}");
    }
    let (stdout, stderr, code) = run_jq_full(&["-c", "[.d | path(.)]"], Some(lone))?;
    assert_eq!((stdout.as_str(), code), ("[[]]\n", 0), "{stderr}");
    Ok(())
}

/// A static target the shipped evaluator rewrites as it runs (a `$v` substituted into
/// `.a[$v]`, a bound `def`) and hands to its own bridges stays on that evaluator: the bridges
/// re-enter it unrouted. Routing them read the rewritten target as dynamic and refused these
/// plain jq programs (review of #5233). Expected outputs are jq 1.7.1's.
#[test]
fn a_rewritten_static_target_stays_on_the_shipped_route_5233() -> Result<()> {
    let input = r#"{"a":{"x":1,"y":2},"k":"x"}"#;
    for (filter, want) in [
        (
            r#"def f($v): .a[$v] |= 9; [limit(1; f("x"))]"#,
            "[{\"a\":{\"x\":9,\"y\":2},\"k\":\"x\"}]\n",
        ),
        (
            r#"def h: "x"; .k as $k | [limit(1; .a[$k] |= h)]"#,
            "[{\"a\":{\"x\":\"x\",\"y\":2},\"k\":\"x\"}]\n",
        ),
        (
            "def f: .; .k as $k | [limit(1; .a[$k] |= f)]",
            "[{\"a\":{\"x\":1,\"y\":2},\"k\":\"x\"}]\n",
        ),
        (
            r#"def f($v): .a[$v] |= 9; first(f("x"))"#,
            "{\"a\":{\"x\":9,\"y\":2},\"k\":\"x\"}\n",
        ),
        (
            r#"def g($v): .a[$v] |= 9; [has("a"), (g("x")|has("a"))]"#,
            "[true,true]\n",
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(input))?;
        assert_eq!((stdout.as_str(), code), (want, 0), "`{filter}`: {stderr}");
    }
    Ok(())
}
