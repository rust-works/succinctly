//! #7412: `-R` over a pipe is read a window at a time once the pipe holds one window, where it waited
//! for four: the probe that learns a pipe's length is the run's first window, so its size was the
//! run's peak. Each row puts what the end of the probe can cut (a line break, CR LF, a character of
//! several bytes, bytes that are not UTF-8, a NUL, a line longer than the window) at or around one
//! window and checks the run against the same bytes read whole: the pipe named as `/dev/stdin`, which
//! the windowed routes decline, so stdout, stderr (with that name read as `<stdin>`) and the exit
//! status are the whole-file route's.
//!
//! In a debug build the window is 64 KiB (`small_window`, #7376); a release build runs the same rows
//! over the shipped 256 KiB window.

#![cfg(unix)]

use anyhow::Result;

use crate::small_window::{spawn_jq, WINDOW};

/// About three windows of short lines, so a pipe of it goes on past the probe.
fn lines() -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut i = 0;
    while bytes.len() < 3 * WINDOW {
        bytes.extend_from_slice(format!("{{\"id\":{i},\"name\":\"user {i}\"}}\n").as_bytes());
        i += 1;
    }
    bytes
}

fn run(args: &[&str], bytes: &[u8]) -> Result<(String, String, i32)> {
    let (output, status) = spawn_jq(args, Some(bytes))?;
    Ok((
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).replace("/dev/stdin", "<stdin>"),
        status,
    ))
}

/// The run over `bytes` on a pipe is the run over the same bytes read whole.
fn same(args: &[&str], bytes: &[u8], what: &str) -> Result<()> {
    let got = run(args, bytes)?;
    let mut whole_args = args.to_vec();
    whole_args.push("/dev/stdin");
    let mut want = run(&whole_args, bytes)?;
    want.0 = want.0.replace("/dev/stdin", "<stdin>");
    assert!(
        got.0 == want.0,
        "{args:?} {what}: stdout differs ({} and {} bytes)",
        got.0.len(),
        want.0.len()
    );
    assert_eq!(got.1, want.1, "{args:?} {what}: stderr");
    assert_eq!(got.2, want.2, "{args:?} {what}: exit status");
    Ok(())
}

const ROWS: [&[&str]; 5] = [
    &["-R", "."],
    &["-cR", "[length, input_line_number]"],
    &["-nR", "[inputs] | length"],
    &["-nR", "input, input | .[0:12]"],
    &[
        "-R",
        "if test(\"id.:100,\") then error(\"boom\") else length end",
    ],
];

#[test]
fn a_raw_pipe_around_one_window_reads_as_the_whole_stream_does_7412() -> Result<()> {
    let clean = lines();
    let line_end = |near: usize| clean[..near].iter().rposition(|&b| b == b'\n').unwrap() + 1;
    let mut cases: Vec<(String, Vec<u8>)> = Vec::new();
    // Lengths either side of the probe, and a pipe that ends at a line break exactly at it.
    for len in [
        WINDOW / 2,
        WINDOW - 1,
        WINDOW,
        WINDOW + 1,
        line_end(WINDOW),
        2 * WINDOW,
    ] {
        cases.push((format!("the first {len} bytes"), clean[..len].to_vec()));
    }
    // A line break as the probe's last byte.
    let at = line_end(WINDOW - 1);
    let mut nl_last = clean[..at].to_vec();
    nl_last.resize(WINDOW - 1, b'q');
    nl_last.push(b'\n');
    nl_last.extend_from_slice(&clean[at..]);
    cases.push(("a line break as the probe's last byte".into(), nl_last));
    // What the probe's end can cut.
    let cut = |what: &str, insert: &[u8]| {
        let mut bytes = clean[..WINDOW - 1].to_vec();
        bytes.extend_from_slice(insert);
        bytes.extend_from_slice(&clean[WINDOW - 1..]);
        (what.to_string(), bytes)
    };
    cases.push(cut("CR LF across the probe's end", b"\r\n"));
    cases.push(cut(
        "a character of three bytes across it",
        "\u{20ac}".as_bytes(),
    ));
    cases.push(cut("bytes that are not UTF-8 across it", b"\xff\xfe"));
    cases.push(cut("a NUL at it", b"\x00"));
    cases.push(cut(
        "a line longer than a window across it",
        &vec![b'x'; WINDOW + 77],
    ));
    let crlf: Vec<u8> = clean
        .split(|&b| b == b'\n')
        .collect::<Vec<_>>()
        .join(&b"\r\n"[..]);
    cases.push(("CR LF throughout".into(), crlf));
    cases.push((
        "no line break at the end".into(),
        clean[..clean.len() - 1].to_vec(),
    ));
    let mut one_long = vec![b'z'; 2 * WINDOW + 3];
    one_long.push(b'\n');
    cases.push(("one line of two windows".into(), one_long));
    for (what, bytes) in &cases {
        for args in ROWS {
            same(args, bytes, what)?;
        }
    }
    Ok(())
}
