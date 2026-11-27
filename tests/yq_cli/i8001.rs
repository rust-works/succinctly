//! #8001: a second `;` in a yq `ireduce` makes UPDATE a block. yq's `;` is the BLOCK operator, of
//! `,`'s precedence and associating to the right, so `(0; U; V)` is `0; (U; V)` and UPDATE is the
//! block `U; V`. A block evaluated as a value is yq's `emptyOperator`: it yields nothing and
//! evaluates neither side, so the fold drops every accumulator at its first source node. A union
//! in front of the block keeps its operands before the one the `;` follows: `(0; A, B; V)` is
//! `0; (A, (B; V))`, whose UPDATE is `A`. Every row is captured from yq v4.53.3, as
//! `yq -o=json -I0 FILTER` on `[1,2]` unless it says otherwise.

use anyhow::Result;

use crate::{run_jq_stdin_with_stderr, run_yq_stdin, run_yq_stdin_with_stderr};

const INPUT: &str = "[1,2]\n";
const JSON: &[&str] = &["-o", "json", "-I0"];

/// Filters yq v4.53.3 answers with no output and exit code 0.
const NOTHING_ROWS: &[&str] = &[
    ".[] as $x ireduce (0; . + $x; .)",
    ".[] as $x ireduce (0; . + $x; 1)",
    ".[] as $x ireduce (0; . + $x ; . )",
    // Neither side of the block is evaluated.
    ".[] as $x ireduce (0; . + $x; error(\"v\"))",
    ".[] as $x ireduce (0; error(\"u\"); 1)",
    ".[] as $x ireduce (0; . + $x; 1, error(\"w\"))",
    ".[] as $x ireduce (0; . + $x; $nope)",
    ".[] as $x ireduce (0; $nope; 1)",
    ".[] as $x ireduce (0; . + $x; $__loc__)",
    ".[] as $x ireduce (0; $__loc__; 1)",
    // Four and five parts, and unions and pipes in the parts after the first `;`.
    ".[] as $x ireduce (0; . + $x; 1; 2)",
    ".[] as $x ireduce (0; . + $x; 1; 2; 3)",
    ".[] as $x ireduce (0; . + $x; 1, 2)",
    ".[] as $x ireduce (0; . + $x; 1 | 2; 3 , 4)",
    // A pipe, `and` and a binding bind tighter than `;`.
    ".[] as $x ireduce (0; . + $x | . * 2; 1)",
    ".[] as $x ireduce (0; . + $x and true; 9)",
    ".[] as $x ireduce (0; . as $z | $z + $x; 9)",
    // Several accumulators, a one-node source (`[]` is one empty array).
    ".[] as $x ireduce ((0, 1); . + $x; 1)",
    ".[0] as $x ireduce (0; . + $x; 1)",
    "[] as $x ireduce (5; . + $x; 1)",
    // Nothing reaches the next stage, and a nested fold of the same shape is empty too.
    ".[] as $x ireduce (0; . + $x; 1) | length",
    ".[] as $x ireduce (.[] as $y ireduce (0; .; .); . + $x)",
    ".[] as $x ireduce (0; . + $x; .[] as $y ireduce (0; .; .))",
];

#[test]
fn a_block_update_folds_to_nothing_8001() -> Result<()> {
    for filter in NOTHING_ROWS {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, INPUT, JSON)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            ("", "", 0),
            "{filter}"
        );
    }
    Ok(())
}

