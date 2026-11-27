//! #7921: a caught input fault, or a NUL in a number, where jq's 4,095-byte read ends.
//!
//! jq reads its input with `fgets` into a 4,096-byte buffer, so a line longer than 4,095 bytes
//! reaches its parser a read at a time, and three rules of that reader decide the rows below:
//!
//! - a byte-order mark at the front of the stream is skipped by the parser but is in the first read,
//!   which so holds 4,092 bytes of text;
//! - a fault drops the rest of the read it is met in, and a token open where a file ends goes on
//!   with the next file's bytes;
//! - jq 1.7.1 hands its parser a read with no line break only up to its first NUL, and a number is
//!   read as a C string, so by both rules a NUL ends a number.
//!
//! `succinctly jq` looked for the token a file ends in without the mark's share of the first read,
//! so a `1` after a fault at the end of that read was a value of its own where jq glues it to the
//! next file. A number with a NUL in a read with no line break was left a malformed token, so the
//! number was lost and what followed it was read from the wrong place. And the pass that reads such
//! a number looked for a whole mark in one file, where the mark may be cut across files.
//!
//! Every expected row without a NUL was captured from jq 1.7.1 (`/usr/bin/jq`). jq 1.8.2
//! (`/opt/homebrew/bin/jq`) gives the same answer for each such row of [`rows`] but the three marked
//! "1.7.1 only": its reads are 4,091 bytes long, so its first read ends before the fault there.
//!
//! #7966 re-pinned every row with a NUL. Under ADR-0018's #7963 amendment E2 there is no NUL-ended
//! number: a NUL outside a string is `Invalid character` at its own column, as jq master
//! (unreleased; commit fbee8f4, jqlang/jq#3606) reads it, and each first fault at a NUL here was
//! checked against a driver over master's `jv_parse.c`. What follows the fault is jq's read model
//! as for any other fault (E1, #7967): the rest of the read holding the NUL is dropped, and reading
//! goes on at the next read. Where a row's comment gives jq's answer, it is that of both releases
//! (they read the number up to the NUL); none of these rows matches either release now.

use anyhow::Result;

use crate::run_jq_over_byte_files;

const MARK: &[u8] = b"\xef\xbb\xbf";

/// A program that reads six inputs past the faults.
const READS: &[&str] = &["-c", "-n", "[limit(6; repeat(try input catch .))]"];
const EACH: &[&str] = &["-c", "."];

/// `pad` spaces between `head` and `tail`.
fn padded(head: &[u8], pad: usize, tail: &[u8]) -> Vec<u8> {
    [head, &vec![b' '; pad], tail].concat()
}

/// `(name, arguments, files, stdout, stderr, exit status)`.
type Row = (
    &'static str,
    &'static [&'static str],
    Vec<Vec<u8>>,
    &'static str,
    &'static str,
    i32,
);

