//! #7105: how yq counts parentheses inside an interpolation slot decides whether the string has an
//! unclosed `\(` (#7063), and with one it keeps the whole string as written. Inside a slot:
//!
//! - every `(` is an open, whatever precedes it: the `\(` of `"\(z\(1)"` is a second open, so that string is
//!   unclosed and reads as written;
//! - a `)` is a close unless the run of backslashes before it, collapsed once (`ceil(n / 2)`), is odd: `\)`
//!   and `\\)` do not close a slot and `\\\)` does (a closed slot that holds `\)` is then refused as a program);
//! - a `"` after an odd run does not end the slot.
//!
//! `succinctly yq` skipped the character after a backslash in that scan, so it counted neither the `(` of a
//! nested `\(` nor read `\\)` as escaped, and refused strings that yq reads as text. Every row was captured
//! from yq v4.53.3 (`yq -n -o=json -I0 --from-file`, so no shell touches the backslashes).
//!
//! Not covered: a slot that does not parse followed by an unclosed `\(` (`"\( + )\("`), which yq refuses
//! because it compiles the slots in order, and this reads as text (limitations.md).

use std::process::{Command, Stdio};

use anyhow::Result;
use tempfile::TempDir;

fn run(program: &str) -> Result<(String, Option<i32>)> {
    let dir = TempDir::new()?;
    let file = dir.path().join("p.yq");
    std::fs::write(&file, program)?;
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .args(["yq", "-n", "-o=json", "-I0", "--from-file"])
        .arg(&file)
        .stdin(Stdio::null())
        .output()?;
    Ok((
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.code(),
    ))
}

/// `(program, stdout)`.
const ROWS: &[(&str, &str)] = &[
    ("\"\\(1\\(\"", "\"\\\\(1\\\\(\"\n"),
    ("\"\\(1\\()\"", "\"\\\\(1\\\\()\"\n"),
    ("\"\\(1\\)\"", "\"\\\\(1\\\\)\"\n"),
    ("\"\\(1\\\\(\"", "\"\\\\(1\\\\\\\\(\"\n"),
    ("\"\\(1\\\\()\"", "\"\\\\(1\\\\\\\\()\"\n"),
    ("\"\\(1\\\\)\"", "\"\\\\(1\\\\)\"\n"),
    ("\"\\(1\\\\\\(\"", "\"\\\\(1\\\\\\\\(\"\n"),
    ("\"\\(1\\\\\\()\"", "\"\\\\(1\\\\\\\\()\"\n"),
    ("\"\\(1\\\\\\\\(\"", "\"\\\\(1\\\\\\\\\\\\(\"\n"),
    ("\"\\(1\\\\\\\\()\"", "\"\\\\(1\\\\\\\\\\\\()\"\n"),
    ("\"\\(1(\"", "\"\\\\(1(\"\n"),
    ("\"\\(1()\"", "\"\\\\(1()\"\n"),
    ("\"\\(1)\"", "\"1\"\n"),
    ("\"\\(1))\"", "\"1)\"\n"),
    ("\"\\(1)))\"", "\"1))\"\n"),
    ("\"\\(1\\(2\"", "\"\\\\(1\\\\(2\"\n"),
    ("\"\\(1\\(2)\"", "\"\\\\(1\\\\(2)\"\n"),
    ("\"\\(1\\)(\"", "\"\\\\(1\\\\)(\"\n"),
    ("\"\\(1\\)()\"", "\"\\\\(1\\\\)()\"\n"),
    ("\"\\(1(2\"", "\"\\\\(1(2\"\n"),
    ("\"\\(1(2)\"", "\"\\\\(1(2)\"\n"),
    ("\"\\(z\\(1)\"", "\"\\\\(z\\\\(1)\"\n"),
    ("\"a\\(z\\(1)\"", "\"a\\\\(z\\\\(1)\"\n"),
    ("\"\\(1 + (2\\))\"", "\"\\\\(1 + (2\\\\))\"\n"),
    ("\"x\\(1\\()y\"", "\"x\\\\(1\\\\()y\"\n"),
    ("\"x\\(1\\\\)y\"", "\"x\\\\(1\\\\)y\"\n"),
    ("\"\\(1)\\(2\\)\"", "\"\\\\(1)\\\\(2\\\\)\"\n"),
    ("\"\\(1)\\(2\\()\"", "\"\\\\(1)\\\\(2\\\\()\"\n"),
    ("\"\\(1)\\(2)\\(3\\)\"", "\"\\\\(1)\\\\(2)\\\\(3\\\\)\"\n"),
    ("\"a\\(1\\()\\(2)\"", "\"a\\\\(1\\\\()\\\\(2)\"\n"),
];

/// Strings yq refuses (a closed slot that is not an expression) and so does this.
const REFUSED: &[&str] = &[
    "\"\\(1\\())\"",
    "\"\\(1\\))\"",
    "\"\\(1\\)))\"",
    "\"\\(1\\\\())\"",
    "\"\\(1\\\\))\"",
    "\"\\(1\\\\)))\"",
    "\"\\(1\\\\\\())\"",
    "\"\\(1\\\\\\)\"",
    "\"\\(1\\\\\\))\"",
    "\"\\(1\\\\\\)))\"",
    "\"\\(1\\\\\\\\)\"",
    "\"\\(1\\\\\\\\))\"",
    "\"\\(1\\\\\\\\)))\"",
    "\"\\(1\\\\\\\\())\"",
    "\"\\(1\\(2))\"",
    "\"\\(1\\(2)))\"",
    "\"\\(1\\)())\"",
    "\"\\(1(2))\"",
    "\"\\(1(2)))\"",
    "\"\\(z\\(1))\"",
    "\"x\\(1\\\\\\)y\"",
    // A run at the very end of the text: the string never ends, which is the parser's own error.
    "\"\\(1\\",
    "\"\\(1\\\\",
];

#[test]
fn a_nested_open_and_an_escaped_close_inside_a_slot_read_as_yq_does_7105() -> Result<()> {
    for &(program, want) in ROWS {
        assert_eq!(run(program)?, (want.to_string(), Some(0)), "{program}");
    }
    Ok(())
}

#[test]
fn a_closed_slot_that_is_not_an_expression_is_refused_7105() -> Result<()> {
    for &program in REFUSED {
        let (stdout, code) = run(program)?;
        assert_eq!((stdout.as_str(), code), ("", Some(1)), "{program}");
    }
    Ok(())
}
