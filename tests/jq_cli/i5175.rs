//! #5175: a `//` over a bind of an outer `$x` in a `reduce` or two-argument `foreach` UPDATE whose right operand holds a
//! bind of its own.

use anyhow::Result;

use crate::{assert_path_rows_both_routes_3749, run_jq_stdin};

// #5167 read the right operand of the `//` only when it held no bind. An operand with one (`.a as $q | 1`,
// `$x as {a:$a} | .`) kept the resolver's refusal, a guess that a `try`/`?` around the `//` swallowed, so the UPDATE
// emitted nothing and `path`/`del`/`|=` answered (or wrote) where jq raises at the emission of the left operand's
// truthy output. jq runs the right operand only after the left one backtracked, from the register the `//` was entered
// with, and runs a bind's source inside a subexpression that tracks no path, so the operand is read as the left one is: a
// chain of binds that each run once, whose source is `.`, an outer `$x` or a navigation of `.`.

/// #5175: the issue's row 2 (a navigation-source bind in the right operand), for both folds, and the `|=` write that ran
/// where jq raises. Every row captured from `/usr/bin/jq` 1.7.1, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5175_alternative_with_a_navigation_source_bind_in_the_right_operand() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | foreach (1,2) as $i (false; try (($x as {a:$a} | 5) // .a as $q | 1)))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | reduce (1,2) as $i (false; try (($x as {a:$a} | 5) // .a as $q | 1)))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r"null",
            r"(. as $x | foreach (.a,.) as $i (5; try (($x as {a:$a} | .) // (.a as $q | $q)))) |= 9",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r"null",
            r"(. as $x | reduce (.a,.) as $i (5; try (($x as {a:$a} | .) // (.a as $q | $q)))) |= 9",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r"null",
            r"path(. as $x | reduce (.,.a) as $i (null; try (($x as [$a] | 5) // (.a as $q | $q))))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | foreach (.,.a) as $i (null; try (($x as {a:{b:$a}} | .) // .a as $q | 1)))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
    ])
}

/// #5175: a right operand that destructures the outer `$x` again, which jq runs from the register the `//` was entered with.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5175_alternative_with_a_destructuring_bind_in_the_right_operand() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1}}"#,
            r"del(. as $x | foreach (1,2) as $i (null; try (($x as {a:$a} | .) // ($x as {a:$a} | .))))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(. as $x | reduce (1,2) as $i (null; try (($x as {a:$a} | .) // ($x as {a:$a} | .))))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        // a falsy left output hands the register back, so the operand's own bind runs from it and the fold ends clean
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | foreach (1,2) as $i (.; try (($x as {a:$a} | false) // .a as $q | .)))",
            "[]\n[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | reduce (1,2) as $i (.; try (($x as {a:$a} | false) // .a as $q | .)))",
            "[]\n",
            "",
            0,
        ),
    ])
}

/// #5175: the three-argument `foreach`, whose EXTRACT is the emission, reads the same right operand. Rows captured from
/// `/usr/bin/jq` 1.7.1, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5175_three_argument_foreach_with_a_bind_in_the_right_operand() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | foreach (1,2) as $i (false; try (($x as {a:$a} | 5) // .a as $q | 1); .))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r"null",
            r"(. as $x | foreach (.a,.) as $i (5; try (($x as {a:$a} | .) // (.a as $q | $q)); $i)) |= 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | foreach (1,2) as $i (.; try (($x as {a:$a} | false) // .a as $q | .); $i))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
    ])
}

/// #5175: a right operand with a bind behind a left operand that fans out stays refused. The bare reading answered
/// `["a"]` once where jq answers it twice (the #5170 under-count), and `del`/`=` write through such a count.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5175_left_operand_that_fans_out_is_not_read_with_a_bind_in_the_right_operand() -> Result<()>
{
    let (stdout, _code) = run_jq_stdin(
        r"path(. as $x | foreach . as $i (5; ($x as {a:$a} | (.,.)) // ($x as {a:$a} | .); $x))",
        "null",
        &["-c"],
    )?;
    assert_ne!(
        stdout, "[\"a\"]\n",
        "answered one path where jq answers two"
    );
    Ok(())
}

