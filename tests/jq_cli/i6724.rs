//! #6724: `-s --stream` and `-s --stream-errors` read a last file that did not open as one more
//! source of the slurp, an empty one, as `--seq --stream -s` does (#6676). The array is read at the
//! failure, so `input_filename` names the file that failed and `input_line_number` is 0, where the
//! slurp used to keep the location of the file before it; a closing event held back at the end of
//! the text before it goes in the array, and a program that reads `input` after it meets the end.
//! Every row was captured from jq 1.7.1; the `nors plain` and `nors two` rows have no failed file, and
//! do not move.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_slurp_is_read_at_a_last_file_that_did_not_open_6724() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::create_dir(dir.path().join("dir"))?;
    std::fs::create_dir(dir.path().join("dir2"))?;
    for (name, text) in [
        ("two", "\u{1e}2\n"),
        ("empty", ""),
        ("nors", "[1]\n"),
        ("plain", "1 [2]"),
        ("p3", "3"),
        ("ptr", "[1,"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    #[rustfmt::skip]
    let rows = [
        (&["-c", "-s", "--stream-errors", "[., input_filename]"][..], &["two", "miss"][..], "[[[\"Invalid numeric literal at line 2, column 0\",[]]],\"@DIR@/miss\"]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "-s", "--stream-errors", "[., input_line_number]"][..], &["two", "miss"][..], "[[[\"Invalid numeric literal at line 2, column 0\",[]]],0]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "-s", "--stream-errors", "[., input_filename]"][..], &["ptr", "dir", "miss"][..], "[[[[0],1],[\"Unfinished JSON term at EOF at line 1, column 3\",[1]]],\"@DIR@/miss\"]\n", "jq: error: Is a directory\njq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "-s", "--stream", "[., input_filename]"][..], &["empty", "miss"][..], "[[],\"@DIR@/miss\"]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "-s", "--stream", "[., input_filename]"][..], &["nors", "plain"][..], "[[[[0],1],[[0]],[[],1],[[0],2]],\"@DIR@/plain\"]\n[[[0]],null]\n", "", 0),
        (&["-c", "-s", "--stream-errors", "[., input_line_number]"][..], &["nors", "two"][..], "[[[[0],1],[[0]],[\"Invalid numeric literal at line 3, column 0\",[]]],1]\n", "", 0),
        (&["-c", "-s", "--stream", "[., input_filename]"][..], &["p3", "miss"][..], "[[[[],3]],\"@DIR@/miss\"]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "-s", "--stream", "[., input_line_number]"][..], &["nors", "miss"][..], "[[[[0],1],[[0]]],0]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "-s", "--stream", "[., input_filename]"][..], &["nors", "dir", "miss"][..], "[[[[0],1],[[0]]],\"@DIR@/miss\"]\n", "jq: error: Is a directory\njq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "-s", "--stream", "error(\"x\")"][..], &["dir2", "nors", "miss2", "miss2"][..], "", "jq: error: Is a directory\njq: error: Could not open file @DIR@/miss2: No such file or directory\njq: error: Could not open file @DIR@/miss2: No such file or directory\njq: error (at @DIR@/miss2:0): x\n", 2),
        (&["-c", "-s", "--stream", "[., (try input catch .), (try input catch .)]"][..], &["plain", "plain", "miss"][..], "[[[[],1],[[0],2],[[0]],[[],1],[[0],2],[[0]]],\"break\",\"break\"]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
    ];
    check_rows(&d, &rows)
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
