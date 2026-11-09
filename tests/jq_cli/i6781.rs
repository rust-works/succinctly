//! #6781: a `--seq --stream` or `--seq --stream-errors` program that reads `input` without `-n` and
//! stops before the end of the stream (`first(inputs)`, `[limit(2;inputs)]`, `try input` twice)
//! leaves jq's main loop stopped once a file that did not open (or a directory) has been reached,
//! so the fault at the end of the text is never popped: no `Unfinished JSON term at EOF` warning
//! under `--stream`, no further iteration for the fault event under `--stream-errors`. The loop's
//! stop followed the location of the last value read, which the held-back closing event the
//! program's `input` read does not move past the failure. Every row was captured from jq 1.7.1.

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
        ("arrj", "[1]"),
        ("lines", "a\nb\n"),
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
fn seq_stream_says_no_warning_after_a_program_stopped_at_a_failure_6781() -> Result<()> {
    check(&[
        (
            &["--seq", "--stream", "first(inputs)"],
            &["rend", "gone", "obj"],
            "\x1e[[0],1]\n\x1e[[1,\"a\"]]\n",
            "jq: error: Could not open file @D@/gone: No such file or directory\n",
            2,
        ),
        (
            &["--seq", "--stream", "[limit(2;inputs)]"],
            &["trunc", "gone", "obj"],
            "\x1e[[[1,\"a\"],1],[[1,\"a\"]]]\n",
            "jq: error: Could not open file @D@/gone: No such file or directory\n",
            2,
        ),
        (
            &[
                "--seq",
                "--stream",
                "(try input catch .), (try input catch .)",
            ],
            &["trunc", "dir", "nors"],
            "\x1e[[1,0],1]\n\x1e[[1,0]]\n",
            "jq: error: Is a directory\n",
            2,
        ),
        (
            &["--seq", "--stream", "first(inputs)"],
            &["rend", "dir", "nors"],
            "\x1e[[0],1]\n\x1e[[1,0]]\n",
            "jq: error: Is a directory\n",
            2,
        ),
    ])
}

#[test]
fn seq_stream_errors_runs_no_more_iterations_after_a_program_stopped_at_a_failure_6781(
) -> Result<()> {
    check(&[
        (
            &["--seq", "--stream-errors", "[limit(2;inputs)]"],
            &["trunc", "gone", "obj"],
            "\x1e[[[1,\"a\"],1],[[1,\"a\"]]]\n",
            "jq: error: Could not open file @D@/gone: No such file or directory\n",
            2,
        ),
        (
            &[
                "--seq",
                "--stream-errors",
                "(try input catch .), (try input catch .)",
            ],
            &["trunc", "dir", "nors"],
            "\x1e[[1,0],1]\n\x1e[[1,0]]\n",
            "jq: error: Is a directory\n",
            2,
        ),
        (
            &["--seq", "--stream-errors", "first(inputs)"],
            &["rend", "gone", "obj"],
            "\x1e[[0],1]\n\x1e[[1,\"a\"]]\n",
            "jq: error: Could not open file @D@/gone: No such file or directory\n",
            2,
        ),
    ])
}

/// Rows that matched jq before: a program that reads to the end, one that reads no `input`, `-n`,
/// and the readers that are not `--seq --stream`.
#[test]
fn the_loop_still_reads_on_where_jq_does_6781() -> Result<()> {
    check(&[
        (&["--seq", "--stream", "[inputs]"], &["rend", "gone", "obj"], "", "jq: error: Could not open file @D@/gone: No such file or directory\njq: error (at <unknown>): Unfinished JSON term at EOF at line 2, column 11\n", 2),
        (&["--seq", "--stream", "first(inputs)"], &["trunc", "gone", "nors"], "\x1e[[1,0],1]\n", "jq: error: Could not open file @D@/gone: No such file or directory\n", 2),
        (&["--seq", "--stream", "."], &["rend", "gone", "obj"], "\x1e[[],12]\n\x1e[[0],1]\n\x1e[[1,\"a\"],1]\n", "jq: error: Could not open file @D@/gone: No such file or directory\n", 2),
        (&["--seq", "--stream", "input"], &["trunc", "gone", "nors"], "\x1e[[1,0],1]\n", "jq: error: Could not open file @D@/gone: No such file or directory\n", 2),
        (&["-n", "--seq", "--stream", "first(inputs)"], &["rend", "gone", "obj"], "\x1e[[],12]\n", "", 0),
        (&["--seq", "first(inputs)"], &["rend", "gone", "obj"], "", "jq: error: Could not open file @D@/gone: No such file or directory\njq: error (at @D@/obj:0): Unfinished JSON term at EOF at line 2, column 11\n", 2),
        (&["--stream", "[limit(2;inputs)]"], &["arrj", "gone", "arrj"], "[[[0]],[[0],1]]\n", "jq: error: Could not open file @D@/gone: No such file or directory\n", 2),
        (&["--stream-errors", "first(inputs)"], &["arrj", "gone", "arrj"], "[[0]]\n", "jq: error: Could not open file @D@/gone: No such file or directory\n", 2),
        (&["[limit(2;inputs)]"], &["arrj", "gone", "arrj"], "[[1]]\n", "jq: error: Could not open file @D@/gone: No such file or directory\n", 2),
        (&["-R", "first(inputs)"], &["lines", "gone", "lines"], "\"b\"\n\"b\"\n", "jq: error: Could not open file @D@/gone: No such file or directory\n", 2),
    ])
}
