//! #7164: `succinctly jq --seq` over a pipe on standard input that holds 4 MiB or more is read a window at a
//! time while its records are RFC 7464's own framing (`RS value LF`), and the rest of the stream, from the
//! first record that is not, is read whole by the route that walks jq's reader, resumed where that reader
//! stood. Every row is checked against the same bytes read whole: the pipe named as `-`, which the windowed
//! routes decline, so stdout, stderr and the exit status are the whole-stream route's. (Not `/dev/stdin`,
//! as #7024's pipe rows use: that name reads a `--seq` stream that starts with a BOM as no records at all.) The rows where the stream turns at a record whose wording depends on what came
//! before it (a bare `n` after a token that left a `u` in jq's token buffer, an RS in the final buffer) are
//! also pinned to jq 1.7.1's own output.
//!
//! In a debug build the fixtures are a sixteenth of the size and the binary's window is 64 KiB
//! (`small_window`, #7376); a release build runs the same rows over the 1 MiB window. The lines jq's
//! messages name are the fixture's, so they are made from [`RECORDS`].

#![cfg(unix)]

use anyhow::Result;

use crate::small_window::{self, scaled, spawn_jq as spawn_jq_deadline};

/// How many records the fixtures have: about six windows (6.5 MiB on the shipped one), well over the
/// four a pipe needs to be windowed.
const RECORDS: usize = scaled(60_000);

fn record(i: usize) -> String {
    format!(
        "\x1e{{\"id\":{i},\"name\":\"user {i}\",\"email\":\"user{i}@example.com\",\"tags\":[\"a\",\"b\",\"c\"],\"score\":{}}}\n",
        i % 1000
    )
}

fn records(range: std::ops::Range<usize>) -> String {
    range.map(record).collect()
}

/// `succinctly jq ARGS` over `bytes` on a pipe, read the windowed way and the whole way: the stdout,
/// stderr and exit status of the windowed run, once they are checked to be the whole run's.
fn same(args: &[&str], bytes: &[u8]) -> Result<(String, String, i32)> {
    assert!(
        bytes.len() > small_window::MIN_BYTES,
        "the pipe must hold enough to be windowed"
    );
    let (windowed, windowed_status) = spawn_jq_deadline(args, Some(bytes))?;
    let mut whole_args = args.to_vec();
    whole_args.push("-");
    let (whole, whole_status) = spawn_jq_deadline(&whole_args, Some(bytes))?;
    let whole_err = String::from_utf8_lossy(&whole.stderr).into_owned();
    let err = String::from_utf8_lossy(&windowed.stderr).into_owned();
    assert!(
        windowed.stdout == whole.stdout,
        "{args:?}: stdout differs ({} and {} bytes)",
        windowed.stdout.len(),
        whole.stdout.len()
    );
    assert_eq!(err, whole_err, "{args:?}: stderr");
    assert_eq!(windowed_status, whole_status, "{args:?}: exit status");
    Ok((
        String::from_utf8_lossy(&windowed.stdout).into_owned(),
        err,
        windowed_status,
    ))
}

/// The programs: the raw-bytes fast path, a field, a runtime error at the record in the middle, and
/// `-e` over the last record.
fn programs() -> [Vec<String>; 4] {
    let args = |args: &[&str]| args.iter().map(|&arg| arg.to_owned()).collect::<Vec<_>>();
    [
        args(&["--seq", "-c", "."]),
        args(&["--seq", "-c", ".id?"]),
        args(&[
            "--seq",
            "-c",
            &format!(
                "if .id? == {} then error(\"boom\") else empty end",
                RECORDS / 2
            ),
        ]),
        args(&[
            "--seq",
            "-e",
            &format!("select(.id? == {}) | .name", RECORDS - 1),
        ]),
    ]
}

fn strs(args: &[String]) -> Vec<&str> {
    args.iter().map(String::as_str).collect()
}

#[test]
fn a_clean_seq_pipe_reads_as_the_whole_stream_does_7164() -> Result<()> {
    let clean = records(0..RECORDS).into_bytes();
    let programs = programs();
    for args in &programs {
        same(&strs(args), &clean)?;
    }
    let (_, err, status) = same(&strs(&programs[2]), &clean)?;
    assert_eq!(
        (err, status),
        (
            format!("jq: error (at <stdin>:{}): boom\n", RECORDS / 2 + 1),
            0
        )
    );
    let (out, _, status) = same(&strs(&programs[3]), &clean)?;
    assert_eq!(
        (out, status),
        (format!("\x1e\"user {}\"\n", RECORDS - 1), 0)
    );
    Ok(())
}

