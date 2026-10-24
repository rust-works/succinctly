//! #5156: a caught foreign-`$x` `foreach` behind a `//` with a non-literal alternate that is a `foreach` UPDATE.
//!
//! #5114 read a literal alternate (`// 5`) as the `(..) // 5 | .` pipe jq runs it as. For the alternate `.` (and `$x`, `{b:1}`
//! and the rest of the value heads) the step falls back to a value whose standing is the register the `//` entered with:
//! jq backtracks out of everything the left operand navigated, so the alternate's output is at the step's entry register.
//! The branch said so (`BranchRegister::AtEntry`) but the fold's relocation dropped it for a branch it could not certify
//! as the register's own, so the `$i` EXTRACT of the first step was refused with the state, where jq emits its path.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

/// #5156: the issue's rows (`path`, `|= 9`, `del` over `// .`) and rows of a differential grid (document x source x caught-bind
/// shape x `//` alternate x wrapper x EXTRACT x `path`/`?`/`|=`/`del`) that were refused or answered wrongly before the fix.
/// Every row captured from `/usr/bin/jq` 1.7.1 (stdout, exit code, message), both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5156_foreach_update_alternative_with_a_value_alternate_of_a_caught_foreign_foreach(
) -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // .; $i))",
            "[\"a\"]\n",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // .; $i)) = 9",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // .; $i))",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // .; $i))",
            "[\"a\"]\n",
            "Invalid path expression with result {\"a\":{\"a\":null}}",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // .; $i)) = 9",
            "",
            "Invalid path expression with result {\"a\":{\"a\":null}}",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // .; $i))",
            "",
            "Invalid path expression with result {\"a\":{\"a\":null}}",
            5,
        ),
        (
            r#"{"a":true}"#,
            r"del(. as $x | foreach (.a) as $i (.; ((foreach (try ($x as {c:$a} | .)) as $y (.; $y; .))|.) // $x; $i))",
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(.a? as $x | foreach (.,.a,.) as $i (0; (foreach (try ($x as {c:$a} | .)) as $y (null; .; .)) // (.,.); $i))?]",
            "[[],[],[\"a\"],[\"a\"],[],[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"try path(. as $x | foreach (.a?,.) as $i (.; ((try (foreach (try ($x as {a:$a} | .)) as $y (0; .; $y))) // {b:1})|.; $i)) catch 5",
            "[\"a\"]\n5\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"[path(.a? as $x | foreach (.a?,.) as $i (.; ((foreach (try ($x as {a:{a:$a}} | .)) as $y (null; .; .))|.) // (.,.); $i))?]",
            "[[\"a\"],[\"a\"],[],[]]\n",
            "",
            0,
        ),
        (
            r"[null]",
            r"try path(. as $x | foreach (.[]?) as $i (.; ((foreach (try ($x as {c:$a} | .)) as $y (0; .; .))|.) // (.,.); $i)) catch 5",
            "[0]\n[0]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"del(.a as $x | foreach (.a) as $i (0; ((1|foreach (try ($x as {a:$a} | .)) as $y (.; .; .))) // (.,.); $i))",
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":{"a":null}}}"#,
            r"[path(. as $x | foreach (.a?,.) as $i (0; ((try (foreach (try ($x as {a:{a:$a}} | 1)) as $y (.; $y; .))) // {b:1})|.; $i))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"try path(. as $x | foreach (.,.a) as $i (0; ((foreach (try ($x as [$a] | 1)) as $y (null; .; .))|.) // .; $i)) catch 5",
            "[]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"path(.a as $x | foreach (.,.a) as $i (0; (try (foreach (try ($x as [$a] | .)) as $y (0; $y; $y))) // .; $i|.))",
            "[]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"path(.a as $x | foreach (..) as $i (0; ((1|foreach (try ($x as [$a] | .)) as $y (null; $y; .))) // .; $i|.))",
            "[]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":{"a":null}}}"#,
            r"[path(.a? as $x | foreach (.,.a,.) as $i (0; (foreach (try ($x as {a:$a} | .)) as $y (null; .; $y)) // .; $i))?]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r"del(.a as $x | foreach (.a,.) as $i (.; ((foreach (try ($x as {c:$a} | .)) as $y (.; $y; .))?) // .; $i))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"del(. as $x | foreach (.a) as $i (0; (foreach (try ($x as [$a] | .)) as $y (0; $y; $y)) // (.,.); $i|.))",
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"del(. as $x | foreach (.a,.) as $i (0; ((foreach (try ($x as {a:$a} | 1)) as $y (null; .; .))?) // .; $i))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"[path(. as $x | foreach (.[]?) as $i (.; (try (foreach (try ($x as {c:$a} | 1)) as $y (0; $y; .))) // .; $i))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r"path(.a as $x | foreach (..) as $i (null; ((foreach (try ($x as {a:$a} | .)) as $y (.; $y; .))|.) // .; $i))",
            "[]\n[\"a\"]\n[\"a\",0]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"(. as $x | foreach (.a?,.) as $i (null; (foreach (try ($x as {a:{a:$a}} | .)) as $y (0; $y; $y)) // (.,.); $i|.)) = 9",
            "9\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"[path(. as $x | foreach (.a?,.) as $i (0; (foreach (try ($x as [$a] | .)) as $y (.; .; .)) // .; $i))?]",
            "[[\"a\"],[]]\n",
            "",
            0,
        ),
    ])
}

/// #5156: still wrong, pinned as it stands (a fix flips it, #5228): the explicit pipe spelling `((FE) // .) | .` refuses the first
/// step with nothing printed (jq prints `["a"]` first), and a `?` around the path of the plain spelling exits 5 where jq
/// answers `[["a"]]`: the second step's refusal is a guess nothing catches.
///
/// #5223: the path-mode interpreter carries jq's register, so the rows the shipped resolver
/// refused here now hold jq 1.7.1's answers.
#[cfg(not(feature = "unshared-containers"))]
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5156_foreach_update_alternative_residuals_pinned_as_they_stand_5228() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // .) | .; $i))",
            "[\"a\"]\n",
            r#"Invalid path expression with result {"a":null}"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // .; $i))?]",
            "[[\"a\"]]\n",
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
fn test_superseded_rows_match_jq_through_the_path_mode_interpreter_5156() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // .; $i))",
            "[\"a\"]\n",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // .; $i))",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // .; $i)) |= 9",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // $x; $i))",
            "[\"a\"]\n",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // $x; $i))",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // $x; $i)) |= 9",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // (5,6); $i))",
            "[\"a\"]\n[\"a\"]\n",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // (5,6); $i))",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // (5,6); $i)) |= 9",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r#"path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // error("z"); $i))"#,
            "",
            "z",
            5,
        ),
        (
            r#"{"a":null}"#,
            r#"del(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // error("z"); $i))"#,
            "",
            "z",
            5,
        ),
        (
            r#"{"a":null}"#,
            r#"(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // error("z"); $i)) |= 9"#,
            "",
            "z",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // .a; $i))",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // .a; $i))",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // .a; $i)) |= 9",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":null}",
            5,
        ),
    ])
}
