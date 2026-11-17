//! #7341: `succinctly jq --seq` over several regular files reads a file that is not all plain
//! `RS value LF` records by a gate of its own, as a lone file is read, and the windows carry on after
//! its stretches; where its gate cannot carry on, the rest of that file and the files after it are read
//! whole. Every row is checked against the same files behind named pipes (the route that reads them
//! whole) and pinned to jq 1.7.1's own output: jq's reader counts the lines it warns at over the whole
//! stream, and names a runtime error's line in its file.

#![cfg(unix)]

use std::path::Path;
use std::process::Command;

use anyhow::Result;

use crate::fifo_feed::{self, Feed};

type Run = (String, String, i32);

/// A record cut short by the RS of the next one.
const CUT: &str = "\x1e{\"id\":-1,\"name\":\"cut\n";

/// Records jq's reader reads values from and warns of (#7322's), and a number cut short by the next
/// record's RS.
const MIXED: &str = "\x1e} 5\n\x1e1 2\n\x1e[1,]\n\x1e\"bu\"\n\x1en\n\x1e7";

/// A value jq reads and RFC 8259 does not.
const NAN: &str = "\x1enan\n";

/// A stretch that runs to the end of its file and on into the next one's first record.
const TAIL: &str = "\x1e[1,\n\x1e2 3\x1e";

/// Prints the values that are not one of the fixture's records.
const ODD: &[&str] = &[
    "--seq",
    "-c",
    "if type==\"object\" and has(\"id\") then empty else . end",
];

/// Raises an error at each number, so the line jq names for it is on stderr among the warnings.
const NUMBERS: &[&str] = &[
    "--seq",
    "if type==\"number\" then error(\"n\") else empty end",
];

fn records(range: std::ops::Range<usize>) -> String {
    range
        .map(|i| {
            format!(
                "\x1e{{\"id\":{i},\"name\":\"user {i}\",\"email\":\"user{i}@example.com\",\"tags\":[\"a\",\"b\",\"c\"],\"score\":{}}}\n",
                i % 1000
            )
        })
        .collect()
}

/// `succinctly jq ARGS NAMES...`, run in `dir`, fed by `feeds` (#7304).
fn jq(args: &[&str], dir: &Path, names: &[&str], feeds: &[Feed]) -> Result<Run> {
    let output = fifo_feed::output(
        Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .arg("jq")
            .args(args)
            .args(names)
            .current_dir(dir)
            .env("LC_ALL", "C"),
        feeds,
    )?;
    Ok((
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    ))
}

/// `succinctly jq ARGS` over `files` (each a name and its text) as regular files (the windowed route)
/// and behind named pipes of the same names (the route that reads them whole): the run, once the two
/// are checked to be one.
fn several(args: &[&str], files: &[(&str, String)]) -> Result<Run> {
    let total: usize = files.iter().map(|(_, text)| text.len()).sum();
    assert!(total > 4 * 1024 * 1024, "the files must be windowed");
    let dir = tempfile::tempdir()?;
    let (regular, piped) = (dir.path().join("f"), dir.path().join("p"));
    std::fs::create_dir(&regular)?;
    std::fs::create_dir(&piped)?;
    let names: Vec<&str> = files.iter().map(|(name, _)| *name).collect();
    let mut feeds = Vec::new();
    for (name, text) in files {
        std::fs::write(regular.join(name), text)?;
        feeds.push(Feed::new(piped.join(name), text.clone().into_bytes()));
    }
    let windowed = jq(args, &regular, &names, &[])?;
    let whole = jq(args, &piped, &names, &feeds)?;
    assert_eq!(windowed, whole, "{args:?}");
    Ok(windowed)
}

/// A file in the middle with stretches the windows carry on past (a cut record, faults, a `nan`): every
/// record of every file is read, and the warnings name the stream's lines.
#[test]
fn a_file_with_stretches_among_plain_files_is_read_by_its_own_gate_7341() -> Result<()> {
    let files = [
        ("a.seq", records(0..25_000)),
        (
            "b-mid.seq",
            format!(
                "{}{CUT}{}{MIXED}{}{NAN}{}",
                records(25_000..35_000),
                records(35_000..45_000),
                records(45_000..50_000),
                records(50_000..55_000)
            ),
        ),
        ("c.seq", records(55_000..80_000)),
    ];
    let warnings = [
        "jq: ignoring parse error: Truncated value at line 35002, column 1\n",
        "jq: ignoring parse error: Unmatched '}' at line 45002, column 2 (need RS to resync)\n",
        "jq: ignoring parse error: Expected another array element at line 45004, column 5 (need RS to resync)\n",
        "jq: ignoring parse error: Invalid literal at line 45007, column 0 (need RS to resync)\n",
        "jq: ignoring parse error: Potentially truncated top-level numeric value at line 45007, column 3\n",
    ];
    let (out, err, status) = several(ODD, &files)?;
    assert_eq!(out, "\x1e5\n\x1e1\n\x1e2\n\x1e\"bu\"\n\x1enull\n");
    assert_eq!((err, status), (warnings.concat(), 0));
    let (out, err, status) = several(NUMBERS, &files)?;
    let errors = "jq: error (at b-mid.seq:20002): n\njq: error (at b-mid.seq:20003): n\njq: error (at b-mid.seq:20003): n\n";
    assert_eq!(
        (out.as_str(), err, status),
        (
            "",
            [
                warnings[0],
                warnings[1],
                errors,
                warnings[2],
                warnings[3],
                warnings[4],
                "jq: error (at b-mid.seq:25007): n\n",
            ]
            .concat(),
            0
        )
    );
    let (out, _, _) = several(&["--seq", "-c", ".id?"], &files)?;
    assert_eq!(out.lines().count(), 80_000);
    Ok(())
}

