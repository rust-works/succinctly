//! #7322: `succinctly jq --seq` over one regular file of 4 MiB or more, or a pipe, goes on reading a
//! window at a time after a record that is not `RS value LF` when plain records follow it: the stretch
//! is read as jq's `--seq` reader reads it, its values run and its warnings are printed where that
//! reader met them, and the rest of the stream is not held. Every row is checked against the same
//! bytes read whole (behind a named pipe, which the windowed routes decline, or on a pipe named `-`),
//! and pinned to jq 1.7.1's own output.

#![cfg(unix)]

use std::io::Write;
use std::process::Command;

use anyhow::Result;

use crate::fifo_feed::{self, Feed};

type Run = (String, String, i32);

/// How many records the fixtures have: about 6.5 MiB, well over the 4 MiB a file needs to be windowed.
const RECORDS: usize = 60_000;

/// A record cut short by the RS of the next one (the benchmark's `json-seq-bad-first` shape).
const CUT: &str = "\x1e{\"id\":-1,\"name\":\"cut\n";

/// Records jq's reader reads values from and warns of: a value after a fault, two values in one
/// record, a fault alone, a plain record between them, a bare `n` (worded by the `u` the string
/// before it left in jq's token buffer), and a number cut short by the next record's RS, which takes
/// that record with it as far as the windows go.
const MIXED: &str = "\x1e} 5\n\x1e1 2\n\x1e[1,]\n\x1e\"bu\"\n\x1en\n\x1e7";

/// A record jq reads a value from that is no RFC 8259 document, which the windows do not carry on
/// after, and a bare `n` behind it, read by the route that walks the rest whole.
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

/// `succinctly jq ARGS OPERAND` with stdin null, fed by `feeds` and held to their deadline (#7304).
fn jq(args: &[&str], operand: &std::path::Path, feeds: &[Feed]) -> Result<Run> {
    let output = fifo_feed::output(
        Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .arg("jq")
            .args(args)
            .arg(operand)
            .env("LC_ALL", "C"),
        feeds,
    )?;
    Ok(shown(&output, Some(operand)))
}

/// `succinctly jq ARGS FILE` over `bytes` in a named file (the windowed route) and behind a named pipe
/// (the route that reads it whole): the run, once the two are checked to be one.
fn named(args: &[&str], bytes: &[u8]) -> Result<Run> {
    assert!(bytes.len() > 4 * 1024 * 1024, "the file must be windowed");
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("f.seq");
    std::fs::File::create(&path)?.write_all(bytes)?;
    let fifo = dir.path().join("p");
    let feed = Feed::new(fifo.clone(), bytes.to_vec());
    let windowed = jq(args, &path, &[])?;
    let whole = jq(args, &fifo, std::slice::from_ref(&feed))?;
    assert_eq!(windowed, whole, "{args:?}");
    Ok(windowed)
}

/// `succinctly jq ARGS` over `bytes` on a pipe on standard input (windowed, #7164) and on a pipe named
/// `-` (the route that reads it whole).
fn piped(args: &[&str], bytes: &[u8]) -> Result<Run> {
    let run = |dash: bool| -> Result<Run> {
        let mut command = Command::new(env!("CARGO_BIN_EXE_succinctly"));
        command.arg("jq").args(args).env("LC_ALL", "C");
        if dash {
            command.arg("-");
        }
        Ok(shown(&fifo_feed::output_stdin(&mut command, bytes)?, None))
    };
    let windowed = run(false)?;
    assert_eq!(windowed, run(true)?, "{args:?}");
    Ok(windowed)
}

/// The benchmark's shape: the first record is cut short, and every record after it is plain. jq 1.7.1
/// warns once, at the RS that cuts it, and reads every record.
#[test]
fn a_seq_stream_whose_first_record_is_cut_reads_every_record_7322() -> Result<()> {
    let stream = format!("{CUT}{}", records(0..RECORDS)).into_bytes();
    let warning = "jq: ignoring parse error: Truncated value at line 2, column 1\n";
    for run in [named, piped] {
        let (out, err, status) = run(&["--seq", "-c", ".id?"], &stream)?;
        assert_eq!((err.as_str(), status), (warning, 0));
        assert_eq!(out.lines().count(), RECORDS);
        assert!(out.starts_with("\x1e0\n") && out.ends_with("\x1e59999\n"));
        let (out, err, status) = run(ODD, &stream)?;
        assert_eq!((out.as_str(), err.as_str(), status), ("", warning, 0));
    }
    Ok(())
}

