//! #5102: a navigation stage ahead of a caught foreign-`$x` `foreach` in a `foreach` UPDATE. The converse, where `$x` is
//! the register the stage navigated to, is #5107's.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

// `FE = foreach (try ($x as {a:$a} | .)) as $y (.; .; .)`, `Q = . as $x | foreach . as $i (.; U; null)`. A stage ahead
// of FE that navigates (`.b | FE`) moves jq's register off the root `$x` is bound at, so FE's destructure of `$x` raises
// its path error, the `try` swallows it, and FE's source is empty: the UPDATE emits nothing, the fold has no output, and
// `path(Q)` is empty, `Q |= 9` leaves the document. The binary read FE as the sole UPDATE only, so with a stage
// ahead of it the source was driven by value: `["b"]` came back as a path (a silent write of `9` at `.b` under `|=`).

/// #5102: the issue's rows on `{"a":null}` (a head stage that lands on `null`): `.b`, `.a`, `.c.d`, `.[0]?`, FE
/// piped into `.`, FE behind a `try` and an `if`, in `|=`, `[path(..)?]`, `path` and `del`; the `[$a]` pattern and the
/// `?` spelling; and the heads that land on a container, which used to be refused loudly. Every row captured from
/// `/usr/bin/jq` 1.7.1 (stdout, exit code), on both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5102_navigation_stage_ahead_of_a_caught_foreign_foreach_in_a_foreach_update() -> Result<()>
{
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach . as $i (.; .b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach . as $i (.; .b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach . as $i (.; .b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach . as $i (.; .a | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach . as $i (.; .a | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach . as $i (.; .a | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach . as $i (.; .c.d | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach . as $i (.; .c.d | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach . as $i (.; .c.d | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach . as $i (.; .[0]? | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach . as $i (.; .[0]? | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach . as $i (.; .[0]? | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach . as $i (.; .b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .; null)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach . as $i (.; .b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .; null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach . as $i (.; .b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .; null))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach . as $i (.; try (.b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) catch 1; null)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach . as $i (.; try (.b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) catch 1; null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach . as $i (.; try (.b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) catch 1; null))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach . as $i (.; if . then (.b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else 3 end; null)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach . as $i (.; if . then (.b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else 3 end; null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach . as $i (.; if . then (.b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else 3 end; null))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach . as $i (.; .b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":[1]}"#,
            r"(. as $x | foreach . as $i (.; .b | foreach (($x as [$a] | .)?) as $y (.; .; .); null)) |= 9",
            "{\"a\":[1]}\n",
            "",
            0,
        ),
        (
            r#"{"a":[1]}"#,
            r"[path(. as $x | foreach . as $i (.; .b | foreach (($x as [$a] | .)?) as $y (.; .; .); null))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":{"c":1}}"#,
            r"[path(. as $x | foreach . as $i (.; .b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":{"c":1}}"#,
            r"(. as $x | foreach . as $i (.; .b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null)) |= 9",
            "{\"a\":null,\"b\":{\"c\":1}}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"z":1},"b":[1]}"#,
            r"[path(. as $x | foreach . as $i (.; .b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"z":1},"b":[1]}"#,
            r"(. as $x | foreach . as $i (.; .b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null)) |= 9",
            "{\"a\":{\"z\":1},\"b\":[1]}\n",
            "",
            0,
        ),
    ])
}

/// #5102: the same rows where the outer `foreach` has a two-element source, a `reduce` UPDATE, a wrapper around the
/// `foreach` (`first`, `limit`, `//`, `label`, a comma, `select(true)`), the `foreach` in EXTRACT, two navigating
/// stages, and a `null` `$x` beside a `null` register (identical by kind, so the destructure passes and `FE` emits).
/// Every row captured from `/usr/bin/jq` 1.7.1, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5102_the_wrappers_sources_and_bodies_around_the_navigated_foreach() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null,"b":null}"#,
            r"[path(.a as $x | foreach . as $i (.; .b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))]",
            "[[\"b\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":null}"#,
            r"(.a as $x | foreach . as $i (.; .b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null)) |= 9",
            "{\"a\":null,\"b\":{\"a\":9}}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (1,2) as $i (.; .b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (1,2) as $i (.; .b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null)) |= 9",
            "",
            "Invalid path expression near attempt to access element \"b\" of null",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.,.) as $i (.; .b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.,.) as $i (.; .b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null)) |= 9",
            "",
            "Invalid path expression near attempt to access element \"b\" of null",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.b) as $i (.; .b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.b) as $i (.; .b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null)) |= 9",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | reduce (1,2) as $i (.; .b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .)))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"[path(.a as $x | reduce (1,2) as $i (.; .b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .)))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach . as $i (.; first(.b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach . as $i (.; first(.b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); null)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach . as $i (.; (.b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // 3; null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach . as $i (.; (.b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // 3; null)) |= 9",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach . as $i (.; limit(1; .b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach . as $i (.; limit(1; .b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); null)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach . as $i (.; label $l | .b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach . as $i (.; label $l | .b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach . as $i (.; (.b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), 2; null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach . as $i (.; (.b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), 2; null)) |= 9",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach . as $i (.; select(true) | .b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach . as $i (.; select(true) | .b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach . as $i (.; .; .b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .)))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach . as $i (.; .; .b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .))) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach . as $i (.; .b | .c | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach . as $i (.; .b | .c | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
    ])
}

/// #5102: rows that stay wrong, pinned to jq's answers and ignored until they are fixed. The first is an `INIT null`
/// EXTRACT whose `if` has an `elif` and a pipe after the nested `foreach` (#5106's other arms); the last is the
/// converse, `$x` the node the stage navigates to (#5107). Every row captured from `/usr/bin/jq` 1.7.1, both routes.
#[test]
#[ignore = "#5106, #5107"]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5102_known_gaps_next_to_the_navigated_foreach() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r"{}",
            r"[path(. as $x | foreach (.a,.) as $i (null; .; if true then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .) elif .b then .[0]? else .[0]? end))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":{"a":1}}"#,
            r"[path(.b as $x | foreach . as $i (.; .b | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))?]",
            "[]\n",
            "",
            0,
        ),
    ])
}
