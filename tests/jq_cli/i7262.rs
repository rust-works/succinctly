//! #7262: `succinctly jq --seq` over one regular file of 4 MiB or more (named, or on standard input) that
//! is not all `RS value LF` records is read as a `--seq` pipe is (#7164): a window at a time up to its
//! first record that is not, and from there by the route that walks jq's reader, resumed where that reader
//! stood. Every row is checked against the same bytes read whole: a named file against the same bytes
//! behind a named pipe (which the windowed routes decline), with each operand's name made one, and a file
//! on standard input against the bytes from where it stands on a pipe named `-`. The rows the turn is
//! worded by what came before it are pinned to jq 1.7.1's own output as well.

#![cfg(unix)]

use std::io::{Seek, SeekFrom, Write};
use std::process::{Command, Stdio};

use anyhow::Result;

use crate::fifo_feed::{self, Feed};

type Run = (String, String, i32);

/// How many records the fixtures have: about 6.5 MiB, well over the 4 MiB a file needs to be windowed.
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
    let name = |bytes: &[u8]| {
        String::from_utf8_lossy(bytes).replace(&operand.display().to_string(), "<file>")
    };
    Ok((
        name(&output.stdout),
        name(&output.stderr),
        output.status.code().unwrap_or(-1),
    ))
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

/// `succinctly jq ARGS` over `bytes` in a regular file on standard input, from `offset` (the windowed
/// route), and over the bytes from there on a pipe named `-` (the route that reads it whole).
fn on_stdin(args: &[&str], bytes: &[u8], offset: u64) -> Result<Run> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("f.seq");
    std::fs::File::create(&path)?.write_all(bytes)?;
    let mut file = std::fs::File::open(&path)?;
    file.seek(SeekFrom::Start(offset))?;
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("jq")
        .args(args)
        .stdin(file)
        .env("LC_ALL", "C")
        .output()?;
    let windowed = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("jq")
        .args(args)
        .arg("-")
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut pipe = child.stdin.take().expect("piped");
    let rest = bytes[offset as usize..].to_vec();
    let writer = std::thread::spawn(move || {
        let _ = pipe.write_all(&rest);
    });
    let output = child.wait_with_output()?;
    writer.join().expect("the writer ends");
    let whole = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    );
    assert_eq!(windowed, whole, "{args:?} from {offset}");
    Ok(windowed)
}

const PROGRAMS: [&[&str]; 3] = [
    &["--seq", "-c", "."],
    &["--seq", "-c", ".id?"],
    &[
        "--seq",
        "-c",
        "if .id? == 30000 then error(\"boom\") else empty end",
    ],
];

/// A file that stops being plain part-way is the whole-stream route's from there: its warnings name the
/// lines and columns of the whole file, a runtime error after the turn names the line its value was read
/// on, and the records after it are read.
#[test]
fn a_seq_file_that_turns_is_read_on_as_the_whole_file_7262() -> Result<()> {
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
        named(PROGRAMS[0], &stream)?;
        named(PROGRAMS[2], &stream)?;
        on_stdin(PROGRAMS[2], &stream, 0)?;
    }
    let stream = format!("{head}\x1e[1,]\n{rest}").into_bytes();
    let expected = |name: &str| {
        format!(
            "jq: ignoring parse error: Expected another array element at line 30001, column 5 (need RS to resync)\n\
             jq: error (at {name}:30002): boom\n"
        )
    };
    let (_, err, status) = named(PROGRAMS[2], &stream)?;
    assert_eq!((err, status), (expected("<file>"), 0));
    let (_, err, _) = on_stdin(PROGRAMS[2], &stream, 0)?;
    assert_eq!(err, expected("<stdin>"));
    Ok(())
}

/// The benchmark's shapes (`json-seq-bad-first`, `json-seq-bad-last`): a record cut short by the next RS
/// at the front, and one cut short by the end of the file at the back, as jq 1.7.1 words them.
#[test]
fn a_seq_file_with_a_cut_record_first_or_last_7262() -> Result<()> {
    let cut = "\x1e{\"id\":-1,\"name\":\"cut\n";
    let clean = records(0..RECORDS);
    let first = format!("{cut}{clean}").into_bytes();
    let (out, err, status) = named(&["--seq", "-r", ".name"], &first)?;
    assert_eq!(
        (err.as_str(), status),
        (
            "jq: ignoring parse error: Truncated value at line 2, column 1\n",
            0
        )
    );
    assert!(out.starts_with("user 0\n") && out.ends_with("user 59999\n"));
    let last = format!("{clean}{cut}").into_bytes();
    let (out, err, status) = named(&["--seq", "-r", ".name"], &last)?;
    assert_eq!(
        (err.as_str(), status),
        (
            "jq: ignoring parse error: Unfinished string at EOF at line 60002, column 0\n",
            0
        )
    );
    assert_eq!(out.lines().count(), RECORDS);
    on_stdin(&["--seq", "-r", ".name"], &last, 0)?;
    Ok(())
}

/// What the records before the turn leave in jq's reader decides how the turn reads: a bare `n` after a
/// token whose second byte is `u` takes the keyword path ("Invalid literal"), and the number path after
/// any other; a file that ends in an RS straight after its last record ends there (#2998). jq 1.7.1's
/// output for each.
#[test]
fn a_seq_file_turns_with_what_its_records_left_7262() -> Result<()> {
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
        let (out, got_err, status) = named(&["--seq", "-c", "."], &stream)?;
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

/// A file with nothing plain before its first byte that is not a record's (a BOM, text before the first
/// RS) is the whole-stream route's from its first byte; a file on standard input is read from where it
/// stands, a record's start or not; and a run that halts before the turn reads no further.
#[test]
fn a_seq_file_that_never_turns_plain_or_stops_first_7262() -> Result<()> {
    let clean = records(0..RECORDS);
    for front in ["\u{feff}", "x\n", "\n"] {
        let stream = format!("{front}{clean}{}", "\x1e[1,]\n").into_bytes();
        named(PROGRAMS[1], &stream)?;
        on_stdin(PROGRAMS[1], &stream, 0)?;
    }
    let stream = format!("{clean}\x1e{{\"a\"\n").into_bytes();
    let at = clean.find("\x1e{\"id\":1000,").expect("a record") as u64;
    for offset in [at, at + 3] {
        on_stdin(PROGRAMS[1], &stream, offset)?;
    }
    let (_, err, status) = named(
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
