//! #7164: `succinctly jq --seq` over a pipe on standard input that holds 4 MiB or more is read a window at a
//! time while its records are RFC 7464's own framing (`RS value LF`), and the rest of the stream, from the
//! first record that is not, is read whole by the route that walks jq's reader, resumed where that reader
//! stood. Every row is checked against the same bytes read whole: the pipe named as `-`, which the windowed
//! routes decline, so stdout, stderr and the exit status are the whole-stream route's. (Not `/dev/stdin`,
//! as #7024's pipe rows use: that name reads a `--seq` stream that starts with a BOM as no records at all.) The rows where the stream turns at a record whose wording depends on what came
//! before it (a bare `n` after a token that left a `u` in jq's token buffer, an RS in the final buffer) are
//! also pinned to jq 1.7.1's own output.

#![cfg(unix)]

use anyhow::Result;

use crate::spawn_jq;

/// How many records the fixtures have: about 6.5 MiB, well over the 4 MiB a pipe needs to be windowed.
const RECORDS: usize = 60_000;

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
        bytes.len() > 4 * 1024 * 1024,
        "the pipe must hold enough to be windowed"
    );
    let (windowed, windowed_status) = spawn_jq(args, Some(bytes))?;
    let mut whole_args = args.to_vec();
    whole_args.push("-");
    let (whole, whole_status) = spawn_jq(&whole_args, Some(bytes))?;
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

const PROGRAMS: [&[&str]; 4] = [
    &["--seq", "-c", "."],
    &["--seq", "-c", ".id?"],
    &[
        "--seq",
        "-c",
        "if .id? == 30000 then error(\"boom\") else empty end",
    ],
    &["--seq", "-e", "select(.id? == 59999) | .name"],
];

#[test]
fn a_clean_seq_pipe_reads_as_the_whole_stream_does_7164() -> Result<()> {
    let clean = records(0..RECORDS).into_bytes();
    for args in PROGRAMS {
        same(args, &clean)?;
    }
    let (_, err, status) = same(PROGRAMS[2], &clean)?;
    assert_eq!(
        (err.as_str(), status),
        ("jq: error (at <stdin>:30001): boom\n", 0)
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
        for args in [PROGRAMS[0], PROGRAMS[2]] {
            same(args, &stream)?;
        }
    }
    let stream = format!("{head}\x1e[1,]\n{rest}").into_bytes();
    let (_, err, _) = same(PROGRAMS[2], &stream)?;
    assert_eq!(
        err,
        "jq: ignoring parse error: Expected another array element at line 30001, column 5 (need RS to resync)\n\
         jq: error (at <stdin>:30002): boom\n"
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
    let cases: [(&str, &str, &str); 4] = [
        (
            "\x1e[\"x\\u0075\"]\n\x1en\n",
            "\x1e[\"xu\"]\n",
            "jq: ignoring parse error: Invalid literal at line 60003, column 0 (need RS to resync)\n",
        ),
        (
            "\x1enull\n\x1en\n",
            "\x1enull\n",
            "jq: ignoring parse error: Invalid literal at line 60003, column 0 (need RS to resync)\n",
        ),
        (
            "\x1e7\n\x1en\n",
            "\x1e7\n",
            "jq: ignoring parse error: Invalid numeric literal at line 60003, column 0 (need RS to resync)\n",
        ),
        ("\x1e12.5", "", ""),
    ];
    for (turn, last, err) in cases {
        let stream = format!("{head}{turn}").into_bytes();
        let (out, got_err, status) = same(&["--seq", "-c", "."], &stream)?;
        assert!(
            out.ends_with(&format!(
                "\"tags\":[\"a\",\"b\",\"c\"],\"score\":999}}\n{last}"
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
        same(PROGRAMS[1], &stream)?;
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
