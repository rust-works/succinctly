//! #5193: a generator in a `foreach` UPDATE delivers every output, not the first a `path()` leaf asks for.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

// `path()` asks a generator leaf for its first output only ([`Keep::First`]): jq raises on it and never asks for another. A
// `foreach` consumes every output of its UPDATE, so `range(3)` there is three states. The UPDATE was resolved under the
// caller's bound, so the EXTRACT ran once. A bind of an outer `$x` ahead of the generator lost them the same way.

/// #5193: the issue's rows, the bare generator, a bounded `limit`, and the write forms. Every row captured from
/// `/usr/bin/jq` 1.7.1, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5193_foreach_update_generator_delivers_every_output() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r"null",
            r"path(foreach .a as $i (5; range(3); $i))",
            "[\"a\"]\n[\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach .a as $i (5; $x as {a:$a} | range(3); $i))",
            "[\"a\",\"a\"]\n[\"a\",\"a\"]\n[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; ($x as {a:$a} | range(2)); $i))",
            "[\"a\",\"a\"]\n[\"a\",\"a\"]\n[\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach .a as $i (5; $x as {a:$a} | (7,8) | range(2); $i))",
            "[\"a\",\"a\"]\n[\"a\",\"a\"]\n[\"a\",\"a\"]\n[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach .a as $i (null; $x as {a:$a} | range(3); $i))",
            "[\"a\",\"a\"]\n[\"a\",\"a\"]\n[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach .a as $i (5; $x as {a:$a} | limit(2; range(5)); $i))",
            "[\"a\",\"a\"]\n[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(foreach .a as $i (5; range(3); .))",
            "",
            "Invalid path expression with result",
            5,
        ),
        (
            r"null",
            r"[limit(2; path(foreach .a as $i (5; range(100000000000); $i)))]",
            "[[\"a\"],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r"null",
            r"del(. as $x | foreach .a as $i (5; $x as {a:$a} | range(2); $i))",
            "null\n",
            "",
            0,
        ),
        (
            r"null",
            r"(. as $x | foreach .a as $i (5; $x as {a:$a} | range(2); $i)) |= 1",
            "{\"a\":{\"a\":1}}\n",
            "",
            0,
        ),
        (
            r"null",
            r"first(path(foreach .a as $i (5; range(100000000000); $i)))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"isempty(path(foreach .a as $i (5; range(100000000000); $i)))",
            "false\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(foreach .a as $i (5; .; range(3)))",
            "",
            "Invalid path expression with result",
            5,
        ),
    ])
}
