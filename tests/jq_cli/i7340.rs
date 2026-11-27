//! #7340, re-pinned by #7966: jq's `--seq` reader treated a NUL byte two ways that are not an
//! ordinary byte's. jq 1.7.1 measures an `fgets` chunk that holds no newline with `strlen`
//! (`jq_util_input_read_more`, `src/util.c`), so its parser never sees the chunk from its first NUL
//! on; and both releases read a number token as a C string (`check_literal`), so a NUL in one ends
//! the number. Under ADR-0018's #7963 amendment E2 `succinctly jq` reproduces neither: a NUL outside
//! a string is `Invalid character` at its own line and column, the token before it checked first,
//! as jq master (unreleased; commit fbee8f4, jqlang/jq#3606) reads it. Each first fault below was
//! captured from a driver over master's `jv_parse.c` with `JV_PARSE_SEQ`; what follows a fault is
//! the reader's best effort (E1, #7967), which today reads on in jq 1.7.1's and 1.8.2's way: their
//! `parser_reset` undoes the wait for the next RS (master waits).
//!
//! Each row is run as a plain read, a slurp and an `input` loop, the three routes that build the
//! values. jq 1.7.1 (`/usr/bin/jq`, macOS) and jq 1.8.2 (`/opt/homebrew/bin/jq`) answer each row
//! as the comment on it says, so every row with a NUL outside a string differs from both releases.
//! The rows with no NUL outside a string (one in a string, one before the first RS) are jq 1.8.2's
//! answer, which reads through the NUL where jq 1.7.1 hid the rest of the read.

use anyhow::Result;

use crate::run_jq_over_byte_files;

const RS: &str = "\u{1e}";

