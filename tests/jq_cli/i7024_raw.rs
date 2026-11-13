//! #7024: `-R` reads standard input a window at a time when it is a pipe that holds 4 MiB or more, or a
//! regular file with 4 MiB or more left (`jq -R . < log`), as #7023 reads a file named on the command
//! line. A stream of lines has no fault to word, so neither is read again. Every row is checked against
//! the same bytes read whole: the pipe named as `/dev/stdin`, which the windowed routes decline (it names
//! a descriptor the run already holds), so stdout, stderr (with that name read as `<stdin>`) and the exit
//! status are the whole-file route's.

#![cfg(unix)]

use std::io::{Seek, SeekFrom, Write};
use std::process::{Command, Stdio};

use anyhow::Result;

use crate::spawn_jq;

const LINES: usize = 60_000;
const MIN: usize = 4 * 1024 * 1024;

fn lines(count: usize) -> Vec<String> {
    (0..count)
        .map(|i| {
            format!(
                "{{\"id\":{i},\"name\":\"user {i}\",\"email\":\"user{i}@example.com\",\"tags\":[\"a\",\"b\",\"c\"],\"score\":{}}}",
                i % 1000
            )
        })
        .collect()
}

fn join(lines: &[String]) -> Vec<u8> {
    let mut bytes = lines.join("\n").into_bytes();
    bytes.push(b'\n');
    bytes
}

/// `bytes` read whole: on a pipe named `/dev/stdin`, with that name read back as standard input's.
fn whole(args: &[&str], bytes: &[u8]) -> Result<(String, String, i32)> {
    let mut whole_args = args.to_vec();
    whole_args.push("/dev/stdin");
    let (output, status) = spawn_jq(&whole_args, Some(bytes))?;
    Ok((
        String::from_utf8_lossy(&output.stdout).replace("/dev/stdin", "<stdin>"),
        String::from_utf8_lossy(&output.stderr).replace("/dev/stdin", "<stdin>"),
        status,
    ))
}

fn assert_same(
    args: &[&str],
    what: &str,
    got: &(String, String, i32),
    want: &(String, String, i32),
) {
    assert!(
        got.0 == want.0,
        "{args:?} {what}: stdout differs ({} and {} bytes)",
        got.0.len(),
        want.0.len()
    );
    assert_eq!(got.1, want.1, "{args:?} {what}: stderr");
    assert_eq!(got.2, want.2, "{args:?} {what}: exit status");
}

/// `succinctly jq ARGS` over `bytes` on a pipe, read the windowed way, once it is checked to be the
/// whole route's.
fn same_on_pipe(args: &[&str], bytes: &[u8]) -> Result<(String, String, i32)> {
    let (output, status) = spawn_jq(args, Some(bytes))?;
    let got = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        status,
    );
    assert_same(args, "on a pipe", &got, &whole(args, bytes)?);
    Ok(got)
}

/// `succinctly jq ARGS` with `bytes` as a regular file on standard input from `offset`, once it is checked
/// to be the whole route's over the bytes from there; and where the run left the file's offset.
fn same_on_file(
    args: &[&str],
    bytes: &[u8],
    offset: usize,
) -> Result<((String, String, i32), u64)> {
    let mut file = tempfile::tempfile()?;
    file.write_all(bytes)?;
    file.seek(SeekFrom::Start(offset as u64))?;
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("jq")
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::from(file.try_clone()?))
        .output()?;
    let got = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    );
    assert_same(
        args,
        &format!("from a file at {offset}"),
        &got,
        &whole(args, &bytes[offset..])?,
    );
    Ok((got, file.stream_position()?))
}

const ROWS: [&[&str]; 7] = [
    &["-R", "."],
    &["-R", "select(endswith(\"9}\")) | length"],
    &["-nR", "[inputs] | length"],
    &["-nR", "first(inputs)"],
    &["-nR", "input, input | .[0:12]"],
    &["-R", "-e", "select(startswith(\"{\\\"id\\\":59999,\"))"],
    &["-rR", "select(test(\"user 4[0-9]{4}\\\"\")) | .[0:20]"],
];

#[test]
fn raw_lines_on_a_pipe_read_as_the_whole_stream_does_7024() -> Result<()> {
    let clean = join(&lines(LINES));
    assert!(clean.len() > MIN + 1024 * 1024);
    for args in ROWS {
        let (out, err, status) = same_on_pipe(args, &clean)?;
        assert_eq!((err.as_str(), status), ("", 0), "{args:?}");
        assert!(!out.is_empty(), "{args:?} reads something");
    }
    let (out, _, _) = same_on_pipe(&["-nR", "[inputs] | length"], &clean)?;
    assert_eq!(out, format!("{LINES}\n"));
    // The line a runtime error names.
    let (_, err, _) = same_on_pipe(
        &[
            "-R",
            "if startswith(\"{\\\"id\\\":30000,\") then error(\"boom\") else empty end",
        ],
        &clean,
    )?;
    assert_eq!(err, "jq: error (at <stdin>:30001): boom\n");
    // The name a line read from standard input has: jq 1.7.1 answers `"<stdin>"` here, and so does the
    // reference, whose `/dev/stdin` is read back as that name.
    let (out, _, _) = same_on_pipe(&["-nR", "input | input_filename"], &clean)?;
    assert_eq!(out, "\"<stdin>\"\n");
    Ok(())
}

