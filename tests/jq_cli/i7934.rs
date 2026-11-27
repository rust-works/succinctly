//! #7934: a fault after a byte-order mark, then invalid UTF-8 and a NUL-ended number at the end of
//! jq's second 4,095-byte read, gave a value jq never reads and exit status 0.
//!
//! jq drops the rest of the read it meets a fault in and reads on from the next one. `succinctly
//! jq` counted where that read ends over the text after the invalid byte `0xC3` had become a
//! three-byte U+FFFD, so it resumed two bytes early, inside `123` (which #7921 reads as a number
//! where its NUL ends it), and read `23` as a further input: the program ran again on it, and the
//! run's status was that last run's, 0. jq's next read is the newline alone.
//!
//! The same count placed a value on a line one short, wherever a line was counted over such a text
//! (the document loop, the lazy route, the eager reader), and ended a read of the bytes moved onto a
//! file early, or two bytes late when the file starts inside a replacement; and the windowed route
//! looked for a number's NUL tail after substituting a window.
//!
//! Every expected row was captured from jq 1.7.1 (`/usr/bin/jq`); jq 1.8.2 (`/opt/homebrew/bin/jq`)
//! gives the same stdout and exit status for each except where a row says otherwise (its `(at …)`
//! line differs on the first row).

use std::process::Command;

use anyhow::Result;

use crate::{run_jq_over_byte_files, small_window};

/// The issue's 8,200 bytes: a mark, `[1,2]` and a newline, spaces to byte 4,092, `x`, `a`s, and
/// `0xC3`, `]123`, a NUL and a newline at the end.
fn input() -> Vec<u8> {
    let head = b"\xef\xbb\xbf[1,2]\n";
    let tail = b"\xc3]123\x00\n";
    let mut text = head.to_vec();
    text.resize(4092, b' ');
    text.push(b'x');
    text.resize(8200 - tail.len(), b'a');
    text.extend_from_slice(tail);
    text
}

fn run(args: &[&str]) -> Result<(String, String, i32)> {
    let text = input();
    let (out, err, code, paths) = run_jq_over_byte_files(args, &[&text])?;
    let err = err.replace("succinctly jq:", "jq:").replace(&paths[0], "f");
    Ok((out, err, code))
}

#[test]
fn a_fault_before_a_replaced_byte_resumes_at_jqs_read_end_7934() -> Result<()> {
    assert_eq!(
        run(&["-c", "[inputs|type]"])?,
        (
            String::new(),
            "jq: error (at f:1): Invalid numeric literal at line 2, column 8186\n".to_string(),
            5
        ),
    );
    assert_eq!(
        run(&["-c", "[., (try input catch \"E\")]"])?,
        ("[[1,2],\"E\"]\n".to_string(), String::new(), 0),
    );
    assert_eq!(
        run(&["-c", "-n", "[limit(8; repeat(try input catch .))]"])?,
        (
            "[[1,2],\"Invalid numeric literal at line 2, column 8186\",\"break\",\"break\",\"break\",\"break\",\"break\",\"break\"]\n".to_string(),
            String::new(),
            0
        ),
    );
    Ok(())
}

/// A string holding the byte 0x80, spaces to byte 4,080, then `1` to `9` and a newline, and `10`
/// on a line of its own: jq's first read of the line ends after byte 4,094, inside `8`'s run of
/// bytes, so `1` to `7` complete in it and `8` and `9` in the read that holds the line break.
/// Counted over the text, where the byte is U+FFFD's three, the read ended at `6` (#7934).
fn long_line_past_a_replaced_byte() -> Vec<u8> {
    let mut text = b"\"\x80\"".to_vec();
    text.resize(4080, b' ');
    text.extend_from_slice(b" 1 2 3 4 5 6 7 8 9\n10\n");
    text
}

fn run_over(args: &[&str], files: &[&[u8]]) -> Result<(String, String, i32)> {
    let (out, err, code, paths) = run_jq_over_byte_files(args, files)?;
    let mut err = err.replace("succinctly jq:", "jq:");
    for (index, path) in paths.iter().enumerate() {
        err = err.replace(path.as_str(), &format!("f{index}"));
    }
    Ok((out, err, code))
}