/// A stream that stops being plain part-way is the whole-stream route's from there: its warnings name
/// the lines and columns of the whole stream, a runtime error after the turn names the line its value
/// was read on, and the records after it are read.
#[test]
fn a_seq_pipe_that_turns_is_read_on_as_the_whole_stream_7164() -> Result<()> {
    let head = records(0..RECORDS / 2);
    let rest = records(RECORDS / 2..RECORDS);
    let programs = programs();
    for turn in [
        "\x1e{\"a\":1\n",
        "\x1e[1,]\n",
        "\x1enan\n",
        "\x1e1 2\n",
        "\x1e\x1e",
        "garbage\n",
        "\x1e\"open\n",
        "\x1e{}\n\n",
    ] {
        let stream = format!("{head}{turn}{rest}").into_bytes();
        // The raw-bytes fast path, and the general one with the line of a runtime error.
        for args in [&programs[0], &programs[2]] {
            same(&strs(args), &stream)?;
        }
    }
    let stream = format!("{head}\x1e[1,]\n{rest}").into_bytes();
    let (_, err, _) = same(&strs(&programs[2]), &stream)?;
    assert_eq!(
        err,
        format!(
            "jq: ignoring parse error: Expected another array element at line {}, column 5 (need RS to resync)\n\
             jq: error (at <stdin>:{}): boom\n",
            RECORDS / 2 + 1,
            RECORDS / 2 + 2
        )
    );
    Ok(())
}

/// What the records before the turn leave in jq's reader decides how the turn reads: a bare `n` after a
/// token whose second byte is `u` takes the keyword path ("Invalid literal"), and the number path after
/// any other; a stream that ends in an RS straight after its last record ends there (#2998), with the
/// record after it dropped. jq 1.7.1's output for each.
#[test]
fn a_seq_pipe_turns_with_what_its_records_left_7164() -> Result<()> {
    let head = records(0..RECORDS);
    // The line of the bare `n`: the records, the record before it, and its own.
    let line = RECORDS + 3;
    let literal = format!(
        "jq: ignoring parse error: Invalid literal at line {line}, column 0 (need RS to resync)\n"
    );
    let numeric = format!(
        "jq: ignoring parse error: Invalid numeric literal at line {line}, column 0 (need RS to resync)\n"
    );
    let cases: [(&str, &str, &str); 4] = [
        ("\x1e[\"x\\u0075\"]\n\x1en\n", "\x1e[\"xu\"]\n", &literal),
        ("\x1enull\n\x1en\n", "\x1enull\n", &literal),
        ("\x1e7\n\x1en\n", "\x1e7\n", &numeric),
        ("\x1e12.5", "", ""),
    ];
    for (turn, last, err) in cases {
        let stream = format!("{head}{turn}").into_bytes();
        let (out, got_err, status) = same(&["--seq", "-c", "."], &stream)?;
        assert!(
            out.ends_with(&format!(
                "\"tags\":[\"a\",\"b\",\"c\"],\"score\":{}}}\n{last}",
                (RECORDS - 1) % 1000
            )),
            "{turn:?}"
        );
        assert_eq!((got_err.as_str(), status), (err, 0), "{turn:?}");
    }
    Ok(())
}

/// A stream with nothing plain before its first byte that is not a record's (a BOM, text before the first
/// RS) is the whole-stream route's from its first byte; `-e` answers for the last value whichever route
/// read it; and a run that halts before the turn reads no further.
#[test]
fn a_seq_pipe_that_never_turns_plain_or_stops_first_7164() -> Result<()> {
    let clean = records(0..RECORDS);
    for front in ["\u{feff}", "x\n", "\n"] {
        let stream = format!("{front}{clean}").into_bytes();
        same(&strs(&programs()[1]), &stream)?;
    }
    // The last value is in the windows, and the rest reads none.
    let stream = format!("{clean}\x1efalse\n\x1e{{\"a\"\n").into_bytes();
    let (_, _, status) = same(&["--seq", "-e", "."], &stream)?;
    assert_eq!(status, 1);
    let stream = format!("{clean}\x1e{{\"a\"\n").into_bytes();
    let (_, err, status) = same(
        &[
            "--seq",
            "-c",
            "if .id == 100 then halt_error else empty end",
        ],
        &stream,
    )?;
    assert_eq!((err.as_str(), status), ("{\"id\":100,\"name\":\"user 100\",\"email\":\"user100@example.com\",\"tags\":[\"a\",\"b\",\"c\"],\"score\":100}\n", 5));
    Ok(())
}
