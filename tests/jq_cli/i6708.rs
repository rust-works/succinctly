//! #6708: a program that reads `input` without `-n` stops at the first failure its reads reach.
//!
//! jq's main loop reads the first value, and the `input` calls of the program read on; once a read
//! has gone past a file that did not open (a last value that completes only at the end of its file
//! is read on into the next one), the loop reads no more, so a failure after the file that value
//! comes from is never opened. The up-front report said every failure at once. A program that reads
//! on past it (`[inputs]`) still meets them all. Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

/// One row: the flags, the files, then jq's stdout, stderr and exit status.
type Row<'a> = (&'a [&'a str], &'a [&'a str], &'a str, &'a str, i32);

/// Creates the files the rows name (`miss` and `miss2` stay absent) and runs each row.
fn check(rows: &[Row<'_>]) -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    for (name, text) in [
        ("str.json", "\"a\" 12"),
        ("one.json", "1"),
        ("empty.json", ""),
        ("s.json", "\"a\"\n"),
        ("arr.json", "[1,2]"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    for &(flags, names, stdout, stderr, status) in rows {
        let files: Vec<String> = names
            .iter()
            .map(|name| format!("{d}/{name}.json"))
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
fn a_second_failure_behind_the_value_that_passed_the_first_is_not_reached_6708() -> Result<()> {
    #[rustfmt::skip]
    let rows = [
        (&["-c", "input"][..], &["one", "miss", "s", "miss2"][..], "\"a\"\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "input"][..], &["str", "miss", "s", "miss2"][..], "12\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "first(inputs)"][..], &["one", "miss", "s", "miss2"][..], "\"a\"\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "first(inputs)"][..], &["str", "miss", "s", "miss2"][..], "12\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "[., input]"][..], &["one", "miss", "s", "miss2"][..], "[1,\"a\"]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "[., input]"][..], &["str", "miss", "s", "miss2"][..], "[\"a\",12]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "[input_line_number, input]"][..], &["one", "miss", "s", "miss2"][..], "[1,\"a\"]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "[input_line_number, input]"][..], &["str", "miss", "s", "miss2"][..], "[0,12]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "input"][..], &["one", "miss", "s", "miss2", "one"][..], "\"a\"\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "input"][..], &["one", "empty", "miss", "s", "miss2"][..], "\"a\"\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "[limit(1;inputs)]"][..], &["one", "miss", "s", "miss2"][..], "[\"a\"]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "try input catch \"e\""][..], &["one", "miss", "s", "miss2"][..], "\"a\"\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-e", "input"][..], &["one", "miss", "s", "miss2"][..], "\"a\"\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "--arg", "x", "1", "input"][..], &["one", "miss", "s", "miss2"][..], "\"a\"\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "[.,input]"][..], &["one", "miss", "s", "miss2"][..], "[1,\"a\"]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
    ];
    check(&rows)
}

/// Rows that matched jq before this change, and must go on matching.
#[test]
fn reads_that_go_on_and_the_main_loop_alone_keep_their_rows_6708() -> Result<()> {
    #[rustfmt::skip]
    let rows = [
        (&["-c", "input"][..], &["arr", "miss", "s", "miss2"][..], "\"a\"\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "input"][..], &["s", "miss", "s", "miss2"][..], "\"a\"\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "input"][..], &["one", "miss", "miss2", "s"][..], "\"a\"\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Could not open file @DIR@/miss2.json: No such file or directory\n", 2),
        (&["-c", "input"][..], &["one", "miss", "one", "miss2"][..], "", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Could not open file @DIR@/miss2.json: No such file or directory\njq: error (at @DIR@/miss2.json:0): break\n", 2),
        (&["-c", "[inputs]"][..], &["one", "miss", "s", "miss2"][..], "[\"a\"]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Could not open file @DIR@/miss2.json: No such file or directory\n", 2),
        (&["-c", "[input, input]"][..], &["one", "miss", "s", "miss2"][..], "", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Could not open file @DIR@/miss2.json: No such file or directory\njq: error (at @DIR@/miss2.json:0): break\n", 2),
        (&["-c", "."][..], &["one", "miss", "s", "miss2"][..], "1\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "reduce inputs as $x (.;.)"][..], &["one", "miss", "s", "miss2"][..], "1\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Could not open file @DIR@/miss2.json: No such file or directory\n", 2),
        (&["-c", "[limit(2;inputs)]"][..], &["one", "miss", "s", "miss2"][..], "[\"a\"]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Could not open file @DIR@/miss2.json: No such file or directory\n", 2),
        (&["-c", "[inputs]|length"][..], &["one", "miss", "s", "miss2"][..], "1\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Could not open file @DIR@/miss2.json: No such file or directory\n", 2),
        (&["-c", "[.,input,input]"][..], &["one", "miss", "s", "miss2"][..], "", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Could not open file @DIR@/miss2.json: No such file or directory\njq: error (at @DIR@/miss2.json:0): break\n", 2),
        (&["-c", "-s", "input"][..], &["one", "miss", "s", "miss2"][..], "", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Could not open file @DIR@/miss2.json: No such file or directory\njq: error (at @DIR@/miss2.json:0): break\n", 2),
        (&["-c", "-R", "input"][..], &["one", "miss", "s", "miss2"][..], "", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Could not open file @DIR@/miss2.json: No such file or directory\njq: error (at @DIR@/miss2.json:0): break\n", 2),
        (&["-c", "--seq", "input"][..], &["one", "miss", "s", "miss2"][..], "", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Could not open file @DIR@/miss2.json: No such file or directory\njq: ignoring parse error: Unfinished abandoned text at EOF at line 2, column 0\n", 2),
        (&["-c", "-n", "first(inputs)"][..], &["one", "miss", "s", "miss2"][..], "1\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "[limit(1;inputs)]"][..], &["one", "miss", "s", "miss2"][..], "[1]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
    ];
    check(&rows)
}
