//! #5170: a comma beside a bind of an outer `$x` behind a `//` in a three-argument `foreach` UPDATE.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

// `(B, 7) // 5`: `//` keeps the truthy outputs of the comma, so the bind's `.` is the UPDATE's unchecked output as it is
// alone, and a sibling that cannot move the register runs from the register the comma was entered with, so the chain and
// the sibling are one sequence of path-checked steps. #5157 read `(chain) // constant`, whose chain was a lone bind or a
// comma of binds and the accumulator; a literal sibling made the comma fan out, the register was not carried in, and the
// step after the first refused the bind of a `null` `$x`.

/// #5170: the issue's row (a literal sibling after the bind), the sibling ahead of it, a `null` sibling, a comma inside
/// the `//`'s left operand, and the writes through it. Every row captured from `/usr/bin/jq` 1.7.1, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5170_three_argument_foreach_alternative_over_a_comma_beside_a_bind() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; (($x as {a:$a} | .), 7) // 5; $i))",
            "[\"a\"]\n[\"a\"]\n[]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; (7, ($x as {a:$a} | .)) // 5; $i))",
            "[\"a\"]\n[]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; (($x as {a:$a} | .), null) // 5; $i))",
            "[\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; (($x as {a:$a} | .), (6, 7)) // 5; $i))",
            "[\"a\"]\n[\"a\"]\n[\"a\"]\n[]\n[]\n",
            "",
            0,
        ),
        (
            r"null",
            r"(. as $x | foreach (.a,.) as $i (.; (($x as {a:$a} | .), 7) // 5; $i)) |= 9",
            "9\n",
            "",
            0,
        ),
        (
            r"null",
            r"del(. as $x | foreach (.a,.) as $i (.; (($x as {a:$a} | .), 7) // 5; $i))",
            "null\n",
            "",
            0,
        ),
        // The same sibling in a comma with no `//` round it.
        (
            r"null",
            r"path(. as $x | foreach (.a,.) as $i (.; (($x as {a:$a} | .), 7); $i))",
            "[\"a\",\"a\"]\n[\"a\"]\n[\"a\"]\n[]\n",
            "",
            0,
        ),
    ])
}

/// #5170: the rows that stay as they were. A document whose `$x` is not the register raises on the bind, and a sibling
/// that navigates moves the register, so neither is read.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5170_rows_that_keep_their_reading() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; (($x as {a:$a} | .), 7) // 5; $i))",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":null}",
            5,
        ),
        (
            r#"{"a":1}"#,
            r"path(. as $x | foreach (.a,.) as $i (.; (($x as {a:$a} | 5), 7) // 6; $i))",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":1}",
            5,
        ),
    ])
}

/// #5170: a bind whose body emits more than once (`(.,.)`, `range(2)`) beside a literal sibling was refused, not read: the
/// resolver delivered a generator body's outputs fewer times than jq when the state is not the register (#5193). The
/// path-mode interpreter (#5223) answers each with jq's paths. Rows captured from jq 1.7.1, both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5170_a_fanning_out_bind_body_beside_a_sibling_answers_as_jq_does() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            "null",
            r"path(. as $x | foreach (.a,.a) as $i (.a; (($x as {a:$a} | (.,.)), 7) // 5; $i))",
            "[\"a\",\"a\"]\n[\"a\",\"a\",\"a\"]\n[\"a\",\"a\",\"a\"]\n[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            "null",
            r"path(. as $x | foreach (.a,.) as $i (.; (($x as {a:$a} | range(2)), 7) // 5; $i))",
            "[\"a\",\"a\"]\n[\"a\",\"a\"]\n[\"a\"]\n[\"a\"]\n[\"a\"]\n[]\n",
            "",
            0,
        ),
        (
            "null",
            r"path(. as $x | foreach (.a,.) as $i (.; (($x as {a:$a} | (range(2) | .)), 7) // 5; $i))",
            "[\"a\",\"a\"]\n[\"a\",\"a\"]\n[\"a\"]\n[\"a\"]\n[\"a\"]\n[]\n",
            "",
            0,
        ),
        (
            "null",
            r"(. as $x | foreach (.a,.) as $i (.; (($x as {a:$a} | range(2)), 7) // 5; $i)) |= 9",
            "9\n",
            "",
            0,
        ),
    ])?;
    Ok(())
}
