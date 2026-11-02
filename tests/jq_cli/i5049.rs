//! #5049: a caught foreign-`$x` `foreach` UPDATE whose nested `foreach` has a handler in its own UPDATE
//! (`try (.a) catch 7`), and the stage after it.

use anyhow::Result;

use crate::{assert_path_rows_both_routes_3749, run_jq_full};

// `FI = foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .)`. On the outer step whose element is the root,
// `$x` is jq's register, so FI's source moves it onto `$x.a`; FI's `.a` on the `null` state then fails `path_intact`
// (an error its `try` catches, emitting 7), and a stage that is FI again finds `$x` no longer the register: its caught
// source is empty and there is no path. The binary drove FI's source by value, lost the register, and raised an
// uncatchable guess at `.a of null` in every path context.

/// #5049: the issue's rows on both documents (`path`, `[path(..)?]`, `del`, `|=`), the stage-less form's `with result
/// 7`, and the `try (FI) catch 7` UPDATE. Every row captured from `/usr/bin/jq` 1.7.1 (stdout, exit code), both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5049_caught_foreign_foreach_update_with_an_inner_handler_and_the_stage_after_it(
) -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":[null]}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .)) | foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .)) | foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .)) | foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .))",
            "{\"a\":[null]}\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r"(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .)) | foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .)) |= 9",
            "{\"a\":[null]}\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .)))",
            "",
            "Invalid path expression with result 7",
            5,
        ),
        (
            r#"{"a":[null]}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .)))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .)) catch 7) | foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .)) | foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .)) | foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .)) | foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .))",
            "{\"a\":{\"a\":null}}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .)) | foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .)) |= 9",
            "{\"a\":{\"a\":null}}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .)))",
            "",
            "Invalid path expression with result 7",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .)))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; try (foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .)) catch 7) | foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .))",
            "",
            "",
            0,
        ),
    ])
}

/// #5049: a three-argument outer `foreach` whose EXTRACT cannot move the register (`$i`) emits where the inner
/// `foreach`'s source left it, so the stage after it finds `$x` foreign as jq does. Here the inner `foreach`'s EXTRACT is
/// `$y`, emitted at the register its source walked to. A `try` around the whole path used to catch an exact refusal
/// the stage made of the root (jq: no output).
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5049_extract_that_cannot_move_the_register_states_the_walked_step_register() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (.a?,.) as $i (.; foreach (($x as {a:$a} | .)?) as $y (.; try (.[0]) catch 7; $y); $i) | foreach (($x as {a:$a} | .)?) as $y (.; try (.[0]) catch 7; $y)) catch "C""#,
            "",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a?,.) as $i (.; foreach (($x as {a:$a} | .)?) as $y (.; try (.[0]) catch 7; $y); $i) | foreach (($x as {a:$a} | .)?) as $y (.; try (.[0]) catch 7; $y))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a?,.) as $i (.; foreach (($x as {a:$a} | .)?) as $y (.; try (.[0]) catch 7; $y); $i))",
            "",
            "Invalid path expression with result {\"a\":{\"a\":null}}",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a?,.) as $i (.; foreach (($x as {a:$a} | .)?) as $y (.; try (.[0]) catch 7; $y)))",
            "",
            "Invalid path expression with result null",
            5,
        ),
    ])
}

/// #5049: a two-argument `foreach` emits its UPDATE's output with the register that UPDATE left (here `.a | select(true)
/// | 2` leaves it on `.a`), so a later `$x.a` or `.b?` meets jq's own path check rather than a by-value guess.
#[test]
fn test_5049_two_argument_foreach_emission_keeps_the_register_update_left() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"a":true}}"#,
            r"[path(. as $x | .a? | foreach (1,2) as $i (.; .a | select(true) | 2) | $x.a)?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":[null]}}"#,
            r"[path(. as $x | .a as $y | foreach . as $i ($x; .a | select(true) | 2) | $x.a)?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":true}}"#,
            r"path(. as $x | .a? | foreach (1,2) as $i (.; .a | select(true) | 2) | $x.a)",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":{\"a\":true}}",
            5,
        ),
        (
            r#"{"a":{"a":true}}"#,
            r"path(. as $x | .a? | foreach (1,2) as $i (.; .a | select(true) | 2) | .b?)",
            "",
            "Invalid path expression near attempt to access element \"b\" of 2",
            5,
        ),
    ])
}

