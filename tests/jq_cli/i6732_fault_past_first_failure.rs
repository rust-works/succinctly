//! #6732: under `--stream-errors` a token the files leave open, with a file that did not open
//! after it and one that completes it later, meets its fault at the read jq makes it in. A bare
//! number (`2` of `1 2`) completes only when the reader asks for more, so what follows a failure may
//! be a fault raised at the end of the text, after the reader has opened, and counted, every file
//! that did not open (`[` then `miss`, `1 2`, `miss`: both reported, where we stopped at the first),
//! or one raised in a file that comes after a second failure (`[`, `miss`, `1 2`, `miss`, `[`).
//! A fault met mid-text in the file before the failures (`[1,2` then `{"a":1}`) is not a closing
//! event held back for the failure's read either. Every row was captured from jq 1.7.1; `unchanged`
//! holds the rows that already matched and must not move, a fault met in an earlier file among them.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_fault_past_the_first_failure_counts_every_failure_the_reader_asked_past_in_6732() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::create_dir(dir.path().join("dir"))?;
    for (name, text) in [
        ("open", "["),
        ("open2", "[1,"),
        ("open3", "[1,2"),
        ("r2", "1 2"),
        ("obj", "{\"a\":1}"),
        ("arr", "[1]"),
        ("bad", "1}"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    #[rustfmt::skip]
    let fixed = [
        (&["-c", "--stream-errors", "."][..], &["open", "miss", "r2", "miss"][..], "[\"Expected separator between values at EOF at line 1, column 4\",[0]]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\njq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "--stream-errors", "."][..], &["open", "miss", "r2", "miss", "dir"][..], "[\"Expected separator between values at EOF at line 1, column 4\",[0]]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\njq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "--stream-errors", "halt_error"][..], &["open", "miss", "r2", "miss"][..], "", "jq: error: Could not open file @DIR@/miss: No such file or directory\njq: error: Could not open file @DIR@/miss: No such file or directory\n[\"Expected separator between values at EOF at line 1, column 4\",[0]]\n", 2),
        (&["-c", "--stream-errors", "."][..], &["open", "dir", "r2", "miss"][..], "[\"Expected separator between values at EOF at line 1, column 4\",[0]]\n", "jq: error: Is a directory\njq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "--stream-errors", "."][..], &["open", "miss", "r2", "dir", "miss"][..], "[\"Expected separator between values at EOF at line 1, column 4\",[0]]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\njq: error: Is a directory\njq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "--stream-errors", "."][..], &["open2", "miss", "r2", "miss"][..], "[[0],1]\n[\"Expected separator between values at EOF at line 1, column 6\",[1]]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\njq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "--stream-errors", "."][..], &["open", "miss", "r2", "miss", "open"][..], "[\"Expected separator between values at line 1, column 5\",[0]]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\njq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "--stream-errors", "."][..], &["open2", "miss", "r2", "miss", "obj"][..], "[[0],1]\n[\"Expected separator between values at line 1, column 7\",[1]]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\njq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "--stream-errors", "."][..], &["open3", "obj", "miss", "miss"][..], "[[0],1]\n[\"Expected a separator between values at line 1, column 5\",[1]]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\njq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "--stream-errors", "."][..], &["open3", "obj", "dir", "miss"][..], "[[0],1]\n[\"Expected a separator between values at line 1, column 5\",[1]]\n", "jq: error: Is a directory\njq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "--stream-errors", "."][..], &["open3", "arr", "miss", "miss"][..], "[[0],1]\n[\"Expected a separator between values at line 1, column 5\",[1]]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\njq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
    ];
    #[rustfmt::skip]
    let unchanged = [
        (&["-c", "--stream-errors", "input"][..], &["open", "miss", "r2", "miss"][..], "", "jq: error: Could not open file @DIR@/miss: No such file or directory\njq: error: Could not open file @DIR@/miss: No such file or directory\njq: error (at @DIR@/miss:0): break\n", 2),
        (&["-c", "--stream-errors", "."][..], &["open", "miss", "r2"][..], "[\"Expected separator between values at EOF at line 1, column 4\",[0]]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "--stream-errors", "."][..], &["open", "r2", "miss", "miss"][..], "[\"Expected separator between values at EOF at line 1, column 4\",[0]]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\njq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "--stream-errors", "."][..], &["open", "miss", "obj", "miss"][..], "[[0,\"a\"],1]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "--stream-errors", "."][..], &["open", "miss", "miss", "r2"][..], "[\"Expected separator between values at EOF at line 1, column 4\",[0]]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\njq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "--stream-errors", "."][..], &["bad", "obj", "miss", "miss"][..], "[\"Unmatched '}' at the top-level at line 1, column 2\",[]]\n[[\"a\"],1]\n[[\"a\"]]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
    ];
    check_rows(&d, &fixed)?;
    check_rows(&d, &unchanged)
}

/// One row: the flags and program, the files under the fixture directory, and jq 1.7.1's stdout,
/// stderr and exit status, with `@DIR@` for the directory.
type Row<'a> = (&'a [&'a str], &'a [&'a str], &'a str, &'a str, i32);

fn check_rows(d: &str, rows: &[Row<'_>]) -> Result<()> {
    for &(flags, names, stdout, stderr, status) in rows {
        let files: Vec<String> = names.iter().map(|name| format!("{d}/{name}")).collect();
        let mut args: Vec<&str> = flags.to_vec();
        args.extend(files.iter().map(String::as_str));
        let (out, err, code) = run_jq_full(&args, None)?;
        let (want_out, want_err) = (stdout.replace("@DIR@", d), stderr.replace("@DIR@", d));
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (want_out.as_str(), want_err.as_str(), status),
            "{flags:?} {names:?}"
        );
    }
    Ok(())
}
