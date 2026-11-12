//! #7019: yq writes `---` only before a result whose document index differs from the previous
//! result's, and a node yq builds from a document's root or from nothing (a literal, `[...]`,
//! `length`, `keys`, `tag`, `@json`, ...) has index 0, as the first document's nodes do. A filter
//! whose results are all of that kind prints no `---`; a navigation (`.a`, `.a | length`) keeps
//! the separator before each document's first result. Every answer was captured from yq v4.53.3.

use std::io::Write;

use anyhow::Result;
use tempfile::NamedTempFile;

use crate::{run_yq_files, run_yq_stdin};

const INPUT: &str = "a: 1\nl: [1, 2]\n---\na: 2\nl: [3]\n---\na: 3\nl: []\n";

/// (filter, yq's stdout over [`INPUT`]).
const FRESH_ROWS: &[(&str, &str)] = &[
    ("\"x\"", "x\nx\nx\n"),
    ("[.a]", "- 1\n- 2\n- 3\n"),
    ("length", "2\n2\n2\n"),
    ("keys", "- a\n- l\n- a\n- l\n- a\n- l\n"),
    ("keys | .[0]", "a\na\na\n"),
    ("tag", "!!map\n!!map\n!!map\n"),
    ("type", "!!map\n!!map\n!!map\n"),
    ("has(\"a\")", "true\ntrue\ntrue\n"),
    ("tostring", "a: 1\nl: [1, 2]\na: 2\nl: [3]\na: 3\nl: []\n"),
    ("to_json", "{\n  \"a\": 1,\n  \"l\": [\n    1,\n    2\n  ]\n}\n\n{\n  \"a\": 2,\n  \"l\": [\n    3\n  ]\n}\n\n{\n  \"a\": 3,\n  \"l\": []\n}\n\n"),
    ("@json", "{\"a\":1,\"l\":[1,2]}\n{\"a\":2,\"l\":[3]}\n{\"a\":3,\"l\":[]}\n"),
    ("@props", "a = 1\nl.0 = 1\nl.1 = 2\n\na = 2\nl.0 = 3\n\na = 3\n\n"),
    ("\"\\(.a)\"", "1\n2\n3\n"),
    ("to_entries", "- key: a\n  value: 1\n- key: l\n  value: [1, 2]\n- key: a\n  value: 2\n- key: l\n  value: [3]\n- key: a\n  value: 3\n- key: l\n  value: []\n"),
    ("with_entries(.)", "a: 1\nl: [1, 2]\na: 2\nl: [3]\na: 3\nl: []\n"),
    ("not", "false\nfalse\nfalse\n"),
    ("line", "1\n4\n7\n"),
    ("document_index", "0\n1\n2\n"),
    ("{\"k\": .a} | length", "1\n1\n1\n"),
    ("1 + 1", "2\n2\n2\n"),
    (". == 1", "false\nfalse\nfalse\n"),
    ("\"x\" // 1", "x\nx\nx\n"),
    ("select(.a > 1) | \"x\"", "x\nx\nx\n"),
    (". as $x | \"x\"", "x\nx\nx\n"),
    ("\"x\", 1", "x\n1\nx\n1\nx\n1\n"),
    ("[.a] | map(. + 1)", "- 2\n- 3\n- 4\n"),
    ("{}", "{}\n{}\n{}\n"),
    ("1 // .a", "1\n1\n1\n"),
    ("false // \"x\"", "x\nx\nx\n"),
    ("\"x\" as $y | $y", "x\nx\nx\n"),
    ("[.a] as $x | $x", "- 1\n- 2\n- 3\n"),
    (".a as $x | $x | \"y\"", "y\ny\ny\n"),
    ("del(.a) | \"x\"", "x\nx\nx\n"),
    ("(.a // 1) | \"x\"", "x\nx\nx\n"),
];

const DOCUMENT_ROWS: &[(&str, &str)] = &[
    (
        ".",
        "a: 1\nl: [1, 2]\n---\na: 2\nl: [3]\n---\na: 3\nl: []\n",
    ),
    (".a", "1\n---\n2\n---\n3\n"),
    (".a | length", "1\n---\n1\n---\n1\n"),
    (".l | [.[]]", "- 1\n- 2\n---\n- 3\n---\n[]\n"),
    ("{\"k\": .a}", "k: 1\n---\nk: 2\n---\nk: 3\n"),
    (".a + 1", "2\n---\n3\n---\n4\n"),
    (".a == 1", "true\n---\nfalse\n---\nfalse\n"),
    (".a // 1", "1\n---\n2\n---\n3\n"),
    (
        "sort_keys(.)",
        "a: 1\nl: [1, 2]\n---\na: 2\nl: [3]\n---\na: 3\nl: []\n",
    ),
    (".a | tag", "!!int\n---\n!!int\n---\n!!int\n"),
];

/// Recorded divergences (`docs/compliance/yq/limitations.md`, "The `---` between results is
/// per-run"): yq's answer is in the comment, the row pins succinctly's.
const RESIDUAL_ROWS: &[(&str, &str)] = &[
    // yq: `[1, 2]\n[3]\n---\n[]\n` (`map` copies an empty input, which keeps its document).
    (".l | map(.)", "[1, 2]\n---\n[3]\n---\n[]\n"),
    // yq: `1\nx\n---\n2\n---\nx\n---\n3\n---\nx\n` (it compares every result's document).
    (".a, \"x\"", "1\nx\n---\n2\nx\n---\n3\nx\n"),
];

