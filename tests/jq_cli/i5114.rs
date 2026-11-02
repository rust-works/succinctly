//! #5114: a caught foreign-`$x` `foreach` behind a `//` that is a `foreach` UPDATE, with a `$i` EXTRACT.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

// `foreach (.a,.) as $i (.; (FE) // 5; $i)` with `FE = foreach (try ($x as {a:$a} | .)) as $y (.; .; .)` over a moving
// source: jq emits the first step's path before the second step raises, and the bare UPDATE refused the first step
// (`with result <the source element>`) with nothing on stdout. The same `//` behind a no-op `| .` already matched,
// so a `//` with a literal alternate that is a `foreach`'s whole UPDATE is read as that pipe. A non-literal alternate
// (`// .`, `// $x`) is read since #5156 (`tests/jq_cli/i5156.rs`), though not through the explicit pipe.

/// #5114: the issue's rows and rows of a differential grid (document x source x caught-bind shape x `//` wrapper x
/// EXTRACT x `path`/`|=`/`del`) that were refused or answered wrongly before the fix. Every row captured from
/// `/usr/bin/jq` 1.7.1 (stdout, exit code, message), both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5114_foreach_update_alternative_of_a_caught_foreign_foreach() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // 5; $i))",
            "[\"a\"]\n",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // 5; $i)) |= 9",
            "",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":1}"#,
            r"path(. as $x | foreach (.a) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // 5; $i))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | foreach (.a,.a) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; $y)) // null; .))",
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"a":[null,1]}"#,
            r"(. as $x | foreach (.a,.,.a) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .))? // 5; $i)) |= 9",
            "",
            "Invalid path expression with result {\"a\":[null,1]}",
            5,
        ),
        (
            r#"{"a":[null,1]}"#,
            r"del(. as $x | foreach (.a,.,.a) as $i (.; ((foreach (try ($x as {a:$a} | .)) as $y (.; .; $y)) // 5) // 6; $i | .))",
            "",
            "Invalid path expression with result {\"a\":[null,1]}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.,.a) as $i (.; ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // 5) // 6; $i | .))",
            "[\"a\"]\n",
            "Invalid path expression with result {\"a\":null}",
            5,
        ),
        (
            r#"{"a":false}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // false; $i))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"del(. as $x | foreach .a as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; $y))? // 5; $i | .))",
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach .a as $i (.; (try (($x as {a:$a} | .))) // 5; $i | .))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":[null,1]}"#,
            r"(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // 5; $i)) |= 9",
            "",
            "Invalid path expression with result {\"a\":[null,1]}",
            5,
        ),
        (
            r#"{"a":true}"#,
            r"(. as $x | foreach (.a,.a) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // null; $i)) |= 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":false}"#,
            r"del(. as $x | foreach (.a,.,.a) as $i (.; (try (foreach (try ($x as {a:$a} | .)) as $y (.; .; $y))) // 5; $i))",
            "",
            "Invalid path expression with result {\"a\":false}",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"(. as $x | foreach (.a,.,.a) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // false; $i | .)) |= 9",
            "",
            "Cannot index number with string \"a\"",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.a) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; $y)) // null; $i))",
            "[\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"path(. as $x | foreach (.a,.,.a) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .))? // 5; $i))",
            "[\"a\"]\n",
            "Invalid path expression with result {\"a\":true}",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"(. as $x | foreach (.a,.a) as $i (.; ((foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // 5) // 6; $i)) |= 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"del(. as $x | foreach (.a,.) as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // 5; $i | .))",
            "",
            "Invalid path expression with result {\"a\":1}",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach .a as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; $y)) // 5; $i))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"(. as $x | foreach .a as $i (.; (try (($x as {a:$a} | .))) // 5; $i)) |= 9",
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":[null,1]}"#,
            r"path(. as $x | foreach .a as $i (.; (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) // false; $i))",
            "[\"a\"]\n",
            "",
            0,
        ),
    ])
}
