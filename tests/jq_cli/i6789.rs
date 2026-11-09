//! #6789: under `--seq -s --stream-errors` a program that reads one `input` over an unfinished
//! record and a last directory prints the fault as an event, and jq says `Is a directory` on that
//! read: the slurp's own read went through the directory, and the program's read after it is the
//! next one the reader is asked for. The directory was left to a read that reached it by location
//! (or found the stream at its end), which the fault event the program read does not. Every row was
//! captured from jq 1.7.1.

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
    for (name, text) in [("trunc", "\x1e[1,"), ("pa", "[1]")] {
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
fn a_program_that_reads_one_input_says_the_slurps_last_directory_6789() -> Result<()> {
    check(&[
        (&["--seq", "-s", "--stream-errors", "input"], &["trunc", "pa", "dir"], "\x1e[\"Unfinished JSON term at EOF at line 1, column 7\",[1]]\n", "jq: error: Is a directory\n", 2),
        (&["--seq", "-s", "--stream-errors", "[., input]"], &["trunc", "pa", "dir"], "\x1e[[[[0],1],[[1,0],1],[[1,0]]],[\"Unfinished JSON term at EOF at line 1, column 7\",[1]]]\n", "jq: error: Is a directory\n", 2),
        (&["--seq", "-s", "--stream-errors", "[., input]"], &["pa", "trunc", "pa", "dir"], "\x1e[[[[0],1],[[1,0],1],[[1,0]]],[\"Unfinished JSON term at EOF at line 1, column 10\",[1]]]\n", "jq: error: Is a directory\n", 2),
    ])
}

/// Rows that matched jq before: a program that reads to the end, a last file that is missing, the
/// other readers, and `-n`.
#[test]
fn the_slurps_other_last_directory_reads_still_match_jq_6789() -> Result<()> {
    check(&[
        (&["--seq", "-s", "--stream-errors", "[inputs]"], &["trunc", "pa", "dir"], "\x1e[[\"Unfinished JSON term at EOF at line 1, column 7\",[1]]]\n", "jq: error: Is a directory\n", 2),
        (&["--seq", "-s", "--stream-errors", "input"], &["trunc", "pa", "gone"], "\x1e[\"Unfinished JSON term at EOF at line 1, column 7\",[1]]\n", "jq: error: Could not open file @D@/gone: No such file or directory\n", 2),
        (&["--seq", "-s", "--stream", "input"], &["trunc", "pa", "dir"], "", "jq: error: Is a directory\njq: error (at <unknown>): Unfinished JSON term at EOF at line 1, column 7\n", 2),
        (&["-n", "-s", "--stream-errors", "[., (try input catch .), (try input catch .)]"], &["trunc", "pa", "dir"], "[null,[[\"Invalid numeric literal at line 1, column 2\",[]],[[0],1],[[0]]],\"break\"]\n", "jq: error: Is a directory\n", 2),
        (&["--seq", "-s", "--stream-errors", "."], &["trunc", "pa", "dir"], "\x1e[[[0],1],[[1,0],1],[[1,0]]]\n", "", 2),
        (&["-s", "input"], &["pa", "dir"], "", "jq: error: Is a directory\njq: error (at <unknown>): break\n", 2),
        (&["-s", "--stream-errors", "input"], &["trunc", "pa", "dir"], "", "jq: error: Is a directory\njq: error (at <unknown>): break\n", 2),
    ])
}
