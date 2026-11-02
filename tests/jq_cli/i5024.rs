//! #5024: a collect INIT (`[.a]`, `[.[]?]`) of a `foreach` or `reduce` leaves jq's register where the fold entered.

use anyhow::Result;

use crate::{assert_path_rows_both_routes_3749, run_jq_full};

// jq runs `[E]` to exhaustion and so backtracks to the fork it began at: the register is where it entered, whatever `E`
// navigated. The resolver read an INIT that navigates as one that may have moved it, so a source element that is the root
// met an accumulator that was not the register, and a `$x` EXTRACT (a `null`/boolean on a `null` document, which
// `jv_identical` holds equal by kind) was refused as a guess no `?` catches, where jq answers every step.

/// #5024: the issue's three rows, and a spread of collect INITs (`[.a]`, `[.[]?]`, `[..]`, a collect piped into a stage
/// that cannot move the register, a comma of collects) over the sources, UPDATEs, EXTRACTs and documents of a
/// differential grid, in `path`, `[path(..)?]`, `try .. catch`, `del`, `=` and `|=`. Every row captured from
/// `/usr/bin/jq` 1.7.1 (stdout, first stderr line up to the container, exit code), both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5024_collect_init_leaves_the_register_where_the_foreach_entered() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r"null",
            r"path(. as $x | foreach (.,.a?) as $i ([.[]?]; .; $x))",
            "[]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"[path(. as $x | foreach (.,.a?) as $i ([.[]?]; .; $x))?]",
            "[[],[\"a\"]]\n",
            "",
            0,
        ),
        (
            r"null",
            r#"try ((. as $x | foreach (.,.a?) as $i ([.[]?]; .; $x)) |= 5) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"[path(. as $x | foreach (1,2) as $i ([.a?]; if . then . else $x end; $x))]",
            "[[],[]]\n",
            "",
            0,
        ),
        (
            r"{}",
            r"path(. as $x | foreach (.,.a?) as $i (([.a], .); null; $i))",
            "[]\n[\"a\"]\n",
            r"Invalid path expression with result null",
            5,
        ),
        (
            r"true",
            r"(. as $x | foreach (null) as $i ([..]; .; $x)) |= 5",
            "5\n",
            "",
            0,
        ),
        (
            r"null",
            r"[limit(2; path(. as $x | foreach (.a?,.) as $i ([.[]?]; try .a; 7)))]",
            "",
            r"Invalid path expression with result 7",
            5,
        ),
        (
            r"false",
            r"first(path(. as $x | foreach (.) as $i ([recurse]; $i; $x)))",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r#"try path(. as $x | foreach (..) as $i ([.a]|length; 5; $x.a?)) catch "C""#,
            "[\"a\"]\n\"C\"\n",
            "",
            0,
        ),
        (
            r"{}",
            r"[del(. as $x | foreach (.,.a?) as $i ([.a]|.; 5; $i))?]",
            "[null]\n",
            "",
            0,
        ),
        (
            r"null",
            r"[path(. as $x | foreach (.a?,.) as $i (([.a]|length); try .a; (.a == null)))]",
            "",
            r"Invalid path expression with result true",
            5,
        ),
        (
            r"[]",
            r"[limit(2; path(. as $x | foreach (.,.a?,.) as $i ([.[]?]; .; $x.a?)))]",
            "[]\n",
            "",
            0,
        ),
        (
            r"[null]",
            r"path(. as $x | foreach (.,.a?) as $i ([recurse]; $x; .))",
            "[]\n",
            "",
            0,
        ),
        (
            r"[1]",
            r"[del(. as $x | foreach (first(.,.a?)) as $i ([.a?]; null; $x))?]",
            "[null]\n",
            "",
            0,
        ),
        (
            r"{}",
            r#"try path(. as $x | foreach (1,.) as $i ([.a]; $x; .a?)) catch "C""#,
            "[\"a\"]\n[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"(. as $x | foreach (first(.,.a?)) as $i ([.a,.b]; if . then . else $x end; $i)) |= 5",
            "5\n",
            "",
            0,
        ),
        (
            r"[]",
            r"first(path(. as $x | foreach (.,.a?,.) as $i ([..]; $x; .a?)))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r"path(. as $x | foreach (..) as $i ([.a]|length; $x; $x.a?))",
            "[\"a\"]\n",
            r#"Invalid path expression near attempt to access element "a""#,
            5,
        ),
        (
            r"[null]",
            r"[path(. as $x | foreach (.,.,.a?) as $i ([recurse]; null; $i))]",
            "[[],[]]\n",
            "",
            0,
        ),
        (
            r"null",
            r#"try path(. as $x | foreach (.,.a?) as $i (([.a], .); $i; .a)) catch "C""#,
            "[\"a\"]\n[\"a\",\"a\"]\n[\"a\"]\n[\"a\",\"a\"]\n",
            "",
            0,
        ),
        (
            r"false",
            r"(. as $x | foreach ($x) as $i ([..]; 5; not)) |= 5",
            "5\n",
            "",
            0,
        ),
        (
            r"true",
            r"[limit(2; path(. as $x | foreach (.a?,.) as $i ([.[]?]; $x; $i)))]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r"[1]",
            r"[del(. as $x | foreach (1,2) as $i ([.a?]; try .a; $x.a?))?]",
            "[[1]]\n",
            "",
            0,
        ),
        (
            r"{}",
            r"path(. as $x | foreach (.,.a?) as $i ([.a]; $x; $x))",
            "[]\n",
            r"Invalid path expression with result {}",
            5,
        ),
        (
            r"null",
            r"first(path(. as $x | foreach (.,.a?) as $i (([.a]|length); $x; null)))",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r"(. as $x | foreach ($x) as $i ([.a]|.; $x; .a?)) |= 5",
            "{\"a\":5}\n",
            "",
            0,
        ),
        (
            r"{}",
            r#"try path(. as $x | foreach (.) as $i ([.a,.b]; . as $q | .; $x)) catch "C""#,
            "[]\n",
            "",
            0,
        ),
        (
            r"[1]",
            r"[path(. as $x | foreach (.,.a?,.) as $i ([..]; $x; .))?]",
            "[[],[]]\n",
            "",
            0,
        ),
        (
            r"false",
            r"[limit(2; path(. as $x | foreach (.) as $i ([.a?]; $x; $x)))]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r"[null]",
            r"first(path(. as $x | foreach (.,.,.a?) as $i ([.[]?]; if . then . else $x end; $i)))",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":false,"b":null}"#,
            r"path(. as $x | foreach (.,.a?) as $i ([.a]; $x; .a))",
            "[\"a\"]\n",
            r#"Invalid path expression near attempt to access element "a""#,
            5,
        ),
        (
            r"null",
            r"[del(. as $x | foreach (1,2) as $i ([.a]|length; $x; null))?]",
            "[null]\n",
            "",
            0,
        ),
        (
            r"true",
            r"[path(. as $x | foreach (..) as $i ([recurse]; if . then . else $x end; $x.a?))]",
            "[]\n",
            "",
            0,
        ),
        (
            r"[]",
            r"(. as $x | foreach (..) as $i ([.[]?]; null; $x)) = 5",
            "5\n",
            "",
            0,
        ),
        (
            r"{}",
            r#"try path(. as $x | foreach (.,.a?,.) as $i (([.a]|length); $x; .)) catch "C""#,
            "[]\n\"C\"\n",
            "",
            0,
        ),
        (
            r"null",
            r"[limit(2; path(. as $x | foreach ($x) as $i (([.a], .); null; null)))]",
            "[[],[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r"[del(. as $x | foreach (first(.,.a?)) as $i ([.a]|.; $x; .a?))?]",
            "[{}]\n",
            "",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"first(path(. as $x | foreach ($x) as $i ([.a,.b]; $x; .a)))",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r"null",
            r"[path(. as $x | foreach (.a?,.) as $i ([..]; try .a; 7))]",
            "",
            r"Invalid path expression with result 7",
            5,
        ),
        (
            r#"{"a":{"a":null}}"#,
            r"[del(. as $x | reduce (.) as $i ([.[]?]; $i))?]",
            "[null]\n",
            "",
            0,
        ),
        (
            r"{}",
            r"path(. as $x | reduce ((.a,.)) as $i ([.a?]; $x))",
            "[]\n",
            "",
            0,
        ),
        (
            r"[1]",
            r"(. as $x | reduce (.,.a?,.) as $i ([..]; $i)) = 5",
            "5\n",
            "",
            0,
        ),
        (
            r"null",
            r#"try del(. as $x | reduce (.,.a?,.) as $i ([.a]|.; $x | .a?)) catch "C""#,
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r"path(. as $x | reduce (.a?) as $i (([.a],.); $x))",
            "[]\n",
            r#"Invalid path expression near attempt to access element "a""#,
            5,
        ),
        (
            r"[]",
            r#"try path(. as $x | reduce (1,.) as $i ([.a?]; $x)) catch "C""#,
            "[]\n",
            "",
            0,
        ),
        (
            r"{}",
            r"[path(. as $x | reduce (.,.a?,.) as $i ([.a,.b]; $i))?]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":[1,2]}"#,
            r"first(path(. as $x | reduce ($x) as $i ([.a]|length; $x)))",
            "[]\n",
            "",
            0,
        ),
        (
            r"[null]",
            r"[path(. as $x | reduce ($x) as $i ([..]; $x))?]",
            "[[]]\n",
            "",
            0,
        ),
        (
            r"null",
            r"(. as $x | reduce (first(.,.a?)) as $i ([.a]; $x | .a?)) = 5",
            "5\n",
            "",
            0,
        ),
        (
            r"[1]",
            r#"try del(. as $x | reduce (.[]?) as $i ([.[]?]; $x)) catch "C""#,
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":[null]}"#,
            r"path(. as $x | reduce (.[]?) as $i (([.a],.); $x))",
            "[]\n",
            r"Invalid path expression near attempt to iterate through null",
            5,
        ),
        (
            r"[]",
            r"first(path(. as $x | reduce (1,2) as $i ([..]; $x)))",
            "[]\n",
            "",
            0,
        ),
        (
            r"{}",
            r"[del(. as $x | reduce (1,.) as $i ([.a]; $x))?]",
            "[null]\n",
            "",
            0,
        ),
    ])
}