/// #5049: where the source did not move the inner `foreach`'s step register (`MovedRegister::At`), its EXTRACT's
/// emission states no register: read as the walked step register, differential grids turned the first row's refusal
/// into an accept (jq refuses) and the second's into a write one level short of jq's. Each row must answer as jq does or stay refused (exit 5, nothing on stdout), on both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5049_extract_states_no_register_the_source_did_not_move_alone() -> Result<()> {
    let rows: &[(&str, &str, &str)] = &[
        (
            r#"{"a":{"b":1},"b":{"b":1}}"#,
            r"first(path(. as $x | foreach . as $i (.; if . then foreach ($x as {a:$a} | .) as $y (.a?; try (.a) catch $y; 7) else . end; null) | .a))",
            "",
        ),
        (
            "null",
            r"(. as $x | foreach (.a,.) as $i (.; not | foreach ($x as {a:$a} | .) as $y (null; try (.a) catch $y; .a?); 7) | foreach ($x as {a:$a} | .) as $y (null; try (.a) catch $y; .a?)) |= 9",
            "{\"a\":{\"a\":{\"a\":{\"a\":{\"a\":{\"a\":9}}}}}}\n",
        ),
    ];
    for &(input, filter, jq_stdout) in rows {
        let routes = [
            run_jq_full(&["-c", filter], Some(input))?,
            run_jq_full(&["-nc", &format!("{input} | {filter}")], None)?,
        ];
        for (stdout, stderr, code) in routes {
            // A refusal is exit 5 with nothing on stdout; jq's answer, where it answers, is its stdout at exit 0.
            let refused = code == 5 && stdout.is_empty();
            let answered =
                !jq_stdout.is_empty() && stdout == jq_stdout && code == 0 && stderr.is_empty();
            assert!(
                refused || answered,
                "`{filter}` on {input}: neither jq's {jq_stdout:?} nor a refusal: {stdout:?} {stderr:?} exit {code}"
            );
        }
    }
    Ok(())
}

/// #5049: the same reading on an outer step that is not the `foreach`'s last, or that binds `[$i] ?// $i`, where no
/// `try` around the nested `foreach` is placed (#5004 seeds only a bare `$name` on the last step): the `null` INIT is the
/// state the first step hands it, `$x` is the register there, and `.a` on that `null` is jq's own catchable raise.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5049_inner_handler_reading_on_a_step_that_is_not_the_last_or_not_a_bare_name() -> Result<()>
{
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":[null]}"#,
            r"path(. as $x | foreach (.,.a) as $i (null; foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .)))",
            "",
            "Invalid path expression with result 7",
            5,
        ),
        (
            r#"{"a":[null]}"#,
            r"[path(. as $x | foreach (.,.a) as $i (null; foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .)))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r"path(. as $x | foreach (.,.) as $i (null; foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .)))",
            "",
            "Invalid path expression with result 7",
            5,
        ),
        (
            r#"{"a":[null]}"#,
            r"[path(. as $x | foreach (.,.) as $i (null; foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .)))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r"path(. as $x | foreach (.,.a) as [$i] ?// $i (null; foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .)))",
            "",
            "Invalid path expression with result 7",
            5,
        ),
        (
            r#"{"a":[null]}"#,
            r"[path(. as $x | foreach (.,.a) as [$i] ?// $i (null; foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .)))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.,.a) as $i (null; foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .)))",
            "",
            "Invalid path expression with result 7",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.,.a) as $i (null; foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .)))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.,.) as $i (null; foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .)))",
            "",
            "Invalid path expression with result 7",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.,.) as $i (null; foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .)))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.,.a) as [$i] ?// $i (null; foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .)))",
            "",
            "Invalid path expression with result 7",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.,.a) as [$i] ?// $i (null; foreach (try ($x as {a:$a} | .)) as $y (.; try (.a) catch 7; .)))?]",
            "[]\n",
            "",
            0,
        ),
    ])
}