/// `(name, files, [plain, slurp, inputs])`, each route's `(stdout, stderr)`; every run exits 0.
type Row = (&'static str, Vec<Vec<u8>>, [(String, String); 3]);

fn warn(body: &str) -> String {
    format!("jq: ignoring parse error: {body}\n")
}

fn rows() -> Vec<Row> {
    let breaks = |n: usize| ",\"break\"".repeat(n);
    let control = "Invalid string: control characters from U+0000 through U+001F must be escaped";
    vec![
        (
            // The issue's first repro. Both releases read `1` and give no warning.
            "a NUL straight after a number",
            vec![b"\x1e\"ab\"\n\x1e1\x00\n\x1e7 \n\x1e{\"id\":1}\n".to_vec()],
            {
                let w = "Invalid character at line 2, column 3 (need RS to resync)";
                [
                    (format!("{RS}\"ab\"\n{RS}7\n{RS}{{\"id\":1}}\n"), warn(w)),
                    (format!("{RS}[\"ab\",7,{{\"id\":1}}]\n"), warn(w)),
                    (
                        format!("{RS}[\"ab\",\"{w}\",7,{{\"id\":1}}{}]\n", breaks(4)),
                        String::new(),
                    ),
                ]
            },
        ),
        (
            // The issue's second repro. jq 1.7.1 never parses ` 7 [8] ` and the spaces after it;
            // jq 1.8.2 reads them, with `Invalid numeric literal at line 2, column 8` for the NUL.
            "a NUL in a chunk with no newline",
            vec![[
                &b"\x1e{\"id\":1}\n\x1e\"ab\" \x00 7 [8] "[..],
                &[b' '; 5000],
                b"9 10\n\x1e{\"id\":2}\n",
            ]
            .concat()],
            {
                let w = "Invalid character at line 2, column 7 (need RS to resync)";
                [
                    (
                        format!(
                            "{RS}{{\"id\":1}}\n{RS}\"ab\"\n{RS}7\n{RS}[8]\n{RS}9\n{RS}10\n{RS}{{\"id\":2}}\n"
                        ),
                        warn(w),
                    ),
                    (
                        format!("{RS}[{{\"id\":1}},\"ab\",7,[8],9,10,{{\"id\":2}}]\n"),
                        warn(w),
                    ),
                    (
                        format!("{RS}[{{\"id\":1}},\"ab\",\"{w}\",7,[8],9,10,{{\"id\":2}}]\n"),
                        String::new(),
                    ),
                ]
            },
        ),
        (
            // Both releases read `[1]`, `{"a":2.50}` and `nan` (as `null`), and refuse `true` then
            // NUL as `Invalid literal at line 5, column 0` and `-` then NUL as `Invalid numeric
            // literal at line 6, column 0`, at the newline that ends each token. Master checks the
            // token at the NUL: `-` is refused there as a number, the rest are `Invalid character`.
            "numbers ended by a NUL",
            vec![
                b"\x1e[1\x00]\n\x1e{\"a\":2.50\x00x}\n\x1enan\x00\n\x1etrue\x00\n\x1e-\x00\n"
                    .to_vec(),
            ],
            {
                let ws = [
                    "Invalid character at line 1, column 4 (need RS to resync)",
                    "Unmatched ']' at line 1, column 5 (need RS to resync)",
                    "Invalid character at line 2, column 11 (need RS to resync)",
                    "Invalid numeric literal at line 2, column 13 (need RS to resync)",
                    "Invalid character at line 3, column 5 (need RS to resync)",
                    "Invalid character at line 4, column 6 (need RS to resync)",
                    "Invalid numeric literal at line 5, column 3 (need RS to resync)",
                ];
                let stderr: String = ws.iter().map(|w| warn(w)).collect();
                let caught: String = ws.iter().map(|w| format!("\"{w}\",")).collect();
                [
                    (String::new(), stderr.clone()),
                    (format!("{RS}[]\n"), stderr),
                    (format!("{RS}[{caught}\"break\"]\n"), String::new()),
                ]
            },
        ),
        (
            // jq 1.7.1 never sees the NUL and `garbage`, and reads `[1,2]`; jq 1.8.2 refuses the
            // token `\0garbage` at the space after it (column 4013), then the `]`.
            "an array across the dropped bytes",
            vec![[
                &b"\x1e[1,"[..],
                &[b' '; 4000],
                b"\x00garbage",
                &[b' '; 200],
                b"2]\n",
            ]
            .concat()],
            {
                let ws = [
                    "Invalid character at line 1, column 4005 (need RS to resync)",
                    "Invalid numeric literal at line 1, column 4013 (need RS to resync)",
                    "Unmatched ']' at line 1, column 4214 (need RS to resync)",
                ];
                let stderr: String = ws.iter().map(|w| warn(w)).collect();
                let caught: String = ws.iter().map(|w| format!("\"{w}\",")).collect();
                [
                    (String::new(), stderr.clone()),
                    (format!("{RS}[]\n"), stderr),
                    (
                        format!("{RS}[{caught}\"break\"{}]\n", breaks(4)),
                        String::new(),
                    ),
                ]
            },
        ),
        (
            // Unchanged by #7966: a NUL in a string is a control character, as jq 1.8.2 refuses
            // it. jq 1.7.1 never saw it, and read the string without the 3 bytes after it.
            "a string across the dropped bytes",
            vec![[
                &b"\x1e\"x"[..],
                &[b'y'; 4000],
                b"\x00zz",
                &[b'y'; 200],
                b"\"\n",
            ]
            .concat()],
            {
                let w = format!("{control} at line 1, column 4207 (need RS to resync)");
                [
                    (String::new(), warn(&w)),
                    (format!("{RS}[]\n"), warn(&w)),
                    (format!("{RS}[\"{w}\"{}]\n", breaks(7)), String::new()),
                ]
            },
        ),
        (
            // A file's last chunk is no longer cut at a NUL: the fault is at the NUL, and the run
            // reads on into the next file. jq 1.7.1 reads `[1,2]`; jq 1.8.2 refuses the token
            // `\0zz2` at the `]` (column 109).
            "a NUL in a file's last chunk",
            vec![
                [&b"\x1e[1,"[..], &[b' '; 100], b"\x00zz"].concat(),
                b"2]\n".to_vec(),
            ],
            {
                let ws = [
                    "Invalid character at line 1, column 105 (need RS to resync)",
                    "Invalid numeric literal at line 1, column 109 (need RS to resync)",
                ];
                let stderr: String = ws.iter().map(|w| warn(w)).collect();
                let caught: String = ws.iter().map(|w| format!("\"{w}\",")).collect();
                [
                    (String::new(), stderr.clone()),
                    (format!("{RS}[]\n"), stderr),
                    (
                        format!("{RS}[{caught}\"break\"{}]\n", breaks(5)),
                        String::new(),
                    ),
                ]
            },
        ),
        (
            // An RS after a NUL is an RS: `2` is a truncated record and `3` one. jq 1.7.1 never
            // sees either and gives `1` and `4`; jq 1.8.2 gives `1`, `3` and `4`, with `Truncated
            // value at line 1, column 5` for the NUL's token.
            "an RS after a NUL",
            vec![[&b"\x1e1 \x00\x1e2\x1e3 "[..], &[b' '; 4100], b"\n\x1e4\n"].concat()],
            {
                let ws = [
                    "Invalid character at line 1, column 4 (need RS to resync)",
                    "Potentially truncated top-level numeric value at line 1, column 7",
                ];
                let stderr: String = ws.iter().map(|w| warn(w)).collect();
                [
                    (format!("{RS}1\n{RS}3\n{RS}4\n"), stderr.clone()),
                    (format!("{RS}[1,3,4]\n"), stderr),
                    (
                        format!("{RS}[1,\"{}\",\"{}\",3,4{}]\n", ws[0], ws[1], breaks(3)),
                        String::new(),
                    ),
                ]
            },
        ),
        (
            // With no RS byte at all, nothing is scanned, so the NUL is no fault, and every byte
            // is counted (jq 1.7.1 stopped the column at the NUL, column 2; jq 1.8.2 says 5).
            "no RS byte, and a NUL",
            vec![b"ab\x00cd".to_vec()],
            {
                let w = "Unfinished abandoned text at EOF at line 1, column 5";
                [
                    (String::new(), warn(w)),
                    (format!("{RS}[]\n"), warn(w)),
                    (format!("{RS}[\"{w}\"{}]\n", breaks(7)), String::new()),
                ]
            },
        ),
        (
            // A NUL after `EF BB` makes the BOM malformed, so the parser reads from it on (jq's
            // `parser_reset` leaves it reading), and the NUL is its fault at column 1. jq 1.7.1
            // never sees the NUL (`Unfinished abandoned text at EOF at line 1, column 0`); jq 1.8.2
            // says nothing at all here, and with a newline after `x` both releases say `Invalid
            // numeric literal at line 2, column 0`.
            "a NUL after a partial BOM",
            vec![b"\xef\xbb\x00x".to_vec()],
            {
                let w = "Invalid character at line 1, column 1 (need RS to resync)";
                [
                    (String::new(), warn(w)),
                    (format!("{RS}[]\n"), warn(w)),
                    (format!("{RS}[\"{w}\"{}]\n", breaks(7)), String::new()),
                ]
            },
        ),
        (
            "a NUL after a partial BOM, in the next file",
            vec![b"\xef".to_vec(), b"\x00x".to_vec()],
            {
                let w = "Invalid character at line 1, column 1 (need RS to resync)";
                [
                    (String::new(), warn(w)),
                    (format!("{RS}[]\n"), warn(w)),
                    (format!("{RS}[\"{w}\"{}]\n", breaks(7)), String::new()),
                ]
            },
        ),
        (
            // jq 1.7.1 completes the BOM across the bytes it never sees and reads `1`; jq 1.8.2
            // gives `Invalid numeric literal at line 1, column 2`, the truncation and `1`.
            "a BOM across the dropped bytes",
            vec![[&b"\xef\xbb\x00"[..], &[b' '; 4092], b"\xbf\x1e1\n"].concat()],
            {
                let ws = [
                    "Invalid character at line 1, column 1 (need RS to resync)",
                    "Truncated value at line 1, column 4095",
                ];
                let stderr: String = ws.iter().map(|w| warn(w)).collect();
                [
                    (format!("{RS}1\n"), stderr.clone()),
                    (format!("{RS}[1]\n"), stderr),
                    (
                        format!("{RS}[\"{}\",\"{}\",1{}]\n", ws[0], ws[1], breaks(5)),
                        String::new(),
                    ),
                ]
            },
        ),
        (
            // A NUL before the first RS is never scanned; both releases agree here.
            "an RS after a NUL, then one in the next chunk",
            vec![b"x\nab\x00cd\x1e1\nzz".to_vec()],
            {
                let w = "Invalid numeric literal at EOF at line 3, column 2";
                [
                    (format!("{RS}1\n"), warn(w)),
                    (format!("{RS}[1]\n"), warn(w)),
                    (format!("{RS}[1,\"{w}\"{}]\n", breaks(6)), String::new()),
                ]
            },
        ),
    ]
}

#[test]
fn a_nul_outside_a_string_is_a_fault_at_its_own_column_7966() -> Result<()> {
    let routes: [&[&str]; 3] = [
        &["--seq", "-c", "."],
        &["--seq", "-s", "-c", "."],
        &["--seq", "-nc", "[limit(8; repeat(try input catch .))]"],
    ];
    for (name, files, want) in rows() {
        let files: Vec<&[u8]> = files.iter().map(Vec::as_slice).collect();
        for (args, (stdout, stderr)) in routes.iter().zip(&want) {
            let (got_out, got_err, code, _) = run_jq_over_byte_files(args, &files)?;
            assert_eq!(
                (got_out.as_str(), got_err.as_str(), code),
                (stdout.as_str(), stderr.as_str(), 0),
                "{name}: {args:?}"
            );
        }
    }
    Ok(())
}
