//! #4941: a stage that hands its input on once ahead of a destructure of `.` inside `[ .. ]`, in a fold nested in a
//! `reduce` UPDATE.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

// In the body of a `reduce` whose source moved jq's register onto an element, `.` is not the register, so a
// destructure of it raises `near attempt to access element "b" of <accumulator>`, also behind a stage that only
// hands its input on (`select(true)`, `first(.)`, `limit(2; .)`, `(.)?`) inside the brackets of a nested fold's
// source. The binary read `[ .. ]` with such a stage as opaque and answered (`path()` printed `[]`, `del()`
// deleted the document, `|= 9` replaced it).

/// #4941: `wrapped-sel-true` -- a sample of the grid rows over the `rd-nav`, `rd-nava`, `rd-rec` and `rd-lit` outers (the
/// destructure behind this stage raises in jq, and the binary used to answer it); every row captured from
/// `/usr/bin/jq` 1.7.1, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_4941_select_true_ahead_of_a_bracketed_destructure_raises() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":1,"b":2}"#,
            r"path(.a as $x | reduce (1,2) as $i (.; foreach ([select(true) | . as {b:$b} | .]) as $y (.; .; .)))",
            "[]\n",
            r"",
            0,
        ),
        (
            r"[true]",
            r"del(.a as $x | reduce (1,2) as $i (.; foreach ([select(true) | . as {b:$b} | .]) as $y (.; .; .)))",
            r"",
            r#"Cannot index array with string "a""#,
            5,
        ),
        (
            r"null",
            r"del(.a as $x | reduce .. as $i (.; foreach ([select(true) | . as {b:$b} | .]) as $y (.; .; .)))",
            "null\n",
            r"",
            0,
        ),
        (
            r"[null]",
            r"(. as $x | reduce .a? as $i (.; foreach ([select(true) | . as {b:$b} | .]) as $y (.; .; .))) |= 9",
            "9\n",
            r"",
            0,
        ),
        (
            r#"{"a":false,"b":null}"#,
            r"(.a as $x | reduce .[]? as $i (.; foreach ([select(true) | . as {b:$b} | .]) as $y (.; $y; .))) |= 9",
            r"",
            r#"Invalid path expression near attempt to access element "b" of {"a":false,"b":null}"#,
            5,
        ),
        (
            r"[null]",
            r"(.a as $x | reduce (1,2) as $i (.; foreach ([select(true) | . as {b:$b} | .]) as $y (.; $y; .))) |= 9",
            r"",
            r#"Cannot index array with string "a""#,
            5,
        ),
        (
            r#"[{"a":1}]"#,
            r"del(. as $x | reduce .a? as $i (.; foreach ([select(true) | . as {b:$b} | .]) as $y (.; .; .)))",
            "null\n",
            r"",
            0,
        ),
        (
            r"null",
            r"(. as $x | reduce .[]? as $i (.; foreach ([select(true) | . as {b:$b} | .]) as $y (.; $y; .))) |= 9",
            "9\n",
            r"",
            0,
        ),
        (
            r"[null]",
            r"path(.a as $x | reduce .. as $i (.; reduce ([select(true) | . as {b:$b} | .]) as $y (.; .)))",
            r"",
            r#"Cannot index array with string "a""#,
            5,
        ),
        (
            r"[]",
            r"path(.a as $x | reduce .[]? as $i (.; reduce ([select(true) | . as {b:$b} | .]) as $y (.; .)))",
            r"",
            r#"Cannot index array with string "a""#,
            5,
        ),
    ])
}

