//! #7754: with `-s`, a fault the program never reads followed by a file that does not open printed
//! the program's answer with nothing on stderr and exited 0. jq's slurp ends at the fault, which its
//! main loop reports as a parse error after the files its reader met before it, and the program never
//! runs: exit 5, or 2 when a failure came first. The open failures after the fault were rightly not
//! reported (#6485), but the lazy slurp of #2847 never raised the fault either. Every expected string
//! was captured from `/usr/bin/jq` 1.7.1, and jq 1.8.2 gives the same.

use anyhow::Result;

use crate::run_jq_in_dir;

fn files() -> Result<tempfile::TempDir> {
    let dir = tempfile::tempdir()?;
    for (name, text) in [
        ("c.json", "[1,x]\n"),
        ("one.json", "1\n"),
        ("two.json", "2\n"),
        ("late.json", "3\n{\"a\":}\n"),
        ("eof.json", "[1,"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    std::fs::create_dir(dir.path().join("dir"))?;
    Ok(dir)
}

/// (arguments, standard input, stdout, stderr, exit status).
type Row = (
    &'static [&'static str],
    Option<&'static str>,
    &'static str,
    &'static str,
    i32,
);

fn check(rows: &[Row]) -> Result<()> {
    let dir = files()?;
    for &(args, input, stdout, stderr, code) in rows {
        let (out, err, status) = run_jq_in_dir(dir.path(), args, input)?;
        assert_eq!(out, stdout, "{args:?} stdout");
        assert_eq!(err, stderr, "{args:?} stderr");
        assert_eq!(status, code, "{args:?} status");
    }
    Ok(())
}

#[test]
fn a_slurp_faults_before_a_file_that_does_not_open_7754() -> Result<()> {
    check(&[
        (&["-c", "-s", "length", "c.json", "missing.json", "one.json"], None, "", "jq: parse error: Invalid numeric literal at line 1, column 5\n", 5),
        (&["-c", "-s", ".", "c.json", "missing.json", "one.json"], None, "", "jq: parse error: Invalid numeric literal at line 1, column 5\n", 5),
        (&["-c", "-s", "type", "c.json", "missing.json", "one.json"], None, "", "jq: parse error: Invalid numeric literal at line 1, column 5\n", 5),
        (&["-c", "-s", "input_filename", "c.json", "missing.json", "one.json"], None, "", "jq: parse error: Invalid numeric literal at line 1, column 5\n", 5),
        (&["-c", "-s", "1+1", "c.json", "missing.json", "one.json"], None, "", "jq: parse error: Invalid numeric literal at line 1, column 5\n", 5),
        (&["-c", "-s", "length", "one.json", "missing.json", "late.json"], None, "", "jq: error: Could not open file missing.json: No such file or directory\njq: parse error: Unmatched '}' at line 3, column 6\n", 2),
        (&["-c", "-s", "length", "two.json", "missing.json", "c.json", "missing2.json"], None, "", "jq: error: Could not open file missing.json: No such file or directory\njq: parse error: Invalid numeric literal at line 2, column 5\n", 2),
        (&["-c", "-s", "length", "one.json", "eof.json", "missing.json"], None, "", "jq: error: Could not open file missing.json: No such file or directory\njq: parse error: Unfinished JSON term at EOF at line 2, column 3\n", 2),
        (&["-c", "-s", "length", "-", "missing.json", "one.json"], Some("[1,x]\n"), "", "jq: parse error: Invalid numeric literal at line 1, column 5\n", 5),
        (&["-c", "-s", "length", "c.json", "dir", "one.json"], None, "", "jq: parse error: Invalid numeric literal at line 1, column 5\n", 5),
        (&["-c", "-s", ".", "c.json", "dir", "one.json"], None, "", "jq: parse error: Invalid numeric literal at line 1, column 5\n", 5),
        (&["-c", "-s", "type", "c.json", "dir", "one.json"], None, "", "jq: parse error: Invalid numeric literal at line 1, column 5\n", 5),
        (&["-c", "-s", "input_filename", "c.json", "dir", "one.json"], None, "", "jq: parse error: Invalid numeric literal at line 1, column 5\n", 5),
        (&["-c", "-s", "1+1", "c.json", "dir", "one.json"], None, "", "jq: parse error: Invalid numeric literal at line 1, column 5\n", 5),
        (&["-c", "-s", "length", "one.json", "dir", "late.json"], None, "", "jq: error: Is a directory\njq: parse error: Unmatched '}' at line 3, column 6\n", 2),
        (&["-c", "-s", "length", "two.json", "dir", "c.json", "missing2.json"], None, "", "jq: error: Is a directory\njq: parse error: Invalid numeric literal at line 2, column 5\n", 2),
        (&["-c", "-s", "length", "-", "dir", "one.json"], Some("[1,x]\n"), "", "jq: parse error: Invalid numeric literal at line 1, column 5\n", 5),
    ])
}

/// With every file open nothing is scanned for a fault (#7176), and the lazy slurp of #2847 answers
/// as before: `length` reads no member, so it prints `2` where jq 1.7.1 and 1.8.2 report the parse
/// error and exit 5 (recorded under "Every M2 filter streams" in
/// `docs/compliance/jq/limitations.md`).
#[test]
fn a_slurp_with_every_file_open_reads_only_what_it_returns_7754() -> Result<()> {
    check(&[
        (
            &["-c", "-s", "length", "c.json", "one.json"],
            None,
            "2\n",
            "",
            0,
        ),
        (
            &["-c", "-s", "1+1", "c.json", "one.json"],
            None,
            "2\n",
            "",
            0,
        ),
    ])
}
