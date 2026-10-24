//! #5113: a `foreach` UPDATE `if` whose literal `then` ran beside a fold in the `else`, on the last step of an INIT fork
//! that is not the last.

use anyhow::Result;

use crate::{assert_path_rows_both_routes_3749, run_jq_full};

// jq stores the literal the `then` arm emits as the state, and the next INIT fork replaces it with INIT's own output
// (the `foreach` re-runs INIT), so on the last step of any INIT fork the literal feeds no later UPDATE: the `if` is
// resolved as that arm, as #5104 does on the last step of the last fork. jq's `foreach` hands the source the input
// only once (`DUPN`), so a later fork's source runs on `null` and its path error is caught by the `?` around it.

/// #5113: the issue's two-value INIT row (and the same over an INIT whose count is unbounded, `(.,.) | .` and
/// `range(2) as $n | .`: only the source's bound says the step is the fork's last), and rows of `scripts/jq-foreach-update-if-arm-sweep.py` (with `null`, `true`
/// and `false` literal arms added) that `main` answered differently and now match. Captured from `/usr/bin/jq` 1.7.1,
/// both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5113_literal_then_on_the_last_step_of_an_earlier_init_fork() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach .a as $i ((.,.); try (if .a then 1 else (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end) catch 1; $i))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach . as $i ((., .a); (7, if .a then 1 elif . then 2 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end); $i))",
            "[]\n[]\n",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach .a as $i ((., .a); try (if . then null else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end) catch 1; .a))",
            "[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach . as $i ((., .a); (7, if .a then true else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end); $i))",
            "[]\n[]\n",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"b":true}"#,
            r"(. as $x | foreach .a as $i ((., .a); try (if .b then null else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end) catch .; $i)) |= 9",
            "{\"b\":true,\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach .a as $i ((., .a); try (if .a then 1 elif . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else 2 end) catch 1; $i))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach .a as $i ((.,.) | .; try (if .a then 1 else (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end) catch 1; $i))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach .a as $i (range(2) as $n | .; try (if .a then 1 else (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end) catch 1; $i))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
    ])
}

/// #5113 review: where the literal `then` must still not be read, so the step keeps its whole-`if` refusal: a `?//`
/// pattern (a retry runs UPDATE again on the stored literal; read there, the first row wrote `{"b":true,"a":{"a":9}}`
/// where jq raises), a step that is not its fork's last (the next step runs the fold on the stored literal; read
/// there, a `?` truncated jq's answer), and an UPDATE whose fold source is a `?//` chain (read on the first of three
/// forks, the later forks answered nothing where jq emits `["a"]`). Each row must answer as jq does or stay refused, on both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5113_literal_then_stays_unread_under_a_retry_or_before_the_forks_last_step() -> Result<()> {
    // (document, filter, jq's stdout)
    let rows: &[(&str, &str, &str)] = &[
        (
            r#"{"b":true}"#,
            r"(. as $x | foreach .a as {a:$k} ?// $k ((.,.a); (if . then true else (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end, 7); $k)) |= 9",
            "",
        ),
        (
            r#"{"b":true}"#,
            r"[path(. as $x | foreach (.a,.) as $i ((.,.); try (if .b then null else (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end) catch 1; .a))?]",
            "[[\"a\",\"a\"],[\"a\",\"a\"]]\n",
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach . as $i ((., null, .a); try (if . then 1 else foreach (try ($x as {a:$a} ?// [$a] | .)) as $y (.; .; .) end) catch 1; $i))",
            "[]\n[\"a\"]\n",
        ),
    ];
    for &(input, filter, jq_stdout) in rows {
        for (stdout, stderr, code) in [
            run_jq_full(&["-c", filter], Some(input))?,
            run_jq_full(&["-nc", &format!("{input} | {filter}")], None)?,
        ] {
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

// A handler `try` around an `if` hid the arm the state took from the resolve, which narrows a handler `try` only to a
// literal arm: `try (if .b then null else FE end) catch 1` on a `null` state ran the computed `else` and was refused
// where the bare `FE` and `try (FE) catch 1` answer. And an `if` whose condition raises on the state (`.a` of the
// stored `1`) never reaches an arm: the literal handler answers.

/// #5113 rows 2-3: the two single-step reductions from the findings comment, with `path`, `[path()?]` and `del`, on
/// both routes. Captured from `/usr/bin/jq` 1.7.1.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5113_handler_try_around_an_if_takes_the_arm_or_the_handler_the_state_ran() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"b":true}"#,
            r"path(. as $x | foreach (.) as $i (null; try (if .b then null else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end) catch 1; .a))",
            "[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"b":true}"#,
            r"[path(. as $x | foreach (.) as $i (null; try (if .b then null else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end) catch 1; .a))?]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"b":true}"#,
            r"del(. as $x | foreach (.) as $i (null; try (if .b then null else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end) catch 1; .a))",
            "{\"b\":true}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.) as $i (1; try (if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end) catch 1; $i))",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.) as $i (1; try (if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end) catch 1; $i))?]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach (.) as $i (1; try (if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end) catch 1; $i))",
            "null\n",
            "",
            0,
        ),
    ])
}

