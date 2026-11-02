//! #5199: a `foreach` UPDATE that is a caught-bind `foreach` piped into a stage that computes a value.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

// `FE | 7` where `$x` is not the step's register: the pattern's step raises, the `try` empties the source, the `foreach` emits
// nothing and the `| 7` has nothing to run on. The tail that computes a value was read as the bind having passed, naming a
// path (`["b","a"]`) that `|=` wrote. `main` already matches jq on these rows (verified against a build of it, the commit that
// fixed it not traced); they are pinned here with the `FE | .` spelling beside them. Every row captured from `/usr/bin/jq` 1.7.1.

/// #5199: the issue's rows, over `path`, `|=` and `del`, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5199_bind_foreach_with_a_value_tail_on_a_foreign_marker() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.b) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | 7; $i))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.b) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | null; $i))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | (foreach (.b) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | 7; $i)) |= 9",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.b) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .; $i))]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | del(foreach (.b) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | 7; $i))",
            "{\"a\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r". as $x | [path(foreach (.) as $i (.; foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | 7; $i))?]",
            "[]\n",
            "",
            0,
        ),
    ])
}
