//! #6657: a `-n` program that stops before the last value of a file that ends in a bare number or
//! literal never makes the read that carries a failure after it.
//!
//! `"a" 12` completes `12` only when jq's reader has gone on into the next file, so a failure
//! between the files is the read of `12`'s to reach, and a program that reads `"a"` alone never
//! gets there: jq says nothing and exits 0. The up-front report said it anyway. A program that
//! reads on to `12` meets the failure there, so those rows keep their message. Every row was
//! captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

/// One row: the flags, the files, then jq's stdout, stderr and exit status.
type Row<'a> = (&'a [&'a str], &'a [&'a str], &'a str, &'a str, i32);

/// Creates the files the rows name (`miss` and `miss2` stay absent) and runs each row.
fn check(rows: &[Row<'_>]) -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::create_dir(dir.path().join("dir"))?;
    for (name, text) in [
        ("str.json", "\"a\" 12"),
        ("bare.json", "12"),
        ("one.json", "1"),
        ("empty.json", ""),
        ("s.json", "\"a\"\n"),
        ("flt.json", "1 }"),
        ("trunc.json", "[1,"),
        ("txt.json", "a\nb"),
        ("seqb.json", "\u{1e}\"a\"\u{1e}12"),
        ("arr.json", "[1,2]"),
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
fn a_program_that_stops_early_never_reaches_a_failure_after_the_last_value_6657() -> Result<()> {
    #[rustfmt::skip]
    let rows = [
        (&["-c", "-n", "input"][..], &["str", "miss", "one"][..], "\"a\"\n", "", 0),
        (&["-c", "-n", "first(inputs)"][..], &["str", "miss", "one"][..], "\"a\"\n", "", 0),
        (&["-c", "-n", "[., input]"][..], &["str", "miss", "one"][..], "[null,\"a\"]\n", "", 0),
        (&["-c", "-n", "[input_line_number, input]"][..], &["str", "miss", "one"][..], "[0,\"a\"]\n", "", 0),
        (&["-c", "-n", "input"][..], &["str", "miss", "miss2", "one"][..], "\"a\"\n", "", 0),
        (&["-c", "-n", "input"][..], &["str", "empty", "miss", "one"][..], "\"a\"\n", "", 0),
        (&["-c", "-n", "input"][..], &["str", "miss", "empty", "one"][..], "\"a\"\n", "", 0),
        (&["-c", "-n", "input"][..], &["str", "miss", "dir", "one"][..], "\"a\"\n", "", 0),
        (&["-c", "-n", "input"][..], &["str", "miss", "s"][..], "\"a\"\n", "", 0),
        // The failure after the last file's bare number was already a read's to place.
        (&["-c", "-n", "input"][..], &["str", "one", "miss"][..], "\"a\"\n", "", 0),
    ];
    check(&rows)
}

#[test]
fn a_program_that_reads_the_last_value_reaches_the_failure_with_it_6657() -> Result<()> {
    #[rustfmt::skip]
    let rows = [
        (&["-c", "-n", "[inputs]"][..], &["str", "miss", "one"][..], "[\"a\",121]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "[inputs]"][..], &["str", "miss", "miss2", "one"][..], "[\"a\",121]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Could not open file @DIR@/miss2.json: No such file or directory\n", 2),
        (&["-c", "-n", "[inputs, input_filename]"][..], &["str", "miss", "s"][..], "[\"a\",12,\"a\",\"@DIR@/s.json\"]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "input, input"][..], &["str", "miss", "s"][..], "\"a\"\n12\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "[input, input, input]"][..], &["str", "miss", "s"][..], "[\"a\",12,\"a\"]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "input"][..], &["bare", "miss", "one"][..], "121\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "[., input]"][..], &["bare", "miss", "one"][..], "[null,121]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "[inputs]"][..], &["bare", "miss", "one"][..], "[121]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
    ];
    check(&rows)
}

/// The fix touches the way `-n` places a failure behind any file, so the readers and texts it must
/// not change are pinned too: a text that faults, `-R`, `--seq`, `--stream`, `--stream-errors` and
/// `-s`. Each row matched jq before this change as well.
#[test]
fn faulting_texts_and_the_other_readers_keep_their_rows_6657() -> Result<()> {
    #[rustfmt::skip]
    let rows = [
        (&["-c", "-n", "input"][..], &["flt", "miss", "one"][..], "1\n", "", 0),
        (&["-c", "-n", "[inputs]"][..], &["flt", "miss", "one"][..], "", "jq: error (at @DIR@/flt.json:0): Unmatched '}' at line 1, column 3\n", 5),
        (&["-c", "-n", "first(inputs)"][..], &["trunc", "miss", "one"][..], "", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error (at @DIR@/one.json:0): Unfinished JSON term at EOF at line 1, column 4\n", 2),
        (&["-c", "-n", "input"][..], &["trunc", "miss", "one"][..], "", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error (at @DIR@/one.json:0): Unfinished JSON term at EOF at line 1, column 4\n", 2),
        (&["-c", "-n", "[., input]"][..], &["bare", "miss", "trunc"][..], "[null,12]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "-R", "input"][..], &["txt", "miss", "one"][..], "\"a\"\n", "", 0),
        (&["-c", "-n", "-R", "[inputs]"][..], &["txt", "miss", "one"][..], "[\"a\",\"b1\"]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "--seq", "input"][..], &["seqb", "miss", "one"][..], "\u{1e}\"a\"\n", "", 0),
        (&["-c", "-n", "--seq", "[inputs]"][..], &["seqb", "miss", "one"][..], "", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error (at @DIR@/one.json:0): Potentially truncated top-level numeric value at EOF at line 1, column 8\n", 2),
        (&["-c", "-n", "--stream", "input"][..], &["str", "miss", "one"][..], "[[],\"a\"]\n", "", 0),
        (&["-c", "-n", "--stream", "[inputs]"][..], &["str", "miss", "one"][..], "[[[],\"a\"],[[],121]]\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "-n", "--stream-errors", "first(inputs)"][..], &["str", "miss", "one"][..], "[[],\"a\"]\n", "", 0),
        (&["-c", "-n", "-s", "."][..], &["str", "miss", "one"][..], "null\n", "", 0),
        (&["-c", "-n", "--stream", "input"][..], &["arr", "miss", "one"][..], "[[0],1]\n", "", 0),
    ];
    check(&rows)
}
