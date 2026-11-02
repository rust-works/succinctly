//! #5185: a bind of an outer `$x` whose body is a comma of `.` delivers every output in a `foreach` UPDATE whose state is not
//! the register.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

// The bind's body handed the accumulator on as one branch when it only emitted `.` (#4932). A comma of `.` emits it once per
// item, so the three-argument `foreach` EXTRACT ran once where jq runs it once per item.

/// #5185: the issue's four rows and the comma shapes beside them. Every row captured from `/usr/bin/jq` 1.7.1, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5185_comma_of_dots_in_a_bind_body_delivers_each_output() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            "null",
            r"path(. as $x | foreach .a as $i (5; $x as {a:$a} | (.,.); $i))",
            "[\"a\",\"a\"]\n[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            "null",
            r"path(. as $x | foreach .a as $i (5; $x as {a:$a} | (.,.,.); $i))",
            "[\"a\",\"a\"]\n[\"a\",\"a\"]\n[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            "null",
            r"path(. as $x | foreach (.a,.a) as $i (5; $x as {a:$a} | (.,.); $i))",
            "[\"a\",\"a\"]\n[\"a\",\"a\"]\n[\"a\",\"a\"]\n[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            "null",
            r"path(. as $x | foreach .a as $i ([1]; $x as {a:$a} | (.,.); $i))",
            "[\"a\",\"a\"]\n[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            "null",
            r"path(. as $x | foreach .a as $i (5; $x as {a:$a} | try (.,.); $i))",
            "[\"a\",\"a\"]\n[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            "null",
            r"path(. as $x | foreach .a as $i (5; $x as {a:$a} | first(.,.); $i))",
            "[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            "null",
            r"path(. as $x | foreach .a as $i (5; $x as {a:$a} | (., 7, .); $i))",
            "[\"a\",\"a\"]\n[\"a\",\"a\"]\n[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            "null",
            r"path(. as $x | foreach .a as $i (5; $x as {a:$a} | limit(2; .,.); $i))",
            "[\"a\",\"a\"]\n[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            "null",
            r"path(. as $x | foreach .a as $i (5; $x as {a:$a} | limit(1; .,.); $i))",
            "[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            "null",
            r"path(. as $x | foreach .a as $i (5; $x as {a:$a} | limit(5; .,.); $i))",
            "[\"a\",\"a\"]\n[\"a\",\"a\"]\n",
            "",
            0,
        ),
    ])
}