/// #4941: `wrapped-sel-1` -- a sample of the grid rows over the `rd-nav`, `rd-nava`, `rd-rec` and `rd-lit` outers (the
/// destructure behind this stage raises in jq, and the binary used to answer it); every row captured from
/// `/usr/bin/jq` 1.7.1, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_4941_select_one_ahead_of_a_bracketed_destructure_raises() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r"[[1],[1]]",
            r"del(.a as $x | reduce .. as $i (.; foreach ([select(1) | . as {b:$b} | .]) as $y (.; .; .)))",
            r"",
            r#"Cannot index array with string "a""#,
            5,
        ),
        (
            r"[[1],[1]]",
            r"del(.a as $x | reduce .[]? as $i (.; foreach ([select(1) | . as {b:$b} | .]) as $y (.; .; .)))",
            r"",
            r#"Cannot index array with string "a""#,
            5,
        ),
        (
            r#"{"a":[true]}"#,
            r"del(.a as $x | reduce .a? as $i (.; foreach ([select(1) | . as {b:$b} | .]) as $y (.; .; .)))",
            r"",
            r#"Invalid path expression near attempt to access element "b" of {"a":[true]}"#,
            5,
        ),
        (
            r#"[{"a":1}]"#,
            r"del(.a as $x | reduce (1,2) as $i (.; foreach ([select(1) | . as {b:$b} | .]) as $y (.; $y; .)))",
            r"",
            r#"Cannot index array with string "a""#,
            5,
        ),
        (
            r"[]",
            r"path(. as $x | reduce .[]? as $i (.; foreach ([select(1) | . as {b:$b} | .]) as $y (.; $y; .)))",
            "[]\n",
            r"",
            0,
        ),
        (
            r#"{"a":false,"b":null}"#,
            r"path(. as $x | reduce .. as $i (.; reduce ([select(1) | . as {b:$b} | .]) as $y (.; .)))",
            r"",
            r#"Invalid path expression near attempt to access element "b" of {"a":false,"b":null}"#,
            5,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"(.a as $x | reduce .a? as $i (.; foreach ([select(1) | . as {b:$b} | .]) as $y (.; $y; .))) |= 9",
            r"",
            r#"Invalid path expression near attempt to access element "b" of {"a":1,"b":2}"#,
            5,
        ),
        (
            r"true",
            r"del(. as $x | reduce .. as $i (.; reduce ([select(1) | . as {b:$b} | .]) as $y (.; .)))",
            r"",
            r#"Cannot index boolean with string "b""#,
            5,
        ),
        (
            r"[null]",
            r"path(.a as $x | reduce .a? as $i (.; foreach ([select(1) | . as {b:$b} | .]) as $y (.; $y; .)))",
            r"",
            r#"Cannot index array with string "a""#,
            5,
        ),
        (
            r"[]",
            r"(. as $x | reduce .a? as $i (.; reduce ([select(1) | . as {b:$b} | .]) as $y (.; .))) |= 9",
            "9\n",
            r"",
            0,
        ),
    ])
}

/// #4941: `wrapped-first-dot` -- a sample of the grid rows over the `rd-nav`, `rd-nava`, `rd-rec` and `rd-lit` outers (the
/// destructure behind this stage raises in jq, and the binary used to answer it); every row captured from
/// `/usr/bin/jq` 1.7.1, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_4941_first_dot_ahead_of_a_bracketed_destructure_raises() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":true}"#,
            r"path(.a as $x | reduce (1,2) as $i (.; reduce ([first(.) | . as {b:$b} | .]) as $y (.; .)))",
            "[]\n",
            r"",
            0,
        ),
        (
            r"[true]",
            r"del(.a as $x | reduce (1,2) as $i (.; reduce ([first(.) | . as {b:$b} | .]) as $y (.; .)))",
            r"",
            r#"Cannot index array with string "a""#,
            5,
        ),
        (
            r"[null]",
            r"del(. as $x | reduce (1,2) as $i (.; foreach ([first(.) | . as {b:$b} | .]) as $y (.; $y; .)))",
            r"",
            r#"Cannot index array with string "b""#,
            5,
        ),
        (
            r"[[]]",
            r"(. as $x | reduce .[]? as $i (.; foreach ([first(.) | . as {b:$b} | .]) as $y (.; .; .))) |= 9",
            r"",
            r#"Invalid path expression near attempt to access element "b" of [[]]"#,
            5,
        ),
        (
            r"true",
            r"path(. as $x | reduce .[]? as $i (.; reduce ([first(.) | . as {b:$b} | .]) as $y (.; .)))",
            "[]\n",
            r"",
            0,
        ),
        (
            r#"{"a":false,"b":null}"#,
            r"(.a as $x | reduce .[]? as $i (.; foreach ([first(.) | . as {b:$b} | .]) as $y (.; .; .))) |= 9",
            r"",
            r#"Invalid path expression near attempt to access element "b" of {"a":false,"b":null}"#,
            5,
        ),
        (
            r#"[{"a":1}]"#,
            r"path(. as $x | reduce .[]? as $i (.; reduce ([first(.) | . as {b:$b} | .]) as $y (.; .)))",
            r"",
            r#"Invalid path expression near attempt to access element "b" of [{"a":1}]"#,
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(. as $x | reduce .. as $i (.; foreach ([first(.) | . as {b:$b} | .]) as $y (.; .; .)))",
            r"",
            r#"Invalid path expression near attempt to access element "b" of {"a":{"b":1}}"#,
            5,
        ),
        (
            r#"{"a":[true]}"#,
            r"del(. as $x | reduce .. as $i (.; reduce ([first(.) | . as {b:$b} | .]) as $y (.; .)))",
            r"",
            r#"Invalid path expression near attempt to access element "b" of {"a":[true]}"#,
            5,
        ),
        (
            r#"{"a":[true]}"#,
            r"path(.a as $x | reduce .. as $i (.; reduce ([first(.) | . as {b:$b} | .]) as $y (.; .)))",
            r"",
            r#"Invalid path expression near attempt to access element "b" of {"a":[true]}"#,
            5,
        ),
    ])
}