/// #5024: a collect piped into a generator (`paths`, `tostream`) or a variable stage forks the INIT or reaches a node the
/// register may be on, so it keeps the reading it had: each row answers as jq does or refuses. Each row is `(input,
/// filter, jq 1.7.1's stdout, jq's exit code)`.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5024_collect_piped_into_a_generator_or_variable_never_answers_where_jq_differs(
) -> Result<()> {
    let rows: &[(&str, &str, &str, i32)] = &[
        (
            r#"{"a":[1]}"#,
            r"[del(. as $x | foreach (.,.a?) as $i ([.a] | paths; $i; .a?))?]",
            "[]\n",
            0,
        ),
        (
            r#"{"a":[1]}"#,
            r"[del(. as $x | foreach (.,.a?) as $i ([.a] | paths; $x; $i))?]",
            "[]\n",
            0,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"(. as $x | foreach (.) as $i ([.a] | tostream; $i; .a?)) |= 5",
            "",
            5,
        ),
        (
            r"null",
            r"(. as $x | foreach (.a?,.) as $i ([.a] | tostream; $x; $i)) |= 5",
            "",
            5,
        ),
        (
            r"null",
            r"[path(. as $x | foreach (.,.a?) as $i ([.a] | $x | .a?; .; $x))?]",
            "[[\"a\"],[\"a\",\"a\"]]\n",
            0,
        ),
    ];
    for (input, filter, jq_stdout, jq_code) in rows {
        let (stdout, _stderr, code) = run_jq_full(&["-c", filter], Some(input))?;
        let same_as_jq = stdout == *jq_stdout && code == *jq_code;
        // jq answers and the binary refuses (exit 5, nothing printed), or jq refuses and the binary refuses too.
        let refuses = code == 5 && (*jq_code != 0 || stdout.is_empty());
        assert!(
            same_as_jq || refuses,
            "{filter} on {input}: jq {jq_stdout:?} (exit {jq_code}), binary {stdout:?} (exit {code})"
        );
    }
    Ok(())
}
