//! #7260: `succinctly jq --seq` reads a lone regular file of 64 KiB or more a window at a time, where it
//! waited for 4 MiB before: a stream of under 4 MiB with a record that is not `RS value LF` was walked
//! whole by jq's reader and every record converted to an owned value, which made a 1 MiB stream with
//! one bad record slower than jq 1.7.1. Every row is checked against the same bytes read whole (behind
//! a named pipe, which the windowed routes decline), as a named file and as a regular file on standard
//! input, and pinned to jq 1.7.1's own output.

#![cfg(unix)]

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::Result;

use crate::fifo_feed::{self, Feed};

type Run = (String, String, i32);

/// About 184 KiB of records: over the 64 KiB a `--seq` file needs to be windowed, under the plain
/// route's 4 MiB.
const RECORDS: usize = 2_000;

/// A record cut short by the RS of the next one (the benchmark's `jsonseq-bad-first` shape).
const CUT: &str = "\x1e{\"id\":-1,\"name\":\"cut\n";

/// Records jq's reader reads values from and warns of (as in `i7322.rs`).
const MIXED: &str = "\x1e} 5\n\x1e1 2\n\x1e[1,]\n\x1e\"bu\"\n\x1en\n\x1e7";

/// A record jq reads a value from that is no RFC 8259 document, and a bare `n` behind it.
const NOT_JSON: &str = "\x1enan\n\x1en\n";

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

const IDS: &[&str] = &["--seq", "-c", ".id?"];

fn record(i: usize) -> String {
    format!(
        "\x1e{{\"id\":{i},\"name\":\"user {i}\",\"email\":\"user{i}@example.com\",\"tags\":[\"a\",\"b\",\"c\"],\"score\":{}}}\n",
        i % 1000
    )
}

fn records(range: std::ops::Range<usize>) -> String {
    range.map(record).collect()
}

fn shown(output: &std::process::Output, operand: Option<&std::path::Path>) -> Run {
    let name = |bytes: &[u8]| {
        let text = String::from_utf8_lossy(bytes).into_owned();
        match operand {
            Some(operand) => text.replace(&operand.display().to_string(), "<file>"),
            None => text,
        }
    };
    (
        name(&output.stdout),
        name(&output.stderr),
        output.status.code().unwrap_or(-1),
    )
}

fn command(args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_succinctly"));
    command.arg("jq").args(args).env("LC_ALL", "C");
    command
}

/// `succinctly jq ARGS` over `bytes` in a named file (the windowed route), as a regular file on standard
/// input (windowed too), and behind a named pipe (the route that reads it whole): the named file's run,
/// once the three are checked to be one (a runtime error names `<stdin>` for standard input, so that run
/// is compared with the file's under that name).
fn seq(args: &[&str], bytes: &[u8]) -> Result<Run> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("f.seq");
    std::fs::File::create(&path)?.write_all(bytes)?;
    let fifo = dir.path().join("p");
    let feed = Feed::new(fifo.clone(), bytes.to_vec());
    let windowed = shown(
        &fifo_feed::output(command(args).arg(&path), &[])?,
        Some(&path),
    );
    let whole = shown(
        &fifo_feed::output(command(args).arg(&fifo), std::slice::from_ref(&feed))?,
        Some(&fifo),
    );
    assert_eq!(windowed, whole, "{args:?}");
    let redirected = command(args)
        .stdin(Stdio::from(std::fs::File::open(&path)?))
        .output()?;
    let (out, err, status) = shown(&redirected, None);
    assert_eq!(
        (out, err.replace("<stdin>", "<file>"), status),
        windowed,
        "{args:?} < file"
    );
    Ok(windowed)
}

/// The benchmark's `seq_field_bad_first` shape under 4 MiB: jq 1.7.1 warns once, at the RS that cuts the
/// first record, and reads every record after it.
#[test]
fn a_seq_file_under_four_mib_whose_first_record_is_cut_reads_every_record_7260() -> Result<()> {
    let stream = format!("{CUT}{}", records(0..RECORDS)).into_bytes();
    assert!(stream.len() > 64 << 10 && stream.len() < 4 << 20);
    let warning = "jq: ignoring parse error: Truncated value at line 2, column 1\n";
    let (out, err, status) = seq(IDS, &stream)?;
    assert_eq!((err.as_str(), status), (warning, 0));
    assert_eq!(out.lines().count(), RECORDS);
    assert!(out.starts_with("\x1e0\n") && out.ends_with("\x1e1999\n"));
    assert_eq!(seq(ODD, &stream)?, (String::new(), warning.to_string(), 0));
    Ok(())
}

