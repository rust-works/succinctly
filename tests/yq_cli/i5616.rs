//! #5616: yq v4.53.3's lexer skips only space, tab and newline between tokens; CR, form feed,
//! vertical tab, NBSP, U+0085, U+2028 and U+3000 are `lexer: invalid input text`. `--from-file`
//! reads the file with every CRLF turned into LF (even inside a string literal) and nothing else
//! touched, so a lone CR in a file is refused like one in an argument. Every row was captured
//! from yq v4.53.3.

use std::process::Command;

use anyhow::Result;

/// `(program, expected stdout)`; `None` is a refused program (exit status 1).
const ROWS: &[(&str, Option<&str>)] = &[
    ("1\r+ 1", None),
    ("1\r\n+ 1", None),
    ("\r1", None),
    ("1\r", None),
    ("1\r\n", None),
    ("[1,\x0c2]", None),
    ("1\x0b+1", None),
    ("1\u{a0}+1", None),
    ("1\u{85}+1", None),
    ("1\u{2028}+1", None),
    ("1\u{3000}+1", None),
    ("\"a\\(1\r+2)\"", None),
    ("1 # c\r\n+ 1", Some("2\n")),
    ("1 # a\rb\n+ 1", Some("2\n")),
    ("1\t+\t1", Some("2\n")),
    ("1 +\n1", Some("2\n")),
];

fn run(args: &[&str]) -> Result<(String, i32)> {
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .args(args)
        .output()?;
    Ok((
        String::from_utf8(output.stdout)?,
        output.status.code().unwrap_or(-1),
    ))
}

#[test]
fn test_an_expression_argument_reads_only_space_tab_and_newline_as_whitespace_5616() -> Result<()> {
    for (program, expected) in ROWS {
        let (out, code) = run(&["-n", program])?;
        match expected {
            Some(stdout) => assert_eq!((out.as_str(), code), (*stdout, 0), "{program:?}"),
            None => assert_eq!((out.as_str(), code), ("", 1), "{program:?}"),
        }
    }
    Ok(())
}

#[test]
fn test_an_expression_file_has_its_crlf_turned_into_lf_and_nothing_else_5616() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("prog.yq");
    for (text, expected) in [
        ("1\r\n", Some("1\n")),
        ("1\r\n+1", Some("2\n")),
        ("1\n\r\n", Some("1\n")),
        ("\r\n1", Some("1\n")),
        ("\"a\r\nb\"", Some("a\nb\n")),
        ("\"a\rb\"", Some("a\rb\n")),
        ("1\r", None),
        ("\r1", None),
        ("1\r\r\n", None),
        ("1\r \n", None),
        ("1\u{a0}", None),
        ("1 #c\r\n", Some("1\n")),
    ] {
        std::fs::write(&path, text)?;
        let (out, code) = run(&["-n", "--from-file", path.to_str().unwrap()])?;
        match expected {
            Some(stdout) => assert_eq!((out.as_str(), code), (stdout, 0), "{text:?}"),
            None => assert_eq!((out.as_str(), code), ("", 1), "{text:?}"),
        }
    }
    Ok(())
}