/// (filter, stdout) that yq v4.53.3 answers with exit code 0.
const VALUE_ROWS: &[(&str, &str)] = &[
    // A union before the block keeps the operands before the one the `;` follows.
    (".[] as $x ireduce (0; . + $x, . + 1; 9)", "3\n"),
    (".[] as $x ireduce (0; . + $x, error(\"u\"); 9)", "3\n"),
    (".[] as $x ireduce (0; . + $x, .; 9)", "3\n"),
    (".[] as $x ireduce (0; . + $x, $nope; 9)", "3\n"),
    (".[] as $x ireduce (0; . + $x, $__loc__; 9)", "3\n"),
    (
        ".[] as $x ireduce (0; . + $x, . + 10, . + 100; 9)",
        "3\n12\n11\n20\n",
    ),
    (".[] as $x ireduce (0; . + $x, . + 10)", "3\n12\n11\n20\n"),
    (".[] as $x ireduce (0; . as $z | $z + $x, 4; 9)", "3\n"),
    // That union is a fresh list next to the block, so two identities do not collapse there, where
    // the last two of a union without a block do.
    (".[] as $x ireduce (0; ., .; 9)", "0\n"),
    (".[] as $x ireduce (0; ., ., .; 9)", "0\n0\n0\n0\n"),
    (".[] as $x ireduce (0; ., .)", "0\n"),
    // An inner fold of this shape over an empty source (`.[]` of a number) is its INIT.
    (
        ".[] as $x ireduce (0; .[] as $y ireduce (0; .; .), 5; 1)",
        "0\n",
    ),
    (
        ".[] as $x ireduce (0; 5, .[] as $y ireduce (0; .; .); 1)",
        "5\n",
    ),
    (".[] as $x ireduce (0; .[] as $y ireduce (0; .; .))", "0\n"),
    // Over an empty source the fold is INIT, and UPDATE never runs.
    ("(.[] | select(. > 5)) as $x ireduce (5; . + $x; 1)", "5\n"),
    (
        "(.[] | select(. > 5)) as $x ireduce ((5, 6); . + $x; 1)",
        "5\n6\n",
    ),
    (
        "(.[] | select(. > 5)) as $x ireduce (5; . + $x, 7; 1)",
        "5\n",
    ),
    // The empty fold inside other expressions.
    ("[.[] as $x ireduce (0; . + $x; .)]", "[]\n"),
    ("[.[] as $x ireduce (0; . + $x; .)] | length", "0\n"),
    ("1, (.[] as $x ireduce (0; . + $x; .)), 2", "1\n2\n"),
    ("(.[] as $x ireduce (0; . + $x; .)) as $q | [$q]", "[]\n"),
    ("[1] as $x ireduce (0; . + 1; 1) | \"after\"", "\"after\"\n"),
    ("\"x\" + (.[] as $x ireduce (0; . + 1; 1))", "\"x\"\n"),
];

#[test]
fn a_union_before_the_block_is_the_update_8001() -> Result<()> {
    for (filter, expected) in VALUE_ROWS {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, INPUT, JSON)?;
        assert_eq!(code, 0, "{filter}: {stderr}");
        assert_eq!(&stdout, expected, "{filter}");
    }
    Ok(())
}

/// No output with no input read (`-n`), per document, under `--eval-all` and with
/// `--jq-extensions` (`ireduce` is yq's syntax there too).
#[test]
fn a_block_update_on_every_route_8001() -> Result<()> {
    let docs = "a: 1\n---\nb: 2\n";
    for (filter, input, args) in [
        ("[1,2] | .[] as $x ireduce (0; . + $x; .)", "", &["-n"][..]),
        (". as $x ireduce (3; . + $x; .)", INPUT, &["-n"][..]),
        (". as $x ireduce (3; .; .)", docs, &[][..]),
        (". as $x ireduce (3; .; .)", docs, &["--eval-all"][..]),
        (
            ".[] as $x ireduce (0; . + $x; .)",
            INPUT,
            &["--jq-extensions"][..],
        ),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, input, args)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            ("", "", 0),
            "{filter} {args:?} on {input:?}"
        );
    }
    Ok(())
}

/// As the value of a write: `=` evaluates the fold against the document, where its UPDATE runs and
/// leaves nothing, and `|=` against `.b`, where `.a[]` reads nothing and the fold is INIT.
#[test]
fn a_block_update_as_a_written_value_8001() -> Result<()> {
    let input = "a: [1, 2]\n";
    for (filter, expected) in [
        (
            ".b = (.a[] as $x ireduce (0; . + $x; .))",
            "a: [1, 2]\nb: null\n",
        ),
        (
            ".b |= (.a[] as $x ireduce (0; . + $x; .))",
            "a: [1, 2]\nb: 0\n",
        ),
    ] {
        let (stdout, code) = run_yq_stdin(filter, input, &[])?;
        assert_eq!((stdout.as_str(), code), (expected, 0), "{filter}");
    }
    Ok(())
}

