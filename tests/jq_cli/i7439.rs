//! #7439: jq 1.7.1's `-f`/`--from-file` is a flag, not an option with a value. It only marks the
//! program as a file: the first non-option word, wherever it sits in argv (after other flags, after
//! `--args`, before `-f` itself), is that file and every later word is an input. `succinctly jq`
//! took the word right after `-f` as the file, so `-f -r prog.jq`, `-fr prog.jq` and
//! `prog.jq in.json -f` failed and `in.json -f prog.jq` ran the wrong program. Every row captured
//! from `/usr/bin/jq` 1.7.1 with standard input at `/dev/null` (jq names itself by its argv[0],
//! `/usr/bin/jq:`, where `succinctly jq` says `jq:`); jq 1.8.2 (`/opt/homebrew/bin/jq`) agrees on
//! every one of them but `-f` with no program file, where it follows 1.8.2's fix (see
//! `test_7439_from_file_with_no_program_file_is_jq_1_8_2s_usage_error`).

use anyhow::Result;

use crate::run_jq_in_dir;

/// `prog.jq` (`.a`), `b.jq` (`.b`, not JSON), `args.jq` (`[$x, $ARGS.positional]`), `a.json`
/// (`{"a":1}`) and `b.json` (`{"a":2}`).
fn fixture() -> Result<tempfile::TempDir> {
    let dir = tempfile::tempdir()?;
    for (file, text) in [
        ("prog.jq", ".a\n"),
        ("b.jq", ".b\n"),
        ("args.jq", "[$x, $ARGS.positional]\n"),
        ("a.json", "{\"a\":1}\n"),
        ("b.json", "{\"a\":2}\n"),
    ] {
        std::fs::write(dir.path().join(file), text)?;
    }
    Ok(dir)
}

const PARSE_ERROR: &str = "jq: parse error: Invalid numeric literal at line 2, column 0\n";
const ARG_NO_WORDS: &str = "[\n  \"1\",\n  []\n]\n";
const ARG_P_Q: &str = "[\n  \"1\",\n  [\n    \"p\",\n    \"q\"\n  ]\n]\n";

/// (argv, stdout, stderr, exit status)
const ROWS: &[(&str, &str, &str, i32)] = &[
    ("-f prog.jq a.json", "1\n", "", 0),
    ("-f -c prog.jq a.json", "1\n", "", 0),
    ("-fr prog.jq a.json", "1\n", "", 0),
    ("-rf prog.jq a.json", "1\n", "", 0),
    ("-nf prog.jq a.json", "null\n", "", 0),
    ("-fn prog.jq a.json", "null\n", "", 0),
    ("-fe prog.jq a.json", "1\n", "", 0),
    ("--from-file prog.jq a.json", "1\n", "", 0),
    ("--from-file -c prog.jq a.json", "1\n", "", 0),
    ("prog.jq a.json -f", "1\n", "", 0),
    ("prog.jq a.json --from-file", "1\n", "", 0),
    ("prog.jq -f a.json", "1\n", "", 0),
    // The first positional is the program file: `a.json` here, read as a program file, and
    // `prog.jq` is the input jq fails to parse.
    ("a.json -f prog.jq", "", PARSE_ERROR, 5),
    ("-f a.json prog.jq", "", PARSE_ERROR, 5),
    ("-f prog.jq a.json b.json", "1\n2\n", "", 0),
    (
        "-sf prog.jq a.json b.json",
        "",
        "jq: error (at b.json:1): Cannot index array with string \"a\"\n",
        5,
    ),
    ("-f -L . prog.jq a.json", "1\n", "", 0),
    ("-f --indent 3 prog.jq a.json", "1\n", "", 0),
    ("-f -nr prog.jq", "null\n", "", 0),
    ("-f prog.jq", "", "", 0),
    // Named and positional arguments, with `-f` anywhere, `--args` and `--jsonargs` included.
    ("-f --arg x 1 args.jq a.json", ARG_NO_WORDS, "", 0),
    ("-f args.jq --arg x 1 a.json", ARG_NO_WORDS, "", 0),
    ("-n -f args.jq --arg x 1 --args p q", ARG_P_Q, "", 0),
    ("-n --arg x 1 -f --args args.jq p q", ARG_P_Q, "", 0),
    ("-n --arg x 1 --args -f args.jq p q", ARG_P_Q, "", 0),
    ("-n --arg x 1 --args args.jq -f p q", ARG_P_Q, "", 0),
    (
        "-n --arg x 1 --jsonargs -f args.jq 1 2",
        "[\n  \"1\",\n  [\n    1,\n    2\n  ]\n]\n",
        "",
        0,
    ),
    (
        "-n --arg x 1 --args p -f",
        "",
        "jq: Could not open p: No such file or directory\n",
        2,
    ),
    // A repeated `-f` is the same flag: the second program-looking word is an input.
    ("-f -f prog.jq a.json", "1\n", "", 0),
    ("-f prog.jq -f b.jq a.json", "", PARSE_ERROR, 5),
    // `--` ends the options: before it `-f` is a flag, after it a word of the program.
    ("-f prog.jq -- a.json", "1\n", "", 0),
    ("-f -- prog.jq a.json", "1\n", "", 0),
    (
        "-- -f prog.jq a.json",
        "",
        "jq: error: f/0 is not defined at <top-level>, line 1:\n-f \njq: 1 compile error\n",
        3,
    ),
    // `.` named as the program file is a directory, in jq's words (1.7.1 and 1.8.2 alike).
    (
        "-f . a.json",
        "",
        "jq: Could not open .: It's a directory\n",
        2,
    ),
    (
        "-f missing.jq a.json",
        "",
        "jq: Could not open missing.jq: No such file or directory\n",
        2,
    ),
];

