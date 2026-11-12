//! #7024: standard input that is a regular file (`jq … < file`) is read a window at a time like a path
//! is (#6920): it can be read again by position for a fault's wording, which a pipe cannot. Every row is
//! what the same bytes give on a pipe: stdout, stderr and exit status, and the file here starts at an
//! offset when the shell or a parent has read part of it already. A pipe of 4 MiB or more is windowed
//! too since #7024's pipe slice, and words its fault as it reads instead (`i7024_pipe.rs` checks it
//! against the whole-file route), so the two sides here are the two ways a window words a fault.
//!
//! A terminal and a stdin under 4 MiB take the route they always did.

#![cfg(unix)]

use std::io::{Seek, SeekFrom, Write};
use std::process::{Command, Stdio};

use anyhow::Result;

use crate::spawn_jq;

const LINES: usize = 60_000;

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

/// `succinctly jq ARGS` with `file` as standard input, from its offset `offset`: stdout, stderr, status.
fn with_stdin_file(args: &[&str], bytes: &[u8], offset: u64) -> Result<(Vec<u8>, Vec<u8>, i32)> {
    let mut file = tempfile::tempfile()?;
    file.write_all(bytes)?;
    file.seek(SeekFrom::Start(offset))?;
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("jq")
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::from(file))
        .output()?;
    Ok((
        output.stdout,
        output.stderr,
        output.status.code().unwrap_or(-1),
    ))
}

/// The same bytes (from `offset`) on a pipe.
fn with_stdin_pipe(args: &[&str], bytes: &[u8], offset: usize) -> Result<(Vec<u8>, Vec<u8>, i32)> {
    let (output, status) = spawn_jq(args, Some(&bytes[offset..]))?;
    Ok((output.stdout, output.stderr, status))
}

fn same(args: &[&str], bytes: &[u8], offset: usize) -> Result<(String, String, i32)> {
    assert!(
        bytes.len() - offset > 4 * 1024 * 1024,
        "the file must be over the windowed route's minimum"
    );
    let file = with_stdin_file(args, bytes, offset as u64)?;
    let pipe = with_stdin_pipe(args, bytes, offset)?;
    assert!(
        file.0 == pipe.0,
        "{args:?} at {offset}: stdout differs ({} and {} bytes)",
        file.0.len(),
        pipe.0.len()
    );
    assert_eq!(
        String::from_utf8_lossy(&file.1),
        String::from_utf8_lossy(&pipe.1),
        "{args:?} at {offset}: stderr"
    );
    assert_eq!(file.2, pipe.2, "{args:?} at {offset}: exit status");
    Ok((
        String::from_utf8_lossy(&file.0).into_owned(),
        String::from_utf8_lossy(&file.1).into_owned(),
        file.2,
    ))
}

#[test]
fn a_redirected_file_reads_as_a_pipe_does_7024() -> Result<()> {
    let clean = join(&lines(LINES));
    for args in [
        &["-c", "select(.id % 7919 == 0) | .id"][..],
        &["-r", "select(.id % 9973 == 0) | .name"],
        &["-e", "select(.id == 59999) | .name"],
        &["-c", "select(.id == 3) | [input_filename, .id]"],
    ] {
        let (out, err, status) = same(args, &clean, 0)?;
        assert_eq!((err.as_str(), status), ("", 0), "{args:?}");
        assert!(!out.is_empty(), "{args:?} selects something");
    }
    // The line a runtime error names.
    let (out, err, _) = same(
        &["-c", "if .id == 30000 then error(\"boom\") else empty end"],
        &clean,
        0,
    )?;
    assert_eq!(
        (out.as_str(), err.as_str()),
        ("", "jq: error (at <stdin>:30001): boom\n")
    );
    // A fault, worded from the stream's first bytes read again from the file.
    let mut faulty = lines(LINES);
    faulty[30_000] = "}".to_string();
    let (out, err, _) = same(&["-c", "select(.id % 10000 == 0) | .id"], &join(&faulty), 0)?;
    assert_eq!(out, "0\n10000\n20000\n");
    assert_eq!(
        err,
        "jq: parse error: Unmatched '}' at line 30001, column 1\n"
    );
    // A BOM, bytes that are not UTF-8, and a document the stream ends inside.
    let mut bom = b"\xef\xbb\xbf".to_vec();
    bom.extend(&clean);
    same(&["-c", "select(.id == 0) | .name"], &bom, 0)?;
    let mut invalid = clean.clone();
    let at = invalid
        .windows(10)
        .position(|w| w == b"user 40000")
        .unwrap();
    invalid.insert(at, 0xff);
    same(&["-c", "select(.id == 40000) | .name"], &invalid, 0)?;
    let mut cut = clean.clone();
    cut.extend_from_slice(b"{\"id\":1,");
    same(&["-c", "select(.id == 59999) | .id"], &cut, 0)?;
    Ok(())
}

/// A stdin something has already read from starts where it was left, as a pipe does: the stream is
/// what is left, and a fault's line and column count from there.
#[test]
fn a_redirected_file_starts_at_its_offset_7024() -> Result<()> {
    let mut faulty = lines(LINES);
    faulty[40_000] = "}".to_string();
    let bytes = join(&faulty);
    // Offsets at a line start, in the middle of a line and in the middle of a document's string.
    let line_start = bytes
        .iter()
        .take(1_000_000)
        .rposition(|&b| b == b'\n')
        .unwrap()
        + 1;
    // At a line start the stream is whole documents and ends at the stray `}`: line 40001 less the lines before.
    let (out, err, status) = same(
        &["-c", "select(.id % 20000 == 0) | .id"],
        &bytes,
        line_start,
    )?;
    let skipped = bytes[..line_start].iter().filter(|&&b| b == b'\n').count();
    assert_eq!(out, "20000\n");
    assert_eq!(
        err,
        format!(
            "jq: parse error: Unmatched '}}' at line {}, column 1\n",
            40_001 - skipped
        )
    );
    assert_ne!(status, 0);
    // Mid-line and mid-string, the stream starts inside a document: whatever the pipe says, the file says.
    for offset in [line_start + 17, 3] {
        let (_, err, status) = same(&["-c", "select(.id % 20000 == 0) | .id"], &bytes, offset)?;
        assert_ne!(status, 0, "offset {offset}");
        assert!(!err.is_empty(), "offset {offset}");
    }
    Ok(())
}

/// A stdin the run also writes to (`jq … <> f >&0`) is not read a window at a time: its output would land
/// where the next window is read from. It reads whole, so the file ends as the input then the output.
#[test]
fn a_stdin_that_is_also_stdout_is_read_whole_7024() -> Result<()> {
    let bytes = join(&lines(LINES));
    let mut file = tempfile::tempfile()?;
    file.write_all(&bytes)?;
    file.seek(SeekFrom::Start(0))?;
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .args(["jq", "-c", "."])
        .stdin(Stdio::from(file.try_clone()?))
        .stdout(Stdio::from(file.try_clone()?))
        .output()?;
    assert!(output.status.success(), "{:?}", output.status);
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let (expected, _) = spawn_jq(&["-c", "."], Some(&bytes))?;
    let mut written = Vec::new();
    file.seek(SeekFrom::Start(0))?;
    std::io::Read::read_to_end(&mut file, &mut written)?;
    let mut want = bytes.clone();
    want.extend(&expected.stdout);
    assert!(
        written == want,
        "{} bytes, wanted {}",
        written.len(),
        want.len()
    );
    Ok(())
}
