//! #6712: a program that reads `input` without `-n`, over a text that faults before its last value,
//! reaches the failure after that value as it does without a fault (#6708).
//!
//! The fault is raised by one of the program's reads, and the reads after it go on: the last value
//! of the text (`12` of `"a" 12`) completes only when jq's reader has gone on past the end of the
//! last file that opened, so the read of it meets a last file that did not open. The text's last
//! value was placed at that file only when nothing faulted. Every row was captured from jq 1.7.1.

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
        ("flt.json", "1 }"),
        ("trunc.json", "[1,"),
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
fn a_last_value_after_a_fault_reaches_the_failure_behind_it_6712() -> Result<()> {
    #[rustfmt::skip]
    let rows = [
        (&["-c", "input"][..], &["flt", "miss", "str", "miss2"][..], "12\n", "jq: error (at @DIR@/flt.json:0): Unmatched '}' at line 1, column 3\njq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Could not open file @DIR@/miss2.json: No such file or directory\n", 2),
        (&["-c", "first(inputs)"][..], &["flt", "miss", "str", "miss2"][..], "12\n", "jq: error (at @DIR@/flt.json:0): Unmatched '}' at line 1, column 3\njq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Could not open file @DIR@/miss2.json: No such file or directory\n", 2),
        (&["-c", "[., input]"][..], &["flt", "miss", "str", "miss2"][..], "[\"a\",12]\n", "jq: error (at @DIR@/flt.json:0): Unmatched '}' at line 1, column 3\njq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Could not open file @DIR@/miss2.json: No such file or directory\n", 2),
        (&["-c", "[input_line_number, input]"][..], &["flt", "miss", "str", "miss2"][..], "[0,12]\n", "jq: error (at @DIR@/flt.json:0): Unmatched '}' at line 1, column 3\njq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Could not open file @DIR@/miss2.json: No such file or directory\n", 2),
        (&["-c", "[., input]"][..], &["one", "flt", "miss", "str", "miss2"][..], "[\"a\",12]\n", "jq: error (at @DIR@/flt.json:0): Unmatched '}' at line 1, column 4\njq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Could not open file @DIR@/miss2.json: No such file or directory\n", 2),
    ];
    check(&rows)
}

/// Rows that matched jq before this change, and must go on matching.
#[test]
fn the_rows_around_a_fault_that_already_matched_keep_matching_6712() -> Result<()> {
    #[rustfmt::skip]
    let rows = [
        (&["-c", "-n", "[inputs]"][..], &["flt", "miss", "str", "miss2"][..], "", "jq: error (at @DIR@/flt.json:0): Unmatched '}' at line 1, column 3\n", 5),
        (&["-c", "-n", "input"][..], &["flt", "miss", "str", "miss2"][..], "1\n", "", 0),
        (&["-c", "[inputs]"][..], &["flt", "miss", "str", "miss2"][..], "[12]\n", "jq: error (at @DIR@/flt.json:0): Unmatched '}' at line 1, column 3\njq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Could not open file @DIR@/miss2.json: No such file or directory\n", 2),
        (&["-c", "."][..], &["flt", "miss", "str", "miss2"][..], "1\n", "jq: parse error: Unmatched '}' at line 1, column 3\n", 5),
        (&["-c", "input"][..], &["flt", "miss", "str", "miss2", "one"][..], "121\n", "jq: error (at @DIR@/flt.json:0): Unmatched '}' at line 1, column 3\njq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Could not open file @DIR@/miss2.json: No such file or directory\n", 2),
        (&["-c", "input"][..], &["flt", "miss", "miss2", "str"][..], "12\n", "jq: error (at @DIR@/flt.json:0): Unmatched '}' at line 1, column 3\njq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Could not open file @DIR@/miss2.json: No such file or directory\n", 2),
        (&["-c", "input"][..], &["flt", "miss", "one", "miss2"][..], "", "jq: error (at @DIR@/flt.json:0): Unmatched '}' at line 1, column 3\njq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Could not open file @DIR@/miss2.json: No such file or directory\njq: error (at @DIR@/miss2.json:0): break\n", 2),
        (&["-c", "input"][..], &["str", "miss", "flt", "miss2"][..], "121\n", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\n", 2),
        (&["-c", "input"][..], &["trunc", "miss", "str", "miss2"][..], "", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Could not open file @DIR@/miss2.json: No such file or directory\njq: parse error: Expected separator between values at EOF at line 1, column 9\n", 2),
        (&["-c", "."][..], &["flt", "miss", "miss2"][..], "1\n", "jq: parse error: Unmatched '}' at line 1, column 3\n", 5),
        (&["-c", "-n", "input"][..], &["flt", "miss", "miss2"][..], "1\n", "", 0),
        (&["-c", "-s", "input"][..], &["flt", "miss", "str", "miss2"][..], "", "jq: parse error: Unmatched '}' at line 1, column 3\n", 5),
        (&["-c", "-R", "input"][..], &["flt", "miss", "str", "miss2"][..], "", "jq: error: Could not open file @DIR@/miss.json: No such file or directory\njq: error: Could not open file @DIR@/miss2.json: No such file or directory\njq: error (at @DIR@/miss2.json:0): break\n", 2),
    ];
    check(&rows)
}