#[test]
fn test_7439_from_file_is_a_flag_and_the_first_positional_is_the_program_file() -> Result<()> {
    let dir = fixture()?;
    for &(argv, stdout, stderr, code) in ROWS {
        let args: Vec<&str> = argv.split(' ').collect();
        let got = run_jq_in_dir(dir.path(), &args, None)?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (stdout, stderr, code),
            "jq {argv}"
        );
    }
    Ok(())
}

/// `-f` with no positional word at all. jq 1.7.1 fell back to the program `.` (standard input or
/// output not a terminal) and then failed to open `.` as the file: `/usr/bin/jq: Could not open .:
/// It's a directory`, exit 2. jq 1.8 fixed that (jqlang/jq commit 7a1fd72d, "Fix missing program
/// from file"): the `.` default no longer applies under `-f`, and `/opt/homebrew/bin/jq` 1.8.2
/// prints its short usage text, exit 2. This follows 1.8.2, with this build's version in the
/// first line. A named argument is still read first (`--rawfile`'s error wins), as in both.
#[test]
fn test_7439_from_file_with_no_program_file_is_jq_1_8_2s_usage_error() -> Result<()> {
    let dir = fixture()?;
    let usage_tail = "\nUsage:\tjq [options] <jq filter> [file...]\n\
                      \tjq [options] --args <jq filter> [strings...]\n\
                      \tjq [options] --jsonargs <jq filter> [JSON_TEXTS...]\n\n\
                      jq is a tool for processing JSON inputs, applying the given filter to\n\
                      its JSON text inputs and producing the filter's results as JSON on\n\
                      standard output.\n\n\
                      The simplest filter is ., which copies jq's input to its output\n\
                      unmodified except for formatting. For more advanced filters see\n\
                      the jq(1) manpage (\"man jq\") and/or https://jqlang.org/.\n\n\
                      Example:\n\n\t$ echo '{\"foo\": 0}' | jq .\n\
                      \t{\n\t  \"foo\": 0\n\t}\n\n\
                      For listing the command options, use jq --help.\n";
    let usage = format!(
        "jq - commandline JSON processor [version {}]\n{usage_tail}",
        env!("CARGO_PKG_VERSION")
    );
    for argv in [
        "-f",
        "-n -f",
        "-f -n",
        "-n -f --args",
        "-f --",
        "-f --arg x 1",
        "-f --jsonargs",
    ] {
        let args: Vec<&str> = argv.split(' ').collect();
        let (stdout, stderr, code) = run_jq_in_dir(dir.path(), &args, None)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            ("", usage.as_str(), 2),
            "jq {argv}"
        );
    }
    let (stdout, stderr, code) =
        run_jq_in_dir(dir.path(), &["-f", "--rawfile", "x", "nope"], None)?;
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), code),
        (
            "",
            "jq: Bad JSON in --rawfile x nope: Could not open nope: No such file or directory\n",
            2
        )
    );
    Ok(())
}

/// Two shapes of #7438 that this flag also settles: jq has no `-fFILE` and no `--from-file=FILE`,
/// and refuses both with `Unknown option` and exit 2. clap's own wording stays (#6017); the run
/// must not start.
#[test]
fn test_7439_a_value_attached_to_from_file_is_refused() -> Result<()> {
    let dir = fixture()?;
    for argv in [["-fprog.jq", "a.json"], ["--from-file=prog.jq", "a.json"]] {
        let (stdout, stderr, code) = run_jq_in_dir(dir.path(), &argv, None)?;
        assert_eq!((stdout.as_str(), code), ("", 2), "jq {argv:?}: {stderr}");
    }
    Ok(())
}
