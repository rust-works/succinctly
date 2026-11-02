//! #4957: a caught foreign-`$x` `foreach` as, or inside, the UPDATE of a `foreach`: an `if` on a literal truthy condition
//! around it over a moving source, and a bare one over a source that leaves the register on the root.

use anyhow::Result;

use crate::{assert_path_rows_both_routes_3749, run_jq_full};

// `if true then FE else . end` runs `FE` and nothing else, so jq answers it as it does the bare `FE`. The binary read
// the `if` as one that may leave the state `null` (the `truthy` check of `if . then FE else . end`), so the
// foreach's EXTRACT `.a` raised jq's index error as a guess that no `?` or `try` catches, and `[path(..)?]` exited 5
// where jq prints `[["a","a"]]`.

/// #4957: `true`, `1` and `"s"` as the condition, with and without an `else .`, and nested in a taken `then`; a falsy
/// literal and a literal `else` keep the reading they had. Every row captured from `/usr/bin/jq` 1.7.1 (stdout, first stderr line up to the
/// container, exit code), both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_4957_literal_truthy_if_around_a_caught_foreign_foreach() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if true then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .a))",
            "[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if true then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; if true then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .a)) catch "C""#,
            "[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; if true then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .a))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; if true then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .a)) |= 5",
            "{\"a\":{\"a\":5}}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if true then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; if true then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; $i)) |= 5",
            "",
            r#"Invalid path expression with result {"a":null}"#,
            5,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if true then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":1}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if true then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if 1 then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[del(. as $x | foreach (.a,.) as $i (.; if 1 then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .a))?]",
            "[{\"a\":null}]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"[path(. as $x | foreach (.a,.) as $i (.; if "s" then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .a))?]"#,
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if true then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end; .a))",
            "[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[del(. as $x | foreach (.a,.) as $i (.; if true then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end; .a))?]",
            "[{\"a\":null}]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if true then (foreach (try ($x as {a:$a} | .)) as $y (.; .; $y)) else . end; .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if true then (foreach (try ($x as {a:$a} | .)) as $y (.; .; $y)) else . end; $i))",
            "",
            r#"Invalid path expression with result {"a":null}"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if false then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; if false then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .a))",
            "",
            r#"Invalid path expression near attempt to access element "a""#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if true then (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end) else . end; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if true then (if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end) else . end; $i))?]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; if true then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) elif . then 1 else . end; $i))?]",
            "[]\n",
            "",
            0,
        ),
    ])
}

// Rows 2 and 3 of the issue: a bare caught-bind `foreach` (`FE`) as the UPDATE of a `foreach` whose source leaves jq's
// register on the root at the step (`1,2`, `.`, `..`, `.,.a`), or whose caught bind is of `.`. The accumulator is the
// register there, so `$x` (bound at the root) is it: the pattern's step passes and moves the register, `FE` emits the
// state, and the EXTRACT's `.a` raises jq's own index error -- which the enclosing `?` catches. The binary refused
// instead, with a guess no `?` or `try` catches (exit 5).

