//! #5283: a top-level `,` whose branches are all navigation streams each branch on its own, so a
//! container with a repeated mapping key keeps it (`., .b` collapsed `a: 1 / a: 2` into `a: 2`),
//! a node keeps its flow style when a sibling branch is `null`, and the colliding-undecodable-key
//! document that `.` echoes is echoed by `.,.` too (#2645). Every expected output in
//! `ORACLE_ROWS` is what yq v4.53.3 prints.

use anyhow::Result;

use crate::{run_yq_stdin, run_yq_stdin_with_stderr};

const DUP: &str = "a: 1\na: 2\nb: [1, 2]\n";
const FLOW: &str = "a: {b: 1, c: [1, 2]}\nl: [10, 20]\n";
const MULTI: &str = "a: 1\n---\nb: 2\n---\nc: [1, 2]\n";
const ALIAS: &str = "x: &a {p: 1}\ny:\n  q: *a\n";

/// (document, filter, extra args, expected stdout).
const ORACLE_ROWS: &[(&str, &str, &[&str], &str)] = &[
    (DUP, "., .b", &[], "a: 1\na: 2\nb: [1, 2]\n[1, 2]\n"),
    (
        DUP,
        "., .b",
        &["-o=json", "-I0"],
        "{\"a\":1,\"a\":2,\"b\":[1,2]}\n[1,2]\n",
    ),
    (DUP, ".b, .", &[], "[1, 2]\na: 1\na: 2\nb: [1, 2]\n"),
    (
        DUP,
        "(.a, .b), .",
        &[],
        "2\n[1, 2]\na: 1\na: 2\nb: [1, 2]\n",
    ),
    // `. , .` is one node: yq prints it once.
    (DUP, ". , .", &[], "a: 1\na: 2\nb: [1, 2]\n"),
    // A sibling that is not a node (an absent key) does not turn the others into copies.
    (FLOW, ".a, .zz", &[], "{b: 1, c: [1, 2]}\nnull\n"),
    (
        FLOW,
        ".a, .l, .a.c",
        &[],
        "{b: 1, c: [1, 2]}\n[10, 20]\n[1, 2]\n",
    ),
    // The document separator is written once per document, before its first output.
    (
        MULTI,
        ".a, .b",
        &[],
        "1\nnull\n---\nnull\n2\n---\nnull\nnull\n",
    ),
    (
        MULTI,
        "(.a, .b), (.c | .[0])",
        &[],
        "1\nnull\nnull\n---\nnull\n2\nnull\n---\nnull\nnull\n1\n",
    ),
    (
        "# head\nx: 1 # xc\ny:\n  - 1.5 # n\n",
        ".x, .y",
        &[],
        "1\n- 1.5 # n\n",
    ),
    // `--no-doc` drops the separators and nothing else.
    (MULTI, ".a, .b", &["-N"], "1\nnull\nnull\n2\nnull\nnull\n"),
    // JSON input, JSON output.
    (
        "{\"a\":1,\"a\":2,\"b\":[1.50,2]}\n",
        "., .b",
        &["-p=json", "-o=json", "-I0"],
        "{\"a\":1,\"a\":2,\"b\":[1.5,2]}\n[1.5,2]\n",
    ),
    // An alias keeps its `*`, as `.[]` and yq print it.
    (ALIAS, ".x, .y", &[], "&a {p: 1}\nq: *a\n"),
    (ALIAS, ".zz, .y", &[], "null\nq: *a\n"),
];

#[test]
fn test_a_top_level_comma_streams_each_branch_5283() -> Result<()> {
    for (doc, filter, args, expected) in ORACLE_ROWS {
        let (out, code) = run_yq_stdin(filter, doc, args)?;
        assert_eq!(code, 0, "{filter} {args:?}");
        assert_eq!(out, *expected, "{filter} {args:?}");
    }
    Ok(())
}

/// `-e` reads the last output across the branches.
#[test]
fn test_exit_status_follows_the_last_branch_5283() -> Result<()> {
    let (out, code) = run_yq_stdin(".a, .b", "a: null\nb: false\n", &["-e"])?;
    assert_eq!((out.as_str(), code), ("null\nfalse\n", 1));
    let (out, code) = run_yq_stdin(".a, .b", "a: 1\nb: false\n", &["-e"])?;
    assert_eq!((out.as_str(), code), ("1\nfalse\n", 0));
    Ok(())
}

/// An evaluation error in any branch prints nothing for the others, not even a document separator,
/// as before. (A failure found while a later branch streams, an undecodable scalar, comes after the
/// earlier branches were written, as it does for `.[]`.)
#[test]
fn test_an_error_in_one_branch_prints_nothing_5283() -> Result<()> {
    let doc = "a: [1]\n---\nb: 2\n";
    let (out, err, code) = run_yq_stdin_with_stderr(".a[0], .a[-5]", doc, &[])?;
    assert_eq!((out.as_str(), code), ("", 1), "stderr: {err}");
    assert!(err.contains("out of range"), "stderr: {err}");
    Ok(())
}

/// The colliding-undecodable-key document: real yq rejects it for every filter, a lone `.` echoes
/// it here, and a comma of navigation reads it the same way `.` does now (#2645). Anything that
/// builds a map from it (`[.]`) still raises, as the limitations page records.
#[test]
fn test_a_comma_reads_a_colliding_undecodable_key_as_dot_does_5283() -> Result<()> {
    let doc = "\"\\ud800\": 1\n\"\\ud800\": 2\nk: 3\n";
    let (dot, code) = run_yq_stdin(".", doc, &[])?;
    assert_eq!((dot.as_str(), code), ("\"\": 1\n\"\": 2\nk: 3\n", 0));
    for (filter, expected) in [
        (". , .", dot.clone()),
        (".k, .", format!("3\n{dot}")),
        (".b, .", format!("null\n{dot}")),
    ] {
        let (out, code) = run_yq_stdin(filter, doc, &[])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(out, expected, "{filter}");
    }
    let (out, err, code) = run_yq_stdin_with_stderr("[.], .", doc, &[])?;
    assert_eq!((out.as_str(), code), ("", 1));
    assert!(err.contains("is ambiguous"), "stderr: {err}");
    Ok(())
}

/// Colored and NUL-terminated output keep the route they were tuned on, and since #1344
/// (ADR-0027) that route keeps the repeated key too, as yq does (#5414).
#[test]
fn test_color_and_nul_output_keep_the_repeated_key_on_the_dom_route_5283() -> Result<()> {
    let strip_ansi = |text: &str| {
        let mut out = String::new();
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            if c == '\u{1b}' {
                for skipped in chars.by_ref() {
                    if skipped == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    };
    let (colored, code) = run_yq_stdin("., .b", DUP, &["-C"])?;
    assert_eq!(code, 0);
    let colored = strip_ansi(&colored);
    assert!(
        colored.contains("a: 2") && colored.contains("a: 1"),
        "{colored:?}"
    );
    let (nul, code) = run_yq_stdin("., .b", DUP, &["-0"])?;
    assert_eq!(code, 0);
    assert!(nul.contains("a: 2") && nul.contains("a: 1"), "{nul:?}");
    Ok(())
}