/// Two files with stretches in a row: the second's gate counts the lines it warns at from the stream's
/// start, past the first gated file's lines (where the files after a gated one start was once taken to
/// be its start, which restarted the count).
#[test]
fn two_gated_files_in_a_row_count_the_streams_lines_7341() -> Result<()> {
    let files = [
        ("a.seq", records(0..25_000)),
        (
            "b-mid.seq",
            format!(
                "{}{CUT}{}{MIXED}{}{NAN}{}",
                records(25_000..35_000),
                records(35_000..45_000),
                records(45_000..50_000),
                records(50_000..55_000)
            ),
        ),
        (
            "b2.seq",
            format!(
                "{}{CUT}{}{NAN}{}",
                records(55_000..60_000),
                records(60_000..65_000),
                records(65_000..70_000)
            ),
        ),
        ("c2.seq", records(70_000..80_000)),
    ];
    let (out, err, status) = several(ODD, &files)?;
    assert_eq!(out, "\x1e5\n\x1e1\n\x1e2\n\x1e\"bu\"\n\x1enull\n\x1enull\n");
    assert_eq!(status, 0);
    assert!(
        err.ends_with(
            "at line 45007, column 3\njq: ignoring parse error: Truncated value at line 60009, column 1\n"
        ),
        "{err}"
    );
    let (_, err, _) = several(NUMBERS, &files)?;
    assert!(
        err.ends_with(
            "jq: ignoring parse error: Truncated value at line 60009, column 1\njq: error (at b2.seq:10002): n\n"
        ),
        "{err}"
    );
    Ok(())
}

/// A file whose last stretch runs on into the next file: its gate hands the rest of it on, from that
/// stretch, with the files after it, and the route that reads them whole reads across the files'
/// boundary as jq's reader does.
#[test]
fn a_stretch_into_the_next_file_hands_on_the_rest_of_its_file_7341() -> Result<()> {
    let files = [
        ("a.seq", records(0..25_000)),
        ("b-tail.seq", format!("{}{TAIL}", records(25_000..55_000))),
        ("c.seq", records(55_000..80_000)),
    ];
    let warning =
        "jq: ignoring parse error: Potentially truncated top-level numeric value at line 55002, column 5\n";
    let (out, err, status) = several(ODD, &files)?;
    assert_eq!(
        (out.as_str(), err.as_str(), status),
        ("\x1e2\n", warning, 0)
    );
    let (out, err, status) = several(NUMBERS, &files)?;
    assert_eq!(
        (out.as_str(), err, status),
        (
            "",
            format!("jq: error (at b-tail.seq:30001): n\n{warning}"),
            0
        )
    );
    let (out, _, _) = several(&["--seq", "-c", ".id?"], &files)?;
    assert_eq!(out.lines().count(), 80_000);
    Ok(())
}

/// The first file is the one that is not plain, and a file that does not open with an RS (whose first
/// bytes continue the record before it, which no gate can start in) ends the windows before it as it
/// did.
#[test]
fn a_first_file_with_a_stretch_and_one_that_does_not_open_with_an_rs_7341() -> Result<()> {
    let first = [
        ("b-first.seq", format!("{NAN}{}", records(25_000..55_000))),
        ("a.seq", records(0..25_000)),
        ("c.seq", records(55_000..80_000)),
    ];
    let (out, err, status) = several(ODD, &first)?;
    assert_eq!((out.as_str(), err.as_str(), status), ("\x1enull\n", "", 0));
    let (_, err, _) = several(NUMBERS, &first)?;
    assert_eq!(err, "jq: error (at b-first.seq:1): n\n");
    let no_rs = [
        ("a.seq", records(0..25_000)),
        ("b-nors.seq", format!("7\n{}", records(25_000..55_000))),
        ("c.seq", records(55_000..80_000)),
    ];
    let (out, err, status) = several(ODD, &no_rs)?;
    assert_eq!((out.as_str(), err.as_str(), status), ("\x1e7\n", "", 0));
    let (_, err, _) = several(NUMBERS, &no_rs)?;
    assert_eq!(err, "jq: error (at b-nors.seq:1): n\n");
    Ok(())
}