/// #4957: the root-leaving sources against the `if . then FE else . end`, bare, comma, pipe and `try` spellings of the
/// UPDATE, in `path`, `del`, `|=` and `=` position, with the `$i` and `.` EXTRACTs. Every row captured from
/// `/usr/bin/jq` 1.7.1 (stdout, first stderr line up to the container, exit code), both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_4957_bare_caught_bind_foreach_update_on_a_root_register() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.,.a) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (1,2) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (..) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"[path(. as $x | foreach (1,2) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (1,2) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else . end; .a))",
            "",
            r#"Invalid path expression near attempt to access element "a""#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.,.a) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a))",
            "",
            r#"Invalid path expression near attempt to access element "a""#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.,.a) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.,.a) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.,.a) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .))",
            "",
            r#"Invalid path expression with result {"a":null}"#,
            5,
        ),
        (
            r#"{"a":{"b":2}}"#,
            r"[del(. as $x | foreach (1,2) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":2}}"#,
            r"[(. as $x | foreach (1,2) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a)) |= 5]",
            "",
            r#"Invalid path expression near attempt to access element "a""#,
            5,
        ),
        (
            r"[1]",
            r"[path(. as $x | foreach (1,2) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r"[1]",
            r"path(. as $x | foreach (1,2) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (1,2) as $i (.; foreach (try (. as {a:$a} | .)) as $y (.; .; .); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"[path(. as $x | foreach (.,.a) as $i (.; foreach (try (. as {a:$a} | .)) as $y (.; .; .); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"[del(. as $x | foreach (.a,.) as $i (.; foreach (try (. as {a:$a} | .) catch .) as $y (.; .; .); $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; foreach (try (. as {a:$a} | .) catch .) as $y (.; .; .); $i))",
            "",
            r#"Invalid path expression with result {"a":1}"#,
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"[del(. as $x | foreach (.a,.) as $i (.; foreach (try (. as {a:$a} | .) catch .) as $y (.; .; .); $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.,.a) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .; .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (1,2) as $i (.; foreach (try ($x as [$a] | .)) as $y (.; .; .); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (..) as $i (.; foreach ($x as {a:$a} | .) as $y (.; .; .); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (1,2) as $i (.; foreach ($x as {a:$a} | .) as $y (.; .; .); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (1,2) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (1,2) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a)) |= 5",
            "",
            r#"Invalid path expression near attempt to access element "a""#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (1,2) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a)) = 5",
            "",
            r#"Invalid path expression near attempt to access element "a""#,
            5,
        ),
    ])
}

/// #4957: the spellings of the `foreach` that is read (a `$name` or `.` bound, an object or array pattern, a
/// `catch .` handler, a `?` around the bind) and of the UPDATE around it (`try`, a comma, a pipe, an `if`), each on a
/// row the rest of the file's rows do not reach. Every row captured from `/usr/bin/jq` 1.7.1, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_4957_bare_caught_bind_foreach_update_spellings() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r"[1]",
            r"[del(. as $x | foreach (.) as $i (.; (foreach (($x as [$a] | .)) as $y (.; .; .)) | 5; .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r"[1]",
            r"[del(. as $x | foreach (..) as $i (.; (foreach (($x as [$a] | .)) as $y (.; .; .)) | 5; $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r"[1]",
            r"del(. as $x | foreach (.) as $i (.; try (foreach ($x as {a:$a} | .) as $y (.; .; .)); .))",
            "[1]\n",
            "",
            0,
        ),
        (
            r"1",
            r"del(. as $x | foreach (.) as $i (.; try (foreach ($x as {a:$a} | .) as $y (.; .; .)); .))",
            "1\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"[path(. as $x | foreach (.) as $i (.; foreach (try (. as {a:$a} | .) catch .) as $y (.; .; .); .a))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":2}}"#,
            r"[path(. as $x | foreach (..) as $i (.; foreach (try ($x as {a:$a} | .) catch .) as $y (.; .; .); .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":false,"b":null}"#,
            r"[path(. as $x | foreach ($x) as $i (.; foreach (($x as {b:$a} | .)?) as $y (.; .; .); .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[del(. as $x | foreach (.) as $i (.; foreach (($x as {a:$a} | .)?) as $y (.; .; .); .[0]))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":2}}"#,
            r"[path(. as $x | foreach ((.,.)) as $i (.; foreach ((. as [$a] | .)) as $y (.; .; .); .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[del(. as $x | foreach (.) as $i (.; foreach (try (. as {a:$a} | .)) as $y (.; .; .); .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[del(. as $x | foreach (.) as $i (.; foreach ($x as {a:$a} | .) as $y (.; .; .); .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"[del(. as $x | foreach (.) as $i (.; foreach ($x as {a:$a} | .) as $y (.; .; .); .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r"[1]",
            r"[del(. as $x | foreach (.) as $i (.; foreach (try ($x as [$a] | .)) as $y (.; .; .); .))?]",
            "[]\n",
            "",
            0,
        ),
    ])
}