/// #4941: `wrapped-limit-dot` -- a sample of the grid rows over the `rd-nav`, `rd-nava`, `rd-rec` and `rd-lit` outers (the
/// destructure behind this stage raises in jq, and the binary used to answer it); every row captured from
/// `/usr/bin/jq` 1.7.1, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_4941_limit_dot_ahead_of_a_bracketed_destructure_raises() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":[true]}"#,
            r"del(. as $x | reduce .a? as $i (.; foreach ([limit(2; .) | . as {b:$b} | .]) as $y (.; $y; .)))",
            r"",
            r#"Invalid path expression near attempt to access element "b" of {"a":[true]}"#,
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(.a as $x | reduce .[]? as $i (.; reduce ([limit(2; .) | . as {b:$b} | .]) as $y (.; .)))",
            r"",
            r#"Invalid path expression near attempt to access element "b" of {"a":{"b":1}}"#,
            5,
        ),
        (
            r#"{"a":false,"b":null}"#,
            r"del(. as $x | reduce .. as $i (.; reduce ([limit(2; .) | . as {b:$b} | .]) as $y (.; .)))",
            r"",
            r#"Invalid path expression near attempt to access element "b" of {"a":false,"b":null}"#,
            5,
        ),
        (
            r"[[1],[1]]",
            r"path(.a as $x | reduce .. as $i (.; foreach ([limit(2; .) | . as {b:$b} | .]) as $y (.; $y; .)))",
            r"",
            r#"Cannot index array with string "a""#,
            5,
        ),
        (
            r"[true]",
            r"(. as $x | reduce .a? as $i (.; foreach ([limit(2; .) | . as {b:$b} | .]) as $y (.; $y; .))) = 9",
            "9\n",
            r"",
            0,
        ),
        (
            r#"{"a":[true]}"#,
            r"(.a as $x | reduce .a? as $i (.; reduce ([limit(2; .) | . as {b:$b} | .]) as $y (.; .))) |= 9",
            r"",
            r#"Invalid path expression near attempt to access element "b" of {"a":[true]}"#,
            5,
        ),
        (
            r"[null]",
            r"(. as $x | reduce .[]? as $i (.; foreach ([limit(2; .) | . as {b:$b} | .]) as $y (.; .; .))) = 9",
            r"",
            r#"Invalid path expression near attempt to access element "b" of [null]"#,
            5,
        ),
        (
            r"[true]",
            r"del(. as $x | reduce (1,2) as $i (.; foreach ([limit(2; .) | . as {b:$b} | .]) as $y (.; $y; .)))",
            r"",
            r#"Cannot index array with string "b""#,
            5,
        ),
        (
            r"[]",
            r"del(.a as $x | reduce .a? as $i (.; foreach ([limit(2; .) | . as {b:$b} | .]) as $y (.; $y; .)))",
            r"",
            r#"Cannot index array with string "a""#,
            5,
        ),
        (
            r"[[1],[1]]",
            r"(.a as $x | reduce (1,2) as $i (.; reduce ([limit(2; .) | . as {b:$b} | .]) as $y (.; .))) |= 9",
            r"",
            r#"Cannot index array with string "a""#,
            5,
        ),
    ])
}

