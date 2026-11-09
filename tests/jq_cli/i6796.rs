//! #6796: under `--seq -s --stream` and `--seq -s --stream-errors` a program that reads one `input`
//! (`input`, `[., input]`, `first(inputs)`) over a file that did not open, or a directory, in the
//! middle of the list runs jq's main loop once: the slurp read every file in its first call and
//! counted the failure there, so the loop ended after that call and never popped what the stream
//! still held (the fault at the end of the text). succinctly ran the loop again and said it
//! (`Unfinished JSON term at EOF` under `--stream`, an uncaught `break` under `--stream-errors`).
//! Every row was captured from jq 1.7.1.

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
        (
            &["--seq", "-s", "--stream", "input"],
            &["trunc", "gone", "obj"],
            "\x1e[[1,\"a\"]]\n",
            "jq: error: Could not open file @D@/gone: No such file or directory\n",
            2,
        ),
        (
            &["--seq", "-s", "--stream", "first(inputs)"],
            &["rend", "dir", "nors"],
            "\x1e[[1,0]]\n",
            "jq: error: Is a directory\n",
            2,
        ),
        (
            &["--seq", "-s", "--stream", "[., input]"],
            &["trunc", "gone", "nors"],
            "\x1e[[[[0],1],[[1,0],1]],[[1,0]]]\n",
            "jq: error: Could not open file @D@/gone: No such file or directory\n",
            2,
        ),
        (
            &[
                "--seq",
                "-s",
                "--stream",
                "(try input catch .), (try input catch .)",
            ],
            &["rend", "dir", "obj"],
            "\x1e[[1,\"a\"]]\n\x1e\"Unfinished JSON term at EOF at line 2, column 11\"\n",
            "jq: error: Is a directory\n",
            2,
        ),
    ])
}

#[test]
fn a_slurp_by_the_seq_stream_errors_reader_runs_the_loop_once_past_a_failure_6796() -> Result<()> {
    check(&[
        (
            &["--seq", "-s", "--stream-errors", "input"],
            &["trunc", "gone", "obj"],
            "\x1e[[1,\"a\"]]\n",
            "jq: error: Could not open file @D@/gone: No such file or directory\n",
            2,
        ),
        (
            &["--seq", "-s", "--stream-errors", "[., input]"],
            &["rend", "dir", "nors"],
            "\x1e[[[[],12],[[0],1],[[1,0],1]],[[1,0]]]\n",
            "jq: error: Is a directory\n",
            2,
        ),
        (
            &["--seq", "-s", "--stream-errors", "first(inputs)"],
            &["rend", "gone", "obj"],
            "\x1e[[1,\"a\"]]\n",
            "jq: error: Could not open file @D@/gone: No such file or directory\n",
            2,
        ),
    ])
}

/// Rows that matched jq before: a program that reads nothing or reads to the end, a slurp with no
/// failure, `-n`, and the readers that are not `--seq`.
#[test]
fn the_slurps_other_loops_still_match_jq_6796() -> Result<()> {
    check(&[
        (&["--seq", "-s", "--stream", "."], &["trunc", "gone", "obj"], "\x1e[[[0],1],[[1,\"a\"],1]]\n", "jq: error: Could not open file @D@/gone: No such file or directory\n", 2),
        (&["--seq", "-s", "--stream", "[inputs]"], &["trunc", "gone", "obj"], "", "jq: error: Could not open file @D@/gone: No such file or directory\njq: error (at <unknown>): Unfinished JSON term at EOF at line 1, column 11\n", 2),
        (&["--seq", "-s", "--stream-errors", "[inputs]"], &["rend", "dir", "nors"], "\x1e[[[1,0]],[\"Unfinished JSON term at EOF at line 2, column 7\",[1]]]\n", "jq: error: Is a directory\n", 2),
        (&["--seq", "-s", "--stream", "input"], &["trunc", "obj"], "\x1e[[1,\"a\"]]\n", "jq: ignoring parse error: Unfinished JSON term at EOF at line 1, column 11\n", 0),
        (&["-n", "--seq", "-s", "--stream", "[inputs]"], &["trunc", "gone", "obj"], "", "jq: error: Could not open file @D@/gone: No such file or directory\njq: error (at <unknown>): Unfinished JSON term at EOF at line 1, column 11\n", 2),
        (&["-s", "--stream", "input"], &["obj", "gone", "obj"], "[[\"a\"]]\n", "jq: error: Could not open file @D@/gone: No such file or directory\n", 2),
        (&["-s", "--stream-errors", "input"], &["obj", "gone", "obj"], "[[\"a\"]]\n", "jq: error: Could not open file @D@/gone: No such file or directory\n", 2),
        (&["-s", "input"], &["obj", "gone", "obj"], "", "jq: error: Could not open file @D@/gone: No such file or directory\njq: error (at <unknown>): break\n", 2),
    ])
}
