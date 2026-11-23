//! #7748: a NUL glued to a number token in the front of a lone source the lazy `input`/`inputs` route
//! reads (its first 256 KiB on standard input, its first 16 KiB of a file), past the first chunk the
//! route scans to start on it. #7437 rewrote the windows read after the front and the sources read
//! whole, not the front itself, so `[12`, NUL, `x,34]` there was refused on standard input
//! (`Invalid JSON text: expected ',' or ']', found '\0'`), where jq reads `[12,34]`. Every answer
//! was captured from jq 1.7.1 (`/usr/bin/jq`), and jq 1.8.2 (`/opt/homebrew/bin/jq`) gives the same.

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
            ("16001\n", "", 0),
            ("[12,34]\n", "", 0),
            ("16001\n", "", 0),
            ("[[12,34],3001]\n", "", 0),
        ],
    ),
    (
        400,
        "[12\0x,34]\n",
        [
            ("13401\n", "", 0),
            ("[12,34]\n", "", 0),
            ("13401\n", "", 0),
            ("[[12,34],401]\n", "", 0),
        ],
    ),
    (
        9000,
        "[12\0x,34]\n",
        [
            ("22001\n", "", 0),
            ("[12,34]\n", "", 0),
            ("22001\n", "", 0),
            ("[[12,34],9001]\n", "", 0),
        ],
    ),
    (
        3000,
        "[[1]1\0]\n",
        [
            ("", "jq: error (at <stdin>:3001): Expected separator between values at line 3001, column 7\n", 5),
            ("", "jq: error (at <stdin>:3001): Expected separator between values at line 3001, column 7\n", 5),
            ("", "jq: error (at <stdin>:3001): Expected separator between values at line 3001, column 7\n", 5),
            ("", "jq: parse error: Expected separator between values at line 3001, column 7\n", 5),
        ],
    ),
    (
        3000,
        "1\0}\n",
        [
            ("", "jq: error (at <stdin>:3001): Unmatched '}' at line 3001, column 3\n", 5),
            ("", "jq: error (at <stdin>:3001): Unmatched '}' at line 3001, column 3\n", 5),
            ("", "jq: error (at <stdin>:3001): Unmatched '}' at line 3001, column 3\n", 5),
            ("", "jq: parse error: Unmatched '}' at line 3001, column 3\n", 5),
        ],
    ),
];

#[test]
fn a_nul_tail_in_the_lazy_front_is_read_as_jq_reads_it_7748() -> Result<()> {
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