/// #4941: `wrapped-opt-dot` -- a sample of the grid rows over the `rd-nav`, `rd-nava`, `rd-rec` and `rd-lit` outers (the
/// destructure behind this stage raises in jq, and the binary used to answer it); every row captured from
/// `/usr/bin/jq` 1.7.1, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_4941_optional_dot_ahead_of_a_bracketed_destructure_raises() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r"[[1],[1]]",
            r"del(. as $x | reduce .a? as $i (.; foreach ([(.)? | . as {b:$b} | .]) as $y (.; .; .)))",
            "null\n",
            r"",
            0,
        ),
        (
            r"true",
            r"(. as $x | reduce .a? as $i (.; reduce ([(.)? | . as {b:$b} | .]) as $y (.; .))) = 9",
            "9\n",
            r"",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | reduce .a? as $i (.; foreach ([(.)? | . as {b:$b} | .]) as $y (.; .; .)))",
            r"",
            r#"Invalid path expression near attempt to access element "b" of {"a":{"b":1}}"#,
            5,
        ),
        (
            r#"{"a":false,"b":null}"#,
            r"(.a as $x | reduce .. as $i (.; foreach ([(.)? | . as {b:$b} | .]) as $y (.; .; .))) |= 9",
            r"",
            r#"Invalid path expression near attempt to access element "b" of {"a":false,"b":null}"#,
            5,
        ),
        (
            r"true",
            r"path(. as $x | reduce .[]? as $i (.; foreach ([(.)? | . as {b:$b} | .]) as $y (.; .; .)))",
            "[]\n",
            r"",
            0,
        ),
        (
            r"[true]",
            r"path(. as $x | reduce .a? as $i (.; reduce ([(.)? | . as {b:$b} | .]) as $y (.; .)))",
            "[]\n",
            r"",
            0,
        ),
        (
            r#"{"a":[true]}"#,
            r"del(.a as $x | reduce (1,2) as $i (.; reduce ([(.)? | . as {b:$b} | .]) as $y (.; .)))",
            "null\n",
            r"",
            0,
        ),
        (
            r"[[1],[1]]",
            r"path(.a as $x | reduce .[]? as $i (.; foreach ([(.)? | . as {b:$b} | .]) as $y (.; $y; .)))",
            r"",
            r#"Cannot index array with string "a""#,
            5,
        ),
        (
            r#"{"a":false,"b":null}"#,
            r"(. as $x | reduce (1,2) as $i (.; foreach ([(.)? | . as {b:$b} | .]) as $y (.; .; .))) |= 9",
            "9\n",
            r"",
            0,
        ),
        (
            r"true",
            r"path(. as $x | reduce .[]? as $i (.; reduce ([(.)? | . as {b:$b} | .]) as $y (.; .)))",
            "[]\n",
            r"",
            0,
        ),
    ])
}