/// #4957: shapes the read does not take refuse as they did, loudly, or answer as jq does: a pattern of two or more
/// variables (its absent key lands on a `null` the resolver places differently from jq's), a body other than `.`, an
/// INIT or UPDATE of the inner `foreach` that is not `.`, an `if` arm that navigates (`else .a`), a comma item beside
/// the `foreach` (`(FE), 7`), a `null` or boolean EXTRACT that navigates or follows a `catch .`, a `?//` chain, a nested pattern, a `select` stage, a source
/// that computes. Reading them as the bare one-variable bind flipped rows like these to a caught raise that printed
/// `[]`/`"C"` where jq prints a path, or to an answer where jq refuses (random grids of 30,000 to 50,000 rows found
/// 2 to 11 of each). Each row is `(input, filter, jq 1.7.1's stdout, jq's exit code)`.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_4957_shapes_outside_the_read_never_answer_where_jq_differs() -> Result<()> {
    let rows: &[(&str, &str, &str, i32)] = &[
        (
            r"[1]",
            r"[del(. as $x | foreach ((1,.)) as $i (.; (foreach (($x as [$a,$b,$c] | .)?) as $y (.; .; .)) | .; .[0]))?]",
            "[[1]]\n",
            0,
        ),
        (
            r"[1]",
            r#"try path(. as $x | foreach ((.,.)) as $i (.; (foreach (try ($i as [$a,$b,$c] | .)) as $y (.; .; .)) | .; .)) catch "C""#,
            "",
            0,
        ),
        (
            r"[[1],[2]]",
            r"[del(. as $x | foreach (1,2) as $i (.; try (foreach (try ($x as [$a,$b] | .)) as $y (.; .; .)); $i))?]",
            "[[[1],[2]]]\n",
            0,
        ),
        (
            r#"{"a":false,"b":null}"#,
            r#"try path(. as $x | foreach ($x,.a) as $i (.; (foreach (try ($x as {a:$a,b:$b} | .)) as $y (.; .; .)), .; $i)) catch "C""#,
            "[]\n[\"a\"]\n",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"[del(. as $x | foreach (first(.,.a)) as $i (.; (foreach (try ($x as {a:$a,b:$b} | .)) as $y (.; .; .)), 7; $i))?]",
            "[null]\n",
            0,
        ),
        (
            r#"{"a":{"b":2}}"#,
            r#"try path(. as $x | foreach (1,2) as $i (.; foreach (try ($x as {a:$a,b:$b} | .)) as $y (.; .; .); .a)) catch "C""#,
            "",
            0,
        ),
        (
            r"[1]",
            r"[path(. as $x | foreach (.) as $i (.; foreach (first(try ($x as {a:$a} ?// [$a] | $i))) as $y (0; ($i), $i; .); $i))]",
            "",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach ((.,.)) as $i (.; foreach (select(.) | try (. as {a:{b:$a}} | .)) as $y (0; try (5); $i); $i))",
            "",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (null) as $i (.; foreach (($x as {a:$a} ?// [$a] | .)) as $y ($x; (.a?), .; .a?); .))",
            "",
            5,
        ),
        (
            r#"{"a":{"b":2}}"#,
            r"[path(. as $x | foreach (..) as $i (.; (try (.a? // .)), foreach (select($i) | ($x as {a:{b:$a}} | $x)) as $y (.; empty; $i); .))]",
            "",
            5,
        ),
        (
            r"[1]",
            r"path(. as $x | foreach (..) as $i (.; if true then (foreach (try (($i as {a:$a} ?// [$a] | .))) as $y ($x; .a?; .)) else empty end; .a))",
            "",
            5,
        ),
        (
            r#"{"a":[1,2]}"#,
            r"[path(. as $x | foreach (..) as $i (.; foreach (select($x) | try ($i as {a:$a} ?// [$a] | $x)) as $y (.; try ($i); .a?); $i))]",
            "",
            5,
        ),
        (
            r#"{"a":true}"#,
            r"[del(. as $x | foreach (1,2) as $i (.; try (foreach (($x as {a:$a} | $a)) as $y (null; (., 1); $y)); .))?]",
            "[{}]\n",
            0,
        ),
        (
            r#"{"a":[1,2]}"#,
            r"[path(. as $x | foreach (1,2) as $i (.; try (foreach (try ($x as {b:$a} | $a)) as $y (.; $y; .)); .))?]",
            "[[\"b\"],[\"b\"]]\n",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else .a end; empty)) catch "C""#,
            "\"C\"\n",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (..) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else .a end; empty)) catch "C""#,
            "",
            0,
        ),
        (
            r#"{"a":{"b":2}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else .a? end; empty)) catch "C""#,
            "\"C\"\n",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), .a; empty)) catch "C""#,
            "\"C\"\n",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; .a?, (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); empty)) catch "C""#,
            "\"C\"\n",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; if . then .a else (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end; empty)) catch "C""#,
            "\"C\"\n",
            0,
        ),
        (
            r#"{"a":{"b":2}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else 5 end; empty)) catch "C""#,
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r#"try del(. as $x | foreach ((.,.)) as $i (.; foreach (try ($x as {b:$a} | .)) as $y (0; $y; .); null)) catch "C""#,
            "{\"a\":1}\n",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[del(. as $x | foreach ((.,1)) as $i (.; foreach (try ($x as {b:$a} | .)) as $y (0; $y; .); null))?]",
            "[{\"a\":{\"a\":null}}]\n",
            0,
        ),
        (
            r#"{"b":null}"#,
            r#"try path(. as $x | foreach ((.,.)) as $i (.; (foreach (try ($x as {b:$a} | .)) as $y (0; $y; .)) | .; null)) catch "C""#,
            "[\"b\"]\n[\"b\"]\n",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (1,2) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), 7; .a))?]",
            "[]\n",
            0,
        ),
        (
            r"[1]",
            r"[del(. as $x | foreach (.) as $i (.; (foreach (($x as {b:$a} | .)) as $y (.; .; .)), .; .))?]",
            "[]\n",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"[del(. as $x | foreach (1,2) as $i (.; (foreach ((. as {a:$a} | .)) as $y (.; .; .)), .; .))?]",
            "[]\n",
            0,
        ),
        (
            r"[1]",
            r#"try del(. as $x | foreach (first(.,.a)) as $i (.; (try (foreach ($x as {a:$a} | .) as $y (.; .; .))), []; $x)) catch "C""#,
            "null\n",
            0,
        ),
        (
            r"[[1],[2]]",
            r#"try del(. as $x | foreach ($x) as $i (.; (try (foreach ($x as {a:$a} | .) as $y (.; .; .))), "s"; $i)) catch "C""#,
            "null\n",
            0,
        ),
        (
            r"[[1],[2]]",
            r"[path(. as $x | foreach ($x) as $i (.; (try (foreach ($x as {a:$a} | .) as $y (.; .; .))), null; $x))?]",
            "[[]]\n",
            0,
        ),
        (
            r#"{"b":null}"#,
            r#"try del(. as $x | foreach ((.,.a),.) as $i (.; (foreach (($x as {a:$a} | .)?) as $y (.; .; .)) | not; null)) catch "C""#,
            "{\"b\":null}\n",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r#"try path(. as $x | foreach ((.a,.),.) as $i (.; try (foreach (try ($x as [$a] | .) catch .) as $y (.; .; .)); null)) catch "C""#,
            "\"C\"\n",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); (.a?, null)))?]",
            "[[\"a\",\"a\"],[\"a\"]]\n",
            0,
        ),
        (
            r#"{"a":1}"#,
            r#"try path(. as $x | foreach (.,.a) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); (.a? // null))) catch "C""#,
            "\"C\"\n",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.a; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))?]",
            "[]\n",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"[del(. as $x | foreach (1,2) as $i (null; (foreach (try ($x as {b:$a} | .)) as $y (.; .; .))?; try (.a | not) catch null))?]",
            "[]\n",
            0,
        ),
        (
            r"{}",
            r#"try del(. as $x | foreach (null) as $i (null; foreach (($x as {a:$a} | .)?) as $y (.; .; .); try (.a | not) catch null)) catch "C""#,
            "\"C\"\n",
            0,
        ),
        (
            r"{}",
            r"[path(. as $x | foreach (true) as $i (null; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); (.. | select(. == null))))] | length",
            "1\n",
            0,
        ),
    ];
    for (input, filter, jq_stdout, jq_code) in rows {
        let (stdout, _stderr, code) = run_jq_full(&["-c", filter], Some(input))?;
        let same_as_jq = stdout == *jq_stdout && code == *jq_code;
        // jq answers and the binary refuses (exit 5, nothing printed), or jq refuses and the binary refuses too.
        let refuses = code == 5 && (*jq_code != 0 || stdout.is_empty());
        assert!(
            same_as_jq || refuses,
            "{filter} on {input}: jq {jq_stdout:?} (exit {jq_code}), binary {stdout:?} (exit {code})"
        );
    }
    Ok(())
}