/// What a line edge can hold: a BOM, CR LF, bytes that are not UTF-8 (early, and past the first windows),
/// a last line with no line break, a line longer than a window, and one longer than the probe.
#[test]
fn raw_lines_on_a_pipe_keep_every_line_edge_7024() -> Result<()> {
    let clean = join(&lines(LINES));
    let rows: [&[&str]; 3] = [
        &["-R", "."],
        &["-nR", "[inputs] | length"],
        &["-R", "length"],
    ];
    let mut cases: Vec<Vec<u8>> = Vec::new();
    let mut bom = b"\xef\xbb\xbf".to_vec();
    bom.extend(&clean);
    cases.push(bom);
    cases.push(
        clean
            .split(|&b| b == b'\n')
            .collect::<Vec<_>>()
            .join(&b"\r\n"[..]),
    );
    for near in [100, MIN + 100_000] {
        let mut invalid = clean.clone();
        invalid.splice(near..near, *b"\xff\xfe");
        cases.push(invalid);
    }
    cases.push(clean[..clean.len() - 1].to_vec());
    let mut long = clean.clone();
    long.splice(MIN + 77..MIN + 77, vec![b'x'; 2 * 1024 * 1024]);
    cases.push(long);
    let mut longer = vec![b'y'; MIN + 4096];
    longer.push(b'\n');
    longer.extend(&clean);
    cases.push(longer);
    for (case, bytes) in cases.iter().enumerate() {
        for args in rows {
            let (_, err, status) = same_on_pipe(args, bytes)?;
            assert_eq!((err.as_str(), status), ("", 0), "case {case} {args:?}");
        }
    }
    Ok(())
}

/// A pipe under 4 MiB is read whole, as before: the probe hands what it read to that route. One of
/// exactly 4 MiB and one a byte either side read the same.
#[test]
fn a_short_raw_pipe_and_the_threshold_read_the_same_7024() -> Result<()> {
    let clean = join(&lines(LINES));
    let line_end = |near: usize| clean[..near].iter().rposition(|&b| b == b'\n').unwrap() + 1;
    for len in [10_000, line_end(MIN), MIN - 1, MIN, MIN + 1] {
        for args in [
            &["-nR", "[inputs] | length"][..],
            &["-R", "select(endswith(\"0}\")) | .[0:9]"],
        ] {
            same_on_pipe(args, &clean[..len])?;
        }
    }
    Ok(())
}

/// A regular file on standard input is read from the offset it stands at, and a run that reads it to
/// the end leaves it there, as the whole route does; one that stops early leaves it short of the end.
#[test]
fn a_redirected_file_reads_its_lines_from_its_offset_7024() -> Result<()> {
    let clean = join(&lines(LINES));
    let line_start = clean[..1_000_000]
        .iter()
        .rposition(|&b| b == b'\n')
        .unwrap()
        + 1;
    for offset in [0, line_start, line_start + 17] {
        for args in ROWS {
            let ((_, err, status), _) = same_on_file(args, &clean, offset)?;
            assert_eq!((err.as_str(), status), ("", 0), "{args:?} at {offset}");
        }
        let (_, left) = same_on_file(&["-nR", "[inputs] | length"], &clean, offset)?;
        assert_eq!(left, clean.len() as u64, "read to the end from {offset}");
    }
    let (_, left) = same_on_file(&["-nR", "first(inputs) | length"], &clean, 0)?;
    assert!(
        left < clean.len() as u64,
        "an early exit leaves the rest unread: {left}"
    );
    // Less than the minimum left from the offset: read whole, as before.
    let near_end = clean.len() - MIN + 10;
    same_on_file(&["-nR", "[inputs] | length"], &clean, near_end)?;
    Ok(())
}

/// A standard input that was not opened for reading (`jq . 0>>file`, `jq . 0> >(cat)`) reads as empty,
/// whatever its size and whichever route would read it: the answer the whole route has always given,
/// which a window's own read of the descriptor (`EBADF`) had turned into a failure at 4 MiB or more, on
/// the plain route over a pipe or a redirected file and on `-R`.
#[test]
fn a_stdin_not_open_for_reading_reads_as_empty_at_every_size_7024() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let big = dir.path().join("big.ndjson");
    std::fs::write(&big, join(&lines(LINES)))?;
    let small = dir.path().join("small.ndjson");
    std::fs::write(&small, b"{\"a\":1}\n")?;
    for args in [
        &["-c", "."][..],
        &["-R", "length"],
        &["-nR", "[inputs] | length"],
    ] {
        let mut stdins: Vec<(&str, Stdio)> = Vec::new();
        for (name, path) in [("big", &big), ("small", &small)] {
            let file = std::fs::OpenOptions::new().append(true).open(path)?;
            stdins.push((name, Stdio::from(file)));
        }
        let (_reader, writer) = std::io::pipe()?;
        stdins.push(("pipe write end", Stdio::from(writer)));
        let mut answers = Vec::new();
        for (name, stdin) in stdins {
            let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
                .arg("jq")
                .args(args)
                .stdin(stdin)
                .output()?;
            answers.push((
                name,
                String::from_utf8_lossy(&output.stdout).into_owned(),
                String::from_utf8_lossy(&output.stderr).into_owned(),
                output.status.code(),
            ));
        }
        let (_, out, err, status) = &answers[1];
        for answer in &answers {
            assert_eq!(
                (&answer.1, &answer.2, answer.3),
                (out, err, *status),
                "{args:?} on {}",
                answer.0
            );
        }
        // jq reads such a descriptor as empty and exits 2 once the run is over (#7126).
        assert_eq!(*status, Some(2), "{args:?}: {err}");
    }
    Ok(())
}