fn rows() -> Vec<Row> {
    vec![
        // The issue's first case: the fault ends the first read (mark, 4,090 spaces, `l`, a
        // space), the `1` after it is open at the end of the file and the next file goes on with it.
        (
            "mark, seam",
            READS,
            vec![padded(MARK, 4090, b"l 1"), b"\xbb\xbf3\n9\n".to_vec()],
            "[\"Invalid numeric literal at line 1, column 4092\",\"Invalid numeric literal at line 2, column 0\",9,\"break\",\"break\",\"break\"]\n",
            "",
            0,
        ),
        // 1.7.1 only. Files that are all UTF-8, which the lazy route tries first.
        (
            "mark, seam, a digit goes on",
            READS,
            vec![padded(MARK, 4090, b"l 1"), b"3\n9\n".to_vec()],
            "[\"Invalid numeric literal at line 1, column 4092\",13,9,\"break\",\"break\",\"break\"]\n",
            "",
            0,
        ),
        // 1.7.1 only.
        (
            "mark, seam, a string goes on",
            READS,
            vec![padded(MARK, 4090, b"l \"a"), b"b\"\n9\n".to_vec()],
            "[\"Invalid numeric literal at line 1, column 4092\",\"ab\",9,\"break\",\"break\",\"break\"]\n",
            "",
            0,
        ),
        // 1.7.1 only.
        (
            "mark, seam, an array goes on",
            READS,
            vec![padded(MARK, 4090, b"l [1,"), b"2]\n9\n".to_vec()],
            "[\"Invalid numeric literal at line 1, column 4092\",[1,2],9,\"break\",\"break\",\"break\"]\n",
            "",
            0,
        ),
        // Controls that already agreed: the mark cut across two files, and no mark.
        (
            "mark cut across files, seam",
            READS,
            vec![
                b"\xef".to_vec(),
                padded(b"\xbb\xbf", 4090, b"l 1"),
                b"\xbb\xbf3\n9\n".to_vec(),
            ],
            "[\"Invalid numeric literal at line 1, column 4092\",\"Invalid numeric literal at line 2, column 0\",9,\"break\",\"break\",\"break\"]\n",
            "",
            0,
        ),
        (
            "no mark, seam",
            READS,
            vec![padded(b"", 4093, b"l 1"), b"\xbb\xbf3\n9\n".to_vec()],
            "[\"Invalid numeric literal at line 1, column 4095\",\"Invalid numeric literal at line 2, column 0\",9,\"break\",\"break\",\"break\"]\n",
            "",
            0,
        ),
        // The issue's second case: the read ends where the token `1`, NUL, `x`, 0x80 does, and the
        // quote that starts the next read opens a string `8` and `9` are inside. jq: the fault, `1`,
        // the unfinished string. The NUL is the second fault now, and the quote still starts the
        // next read.
        (
            "NUL, the read ends at the token's end",
            READS,
            vec![padded(b"}\n", 4091, b"1\0x\x80\"\n8\n9\n")],
            "[\"Unmatched '}' at line 1, column 1\",\"Invalid character at line 1, column 4094\",\"Unfinished string at EOF at line 4, column 0\",\"break\",\"break\",\"break\"]\n",
            "",
            0,
        ),
        // Neither the fault before it nor the invalid byte is needed (jq: `1`, the unfinished string).
        (
            "NUL, no fault before",
            READS,
            vec![padded(b"", 4091, b"1\0x\x80\"\n8\n9\n")],
            "[\"Invalid character at line 1, column 4093\",\"Unfinished string at EOF at line 4, column 0\",\"break\",\"break\",\"break\",\"break\"]\n",
            "",
            0,
        ),
        (
            "NUL, ASCII tail",
            READS,
            vec![padded(b"0\n", 4091, b"1\0xy\"\n8\n9\n")],
            "[0,\"Invalid character at line 2, column 4093\",\"Unfinished string at EOF at line 5, column 0\",\"break\",\"break\",\"break\"]\n",
            "",
            0,
        ),
        (
            "NUL, a space starts the next read",
            READS,
            vec![padded(b"}\n", 4091, b"1\0x\x80 \n8\n9\n")],
            "[\"Unmatched '}' at line 1, column 1\",\"Invalid character at line 1, column 4094\",8,9,\"break\",\"break\"]\n",
            "",
            0,
        ),
        // The read hides the token's tail and white space after it (jq: `1`, 8, 9 and `[1]`, 8, 9).
        (
            "NUL, the read hides a space too",
            READS,
            vec![padded(b"", 4091, b"1\0x \n8\n9\n")],
            "[\"Invalid character at line 1, column 4093\",8,9,\"break\",\"break\",\"break\"]\n",
            "",
            0,
        ),
        (
            "NUL in an array, the read hides white space",
            READS,
            vec![padded(b"", 4089, b"[1\0x \t]\n8\n9\n")],
            "[\"Invalid character at line 1, column 4092\",\"Unmatched ']' at line 1, column 4093\",8,9,\"break\",\"break\"]\n",
            "",
            0,
        ),
        // The token goes on into the next read with another NUL (jq: 12, 7, 8): the second NUL starts
        // the next read and is a fault of its own.
        (
            "NUL, the token goes on with a NUL",
            READS,
            vec![padded(b"", 4092, b"12\0\0 7\n8\n")],
            "[\"Invalid character at line 1, column 4095\",\"Invalid character at line 1, column 4096\",8,\"break\",\"break\",\"break\"]\n",
            "",
            0,
        ),
        // Behind a mark the first read holds three bytes fewer of the text.
        (
            "NUL behind a mark",
            READS,
            vec![padded(MARK, 4088, b"1\0x\x80\"\n8\n9\n")],
            "[\"Invalid character at line 1, column 4090\",\"Unfinished string at EOF at line 4, column 0\",\"break\",\"break\",\"break\",\"break\"]\n",
            "",
            0,
        ),
        // The main loop, which stops at a fault (jq: `1` and `2`; `1` and the unfinished string).
        (
            "NUL, main loop",
            EACH,
            vec![padded(b"", 4091, b"1\0xy 2\n")],
            "",
            "jq: parse error: Invalid character at line 1, column 4093\n",
            5,
        ),
        (
            "NUL, main loop, fault after",
            EACH,
            vec![padded(b"", 4091, b"1\0x\x80\"\n8\n9\n")],
            "",
            "jq: parse error: Invalid character at line 1, column 4093\n",
            5,
        ),
        // A mark cut across files, then a number with a NUL in a read that holds a line break (jq:
        // `1`, `1`, `[1,2]`); the mark is skipped, so the NUL is at column 2 or 3.
        (
            "mark cut 2+1, NUL",
            EACH,
            vec![b"\xef\xbb".to_vec(), b"\xbf1\0x\n".to_vec()],
            "",
            "jq: parse error: Invalid character at line 1, column 2\n",
            5,
        ),
        (
            "mark cut 1+2, NUL",
            EACH,
            vec![b"\xef".to_vec(), b"\xbb\xbf1\0x\n".to_vec()],
            "",
            "jq: parse error: Invalid character at line 1, column 2\n",
            5,
        ),
        (
            "mark cut 1+1+1, NUL in an array",
            EACH,
            vec![b"\xef".to_vec(), b"\xbb".to_vec(), b"\xbf[1\0x,2]\n".to_vec()],
            "",
            "jq: parse error: Invalid character at line 1, column 3\n",
            5,
        ),
    ]
}

