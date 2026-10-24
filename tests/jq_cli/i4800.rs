//! #4800: a `foreach` UPDATE that binds `.` with a single pattern, beside a `limit`/`first`/`nth(0; ..)` of `.` alone; and an inert UPDATE beside an EXTRACT that emits nothing.

use crate::assert_path_rows_both_routes_3749;
use anyhow::Result;

/// #4800: `limit(N; .)` for a constant nonzero count (positive, fractional, a quotient, or negative, which
/// takes every output in jq 1.7.1), `first(.)` and `nth(0; .)` emit only `.`, so beside the single-pattern
/// bind of `.` they hand `.` on as the bare `.` does ([`test_foreach_update_single_pattern_bind_beside_a_constant_division_count_4800`]
/// reads the same count with an `empty` body). Before, the sibling was not an identity item, so the UPDATE
/// was "not a bind-only shape" and refused naming its own state: `[path(..)?]` answered `[]` and an empty
/// EXTRACT raised where jq answers `[["a"]]` and nothing. The controls keep their refusal: a body that stores
/// a node of its own (`.a`, `7`), a state-dependent count (`limit(.; .)`) and a raising one (`1/0`), where jq
/// answers the outputs it reached before it raised and only the refusal's kind is pinned here. Every other
/// row captured from jq 1.7.1.
///
/// #5223: the path-mode interpreter carries jq's register, so the rows the shipped resolver
/// refused here now hold jq 1.7.1's answers.
#[cfg(not(feature = "unshared-containers"))]
#[test]
fn test_foreach_update_single_pattern_bind_beside_limit_of_dot_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; (. as {a:$q} | .), limit(1; .); null))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; (. as {a:$q} | .), limit(1; .); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; limit(1; .), (. as {a:$q} | .); null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; limit(1; .), (. as {a:$q} | .); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; (. as {a:$q} | .), limit(2; .); null))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; (. as {a:$q} | .), limit(2; .); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; limit(2; .), (. as {a:$q} | .); null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; limit(2; .), (. as {a:$q} | .); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; (. as {a:$q} | .), limit(5 / 2; .); null))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; (. as {a:$q} | .), limit(5 / 2; .); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; limit(5 / 2; .), (. as {a:$q} | .); null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; limit(5 / 2; .), (. as {a:$q} | .); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; (. as {a:$q} | .), limit(-1; .); null))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; (. as {a:$q} | .), limit(-1; .); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; limit(-1; .), (. as {a:$q} | .); null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; limit(-1; .), (. as {a:$q} | .); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; (. as {a:$q} | .), limit(0.5; .); null))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; (. as {a:$q} | .), limit(0.5; .); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; limit(0.5; .), (. as {a:$q} | .); null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; limit(0.5; .), (. as {a:$q} | .); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; (. as {a:$q} | .), first(.); null))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; (. as {a:$q} | .), first(.); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; first(.), (. as {a:$q} | .); null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; first(.), (. as {a:$q} | .); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; (. as {a:$q} | .), nth(0; .); null))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; (. as {a:$q} | .), nth(0; .); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; nth(0; .), (. as {a:$q} | .); null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; nth(0; .), (. as {a:$q} | .); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; (. as {a:$q} | .), limit(2; (., .)); null))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; (. as {a:$q} | .), limit(2; (., .)); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; limit(2; (., .)), (. as {a:$q} | .); null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; limit(2; (., .)), (. as {a:$q} | .); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; (. as {a:$q} | .), first(first(.)); null))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; (. as {a:$q} | .), first(first(.)); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; first(first(.)), (. as {a:$q} | .); null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; first(first(.)), (. as {a:$q} | .); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; (. as {a:$q} | .), limit(1; select(true)); null))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; (. as {a:$q} | .), limit(1; select(true)); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; limit(1; select(true)), (. as {a:$q} | .); null))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; limit(1; select(true)), (. as {a:$q} | .); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; (. as {a:$q} | .), nth(0+0; .); null))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; (. as {a:$q} | .), nth(0+0; .); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; (. as {a:$q} | .), limit(1; .); null))",
            "[\"a\"]\n",
            "Invalid path expression with result",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"del(foreach (1,2) as $i (.; (. as {a:$q} | .), limit(2; .); null))",
            "",
            "Invalid path expression with result",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"(foreach (1,2) as $i (.; (. as {a:$q} | .), first(.); null)) |= 9",
            "",
            "Invalid path expression with result",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(foreach (1,2) as $i (.; (. as {a:$q} | .), limit(1; .); $i))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; (. as {a:$q} | .), limit(0.5; .a); null))",
            "[\"a\"]\n[\"a\"]\n",
            r#"Invalid path expression near attempt to access element "a" of null"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; (. as {a:$q} | .), limit(1; 7); null))",
            "[\"a\"]\n",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; (. as {a:$q} | .), limit(.; .); null))",
            "[\"a\"]\n",
            r#"object ({"a":null}) and number (1) cannot be subtracted"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; (. as {a:$q} | .), limit(1/0; .); null))",
            "[\"a\"]\n",
            "number (1) and number (0) cannot be divided because the divisor is zero",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(foreach (1,2) as $i (.; (. as {a:$q} | .), limit(1; .a); null))",
            "[\"a\"]\n[\"a\"]\n",
            r#"Invalid path expression near attempt to access element "a" of null"#,
            5,
        ),
    ])
}

