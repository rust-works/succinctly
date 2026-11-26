//! #7938: the windowed document loop counted a value's 4,095-byte read over the substituted text, so
//! past invalid UTF-8 on a line longer than a read a value just past the read boundary was put on
//! the next line (`(at f:N)` one line late).
//!
//! A window's text is substituted region by region, and the edits were kept only for a window whose
//! tail is handed to a later file. The windows now keep every edit, window-relative, and hand them
//! to the document loop's locator, which counts reads in the bytes jq read as the whole-file route
//! has since #7934.
//!
//! Expected rows were captured from jq 1.7.1 (`/usr/bin/jq`), on the issue's file of 2,097,162 lines
//! of `0` and on one of 524,298 (the count [`scaled`] gives a release build); a debug build takes the
//! windowed route with its small window over fewer. jq 1.8.2 reads 4,091 bytes at a time, so it
//! splits the long line's values elsewhere.

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::Result;

use crate::small_window::{self, scaled, MIN_BYTES};

fn input(zeros: usize) -> Vec<u8> {
    let mut line = b"\"\x80\"".to_vec();
    while line.len() < 4080 {
        line.extend_from_slice(b" 0");
    }
    line.extend_from_slice(b" 1 2 3 4 5 6 7 8 9\n");
    let mut text = b"0\n".repeat(zeros);
    text.extend_from_slice(&line);
    text
}

/// The lines `(at <name>:N)` names, one per error, in order.
fn lines(stderr: &str) -> Vec<usize> {
    stderr
        .lines()
        .filter_map(|line| line.split("(at ").nth(1))
        .filter_map(|at| at.split(':').nth(1)?.split(')').next()?.parse().ok())
        .collect()
}

#[test]
fn a_window_counts_reads_in_the_bytes_jq_read_7938() -> Result<()> {
    let zeros = scaled(2 << 20) + 10;
    let text = input(zeros);
    assert!(text.len() >= MIN_BYTES, "{} bytes", text.len());
    // The string and `1` to `6` complete in the long line's first read, which holds no line break,
    // so jq names the line before it; `7` to `9` in the read that holds the break.
    let mut expected = vec![zeros; 7];
    expected.extend([zeros + 1; 3]);
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("f.json");
    std::fs::File::create(&path)?.write_all(&text)?;
    let program = "select(. != 0) | error";
    let file = small_window::apply(
        Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .args(["jq", "-c", program])
            .arg(&path),
    )
    .output()?;
    assert_eq!(lines(&String::from_utf8(file.stderr)?), expected, "file");
    let stdin = small_window::apply(
        Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .args(["jq", "-c", program])
            .stdin(Stdio::from(std::fs::File::open(&path)?)),
    )
    .output()?;
    assert_eq!(lines(&String::from_utf8(stdin.stderr)?), expected, "stdin");
    Ok(())
}