/// SRC and INIT are evaluated, so an error in either still fails the run; a union at the top of
/// INIT is still #7617's error, whatever follows it.
#[test]
fn src_and_init_still_raise_8001() -> Result<()> {
    for (filter, error) in [
        (".[] as $x ireduce (error(\"i\"); . + $x; 1)", "Error: i"),
        (
            "(.[] | error(\"s\")) as $x ireduce (0; . + $x; 1)",
            "Error: s",
        ),
        (
            ".[] as $x ireduce (0, 1; . + $x; 1)",
            "Error: reduce must be given a block, got UNION instead",
        ),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, INPUT, JSON)?;
        assert_eq!(
            (stdout.as_str(), stderr.trim_end(), code),
            ("", error, 1),
            "{filter}"
        );
    }
    Ok(())
}

/// An undefined function in a part that is never evaluated still fails the run in both: yq's lexer
/// rejects the name (`Error: 1:31: lexer: invalid input text "nope)"`), and the resolver here.
#[test]
fn an_undefined_function_in_the_block_still_raises_8001() -> Result<()> {
    for (filter, name) in [
        (".[] as $x ireduce (0; . + $x; nope)", "nope/0"),
        (".[] as $x ireduce (0; nope; 9)", "nope/0"),
        (".[] as $x ireduce (0; . + $x, nope; 9)", "nope/0"),
        (".[] as $x ireduce (0; . + $x, nope(1); 9)", "nope/1"),
        (".[] as $x ireduce (0; . + $x; 1; nope)", "nope/0"),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, INPUT, JSON)?;
        // The first line only: under `RUST_BACKTRACE` (CI sets it) the message is followed by a
        // backtrace.
        assert_eq!(
            (stdout.as_str(), stderr.lines().next(), code),
            (
                "",
                Some(format!("Error: {name} is not defined").as_str()),
                1
            ),
            "{filter}"
        );
    }
    Ok(())
}

/// An empty part is an error in both (yq's message is `'ireduce' expects 2 args but there is 1`,
/// a parse error here).
#[test]
fn an_empty_part_is_still_an_error_8001() -> Result<()> {
    for filter in [
        ".[] as $x ireduce (0; . + $x; )",
        ".[] as $x ireduce (0; . + $x;;1)",
        ".[] as $x ireduce (0; . + $x, ; 1)",
        ".[] as $x ireduce (0; . + $x; 1 ,)",
        ".[] as $x ireduce (0; ; 1)",
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, INPUT, JSON)?;
        assert_eq!((stdout.as_str(), code), ("", 1), "{filter}");
        assert!(
            stderr.starts_with("Error: parse error"),
            "{filter}: {stderr}"
        );
    }
    Ok(())
}

/// jq's `reduce` takes exactly two parts and is unchanged: jq 1.7.1 fails to compile a third
/// (exit 3), and so does `succinctly jq`; `reduce` under `--jq-extensions` keeps refusing it.
#[test]
fn reduce_still_takes_two_parts_8001() -> Result<()> {
    let filter = "reduce .[] as $x (0; . + $x; 1)";
    let (stdout, stderr, code) = run_jq_stdin_with_stderr(filter, INPUT, &["-c"])?;
    assert_eq!((stdout.as_str(), code), ("", 3), "{stderr}");
    let (stdout, stderr, code) =
        run_yq_stdin_with_stderr(filter, INPUT, &["--jq-extensions", "-o", "json"])?;
    assert_eq!((stdout.as_str(), code), ("", 1), "{stderr}");
    assert!(stderr.starts_with("Error: parse error"), "{stderr}");
    Ok(())
}
