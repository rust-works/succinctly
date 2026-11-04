//! #6162: yq compares a mapping's values for `min`/`max` as it compares a sequence's elements: a
//! string, boolean, array or map among them is an error. Every expected output is what yq v4.53.3
//! prints (`-o=json -I=0`).

use anyhow::Result;

use crate::{run_yq_stdin, run_yq_stdin_with_stderr};

/// (document, filter, expected stdout).
const ROWS: &[(&str, &str, &str)] = &[
    ("{a: 3, b: 1}\n", "min", "1\n"),
    ("{a: 3, b: 1}\n", "max", "3\n"),
    ("{}\n", "min", ""),
    ("{}\n", "max", ""),
    ("{a: null, b: 1}\n", "min", "null\n"),
    ("{a: null, b: 1}\n", "max", "null\n"),
    ("{a: 1.5, b: 2}\n", "min", "1.5\n"),
    ("{a: 1.5, b: 2}\n", "max", "2\n"),
    ("{a: x}\n", "min", "\"x\"\n"),
    ("{a: x}\n", "max", "\"x\"\n"),
];

/// (document, filter, yq's error line).
const ERROR_ROWS: &[(&str, &str, &str)] = &[
    (
        "{a: 1, b: [2]}\n",
        "min",
        "Error: arrays not yet supported for comparison",
    ),
    (
        "{a: 1, b: [2]}\n",
        "max",
        "Error: arrays not yet supported for comparison",
    ),
    (
        "{a: [1], b: [2]}\n",
        "min",
        "Error: arrays not yet supported for comparison",
    ),
    (
        "{a: [1], b: [2]}\n",
        "max",
        "Error: arrays not yet supported for comparison",
    ),
    (
        "{a: 1, b: x}\n",
        "min",
        "Error: !!str not yet supported for comparison",
    ),
    (
        "{a: 1, b: x}\n",
        "max",
        "Error: !!str not yet supported for comparison",
    ),
    (
        "{a: 2, b: {c: 1}}\n",
        "min",
        "Error: maps not yet supported for comparison",
    ),
    (
        "{a: 2, b: {c: 1}}\n",
        "max",
        "Error: maps not yet supported for comparison",
    ),
    (
        "{a: true, b: 1}\n",
        "min",
        "Error: !!int not yet supported for comparison",
    ),
    (
        "{a: true, b: 1}\n",
        "max",
        "Error: !!int not yet supported for comparison",
    ),
];

#[test]
fn a_mappings_extremum_picks_among_numbers_and_nulls_6162() -> Result<()> {
    for &(document, filter, expected) in ROWS {
        let (output, code) = run_yq_stdin(filter, document, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{filter} over {document:?}");
        assert_eq!(output, expected, "{filter} over {document:?}");
    }
    Ok(())
}

#[test]
fn a_mappings_extremum_refuses_what_a_sequences_does_6162() -> Result<()> {
    for &(document, filter, expected) in ERROR_ROWS {
        let (_, stderr, code) = run_yq_stdin_with_stderr(filter, document, &["-o=json", "-I=0"])?;
        assert_eq!(code, 1, "{filter} over {document:?}");
        assert!(
            stderr.contains(expected),
            "{filter} over {document:?}: {stderr}"
        );
    }
    Ok(())
}

/// The same answer through a path, `map`, an iteration and a construction.
#[test]
fn a_mappings_extremum_agrees_on_every_route_6162() -> Result<()> {
    let document = "a: {p: 1, q: x}\nb: {p: 2, q: 3}\nc: [{u: 1}, {u: y}]\n";
    for (filter, expected) in [
        (".b | max", "3\n"),
        (".b | min", "2\n"),
        (".c[] | max", "1\n\"y\"\n"),
        (".c | map(max)", "[1,\"y\"]\n"),
    ] {
        let (output, code) = run_yq_stdin(filter, document, &["-o=json", "-I=0"])?;
        assert_eq!((output.as_str(), code), (expected, 0), "{filter}");
    }
    for filter in [".a | max", ".[] | max", "{\"k\": .a} | .k | max"] {
        let (_, code) = run_yq_stdin(filter, document, &[])?;
        assert_eq!(code, 1, "{filter}");
    }
    Ok(())
}