/// #5175: a comma around the binds of the right operand, whose items each run from the register the `//` was entered with. Before, the operand kept the resolver's refusal, a `try` around the `//` swallowed it, and
/// `path`/`del`/`|=` answered where jq raises at the emission of the left operand's truthy output. Every row captured from
/// `/usr/bin/jq` 1.7.1, both routes, for a `reduce`, a two-argument and a three-argument `foreach`.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5175_right_operand_with_a_comma() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (1,2) as $i (false; try ((($x as {a:$a} | 5)) // ((.a as $q | $q), 3))))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (false; try ((($x as {a:$a} | 5)) // ((.a as $q | $q), 3)); .))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r"null",
            r"(. as $x | reduce (1,2) as $i (null; try ((($x as {a:$a} | 5)) // ((.a as $q | $q), 3)))) |= 9",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (1,2) as $i (false; try ((($x as {a:$a} | 5)) // ((.a as $q | $q), (.a as $q | 1)))))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (false; try ((($x as {a:$a} | 5)) // ((.a as $q | $q), (.a as $q | 1))); .))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r"null",
            r"(. as $x | reduce (1,2) as $i (null; try ((($x as {a:$a} | 5)) // ((.a as $q | $q), (.a as $q | 1))))) |= 9",
            "",
            "Invalid path expression with result 5",
            5,
        ),
    ])
}

/// #5175: an `if` around the binds of the right operand (its condition cannot move the register). Before, the operand kept the resolver's refusal, a `try` around the `//` swallowed it, and
/// `path`/`del`/`|=` answered where jq raises at the emission of the left operand's truthy output. Every row captured from
/// `/usr/bin/jq` 1.7.1, both routes, for a `reduce`, a two-argument and a three-argument `foreach`.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5175_right_operand_with_an_if() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (1,2) as $i (false; try ((($x as {a:$a} | 5)) // (if true then (.a as $q | $q) else 1 end))))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (false; try ((($x as {a:$a} | 5)) // (if true then (.a as $q | $q) else 1 end)); .))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r"null",
            r"(. as $x | reduce (1,2) as $i (null; try ((($x as {a:$a} | 5)) // (if true then (.a as $q | $q) else 1 end)))) |= 9",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (1,2) as $i (false; try ((($x as {a:$a} | 5)) // (if $i then ($x as {a:$a} | .) else (.a as $q | 1) end))))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (false; try ((($x as {a:$a} | 5)) // (if $i then ($x as {a:$a} | .) else (.a as $q | 1) end)); .))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r"null",
            r"(. as $x | reduce (1,2) as $i (null; try ((($x as {a:$a} | 5)) // (if $i then ($x as {a:$a} | .) else (.a as $q | 1) end)))) |= 9",
            "",
            "Invalid path expression with result 5",
            5,
        ),
    ])
}

/// #5175: a bind in the right operand whose source is a comma of sources or `.[]`, so its body runs once per source output. Before, the operand kept the resolver's refusal, a `try` around the `//` swallowed it, and
/// `path`/`del`/`|=` answered where jq raises at the emission of the left operand's truthy output. Every row captured from
/// `/usr/bin/jq` 1.7.1, both routes, for a `reduce`, a two-argument and a three-argument `foreach`.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5175_right_operand_with_a_generator_source() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (1,2) as $i (false; try ((($x as {a:$a} | 5)) // (.[] as $q | $q))))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (false; try ((($x as {a:$a} | 5)) // (.[] as $q | $q)); .))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r"null",
            r"(. as $x | reduce (1,2) as $i (null; try ((($x as {a:$a} | 5)) // (.[] as $q | $q)))) |= 9",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (1,2) as $i (false; try ((($x as {a:$a} | 5)) // ((.a, .) as $q | 1))))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (false; try ((($x as {a:$a} | 5)) // ((.a, .) as $q | 1)); .))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r"null",
            r"(. as $x | reduce (1,2) as $i (null; try ((($x as {a:$a} | 5)) // ((.a, .) as $q | 1)))) |= 9",
            "",
            "Invalid path expression with result 5",
            5,
        ),
    ])
}