const CHILD_INPUT: &str = "a: {x: 1, y: [1]}\nn: 2\ns: ab\n---\na: {x: 2, y: [2]}\nn: 3\ns: cd\n";

/// Over a child: `keys`, `with_entries` and interpolation build a new node whatever their input;
/// a comparison or `and`/`or` builds from its left operand, and arithmetic from a non-null left
/// literal. (filter, yq's stdout over [`CHILD_INPUT`]).
const CHILD_ROWS: &[(&str, &str)] = &[
    (".a | keys", "- x\n- y\n- x\n- y\n"),
    (".a | with_entries(.)", "x: 1\ny: [1]\nx: 2\ny: [2]\n"),
    (".a | \"\\(.)\"", "{x: 1, y: [1]}\n{x: 2, y: [2]}\n"),
    ("1 == .n", "false\nfalse\n"),
    ("\"x\" and .a", "true\ntrue\n"),
    ("true or .a", "true\ntrue\n"),
    ("\"x\" + .s", "xab\nxcd\n"),
    ("2 * .n", "4\n6\n"),
    (".n == 1", "false\n---\nfalse\n"),
    (".a | length", "2\n---\n2\n"),
];

/// A document's own leading `---` stays where the run's separators go (input, filter, yq's stdout).
const PREAMBLE_ROWS: &[(&str, &str, &str)] = &[
    ("# c\n---\na: 1\n", "[.][]", "# c\n---\na: 1\n"),
    ("# c\n---\na: 1\n", "[.] | .[0]", "# c\n---\na: 1\n"),
    ("---\na: 1\n---\na: 2\n", "[.][]", "---\na: 1\na: 2\n"),
    ("---\na: 1\n---\na: 2\n", "[.] | .[0]", "---\na: 1\na: 2\n"),
];

#[test]
fn results_built_over_a_child_or_from_the_left_operand_7019() -> Result<()> {
    for (filter, expected) in CHILD_ROWS {
        let (stdout, status) = run_yq_stdin(filter, CHILD_INPUT, &[])?;
        assert_eq!((stdout.as_str(), status), (*expected, 0), "{filter}");
    }
    Ok(())
}

#[test]
fn a_documents_leading_separator_stays_7019() -> Result<()> {
    for (input, filter, expected) in PREAMBLE_ROWS {
        let (stdout, status) = run_yq_stdin(filter, input, &[])?;
        assert_eq!(
            (stdout.as_str(), status),
            (*expected, 0),
            "{input:?} {filter}"
        );
    }
    Ok(())
}

#[test]
fn results_built_with_no_document_print_no_separator_7019() -> Result<()> {
    for (filter, expected) in FRESH_ROWS.iter().chain(DOCUMENT_ROWS).chain(RESIDUAL_ROWS) {
        let (stdout, status) = run_yq_stdin(filter, INPUT, &[])?;
        assert_eq!((stdout.as_str(), status), (*expected, 0), "{filter}");
    }
    Ok(())
}

/// `-0` ends each result in a NUL, and `---` follows it where yq separates (`.a`), on a line of its
/// own (the "`-0`/`--nul-output` multi-document separator" divergence; yq writes `1\0---`).
#[test]
fn nul_output_separates_only_document_results_7019() -> Result<()> {
    for (filter, expected) in [
        ("\"x\"", "x\0x\0x\0"),
        ("length", "2\x002\x002\x00"),
        (".a", "1\0\n---\n2\0\n---\n3\0"),
    ] {
        let (stdout, status) = run_yq_stdin(filter, INPUT, &["-0"])?;
        assert_eq!((stdout.as_str(), status), (expected, 0), "{filter}");
    }
    Ok(())
}

/// `-i` keeps the separators where yq drops them: yq's `2`/`2`/`2` would read back as the one
/// string `2 2 2` (a recorded divergence).
#[test]
fn inplace_keeps_the_documents_apart_7019() -> Result<()> {
    let mut file = NamedTempFile::new()?;
    write!(file, "{INPUT}")?;
    let (_, _, status) = run_yq_files("length", &[file.path()], &["-i"])?;
    assert_eq!(status, 0);
    assert_eq!(std::fs::read_to_string(file.path())?, "2\n---\n2\n---\n2\n");
    Ok(())
}

/// A fresh result of a second file has index 0 too, so two files print no `---` between them.
#[test]
fn results_built_with_no_document_join_files_7019() -> Result<()> {
    let mut f1 = NamedTempFile::new()?;
    write!(f1, "a: 1\nname: first\n")?;
    let mut f2 = NamedTempFile::new()?;
    write!(f2, "b: 2\nname: second\n")?;
    let files = [f1.path(), f2.path()];
    for (filter, expected) in [
        ("length", "2\n2\n"),
        ("file_index", "0\n1\n"),
        (".name", "first\n---\nsecond\n"),
    ] {
        let (stdout, _, status) = run_yq_files(filter, &files, &[])?;
        assert_eq!((stdout.as_str(), status), (expected, 0), "{filter}");
    }
    Ok(())
}