// A `null` or boolean EXTRACT after the bare caught-bind `foreach` UPDATE. Where the inner bind passes, jq's register
// moves onto the bound child (`$x`'s `.a`, or `[0]`), the `foreach` emits its state with the register there, and the
// EXTRACT's `null`/`true`/`false` is that register's node (`jv_identical` holds those equal by kind) or a raise. The
// resolver judged the emission against the accumulator's register instead, so the bare `foreach` kept its refusal for
// any EXTRACT that may emit one, and `try path(..) catch "C"` exited 5 where jq answers.

/// #4957: the `null`/`true`/`not`/comparison EXTRACT over a root register's child that is `null`, `false`, a number
/// and a container; the bare, `try`, `?`, `$x`/`.`/`$i` and array-pattern spellings; sources that leave the register
/// on the root or move it; `path`, `[path(..)?]`, `try .. catch`, `del`, `=` and `|=`. Every row captured from
/// `/usr/bin/jq` 1.7.1 (stdout, first stderr line up to the container, exit code), both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_4957_null_boolean_extract_after_a_bare_caught_bind_foreach_update() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); true))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); not))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); (.a == null)))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); true)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); true))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); not))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); (.a == null)))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))",
            "",
            r"Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":false}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); true)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); true))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); not))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); (.a == null)))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))",
            "",
            r"Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":1}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); true)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); true))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); not))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); (.a == null)))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))",
            "",
            r"Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); true)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))",
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null)) |= 5",
            "{\"a\":5}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null)) = 5",
            "{\"a\":5}\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r#"try del(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); not)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r#"try ((. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); false)) |= 5) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); null))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach ((.,.a)) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); not))",
            "",
            r"Invalid path expression with result true",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .))?; null))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach ((.,.a)) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .))?; not))",
            "",
            r"Invalid path expression with result true",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach ($x as {a:$a} | .) as $y (.; .; .); null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach ((.,.a)) as $i (.; foreach ($x as {a:$a} | .) as $y (.; .; .); not))",
            "",
            r"Invalid path expression with result true",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as [$a] | .)) as $y (.; .; .); null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach ((.,.a)) as $i (.; foreach (try ($x as [$a] | .)) as $y (.; .; .); not))",
            "",
            r"Invalid path expression with result true",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try (. as {a:$a} | .)) as $y (.; .; .); null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach ((.,.a)) as $i (.; foreach (try (. as {a:$a} | .)) as $y (.; .; .); not))",
            "",
            r"Invalid path expression with result true",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($i as {a:$a} | .)) as $y (.; .; .); null))?]",
            "[[\"a\",\"a\"],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach ((.,.a)) as $i (.; foreach (try ($i as {a:$a} | .)) as $y (.; .; .); not))",
            "",
            r"Invalid path expression with result true",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.,.a) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); (null, true)))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (1,2) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); (null, true)))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (..) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); (null, true)))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.[]?) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); (null, true)))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (null) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); (null, true)))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach ((.a,.),.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); (null, true)))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (first(.,.a)) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); (null, true)))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (0; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (null; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (true; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r"[null]",
            r"[path(. as $x | foreach ((.,.[0])) as $i (.; foreach (try ($x as [$a] | .)) as $y (.; .; .); null))?]",
            "[[0]]\n",
            "",
            0,
        ),
        (
            r"[false]",
            r"path(. as $x | foreach ((.,.[0])) as $i (.; foreach (try ($x as [$a] | .)) as $y (.; .; .); false))",
            "[0]\n",
            "",
            0,
        ),
        (
            r"[1]",
            r#"try path(. as $x | foreach ((.,.[0])) as $i (.; foreach (try ($x as [$a] | .)) as $y (.; .; .); true)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":null}"#,
            r"(. as $x | foreach (null) as $i (null; foreach ($x as {a:$a} | .) as $y (.; .; .); null)) = 5",
            "{\"a\":5,\"b\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"[del(. as $x | foreach ($x) as $i (0; foreach (try ($i as {a:$a} | .)) as $y (.; .; .); not))?]",
            "[{}]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":null}"#,
            r"(. as $x | foreach (.,.a) as $i (0; ((foreach ($x as {a:$a} | .) as $y (.; .; .))); null, 1)) |= 5",
            "",
            r"Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":null,"b":null}"#,
            r"path(. as $x | foreach (1,2) as $i (true; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))",
            "[\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":[1,2]}"#,
            r"[path(. as $x | foreach ((.,.)) as $i (0; foreach (try ($x as {b:$a} | .)) as $y (.; .; .); null))]",
            "[[\"b\"],[\"b\"]]\n",
            "",
            0,
        ),
        (
            r"{}",
            r"path(. as $x | foreach (null) as $i (true; foreach (($x as {a:$a} | .)?) as $y (.; .; .); null, 1))",
            "[\"a\"]\n",
            r"Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":false}"#,
            r"[del(. as $x | foreach ([]) as $i (true; foreach (try ($x as {b:$a} | .)) as $y (.; .; .); null))?]",
            "[{\"a\":false}]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (1,2) as $i (0; foreach ($x as {a:$a} | .) as $y (.; .; .); (null, true)))]",
            "",
            r"Invalid path expression with result true",
            5,
        ),
        (
            r"[false]",
            r"(. as $x | foreach (true) as $i (0; try (foreach (try ($x as [$a] | .)) as $y (.; .; .)); not)) = 5",
            "[5]\n",
            "",
            0,
        ),
    ])
}