/// #5175: a right operand whose bind bodies fan out, or that nests two generator sources, is not read: the resolver
/// delivers only the first output of a generator body when the state is not the register (#5193), so reading it would risk
/// a truncated sequence. Every row raises in jq (the left operand's truthy `5`) and still raises here. Over the sweep grid
/// the guard is observable only in the wording of the step (`of 0` where unguarded, `of null` guarded, jq's `of 1`), so
/// this pins the refusal, not the wording.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5175_right_operand_that_fans_out_through_a_body_or_nested_sources_stays_unread(
) -> Result<()> {
    for program in [
        r"path(. as $x | reduce (1,2) as $i (false; try (($x as {a:$a} | 5) // ((1,2) as $q | (3,4) as $r | $r))))",
        r"path(. as $x | reduce (1,2) as $i (false; try (($x as {a:$a} | 5) // ((.a as $q | range(2)), 3))))",
        r"path(. as $x | reduce (1,2) as $i (false; try (($x as {a:$a} | 5) // (if true then (.a as $q | range(2)) else 1 end))))",
    ] {
        let (stdout, code) = run_jq_stdin(program, r#"{"a":{"b":1}}"#, &["-c"])?;
        assert_eq!(
            (stdout.as_str(), code),
            ("", 5),
            "{program} answered where jq raises"
        );
    }
    Ok(())
}

/// #5175: a bind in the right operand whose source navigates an outer `$x` (`$x.a`, `$x.a.b`). jq runs a bind's source in a
/// subexpression that tracks no path, so it moves no register, as `.a` and a bare `$x` do. Before, the source kept the
/// resolver's refusal, a `try` around the `//` swallowed it, and `path`/`del`/`|=` answered (or wrote) where jq raises at the
/// emission of the left operand's truthy `5`. Every row captured from `/usr/bin/jq` 1.7.1, both routes, for a `reduce`, a
/// two-argument and a three-argument `foreach`; the last row is a falsy left operand, where the right operand runs and jq
/// answers.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5175_right_operand_with_a_bind_source_that_navigates_an_outer_marker() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | foreach (1,2) as $i (false; try (($x as {a:$a} | 5) // ($x.a as $q | $q))))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | reduce (1,2) as $i (false; try (($x as {a:$a} | 5) // ($x.a.b as $q | $q))))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | foreach (1,2) as $i (false; try (($x as {a:$a} | 5) // ($x.a.b as {c:$c} | $c))))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(. as $x | foreach (.a,.) as $i (false; try (($x as {a:$a} | 5) // ($x.a as $q | $q)); .))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r"null",
            r"(. as $x | reduce (1,2) as $i (null; try (($x as {a:$a} | 5) // ($x.a as $q | $q)))) |= 9",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | reduce (1,2) as $i (.; (($x as {a:$a} | false) // ($x.a as $q | .))))",
            "[]\n",
            "",
            0,
        ),
    ])
}

/// #5223 Phase 3: this issue's repro rows, with `path`, `del` and `|=`
/// wherever the row is a `path(...)`, as jq 1.7.1 answers them (stdout, the
/// first error message, the exit code). The path-mode interpreter carries
/// jq's register, so they match by construction, on both routes.
#[cfg(not(feature = "unshared-containers"))]
#[test]
fn test_superseded_rows_match_jq_through_the_path_mode_interpreter_5175() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"path(. as $x | reduce (.a,.) as $i (.; (($x as {a:$a} ?// $a | $a) // 5)?))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | reduce (.a,.) as $i (.; (($x as {a:$a} ?// $a | $a) // 5)?))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | reduce (.a,.) as $i (.; (($x as {a:$a} ?// $a | $a) // 5)?)) |= 9",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | foreach (1,2) as $i (false; try (($x as {a:$a} | 5) // .a as $q | 1)))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(. as $x | foreach (1,2) as $i (false; try (($x as {a:$a} | 5) // .a as $q | 1)))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"(. as $x | foreach (1,2) as $i (false; try (($x as {a:$a} | 5) // .a as $q | 1))) |= 9",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | reduce (1,2) as $i (.; try (($x as {a:$a} | .) // $x)))",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | reduce (1,2) as $i (.; try (($x as {a:$a} | .) // $x)))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | reduce (1,2) as $i (.; try (($x as {a:$a} | .) // $x))) |= 9",
            "9\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | reduce (.a,.) as $i (null; (($x as {a:$a} ?// $a | .a) // 5)?))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | reduce (.a,.) as $i (null; (($x as {a:$a} ?// $a | .a) // 5)?))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | reduce (.a,.) as $i (null; (($x as {a:$a} ?// $a | .a) // 5)?)) |= 9",
            "",
            "Invalid path expression with result null",
            5,
        ),
    ])
}
