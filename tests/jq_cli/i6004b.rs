//! #6004: a bare number or literal straight before the last byte of the input, a `[` or `{`, is a
//! value by the time jq's parser meets the end of input: it completes on the byte after it, which
//! is the opener the end leaves unfinished. So a slurp holds it (`printf '1[' | jq -cs .` prints
//! `[1]` and then fails), and the fault is named `<unknown>`, since the read that completed the
//! value is the read that reached the end. Space before the opener (`1 [`), or a string or
//! container before it (`"a"[`, `[1][`), leaves the value complete in an earlier read, so those
//! keep the old behaviour. Every row is what `/usr/bin/jq` 1.7.1 printed for the same stdin.

use anyhow::Result;

use crate::run_jq_full;

/// `(stdin, stdout, exit code, the first line of stderr)`
type Row = (&'static str, &'static str, i32, &'static str);

fn check(args: &[&str], rows: &[Row]) -> Result<()> {
    for &(text, stdout, code, first_stderr) in rows {
        let (out, err, exit) = run_jq_full(args, Some(text))?;
        assert_eq!(out.trim_end(), stdout, "{args:?} on {text:?}: stdout");
        assert_eq!(exit, code, "{args:?} on {text:?}: exit code");
        assert_eq!(
            err.lines().next().unwrap_or(""),
            first_stderr,
            "{args:?} on {text:?}: stderr"
        );
    }
    Ok(())
}

#[test]
fn a_slurp_holds_a_bare_token_that_ends_before_a_final_opener() -> Result<()> {
    check(
        &["-cs", "."],
        &[
            (
                "1[",
                "[1]",
                5,
                "jq: parse error: Unfinished JSON term at EOF at line 1, column 2",
            ),
            (
                "12{",
                "[12]",
                5,
                "jq: parse error: Unfinished JSON term at EOF at line 1, column 3",
            ),
            (
                "-1.5[",
                "[-1.5]",
                5,
                "jq: parse error: Unfinished JSON term at EOF at line 1, column 5",
            ),
            (
                "true[",
                "[true]",
                5,
                "jq: parse error: Unfinished JSON term at EOF at line 1, column 5",
            ),
            (
                "null{",
                "[null]",
                5,
                "jq: parse error: Unfinished JSON term at EOF at line 1, column 5",
            ),
            (
                "\"a\"1[",
                "[\"a\",1]",
                5,
                "jq: parse error: Unfinished JSON term at EOF at line 1, column 5",
            ),
            (
                "1 2[",
                "[1,2]",
                5,
                "jq: parse error: Unfinished JSON term at EOF at line 1, column 4",
            ),
            (
                "[1]1{",
                "[[1],1]",
                5,
                "jq: parse error: Unfinished JSON term at EOF at line 1, column 5",
            ),
            (
                "1 \"a\"1[",
                "[1,\"a\",1]",
                5,
                "jq: parse error: Unfinished JSON term at EOF at line 1, column 7",
            ),
            (
                "1 [",
                "",
                5,
                "jq: parse error: Unfinished JSON term at EOF at line 1, column 3",
            ),
            (
                "\"a\"[",
                "",
                5,
                "jq: parse error: Unfinished JSON term at EOF at line 1, column 4",
            ),
            (
                "[1][",
                "",
                5,
                "jq: parse error: Unfinished JSON term at EOF at line 1, column 4",
            ),
            (
                "{}{",
                "",
                5,
                "jq: parse error: Unfinished JSON term at EOF at line 1, column 3",
            ),
            (
                "true [",
                "",
                5,
                "jq: parse error: Unfinished JSON term at EOF at line 1, column 6",
            ),
            (
                "1[1",
                "",
                5,
                "jq: parse error: Unfinished JSON term at EOF at line 1, column 3",
            ),
            (
                "1[ ",
                "",
                5,
                "jq: parse error: Unfinished JSON term at EOF at line 1, column 3",
            ),
            (
                "1[[",
                "",
                5,
                "jq: parse error: Unfinished JSON term at EOF at line 1, column 3",
            ),
            (
                "x[",
                "",
                5,
                "jq: parse error: Invalid numeric literal at line 1, column 2",
            ),
            (
                "tru[",
                "",
                5,
                "jq: parse error: Invalid literal at line 1, column 4",
            ),
        ],
    )
}

#[test]
fn the_fault_after_such_a_token_is_named_unknown() -> Result<()> {
    check(
        &["-nc", "[inputs]"],
        &[
            (
                "1[",
                "",
                5,
                "jq: error (at <unknown>): Unfinished JSON term at EOF at line 1, column 2",
            ),
            (
                "12{",
                "",
                5,
                "jq: error (at <unknown>): Unfinished JSON term at EOF at line 1, column 3",
            ),
            (
                "true[",
                "",
                5,
                "jq: error (at <unknown>): Unfinished JSON term at EOF at line 1, column 5",
            ),
            (
                "\"a\"1[",
                "",
                5,
                "jq: error (at <unknown>): Unfinished JSON term at EOF at line 1, column 5",
            ),
            (
                "1 2[",
                "",
                5,
                "jq: error (at <unknown>): Unfinished JSON term at EOF at line 1, column 4",
            ),
            (
                "1 [",
                "",
                5,
                "jq: error (at <stdin>:0): Unfinished JSON term at EOF at line 1, column 3",
            ),
            (
                "\"a\"[",
                "",
                5,
                "jq: error (at <stdin>:0): Unfinished JSON term at EOF at line 1, column 4",
            ),
            (
                "[1][",
                "",
                5,
                "jq: error (at <stdin>:0): Unfinished JSON term at EOF at line 1, column 4",
            ),
            (
                "1[1",
                "",
                5,
                "jq: error (at <stdin>:0): Unfinished JSON term at EOF at line 1, column 3",
            ),
            (
                "{}{",
                "",
                5,
                "jq: error (at <stdin>:0): Unfinished JSON term at EOF at line 1, column 3",
            ),
            (
                "1[[",
                "",
                5,
                "jq: error (at <stdin>:0): Unfinished JSON term at EOF at line 1, column 3",
            ),
        ],
    )
}

/// The shape counts only where the stream really ends in it: the last file's last byte. A token
/// that ends a file but not the stream is carried onto the next file instead (#4305), and the rows
/// here are what `/usr/bin/jq` 1.7.1 printed for the same files. `{DIR}` is the directory of the files.
#[test]
fn the_shape_counts_only_at_the_end_of_the_whole_stream() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let prefix = dir.path().to_string_lossy().into_owned();
    for (name, bytes) in [
        ("a", &b"1["[..]),
        ("two", b"2"),
        ("empty", b""),
        ("b", b"1 2["),
        ("three", b"3"),
    ] {
        std::fs::write(dir.path().join(name), bytes)?;
    }
    type FileRow<'a> = (&'a [&'a str], &'a [&'a str], &'a str, i32, &'a str);
    let rows: &[FileRow] = &[
        (
            &["-cs", "."],
            &["two", "a"],
            "[21]",
            5,
            "jq: parse error: Unfinished JSON term at EOF at line 1, column 3",
        ),
        (
            &["-cs", "."],
            &["a", "two"],
            "",
            5,
            "jq: parse error: Unfinished JSON term at EOF at line 1, column 3",
        ),
        (
            &["-cs", "."],
            &["a", "empty"],
            "",
            5,
            "jq: parse error: Unfinished JSON term at EOF at line 1, column 2",
        ),
        (
            &["-cs", "."],
            &["empty", "a"],
            "[1]",
            5,
            "jq: parse error: Unfinished JSON term at EOF at line 1, column 2",
        ),
        (
            &["-cn", "[inputs]"],
            &["a", "empty"],
            "",
            5,
            "jq: error (at {DIR}/empty:0): Unfinished JSON term at EOF at line 1, column 2",
        ),
        (
            &["-cn", "[inputs]"],
            &["two", "a"],
            "",
            5,
            "jq: error (at <unknown>): Unfinished JSON term at EOF at line 1, column 3",
        ),
        (
            &["-cn", "[inputs]"],
            &["a", "two"],
            "",
            5,
            "jq: error (at {DIR}/two:0): Unfinished JSON term at EOF at line 1, column 3",
        ),
        (
            &["-cs", "."],
            &["b", "three"],
            "",
            5,
            "jq: parse error: Unfinished JSON term at EOF at line 1, column 5",
        ),
        (
            &["-cs", "."],
            &["three", "b"],
            "[31,2]",
            5,
            "jq: parse error: Unfinished JSON term at EOF at line 1, column 5",
        ),
    ];
    for &(flags, names, stdout, code, first_stderr) in rows {
        let paths: Vec<String> = names
            .iter()
            .map(|n| dir.path().join(n).to_string_lossy().into_owned())
            .collect();
        let mut args: Vec<&str> = flags.to_vec();
        args.extend(paths.iter().map(String::as_str));
        let (out, err, exit) = crate::run_jq_full(&args, None)?;
        let want_err = first_stderr.replace("{DIR}", &prefix);
        assert_eq!(out.trim_end(), stdout, "{flags:?} {names:?}: stdout");
        assert_eq!(exit, code, "{flags:?} {names:?}: exit code");
        assert_eq!(
            err.lines().next().unwrap_or(""),
            want_err,
            "{flags:?} {names:?}: stderr"
        );
    }
    Ok(())
}