/// #5113: under a `?//` loop pattern the same step is the retry case (the stored literal feeds the next step), which
/// is not modelled: jq's answers are read there only where they match, and `main`'s refusal is kept otherwise. A row
/// of the sweep where reading the handler's literal after a raising condition wrote `null` where jq leaves the
/// document alone.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5113_handler_try_around_an_if_stays_refused_under_a_retry_chain() -> Result<()> {
    let input = r#"{"b":true}"#;
    let filter = r"del(. as $x | foreach . as {a:$i} ?// $i (.; try (if .a then foreach (try ($x as {a:$a} ?// [$a] | .)) as $y (.; .; .) elif . then 1 else 2 end) catch 1; $i))";
    for (stdout, stderr, code) in [
        run_jq_full(&["-c", filter], Some(input))?,
        run_jq_full(&["-nc", &format!("{input} | {filter}")], None)?,
    ] {
        assert!(
            (code == 0 && stdout == "{\"b\":true}\n" && stderr.is_empty())
                || (code == 5 && stdout.is_empty()),
            "`{filter}`: neither jq's answer nor a refusal: {stdout:?} {stderr:?} exit {code}"
        );
    }
    Ok(())
}

// A literal `then` a step stores feeds the next step's UPDATE. When every step after it is itself resolved as a literal
// arm (the stored `1` makes `.a` raise and the handler's literal answer, or the condition takes a literal `else`), no
// step runs the fold on a stored literal, so the `then` is read on a step that is not the fork's last.

/// #5113 row 2: `(.a,.)` as the source, so the step that stores the literal `1` is not the last, and the one after it
/// raises on that `1` and takes the handler. Longer sources and an `elif` chain whose later steps stay on the literal
/// arms. Captured from `/usr/bin/jq` 1.7.1, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5113_literal_then_read_when_every_later_step_stores_a_literal() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; try (if .a then 1 else (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end) catch 1; $i))?]",
            "[[\"a\"],[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; try (if .a then 1 else (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end) catch 1; $i))",
            "[\"a\"]\n[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.,.) as $i (.; try (if .a then 1 else (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end) catch 1; $i))?]",
            "[[\"a\"],[],[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.,.a) as $i (.; try (if .a then 1 elif . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else 2 end) catch 1; $i))?]",
            "[[\"a\"],[],[\"a\"]]\n",
            "",
            0,
        ),
    ])
}

/// #5113: a source longer than the lookahead's eight-step cap is read through its cycle: the stored literals repeat
/// (`null`, then `1`, then `1` again), so a step that enters with a state an earlier one entered with repeats the steps
/// between, which were already read. A source of ten elements keeps the literal arm on its first step, with a handler
/// (`catch 1`) and beside a literal comma sibling. Captured from `/usr/bin/jq` 1.7.1, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5113_literal_then_lookahead_reads_a_source_past_eight_steps_through_its_cycle() -> Result<()>
{
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.,.,.,.,.,.,.,.,.) as $i (.; try (if .a then 1 else (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end) catch 1; $i))?]",
            "[[\"a\"],[],[],[],[],[],[],[],[],[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.,.,.,.,.,.,.,.,.) as $i (.; (try (if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end) catch 1, 7); $i))?]",
            "[[\"a\"],[\"a\"],[],[],[],[],[],[],[],[],[],[],[],[],[],[],[],[],[],[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.a,.a,.a,.a,.a,.a,.a,.a,.a) as $i (.; try (if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end) catch 1; $i))?]",
            "[[\"a\"],[\"a\"],[\"a\"],[\"a\"],[\"a\"],[\"a\"],[\"a\"],[\"a\"],[\"a\"],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.,.a,.,.a,.,.a,.,.a,.,.a) as $i (.; (if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end, 7); $i))?]",
            "[[],[]]\n",
            "",
            0,
        ),
    ])
}

