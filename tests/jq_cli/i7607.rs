//! #7607: once the read that reached the end of the last input has found it already at end of file
//! (a last line with no newline), jq 1.7.1 has forgotten the file: an error raised then is `(at
//! <unknown>)`, `input_filename` is `null` and `input_line_number` is 0. A directory given last is
//! not reported (exit 2, no message), where one given earlier prints `jq: error: Is a directory`.
//!
//! jq 1.8.2 keeps the name and the line count (`(at nonl.json:1)`, `1`, `"nonl.json"`) and prints
//! `Is a directory` for a last directory too. Both are by-products of jqlang/jq#3279 (commit 4e3088f,
//! "Fix behavior of `--slurp --stream` when input has no trailing newline character"), whose text
//! names only the `--slurp --stream` fix #7523 adopted; upstream never calls either effect a bug, so
//! `succinctly jq` keeps jq 1.7.1's answers, recorded in `docs/compliance/jq/limitations.md`.
//!
//! Every expected value was captured from jq 1.7.1 (`/usr/bin/jq`); jq 1.8.2's
//! (`/opt/homebrew/bin/jq`) is in the comment where it differs.

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::Result;

use crate::succinctly_bin;

fn files(dir: &Path) -> Result<()> {
    std::fs::write(dir.join("nonl.json"), "[1,2,3]\n[4]")?;
    std::fs::write(dir.join("nl.json"), "[1,2,3]\n[4]\n")?;
    std::fs::write(dir.join("nonl2.json"), "[5]")?;
    std::fs::write(dir.join("raw-nonl.txt"), "a\nb")?;
    std::fs::write(dir.join("obj-nonl.json"), "{\"a\":1}")?;
    std::fs::write(dir.join("a.json"), "1\n")?;
    std::fs::create_dir(dir.join("dir"))?;
    Ok(())
}

/// Runs `succinctly jq <args>` in `dir`, with `stdin` (a file in `dir`) or an empty standard input.
fn run(dir: &Path, args: &[&str], stdin: Option<&str>) -> Result<(String, String, Option<i32>)> {
    let input = match stdin {
        Some(name) => Stdio::from(std::fs::File::open(dir.join(name))?),
        None => Stdio::null(),
    };
    let output = Command::new(succinctly_bin())
        .arg("jq")
        .args(args)
        .current_dir(dir)
        .env("LC_ALL", "C")
        .stdin(input)
        .output()?;
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?,
        output.status.code(),
    ))
}

/// (args, stdin file, stdout, stderr, exit status).
type Row = (
    &'static [&'static str],
    Option<&'static str>,
    &'static str,
    &'static str,
    i32,
);

const ROWS: &[Row] = &[
    // 1.8.2: `jq: error (at nonl.json:1): x`
    (
        &["-n", "-c", "[inputs], error(\"x\")", "nonl.json"],
        None,
        "[[1,2,3],[4]]\n",
        "jq: error (at <unknown>): x\n",
        5,
    ),
    // 1.8.2: `1` and `"nonl.json"`
    (
        &[
            "-n",
            "-c",
            "[inputs] | input_line_number, input_filename",
            "nonl.json",
        ],
        None,
        "0\nnull\n",
        "",
        0,
    ),
    // A last line with its newline keeps the position in both releases.
    (
        &["-n", "-c", "[inputs], error(\"x\")", "nl.json"],
        None,
        "[[1,2,3],[4]]\n",
        "jq: error (at nl.json:2): x\n",
        5,
    ),
    // 1.8.2: `jq: error (at <stdin>:1): x`
    (
        &["-n", "-c", "[inputs], error(\"x\")"],
        Some("nonl.json"),
        "[[1,2,3],[4]]\n",
        "jq: error (at <unknown>): x\n",
        5,
    ),
    // 1.8.2: `1` and `"<stdin>"`
    (
        &["-n", "-c", "[inputs] | input_line_number, input_filename"],
        Some("nonl.json"),
        "0\nnull\n",
        "",
        0,
    ),
    // 1.8.2: `jq: error (at raw-nonl.txt:1): x`
    (
        &["-R", "-n", "-c", "[inputs], error(\"x\")", "raw-nonl.txt"],
        None,
        "[\"a\",\"b\"]\n",
        "jq: error (at <unknown>): x\n",
        5,
    ),
    // 1.8.2: the second row's name is `"obj-nonl.json"`
    (
        &[
            "--stream",
            "-c",
            "[., input_line_number, input_filename]",
            "obj-nonl.json",
        ],
        None,
        "[[[\"a\"],1],0,\"obj-nonl.json\"]\n[[[\"a\"]],0,null]\n",
        "",
        0,
    ),
    // 1.8.2: `jq: error (at nonl2.json:0): x`
    (
        &[
            "-n",
            "-c",
            "[inputs], error(\"x\")",
            "nl.json",
            "nonl2.json",
        ],
        None,
        "[[1,2,3],[4],[5]]\n",
        "jq: error (at <unknown>): x\n",
        5,
    ),
    // A program that runs per input names the file while it is still being read, in both.
    (
        &["-c", "[., input_line_number, input_filename]", "nonl.json"],
        None,
        "[[1,2,3],1,\"nonl.json\"]\n[[4],1,\"nonl.json\"]\n",
        "",
        0,
    ),
    // 1.8.2: `jq: error: Is a directory` on stderr
    (&["-c", ".", "a.json", "dir"], None, "1\n", "", 2),
    // 1.8.2: `jq: error: Is a directory` on stderr
    (&["-s", "-c", ".", "a.json", "dir"], None, "[1]\n", "", 2),
    // A directory before the last file is reported in both releases.
    (
        &["-c", ".", "dir", "a.json"],
        None,
        "1\n",
        "jq: error: Is a directory\n",
        2,
    ),
];

#[test]
fn the_last_input_is_forgotten_at_its_end_as_in_jq_1_7_1_7607() -> Result<()> {
    let dir = tempfile::tempdir()?;
    files(dir.path())?;
    for &(args, stdin, stdout, stderr, code) in ROWS {
        let got = run(dir.path(), args, stdin)?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (stdout, stderr, Some(code)),
            "{args:?} < {stdin:?}"
        );
    }
    Ok(())
}