// A `null` or boolean EXTRACT after the bare caught-bind `foreach` behind a pipe whose later stages leave the register
// where it is (`FE | .`, `FE | length`). The pipe hands the `foreach`'s emission on unchanged, so the register the
// inner bind moved is still the one the EXTRACT's `null`/`true`/`false` is judged against (#4957).

/// #4957: `FE | .`, `try (FE | .)`, `(FE | .)?`, `FE | length` and `FE | has("a")`/`$x`/`tostring` under a `null`, `true` or `not` EXTRACT, over a root
/// register's child that is `null`, `false` and a container; `[path(..)?]` and `try path(..) catch`. Every row captured
/// from `/usr/bin/jq` 1.7.1 (stdout, exit code), both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_4957_null_boolean_extract_after_a_pipe_wrapped_bare_caught_bind_foreach_update(
) -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r#"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | has("a"); null))?]"#,
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r#"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | has("a"); null))?]"#,
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | has("a"); true))?]"#,
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r#"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | has("a"); true))?]"#,
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | $x; null))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | $x; null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | $x; true))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | $x; true))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | tostring; null))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | tostring; null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | tostring; true))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | tostring; true))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .; null))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .; null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .; null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .; true))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .; true))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .; true))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .; not))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .; not))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .; not))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .); null))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .); null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .); null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .); true))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .); true))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .); true))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .); not))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .); not))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .); not))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .)?; null))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .)?; null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .)?; null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .)?; true))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .)?; true))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .)?; true))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .)?; not))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .)?; not))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .)?; not))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | length; null))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | length; null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | length; null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | length; true))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | length; true))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | length; true))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | length; not))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | length; not))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | length; not))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .; null)) catch "C""#,
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .; null)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .; null)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .; true)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .; true)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .; true)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .; not)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .; not)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .; not)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .); null)) catch "C""#,
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .); null)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .); null)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .); true)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .); true)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .); true)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .); not)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .); not)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .); not)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .)?; null)) catch "C""#,
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .)?; null)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .)?; null)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .)?; true)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .)?; true)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .)?; true)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .)?; not)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .)?; not)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .)?; not)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | length; null)) catch "C""#,
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | length; null)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | length; null)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | length; true)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | length; true)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | length; true)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | length; not)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | length; not)) catch "C""#,
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | length; not)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
    ])
}