/// #5113: a source whose bound no static shape says (`.[]`, `range(n)`) may have any number of steps left, so the
/// literal arm is read when every step that could follow stores a literal too, or raises (the cycle of stored states
/// is read as the bounded source's is). Captured from `/usr/bin/jq` 1.7.1, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5113_literal_then_lookahead_reads_an_unbounded_source() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach .[] as $i (.; try (if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end) catch 1; $i))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach .[] as $i (.; (if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end, 7); $i))?]",
            "[[\"a\"],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach .[] as $i (.; (if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end, 7); $i))",
            "[\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"(. as $x | foreach .[] as $i (.; (if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end, 7); $i)) |= 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null},"b":{"a":null}}"#,
            r"[path(. as $x | foreach .[] as $i (.; (if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end, 7); $i))?]",
            "[[\"a\"],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach .[] as $i ((., .a); try (if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end) catch 1; $i))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach .[] as $i ((., null, .a); try (if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end) catch 1; $i))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            "[1]",
            r"[path(. as $x | foreach .[] as $i (.; (if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end, 7); $i))?]",
            "[]\n",
            "",
            0,
        ),
    ])
}

/// #5113 review: the lookahead reads a step's handler literal only where the real last step does. On a fork that is not
/// INIT's last, the fork's last step takes the whole-`if` resolve and refuses, so the earlier step's literal `then` must
/// not be read either: it printed `["a"]` before the refusal where `main` printed nothing. jq answers four `["a"]`.
///
/// #5223: the path-mode interpreter carries jq's register, so the rows the shipped resolver
/// refused here now hold jq 1.7.1's answers.
#[cfg(not(feature = "unshared-containers"))]
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5113_lookahead_agrees_with_the_last_step_of_an_earlier_init_fork() -> Result<()> {
    assert_path_rows_both_routes_3749(&[(
        "null",
        r"path(. as $x | foreach (.a,.a) as $i ((.,.); try (if not then 0 elif .b then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end) catch 2; $i))",
        "[\"a\"]\n[\"a\"]\n[\"a\"]\n[\"a\"]\n",
        "",
        0,
    )])
}

// A `foreach` that is the whole UPDATE of an enclosing `foreach` and whose source is a caught destructure of an outer
// `$x` emits nothing on the steps where `$x` is not the register: the pattern's `INDEX` fails `path_intact`, the `try`
// catches it, and the outer step stores `null`. The step's register is the element its source moved onto, so a `$x`
// holding a container other than that element's value is never it, whatever position (or none) the `$x` was bound at.
// `main` read the bind as passing whenever the nested `foreach` emitted, and stated EXTRACT from the bound child.

/// #5113: the wrong write. `foreach (.a) as $i (.; FE; $i)` on `{"a":null}` is empty in jq (the one step's `$x` is the
/// root, the register is on `.a`'s `null`), where `main` answered `["a","a"]` and `|= 9` wrote `{"a":{"a":9}}`. Every
/// EXTRACT, the three origins a `$x` marker has (`. as`, `.b as`, `(.b|.) as`), the shapes of the caught source (`?`,
/// `catch empty`, an array pattern) and a state-replacing UPDATE. Captured from `/usr/bin/jq` 1.7.1, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5113_caught_bind_foreach_update_is_empty_on_a_step_whose_element_is_not_the_marker(
) -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.a) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | (foreach (.a) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | del(foreach (.a) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.a) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); null))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#". as $x | [path(foreach (.a) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); "k"))]"#,
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.a) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r". as $x | [path(foreach (.a) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r". as $x | (foreach (.a) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i)) |= 9",
            "{\"a\":1}\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":{"a":1}}"#,
            r".b as $x | [path(foreach (.a) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":{"a":1}}"#,
            r"(.b|.) as $x | [path(foreach (.a) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"c":[null]}"#,
            r".c as $x | [path(foreach (.a) as $i (.; foreach (try ($x as [$a] | .)) as $y (.; .; .); $i))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.a) as $i (.; foreach (($x as {a:$a} | .)?) as $y (.; .; .); $i))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.a) as $i (.; foreach (try ($x as {a:$a} | .) catch empty) as $y (.; .; .); $i))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.a) as $i (.; foreach (try ($x as {a:$a} ?// [$a] | .)) as $y (.; .; .); $i))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.a) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; $y; .); $i))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":null}"#,
            r".b as $x | [path(foreach (.a) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i))]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
    ])
}

