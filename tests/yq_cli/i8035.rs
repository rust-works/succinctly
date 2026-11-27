//! #8035: yq's `reduceOperator` (`pkg/yqlib/operator_reduce.go`, v4.53.3) runs UPDATE over the
//! whole accumulator list for every source node, even when that list is empty, and a collect, a
//! literal, a loop variable or an `error` answers over an empty list (the #6943 rule). `succinctly
//! yq` stopped folding once the list was empty and printed nothing. Every row is captured from yq
//! v4.53.3, as `yq -o=json -I0 FILTER` on `[1, 2]` unless it says otherwise.

use anyhow::Result;

use crate::{run_jq_stdin_with_stderr, run_yq_stdin_with_stderr};

const INPUT: &str = "[1, 2]\n";
const JSON: &[&str] = &["-o", "json", "-I0"];

/// `.[] as $x ireduce (select(false); UPDATE)`: INIT yields nothing, so the first step runs UPDATE
/// over an empty list and the second over whatever that left.
const EMPTY_INIT_ROWS: &[(&str, &str)] = &[
    // A collect is `[]` over an empty list, whatever its body.
    ("[.]", "[[]]\n"),
    ("[.[]]", "[]\n"),
    ("[$x]", "[2]\n"),
    ("[.] | length", "1\n"),
    ("[.] + [$x]", "[[],2]\n"),
    ("select($x == 2) | [.]", "[[]]\n"),
    ("[.] | select($x == 2)", "[]\n"),
    // Literals, and operators over them.
    ("1", "1\n"),
    ("\"x\"", "\"x\"\n"),
    ("-1", "-1\n"),
    ("1 + 2", "3\n"),
    ("\"x\" + \"y\"", "\"xy\"\n"),
    ("1 == 1", "true\n"),
    ("null // 1", "1\n"),
    ("\"x\" | test(\"x\")", "true\n"),
    ("{}", "{}\n"),
    ("{\"a\": 1}", "{\"a\":1}\n"),
    // The loop variable is a value whatever the context holds.
    ("$x", "2\n"),
    ("$x + 1", "3\n"),
    ("\"x\" + $x", "\"x2\"\n"),
    ("{$x: 1}", "{\"2\":1}\n"),
    ("{\"k\": $x}", "{\"k\":2}\n"),
    // A union is branch-major over the list: `(1, 2)` makes two accumulators at the first node.
    ("(1, 2)", "1\n1\n2\n2\n"),
    ("($x, .)", "2\n1\n"),
    ("(., $x)", "1\n2\n"),
    // Shapes that read their input answer nothing over an empty list.
    ("length", ""),
    (". + 1", ""),
    (". + $x", ""),
    ("{\"a\": .}", ""),
    ("select(true)", ""),
    ("[.] | .[]", ""),
];

#[test]
fn update_runs_over_an_empty_init_8035() -> Result<()> {
    for (update, expected) in EMPTY_INIT_ROWS {
        let filter = format!(".[] as $x ireduce (select(false); {update})");
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(&filter, INPUT, JSON)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (*expected, "", 0),
            "{filter}"
        );
    }
    Ok(())
}

/// A list that UPDATE empties part-way is still folded for the nodes after it, on `[1, 2, 3]`.
#[test]
fn a_list_emptied_mid_fold_is_still_folded_8035() -> Result<()> {
    let input = "[1, 2, 3]\n";
    for (filter, expected) in [
        // Emptied at 2, and nothing over the empty list refills it.
        (".[] as $x ireduce (0; select($x < 2) | . + $x)", ""),
        (".[] as $x ireduce (0; select($x != 2))", ""),
        (".[] as $x ireduce ((0, 1); select(. == 9))", ""),
        // `[$x]` over the empty list at 3 is `[]`, so `.[]` finds nothing in it.
        (
            ".[] as $x ireduce (0; [$x] | .[] | select(. == 1 or . == 3))",
            "",
        ),
        // Emptied at 2 and refilled at 3 by the loop variable.
        (
            ".[] as $x ireduce (0; select($x == 1) , ($x | select(. == 3)))",
            "3\n",
        ),
        // Three steps of `[.]` from an empty list.
        (".[] as $x ireduce (select(false); [.])", "[[[]]]\n"),
        ("[.[] as $x ireduce (select(false); [.])]", "[[[[]]]]\n"),
        (".[] as $x ireduce (select(false); [.]) | length", "1\n"),
        (
            ".[] as $x ireduce (select(false); ($x | select(. == 2)), [.])",
            "[2]\n[[[]]]\n",
        ),
        // `[]` is one source node, so one step over the empty list.
        ("[] as $x ireduce (select(false); [.])", "[]\n"),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, input, JSON)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (expected, "", 0),
            "{filter}"
        );
    }
    // On the right side of `|=`, over the one target `[1, 2]`.
    for (filter, expected) in [
        (
            ".[0] |= (.[] as $x ireduce (select(false); [.]))",
            "[[[]],3]\n",
        ),
        (".[0] |= (.[] as $x ireduce (select(false); $x))", "[2,3]\n"),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, "[[1, 2], 3]\n", JSON)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (expected, "", 0),
            "{filter}"
        );
    }
    Ok(())
}

