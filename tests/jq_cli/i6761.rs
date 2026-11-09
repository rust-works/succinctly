//! #6761: `--seq --stream` and `--seq --stream-errors` stop at the directory given last when it is
//! the read that delivers the closing event the file before it held back (`[1]` with no record
//! separator, no newline): jq's main loop ends there, so the fault at the end of the text (an
//! unfinished record, `Unfinished JSON term at EOF`) is never reached. A directory in the middle
//! (#6646, #6725) and a missing file last already did this. Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

/// `(flags and program, the file names in order, stdout, stderr, exit status)`; every run is
/// `jq -c FLAGS... FILES...`. `gone` is not there and `dir` is a directory.
type Row<'a> = (&'a [&'a str], &'a [&'a str], &'a str, &'a str, i32);

#[test]
fn a_last_directory_stops_the_loop_at_the_closer_the_file_before_it_held_back_6761() -> Result<()> {
    let rows: &[Row] = &[
        (&["--seq", "--stream", "."], &["trunc", "nors", "dir"], "\x1e[[0],1]\n\x1e[[1,0],1]\n\x1e[[1,0]]\n", "", 2),
        (&["--seq", "--stream-errors", "."], &["trunc", "nors", "dir"], "\x1e[[0],1]\n\x1e[[1,0],1]\n\x1e[[1,0]]\n", "", 2),
        (&["--seq", "--stream", "[., input_filename]"], &["trunc", "nors", "dir"], "\x1e[[[0],1],\"@D@/trunc\"]\n\x1e[[[1,0],1],\"@D@/nors\"]\n\x1e[[[1,0]],\"@D@/dir\"]\n", "", 2),
        (&["--seq", "--stream-errors", "error(\"x\")"], &["trunc", "trunc", "nors", "dir"], "", "jq: error (at @D@/trunc:0): x\njq: error (at @D@/trunc:0): x\njq: error (at @D@/trunc:0): x\njq: error (at @D@/nors:0): x\njq: error (at @D@/dir:0): x\n", 2),
        (&["--seq", "--stream", "."], &["trunc", "nors"], "\x1e[[0],1]\n\x1e[[1,0],1]\n\x1e[[1,0]]\n", "jq: ignoring parse error: Unfinished JSON term at EOF at line 1, column 7\n", 0),
        (&["--seq", "--stream", "."], &["trunc", "dir"], "\x1e[[0],1]\n", "jq: ignoring parse error: Unfinished JSON term at EOF at line 1, column 4\n", 2),
        (&["--seq", "--stream", "."], &["trunc", "nors", "gone"], "\x1e[[0],1]\n\x1e[[1,0],1]\n\x1e[[1,0]]\n", "jq: error: Could not open file @D@/gone: No such file or directory\n", 2),
        (&["--seq", "--stream", "."], &["arr", "dir"], "\x1e[[0],1]\n\x1e[[0]]\n", "", 2),
        (&["--seq", "--stream", "."], &["arr", "nors", "dir"], "\x1e[[0],1]\n\x1e[[0]]\n\x1e[[0],1]\n\x1e[[0]]\n", "", 2),
        (&["--seq", "--stream-errors", "."], &["trunc", "dir"], "\x1e[[0],1]\n\x1e[\"Unfinished JSON term at EOF at line 1, column 4\",[1]]\n", "", 2),
        (&["--stream-errors", "."], &["trunc", "nors", "dir"], "[\"Invalid numeric literal at line 1, column 2\",[]]\n[[0],1]\n[[0]]\n", "", 2),
        (&["-n", "--seq", "--stream", "[inputs]"], &["trunc", "nors", "dir"], "", "jq: error: Is a directory\njq: error (at <unknown>): Unfinished JSON term at EOF at line 1, column 7\n", 2),
        (&["--seq", "--stream", "."], &["dir", "trunc", "nors"], "\x1e[[0],1]\n", "jq: error: Is a directory\n", 2),
        (&["--seq", "--stream", "."], &["nors", "dir"], "", "jq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 3\n", 2),
    ];
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::create_dir(dir.path().join("dir"))?;
    for (name, text) in [("trunc", "\x1e[1,"), ("nors", "[1]"), ("arr", "\x1e[1]")] {
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
