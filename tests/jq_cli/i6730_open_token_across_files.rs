//! #6730: under `-s --stream` and `-s --stream-errors` a token one file leaves open and the next
//! finishes (`ptr` holds `[1,`, `pa` holds `[1]`: the text `[1,[1]`) ends the text with a fault, and
//! where jq's reader meets it depends on the reads of the files after it. Each file that did not open
//! is an empty read of the stream, and a directory is one that fails: the closing event `pa` holds
//! back comes out in the read of the first, the fault in the read of the next, and the slurp loop
//! closes its array at the last. So `ptr pa miss` has the closing event in its array and never meets
//! the fault (jq's main loop reads no further once a file has failed), where `ptr pa miss miss2` has
//! the fault in its array too (`-s --stream` says it as a parse error with nothing printed); a program's `input` after the array reads the fault there (jq's main
//! loop does not). Every row was captured from jq 1.7.1; the rows of `ptr miss`,
//! `pa ptr miss`, `ptr pa`, `pa dir` and `ptr miss pa dir` do not move.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_token_left_open_across_files_meets_its_fault_at_the_read_jq_makes_it_in_6730() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::create_dir(dir.path().join("dir"))?;
    for (name, text) in [("ptr", "[1,"), ("pa", "[1]")] {
        std::fs::write(dir.path().join(name), text)?;
    }
    #[rustfmt::skip]
    let rows = [
        (&["-c", "-s", "--stream", "[., (try input catch .), (try input catch .)]"][..], &["ptr", "pa", "miss"][..], "[[[[0],1],[[1,0],1],[[1,0]]],\"Unfinished JSON term at EOF at line 1, column 6\",\"break\"]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "-s", "--stream", "[., (try input catch .), (try input catch .)]"][..], &["ptr", "pa", "dir"][..], "[[[[0],1],[[1,0],1],[[1,0]]],\"Unfinished JSON term at EOF at line 1, column 6\",\"break\"]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-s", "--stream", "[., input_filename]"][..], &["ptr", "pa", "miss"][..], "[[[[0],1],[[1,0],1],[[1,0]]],\"@DIR@/miss\"]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "-s", "--stream", "[., input_filename]"][..], &["ptr", "pa", "dir"][..], "[[[[0],1],[[1,0],1],[[1,0]]],\"@DIR@/dir\"]\n", "", 2),
        (&["-c", "-s", "--stream", "[., input_line_number]"][..], &["ptr", "pa", "dir", "miss"][..], "", "jq: error: Is a directory\njq: error: Could not open file @DIR@/miss: No such file or directory\njq: parse error: Unfinished JSON term at EOF at line 1, column 6\n", 2),
        (&["-c", "-s", "--stream", "[., input_filename]"][..], &["ptr", "pa", "miss", "miss2"][..], "", "jq: error: Could not open file @DIR@/miss: No such file or directory\njq: error: Could not open file @DIR@/miss2: No such file or directory\njq: parse error: Unfinished JSON term at EOF at line 1, column 6\n", 2),
        (&["-c", "-s", "--stream", "[., input_filename]"][..], &["ptr", "miss"][..], "", "jq: error: Could not open file @DIR@/miss: No such file or directory\njq: parse error: Unfinished JSON term at EOF at line 1, column 3\n", 2),
        (&["-c", "-s", "--stream", "."][..], &["pa", "ptr", "miss"][..], "", "jq: error: Could not open file @DIR@/miss: No such file or directory\njq: parse error: Unfinished JSON term at EOF at line 1, column 6\n", 2),
        (&["-c", "-s", "--stream-errors", "[., input_filename]"][..], &["ptr", "pa", "miss"][..], "[[[[0],1],[[1,0],1],[[1,0]]],\"@DIR@/miss\"]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "-s", "--stream-errors", "[., input_filename]"][..], &["ptr", "pa", "dir"][..], "[[[[0],1],[[1,0],1],[[1,0]]],\"@DIR@/dir\"]\n", "", 2),
        (&["-c", "-s", "--stream-errors", "[., input_filename]"][..], &["ptr", "pa", "miss", "miss2"][..], "[[[[0],1],[[1,0],1],[[1,0]],[\"Unfinished JSON term at EOF at line 1, column 6\",[1]]],\"@DIR@/miss2\"]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\njq: error: Could not open file @DIR@/miss2: No such file or directory\n", 2),
        (&["-c", "-s", "--stream-errors", "[., input_filename]"][..], &["ptr", "pa", "miss", "dir"][..], "[[[[0],1],[[1,0],1],[[1,0]],[\"Unfinished JSON term at EOF at line 1, column 6\",[1]]],\"@DIR@/dir\"]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "-s", "--stream-errors", "[., input_filename]"][..], &["ptr", "miss"][..], "[[[[0],1],[\"Unfinished JSON term at EOF at line 1, column 3\",[1]]],\"@DIR@/miss\"]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "-s", "--stream", "[., input_filename]"][..], &["pa", "dir"][..], "[[[[0],1],[[0]]],\"@DIR@/dir\"]\n", "", 2),
        (&["-c", "-s", "--stream-errors", "[., input_filename]"][..], &["pa", "dir"][..], "[[[[0],1],[[0]]],\"@DIR@/dir\"]\n", "", 2),
        (&["-c", "-s", "--stream", "[., input_filename]"][..], &["ptr", "miss", "pa", "dir"][..], "[[[[0],1],[[1,0],1],[[1,0]]],\"@DIR@/dir\"]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "-s", "--stream-errors", "[., input_filename]"][..], &["ptr", "miss", "pa", "dir"][..], "[[[[0],1],[[1,0],1],[[1,0]]],\"@DIR@/dir\"]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "-s", "--stream-errors", "[., input_filename]"][..], &["pa", "ptr", "miss"][..], "[[[[0],1],[[0]],[[0],1],[\"Unfinished JSON term at EOF at line 1, column 6\",[1]]],\"@DIR@/miss\"]\n", "jq: error: Could not open file @DIR@/miss: No such file or directory\n", 2),
        (&["-c", "-s", "--stream-errors", "[., input_filename]"][..], &["ptr", "pa"][..], "[[[[0],1],[[1,0],1]],\"@DIR@/pa\"]\n[[[1,0]],null]\n[[\"Unfinished JSON term at EOF at line 1, column 6\",[1]],null]\n", "", 0),
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
