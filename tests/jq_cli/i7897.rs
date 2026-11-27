//! #7897, re-pinned by #7966: a number that ends one file, with a NUL at the front of the next,
//! after a fault earlier in the first file's last read. jq drops the rest of the read a fault is met
//! in, so the number's digits there are gone when its parser reaches the next file, and what that
//! file starts with is a token of its own. `succinctly jq` once rewrote the digits and the NUL as one
//! number over both files, which moved the number's last digit into the second file.
//!
//! Under ADR-0018's #7963 amendment E2 there is no NUL-ended number: the NUL is the second fault,
//! `Invalid character` at its own column (jq master, commit fbee8f4, reads it so), counted from where
//! the first fault left jq's parser (the dropped bytes are not counted, so `l 12` then a NUL is
//! column 3). jq 1.7.1 and 1.8.2 agree with each other on every row: they give `Invalid numeric
//! literal at line 2, column 0` for a NUL alone and for `\0 3` the `3` at column 4, read `2`, NUL,
//! `x` as the number `2`, and read the clean row's `12` across the files as `12`. So only the first
//! fault of each row is still theirs. Every run here exits 0 with nothing on stderr except the clean
//! row under `--stream -s`, where the NUL is the run's first fault.

use anyhow::Result;

use crate::run_jq_over_byte_files;

/// A program that reads three inputs past the faults, one answer a line.
const READS: &str = "try input catch ., try input catch ., try input catch .";

/// `(arguments, files, stdout)`; every row but the last exits 0 with nothing on stderr.
type Row = (&'static [&'static str], &'static [&'static [u8]], String);

const FAULT: &str = "Invalid numeric literal at line 1, column 2";
/// The NUL alone, or before ` 3`, after `l 12`: the dropped ` 12` is not counted.
const SECOND: &str = "Invalid character at line 1, column 3";
/// The NUL after `2` in the next file, after `l 1`.
const AFTER_DIGIT: &str = "Invalid character at line 1, column 4";
/// The NUL after `"" 12`, with no fault before it.
const CLEAN: &str = "Invalid character at line 1, column 6";

fn rows() -> Vec<Row> {
    let slurp: &[&str] = &["--stream-errors", "-c", "-s", "."];
    let reads: &[&str] = &["-c", "-n", READS];
    let stream_reads: &[&str] = &["-c", "--stream", "-n", READS];
    let stream_slurp_reads: &[&str] = &["-c", "--stream", "-s", "-n", READS];
    let fault_then_nul: &[&[u8]] = &[b"l 12", b"\0\n"];
    let fault_then_digit: &[&[u8]] = &[b"l 1", b"2\0x\n"];
    let fault_three_files: &[&[u8]] = &[b"l 1", b"2", b"\0\n"];
    let fault_then_more: &[&[u8]] = &[b"l 12", b"\0 3\n"];
    let clean: &[&[u8]] = &[b"\"\" 12", b"\0\n"];
    vec![
        // The issue's row: two faults, where a fault and `[[],2]` came out before #7897.
        (
            slurp,
            fault_then_nul,
            format!("[[\"{FAULT}\",[]],[\"{SECOND}\",[]]]\n"),
        ),
        (
            slurp,
            fault_then_digit,
            format!("[[\"{FAULT}\",[]],[\"{AFTER_DIGIT}\",[]]]\n"),
        ),
        (
            slurp,
            fault_three_files,
            format!("[[\"{FAULT}\",[]],[\"{AFTER_DIGIT}\",[]]]\n"),
        ),
        (
            slurp,
            fault_then_more,
            format!("[[\"{FAULT}\",[]],[\"{SECOND}\",[]]]\n"),
        ),
        (slurp, clean, format!("[[[],\"\"],[\"{CLEAN}\",[]]]\n")),
        // The documents' reader, which reads on past a fault under `input`.
        (
            reads,
            fault_then_nul,
            format!("\"{FAULT}\"\n\"{SECOND}\"\n\"break\"\n"),
        ),
        (
            reads,
            fault_then_digit,
            format!("\"{FAULT}\"\n\"{AFTER_DIGIT}\"\n\"break\"\n"),
        ),
        (
            reads,
            fault_three_files,
            format!("\"{FAULT}\"\n\"{AFTER_DIGIT}\"\n\"break\"\n"),
        ),
        (reads, clean, format!("\"\"\n\"{CLEAN}\"\n\"break\"\n")),
        // `--stream -s` under `-n`: the faults before the array, then the array.
        (
            stream_slurp_reads,
            fault_then_nul,
            format!("\"{FAULT}\"\n\"{SECOND}\"\n[]\n"),
        ),
        (
            stream_slurp_reads,
            fault_then_digit,
            format!("\"{FAULT}\"\n\"{AFTER_DIGIT}\"\n[]\n"),
        ),
        // `--stream` under `-n` reads the files a chunk at a time: the rows the others agree with.
        (
            stream_reads,
            fault_then_nul,
            format!("\"{FAULT}\"\n\"{SECOND}\"\n\"break\"\n"),
        ),
        (
            stream_reads,
            fault_then_digit,
            format!("\"{FAULT}\"\n\"{AFTER_DIGIT}\"\n\"break\"\n"),
        ),
        (
            stream_reads,
            clean,
            format!("[[],\"\"]\n\"{CLEAN}\"\n\"break\"\n"),
        ),
    ]
}

#[test]
fn a_nul_after_a_fault_is_the_second_fault_7966() -> Result<()> {
    for ((args, files, stdout), row) in rows().into_iter().zip(0..) {
        let (out, err, code, _) = run_jq_over_byte_files(args, files)?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (stdout.as_str(), "", 0),
            "row {row}: {args:?} over {files:?}"
        );
    }
    Ok(())
}

/// `--stream -s` over the clean row: the NUL is the run's first fault, and the run ends there (both
/// releases print `[[[],""],[[],12]]`).
#[test]
fn a_nul_with_no_fault_before_it_ends_a_stream_slurp_7966() -> Result<()> {
    let (out, err, code, _) =
        run_jq_over_byte_files(&["-c", "--stream", "-s", "."], &[b"\"\" 12", b"\0\n"])?;
    assert_eq!(
        (out.as_str(), err.as_str(), code),
        ("", format!("jq: parse error: {CLEAN}\n").as_str(), 5)
    );
    Ok(())
}
