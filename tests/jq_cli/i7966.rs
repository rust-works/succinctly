//! #7966: a NUL byte outside a string is a parse fault at the NUL's own line and column, on every
//! route that parses JSON input (ADR-0018's #7963 amendment E2).
//!
//! jq 1.7.1 and 1.8.2 have no such fault. Both read a NUL into the bare token it is in, and their
//! number parser reads the token as a C string, so `[1`, NUL, `x,2]` and a newline is `[1,2]`; a
//! token that starts with a NUL, or follows `true`, is refused one byte later than the NUL (`Invalid
//! numeric literal`, `Invalid literal`). jq 1.7.1 also hands its parser a read with no newline only
//! up to its first NUL, so the rest of that read is lost without a word. `succinctly jq` reproduces
//! none of that. It reads the NUL as jq master does (unreleased; commit fbee8f4, jqlang/jq#3606,
//! "Reject raw NUL bytes in the parser instead of silently truncating tokens"): `classify` answers
//! `INVALID` for it, so the token pending before it is checked first, a value that token completes
//! is dropped, and the fault is `Invalid character` at the NUL's column.
//!
//! The rows were captured from a driver compiled over jq master 5c001b1's `jv_parse.c` (and the rest
//! of its `jv*.c`), which feeds the bytes to `jv_parser_new`/`jv_parser_set_buf`/`jv_parser_next`
//! and prints each value or message (flags `0`, `JV_PARSE_STREAMING`, `JV_PARSE_SEQ`):
//!
//! | input           | jq master                                                       |
//! |-----------------|-----------------------------------------------------------------|
//! | NUL             | `Invalid character at line 1, column 1`                         |
//! | `1 `, NUL, ` 2` | `1`, then `Invalid character at line 1, column 3`               |
//! | `1`, NUL        | `Invalid character at line 1, column 2` (the `1` is dropped)    |
//! | `[1`, NUL, `x,2]` | `Invalid character at line 1, column 3`                       |
//! | `true`, NUL     | `Invalid character at line 1, column 5`                         |
//! | `tr`, NUL, `ue` | `Invalid literal at line 1, column 3`                           |
//! | `-`, NUL        | `Invalid numeric literal at line 1, column 2`                   |
//! | `1`, newline, two spaces, NUL | `1`, then `Invalid character at line 2, column 3` |
//! | `"a`, NUL, `b"` | the control-character error at line 1, column 5, unchanged      |
//!
//! Each route below has a row of its own. What follows the fault is the reader's best effort
//! (E1, #7967), as after any other fault. Unchanged: a NUL in a string, `-R`, `tonumber` (#7521)
//! and a NUL in a region no filter reads (#3035). `--argjson` and `--jsonargs` take their text from
//! the command line, which cannot hold a NUL.

use std::process::Command;

use anyhow::Result;

use super::i7967::masked;
use crate::small_window::{self, MIN_BYTES};
use crate::{run_jq_full, run_jq_over_byte_files};

/// `(stdin, stdout, stderr, exit)` for `jq -c .`, one row of the table above each.
const MAIN_ROWS: &[(&str, &str, &str, i32)] = &[
    (
        "\0",
        "",
        "jq: parse error: Invalid character at line 1, column 1\n",
        5,
    ),
    (
        "1 \0 2",
        "1\n",
        "jq: parse error: Invalid character at line 1, column 3\n",
        5,
    ),
    (
        "1\0",
        "",
        "jq: parse error: Invalid character at line 1, column 2\n",
        5,
    ),
    (
        "[1\0x,2]\n",
        "",
        "jq: parse error: Invalid character at line 1, column 3\n",
        5,
    ),
    (
        "true\0",
        "",
        "jq: parse error: Invalid character at line 1, column 5\n",
        5,
    ),
    (
        "tr\0ue",
        "",
        "jq: parse error: Invalid literal at line 1, column 3\n",
        5,
    ),
    (
        "-\0",
        "",
        "jq: parse error: Invalid numeric literal at line 1, column 2\n",
        5,
    ),
    (
        "1\n  \0",
        "1\n",
        "jq: parse error: Invalid character at line 2, column 3\n",
        5,
    ),
    (
        "\"a\0b\"",
        "",
        "jq: parse error: Invalid string: control characters from U+0000 through U+001F must be escaped at line 1, column 5\n",
        5,
    ),
];