fn check(rows: Vec<Row>) -> Result<()> {
    for (name, args, files, stdout, stderr, status) in rows {
        let files: Vec<&[u8]> = files.iter().map(Vec::as_slice).collect();
        let (out, err, code, _) = run_jq_over_byte_files(args, &files)?;
        let err = err.replace("succinctly jq:", "jq:");
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (stdout, stderr, status),
            "{name}"
        );
    }
    Ok(())
}

#[test]
fn a_fault_or_a_nul_at_the_end_of_a_read_is_read_as_jq_reads_it_7921_7966() -> Result<()> {
    check(rows())
}

/// What was #7921's recorded residual: a read with no line break that hides more than its token's
/// tail and white space, where jq 1.7.1 and jq 1.8.2 differ from each other. With #7966 the NUL is
/// the fault in each, and the run reads on from the read after the one holding it; each comment has
/// the two jq answers, neither of which these rows match.
#[test]
fn the_reads_the_two_jq_releases_differ_on_read_the_nul_as_a_fault_7966() -> Result<()> {
    check(vec![
        // One space fewer: the read hides the quote as well. jq 1.7.1: `}` fault, 1, 8, 9.
        // jq 1.8.2: `}` fault, 1, `Unfinished string at EOF at line 4, column 0`.
        (
            "the read hides a quote",
            READS,
            vec![padded(b"}\n", 4090, b"1\0x\x80\"\n8\n9\n")],
            "[\"Unmatched '}' at line 1, column 1\",\"Invalid character at line 1, column 4093\",8,9,\"break\",\"break\"]\n",
            "",
            0,
        ),
        // One space more: the token goes on with a byte of the next read. jq 1.7.1: `}` fault,
        // `Invalid numeric literal at line 1, column 4096`, 8, 9. jq 1.8.2: as above.
        (
            "the token goes on with a byte",
            READS,
            vec![padded(b"}\n", 4092, b"1\0x\x80\"\n8\n9\n")],
            "[\"Unmatched '}' at line 1, column 1\",\"Invalid character at line 1, column 4095\",\"Invalid numeric literal at line 1, column 4097\",8,9,\"break\"]\n",
            "",
            0,
        ),
        // The read hides a space and the next starts with a digit. jq 1.7.1: 12, 8.
        // jq 1.8.2: 1, 2, 8.
        (
            "the next read starts with a digit",
            READS,
            vec![padded(b"", 4091, b"1\0x 2\n8\n")],
            "[\"Invalid character at line 1, column 4093\",2,8,\"break\",\"break\",\"break\"]\n",
            "",
            0,
        ),
        // An invalid byte after a fault in one read of a long line is read where jq reads it since
        // #7931 (`tests/jq_cli/i7931.rs`).
    ])
}
