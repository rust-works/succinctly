//! #6678: a program that reads `input` meets a failure behind a text that faults at its end in the
//! order jq does, when the text follows a file that reads cleanly.
//!
//! A clean file before the faulting text is the stream's lazy start, and the eager reader that takes
//! the stream over at the fault knew which of its faults is at the end of the text only when the
//! stream was read eagerly from the first byte. The failure behind the text is reached by the read
//! of the fault at its end (jq opens the next file before it raises `Unfinished string at EOF`), and
//! the read of the fault in the middle does not reach it. Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

/// One row: the flags, the files, then jq's stdout, stderr and exit status.
type Row<'a> = (&'a [&'a str], &'a [&'a str], &'a str, &'a str, i32);

/// Creates the files the rows name (`gone` and `gone2` stay absent) and runs each row.
fn check(rows: &[Row<'_>]) -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    for (name, text) in [
        ("m.json", "1 }\n2 ]\n3"),
        ("l3.json", "{\"a\":1}\n}\n5"),
        ("ft.json", "1 }\ntrue"),
        ("unt.json", "1 }\n\"x"),
        ("numtr.json", "1 }\n2e"),
        ("tru.json", "1 }\ntru"),
        ("trunc.json", "[1,"),
        ("multi.json", "1\n2\n3\n"),
        ("o1.json", "1\n"),
        ("br.json", "{\"a\":1}"),
        ("s.json", "\"a\"\n"),
        ("one.json", "1"),
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
fn a_failure_behind_a_text_that_faults_at_its_end_follows_a_clean_file_6678() -> Result<()> {
    #[rustfmt::skip]
    let rows = [
        (&["-c", "[inputs]"][..], &["multi", "unt", "gone"][..], "", "jq: error (at @DIR@/unt.json:1): Unmatched '}' at line 4, column 3\njq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: parse error: Unfinished string at EOF at line 4, column 5\n", 2),
        (&["-c", "[inputs]"][..], &["multi", "numtr", "gone"][..], "", "jq: error (at @DIR@/numtr.json:1): Unmatched '}' at line 4, column 3\njq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: parse error: Invalid numeric literal at EOF at line 4, column 5\n", 2),
        (&["-c", "[inputs]"][..], &["multi", "tru", "gone"][..], "", "jq: error (at @DIR@/tru.json:1): Unmatched '}' at line 4, column 3\njq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: parse error: Invalid literal at EOF at line 4, column 6\n", 2),
        (&["-c", "reduce inputs as $x (0;.+1)"][..], &["multi", "unt", "gone"][..], "", "jq: error (at @DIR@/unt.json:1): Unmatched '}' at line 4, column 3\njq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: parse error: Unfinished string at EOF at line 4, column 5\n", 2),
        (&["-c", "reduce inputs as $x (0;.+1)"][..], &["multi", "numtr", "gone"][..], "", "jq: error (at @DIR@/numtr.json:1): Unmatched '}' at line 4, column 3\njq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: parse error: Invalid numeric literal at EOF at line 4, column 5\n", 2),
        (&["-c", "reduce inputs as $x (0;.+1)"][..], &["multi", "tru", "gone"][..], "", "jq: error (at @DIR@/tru.json:1): Unmatched '}' at line 4, column 3\njq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: parse error: Invalid literal at EOF at line 4, column 6\n", 2),
        (&["-c", "[inputs,input_filename]"][..], &["multi", "unt", "gone"][..], "", "jq: error (at @DIR@/unt.json:1): Unmatched '}' at line 4, column 3\njq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: parse error: Unfinished string at EOF at line 4, column 5\n", 2),
        (&["-c", "[inputs,input_filename]"][..], &["multi", "numtr", "gone"][..], "", "jq: error (at @DIR@/numtr.json:1): Unmatched '}' at line 4, column 3\njq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: parse error: Invalid numeric literal at EOF at line 4, column 5\n", 2),
        (&["-c", "[inputs,input_filename]"][..], &["multi", "tru", "gone"][..], "", "jq: error (at @DIR@/tru.json:1): Unmatched '}' at line 4, column 3\njq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: parse error: Invalid literal at EOF at line 4, column 6\n", 2),
        (&["-c", "[inputs]"][..], &["o1", "unt", "gone", "gone2"][..], "", "jq: error (at @DIR@/unt.json:1): Unmatched '}' at line 2, column 3\njq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\njq: parse error: Unfinished string at EOF at line 2, column 5\n", 2),
        (&["-c", "[inputs]"][..], &["s", "numtr", "gone"][..], "", "jq: error (at @DIR@/numtr.json:1): Unmatched '}' at line 2, column 3\njq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: parse error: Invalid numeric literal at EOF at line 2, column 5\n", 2),
        (&["-c", "[inputs]"][..], &["br", "tru", "gone", "gone2"][..], "", "jq: error (at @DIR@/tru.json:1): Unmatched '}' at line 1, column 10\njq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\njq: parse error: Invalid literal at EOF at line 1, column 13\n", 2),
    ];
    check(&rows)
}

/// Rows that matched jq before this change, and must go on matching.
#[test]
fn the_rows_around_a_fault_at_the_end_that_already_matched_keep_matching_6678() -> Result<()> {
    #[rustfmt::skip]
    let rows = [
        (&["-c", "-n", "[inputs]"][..], &["multi", "unt", "gone"][..], "", "jq: error (at @DIR@/unt.json:1): Unmatched '}' at line 4, column 3\n", 5),
        (&["-c", "[inputs]"][..], &["unt", "gone"][..], "", "jq: error (at @DIR@/unt.json:1): Unmatched '}' at line 1, column 3\njq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: parse error: Unfinished string at EOF at line 1, column 5\n", 2),
        (&["-c", "[inputs]"][..], &["numtr", "gone"][..], "", "jq: error (at @DIR@/numtr.json:1): Unmatched '}' at line 1, column 3\njq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: parse error: Invalid numeric literal at EOF at line 1, column 5\n", 2),
        (&["-c", "input"][..], &["multi", "unt", "gone"][..], "2\n1\n", "jq: parse error: Unmatched '}' at line 4, column 3\n", 5),
        (&["-c", "."][..], &["multi", "unt", "gone"][..], "1\n2\n3\n1\n", "jq: parse error: Unmatched '}' at line 4, column 3\n", 5),
        (&["-c", "[inputs]"][..], &["multi", "trunc", "gone"][..], "", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): Unfinished JSON term at EOF at line 4, column 3\n", 2),
        (&["-c", "[inputs]"][..], &["multi", "unt", "one"][..], "", "jq: error (at @DIR@/unt.json:1): Unmatched '}' at line 4, column 3\njq: parse error: Unfinished string at EOF at line 4, column 6\n", 5),
        (&["-c", "[inputs]"][..], &["multi", "unt", "gone", "one"][..], "", "jq: error (at @DIR@/unt.json:1): Unmatched '}' at line 4, column 3\njq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: parse error: Unfinished string at EOF at line 4, column 6\n", 2),
        (&["-c", "-s", "."][..], &["multi", "unt", "gone"][..], "", "jq: parse error: Unmatched '}' at line 4, column 3\n", 5),
        (&["-c", "input_line_number,input"][..], &["m", "gone"][..], "1\n2\n0\n", "jq: error (at @DIR@/m.json:1): Unmatched '}' at line 1, column 3\njq: error (at @DIR@/m.json:2): Unmatched ']' at line 1, column 6\njq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): break\n", 2),
        (&["-c", "[input_line_number,input]"][..], &["l3", "gone"][..], "", "jq: error (at @DIR@/l3.json:2): Unmatched '}' at line 2, column 1\njq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): break\n", 2),
        (&["-c", "input_line_number,input"][..], &["ft", "gone"][..], "1\n0\n", "jq: error (at @DIR@/ft.json:1): Unmatched '}' at line 1, column 3\njq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): break\n", 2),
    ];
    check(&rows)
}