/// The whole read, on standard input and from a file.
#[test]
fn the_whole_read_reports_the_nul_at_its_own_column_7966() -> Result<()> {
    for (input, stdout, stderr, exit) in MAIN_ROWS {
        assert_eq!(
            run_jq_full(&["-c", "."], Some(input))?,
            ((*stdout).to_string(), (*stderr).to_string(), *exit),
            "stdin {input:?}"
        );
        let (out, err, code, _) = run_jq_over_byte_files(&["-c", "."], &[input.as_bytes()])?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (*stdout, *stderr, *exit),
            "file {input:?}"
        );
    }
    Ok(())
}

/// The lazy `input`/`inputs` route: the fault is the error `input` raises, and a program that reads
/// on past it goes on at jq's next read, the next line here.
#[test]
fn input_and_inputs_raise_the_nul_7966() -> Result<()> {
    let text = "1\n[1\0x,2]\n3\n";
    assert_eq!(
        run_jq_full(&["-nc", "[inputs]"], Some(text))?,
        (
            String::new(),
            "jq: error (at <stdin>:2): Invalid character at line 2, column 3\n".to_string(),
            5
        )
    );
    assert_eq!(
        run_jq_full(
            &["-nc", "[limit(4; repeat(try input catch .))]"],
            Some(text)
        )?,
        (
            "[1,\"Invalid character at line 2, column 3\",3,\"break\"]\n".to_string(),
            String::new(),
            0
        )
    );
    Ok(())
}

/// `zeros` lines of `0`, a line with a NUL glued to a number, and `3`: past [`MIN_BYTES`], so a file
/// or a pipe of it is read a window at a time.
fn windowed_text(zeros: usize) -> Vec<u8> {
    let mut text = b"0\n".repeat(zeros);
    text.extend_from_slice(b"[1\0x,2]\n3\n");
    text
}

/// The windowed file and pipe routes: the values before the NUL, then the fault, as the whole read
/// gives it (a NUL in a window is not rewritten into a number, as before #7966).
#[test]
fn a_windowed_file_and_pipe_report_the_nul_7966() -> Result<()> {
    let zeros = MIN_BYTES / 2 + 10;
    let text = windowed_text(zeros);
    assert!(text.len() > MIN_BYTES);
    let stdout = "0\n".repeat(zeros);
    let stderr = format!(
        "jq: parse error: Invalid character at line {}, column 3\n",
        zeros + 1
    );
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("in.json");
    std::fs::write(&path, &text)?;
    let output = small_window::apply(Command::new(env!("CARGO_BIN_EXE_succinctly")).arg("jq"))
        .args(["-c", "."])
        .arg(&path)
        .output()?;
    assert_eq!(
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code(),
        ),
        (stdout.clone(), stderr.clone(), Some(5)),
        "file"
    );
    #[cfg(unix)]
    {
        let (output, code) = small_window::spawn_jq(&["-c", "."], Some(&text))?;
        assert_eq!(
            (
                String::from_utf8_lossy(&output.stdout).into_owned(),
                String::from_utf8_lossy(&output.stderr).into_owned(),
                code,
            ),
            (stdout, stderr, 5),
            "pipe"
        );
    }
    Ok(())
}

/// #7904's shape: several files read a window at a time, a fault at the end of the first, and a
/// number with a NUL in the second, read by `input` past the fault. Both releases give the fault and
/// `2`; the NUL is the second fault now, at column 4 of the line the first fault is on (jq drops the
/// rest of the read a fault is met in, the newline included, so the line is not counted).
#[test]
fn a_nul_in_a_later_windowed_file_is_the_next_fault_7966() -> Result<()> {
    let ones = MIN_BYTES / 2 + 10;
    let mut first = b"1\n".repeat(ones);
    first.extend_from_slice(b"l 1\n");
    let dir = tempfile::tempdir()?;
    let (a, b) = (dir.path().join("a.json"), dir.path().join("b.json"));
    std::fs::write(&a, &first)?;
    std::fs::write(&b, b"2\0x\n")?;
    let output = small_window::apply(Command::new(env!("CARGO_BIN_EXE_succinctly")).arg("jq"))
        .args([
            "-c",
            "-n",
            "[limit(3; repeat(try input catch .) | select(. != 1))]",
        ])
        .arg(&a)
        .arg(&b)
        .output()?;
    let line = ones + 1;
    assert_eq!(
        (
            masked(&String::from_utf8_lossy(&output.stdout)),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code(),
        ),
        (
            masked(&format!(
                "[\"Invalid numeric literal at line {line}, column 2\",\"Invalid character at line {line}, column 4\",\"break\"]\n"
            )),
            String::new(),
            Some(0)
        )
    );
    Ok(())
}