/// The `seq_field_bad_last` shape: the last record is cut short by the end of the stream, which jq 1.7.1
/// reports at EOF after every record before it.
#[test]
fn a_seq_file_under_four_mib_whose_last_record_is_cut_reads_every_record_7260() -> Result<()> {
    let stream = format!("{}{CUT}", records(0..RECORDS)).into_bytes();
    let warning = "jq: ignoring parse error: Unfinished string at EOF at line 2002, column 0\n";
    let (out, err, status) = seq(IDS, &stream)?;
    assert_eq!((err.as_str(), status), (warning, 0));
    assert_eq!(out.lines().count(), RECORDS);
    assert_eq!(seq(ODD, &stream)?, (String::new(), warning.to_string(), 0));
    Ok(())
}

/// Stretches in the middle of the stream: the values jq's reader finds in them are run, each on the line
/// jq names for it, and the warnings come between them where the reader met them. jq 1.7.1's output.
#[test]
fn a_seq_file_under_four_mib_carries_on_after_stretches_in_its_middle_7260() -> Result<()> {
    let stream = format!(
        "{}{CUT}{}{MIXED}{}{NOT_JSON}{}",
        records(0..700),
        records(700..1400),
        records(1400..1700),
        records(1700..RECORDS)
    )
    .into_bytes();
    let warnings = [
        "jq: ignoring parse error: Truncated value at line 702, column 1\n",
        "jq: ignoring parse error: Unmatched '}' at line 1402, column 2 (need RS to resync)\n",
        "jq: ignoring parse error: Expected another array element at line 1404, column 5 (need RS to resync)\n",
        "jq: ignoring parse error: Invalid literal at line 1407, column 0 (need RS to resync)\n",
        "jq: ignoring parse error: Potentially truncated top-level numeric value at line 1407, column 3\n",
        "jq: ignoring parse error: Invalid numeric literal at line 1709, column 0 (need RS to resync)\n",
    ];
    let (out, err, status) = seq(ODD, &stream)?;
    assert_eq!(out, "\x1e5\n\x1e1\n\x1e2\n\x1e\"bu\"\n\x1enull\n");
    assert_eq!((err, status), (warnings.concat(), 0));
    let (out, err, status) = seq(NUMBERS, &stream)?;
    let error = |line: usize| format!("jq: error (at <file>:{line}): n\n");
    let expected = [
        warnings[0].to_string(),
        warnings[1].to_string(),
        error(1402),
        error(1403),
        error(1403),
        warnings[2].to_string(),
        warnings[3].to_string(),
        warnings[4].to_string(),
        error(1707),
        warnings[5].to_string(),
    ]
    .concat();
    assert_eq!((out.as_str(), err, status), ("", expected, 0));
    let (out, _, _) = seq(IDS, &stream)?;
    assert_eq!(out.lines().count(), RECORDS);
    Ok(())
}

/// A stream of plain records, one with no RS at all (jq 1.7.1 reads nothing from it and warns at EOF),
/// and a stream just under the 64 KiB a `--seq` file needs to be windowed, which keeps the route that
/// reads it whole: each reads as the same bytes read whole do.
#[test]
fn seq_files_either_side_of_the_window_minimum_read_as_read_whole_7260() -> Result<()> {
    let clean = records(0..RECORDS).into_bytes();
    let (out, err, status) = seq(IDS, &clean)?;
    assert_eq!(
        (err.as_str(), status, out.lines().count()),
        ("", 0, RECORDS)
    );
    let bare = records(0..RECORDS).replace('\x1e', "").into_bytes();
    let warning =
        "jq: ignoring parse error: Unfinished abandoned text at EOF at line 2001, column 0\n";
    assert_eq!(seq(IDS, &bare)?, (String::new(), warning.to_string(), 0));
    let mut small = CUT.to_string();
    let mut n = 0;
    while small.len() + record(n).len() < 64 << 10 {
        small.push_str(&record(n));
        n += 1;
    }
    let (out, err, status) = seq(IDS, small.as_bytes())?;
    assert_eq!(
        (err.as_str(), status, out.lines().count()),
        (
            "jq: ignoring parse error: Truncated value at line 2, column 1\n",
            0,
            n
        )
    );
    Ok(())
}

