//! #5613: jq 1.7.1's lexer skips only space, tab and newline between tokens; any other whitespace
//! character is `INVALID_CHARACTER` (compile error, exit 3), so a CRLF program is refused at its
//! first `\r`. `succinctly jq` used to skip everything `char::is_whitespace` accepts, and read
//! a `-f` program file through `trim()`, which also dropped a leading or trailing CR. Every row
//! was captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// `(program, expected stdout, expected exit status)`, run as `-nc <program>`.
const ARGUMENT_ROWS: &[(&str, &str, i32)] = &[
    ("1\r+ 1", "", 3),
    ("1\r\n+ 1", "", 3),
    ("\r1", "", 3),
    ("1\r", "", 3),
    ("[1,\r2]", "", 3),
    ("1\x0c+1", "", 3),
    ("1\x0b+1", "", 3),
    ("1\u{a0}+1", "", 3),
    ("1\u{2028}+1", "", 3),
    ("[1] | add\r", "", 3),
    ("def f: 1;\r f", "", 3),
    ("\"a\\(1\r+2)\"", "", 3),
    ("1 # c\r\n+ 1", "2\n", 0),
    ("1\t+\t1", "2\n", 0),
    ("1 +\n1", "2\n", 0),
    ("[1] | add \n", "1\n", 0),
];

#[test]
fn test_a_program_argument_reads_only_space_tab_and_newline_as_whitespace_5613() -> Result<()> {
    for (program, stdout, status) in ARGUMENT_ROWS {
        let (out, _, code) = run_jq_full(&["-nc", program], None)?;
        assert_eq!((out.as_str(), code), (*stdout, *status), "{program:?}");
    }
    Ok(())
}

/// `-f` reads the file as written: a trailing `\r\n` is an error in jq, not a trimmed newline.
#[test]
fn test_a_program_file_is_not_trimmed_before_it_is_parsed_5613() -> Result<()> {
    let dir = tempfile::tempdir()?;
    for (text, stdout, status) in [
        ("1\r\n", "", 3),
        ("1\r", "", 3),
        ("\r1", "", 3),
        ("1\n", "1\n", 0),
        ("\n\n 1 \n", "1\n", 0),
        ("# c\r\n1", "1\n", 0),
        ("\t.\t", "null\n", 0),
    ] {
        let path = dir.path().join("prog.jq");
        std::fs::write(&path, text)?;
        let (out, _, code) = run_jq_full(&["-nc", "-f", path.to_str().unwrap()], None)?;
        assert_eq!((out.as_str(), code), (stdout, status), "{text:?}");
    }
    Ok(())
}