/// An `error` over the empty list fires, and is still outranked by an error later in the source
/// (#8022): yq reads the whole source before it folds.
#[test]
fn an_error_over_the_empty_list_fires_8035() -> Result<()> {
    for (filter, input, error) in [
        (
            ".[] as $x ireduce (select(false); error(\"u\"))",
            INPUT,
            "Error: u",
        ),
        (
            ".[] as $x ireduce (select(false); error(\"u\" + \"v\"))",
            INPUT,
            "Error: uv",
        ),
        (
            ".[] as $x ireduce (select(false); error($x))",
            INPUT,
            "Error: 1",
        ),
        (
            ".[] as $x ireduce (0; select($x < 2) | error(\"u\"))",
            "[1, 2, 3]\n",
            "Error: u",
        ),
        (
            "(.[], error(\"s\")) as $x ireduce (select(false); error(\"u\"))",
            "[1, 2, 3]\n",
            "Error: s",
        ),
        (
            "(.[], error(\"s\")) as $x ireduce (select(false); [.])",
            "[1, 2, 3]\n",
            "Error: s",
        ),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, input, JSON)?;
        assert_eq!(
            (stdout.as_str(), stderr.trim_end(), code),
            ("", error, 1),
            "{filter}"
        );
    }
    Ok(())
}

/// With no input document (`-n`), over each document and over the whole document list
/// (`--eval-all`) yq v4.53.3 answers the same way; `.a` reads `1` and `2` from the two documents.
#[test]
fn null_input_and_documents_8035() -> Result<()> {
    let docs = "a: 1\n---\na: 2\n";
    let null_input = &["-n", "-o", "json", "-I0"][..];
    let each = &["-o", "json", "-I0"][..];
    let all = &["--eval-all", "-o", "json", "-I0"][..];
    for (filter, input, args, expected) in [
        (
            "[1, 2] | .[] as $x ireduce (select(false); [.])",
            "",
            null_input,
            "[[]]\n",
        ),
        (
            "(1, 2) as $x ireduce (select(false); $x)",
            "",
            null_input,
            "2\n",
        ),
        (
            "select(false) as $x ireduce (select(false); [.])",
            "",
            null_input,
            "",
        ),
        (
            ". as $d ireduce (select(false); [.])",
            docs,
            each,
            "[]\n[]\n",
        ),
        (".a as $x ireduce (select(false); $x)", docs, each, "1\n2\n"),
        (
            ". as $d ireduce (select(false); $d.a)",
            docs,
            each,
            "1\n2\n",
        ),
        (". as $d ireduce (select(false); [.])", docs, all, "[[]]\n"),
        (".a as $x ireduce (select(false); [.])", docs, all, "[[]]\n"),
        (".a as $x ireduce (select(false); $x)", docs, all, "2\n"),
        (". as $d ireduce (select(false); $d.a)", docs, all, "2\n"),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, input, args)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (expected, "", 0),
            "{filter} {args:?}"
        );
    }
    Ok(())
}

/// jq's `reduce` is one fork per INIT output, so an empty INIT is no output, and is unchanged
/// (jq 1.7.1 prints nothing for each). yq-mode `reduce` (behind `--jq-extensions`) shares yq's
/// fold, so it runs UPDATE over the empty list too: a recorded divergence from jq
/// (`docs/compliance/yq/limitations.md`).
#[test]
fn jq_reduce_over_an_empty_init_is_unchanged_8035() -> Result<()> {
    for filter in [
        "reduce .[] as $x (empty; [.])",
        "reduce .[] as $x (empty; 1)",
        "reduce .[] as $x (select(false); $x)",
    ] {
        let (stdout, stderr, code) = run_jq_stdin_with_stderr(filter, INPUT, &["-c"])?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            ("", "", 0),
            "{filter}"
        );
    }
    let ext = &["--jq-extensions", "-o", "json", "-I0"];
    for (filter, expected) in [
        ("reduce .[] as $x (empty; [.])", "[[]]\n"),
        ("reduce .[] as $x (empty; 1)", "1\n"),
        ("reduce .[] as $x (select(false); $x)", "2\n"),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, INPUT, ext)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (expected, "", 0),
            "{filter}"
        );
    }
    Ok(())
}

/// A fold whose UPDATE reads no loop variable and answers nothing over the empty list reads the
/// rest of its source only for errors, so an endless source still runs to its cap (behind
/// `--jq-extensions`; yq has no endless builtin), as before #8035.
#[test]
fn an_endless_source_with_nothing_to_fold_still_reaches_its_cap_8035() -> Result<()> {
    let ext = &["--jq-extensions", "-n", "-o", "json", "-I0"];
    for filter in [
        "range(infinite) as $x ireduce (select(false); .)",
        "repeat(1) as $x ireduce (0; select(false))",
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, "", ext)?;
        assert_eq!((stdout.as_str(), code), ("", 1), "{filter}: {stderr}");
        assert!(stderr.starts_with("Error: "), "{filter}: {stderr}");
    }
    Ok(())
}