/// The line a value names, on each route that counts it: the document loop's `(at f:N)`, the lazy
/// route's `input_line_number` with and without `-n`, and the eager reader past a fault.
#[test]
fn a_value_past_a_replaced_byte_is_on_the_line_of_jqs_read_7934() -> Result<()> {
    let text = long_line_past_a_replaced_byte();
    // jq 1.8.2 reads 4,091 bytes at a time, so it puts `6` and `7` on line 1 as well.
    let errors: String = [(1, 0), (2, 0), (3, 0), (4, 0), (5, 0), (6, 0), (7, 0)]
        .into_iter()
        .chain([(8, 1), (9, 1), (10, 2)])
        .map(|(n, line)| format!("jq: error (at f0:{line}) (not a string): {n}\n"))
        .collect();
    assert_eq!(
        run_over(&["-c", "select(type==\"number\")|error"], &[&text])?,
        (String::new(), errors, 5),
    );
    assert_eq!(
        run_over(
            &["-c", "select(type==\"number\")|input_line_number"],
            &[&text]
        )?,
        (
            "0\n0\n0\n0\n0\n0\n0\n1\n1\n2\n".to_string(),
            String::new(),
            0
        ),
    );
    assert_eq!(
        run_over(
            &["-nc", "[inputs|select(type==\"number\")|input_line_number]"],
            &[&text]
        )?,
        ("[0,0,0,0,0,0,0,1,1,2]\n".to_string(), String::new(), 0),
    );
    // A fault first: the eager reader reads the long line from its next read.
    let faulted = [b"0\n}\n".as_slice(), &text].concat();
    assert_eq!(
        run_over(
            &[
                "-nc",
                "[limit(14; repeat(try (input|[., input_line_number]) catch .))]"
            ],
            &[&faulted]
        )?,
        (
            "[[0,1],\"Unmatched '}' at line 2, column 1\",[\"\u{FFFD}\",2],[1,2],[2,2],[3,2],[4,2],[5,2],[6,2],[7,2],[8,3],[9,3],[10,4],\"break\"]\n".to_string(),
            String::new(),
            0
        ),
    );
    Ok(())
}

/// A surrogate's three bytes cut across two files after `0`, a newline and a quote: the second
/// file's reads start on its first byte, inside the one U+FFFD the substitution of the two makes,
/// so its first read ends 4,095 bytes on from there. Counted from the end of the replacement it
/// ended two bytes late, after the `}` at 4,098 and the 0x80 after it, where jq resumes on the
/// 0x80 itself, the fault's read having ended at the `}`.
#[test]
fn a_file_that_starts_inside_a_replacement_reads_from_its_first_byte_7934() -> Result<()> {
    let mut text = b"0\n\"\xed\xa0\x80\"".to_vec();
    text.resize(2 + 4098, b' ');
    text.extend_from_slice(b"}\x80\"\n5\n6\n");
    let files: [&[u8]; 2] = [&text[..4], &text[4..]];
    assert_eq!(
        run_over(&["-nc", "[limit(8; repeat(try input catch .))]"], &files)?,
        (
            "[0,\"\u{FFFD}\",\"Unmatched '}' at line 2, column 4099\",5,6,\"break\",\"break\",\"break\"]\n".to_string(),
            String::new(),
            0
        ),
    );
    assert_eq!(
        run_over(&["-c", "[inputs|type]"], &files)?,
        (
            "[\"number\"]\n".to_string(),
            "jq: error (at f1:1): Unmatched '}' at line 2, column 4099\n".to_string(),
            0
        ),
    );
    Ok(())
}

/// A file the plain route reads a window at a time: a number with a NUL that jq's first read of a
/// long line cuts after the NUL (`1`, a NUL, `ab` from line offset 4,090), behind a string holding
/// 0x80. jq's read hides the token's tail, so both releases print `0`, `1`, `2` and `3`. The window
/// looked for such a token after its invalid bytes had become U+FFFD, two bytes more each, so the
/// read ended before the NUL and the run stopped at `Invalid JSON text`, exit 5 (#7934).
///
/// Re-pinned by #7966: the NUL is the fault (ADR-0018's E2), `Invalid character` at its own
/// column, which is counted in the bytes as read: 4,092, where the U+FFFD text would put it at
/// 4,094. The run stops there with `0` printed, as jq master (fbee8f4) reads it.
#[test]
fn a_window_reads_a_nul_past_a_replaced_byte_as_a_fault_in_the_bytes_read_7966() -> Result<()> {
    let mut bytes = Vec::new();
    while bytes.len() < small_window::MIN_BYTES {
        bytes.push(b'"');
        bytes.resize(bytes.len() + 60_000, b'a');
        bytes.extend_from_slice(b"\"\n");
    }
    bytes.extend_from_slice(b"0\n");
    let line = bytes.len();
    bytes.extend_from_slice(b"\"\x80\"");
    bytes.resize(line + 4090, b' ');
    bytes.extend_from_slice(b"1\x00ab      2\n3\n");
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("in.json");
    std::fs::write(&path, &bytes)?;
    let output = small_window::apply(Command::new(env!("CARGO_BIN_EXE_succinctly")).arg("jq"))
        .args(["-c", "select(type!=\"string\")"])
        .arg(&path)
        .output()?;
    assert_eq!(
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code(),
        ),
        (
            "0\n".to_string(),
            "jq: parse error: Invalid character at line 7, column 4092\n".to_string(),
            Some(5)
        ),
    );
    Ok(())
}
