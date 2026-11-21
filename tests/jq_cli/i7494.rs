//! #7494: a word that is `-` and then neither a letter nor a `-` (`-1`, `-.a`, `-(1)`, `-[1]`) is
//! positional in jq, and jq keeps reading options after it. `succinctly jq` spliced a `--` in front
//! of such a word to get it past clap (#3389), so every later word (`-n`, `-c`, `--arg`, even `--`)
//! became a file. And after `--args --`, an option-shaped word was never taken as the program,
//! where jq takes the first positional word wherever it is. Both came from parsing jq's command line
//! with clap's grammar; `walk_jq_argv` now reads argv as jq does (#7438). Every row was captured
//! from `/usr/bin/jq` (1.7.1), and jq 1.8.2 (`/opt/homebrew/bin/jq`) agrees on stdout and exit
//! status on all of them (and on stderr but for the compile error's 1.8 column and caret).

use anyhow::Result;

use crate::run_jq_in_dir;

const NO_FILE_1: &str = "jq: error: Could not open file -1: No such file or directory\n";

/// (argv, stdout, stderr, exit status), each with standard input empty and `a.json` holding
/// `{"a":1}`.
const ROWS: &[(&[&str], &str, &str, i32)] = &[
    (&["-1", "-n"], "-1\n", "", 0),
    (&["-1+1", "-n"], "0\n", "", 0),
    (
        &["-.a", "-n"],
        "",
        "jq: error (at <unknown>): null (null) cannot be negated\n",
        5,
    ),
    (&["-n", "-(1)", "-c"], "-1\n", "", 0),
    (
        &["-n", "-[1]", "-c"],
        "",
        "jq: error (at <unknown>): array ([1]) cannot be negated\n",
        5,
    ),
    (&["$x", "-1", "--arg", "x", "2", "-n"], "\"2\"\n", "", 0),
    // `-1` is an input file and `-c` still applies...
    (&[".", "a.json", "-1", "-c"], "{\"a\":1}\n", NO_FILE_1, 2),
    // ...until a `--`, which is consumed, not read as a file.
    (
        &[".", "a.json", "-1", "--", "-c"],
        "{\n  \"a\": 1\n}\n",
        concat!(
            "jq: error: Could not open file -1: No such file or directory\n",
            "jq: error: Could not open file -c: No such file or directory\n"
        ),
        2,
    ),
    (
        &["-n", "--jsonargs", "$ARGS.positional", "-1", "-c", "-2"],
        "[-1,-2]\n",
        "",
        0,
    ),
    (&["-n", "--arg", "x", "-1", "$x", "-c"], "\"-1\"\n", "", 0),
    // After `--args --` the first positional word is the program, option-shaped or not.
    (
        &["-n", "--args", "--", "-c", "."],
        "",
        "jq: error: c/0 is not defined at <top-level>, line 1:\n-c \njq: 1 compile error\n",
        3,
    ),
    (
        &[
            "-n",
            "--args",
            "$ARGS.positional",
            "--",
            "-c",
            "--args",
            "x",
        ],
        "[\n  \"-c\",\n  \"--args\",\n  \"x\"\n]\n",
        "",
        0,
    ),
];

#[test]
fn test_7494_options_are_read_after_a_hyphen_led_positional_word() -> Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("a.json"), "{\"a\":1}\n")?;
    for (argv, stdout, stderr, code) in ROWS {
        let (out, err, status) = run_jq_in_dir(dir.path(), argv, Some(""))?;
        assert_eq!(
            (out.as_str(), err.as_str(), status),
            (*stdout, *stderr, *code),
            "jq {argv:?}"
        );
    }
    Ok(())
}