/// Stretches in the middle of the stream, twice: the values jq's reader finds in them are run, each
/// on the line jq names for it, and its warnings come between them where the reader met them. The
/// bare `n` is "Invalid literal" for the `u` of the plain `"bu"` before it, and the number cut short
/// by an RS is a warning. jq 1.7.1's output.
#[test]
fn a_seq_stream_carries_on_after_stretches_in_its_middle_7322() -> Result<()> {
    let stream = format!(
        "{}{CUT}{}{MIXED}{}",
        records(0..20_000),
        records(20_000..40_000),
        records(40_000..RECORDS)
    )
    .into_bytes();
    let warnings = [
        "jq: ignoring parse error: Truncated value at line 20002, column 1\n",
        "jq: ignoring parse error: Unmatched '}' at line 40002, column 2 (need RS to resync)\n",
        "jq: ignoring parse error: Expected another array element at line 40004, column 5 (need RS to resync)\n",
        "jq: ignoring parse error: Invalid literal at line 40007, column 0 (need RS to resync)\n",
        "jq: ignoring parse error: Potentially truncated top-level numeric value at line 40007, column 3\n",
    ];
    for (run, name) in [(named as fn(_, _) -> _, "<file>"), (piped, "<stdin>")] {
        let (out, err, status) = run(ODD, &stream)?;
        assert_eq!(out, "\x1e5\n\x1e1\n\x1e2\n\x1e\"bu\"\n");
        assert_eq!((err, status), (warnings.concat(), 0));
        let (out, err, status) = run(NUMBERS, &stream)?;
        let errors = format!(
            "jq: error (at {name}:40002): n\njq: error (at {name}:40003): n\njq: error (at {name}:40003): n\n"
        );
        assert_eq!(
            (out.as_str(), err, status),
            (
                "",
                [
                    warnings[0],
                    warnings[1],
                    errors.as_str(),
                    warnings[2],
                    warnings[3],
                    warnings[4]
                ]
                .concat(),
                0
            )
        );
        // The number's RS opens a plain record, which the stretch takes with it and still reads.
        let (out, _, _) = run(&["--seq", "-c", ".id?"], &stream)?;
        assert_eq!(out.lines().count(), RECORDS);
    }
    Ok(())
}

/// A stretch the windows carry on after, and later one they do not (`nan` is a value to jq and no
/// RFC 8259 document): the rest is read by the route that walks it whole, from where jq's reader
/// stood after the stretches before it, so its lines count them and its bare `n` is worded by the
/// last token ("Invalid numeric literal" after `nan`). jq 1.7.1's output.
#[test]
fn a_seq_stream_is_handed_on_after_stretches_it_carried_on_past_7322() -> Result<()> {
    let stream = format!(
        "{CUT}{}{MIXED}{}{NOT_JSON}{}",
        records(0..30_000),
        records(30_000..45_000),
        records(45_000..RECORDS)
    )
    .into_bytes();
    for (run, name) in [(named as fn(_, _) -> _, "<file>"), (piped, "<stdin>")] {
        let (out, err, status) = run(ODD, &stream)?;
        assert_eq!(out, "\x1e5\n\x1e1\n\x1e2\n\x1e\"bu\"\n\x1enull\n");
        assert_eq!(
            (err.as_str(), status),
            (
                "jq: ignoring parse error: Truncated value at line 2, column 1\n\
                 jq: ignoring parse error: Unmatched '}' at line 30002, column 2 (need RS to resync)\n\
                 jq: ignoring parse error: Expected another array element at line 30004, column 5 (need RS to resync)\n\
                 jq: ignoring parse error: Invalid literal at line 30007, column 0 (need RS to resync)\n\
                 jq: ignoring parse error: Potentially truncated top-level numeric value at line 30007, column 3\n\
                 jq: ignoring parse error: Invalid numeric literal at line 45009, column 0 (need RS to resync)\n",
                0
            )
        );
        let (_, err, status) = run(NUMBERS, &stream)?;
        assert_eq!(status, 0);
        assert!(
            err.ends_with(&format!(
                "jq: error (at {name}:30003): n\n\
                 jq: ignoring parse error: Expected another array element at line 30004, column 5 (need RS to resync)\n\
                 jq: ignoring parse error: Invalid literal at line 30007, column 0 (need RS to resync)\n\
                 jq: ignoring parse error: Potentially truncated top-level numeric value at line 30007, column 3\n\
                 jq: error (at {name}:45007): n\n\
                 jq: ignoring parse error: Invalid numeric literal at line 45009, column 0 (need RS to resync)\n"
            )),
            "{err}"
        );
    }
    Ok(())
}

/// A run that stops at a value in a stretch prints nothing the reader met after it: `halt_error` at
/// the stretch's first value leaves the warnings after it unsaid, as in jq 1.7.1.
#[test]
fn a_run_that_halts_in_a_stretch_prints_no_later_warning_7322() -> Result<()> {
    let stream = format!("{}{MIXED}{}", records(0..30_000), records(30_000..RECORDS)).into_bytes();
    let (out, err, status) = named(
        &[
            "--seq",
            "if . == 5 then \"stop\\n\" | halt_error else empty end",
        ],
        &stream,
    )?;
    assert_eq!(
        (out.as_str(), err.as_str(), status),
        (
            "",
            "jq: ignoring parse error: Unmatched '}' at line 30001, column 2 (need RS to resync)\nstop\n",
            5
        )
    );
    Ok(())
}
