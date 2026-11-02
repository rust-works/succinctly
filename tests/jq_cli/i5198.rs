//! #5198: a generator in a `reduce` UPDATE yields its last output to the end of the path, as jq does.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

// `reduce` runs its UPDATE to exhaustion and keeps the last output. A generator leaf (`range(3)`) was resolved under
// `path()`'s stop-after-first bound, so the end-of-path check saw the first output (`result 0`) where jq names the last
// (`result 2`), as `foreach`'s UPDATE did before #5193.

/// #5198: a bare generator, a source that yields twice, a bind ahead of it, the write form, and generators whose last
/// output is the register (`null`). Every row captured from `/usr/bin/jq` 1.7.1, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5198_reduce_update_generator_yields_its_last_output() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r"null",
            r"path(reduce .a as $i (5; range(3)))",
            "",
            "Invalid path expression with result 2",
            5,
        ),
        (
            r"null",
            r"(reduce .a as $i (.; range(3))) |= 1",
            "",
            "Invalid path expression with result 2",
            5,
        ),
        (
            r"null",
            r"path(reduce (.a,.a) as $i (5; range(3)))",
            "",
            "Invalid path expression with result 2",
            5,
        ),
        (
            r"null",
            r"path(. as $x | reduce .a as $i (5; $x as {a:$a} | range(3)))",
            "",
            "Invalid path expression with result 2",
            5,
        ),
        (
            r"null",
            r"path(reduce .a as $i (null; range(3)|null))",
            "[]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(reduce .a as $i (null; range(2) | if .==0 then 7 else null end))",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"path(reduce (1,2) as $i (0; range(3)))",
            "",
            "Invalid path expression with result 2",
            5,
        ),
        (
            r#"{"a":1}"#,
            r"path(reduce (1,2) as $i (.; (., 7)))",
            "",
            "Invalid path expression with result 7",
            5,
        ),
        (
            r#"{"a":1}"#,
            r#"try path(reduce .a as $i (null; (range(3), error("x")))) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"first(path(reduce .a as $i (5; range(3))))",
            "",
            "Invalid path expression with result 2",
            5,
        ),
        (
            r#"{"a":1}"#,
            r"isempty(path(reduce .a as $i (5; range(3))))",
            "",
            "Invalid path expression with result 2",
            5,
        ),
        (
            r#"{"a":1}"#,
            r"[path(reduce (1,2) as $i (.; (.a?, .)))]",
            "[[]]\n",
            "",
            0,
        ),
    ])
}