/// `[.id, .name]` and `keys` (the guard's array and key rows, which the whole-file route answered from
/// the owned record) over a windowed stream cut at both ends: each record's answer, and both warnings,
/// as jq 1.7.1 gives them and as the same bytes read whole give them.
#[test]
fn field_arrays_and_keys_of_a_windowed_seq_file_read_as_read_whole_7260() -> Result<()> {
    let stream = format!("{CUT}{}{CUT}", records(0..RECORDS)).into_bytes();
    let warnings = concat!(
        "jq: ignoring parse error: Truncated value at line 2, column 1\n",
        "jq: ignoring parse error: Unfinished string at EOF at line 2003, column 0\n",
    );
    let (out, err, status) = seq(&["--seq", "-c", "[.id,.name]"], &stream)?;
    assert_eq!((err.as_str(), status), (warnings, 0));
    let expected: String = (0..RECORDS)
        .map(|i| format!("\x1e[{i},\"user {i}\"]\n"))
        .collect();
    assert_eq!(out, expected);
    let (out, err, status) = seq(&["--seq", "-c", "keys"], &stream)?;
    assert_eq!((err.as_str(), status), (warnings, 0));
    assert_eq!(
        out,
        "\x1e[\"email\",\"id\",\"name\",\"score\",\"tags\"]\n".repeat(RECORDS)
    );
    Ok(())
}

/// A stretch that runs to the stream's end and ends at an RS with a value pending: jq's reader meets
/// that RS as the end of a truncated value, so the bare number after it is read on to the end of the
/// stream, where jq warns that it may be truncated. The tail handed on at that RS used to read it as an
/// RS with nothing pending, which in jq's last read ends the stream (#2998), and the warning was lost:
/// a file of 4 MiB or more did that before the windows were taken from 64 KiB. jq 1.7.1's output.
#[test]
fn a_seq_file_whose_stretch_ends_at_its_last_rs_reads_on_to_its_end_7260() -> Result<()> {
    let truncated = |line: usize| {
        format!("jq: ignoring parse error: Truncated value at line {line}, column 1\n")
    };
    let at_eof = |line: usize, column: usize| {
        format!("jq: ignoring parse error: Potentially truncated top-level numeric value at EOF at line {line}, column {column}\n")
    };
    let rows: [(&str, &str, String); 3] = [
        ("\x1e{\"\\\n\x1enan", "", truncated(2002) + &at_eof(2002, 4)),
        (
            "\x1e{\"\\\n\x1e\x1e7",
            "",
            truncated(2002) + &at_eof(2002, 3),
        ),
        (
            "\x1e1 2\n\x1e{\"\\\n\x1e1",
            "\x1e1\n\x1e2\n",
            truncated(2003) + &at_eof(2003, 2),
        ),
    ];
    for (tail, out, err) in rows {
        let stream = format!("{}{tail}", records(0..RECORDS)).into_bytes();
        assert_eq!(seq(ODD, &stream)?, (out.to_string(), err, 0), "{tail:?}");
    }
    Ok(())
}

/// #7404: the same tails at the end of the last file of a `--seq` list of 4 MiB or more between them,
/// whose own gate hands the rest of the file on at that RS: the list's tail is read as jq's reader reads
/// the files as one stream, on past the RS to the end. jq 1.7.1's output.
#[test]
fn a_seq_list_whose_last_file_ends_a_stretch_at_its_last_rs_reads_on_to_its_end_7404() -> Result<()>
{
    let truncated = |line: usize| {
        format!("jq: ignoring parse error: Truncated value at line {line}, column 1\n")
    };
    let at_eof = |line: usize, column: usize| {
        format!("jq: ignoring parse error: Potentially truncated top-level numeric value at EOF at line {line}, column {column}\n")
    };
    let dir = tempfile::tempdir()?;
    let first = dir.path().join("a.seq");
    let plain = records(0..46_000);
    assert!(plain.len() > 4 << 20);
    std::fs::write(&first, plain)?;
    let rows: [(&str, &str, String); 3] = [
        (
            "\x1e{\"\\\n\x1enan",
            "",
            truncated(46012) + &at_eof(46012, 4),
        ),
        (
            "\x1e{\"\\\n\x1e\x1e7",
            "",
            truncated(46012) + &at_eof(46012, 3),
        ),
        (
            "\x1e1 2\n\x1e{\"\\\n\x1e1",
            "\x1e1\n\x1e2\n",
            truncated(46013) + &at_eof(46013, 2),
        ),
    ];
    for (tail, out, err) in rows {
        let last = dir.path().join("b.seq");
        std::fs::write(&last, format!("{}{tail}", records(0..10)))?;
        let output = command(ODD).arg(&first).arg(&last).output()?;
        assert_eq!(shown(&output, None), (out.to_string(), err, 0), "{tail:?}");
    }
    Ok(())
}
