//! #7617: an unparenthesised comma at the top of a yq `ireduce`'s INIT is a union, not the initial
//! value: yq's `,` and `;` share one precedence and associate to the right, so `(0, 1; U)` is
//! `0, (1; U)`, and `reduceOperator` fails with `reduce must be given a block, got UNION instead`
//! because its argument is not a block. The check runs when the operator is evaluated, before SRC,
//! INIT or UPDATE. Every row is `yq -o=json -I0 FILTER` on `[1,2]`, captured from yq v4.53.3.

use anyhow::Result;

use crate::{run_jq_stdin, run_yq_stdin, run_yq_stdin_with_stderr};

const INPUT: &str = "[1,2]\n";
const UNION_ERROR: &str = "Error: reduce must be given a block, got UNION instead";

/// Filters yq v4.53.3 fails on with [`UNION_ERROR`], exit code 1 and no output.
const UNION_ROWS: &[&str] = &[
    ".[] as $x ireduce (0, 1; . + $x)",
    "[.[] as $x ireduce (0, 1; . + $x)]",
    ". as $x ireduce (0, 1, 2; . + 1)",
    ".[] as $x ireduce (0 , 1 , 2; . + 1)",
    ".[] as $x ireduce ((0), 1; . + $x)",
    ".[] as $x ireduce (0, (1); . + $x)",
    ".[] as $x ireduce ((0, 1), 2; . + $x)",
    ".[] as $x ireduce (.[0], .[1]; . + $x)",
    ".[] as $x ireduce (\"a\" + \"b\", 1; .)",
    // A pipe, an alternative, a binding and `and` all bind tighter than the comma.
    ".[] as $x ireduce (0 | 1, 2; . + $x)",
    ".[] as $x ireduce (0, 1 | 2; . + $x)",
    ".[] as $x ireduce (0 // 1, 2; . + $x)",
    ".[] as $x ireduce (0 as $z | 1, 2; . + $x)",
    ".[] as $x ireduce (0, 1 and 2; . + $x)",
    // Whatever follows the union: no UPDATE, a second `;`, an UPDATE that reads nothing defined.
    ".[] as $x ireduce (0, 1)",
    ".[] as $x ireduce (0, 1; .; .)",
    ".[] as $x ireduce (0, 1; $nope)",
    // An empty source and an empty context still reach the operator.
    "[] as $x ireduce (0, 1; .)",
    ".[] | select(false) | (. as $x ireduce (0, 1; .))",
    // The error ends the run before anything prints, wherever the fold sits.
    "1, (.[] as $x ireduce (0, 1; . + $x))",
    ".[] as $x ireduce (0, 1; . + $x) | . + 10",
    "(.[] as $x ireduce (0, 1; . + $x)) // 5",
    "(.[] as $x ireduce (0, 1; . + $x)) as $y | 5",
    ".[0] = (.[] as $x ireduce (0, 1; . + $x))",
    ".[] as $x ireduce (.[] as $y ireduce (0, 1; .); .)",
    ".[] as $x ireduce (0; .[] as $y ireduce (0, 1; .))",
];

#[test]
fn a_union_at_the_top_of_init_raises_7617() -> Result<()> {
    for filter in UNION_ROWS {
        let (stdout, stderr, code) =
            run_yq_stdin_with_stderr(filter, INPUT, &["-o", "json", "-I0"])?;
        assert_eq!(
            (stdout.as_str(), stderr.trim_end(), code),
            ("", UNION_ERROR, 1),
            "{filter}"
        );
    }
    Ok(())
}

/// The same error with no input read (`-n`), on an empty input, per document and under
/// `--eval-all`, and with `--jq-extensions` (`ireduce` is yq's syntax there too).
#[test]
fn the_union_error_on_every_route_7617() -> Result<()> {
    let filter = ". as $x ireduce (0, 1; . + 1)";
    let docs = "a: 1\n---\nb: 2\n";
    for (input, args) in [
        (INPUT, &["-n"][..]),
        ("", &[][..]),
        (docs, &[][..]),
        (docs, &["--eval-all"][..]),
        (INPUT, &["--jq-extensions"][..]),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, input, args)?;
        assert_eq!(
            (stdout.as_str(), stderr.trim_end(), code),
            ("", UNION_ERROR, 1),
            "{args:?} on {input:?}"
        );
    }
    Ok(())
}

/// (filter, stdout) that yq v4.53.3 answers with exit code 0: a parenthesised union, a union in
/// UPDATE, an INIT whose top is another operator, and a fold a short-circuit never reaches.
const ACCEPTED_ROWS: &[(&str, &str)] = &[
    (".[] as $x ireduce ((0, 1); . + $x)", "3\n4\n"),
    ("[.[] as $x ireduce ((0, 1); . + $x)]", "[3,4]\n"),
    (".[] as $x ireduce ((0, 1) | . + 1; . + $x)", "4\n5\n"),
    (".[] as $x ireduce (0; . + $x, . + 1)", "3\n3\n2\n2\n"),
    (".[] as $x ireduce (0 | 1; . + $x)", "4\n"),
    (".[] as $x ireduce (0 // 1; . + $x)", "3\n"),
    (".[] as $x ireduce (0 as $z | 1; . + $x)", "4\n"),
    (".[] as $x ireduce ([0, 1]; . + [$x])", "[0,1,1,2]\n"),
    (
        ".[] as $x ireduce ({\"a\": 0, \"b\": 1}; .)",
        "{\"a\":0,\"b\":1}\n",
    ),
    ("false and (.[] as $x ireduce (0, 1; . + $x))", "false\n"),
    ("true or (.[] as $x ireduce (0, 1; . + $x))", "true\n"),
];

#[test]
fn other_init_shapes_still_fold_7617() -> Result<()> {
    for (filter, expected) in ACCEPTED_ROWS {
        let (stdout, stderr, code) =
            run_yq_stdin_with_stderr(filter, INPUT, &["-o", "json", "-I0"])?;
        assert_eq!(code, 0, "{filter}: {stderr}");
        assert_eq!(&stdout, expected, "{filter}");
    }
    Ok(())
}

/// jq's `reduce` reads a comma in INIT as two folds, and is unchanged: jq 1.7.1 prints `3`, then its
/// second fork reads a `null` source and fails. yq-mode `reduce` (behind `--jq-extensions`) keeps
/// the yq accumulator list it has since #6802: two accumulators, `3` and `4`.
#[test]
fn reduce_keeps_its_comma_in_init_7617() -> Result<()> {
    let filter = "reduce .[] as $x (0, 1; . + $x)";
    let (stdout, code) = run_jq_stdin(filter, INPUT, &["-c"])?;
    assert_eq!((stdout.as_str(), code), ("3\n", 5));
    let (stdout, code) = run_yq_stdin(filter, INPUT, &["--jq-extensions", "-o", "json", "-I0"])?;
    assert_eq!((stdout.as_str(), code), ("3\n4\n", 0));
    Ok(())
}
