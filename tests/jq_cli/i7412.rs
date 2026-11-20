//! #7412: a fold over a document array's elements (`[.[] | f]`) writes its computed values over the
//! cursors it has already read, in the cursors' own buffer, once a long run of elements has each
//! yielded one value or nothing. These pin what the fold yields when the run ends every way it
//! can: to the end, back to the per-element batch at a cursor, a second output, an object, and at
//! an error, caught or not, past the 64 elements after which a run moves (`COMPUTED_RUN_MIN`).
//! Expected outputs are jq 1.7.1's.

use anyhow::Result;

use crate::{run_jq_stdin, run_jq_stdin_streams};

/// `[1, 2, ..., 300]`.
fn numbers() -> String {
    let items: Vec<String> = (1..=300).map(|i| i.to_string()).collect();
    format!("[{}]", items.join(","))
}

/// `["1", "2", ..., "300"]`.
fn strings() -> String {
    let items: Vec<String> = (1..=300).map(|i| format!("\"{i}\"")).collect();
    format!("[{}]", items.join(","))
}

#[test]
fn a_moved_fold_yields_jqs_values_whichever_way_its_run_ends_7412() -> Result<()> {
    let cases: &[(&str, &str)] = &[
        // To the end, every element computed.
        ("[.[] | . * 2] | add", "90300\n"),
        // Elements that yield nothing inside the run.
        ("[.[] | select(. % 3 == 0) | . + 1] | length", "100\n"),
        // Back to the batch at a cursor, after the run moved.
        ("[.[] | if . == 200 then . else . + 0 end] | add", "45150\n"),
        // Back to the batch at an element with two outputs.
        (
            "[.[] | if . == 150 then (., .) else . * 2 end] | length, add",
            "301\n90300\n",
        ),
        // Objects, built as they arrive, at the end of the run.
        (
            "[.[] | if . > 280 then {a: .} else . end] | .[-1], length",
            "{\"a\":300}\n300\n",
        ),
        // An error after the run moved, caught.
        (
            "try [.[] | if . == 250 then error(\"boom\") else . + 1 end] catch .",
            "\"boom\"\n",
        ),
        (
            "[.[] | if . == 100 then error(\"x\") else . end]?, \"after\"",
            "\"after\"\n",
        ),
    ];
    let input = numbers();
    for &(filter, want) in cases {
        let (stdout, code) = run_jq_stdin(filter, &input, &["-c"])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}");
    }
    Ok(())
}

/// Values that own memory (strings) written over the cursors, and an uncaught error after the run
/// moved, which drops the values written so far and the cursors not read.
#[test]
fn a_moved_fold_of_strings_and_a_failing_one_match_jq_7412() -> Result<()> {
    let (stdout, code) = run_jq_stdin(
        "[.[] | tostring | . + \"!\"] | .[0], .[299], length",
        &numbers(),
        &["-c"],
    )?;
    assert_eq!((stdout.as_str(), code), ("\"1!\"\n\"300!\"\n300\n", 0));
    let (stdout, code) = run_jq_stdin("[.[] | ascii_downcase | length] | add", &strings(), &[])?;
    assert_eq!((stdout.as_str(), code), ("792\n", 0));
    let (stdout, stderr, code) = run_jq_stdin_streams(
        "[.[] | if . == 250 then error(\"boom\") else . + 1 end]",
        &numbers(),
        &["-c"],
    )?;
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), code),
        ("", "jq: error (at <stdin>:0): boom\n", 5)
    );
    Ok(())
}
