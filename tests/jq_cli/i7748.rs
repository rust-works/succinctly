//! #7748, re-pinned by #7966: a NUL glued to a number token in the front of a lone source the
//! lazy `input`/`inputs` route reads (its first 256 KiB on standard input, its first 16 KiB of a
//! file), past the first chunk the route scans to start on it. jq 1.7.1 and 1.8.2 read `[12`, NUL,
//! `x,34]` as `[12,34]` (the number parser reads the token as a C string), `[[1]1`, NUL, `]` as
//! `Expected separator between values` at column 7 and `1`, NUL, `}` as `Unmatched '}'` at column 3.
//! Under ADR-0018's #7963 amendment E2 the NUL is the fault at its own column, the token before it
//! checked first, as jq master (unreleased; commit fbee8f4, jqlang/jq#3606) reads it: `Invalid
//! character` at column 4, `Expected separator between values` at column 6 (the `1` after `[1]`
//! is refused at the NUL) and `Invalid character` at column 2. Each first fault was checked against
//! a driver over master's `jv_parse.c`; standard input, `-` and the file answer alike.

use anyhow::Result;

use crate::{run_jq_over_byte_files, spawn_jq};

/// The record the inputs are made of, 21 bytes a line.
const RECORD: &str = "[1234567,\"abcdefgh\"]\n";

/// The programs, each a lazy `inputs` route or a main loop that reads `input_line_number`.
const PROGRAMS: [&[&str]; 4] = [
    &["-n", "-c", "[inputs]|length"],
    &["-n", "-c", "first(inputs|select(.[0]!=1234567))"],
    &["-n", "-c", "reduce inputs as $x (0;.+1)"],
    &["-c", "select(.[0]!=1234567)|[.,input_line_number]"],
];

/// `(records before, the line with the NUL, the programs' (stdout, stderr, exit))`, with 13,000
/// records after it: 280-460 KB, so standard input is read on a front of 256 KiB.
type Row = (usize, &'static str, [(&'static str, &'static str, i32); 4]);

const ROWS: &[Row] = &[
    (
        3000,
        "[12\0x,34]\n",
        [
            ("", "jq: error (at <stdin>:3001): Invalid character at line 3001, column 4\n", 5),
            ("", "jq: error (at <stdin>:3001): Invalid character at line 3001, column 4\n", 5),
            ("", "jq: error (at <stdin>:3001): Invalid character at line 3001, column 4\n", 5),
            ("", "jq: parse error: Invalid character at line 3001, column 4\n", 5),
        ],
    ),
    (
        400,
        "[12\0x,34]\n",
        [
            ("", "jq: error (at <stdin>:401): Invalid character at line 401, column 4\n", 5),
            ("", "jq: error (at <stdin>:401): Invalid character at line 401, column 4\n", 5),
            ("", "jq: error (at <stdin>:401): Invalid character at line 401, column 4\n", 5),
            ("", "jq: parse error: Invalid character at line 401, column 4\n", 5),
        ],
    ),
    (
        9000,
        "[12\0x,34]\n",
        [
            ("", "jq: error (at <stdin>:9001): Invalid character at line 9001, column 4\n", 5),
            ("", "jq: error (at <stdin>:9001): Invalid character at line 9001, column 4\n", 5),
            ("", "jq: error (at <stdin>:9001): Invalid character at line 9001, column 4\n", 5),
            ("", "jq: parse error: Invalid character at line 9001, column 4\n", 5),
        ],
    ),
    (
        3000,
        "[[1]1\0]\n",
        [
            ("", "jq: error (at <stdin>:3001): Expected separator between values at line 3001, column 6\n", 5),
            ("", "jq: error (at <stdin>:3001): Expected separator between values at line 3001, column 6\n", 5),
            ("", "jq: error (at <stdin>:3001): Expected separator between values at line 3001, column 6\n", 5),
            ("", "jq: parse error: Expected separator between values at line 3001, column 6\n", 5),
        ],
    ),
    (
        3000,
        "1\0}\n",
        [
            ("", "jq: error (at <stdin>:3001): Invalid character at line 3001, column 2\n", 5),
            ("", "jq: error (at <stdin>:3001): Invalid character at line 3001, column 2\n", 5),
            ("", "jq: error (at <stdin>:3001): Invalid character at line 3001, column 2\n", 5),
            ("", "jq: parse error: Invalid character at line 3001, column 2\n", 5),
        ],
    ),
];

#[test]
fn a_nul_in_the_lazy_front_is_a_fault_at_its_own_column_7966() -> Result<()> {
    for (before, line, answers) in ROWS {
        let text = [
            RECORD.repeat(*before).as_str(),
            *line,
            &RECORD.repeat(13_000),
        ]
        .concat();
        assert!(
            text.len() > 256 * 1024,
            "the input is longer than the front"
        );
        for (args, (stdout, stderr, exit)) in PROGRAMS.iter().zip(answers) {
            let (output, code) = spawn_jq(args, Some(text.as_bytes()))?;
            let (out, err) = (
                String::from_utf8(output.stdout)?,
                String::from_utf8(output.stderr)?,
            );
            assert_eq!(
                (out.as_str(), err.as_str(), code),
                (*stdout, *stderr, *exit),
                "stdin {args:?} {before} {line:?}"
            );
            let dash = [*args, &["-"]].concat();
            let (output, code) = spawn_jq(&dash, Some(text.as_bytes()))?;
            let (out, err) = (
                String::from_utf8(output.stdout)?,
                String::from_utf8(output.stderr)?,
            );
            assert_eq!(
                (out.as_str(), err.as_str(), code),
                (*stdout, *stderr, *exit),
                "- {args:?} {before} {line:?}"
            );
            let (out, err, code, paths) = run_jq_over_byte_files(args, &[text.as_bytes()])?;
            let err = err.replace(&paths[0], "<stdin>");
            assert_eq!(
                (out.as_str(), err.as_str(), code),
                (*stdout, *stderr, *exit),
                "file {args:?} {before} {line:?}"
            );
        }
    }
    Ok(())
}