/// #4941: `wrapped-sel-true-alt` -- a sample of the grid rows over the `rd-nav`, `rd-nava`, `rd-rec` and `rd-lit` outers (the
/// destructure behind this stage raises in jq, and the binary used to answer it); every row captured from
/// `/usr/bin/jq` 1.7.1, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_4941_select_true_behind_alt_ahead_of_a_bracketed_destructure_raises() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r"[null]",
            r"(.a as $x | reduce .a? as $i (.; reduce ([false // (select(true) | . as {b:$b} | .)]) as $y (.; .))) |= 9",
            r"",
            r#"Cannot index array with string "a""#,
            5,
        ),
        (
            r"[]",
            r"(. as $x | reduce (1,2) as $i (.; reduce ([false // (select(true) | . as {b:$b} | .)]) as $y (.; .))) |= 9",
            r"",
            r#"Cannot index array with string "b""#,
            5,
        ),
        (
            r"true",
            r"del(. as $x | reduce .[]? as $i (.; foreach ([false // (select(true) | . as {b:$b} | .)]) as $y (.; .; .)))",
            "null\n",
            r"",
            0,
        ),
        (
            r"[true]",
            r"del(. as $x | reduce (1,2) as $i (.; reduce ([false // (select(true) | . as {b:$b} | .)]) as $y (.; .)))",
            r"",
            r#"Cannot index array with string "b""#,
            5,
        ),
        (
            r"[null]",
            r"path(.a as $x | reduce .[]? as $i (.; reduce ([false // (select(true) | . as {b:$b} | .)]) as $y (.; .)))",
            r"",
            r#"Cannot index array with string "a""#,
            5,
        ),
        (
            r"[[1],[1]]",
            r"del(.a as $x | reduce .. as $i (.; foreach ([false // (select(true) | . as {b:$b} | .)]) as $y (.; $y; .)))",
            r"",
            r#"Cannot index array with string "a""#,
            5,
        ),
        (
            r"[[1],[1]]",
            r"path(.a as $x | reduce .[]? as $i (.; foreach ([false // (select(true) | . as {b:$b} | .)]) as $y (.; $y; .)))",
            r"",
            r#"Cannot index array with string "a""#,
            5,
        ),
        (
            r"[]",
            r"(.a as $x | reduce (1,2) as $i (.; foreach ([false // (select(true) | . as {b:$b} | .)]) as $y (.; $y; .))) |= 9",
            r"",
            r#"Cannot index array with string "a""#,
            5,
        ),
        (
            r"null",
            r"(. as $x | reduce .a? as $i (.; reduce ([false // (select(true) | . as {b:$b} | .)]) as $y (.; .))) = 9",
            "9\n",
            r"",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"(. as $x | reduce .. as $i (.; reduce ([false // (select(true) | . as {b:$b} | .)]) as $y (.; .))) |= 9",
            r"",
            r#"Invalid path expression near attempt to access element "b" of {"a":{"b":1}}"#,
            5,
        ),
    ])
}

/// #4941: the controls -- a stage that can empty its input (`select(false)`, `limit(0; ..)`, `first(empty)`) never
/// reaches the destructure behind it, so jq answers and the binary must too. Every row captured from `/usr/bin/jq` 1.7.1.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_4941_input_emptying_stage_keeps_the_destructure_unreached() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":true}"#,
            r"path(. as $x | reduce .a? as $i (.; foreach ([select(false) | . as {b:$b} | .]) as $y (.; .; .)))",
            "[]\n",
            r"",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | reduce .a? as $i (.; foreach ([select(false) | . as {b:$b} | .]) as $y (.; .; .)))",
            "[]\n",
            r"",
            0,
        ),
        (
            r"[1]",
            r"path(. as $x | reduce .a? as $i (.; foreach ([select(false) | . as {b:$b} | .]) as $y (.; .; .)))",
            "[]\n",
            r"",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"path(. as $x | reduce .a? as $i (.; foreach ([limit(0; . as {b:$b} | .)]) as $y (.; .; .)))",
            "[]\n",
            r"",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | reduce .a? as $i (.; foreach ([limit(0; . as {b:$b} | .)]) as $y (.; .; .)))",
            "[]\n",
            r"",
            0,
        ),
        (
            r"[1]",
            r"path(. as $x | reduce .a? as $i (.; foreach ([limit(0; . as {b:$b} | .)]) as $y (.; .; .)))",
            "[]\n",
            r"",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"path(. as $x | reduce .[]? as $i (.; foreach ([select(true) | select(false) | . as {b:$b} | .]) as $y (.; .; .)))",
            "[]\n",
            r"",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | reduce .[]? as $i (.; foreach ([select(true) | select(false) | . as {b:$b} | .]) as $y (.; .; .)))",
            "[]\n",
            r"",
            0,
        ),
        (
            r"[1]",
            r"path(. as $x | reduce .[]? as $i (.; foreach ([select(true) | select(false) | . as {b:$b} | .]) as $y (.; .; .)))",
            "[]\n",
            r"",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"path(. as $x | reduce .a? as $i (.; foreach ([first(empty) | . as {b:$b} | .]) as $y (.; .; .)))",
            "[]\n",
            r"",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | reduce .a? as $i (.; foreach ([first(empty) | . as {b:$b} | .]) as $y (.; .; .)))",
            "[]\n",
            r"",
            0,
        ),
        (
            r"[1]",
            r"path(. as $x | reduce .a? as $i (.; foreach ([first(empty) | . as {b:$b} | .]) as $y (.; .; .)))",
            "[]\n",
            r"",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"del(. as $x | reduce .a? as $i (.; foreach ([select(false) | . as {b:$b} | .]) as $y (.; .; .)))",
            "null\n",
            r"",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(. as $x | reduce .a? as $i (.; foreach ([select(false) | . as {b:$b} | .]) as $y (.; .; .)))",
            "null\n",
            r"",
            0,
        ),
        (
            r"[1]",
            r"del(. as $x | reduce .a? as $i (.; foreach ([select(false) | . as {b:$b} | .]) as $y (.; .; .)))",
            "null\n",
            r"",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"(. as $x | reduce .a? as $i (.; foreach ([select(true) | select(false) | . as {b:$b} | .]) as $y (.; .; .))) |= 9",
            "9\n",
            r"",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"(. as $x | reduce .a? as $i (.; foreach ([select(true) | select(false) | . as {b:$b} | .]) as $y (.; .; .))) |= 9",
            "9\n",
            r"",
            0,
        ),
        (
            r"[1]",
            r"(. as $x | reduce .a? as $i (.; foreach ([select(true) | select(false) | . as {b:$b} | .]) as $y (.; .; .))) |= 9",
            "9\n",
            r"",
            0,
        ),
    ])
}