/// `--seq`: the fault is a warning that names the NUL, and the record after the next RS is read. jq
/// master: `Invalid character at line 1, column 4 (need RS to resync)`, then `2`. Both releases
/// print `[1]` (jq 1.7.1 never sees the NUL's read past it, without a newline, either).
#[test]
fn seq_warns_at_the_nul_7966() -> Result<()> {
    assert_eq!(
        run_jq_full(&["--seq", "-c", "."], Some("\u{1e}[1\0\n\u{1e}2\n"))?,
        (
            "\u{1e}2\n".to_string(),
            "jq: ignoring parse error: Invalid character at line 1, column 4 (need RS to resync)\n"
                .to_string(),
            0
        )
    );
    Ok(())
}

/// `--stream`: the events before the NUL, then the fault. jq master with `JV_PARSE_STREAMING`:
/// `[[0],1]`, then `Invalid character at line 1, column 4`.
#[test]
fn stream_reports_the_nul_after_the_events_before_it_7966() -> Result<()> {
    assert_eq!(
        run_jq_full(&["-c", "--stream", "."], Some("[1,\0]"))?,
        (
            "[[0],1]\n".to_string(),
            "jq: parse error: Invalid character at line 1, column 4\n".to_string(),
            5
        )
    );
    assert_eq!(
        run_jq_full(&["-c", "--stream-errors", "."], Some("[1,\0]"))?,
        (
            "[[0],1]\n[\"Invalid character at line 1, column 4\",[1]]\n".to_string(),
            String::new(),
            0
        )
    );
    Ok(())
}

/// `fromjson` reads its text by its length, so no newline is needed; jq quotes the text only up to
/// the NUL, as a C string. A NUL in a string is still the control-character error.
#[test]
fn fromjson_refuses_a_nul_outside_a_string_7966() -> Result<()> {
    assert_eq!(
        run_jq_full(
            &[
                "-nc",
                r#""[1\u0000x,2]", "1 2\u0000", "[\"a\u0000\"]" | try fromjson catch ."#
            ],
            None
        )?,
        (
            "\"Invalid character at line 1, column 3 (while parsing '[1')\"\n\"Invalid character at line 1, column 4 (while parsing '1 2')\"\n\"Invalid string: control characters from U+0000 through U+001F must be escaped at line 1, column 5 (while parsing '[\\\"a')\"\n"
                .to_string(),
            String::new(),
            0
        )
    );
    Ok(())
}

/// `--slurpfile` and a data import, in their own wrappers (#7769).
#[test]
fn slurpfile_and_a_data_import_refuse_the_nul_7966() -> Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("d.json"), b"1.5\0x\n")?;
    let run = |args: &[&str]| -> Result<(String, String, Option<i32>)> {
        let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .arg("jq")
            .args(args)
            .current_dir(dir.path())
            .env_remove("JQ_LIBRARY_PATH")
            .output()?;
        Ok((
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code(),
        ))
    };
    assert_eq!(
        run(&["-nc", "--slurpfile", "x", "d.json", "$x"])?,
        (
            String::new(),
            "jq: Bad JSON in --slurpfile x d.json: Invalid character at line 1, column 4\n"
                .to_string(),
            Some(2)
        )
    );
    let (out, err, code) = run(&["-nc", "-L", ".", "import \"d\" as $x; $x"])?;
    assert_eq!((out.as_str(), code), ("", Some(3)));
    assert!(
        err.starts_with("jq: error loading data file ")
            && err.ends_with(
                "/d.json: Invalid character at line 1, column 4\n\njq: 1 compile error\n"
            ),
        "{err}"
    );
    Ok(())
}

/// What #7966 leaves as it was: `-R` reads a NUL as a byte of its line, `tonumber` refuses a string
/// with one as jq 1.8.2 does (#7521), and a NUL in a region no filter reads is not reported, as for
/// any other fault there (#3035; `[1`, `}`, `]` answers the same).
#[test]
fn raw_input_tonumber_and_an_unread_region_are_unchanged_7966() -> Result<()> {
    assert_eq!(
        run_jq_full(&["-R", "-c", "."], Some("a\0b\n"))?,
        ("\"a\\u0000b\"\n".to_string(), String::new(), 0)
    );
    assert_eq!(
        run_jq_full(&["-nc", r#""1\u00002" | try tonumber catch ."#], None)?,
        (
            "\"string (\\\"1\\\\u00002\\\") cannot be parsed as a number\"\n".to_string(),
            String::new(),
            0
        )
    );
    for input in ["[1\0]", "[1}]"] {
        assert_eq!(
            run_jq_full(&["-c", "type"], Some(input))?,
            ("\"array\"\n".to_string(), String::new(), 0),
            "{input:?}"
        );
    }
    Ok(())
}