/// #4957: the pipe stages, `if`s and comma siblings the read does not cover keep a refusal or jq's answer, never a
/// different one. Every row captured from `/usr/bin/jq` 1.7.1.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_4957_wrapped_shapes_outside_the_pipe_read_never_answer_where_jq_differs() -> Result<()> {
    let rows: &[(&str, &str, &str, i32)] = &[
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .a; null)) catch "C""#,
            "[\"a\",\"a\"]\n",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .a; null)) catch "C""#,
            "\"C\"\n",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .a; true)) catch "C""#,
            "\"C\"\n",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .a; true)) catch "C""#,
            "\"C\"\n",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | null; true)) catch "C""#,
            "\"C\"\n",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | null; true)) catch "C""#,
            "\"C\"\n",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | true; true)) catch "C""#,
            "\"C\"\n",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | true; true)) catch "C""#,
            "\"C\"\n",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .), 7; true)) catch "C""#,
            "\"C\"\n",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .), 7; true)) catch "C""#,
            "\"C\"\n",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .), empty; null)) catch "C""#,
            "[\"a\"]\n",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .), empty; null)) catch "C""#,
            "\"C\"\n",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .), empty; true)) catch "C""#,
            "\"C\"\n",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .), empty; true)) catch "C""#,
            "\"C\"\n",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .) else . end; null)) catch "C""#,
            "\"C\"\n",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .) else . end; null)) catch "C""#,
            "\"C\"\n",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .) else . end; true)) catch "C""#,
            "\"C\"\n",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .) else . end; true)) catch "C""#,
            "\"C\"\n",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .) else . end; true)) catch "C""#,
            "\"C\"\n",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; if .a then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .) else . end; true)) catch "C""#,
            "\"C\"\n",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; . | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null)) catch "C""#,
            "[\"a\"]\n",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; . | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null)) catch "C""#,
            "\"C\"\n",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; . | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); true)) catch "C""#,
            "\"C\"\n",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; . | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); true)) catch "C""#,
            "\"C\"\n",
            0,
        ),
    ];
    for (input, filter, jq_stdout, jq_code) in rows {
        let (stdout, _stderr, code) = run_jq_full(&["-c", filter], Some(input))?;
        let same_as_jq = stdout == *jq_stdout && code == *jq_code;
        // jq answers and the binary refuses (exit 5, nothing printed), or jq refuses and the binary refuses too.
        let refuses = code == 5 && (*jq_code != 0 || stdout.is_empty());
        assert!(
            same_as_jq || refuses,
            "{filter} on {input}: jq {jq_stdout:?} (exit {jq_code}), binary {stdout:?} (exit {code})"
        );
    }
    Ok(())
}
