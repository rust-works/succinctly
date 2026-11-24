//! #7897: a number that ends one file, with a NUL at the front of the next, after a fault earlier
//! in the first file's last read. jq drops the rest of the read a fault is met in, so the number's
//! digits there are gone when its parser reaches the next file, and what that file starts with is a
//! token of its own: a NUL alone is a second fault, and `2`, NUL, `x` is the number `2`.
//! `succinctly jq` rewrote the digits and the NUL as one number over both files, which moved the
//! number's last digit into the second file, where a reader that goes on past the fault found it:
//! `l 12` and then a NUL gave out `2`, under `--stream-errors -s`, under `--stream -s -n` and for the
//! documents an `input` reads past a caught error. Without a fault the digits are still held, and
//! the two files are the one number.
//!
//! Every row was captured from jq 1.7.1 (`/usr/bin/jq`), and jq 1.8.2 (`/opt/homebrew/bin/jq`) gives
//! the same answer for each. Every row exits 0 with nothing on stderr.

use anyhow::Result;

use crate::run_jq_over_byte_files;

/// A program that reads three inputs past the faults, one answer a line.
const READS: &str = "try input catch ., try input catch ., try input catch .";

/// `(arguments, files, stdout)`.
type Row = (&'static [&'static str], &'static [&'static [u8]], String);

const FAULT: &str = "Invalid numeric literal at line 1, column 2";
const SECOND: &str = "Invalid numeric literal at line 2, column 0";

fn rows() -> Vec<Row> {
    let slurp: &[&str] = &["--stream-errors", "-c", "-s", "."];
    let reads: &[&str] = &["-c", "-n", READS];
    let stream_reads: &[&str] = &["-c", "--stream", "-n", READS];
    let stream_slurp: &[&str] = &["-c", "--stream", "-s", "."];
    let stream_slurp_reads: &[&str] = &["-c", "--stream", "-s", "-n", READS];
    let fault_then_nul: &[&[u8]] = &[b"l 12", b"\0\n"];
    let fault_then_digit: &[&[u8]] = &[b"l 1", b"2\0x\n"];
    let fault_three_files: &[&[u8]] = &[b"l 1", b"2", b"\0\n"];
    let fault_then_more: &[&[u8]] = &[b"l 12", b"\0 3\n"];
    let clean: &[&[u8]] = &[b"\"\" 12", b"\0\n"];
    vec![
        // The issue's row: two faults, where a fault and `[[],2]` came out.
        (
            slurp,
            fault_then_nul,
            format!("[[\"{FAULT}\",[]],[\"{SECOND}\",[]]]\n"),
        ),
        (
            slurp,
            fault_then_digit,
            format!("[[\"{FAULT}\",[]],[[],2]]\n"),
        ),
        (
            slurp,
            fault_three_files,
            format!("[[\"{FAULT}\",[]],[[],2]]\n"),
        ),
        (
            slurp,
            fault_then_more,
            format!("[[\"{FAULT}\",[]],[\"Invalid numeric literal at line 1, column 4\",[]]]\n"),
        ),
        (slurp, clean, "[[[],\"\"],[[],12]]\n".to_string()),
        // The documents' reader, which reads on past a fault under `input`.
        (
            reads,
            fault_then_nul,
            format!("\"{FAULT}\"\n\"{SECOND}\"\n\"break\"\n"),
        ),
        (
            reads,
            fault_then_digit,
            format!("\"{FAULT}\"\n2\n\"break\"\n"),
        ),
        (
            reads,
            fault_three_files,
            format!("\"{FAULT}\"\n2\n\"break\"\n"),
        ),
        (reads, clean, "\"\"\n12\n\"break\"\n".to_string()),
        // `--stream -s` under `-n`: the faults before the array, then the array.
        (
            stream_slurp_reads,
            fault_then_nul,
            format!("\"{FAULT}\"\n\"{SECOND}\"\n[]\n"),
        ),
        (
            stream_slurp_reads,
            fault_then_digit,
            format!("\"{FAULT}\"\n[[[],2]]\n\"break\"\n"),
        ),
        // `--stream` under `-n` reads the files a chunk at a time and never held the rewrite: the
        // rows the others are to agree with.
        (
            stream_reads,
            fault_then_nul,
            format!("\"{FAULT}\"\n\"{SECOND}\"\n\"break\"\n"),
        ),
        (
            stream_reads,
            fault_then_digit,
            format!("\"{FAULT}\"\n[[],2]\n\"break\"\n"),
        ),
        (
            stream_reads,
            clean,
            "[[],\"\"]\n[[],12]\n\"break\"\n".to_string(),
        ),
        (stream_slurp, clean, "[[[],\"\"],[[],12]]\n".to_string()),
    ]
}

#[test]
fn a_fault_before_a_number_that_ends_a_file_drops_it_before_a_nul_in_the_next_7897() -> Result<()> {
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
