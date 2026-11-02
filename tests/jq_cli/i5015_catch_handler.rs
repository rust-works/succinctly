//! #5015: a bare `foreach` UPDATE over a caught bind of `$x` with a `catch .` handler, under a `null` or boolean EXTRACT.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

// `. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .) catch .) as $y (.; .; .); null)` on `{"a":null}`:
// a source with a handler emits whether the bind passed or not (the error message stands in as its element, jq's
// register back at the `try`'s entry), so the emission says nothing of where the register is. On the step whose register
// is `$x` (the element `.`) the bind passes and moves it onto the bound child, as for the handler-less form (#4957); on
// the others (`.a`, where `$x` is no node of the register) the handler ran and the register is the step's own. Both
// steps put `["a"]` before the `null` EXTRACT, so jq prints it twice; the binary refused both and `[path(..)?]` was `[]`.
// Decided per step by whether the marker is the step's register, and only under a `null` or boolean EXTRACT, which is what
// the register's standing decides: any other EXTRACT keeps the reading it had.

/// #5015: the issue's row 1 (`[path(..)?]`, `try ... catch "C"`, `|= 9`, `del`, a boolean EXTRACT), controls that keep
/// the reading they had, and rows of a differential grid (document x bind site x handler bind x wrapper x source x INIT x
/// EXTRACT x context) that `main` refused or answered wrongly. Every row captured from `/usr/bin/jq` 1.7.1 (stdout, exit
/// code, message), both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5015_catch_handler_bind_source_is_decided_per_step() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .) catch .) as $y (.; .; .); null))?]",
            "[[\"a\"],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .) catch .) as $y (.; .; .); null)) catch "C""#,
            "[\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .) catch .) as $y (.; .; .); null)) |= 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .) catch .) as $y (.; .; .); null))",
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | foreach (.a,.) as $i (.; foreach (try ($x as {a:$a} | .) catch .) as $y (.; .; .); true))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":0,"b":false}"#,
            r"path(.a as $x | foreach 1 as $i (.; foreach (try ($x as {a:$a} | .) catch .) as $y (.; .; .); .))",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1},"b":2}"#,
            r"path(.a as $x | foreach . as $i (.; foreach (try ($x as {a:$a} | .) catch .) as $y (.; .; .); .a?))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r#"[path(. as $x | foreach . as $i (.; foreach (try ($x as {a:$a} | .) catch .) as $y (.; .; .); getpath(["a"])?))?]"#,
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r#"(. as $x | foreach . as $i (.; foreach (try ($x as {a:$a} | .) catch .) as $y (.; .; .); getpath(["a"])?)) |= 9"#,
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r"null",
            r"[path(.a.a as $x | foreach (.,.) as $i ({}; foreach (try ($x as {a:$a} | .) catch .) as $y (.; .; .); null,1))?]",
            "[[\"a\"]]\n",
            "",
            0,
        ),
        (
            r"{}",
            r"del(. as $x | foreach .. as $i (.; foreach (try ($x as {a:$a} | .) catch .) as $y (.; .); null))",
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r#"try path(. as $x | foreach (1,2) as $i (.; foreach (try ($x as {b:$a} | .) catch .) as $y (.; .); null,1)) catch "c""#,
            "[\"b\"]\n\"c\"\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[path(. as $x | foreach (1,2) as $i (.; foreach (try ($x as {a:$a} | .) catch .) as $y (.; .; .); true))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r"null",
            r#"try path(. as $x | foreach 1 as $i (0; foreach (try ($x as {a:$a} | .) catch .) as $y (.; .); null)) catch "c""#,
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(.b as $x | foreach (.,.) as $i (0; foreach (try ($x as {b:$a} | .) catch .) as $y (.; .; .); null))",
            "[\"b\"]\n[\"b\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(.a as $x | foreach (.,.a) as $i ({}; foreach (try ($x as [$a] | .) catch .) as $y (.; .; .); null))",
            "[0]\n[\"a\",0]\n",
            "",
            0,
        ),
        (
            r"{}",
            r"path(. as $x | foreach 1 as $i (0; foreach (try ($x as {a:$a} | .) catch .) as $y (.; .; .); null,1))",
            "[\"a\"]\n",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r"null",
            r"[path(.a as $x | foreach (.,.a) as $i ({}; foreach (try ($x as [$a] | .) catch .) as $y (.; .); null))?]",
            "[[0],[\"a\",0]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(.a as $x | foreach .a as $i ({}; foreach (try ($x as {a:$a} | .) catch .) as $y (.; .); null))]",
            "[[\"a\",\"a\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"[path(.b as $x | foreach .[]? as $i ({}; foreach (try ($x as {b:$a} | .) catch .) as $y (.; .); null))?]",
            "[[\"a\",\"b\"]]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(.b as $x | foreach .a as $i (0; foreach (try ($x as [$a] | .) catch .) as $y (.; .); null)) |= 9",
            "{\"a\":[9]}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(.b as $x | foreach .[]? as $i (0; foreach (try ($x as [$a] | .) catch .) as $y (.; .; .); null,1))",
            "[\"a\",0]\n",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(.b as $x | foreach (.a,.) as $i (0; foreach (try ($x as {a:$a} | .) catch .) as $y (.; .; .); null,1)) catch "c""#,
            "[\"a\",\"a\"]\n\"c\"\n",
            "",
            0,
        ),
        (
            r"null",
            r"[path(.b as $x | foreach (.,.a) as $i (0; foreach (try ($x as {b:$a} | .) catch .) as $y (.; .; .); null))?]",
            "[[\"b\"],[\"a\",\"b\"]]\n",
            "",
            0,
        ),
        (
            r"{}",
            r#"try path(. as $x | foreach (1,2) as $i ({}; foreach (try ($x as {a:$a} | .) catch .) as $y (.; .; .); null)) catch "c""#,
            "[\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (1,2) as $i (null; foreach (try ($x as {a:$a} | .) catch .) as $y (.; .); null)) |= 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r"del(. as $x | foreach 1 as $i (.; foreach (try ($x as {b:$a} | .) catch .) as $y (.; .; .); null))",
            "{\"a\":[null]}\n",
            "",
            0,
        ),
        (
            r"{}",
            r#"try path(.a as $x | foreach .a as $i (0; foreach (try ($x as {b:$a} | .) catch .) as $y (.; .); null)) catch "c""#,
            "[\"a\",\"b\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"[path(. as $x | foreach (.,.a) as $i (0; foreach (try ($x as [$a] | .) catch .) as $y (.; .; .); null))]",
            "[[0],[\"a\",0]]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"(. as $x | foreach (.,.) as $i (0; foreach (try ($x as {b:$a} | .) catch .) as $y (.; .; .); null)) |= 9",
            "{\"a\":{\"a\":null},\"b\":9}\n",
            "",
            0,
        ),
        (
            r"{}",
            r"path(.a.a as $x | foreach (.a,.) as $i ({}; foreach (try ($x as {b:$a} | .) catch .) as $y (.; .); null,1))",
            "[\"a\",\"b\"]\n",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r"null",
            r"path(.a.a as $x | foreach . as $i ({}; foreach (try ($x as [$a] | .) catch .) as $y (.; .); null,1))",
            "[0]\n",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r"null",
            r"path(.b as $x | foreach .a as $i (0; foreach (try ($x as {a:$a} | .) catch .) as $y (.; .; .); null))",
            "[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(.b as $x | foreach 1 as $i (0; foreach (try ($x as {a:$a} | .) catch .) as $y (.; .); null,1))",
            "[\"a\"]\n",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":null}"#,
            r#"try path(.a.a as $x | foreach (.a,.) as $i ({}; foreach (try ($x as [$a] | .) catch .) as $y (.; .); null,1)) catch "c""#,
            "[\"a\",0]\n\"c\"\n",
            "",
            0,
        ),
    ])
}
