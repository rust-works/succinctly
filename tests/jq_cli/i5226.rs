//! #5226: a `//` right operand whose bind source is the loop variable (`$i`, `$i.a`) in a fold UPDATE is refused as jq refuses it.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

/// #5223 Phase 3: this issue's repro rows, with `path`, `del` and `|=`
/// wherever the row is a `path(...)`, as jq 1.7.1 answers them (stdout, the
/// first error message, the exit code). The path-mode interpreter carries
/// jq's register, so they match by construction, on both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_superseded_rows_match_jq_through_the_path_mode_interpreter_5226() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | foreach (1,2) as $i (null; try (($x as {a:$a} | 5) // ($i as $q | $q))))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(. as $x | foreach (1,2) as $i (null; try (($x as {a:$a} | 5) // ($i as $q | $q))))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"(. as $x | foreach (1,2) as $i (null; try (($x as {a:$a} | 5) // ($i as $q | $q)))) |= 9",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | foreach (1,2) as $i (null; try (($x as {a:$a} | 5) // ($i as $q | $q)); .))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(. as $x | foreach (1,2) as $i (null; try (($x as {a:$a} | 5) // ($i as $q | $q)); .))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"(. as $x | foreach (1,2) as $i (null; try (($x as {a:$a} | 5) // ($i as $q | $q)); .)) |= 9",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | reduce (1,2) as $i (null; try (($x as {a:$a} | 5) // ($i as $q | $q))))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(. as $x | reduce (1,2) as $i (null; try (($x as {a:$a} | 5) // ($i as $q | $q))))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"(. as $x | reduce (1,2) as $i (null; try (($x as {a:$a} | 5) // ($i as $q | $q)))) |= 9",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | foreach (1,2) as $i (null; try (($x as {a:$a} | 5) // ($i.a as $q | $q))))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(. as $x | foreach (1,2) as $i (null; try (($x as {a:$a} | 5) // ($i.a as $q | $q))))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"(. as $x | foreach (1,2) as $i (null; try (($x as {a:$a} | 5) // ($i.a as $q | $q)))) |= 9",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | foreach (1,2) as $i (null; try (($x as {a:$a} | 5) // ($i.a as $q | $q)); .))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(. as $x | foreach (1,2) as $i (null; try (($x as {a:$a} | 5) // ($i.a as $q | $q)); .))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"(. as $x | foreach (1,2) as $i (null; try (($x as {a:$a} | 5) // ($i.a as $q | $q)); .)) |= 9",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | reduce (1,2) as $i (null; try (($x as {a:$a} | 5) // ($i.a as $q | $q))))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(. as $x | reduce (1,2) as $i (null; try (($x as {a:$a} | 5) // ($i.a as $q | $q))))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"(. as $x | reduce (1,2) as $i (null; try (($x as {a:$a} | 5) // ($i.a as $q | $q)))) |= 9",
            "",
            "Invalid path expression with result 5",
            5,
        ),
    ])
}