/// #5113: the controls the reading above must not touch. A step whose element IS the root keeps the bind passing
/// (`(.a,.)`: the first step is empty, the second states the root's `["a"]`); `foreach (.a) ... (.; FE; .)` on
/// `{"a":{"a":null}}` is empty and `.a as $x` over `{"a":{"a":1}}` is the register's own node (equal container,
/// not read), so both keep the answers they had. So does a `?//` chain source, whose retry is not modelled: read as empty
/// on the steps its first alternative fails, `limit(1; FE) ... |= 9` over `..` wrote nothing where jq writes `9`. Captured
/// from `/usr/bin/jq` 1.7.1, both routes. A register that holds a `nan` is its own node though `==` says otherwise
/// (`holds_nan`): the marker equal to it still passes the step.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5113_caught_bind_foreach_update_keeps_the_steps_where_the_marker_is_the_register(
) -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .))]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | path(foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i))",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | path(foreach (.,.a) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i))",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r". as $x | path(foreach (.,.a) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .a))",
            "",
            "Invalid path expression near attempt to access element \"a\" o",
            5,
        ),
        (
            r#"{"a":{"a":1}}"#,
            r".a as $x | [path(foreach (.a) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); $i))]",
            "",
            "Invalid path expression with result {\"a\":1}",
            5,
        ),
        (
            r#"{"a":{"a":1}}"#,
            r".a as $x | [path(foreach (.a) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .))]",
            "",
            "Invalid path expression with result {\"a\":{\"a\":1}}",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r". as $x | path(foreach (.a) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .); .))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":false,"b":{"a":null}}"#,
            r".b as $x | (foreach (..) as $i (null; limit(1; foreach (try ($x as {a:$a} ?// [$a] | .)) as $y (null; .; .)); .)) |= 9",
            "{\"a\":false,\"b\":{\"a\":9}}\n",
            "",
            0,
        ),
        (
            r#"{"a":[nan],"b":1}"#,
            r".a as $x | [path(foreach (.a) as $i (.; foreach (try ($x as [$q] | .)) as $y (.; .; .); $i))]",
            "",
            "Invalid path expression with result [null]",
            5,
        ),
        (
            r#"{"a":{"x":[nan]},"b":1}"#,
            r".a as $x | [path(foreach (.a) as $i (.; foreach (try ($x as {x:$q} | .)) as $y (.; .; .); $i))]",
            "",
            "Invalid path expression with result {\"x\":[null]}",
            5,
        ),
    ])
}

/// #5113: a `null` marker is `jv_identical` to the `null` register of the first step and so passes it, and jq answers
/// `[["a","a"]]`. Reading the second step as empty let the first step's own placement through, as `["a"]` (a grid of 40,000
/// rows turned five refusals into wrong answers, `del` among them), until #5205 placed it. It was pinned as a refusal before.
/// The pipe-spelled `$x | . as P` source, pinned here before #5206, is read now (`tests/jq_cli/i5206.rs`).
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5113_caught_bind_foreach_update_leaves_a_null_marker_5205() -> Result<()> {
    assert_path_rows_both_routes_3749(&[(
        r#"{"a":null}"#,
        r".a as $x | [path(foreach ((.a,.)) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (null; .; .) | .); $i))]",
        "[[\"a\",\"a\"]]\n",
        "",
        0,
    )])
}

// A comma UPDATE stores its last emission, so the literal an `if` item emits is not the state the next step enters with
// when a literal sibling follows it (`(if .a then 1 else FE end, 7)` stores `7`). The literal arm is read on a step
// that is not the fork's last when the lookahead, started from the sibling's literal, finds every later step stores a
// literal too or raises with no handler (which ends the `foreach`). A step the lookahead cannot place keeps the
// whole-`if` resolve.

