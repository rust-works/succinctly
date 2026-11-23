//! #6796: under `--seq -s --stream` and `--seq -s --stream-errors` a program that reads one `input`
//! (`input`, `[., input]`, `first(inputs)`) over a file that did not open, or a directory, in the
//! middle of the list runs jq's main loop once: the slurp read every file in its first call and
//! counted the failure there, so the loop ended after that call and never popped what the stream
//! still held (the fault at the end of the text). succinctly ran the loop again and said it
//! (`Unfinished JSON term at EOF` under `--stream`, an uncaught `break` under `--stream-errors`).
//! Every row was captured from jq 1.7.1.
//!
//! Since #7718 `succinctly jq` reads `--seq` past the last bytes as jq 1.8.2 does (jqlang/jq#3279), so
//! the rows that read differently there are jq 1.8.2's (`/opt/homebrew/bin/jq`), with jq 1.7.1's
//! position after the last input (#7607) and its reads past a last directory.

use anyhow::Result;

use crate::run_jq_full;

/// `(flags and program, the file names in order, stdout, stderr, exit status)`; every run is
/// `jq -c FLAGS... FILES...`. `gone` is not there and `dir` is a directory.
type Row<'a> = (&'a [&'a str], &'a [&'a str], &'a str, &'a str, i32);

/// Runs every row over the fixtures and compares stdout, stderr and the exit status.
fn check(rows: &[Row]) -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::create_dir(dir.path().join("dir"))?;
    for (name, text) in [
        ("rend", "\x1e12\n\x1e[1,"),
        ("trunc", "\x1e[1,"),
        ("obj", "{\"a\":1}"),
        ("nors", "[1]"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    for (flags, order, stdout, stderr, status) in rows {
        let mut args: Vec<String> = vec!["-c".into()];
        args.extend(flags.iter().map(|flag| (*flag).to_string()));
        args.extend(order.iter().map(|name| format!("{d}/{name}")));
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let (out, err, code) = run_jq_full(&args, None)?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (
                stdout.replace("@D@", &d).as_str(),
                stderr.replace("@D@", &d).as_str(),
                *status
            ),
            "{flags:?} over {order:?}"
        );
    }
    Ok(())
}

#[test]
fn a_slurp_by_the_seq_stream_reader_runs_the_loop_once_past_a_failure_6796() -> Result<()> {
    check(&[
        // jq 1.8.2 names a file and line here; the position is jq 1.7.1's (#7607).
        (&["--seq", "-s", "--stream", "input"], &["trunc", "gone", "obj"], "", "jq: error: Could not open file @D@/gone: No such file or directory\njq: error (at <unknown>): Unfinished JSON term at EOF at line 1, column 11\n", 2),
        // jq 1.8.2 names a file and line here; the position is jq 1.7.1's (#7607).
        (&["--seq", "-s", "--stream", "first(inputs)"], &["rend", "dir", "nors"], "", "jq: error: Is a directory\njq: error (at <unknown>): Unfinished JSON term at EOF at line 2, column 7\n", 2),
        // jq 1.8.2 names a file and line here; the position is jq 1.7.1's (#7607).
        (&["--seq", "-s", "--stream", "[., input]"], &["trunc", "gone", "nors"], "", "jq: error: Could not open file @D@/gone: No such file or directory\njq: error (at <unknown>): Unfinished JSON term at EOF at line 1, column 7\n", 2),
        (&[
                "--seq",
                "-s",
                "--stream",
                "(try input catch .), (try input catch .)",
            ], &["rend", "dir", "obj"], "\u{1e}\"Unfinished JSON term at EOF at line 2, column 11\"\n\u{1e}\"break\"\n", "jq: error: Is a directory\n", 2),
    ])
}

#[test]
fn a_slurp_by_the_seq_stream_errors_reader_runs_the_loop_once_past_a_failure_6796() -> Result<()> {
    check(&[
        (&["--seq", "-s", "--stream-errors", "input"], &["trunc", "gone", "obj"], "\u{1e}[\"Unfinished JSON term at EOF at line 1, column 11\",[1]]\n", "jq: error: Could not open file @D@/gone: No such file or directory\n", 2),
        (&["--seq", "-s", "--stream-errors", "[., input]"], &["rend", "dir", "nors"], "\u{1e}[[[[],12],[[0],1],[[1,0],1],[[1,0]]],[\"Unfinished JSON term at EOF at line 2, column 7\",[1]]]\n", "jq: error: Is a directory\n", 2),
        (&["--seq", "-s", "--stream-errors", "first(inputs)"], &["rend", "gone", "obj"], "\u{1e}[\"Unfinished JSON term at EOF at line 2, column 11\",[1]]\n", "jq: error: Could not open file @D@/gone: No such file or directory\n", 2),
    ])
}

/// The failed file first or last in the list, a program that reads nothing, and two failures.
#[test]
fn the_slurps_loop_ends_after_its_first_call_wherever_the_failure_is_6796() -> Result<()> {
    check(&[
        // jq 1.8.2 names a file and line here; the position is jq 1.7.1's (#7607).
        (&["--seq", "-s", "--stream", "input"], &["gone", "trunc", "obj"], "", "jq: error: Could not open file @D@/gone: No such file or directory\njq: error (at <unknown>): Unfinished JSON term at EOF at line 1, column 11\n", 2),
        (&["--seq", "-s", "--stream", "input"], &["trunc", "obj", "gone"], "", "jq: error: Could not open file @D@/gone: No such file or directory\njq: error (at @D@/gone:0): Unfinished JSON term at EOF at line 1, column 11\n", 2),
        (&["--seq", "-s", "--stream-errors", "[., input]"], &["gone", "rend", "nors"], "\u{1e}[[[[],12],[[0],1],[[1,0],1],[[1,0]]],[\"Unfinished JSON term at EOF at line 2, column 7\",[1]]]\n", "jq: error: Could not open file @D@/gone: No such file or directory\n", 2),
        (&["--seq", "-s", "--stream-errors", "[., input]"], &["rend", "nors", "dir"], "\x1e[[[[],12],[[0],1],[[1,0],1],[[1,0]]],[\"Unfinished JSON term at EOF at line 2, column 7\",[1]]]\n", "jq: error: Is a directory\n", 2),
        (&["--seq", "-s", "--stream", "empty"], &["trunc", "gone", "obj"], "", "jq: error: Could not open file @D@/gone: No such file or directory\n", 2),
        (&["--seq", "-s", "--stream", "first(inputs)"], &["gone", "dir", "trunc"], "", "jq: error: Could not open file @D@/gone: No such file or directory\njq: error: Is a directory\njq: ignoring parse error: Unfinished JSON term at EOF at line 1, column 4\n", 2),
    ])
}

/// Rows that matched jq before: a program that reads nothing or reads to the end, a slurp with no
/// failure, `-n`, and the readers that are not `--seq`.
#[test]
fn the_slurps_other_loops_still_match_jq_6796() -> Result<()> {
    check(&[
        (&["--seq", "-s", "--stream", "."], &["trunc", "gone", "obj"], "\u{1e}[[[0],1],[[1,\"a\"],1],[[1,\"a\"]]]\n", "jq: error: Could not open file @D@/gone: No such file or directory\n", 2),
        (&["--seq", "-s", "--stream", "[inputs]"], &["trunc", "gone", "obj"], "", "jq: error: Could not open file @D@/gone: No such file or directory\njq: error (at <unknown>): Unfinished JSON term at EOF at line 1, column 11\n", 2),
        (&["--seq", "-s", "--stream-errors", "[inputs]"], &["rend", "dir", "nors"], "\u{1e}[[\"Unfinished JSON term at EOF at line 2, column 7\",[1]]]\n", "jq: error: Is a directory\n", 2),
        // jq 1.8.2 names a file and line here; the position is jq 1.7.1's (#7607).
        (&["--seq", "-s", "--stream", "input"], &["trunc", "obj"], "", "jq: error (at <unknown>): Unfinished JSON term at EOF at line 1, column 11\n", 5),
        (&["-n", "--seq", "-s", "--stream", "[inputs]"], &["trunc", "gone", "obj"], "", "jq: error: Could not open file @D@/gone: No such file or directory\njq: error (at <unknown>): Unfinished JSON term at EOF at line 1, column 11\n", 2),
        // jq 1.7.1: `[["a"]]`, the closing event its array left out; jq 1.8.2 keeps it in the array, so
        // `input` has nothing left (#7807), and names a file and line where the position is 1.7.1's (#7607).
        (&["-s", "--stream", "input"], &["obj", "gone", "obj"], "", "jq: error: Could not open file @D@/gone: No such file or directory\njq: error (at <unknown>): break\n", 2),
        // jq 1.7.1: `[["a"]]`, the closing event its array left out; jq 1.8.2 keeps it in the array, so
        // `input` has nothing left (#7807), and names a file and line where the position is 1.7.1's (#7607).
        (&["-s", "--stream-errors", "input"], &["obj", "gone", "obj"], "", "jq: error: Could not open file @D@/gone: No such file or directory\njq: error (at <unknown>): break\n", 2),
        (&["-s", "input"], &["obj", "gone", "obj"], "", "jq: error: Could not open file @D@/gone: No such file or directory\njq: error (at <unknown>): break\n", 2),
    ])
}