/// The `$i` and `$i.a` bind sources over varied documents, folds, INITs, sources and EXTRACTs: rows sampled from
/// `scripts/jq-fold-update-alt-bind-sweep.py`'s grid, one per outcome class (the context, the fold shape, jq's exit code,
/// whether it printed, its message), so the loop variable is reached where it answers, where it is the register's node and
/// where jq raises, as jq 1.7.1 answers each (stdout, first error message, exit code).
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_loop_variable_bind_source_rows_sampled_from_the_sweep_grid_match_jq_5226() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1}}"#,
            r"(. as $x | foreach . as $i (.; try (($x as $a | .a) // ($i as $q | $q)); .)) |= 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"path(. as $x | foreach . as $i (.a; try ((. as {a:$a} | .) // ($i.a as $q | $q)); .))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"(. as $x | foreach (.a,.a) as $i (null; try ((. as {a:$a} | $a) // ($i.a as $q | $q)))) |= 9",
            "{\"a\":{\"b\":1}}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | reduce (1,2) as $i (5; try (($x as {a:$a} | $x) // ($i as $q | $q))))",
            "[]\n",
            "",
            0,
        ),
        (
            r"true",
            r"path(. as $x | foreach . as $i (false; try (($x as [$a] | 5) // ($i.a as $q | $q))))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"del(. as $x | foreach (.a,.a) as $i (5; try (($x as {a:{b:$a}} | .) // ($i as $q | $q)); $i))",
            "{\"a\":1}\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"(. as $x | foreach (.a,.a) as $i (false; try (($x as {a:{b:$a}} | .a) // ($i.a as $q | $q)); $x)) |= 9",
            "{\"a\":1}\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"path(. as $x | foreach (.a,.) as $i (null; try (($x as {a:$a} ?// $a | .) // ($i as $q | $q)); .))",
            "[\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"del(. as $x | foreach (.,.a) as $i (.a; try ((. as {a:$a} | .a) // ($i.a as $q | $q)); .))",
            "null\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach .a as $i (5; try (($x as {a:$a} ?// $a | 5) // ($i.a as $q | $q)); $x))",
            "[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (5; try (($x as [$a] | .) // ($i as $q | $q))))",
            "",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (null; try (($x as {a:{b:$a}} | (.,.)) // ($i.a as $q | $q))))",
            "[\"a\"]\n[]\n",
            "",
            0,
        ),
        (
            r"true",
            r"(. as $x | foreach . as $i (null; try (($x as [$a] | $a) // ($i as $q | $q)))) |= 9",
            "true\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(. as $x | foreach (.a,.a) as $i (null; try (($x as {a:$a} ?// $a | .a) // ($i as $q | $q))))",
            "{\"a\":{\"b\":1}}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(. as $x | foreach (.a,.a) as $i (null; try (($x as {a:$a} ?// $a | (.,.)) // ($i.a as $q | $q)); .a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of null",
            5,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.,.a) as $i (.; try (($x as {a:{b:$a}} | 5) // ($i as $q | $q))))",
            "",
            "Invalid path expression with result 5",
            5,
        ),
        (
            r"null",
            r"(. as $x | foreach (1,2) as $i (.a; try (($x as {a:$a} | .) // ($i as $q | $q)); .a)) |= 9",
            "",
            "Invalid path expression near attempt to access element \"a\" of 1",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | foreach (.a,.a) as $i (null; try (($x as $a | $a) // ($i.a as $q | $q)); $x))",
            "",
            "Invalid path expression with result {\"a\":{\"b\":1}}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | reduce (.,.a) as $i (.a; try (($x as {a:$a} | 5) // ($i as $q | $q))))",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":null}",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | foreach (1,2) as $i (null; try (($x as {a:$a} ?// $a | (.,.)) // ($i as $q | $q)); .))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r"false",
            r"(. as $x | reduce . as $i (false; try (($x as [$a] | .) // ($i as $q | $q)))) |= 9",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r"null",
            r"(. as $x | foreach (1,2) as $i (.a; try (($x as {a:$a} ?// $a | $x) // ($i as $q | $q)))) |= 9",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"(. as $x | foreach . as $i (.; try (($x as {a:{b:$a}} | (.,.)) // ($i.a as $q | $q)); .a)) |= 9",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":{\"b\":1}}",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | reduce (.a,.a) as $i (5; try ((. as {a:$a} | $a) // ($i.a as $q | $q))))",
            "",
            "Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach . as $i (.a; try (($x as {a:$a} ?// $a | (.,.)) // ($i as $q | $q)); .))",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":1}"#,
            r"del(. as $x | reduce (.a,.) as $i (null; try (($x as {a:$a} | .) // ($i.a as $q | $q))))",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.) as $i (.a; try (($x as {a:{b:$a}} | (.,.)) // ($i.a as $q | $q))))",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":null}",
            5,
        ),
        (
            r"null",
            r"(. as $x | reduce (.a,.) as $i (false; try (($x as {a:$a} ?// $a | range(2)) // ($i.a as $q | $q)))) |= 9",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r"false",
            r"path(. as $x | foreach (.,.a) as $i (5; try (($x as {a:$a} ?// $a | $x) // ($i.a as $q | $q))))",
            "",
            "Cannot index boolean with string \"a\"",
            5,
        ),
        (
            r"false",
            r"path(. as $x | foreach (.a,.) as $i (.a; try (($x as {a:$a} ?// $a | .) // ($i.a as $q | $q)); .))",
            "",
            "Cannot index boolean with string \"a\"",
            5,
        ),
        (
            r"[1]",
            r"(. as $x | foreach .a as $i (.a; try (($x as {a:{b:$a}} | $x) // ($i as $q | $q)))) |= 9",
            "",
            "Cannot index array with string \"a\"",
            5,
        ),
        (
            r"[1]",
            r"del(. as $x | foreach . as $i (.a; try (($x as {a:{b:$a}} | $a) // ($i as $q | $q)); .a))",
            "",
            "Cannot index array with string \"a\"",
            5,
        ),
        (
            r"[1]",
            r"path(. as $x | foreach (.a,.a) as $i (.a; try (($x as {a:$a} ?// $a | 5) // ($i as $q | $q)); .a))",
            "",
            "Cannot index array with string \"a\"",
            5,
        ),
        (
            r"false",
            r"del(. as $x | foreach .a as $i (5; try (($x as [$a] | $x) // ($i as $q | $q)); .))",
            "",
            "Cannot index boolean with string \"a\"",
            5,
        ),
    ])
}
