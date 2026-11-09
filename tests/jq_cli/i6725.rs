//! #6725: `--seq --stream` and `--seq --stream-errors`, without `-n` and `-s`, say a stream's
//! warnings and a file's open failure when jq's main loop reaches them, not before the program
//! runs. A program that halts or raises an error on an early value comes first, and jq may never
//! reach the warning or the file: it exits 0 or 5 where the stream's failure had made it exit 2.
//! The record reader's faults already went through the input queue (#6147); the stream reader's
//! take it too. Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

/// `(flags and program, the file names in order, stdout, stderr, exit status)`; every run is
/// `jq -c FLAGS... FILES...`. `miss` is not there and `dir` is a directory.
type Row<'a> = (&'a [&'a str], &'a [&'a str], &'a str, &'a str, i32);

#[test]
fn a_halting_program_comes_before_the_streams_warnings_and_failures_6725() -> Result<()> {
    let rows: &[Row] = &[
        (&["--seq", "--stream", "halt"], &["trunc"], "", "", 0),
        (&["--seq", "--stream", "error(\"x\")"], &["two", "mix"], "", "jq: error (at @D@/two:1): x\njq: ignoring parse error: Unmatched ']' in the middle of an object at line 2, column 3 (need RS to resync)\n", 5),
        (&["--seq", "--stream-errors", "halt"], &["arr", "miss"], "", "", 0),
        (&["--seq", "--stream", "halt"], &["multi", "miss", "bad"], "", "", 0),
        (&["--seq", "--stream", "halt_error"], &["two", "miss", "bad", "arr"], "", "[[],2]\n", 5),
        (&["--seq", "--stream-errors", "halt_error"], &["rend", "dir"], "", "[[],12]\n", 5),
        (&["--seq", "--stream", "error(\"x\")"], &["arr", "miss", "two"], "", "jq: error (at @D@/arr:0): x\njq: error: Could not open file @D@/miss: No such file or directory\njq: error (at @D@/miss:0): x\n", 2),
        (&["--seq", "--stream-errors", "error(\"x\")"], &["arr", "miss", "two"], "", "jq: error (at @D@/arr:0): x\njq: error: Could not open file @D@/miss: No such file or directory\njq: error (at @D@/miss:0): x\n", 2),
        (&["--seq", "--stream", "error(\"x\")"], &["arr", "dir", "two"], "", "jq: error (at @D@/arr:0): x\njq: error (at @D@/dir:0): x\n", 2),
        (&["--seq", "--stream", "."], &["mix", "two"], "\x1e[[],2]\n\x1e[[],2]\n", "jq: ignoring parse error: Unmatched ']' in the middle of an object at line 1, column 3 (need RS to resync)\n", 0),
        (&["--seq", "--stream", "halt"], &["two", "trunc", "mix"], "", "", 0),
        (&["--seq", "--stream-errors", "."], &["mix", "two"], "\x1e[\"Unmatched ']' in the middle of an object at line 1, column 3 (need RS to resync)\",[null]]\n\x1e[[],2]\n\x1e[[],2]\n", "", 0),
        (&["--seq", "--stream-errors", "."], &["two", "dir"], "\x1e[[],2]\n", "", 2),
        (&["--seq", "--stream", "."], &["two", "dir"], "\x1e[[],2]\n", "", 2),
        (&["--seq", "--stream-errors", "error(\"x\")"], &["arr", "dir", "two"], "", "jq: error (at @D@/arr:0): x\njq: error (at @D@/dir:0): x\n", 2),
        (&["--seq", "--stream-errors", "error(\"x\")"], &["arr", "dir"], "", "jq: error (at @D@/arr:0): x\njq: error (at @D@/dir:0): x\n", 2),
        (&["-n", "--seq", "--stream", "[inputs]"], &["two", "miss", "arr"], "\x1e[[[],2]]\n", "jq: error: Could not open file @D@/miss: No such file or directory\n", 2),
        (&["-n", "--seq", "--stream-errors", "[inputs]"], &["arr", "miss", "two"], "\x1e[[[0],1],[[0]],[[],2]]\n", "jq: error: Could not open file @D@/miss: No such file or directory\n", 2),
        (&["--seq", "--stream", "[., input]"], &["multi", "two", "miss"], "\x1e[[[],1],[[],2]]\n", "jq: error: Could not open file @D@/miss: No such file or directory\njq: error (at @D@/miss:0): break\n", 2),
        (&["--seq", "--stream-errors", "[., input]"], &["multi", "dir", "arr"], "\x1e[[[],1],[[],2]]\n", "jq: error: Is a directory\n", 2),
    ];
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::create_dir(dir.path().join("dir"))?;
    for (name, text) in [
        ("trunc", "\x1e[1,"),
        ("two", "\x1e2\n"),
        ("mix", "\x1e{]\n\x1e2"),
        ("arr", "\x1e[1]"),
        ("multi", "\x1e1\n\x1e2\n"),
        ("bad", "\x1e[1}"),
        ("rend", "\x1e12\n\x1e[1,"),
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

/// A `--seq --stream -s` run reads the whole stream in its first call, warning as it goes: once a
/// file that did not open has been said, jq's main loop reads no further, so the warnings the file
/// after it holds past its first are never said. Every row was captured from jq 1.7.1.
#[test]
fn a_slurp_says_no_warning_past_the_one_that_reaches_a_failure_6725() -> Result<()> {
    let rows: &[Row] = &[
        (&["--seq", "--stream", "-s", "."], &["miss", "warn2"], "", "jq: error: Could not open file @D@/miss: No such file or directory\njq: ignoring parse error: Truncated value at line 1, column 4\n", 2),
        (&["--seq", "--stream", "-s", "."], &["warn2", "miss", "warn2"], "", "jq: ignoring parse error: Truncated value at line 1, column 4\njq: ignoring parse error: Truncated value at line 1, column 7\njq: error: Could not open file @D@/miss: No such file or directory\njq: ignoring parse error: Truncated value at line 2, column 4\n", 2),
        (&["--seq", "--stream", "-s", "."], &["one", "dir", "warn2"], "", "jq: error: Is a directory\njq: ignoring parse error: Truncated value at line 2, column 4\n", 2),
    ];
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::create_dir(dir.path().join("dir"))?;
    for (name, text) in [("warn2", "\x1e[1\x1e[2\x1e[3]\n"), ("one", "\x1e1\n")] {
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