/// #4800: a `foreach` whose EXTRACT emits nothing (`empty`, `select(false)`, `first(empty)`, `limit(0; ..)`) has
/// no output to path-check, so an UPDATE that can neither raise nor have an effect (`expr_is_inert`: `.`,
/// literals, `not`, `tostring`, `select(true)`, and a pipe, comma, `//`, `if`, `try`/`?` or nested `foreach`
/// of such, `$var` and a bind of `.` or `$var`, a destructuring pattern only inside a `try`/`?`) is read as one that hands the state on. A nested
/// `foreach` in it whose source destructures a `$x` marker (`try ($x as {a:$a} | .)`) was refused for an
/// emission that is never made, where jq answers nothing. The controls keep their refusal: an EXTRACT that
/// does emit (`$i`, `.`) is the shape the earlier #4800 rows decide. (An uncaught destructure was a control too, until #4957
/// read it as jq does.) Every other row captured from jq 1.7.1.
#[test]
fn test_foreach_with_an_emitting_nothing_extract_ignores_an_inert_update_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":1}"#,
            r"path(. as $x | foreach . as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"path(. as $x | foreach . as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | tostring; empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach . as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // 5; empty))",
            "",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach . as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)), 7; empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"path(. as $x | foreach . as $i (.; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else . end; empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"path(. as $x | foreach . as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"path(. as $x | foreach . as $i (.; 1 | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"path(. as $x | foreach . as $i (.; . | foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .; empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach . as $i (.; try (foreach ($x as {a:$a} | .) as $y (.; .; .)); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"path(. as $x | foreach . as $i (.; select(true) | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); empty))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"del(. as $x | foreach . as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); empty))",
            "{\"a\":1}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach . as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); select(false))) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"[path(. as $x | foreach . as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); first(empty)))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"path(. as $x | foreach . as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); limit(0; .)))",
            "",
            "",
            0,
        ),
        (
            r"[1]",
            r"path(. as $x | foreach . as $i (.; try (. as {a:{b:$c}} | foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"path(. as $x | foreach . as $i (.; try (. as [$p,$q] | foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); empty))",
            "",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach . as $i (.; try (. as {a:$q} ?// [$q] | foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"path(. as $x | foreach . as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .))?; empty))",
            "",
            "",
            0,
        ),
        (
            r"[1]",
            r"path(. as $x | foreach . as $i (.; (. as {a:$q} | foreach (try ($x as {a:$a} | .)) as $y (.; .; .))?; empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"path(. as $x | foreach . as $i (.; $x | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"path(. as $x | foreach . as $i (.; foreach (1,2) as [$u] (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .); empty))",
            "",
            "Invalid path expression near attempt to access element 0 of 1",
            5,
        ),
        (
            r"[1]",
            r"path(. as $x | foreach . as $i (.; try (foreach (1,2) as [$u] (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .)); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"path(. as $x | foreach . as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | not; empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"path(. as $x | foreach . as $i (.; foreach . as $y (.; $x; .); empty))",
            "",
            "",
            0,
        ),
    ])?;
    // #4957: an uncaught destructure of the register as the UPDATE answers as jq does -- it raises on `[1]` (the pattern
    // needs an object) and emits nothing on an object -- where it was refused.
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":1}"#,
            r"path(. as $x | foreach . as $i (.; foreach ($x as {a:$a} | .) as $y (.; .; .); empty))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach . as $i (.; foreach ($x as {a:$a} | .) as $y (.; .; .); empty))",
            "",
            "",
            0,
        ),
        (
            r"[1]",
            r"path(. as $x | foreach . as $i (.; foreach ($x as {a:$a} | .) as $y (.; .; .); empty))",
            "",
            r#"Cannot index array with string "a""#,
            5,
        ),
    ])?;
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":1}"#,
            r"path(. as $x | foreach . as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i))",
            "",
            "Invalid path expression with result",
            5,
        ),
        (
            r#"{"a":1}"#,
            r"path(. as $x | foreach . as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .))",
            "",
            "Invalid path expression with result",
            5,
        ),
    ])
}

/// #5223 Phase 3: this issue's repro rows, with `path`, `del` and `|=`
/// wherever the row is a `path(...)`, as jq 1.7.1 answers them (stdout, the
/// first error message, the exit code). The path-mode interpreter carries
/// jq's register, so they match by construction, on both routes.
#[test]
fn test_superseded_rows_match_jq_through_the_path_mode_interpreter_4800() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .))",
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .)) |= 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; not | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; not | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .))",
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"(. as $x | foreach (.a,.) as $i (.; not | foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .)) |= 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .))",
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)); .)) |= 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach . as $i (.; 1 | foreach ($x as {a:$a}|.) as $y (.; .; .); $i))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"del(. as $x | foreach . as $i (.; 1 | foreach ($x as {a:$a}|.) as $y (.; .; .); $i))",
            "null\n",
            "",
            0,
        ),
        (
            r"null",
            r"(. as $x | foreach . as $i (.; 1 | foreach ($x as {a:$a}|.) as $y (.; .; .); $i)) |= 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"path(foreach (1) as $i (.; . as {a:$q} | .; $i))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":1}"#,
            r"del(foreach (1) as $i (.; . as {a:$q} | .; $i))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":1}"#,
            r"(foreach (1) as $i (.; . as {a:$q} | .; $i)) |= 9",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r"null",
            r"path(. as $x | foreach . as $i (.; 1 | ($x as {a:$q} | .); $i))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"del(. as $x | foreach . as $i (.; 1 | ($x as {a:$q} | .); $i))",
            "null\n",
            "",
            0,
        ),
        (
            r"null",
            r"(. as $x | foreach . as $i (.; 1 | ($x as {a:$q} | .); $i)) |= 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; (1 | foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // 5; $i))",
            "[\"a\"]\n",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; (1 | foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // 5; $i))",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; (1 | foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // 5; $i)) |= 9",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
    ])
}
