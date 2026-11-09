//! #6730 (family 2): `-n -s --stream` over a text that faults, then a failure after it, says the
//! failure and exits 2 as jq does.
//!
//! jq's slurp reads every file before it closes the array, so the program's `input` meets the
//! stream's faults one per call and then the array, and the array is read past a last file that did
//! not open. succinctly handed the faults out and then the array with no failure to reach (the
//! reads placed none: `--stream` was left to the stream reader that `-s` does not use), so the run
//! ended with exit status 0 and nothing said. A text with no fault, a last directory, a program
//! that stops at the fault and the other flags keep their rows. Every row was captured from jq 1.7.1.

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
        ("bad.json", "1 }\n2"),
        ("badn.json", "1 }\n2\n"),
        ("flt.json", "1 }"),
        ("m.json", "1 }\n2 ]\n3"),
        ("unt.json", "1 }\n\"x"),
        ("one.json", "1"),
        ("nl.json", "1\n"),
        ("multi.json", "1\n2\n3\n"),
        ("two.json", "1 2\n"),
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
fn a_slurped_stream_that_faults_reaches_the_failure_after_it_6730() -> Result<()> {
    #[rustfmt::skip]
    let rows = [
        (&["-c", "-n", "-s", "--stream", "(try input catch .),(try input catch .)"][..], &["bad", "gone"][..], "\"Unmatched '}' at the top-level at line 1, column 3\"\n[[[],1],[[],2]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "--stream", "(try input catch .),(try input catch .)"][..], &["bad", "gone", "gone2"][..], "\"Unmatched '}' at the top-level at line 1, column 3\"\n[[[],1],[[],2]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "--stream", "(try input catch .),(try input catch .)"][..], &["badn", "gone"][..], "\"Unmatched '}' at the top-level at line 1, column 3\"\n[[[],1],[[],2]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "--stream", "(try input catch .),(try input catch .)"][..], &["flt", "gone"][..], "\"Unmatched '}' at the top-level at line 1, column 3\"\n[[[],1]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "--stream", "(try input catch .),(try input catch .)"][..], &["unt", "gone"][..], "\"Unmatched '}' at the top-level at line 1, column 3\"\n\"Unfinished string at EOF at line 1, column 5\"\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "--stream", "[(try input catch .),(try input catch .),(try input catch .)]"][..], &["bad", "gone"][..], "[\"Unmatched '}' at the top-level at line 1, column 3\",[[[],1],[[],2]],\"break\"]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "--stream", "[(try input catch .),(try input catch .),(try input catch .)]"][..], &["bad", "gone", "gone2"][..], "[\"Unmatched '}' at the top-level at line 1, column 3\",[[[],1],[[],2]],\"break\"]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "--stream", "[(try input catch .),(try input catch .),(try input catch .)]"][..], &["badn", "gone"][..], "[\"Unmatched '}' at the top-level at line 1, column 3\",[[[],1],[[],2]],\"break\"]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "--stream", "[(try input catch .),(try input catch .),(try input catch .)]"][..], &["flt", "gone"][..], "[\"Unmatched '}' at the top-level at line 1, column 3\",[[[],1]],\"break\"]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "--stream", "[(try input catch .),(try input catch .),(try input catch .)]"][..], &["m", "gone"][..], "[\"Unmatched '}' at the top-level at line 1, column 3\",\"Unmatched ']' at the top-level at line 1, column 6\",[[[],1],[[],2],[[],3]]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "--stream", "[(try input catch .),(try input catch .),(try input catch .)]"][..], &["unt", "gone"][..], "[\"Unmatched '}' at the top-level at line 1, column 3\",\"Unfinished string at EOF at line 1, column 5\",[[[],1]]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "--stream", "(try input catch .),(try input catch .),(try input catch .),(try input catch .)"][..], &["bad", "gone"][..], "\"Unmatched '}' at the top-level at line 1, column 3\"\n[[[],1],[[],2]]\n\"break\"\n\"break\"\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "--stream", "(try input catch .),(try input catch .),(try input catch .),(try input catch .)"][..], &["bad", "gone", "gone2"][..], "\"Unmatched '}' at the top-level at line 1, column 3\"\n[[[],1],[[],2]]\n\"break\"\n\"break\"\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error: Could not open file @DIR@/gone2.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "--stream", "(try input catch .),(try input catch .),(try input catch .),(try input catch .)"][..], &["badn", "gone"][..], "\"Unmatched '}' at the top-level at line 1, column 3\"\n[[[],1],[[],2]]\n\"break\"\n\"break\"\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "--stream", "(try input catch .),(try input catch .),(try input catch .),(try input catch .)"][..], &["flt", "gone"][..], "\"Unmatched '}' at the top-level at line 1, column 3\"\n[[[],1]]\n\"break\"\n\"break\"\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "--stream", "(try input catch .),(try input catch .),(try input catch .),(try input catch .)"][..], &["m", "gone"][..], "\"Unmatched '}' at the top-level at line 1, column 3\"\n\"Unmatched ']' at the top-level at line 1, column 6\"\n[[[],1],[[],2],[[],3]]\n\"break\"\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "--stream", "(try input catch .),(try input catch .),(try input catch .),(try input catch .)"][..], &["unt", "gone"][..], "\"Unmatched '}' at the top-level at line 1, column 3\"\n\"Unfinished string at EOF at line 1, column 5\"\n[[[],1]]\n\"break\"\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "--stream", "(try input catch .),(try input catch .)"][..], &["bad", "dir"][..], "\"Unmatched '}' at the top-level at line 1, column 3\"\n[[[],1],[[],2]]\n", "", 2),
        (&["-c", "-n", "-s", "--stream", "(try input catch .),(try input catch .)"][..], &["bad", "gone", "dir"][..], "\"Unmatched '}' at the top-level at line 1, column 3\"\n[[[],1],[[],2]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "--stream", "(try input catch .),(try input catch .)"][..], &["bad", "gone", "nl"][..], "\"Unmatched '}' at the top-level at line 1, column 3\"\n[[[],1],[[],21]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
    ];
    check(&rows)
}

/// Rows that matched jq before this change, and must go on matching.
#[test]
fn the_slurped_streams_that_already_matched_keep_matching_6730() -> Result<()> {
    #[rustfmt::skip]
    let rows = [
        (&["-c", "-n", "-s", "--stream", "(try input catch .),(try input catch .)"][..], &["m", "gone"][..], "\"Unmatched '}' at the top-level at line 1, column 3\"\n\"Unmatched ']' at the top-level at line 1, column 6\"\n", "", 0),
        (&["-c", "-n", "-s", "--stream", "[.,(try input catch .)]"][..], &["bad", "gone"][..], "[null,\"Unmatched '}' at the top-level at line 1, column 3\"]\n", "", 0),
        (&["-c", "-n", "-s", "--stream", "[.,(try input catch .)]"][..], &["bad", "gone", "gone2"][..], "[null,\"Unmatched '}' at the top-level at line 1, column 3\"]\n", "", 0),
        (&["-c", "-n", "-s", "--stream", "[.,(try input catch .)]"][..], &["badn", "gone"][..], "[null,\"Unmatched '}' at the top-level at line 1, column 3\"]\n", "", 0),
        (&["-c", "-n", "-s", "--stream", "[.,(try input catch .)]"][..], &["flt", "gone"][..], "[null,\"Unmatched '}' at the top-level at line 1, column 3\"]\n", "", 0),
        (&["-c", "-n", "-s", "--stream", "[.,(try input catch .)]"][..], &["m", "gone"][..], "[null,\"Unmatched '}' at the top-level at line 1, column 3\"]\n", "", 0),
        (&["-c", "-n", "-s", "--stream", "[.,(try input catch .)]"][..], &["unt", "gone"][..], "[null,\"Unmatched '}' at the top-level at line 1, column 3\"]\n", "", 0),
        (&["-c", "-n", "-s", "--stream", "[inputs]"][..], &["bad", "gone"][..], "", "jq: error (at @DIR@/bad.json:1): Unmatched '}' at the top-level at line 1, column 3\n", 5),
        (&["-c", "-n", "-s", "--stream", "[inputs]"][..], &["bad", "gone", "gone2"][..], "", "jq: error (at @DIR@/bad.json:1): Unmatched '}' at the top-level at line 1, column 3\n", 5),
        (&["-c", "-n", "-s", "--stream", "[inputs]"][..], &["badn", "gone"][..], "", "jq: error (at @DIR@/badn.json:1): Unmatched '}' at the top-level at line 1, column 3\n", 5),
        (&["-c", "-n", "-s", "--stream", "[inputs]"][..], &["flt", "gone"][..], "", "jq: error (at @DIR@/flt.json:0): Unmatched '}' at the top-level at line 1, column 3\n", 5),
        (&["-c", "-n", "-s", "--stream", "[inputs]"][..], &["m", "gone"][..], "", "jq: error (at @DIR@/m.json:1): Unmatched '}' at the top-level at line 1, column 3\n", 5),
        (&["-c", "-n", "-s", "--stream", "[inputs]"][..], &["unt", "gone"][..], "", "jq: error (at @DIR@/unt.json:1): Unmatched '}' at the top-level at line 1, column 3\n", 5),
        (&["-c", "-n", "-s", "--stream", "first(inputs)"][..], &["bad", "gone"][..], "", "jq: error (at @DIR@/bad.json:1): Unmatched '}' at the top-level at line 1, column 3\n", 5),
        (&["-c", "-n", "-s", "--stream", "first(inputs)"][..], &["bad", "gone", "gone2"][..], "", "jq: error (at @DIR@/bad.json:1): Unmatched '}' at the top-level at line 1, column 3\n", 5),
        (&["-c", "-n", "-s", "--stream", "first(inputs)"][..], &["badn", "gone"][..], "", "jq: error (at @DIR@/badn.json:1): Unmatched '}' at the top-level at line 1, column 3\n", 5),
        (&["-c", "-n", "-s", "--stream", "first(inputs)"][..], &["flt", "gone"][..], "", "jq: error (at @DIR@/flt.json:0): Unmatched '}' at the top-level at line 1, column 3\n", 5),
        (&["-c", "-n", "-s", "--stream", "first(inputs)"][..], &["m", "gone"][..], "", "jq: error (at @DIR@/m.json:1): Unmatched '}' at the top-level at line 1, column 3\n", 5),
        (&["-c", "-n", "-s", "--stream", "first(inputs)"][..], &["unt", "gone"][..], "", "jq: error (at @DIR@/unt.json:1): Unmatched '}' at the top-level at line 1, column 3\n", 5),
        (&["-c", "-n", "-s", "--stream", "(try input catch .)"][..], &["bad", "gone"][..], "\"Unmatched '}' at the top-level at line 1, column 3\"\n", "", 0),
        (&["-c", "-n", "-s", "--stream", "(try input catch .)"][..], &["bad", "gone", "gone2"][..], "\"Unmatched '}' at the top-level at line 1, column 3\"\n", "", 0),
        (&["-c", "-n", "-s", "--stream", "(try input catch .)"][..], &["badn", "gone"][..], "\"Unmatched '}' at the top-level at line 1, column 3\"\n", "", 0),
        (&["-c", "-n", "-s", "--stream", "(try input catch .)"][..], &["flt", "gone"][..], "\"Unmatched '}' at the top-level at line 1, column 3\"\n", "", 0),
        (&["-c", "-n", "-s", "--stream", "(try input catch .)"][..], &["m", "gone"][..], "\"Unmatched '}' at the top-level at line 1, column 3\"\n", "", 0),
        (&["-c", "-n", "-s", "--stream", "(try input catch .)"][..], &["unt", "gone"][..], "\"Unmatched '}' at the top-level at line 1, column 3\"\n", "", 0),
        (&["-c", "-n", "-s", "--stream", "input"][..], &["bad", "gone"][..], "", "jq: error (at @DIR@/bad.json:1): Unmatched '}' at the top-level at line 1, column 3\n", 5),
        (&["-c", "-n", "-s", "--stream", "input"][..], &["bad", "gone", "gone2"][..], "", "jq: error (at @DIR@/bad.json:1): Unmatched '}' at the top-level at line 1, column 3\n", 5),
        (&["-c", "-n", "-s", "--stream", "input"][..], &["badn", "gone"][..], "", "jq: error (at @DIR@/badn.json:1): Unmatched '}' at the top-level at line 1, column 3\n", 5),
        (&["-c", "-n", "-s", "--stream", "input"][..], &["flt", "gone"][..], "", "jq: error (at @DIR@/flt.json:0): Unmatched '}' at the top-level at line 1, column 3\n", 5),
        (&["-c", "-n", "-s", "--stream", "input"][..], &["m", "gone"][..], "", "jq: error (at @DIR@/m.json:1): Unmatched '}' at the top-level at line 1, column 3\n", 5),
        (&["-c", "-n", "-s", "--stream", "input"][..], &["unt", "gone"][..], "", "jq: error (at @DIR@/unt.json:1): Unmatched '}' at the top-level at line 1, column 3\n", 5),
        (&["-c", "-n", "-s", "--stream", "."][..], &["bad", "gone"][..], "null\n", "", 0),
        (&["-c", "-n", "-s", "--stream", "."][..], &["bad", "gone", "gone2"][..], "null\n", "", 0),
        (&["-c", "-n", "-s", "--stream", "."][..], &["badn", "gone"][..], "null\n", "", 0),
        (&["-c", "-n", "-s", "--stream", "."][..], &["flt", "gone"][..], "null\n", "", 0),
        (&["-c", "-n", "-s", "--stream", "."][..], &["m", "gone"][..], "null\n", "", 0),
        (&["-c", "-n", "-s", "--stream", "."][..], &["unt", "gone"][..], "null\n", "", 0),
        (&["-c", "-n", "-s", "--stream", "(try input catch .),(try input catch .)"][..], &["one", "gone"][..], "[[[],1]]\n\"break\"\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "--stream", "(try input catch .)"][..], &["one", "gone"][..], "[[[],1]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "--stream", "(try input catch .)"][..], &["bad", "gone"][..], "\"Unmatched '}' at the top-level at line 1, column 3\"\n", "", 0),
        (&["-c", "-n", "-s", "--stream", "(try input catch .),(try input catch .)"][..], &["bad", "nl"][..], "\"Unmatched '}' at the top-level at line 1, column 3\"\n[[[],1],[[],21]]\n", "", 0),
        (&["-c", "-n", "-s", "--stream", "(try input catch .),(try input catch .)"][..], &["one", "dir"][..], "[[[],1]]\n\"break\"\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--stream", "(try input catch .),(try input catch .)"][..], &["nl", "gone"][..], "[[[],1]]\n\"break\"\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "--stream", "(try input catch .),(try input catch .)"][..], &["multi", "gone"][..], "[[[],1],[[],2],[[],3]]\n\"break\"\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "--stream-errors", "(try input catch .),(try input catch .)"][..], &["bad", "gone"][..], "[[[],1],[\"Unmatched '}' at the top-level at line 1, column 3\",[]],[[],2]]\n\"break\"\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "(try input catch .),(try input catch .)"][..], &["bad", "gone"][..], "\"Unmatched '}' at line 1, column 3\"\n[1,2]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "-n", "--stream", "(try input catch .),(try input catch .)"][..], &["bad", "gone"][..], "[[],1]\n\"Unmatched '}' at the top-level at line 1, column 3\"\n", "", 0),
        (&["-c", "-s", "--stream", "."][..], &["bad", "gone"][..], "", "jq: parse error: Unmatched '}' at the top-level at line 1, column 3\n", 5),
        (&["-c", "-n", "-s", "--stream", "(try input catch .),(try input catch .)"][..], &["two", "gone"][..], "[[[],1],[[],2]]\n\"break\"\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
    ];
    check(&rows)
}
