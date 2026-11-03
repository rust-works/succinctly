//! #6006: jq's main loop reads no further input once its reader has moved past a file that did
//! not open (`while (jq_util_input_errors(...) == 0 && ...)`), after processing the input that took
//! it there. A plain `--stream` walk therefore prints the first event of the file after a missing
//! one and stops; `-n` with `inputs` and `-s` read on. `--stream -s` over only a missing file is
//! `[]` (it panicked). Every expected row was captured from jq 1.7.1 (`/usr/bin/jq`); `m` is a
//! file that does not exist.

use std::process::Command;

use anyhow::Result;

use crate::succinctly_bin;

/// `succinctly jq <flags> <files>` in a directory holding `a` (`[3]`), `b` (`{"x":[1,2]}`), `c`
/// (`1 2`) and an empty `e`, but no `m`: stdout and the exit code.
fn run_over(files: &[&str], flags: &[&str]) -> Result<(String, i32)> {
    let dir = tempfile::TempDir::new()?;
    for (name, body) in [
        ("a", "[3]\n"),
        ("b", "{\"x\":[1,2]}\n"),
        ("c", "1 2\n"),
        ("e", ""),
    ] {
        std::fs::write(dir.path().join(name), body)?;
    }
    let output = Command::new(succinctly_bin())
        .current_dir(dir.path())
        .arg("jq")
        .args(flags)
        .args(files)
        .output()?;
    Ok((
        String::from_utf8(output.stdout)?,
        output.status.code().unwrap_or(-1),
    ))
}

#[test]
fn the_main_loop_stops_one_input_past_a_file_that_did_not_open_6006() -> Result<()> {
    // `(files, flags, stdout, exit code)`
    let rows: &[(&[&str], &[&str], &str, i32)] = &[
        (&["m", "a"], &["-c", "--stream", "."], "[[0],3]\n", 2),
        (&["m", "b"], &["-c", "--stream", "."], "[[\"x\",0],1]\n", 2),
        (&["m", "c"], &["-c", "--stream", "."], "[[],1]\n", 2),
        (
            &["a", "m", "b"],
            &["-c", "--stream", "."],
            "[[0],3]\n[[0]]\n[[\"x\",0],1]\n",
            2,
        ),
        (&["m", "m", "a"], &["-c", "--stream", "."], "[[0],3]\n", 2),
        (&["m", "e", "a"], &["-c", "--stream", "."], "[[0],3]\n", 2),
        (&["a", "m"], &["-c", "--stream", "."], "[[0],3]\n[[0]]\n", 2),
        (&["m"], &["-c", "--stream", "."], "", 2),
        (
            &["m", "a"],
            &["-nc", "--stream", "[inputs]"],
            "[[[0],3],[[0]]]\n",
            2,
        ),
        (
            &["m", "a"],
            &["-cs", "--stream", "."],
            "[[[0],3],[[0]]]\n",
            2,
        ),
        (&["m"], &["-cs", "--stream", "."], "[]\n", 2),
        (&["m"], &["-cs", "."], "[]\n", 2),
    ];
    for (files, flags, want, want_code) in rows {
        let (got, code) = run_over(files, flags)?;
        assert_eq!(
            (got.as_str(), code),
            (*want, *want_code),
            "{files:?} {flags:?}"
        );
    }
    Ok(())
}
