//! #8077: yq's datetime operators build a replacement of the timestamp they read
//! (`operator_datetime.go`, v4.53.3): `tz`, `to_unix` and `from_unix` through
//! `candidate.CreateReplacement`, and `format_datetime` by giving the YAML snippet it parses the
//! input's parent. A replacement of a node with no document (`now`, a literal, a document's root)
//! has none either, so over several documents yq prints no `---` between such results, while a
//! replacement of a child (`.t | format_datetime(..)`) keeps the child's document and its separator.
//!
//! `now` moves, so no row prints its value: a row over `now` prints a length (`4` for a year, `20`
//! for an RFC 3339 time with no fraction under `TZ=UTC`). Every row was captured from yq v4.53.3
//! under `TZ=UTC`.

use std::process::{Command, Stdio};

use anyhow::Result;

const TWO: &str = "a: 1\n---\na: 2\n";
const DATES: &str = "a: 1\nt: 2021-03-04T05:06:07Z\nn: 1600000000\n---\na: 2\nt: 2022-03-04T05:06:07Z\nn: 1700000000\n";
const ROOT_DATES: &str = "2021-03-04T05:06:07Z\n---\n2022-03-04T05:06:07Z\n";
const ROOT_EPOCHS: &str = "1600000000\n---\n1700000000\n";

/// (flags, filter, stdin, yq's stdout), each with exit status 0 and nothing on stderr.
const ROWS: &[(&[&str], &str, &str, &str)] = &[
    // The issue's rows: a replacement of `now`'s node, which has no document.
    (
        &[],
        "now | format_datetime(\"2006\") | length",
        TWO,
        "4\n4\n",
    ),
    (&[], "now | tz(\"UTC\") | length", TWO, "20\n20\n"),
    (&[], "now | to_unix | from_unix | length", TWO, "20\n20\n"),
    (
        &[],
        "[now] | .[0] | format_datetime(\"2006\") | length",
        TWO,
        "4\n4\n",
    ),
    (
        &[],
        "now | format_datetime(\"[2006]\") | length",
        TWO,
        "1\n1\n",
    ),
    (
        &[],
        "now | format_datetime(\"a: 2006\") | keys",
        TWO,
        "- a\n- a\n",
    ),
    // No `document_index` row: yq answers `0` for every such node, and succinctly the number of
    // the document it was evaluated over (`now | document_index`, docs/compliance/yq/limitations.md).
    (
        &[],
        "\"2021-03-04T05:06:07Z\" | format_datetime(\"2006\")",
        TWO,
        "2021\n2021\n",
    ),
    (
        &["ea"],
        "now | format_datetime(\"2006\") | length",
        TWO,
        "4\n",
    ),
    (
        &["-o", "json"],
        "now | format_datetime(\"2006\") | length",
        TWO,
        "4\n4\n",
    ),
    // A document's root has no parent either, so its replacement has no document.
    (&[], "format_datetime(\"2006\")", ROOT_DATES, "2021\n2022\n"),
    (
        &[],
        "tz(\"UTC\")",
        ROOT_DATES,
        "2021-03-04T05:06:07Z\n2022-03-04T05:06:07Z\n",
    ),
    (&[], "to_unix", ROOT_DATES, "1614834367\n1646370367\n"),
    (
        &[],
        "from_unix",
        ROOT_EPOCHS,
        "2020-09-13T12:26:40Z\n2023-11-14T22:13:20Z\n",
    ),
    // A replacement of a child keeps the child's document, and so its separator.
    (
        &[],
        ".t | format_datetime(\"2006\")",
        DATES,
        "2021\n---\n2022\n",
    ),
    (
        &[],
        ".t | tz(\"UTC\")",
        DATES,
        "2021-03-04T05:06:07Z\n---\n2022-03-04T05:06:07Z\n",
    ),
    (&[], ".t | to_unix", DATES, "1614834367\n---\n1646370367\n"),
    (
        &[],
        ".n | from_unix",
        DATES,
        "2020-09-13T12:26:40Z\n---\n2023-11-14T22:13:20Z\n",
    ),
    // `{...}` takes its input's document, and `.t` is a child of it.
    (
        &[],
        "{\"t\": now} | .t | format_datetime(\"2006\") | length",
        TWO,
        "4\n---\n4\n",
    ),
    // A branch that navigates the document keeps the separator before it.
    (
        &[],
        "now | format_datetime(\"2006\") | length, .a",
        DATES,
        "4\n1\n4\n---\n2\n",
    ),
    (&[], "now | length, .", TWO, "20\na: 1\n20\n---\na: 2\n"),
];

fn run(args: &[&str], filter: &str, stdin: &str) -> Result<(String, String, i32)> {
    use std::io::Write;
    let mut child = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .args(args)
        .arg(filter)
        .env("TZ", "UTC")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(stdin.as_bytes())?;
    let output = child.wait_with_output()?;
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?,
        output.status.code().unwrap_or(-1),
    ))
}

#[test]
fn a_datetime_replacement_keeps_its_inputs_document_8077() -> Result<()> {
    for (args, filter, stdin, want) in ROWS {
        let got = run(args, filter, stdin)?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (*want, "", 0),
            "{args:?} {filter} over {stdin:?}"
        );
    }
    Ok(())
}
