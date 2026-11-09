//! #6732: under `--stream-errors`, a file of several values behind a directory completes the first
//! event a number the files before it end in started, and the failures behind it are not reached.
//!
//! `1` with no newline goes on into the next file that has bytes; with `r2.json` = `1 2` the first
//! event (`[[],11]`) is whole at the space, so jq's main loop stops there and never opens what follows
//! (`gone.json`). The rule judged the file by its last token (`2`, a bare number) and sent the run up
//! front, which said every failure. Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

/// One row: the flags, the files, then jq's stdout, stderr and exit status.
type Row<'a> = (&'a [&'a str], &'a [&'a str], &'a str, &'a str, i32);

/// Creates the files the rows name (`gone` and `gone2` stay absent) and runs each row.
fn check(rows: &[Row<'_>]) -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::create_dir(dir.path().join("dir"))?;
    for (name, text) in [
        ("p.json", "1"),
        ("one.json", "1"),
        ("empty.json", ""),
        ("pn.json", "1\n"),
        ("r2.json", "1 2"),
        ("r3.json", "1 2 3"),
        ("rn.json", "1 2\n"),
        ("sp.json", " 2"),
        ("ab.json", "ab"),
        ("c1.json", "[1] 2"),
        ("s.json", "\"a\"\n"),
        ("arr.json", "[1,2]"),
        ("str.json", "\"a\" 12"),
        ("str2.json", "\"a\" 12"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    for &(flags, names, stdout, stderr, status) in rows {
        let files: Vec<String> = names
            .iter()
            .map(|name| match *name {
                "dir" => format!("{d}/{name}"),
                _ => format!("{d}/{name}.json"),
            })
            .collect();
        let mut args: Vec<&str> = flags.to_vec();
        args.extend(files.iter().map(String::as_str));
        let (out, err, code) = run_jq_full(&args, None)?;
        let (want_out, want_err) = (stdout.replace("@DIR@", &d), stderr.replace("@DIR@", &d));
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (want_out.as_str(), want_err.as_str(), status),
            "{flags:?} {names:?}"
        );
    }
    Ok(())
}

#[test]
fn a_file_of_several_values_completes_the_event_a_bare_number_started_6732() -> Result<()> {
    #[rustfmt::skip]
    let rows = [
        (&["-c", "--stream-errors", "."][..], &["p", "dir", "r2", "gone"][..], "[[],11]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "--stream-errors", ".,halt_error"][..], &["p", "dir", "r2", "gone"][..], "[[],11]\n", "jq: error: Is a directory\n[[],11]\n", 2),
        (&["-c", "--stream-errors", "halt"][..], &["p", "dir", "r2", "gone"][..], "", "jq: error: Is a directory\n", 2),
        (&["-c", "--stream-errors", "halt_error"][..], &["p", "dir", "r2", "gone"][..], "", "jq: error: Is a directory\n[[],11]\n", 2),
        (&["-c", "--stream-errors", "halt_error(3)"][..], &["p", "dir", "r2", "gone"][..], "", "jq: error: Is a directory\n[[],11]\n", 2),
        (&["-c", "--stream-errors", "halt_error"][..], &["one", "dir", "r2", "gone"][..], "", "jq: error: Is a directory\n[[],11]\n", 2),
        (&["-c", "--stream-errors", "halt_error"][..], &["p", "dir", "r3", "gone"][..], "", "jq: error: Is a directory\n[[],11]\n", 2),
        (&["-c", "--stream-errors", "halt_error"][..], &["p", "dir", "sp", "gone"][..], "", "jq: error: Is a directory\n[[],1]\n", 2),
        (&["-c", "--stream-errors", "halt_error"][..], &["p", "dir", "c1", "gone"][..], "", "jq: error: Is a directory\n[[],1]\n", 2),
        (&["-c", "--stream-errors", "halt_error"][..], &["p", "dir", "str2", "gone"][..], "", "jq: error: Is a directory\n[[],1]\n", 2),
        (&["-c", "--stream-errors", "halt_error"][..], &["p", "empty", "dir", "r2", "gone"][..], "", "jq: error: Is a directory\n[[],11]\n", 2),
        (&["-c", "--stream-errors", "."][..], &["p", "gone", "r2", "gone2"][..], "[[],11]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
    ];
    check(&rows)
}

/// Rows that matched jq before this change, and must go on matching.
#[test]
fn the_files_that_do_not_complete_the_event_keep_their_rows_6732() -> Result<()> {
    #[rustfmt::skip]
    let rows = [
        (&["-c", "--stream-errors", "input"][..], &["p", "dir", "r2", "gone"][..], "[[],2]\n", "jq: error: Is a directory\njq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "--stream-errors", "first(inputs)|halt_error"][..], &["p", "dir", "r2", "gone"][..], "", "jq: error: Is a directory\njq: error: Could not open file @DIR@/gone.json: No such file or directory\n[[],2]\n", 2),
        (&["-c", "--stream-errors", "halt_error"][..], &["p", "dir", "rn", "gone"][..], "", "jq: error: Is a directory\n[[],11]\n", 2),
        (&["-c", "--stream-errors", "halt_error"][..], &["p", "dir", "ab", "gone"][..], "", "jq: error: Is a directory\njq: error: Could not open file @DIR@/gone.json: No such file or directory\n[\"Invalid numeric literal at EOF at line 1, column 3\",[]]\n", 2),
        (&["-c", "--stream-errors", "halt_error"][..], &["p", "dir", "pn", "gone"][..], "", "jq: error: Is a directory\n[[],11]\n", 2),
        (&["-c", "--stream-errors", "halt_error"][..], &["p", "dir", "s", "gone"][..], "", "jq: error: Is a directory\n[[],1]\n", 2),
        (&["-c", "--stream-errors", "halt_error"][..], &["p", "dir", "arr", "gone"][..], "", "jq: error: Is a directory\n[[],1]\n", 2),
        (&["-c", "--stream-errors", "halt_error"][..], &["p", "dir", "p", "gone"][..], "", "jq: error: Is a directory\njq: error: Could not open file @DIR@/gone.json: No such file or directory\n[[],11]\n", 2),
        (&["-c", "--stream-errors", "halt_error"][..], &["p", "gone", "r2", "dir"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n[[],11]\n", 2),
        (&["-c", "--stream-errors", "halt_error"][..], &["s", "dir", "r2", "gone"][..], "", "[[],\"a\"]\n", 5),
        (&["-c", "--stream-errors", "halt_error"][..], &["str", "dir", "r2", "gone"][..], "", "[[],\"a\"]\n", 5),
        (&["-c", "--stream-errors", "."][..], &["pn", "dir", "r2", "gone"][..], "[[],1]\n[[],1]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "--stream-errors", "[inputs]"][..], &["p", "dir", "r2", "gone"][..], "[[[],2]]\n", "jq: error: Is a directory\njq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "--stream-errors", "halt_error"][..], &["p", "dir", "r2"][..], "", "jq: error: Is a directory\n[[],11]\n", 2),
    ];
    check(&rows)
}
