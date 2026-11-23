//! #7736: jq's `?//` (`DESTRUCTURE_ALT`) retries the next alternative on any error raised after
//! it, and a `break` is an error (#7719), so the `break` of `limit`, `first`, `isempty`, ... runs the
//! other alternatives of a `?//` in their generator. The shipped evaluator retries after its own
//! native stop (#1519) but not through `while` or `repeat`, so such a program runs on the path-mode
//! interpreter. Rows from jq 1.7.1 (`/usr/bin/jq`); jq 1.8.2 agrees on each.

use anyhow::Result;

use crate::run_jq_full;

/// `(args, stdin, stdout, stderr, exit)`.
type Row = (
    &'static [&'static str],
    Option<&'static str>,
    &'static str,
    &'static str,
    i32,
);

fn check(rows: &[Row]) -> Result<()> {
    for &(args, stdin, stdout, stderr, code) in rows {
        let (out, err, status) = run_jq_full(args, stdin)?;
        assert_eq!(
            (out.as_str(), err.as_str(), status),
            (stdout, stderr, code),
            "{args:?} on {stdin:?}"
        );
    }
    Ok(())
}

/// Each row is jq 1.7.1's answer.
#[test]
fn test_alternation_retries_on_a_break_7736() -> Result<()> {
    check(&[
        (
            &[
                "-nc",
                "[1] | [limit(1000; while(. as [$x] ?// $x | $x; .))] | length",
            ],
            None,
            "2000\n",
            "",
            0,
        ),
        (
            &[r"-nc", r"1 | [limit(3; while(. as [$x] ?// $x | $x; .))]"],
            None,
            r"[1,1,1]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r"[1] | [limit(3; while(. as [$x] ?// $x | $x; .))]"],
            None,
            r"[[1],[1],[1],[1],[1],[1]]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r"1 | first(while(. as [$x] ?// $x | $x; .))"],
            None,
            r"1
",
            r"",
            0,
        ),
        (
            &[r"-nc", r"[label $f | [1] as [$x] ?// $x | $x, break $f]"],
            None,
            r"[1,[1]]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r"[[1] as [$x] ?// $x | first($x, 2)]"],
            None,
            r"[1]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r"[[1] | . as [$x] ?// $x | limit(1; $x, 2)]"],
            None,
            r"[1]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r"[[1],[2]] | map(. as [$x] ?// $x | isempty($x))"],
            None,
            r"[false,false]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r"[[1] | . as [$x] ?// $x | any($x; . == 1)]"],
            None,
            r"[true]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r"[[1] | . as [$x] ?// $x | IN($x; 1)]"],
            None,
            r"[true]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r#"[{"a":1} | . as {$a} ?// [$a] | .a += 1]"#],
            None,
            r#"[{"a":2}]
"#,
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r#"[{"a":[1]} | . as {$a} ?// [$a] | map_values(.)]"#,
            ],
            None,
            r#"[{"a":[1]}]
"#,
            r"",
            0,
        ),
        (
            &[r"-nc", r"[[1] | . as [$x] ?// $x | nth(0; $x, 2)]"],
            None,
            r"[1]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r"[[[1]] | .[] as [$x] ?// $x | $x]"],
            None,
            r"[1]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r"[[1] | . as [$x] ?// $x | $x]"],
            None,
            r"[1]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r"[[1] | . as [$x] ?// $x | [limit(1; 5, 6)]]"],
            None,
            r"[[5]]
",
            r"",
            0,
        ),
        (
            &[r"-c", r"[.[] as [$x] ?// $x | first($x)]"],
            Some(
                r"[[1],2,[3]]
",
            ),
            r"[1,2,3]
",
            r"",
            0,
        ),
        (
            &[r"-c", r". as [$x] ?// $x | first($x)"],
            Some(
                r"[1]
2
[3]
",
            ),
            r"1
2
3
",
            r"",
            0,
        ),
        (
            &[r"-nc", r"[limit(4; [1] | repeat(. as [$x] ?// $x | $x))]"],
            None,
            r"[1,1,1,1,[1]]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r"[1] | [first(while(. as [$x] ?// $x | $x; .))]"],
            None,
            r"[[1],[1]]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r"[1] | [limit(2; while(. as [$x] ?// $x | $x; .))]"],
            None,
            r"[[1],[1],[1],[1]]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r"[1] | isempty(while(. as [$x] ?// $x | $x; .))"],
            None,
            r"false
false
",
            r"",
            0,
        ),
        (
            &[
                r"-nc",
                r"[1] | [any(while(. as [$x] ?// $x | $x; .); true)]",
            ],
            None,
            r"[true,true]
",
            r"",
            0,
        ),
        (
            &[r"-nc", r"[[1] | nth(1; repeat(. as [$x] ?// $x | $x))]"],
            None,
            r"[1,[1]]
",
            r"",
            0,
        ),
    ])
}

/// Recorded in docs/compliance/jq/limitations.md: a `?//` in the update of `|=` (and `walk`) stays
/// on the shipped route, which answers as before: the first two rows differ from jq 1.7.1, which
/// answers `{"a":[1]}`; the third agrees with it.
#[test]
fn test_alternation_in_an_update_is_recorded_7736() -> Result<()> {
    check(&[
        (
            &[
                r"-nc",
                r#"{"a":[1],"b":2} | .[] |= select((. as [$x] ?// $x | $x) != null)"#,
            ],
            None,
            r"",
            r"jq: error (at <unknown>): Paths must be specified as an array
",
            5,
        ),
        (
            &[r"-c", r"walk(select((. as [$x] ?// $x | $x) != null))"],
            Some(
                r#"{"a":[1],"b":2}
"#,
            ),
            r"",
            r"jq: error (at <stdin>:1): Paths must be specified as an array
",
            5,
        ),
        (
            &[r"-nc", r#"[{"a":1} | . as {$a} ?// [$a] | .a |= $a]"#],
            None,
            r#"[{"a":1}]
"#,
            r"",
            0,
        ),
    ])
}
