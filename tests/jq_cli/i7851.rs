//! #7851: a UTF-8 byte-order mark at the front of the stream that breaks off (`EF` in one file and
//! `EF` in the next, or `EF BF BD 31` in one) stops jq's read in the file it breaks off in: its
//! parser faults there with `Malformed BOM` and its main loop stops, so the files after it are never
//! opened. A missing file or a directory after it is not reported and does not make the run exit 2,
//! and the `(at FILE:0)` of `-n input` names the file the mark broke off in. `succinctly jq` read on
//! past it, reported the files that did not open, exited 2 and named the last file.
//!
//! The exit status, the standard output and the file named were captured from jq 1.7.1
//! (`/usr/bin/jq`); jq 1.8.2 agrees on every row. jq words the fault `Malformed BOM`, and so does
//! `succinctly jq` on the routes ADR-0033's reader model reads (`--stream`, and since step 5 of
//! #7987 the materializing route a program that reads `input` takes). The default route (`.`) and
//! `--slurp` still word it as a numeric literal, the divergence recorded in
//! docs/compliance/jq/limitations.md (#5804), so those rows pin succinctly's wording.

use anyhow::Result;

use crate::run_jq_in_dir;

/// `(flags, files, stderr, exit status)`; standard output is empty on every row. `-` is standard
/// input, which holds a mark and `[1]`.
type Row = (
    &'static [&'static str],
    &'static [&'static str],
    &'static str,
    i32,
);

/// The numeric-literal wording of `Malformed BOM` (#5804) at `column`, on the routes the reader
/// model has not reached.
macro_rules! literal {
    ($column:literal) => {
        concat!(
            "jq: parse error: Invalid numeric literal at EOF at line 1, column ",
            $column,
            "\n"
        )
    };
}

/// jq's `Malformed BOM` as `input` raises it, at `file`, line 0 (#7987 step 5).
macro_rules! malformed {
    ($file:literal) => {
        concat!("jq: error (at ", $file, ":0): Malformed BOM\n")
    };
}

static ROWS: &[Row] = &[
    // The issue's three commands.
    (&["-c", "."], &["ef", "ef", "gone"], literal!("2"), 5),
    (
        &["-c", "-n", "[inputs]"],
        &["ef", "ef", "empty"],
        malformed!("ef.json"),
        5,
    ),
    (
        &["-c", "--stream", "-s", "."],
        &["ef", "efbb", "adir"],
        "jq: parse error: Malformed BOM\n",
        5,
    ),
    // The whole-text routes, a directory and a file with bytes after the mark.
    (&["-c", "."], &["efbb", "ef", "adir"], literal!("3"), 5),
    (
        &["-c", "."],
        &["ef", "efbb", "empty", "one"],
        literal!("3"),
        5,
    ),
    (&["-c", "-s", "."], &["ef", "ef", "gone"], literal!("2"), 5),
    (
        &["-c", "-s", "."],
        &["efbb", "efbb", "adir"],
        literal!("4"),
        5,
    ),
    (
        &["-c", "--stream", "-s", "."],
        &["efbb", "ef", "gone"],
        "jq: parse error: Malformed BOM\n",
        5,
    ),
    // A mark that breaks off inside the first file.
    (&["-c", "."], &["efbad", "gone"], literal!("4"), 5),
    (
        &["-c", "-n", "input"],
        &["efbad", "one"],
        malformed!("efbad.json"),
        5,
    ),
    // `input` and `inputs` name the file the mark broke off in, past one with no bytes too.
    (
        &["-c", "-n", "input"],
        &["ef", "ef", "gone"],
        malformed!("ef.json"),
        5,
    ),
    (
        &["-c", "-n", "input"],
        &["efbb", "ef", "bf"],
        malformed!("ef.json"),
        5,
    ),
    (
        &["-c", "-n", "[inputs]"],
        &["ef", "efbb", "adir"],
        malformed!("efbb.json"),
        5,
    ),
    (
        &["-c", "-n", "[inputs]"],
        &["efbb", "efbb", "bbbf"],
        malformed!("efbb.json"),
        5,
    ),
    (
        &["-c", "-n", "[inputs]"],
        &["ef", "ef", "-"],
        malformed!("ef.json"),
        5,
    ),
    (
        &["-c", "-n", "[inputs]"],
        &["ef", "empty", "efbb", "one"],
        malformed!("efbb.json"),
        5,
    ),
    // A file that does not open before the file the mark breaks off in is reported, as before.
    (
        &["-c", "."],
        &["ef", "gone", "ef", "gone"],
        concat!(
            "jq: error: Could not open file gone.json: No such file or directory\n",
            literal!("2")
        ),
        2,
    ),
];

#[test]
fn a_mark_that_breaks_off_stops_the_read_in_the_file_it_breaks_off_in_7851() -> Result<()> {
    let dir = tempfile::tempdir()?;
    for (name, bytes) in [
        ("ef.json", &b"\xef"[..]),
        ("efbb.json", b"\xef\xbb"),
        ("efbad.json", b"\xef\xbf\xbd1"),
        ("bf.json", b"\xbf[3]\n"),
        ("bbbf.json", b"\xbb\xbf[2]\n"),
        ("one.json", b"[1]\n"),
        ("empty.json", b""),
    ] {
        std::fs::write(dir.path().join(name), bytes)?;
    }
    std::fs::create_dir(dir.path().join("adir.json"))?;
    for &(flags, names, stderr, status) in ROWS {
        let files: Vec<String> = names
            .iter()
            .map(|name| match *name {
                "-" => "-".to_owned(),
                name => format!("{name}.json"),
            })
            .collect();
        let mut args: Vec<&str> = flags.to_vec();
        args.extend(files.iter().map(String::as_str));
        let got = run_jq_in_dir(dir.path(), &args, Some("\u{feff}[1]\n"))?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            ("", stderr, status),
            "{flags:?} {names:?}"
        );
    }
    Ok(())
}

/// The routes that do not read with jq's JSON parser read on: `--seq` goes on past a parse fault and
/// `-R` has no mark to match, so the file after the mark is reached and reported, as in jq. `--stream`
/// stopped there already (#7603).
#[test]
fn the_routes_without_the_mark_read_on_past_it_7851() -> Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("ef.json"), b"\xef")?;
    const GONE: &str = "jq: error: Could not open file gone.json: No such file or directory\n";
    let rows: [(&[&str], &str, &str, i32); 3] = [
        (&["-c", "--seq", "."], "", GONE, 2),
        (&["-c", "-R", "."], "\"\u{fffd}\u{fffd}\"\n", GONE, 2),
        (
            &["-c", "--stream", "."],
            "",
            "jq: parse error: Malformed BOM\n",
            5,
        ),
    ];
    for (flags, stdout, stderr, status) in rows {
        let mut args = flags.to_vec();
        args.extend(["ef.json", "ef.json", "gone.json"]);
        let got = run_jq_in_dir(dir.path(), &args, None)?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (stdout, stderr, status),
            "{flags:?}"
        );
    }
    Ok(())
}