/// #5113: a literal sibling after the `if` (`null` and `true` `then` literals, an `elif` chain, 2-3 element sources,
/// the `path`, `|=` and `del` contexts), and a source of ten elements whose second step raises (it ends the `foreach`,
/// past the eight-step cap). Captured from `/usr/bin/jq` 1.7.1, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5113_literal_then_read_beside_a_literal_sibling_after_the_if() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; (if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end, 7); $i))?]",
            "[[\"a\"],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.,.) as $i (.; (if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end, 7); $i))?]",
            "[[\"a\"],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; (if .a then null else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end, 7); $i))?]",
            "[[\"a\"],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; (if .a then true else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end, 7); $i))?]",
            "[[\"a\"],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; (if .a then 1 elif . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else 2 end, 7); $i))?]",
            "[[\"a\"],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; (if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end, 7); $i))",
            "[\"a\"]\n[\"a\"]\n",
            "Cannot index number with string \"a\"",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"(. as $x | foreach (.a,.) as $i (.; (if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end, 7); $i)) |= 9",
            "",
            "Cannot index number with string \"a\"",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; (if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end, 7); $i))",
            "",
            "Cannot index number with string \"a\"",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.,.,.,.,.,.,.,.,.) as $i (.; (if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end, 7); $i))?]",
            "[[\"a\"],[\"a\"]]\n",
            "",
            0,
        ),
    ])
}

/// #5113: the literal sibling ahead of the `if`: the `if` is the comma's last item, so the literal it emits is the
/// stored state, and the lookahead steps through the comma (`7` then the `if` item read as its literal arm).
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5113_literal_then_read_beside_a_literal_sibling_before_the_if() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; (7, if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end); $i))?]",
            "[[\"a\"],[\"a\"],[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.,.) as $i (.; (7, if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end); $i))?]",
            "[[\"a\"],[\"a\"],[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.,.a,.) as $i (.; (7, if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end); $i))?]",
            "[[],[],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.,.) as $i (.; (7, if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end); $i))",
            "[\"a\"]\n[\"a\"]\n[]\n",
            "Cannot index number with string \"a\"",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; (7, if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end); $i))",
            "",
            "Cannot index number with string \"a\"",
            5,
        ),
    ])
}

/// #5113: a handler `try` around the `if`, beside a literal sibling: the handler literal is the arm a later step takes
/// when its condition raises on the stored `7`.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5113_handler_literal_read_beside_a_literal_sibling() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; (try (if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end) catch 1, 7); $i))?]",
            "[[\"a\"],[\"a\"],[],[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.,.) as $i (.; (try (if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end) catch 1, 7); $i))?]",
            "[[\"a\"],[\"a\"],[],[],[],[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a) as $i (.; (try (if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end) catch 1, 7); $i))?]",
            "[[\"a\"],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; (try (if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end) catch 1, 7); $i))",
            "[\"a\"]\n[\"a\"]\n[]\n[]\n",
            "",
            0,
        ),
    ])
}

/// #5113: what the reading leaves as it was, pinned as it stands (jq answers each; a fix flips them). A sibling that is
/// not a literal (`.b?`) stores a computed state; a `null` sibling makes the next step's `.a` falsy, so it takes the
/// computed `else`; a `?//` pattern retries UPDATE on the stored literal; and a trailing `$i` stores each step's own
/// element, which a lookahead started from the `then` literal cannot see (it printed a truncated answer before the
/// lookahead ran on the unbound UPDATE). jq answers `[["a"]]`, `[["a"],["a"]]`, `[["a"],["a"]]`,
/// `[["a","a"],["a","a"]]` and `[[],[],["b"],["b"],["a"]]`.
#[cfg(not(feature = "unshared-containers"))]
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5113_literal_sibling_reading_stays_unread_where_the_lookahead_cannot_place_the_step(
) -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; (if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end, .b?); $i))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; (if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end, null); $i))?]",
            "[[\"a\"],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; (if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end, 7, .b?); $i))?]",
            "[[\"a\"],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as {a:$i} ?// $i (.; (if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end, 7); $i))?]",
            "[[\"a\",\"a\"],[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.,.b,.a) as $i (.; (if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end, $i); $i))?]",
            "[[],[],[\"b\"],[\"b\"],[\"a\"]]\n",
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
fn test_superseded_rows_match_jq_through_the_path_mode_interpreter_5113() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach .a as $i ((.,.); try (if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end) catch 1; $i))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; try (if .a then 1 else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end) catch 1; $i))?]",
            "[[\"a\"],[]]\n",
            "",
            0,
        ),
        (
            r#"{"b":true}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; try (if .b then null else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end) catch 1; .a))?]",
            "[[\"a\",\"a\"],[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"[path(. as $x | foreach .a as {a:$k} ?// $k (.; (if .a then null else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end, 7); if $k == null then error("x") else $k end))?]"#,
            "[[\"a\"]]\n",
            "",
            0,
        ),
    ])
}
