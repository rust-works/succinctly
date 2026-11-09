//! #6730: the `--seq -s --stream` and `-s --stream-errors` form of the shape #6758 fixed without
//! `--seq`: a record (`trunc` holds `\u{1e}[1,`) left open at the end of a file and finished by the
//! next (`pa` holds `[1]`), then a file that did not open or a directory. jq reads the failed file as
//! an empty read of the stream: the closing event `pa` held back comes out in it and closes the
//! array, and the fault comes in a read the main loop never makes, so the array is printed and the
//! warning never said (`ptr miss`, where nothing is held back, meets the fault inside the array and
//! loses it); a directory is that read too, so `--stream-errors` does not print the fault as a second
//! output. Every row was captured from jq 1.7.1; the rows without a held-back closing event do not
//! move.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_seq_slurp_leaves_a_record_open_across_files_to_the_read_of_the_failed_file_6730() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::create_dir(dir.path().join("dir"))?;
    for (name, text) in [("trunc", "\u{1e}[1,"), ("pa", "[1]")] {
        std::fs::write(dir.path().join(name), text)?;
    }
    #[rustfmt::skip]
    let rows = [
        (&["-c", "--seq", "-s", "--stream", "."][..], &["trunc", "pa", "miss"][..], "\u{1e}[[[0],1],[[1,0],1],[[1,0]]]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "--seq", "-s", "--stream", "[., input_filename]"][..], &["trunc", "pa", "miss"][..], "\u{1e}[[[[0],1],[[1,0],1],[[1,0]]],\"@DIR@/miss\"]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "--seq", "-s", "--stream", "error(\"x\")"][..], &["trunc", "pa", "miss"][..], "", "jq: error: Could not open file @DIR@/miss: No such file or directory\njq: error (at @DIR@/miss:0): x\n", 2),
        (&["-c", "--seq", "-s", "--stream", "[., (try input catch .), (try input catch .)]"][..], &["trunc", "pa", "miss"][..], "\u{1e}[[[[0],1],[[1,0],1],[[1,0]]],\"Unfinished JSON term at EOF at line 1, column 7\",\"break\"]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "--seq", "-s", "--stream", "."][..], &["trunc", "pa", "dir"][..], "\u{1e}[[[0],1],[[1,0],1],[[1,0]]]\n", "", 2),
        (&["-c", "--seq", "-s", "--stream", "[., input_filename]"][..], &["trunc", "pa", "miss", "dir"][..], "", "jq: error: Could not open file @DIR@/miss: No such file or directory\njq: ignoring parse error: Unfinished JSON term at EOF at line 1, column 7\n", 2),
        (&["-c", "--seq", "-s", "--stream", "."][..], &["trunc", "miss"][..], "", "jq: error: Could not open file @DIR@/miss: No such file or directory\njq: ignoring parse error: Unfinished JSON term at EOF at line 1, column 4\n", 2),
        (&["-c", "--seq", "-s", "--stream-errors", "."][..], &["trunc", "pa", "miss"][..], "\u{1e}[[[0],1],[[1,0],1],[[1,0]]]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "--seq", "-s", "--stream-errors", "."][..], &["trunc", "pa", "dir"][..], "\u{1e}[[[0],1],[[1,0],1],[[1,0]]]\n", "", 2),
        (&["-c", "--seq", "-s", "--stream-errors", "length"][..], &["trunc", "pa", "dir"][..], "\u{1e}3\n", "", 2),
        (&["-c", "--seq", "-s", "--stream-errors", "error(\"x\")"][..], &["trunc", "pa", "dir"][..], "", "jq: error (at @DIR@/dir:0): x\n", 2),
        (&["-c", "--seq", "-s", "--stream-errors", "[., input_filename]"][..], &["pa", "dir"][..], "\u{1e}[[[\"Unfinished abandoned text at EOF at line 1, column 3\",[]]],\"@DIR@/dir\"]\n", "", 2),
        (&["-c", "--seq", "-s", "--stream-errors", "."][..], &["trunc", "miss"][..], "\u{1e}[[[0],1],[\"Unfinished JSON term at EOF at line 1, column 4\",[1]]]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
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