/// #4941: the same stage behind the other routes that read a source for its plain bind chain -- the body of a
/// `try ... catch`, the right of a `//` whose left is never truthy, the handler of a `try` that only raises, a
/// parenthesised pipe. Stdout and exit code are jq's; the wording is the step or the end of the path (jq's
/// `result 7` for a handler that returns a literal), so only the common prefix is pinned.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_4941_input_handing_stage_behind_run_branches() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":true}"#,
            r"path(. as $x | reduce .a? as $i (.; foreach (try (select(true) | . as {b:$b} | .) catch 7) as $y (.; .; .)))",
            "[]\n",
            r"",
            0,
        ),
        (
            r"[[1]]",
            r"path(. as $x | reduce .a? as $i (.; foreach (try (select(true) | . as {b:$b} | .) catch 7) as $y (.; .; .)))",
            "[]\n",
            r"",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"path(. as $x | reduce .a? as $i (.; foreach (false // (select(true) | . as {b:$b} | .)) as $y (.; .; .)))",
            r"",
            r"Invalid path expression",
            5,
        ),
        (
            r"[[1]]",
            r"path(. as $x | reduce .a? as $i (.; foreach (false // (select(true) | . as {b:$b} | .)) as $y (.; .; .)))",
            "[]\n",
            r"",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"path(. as $x | reduce .a? as $i (.; foreach ([try error catch (select(true) | . as {b:$b} | .)]) as $y (.; .; .)))",
            r"",
            r"Invalid path expression",
            5,
        ),
        (
            r"[[1]]",
            r"path(. as $x | reduce .a? as $i (.; foreach ([try error catch (select(true) | . as {b:$b} | .)]) as $y (.; .; .)))",
            "[]\n",
            r"",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"path(. as $x | reduce .a? as $i (.; foreach ([(select(true) | .) | . as {b:$b} | .]) as $y (.; .; .)))",
            r"",
            r"Invalid path expression",
            5,
        ),
        (
            r"[[1]]",
            r"path(. as $x | reduce .a? as $i (.; foreach ([(select(true) | .) | . as {b:$b} | .]) as $y (.; .; .)))",
            "[]\n",
            r"",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"del(. as $x | reduce .[]? as $i (.; foreach ([false // (first(.) | . as {b:$b} | .)]) as $y (.; .; .)))",
            r"",
            r"Invalid path expression",
            5,
        ),
        (
            r"[[1]]",
            r"del(. as $x | reduce .[]? as $i (.; foreach ([false // (first(.) | . as {b:$b} | .)]) as $y (.; .; .)))",
            r"",
            r"Invalid path expression",
            5,
        ),
    ])
}
