//! #6676, #6681: under `--seq --stream -s` without `-n`, a program that reads no `input` gets the
//! whole slurp from jq's main loop in its first call, and the stream reader's warnings come out as
//! it meets each fault. A file that fails to open is said when the reader gets to it, so a warning
//! met before it leaves the array (`mix.seq dir` keeps `[[[],2]]`), a failure sits among the
//! warnings by source, and a directory given last is read after every warning, which counts it with
//! nothing said. A warning met after a failure was reported stops the loop with the array lost, and
//! the warning at the end of the text is met after every file. Every row was captured from jq 1.7.1.
//!
//! A program that reads `input` is no exception: that whole slurp comes first, warnings and failures
//! included, and its `input` reads what is left after the array: the events and the one fault the
//! slurp's last call left, then the end of the stream. A last file that did not open is a source of the stream reader's
//! own, with or without `-n`: a closing event held back at the end of the file before it goes in the
//! array, and the array is read at the failure.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_slurp_places_its_open_failures_among_the_stream_readers_warnings_6676() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::create_dir(dir.path().join("dir"))?;
    std::fs::create_dir(dir.path().join("dir2"))?;
    for (name, text) in [
        ("mix.seq", "\u{1e}{]\n\u{1e}2"),
        ("mid.seq", "\u{1e}1\n\u{1e}{]\n\u{1e}3\n"),
        ("trunc.seq", "\u{1e}[1,"),
        ("rend.seq", "\u{1e}12\n\u{1e}[1,"),
        ("ok.seq", "\u{1e}[1]\n"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    #[rustfmt::skip]
    let rows = [
        (&["-c", "-s", "--seq", "--stream", "."][..], &["mix.seq", "dir"][..], "\u{1e}[[[],2]]\n", "jq: ignoring parse error: Unmatched ']' in the middle of an object at line 1, column 3 (need RS to resync)\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["mid.seq", "dir"][..], "\u{1e}[[[],1],[[],3]]\n", "jq: ignoring parse error: Unmatched ']' in the middle of an object at line 2, column 3 (need RS to resync)\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["mix.seq", "missing.json", "dir"][..], "\u{1e}[[[],2]]\n", "jq: ignoring parse error: Unmatched ']' in the middle of an object at line 1, column 3 (need RS to resync)\njq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-s", "--seq", "--stream", "length"][..], &["mid.seq", "dir", "dir2"][..], "\u{1e}2\n", "jq: ignoring parse error: Unmatched ']' in the middle of an object at line 2, column 3 (need RS to resync)\njq: error: Is a directory\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["mid.seq", "missing.json", "dir"][..], "\u{1e}[[[],1],[[],3]]\n", "jq: ignoring parse error: Unmatched ']' in the middle of an object at line 2, column 3 (need RS to resync)\njq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["trunc.seq", "dir"][..], "", "jq: ignoring parse error: Unfinished JSON term at EOF at line 1, column 4\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["rend.seq", "dir"][..], "", "jq: ignoring parse error: Unfinished JSON term at EOF at line 2, column 4\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["dir"][..], "", "jq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 0\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["ok.seq", "dir"][..], "\u{1e}[[[0],1],[[0]]]\n", "", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["mix.seq", "ok.seq", "dir"][..], "\u{1e}[[[],2],[[0],1],[[0]]]\n", "jq: ignoring parse error: Unmatched ']' in the middle of an object at line 1, column 3 (need RS to resync)\n", 2),
        (&["-c", "-s", "--seq", "--stream", "empty"][..], &["mix.seq", "dir"][..], "", "jq: ignoring parse error: Unmatched ']' in the middle of an object at line 1, column 3 (need RS to resync)\n", 2),
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

/// The fixtures of the tests below, each read through `--seq`, and two directories.
fn fixtures() -> Result<tempfile::TempDir> {
    let dir = tempfile::tempdir()?;
    std::fs::create_dir(dir.path().join("dir"))?;
    std::fs::create_dir(dir.path().join("dir2"))?;
    for (name, text) in [
        ("arr.seq", "\u{1e}[1]"),
        ("two.seq", "\u{1e}2\n"),
        ("trunc.seq", "\u{1e}[1,"),
        ("bad.seq", "\u{1e}[1}"),
        ("nors.seq", "[1]\n"),
        ("empty.seq", ""),
        ("mix.seq", "\u{1e}{]\n\u{1e}2"),
        ("rend.seq", "\u{1e}12\n\u{1e}[1,"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    Ok(dir)
}

/// The regression of #6694 is the first row: nothing read the failure, which exited 0 with nothing said.
/// `dir` in the middle of the list and `--stream-errors` read the same way, and the last two rows, with
/// no failure, still close the array at the file's own last buffer.
#[test]
fn a_failed_last_file_is_a_source_of_the_stream_readers_slurp_6676() -> Result<()> {
    let dir = fixtures()?;
    #[rustfmt::skip]
    let rows: &[Row<'_>] = &[
        (&["-c", "-n", "-s", "--seq", "--stream", "input"][..], &["two.seq", "missing.json"][..], "\u{1e}[[[],2]]\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream", "input"][..], &["arr.seq", "missing.json"][..], "\u{1e}[[[0],1],[[0]]]\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream", "input"][..], &["trunc.seq", "missing.json"][..], "", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: error (at @DIR@/missing.json:0): Unfinished JSON term at EOF at line 1, column 4\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream", "[inputs]"][..], &["arr.seq", "missing.json"][..], "\u{1e}[[[[0],1],[[0]]]]\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream", "input"][..], &["two.seq", "dir", "missing.json"][..], "\u{1e}[[[],2]]\n", "jq: error: Is a directory\njq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["arr.seq", "missing.json"][..], "\u{1e}[[[0],1],[[0]]]\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-s", "--seq", "--stream", "length"][..], &["arr.seq", "missing.json"][..], "\u{1e}2\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-s", "--seq", "--stream", "[., input_filename]"][..], &["two.seq", "missing.json"][..], "\u{1e}[[[[],2]],\"@DIR@/missing.json\"]\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-s", "--seq", "--stream", "[., input_line_number]"][..], &["arr.seq", "missing.json"][..], "\u{1e}[[[[0],1],[[0]]],0]\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["arr.seq", "dir", "missing.json"][..], "\u{1e}[[[0],1],[[0]]]\n", "jq: error: Is a directory\njq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-s", "--seq", "--stream-errors", "."][..], &["arr.seq", "missing.json"][..], "\u{1e}[[[0],1],[[0]]]\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-s", "--seq", "--stream-errors", "[., input_filename]"][..], &["two.seq", "missing.json"][..], "\u{1e}[[[[],2]],\"@DIR@/missing.json\"]\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["arr.seq"][..], "\u{1e}[[[0],1]]\n\u{1e}[[0]]\n", "", 0),
        (&["-c", "-n", "-s", "--seq", "--stream", "[inputs]"][..], &["arr.seq"][..], "\u{1e}[[[[0],1]],[[0]]]\n", "", 0),
    ];
    check_rows(&dir.path().to_string_lossy(), rows)
}

/// The warnings and the failures come first, in the order the reader meets them, and a warning met after
/// a failure stops the main loop before the program runs. The last rows have no failure.
#[test]
fn a_slurp_warns_before_a_program_that_reads_input_runs_6676() -> Result<()> {
    let dir = fixtures()?;
    #[rustfmt::skip]
    let rows: &[Row<'_>] = &[
        (&["-c", "-s", "--seq", "--stream", "input"][..], &["bad.seq", "missing.json"][..], "", "jq: ignoring parse error: Unmatched '}' in the middle of an array at line 1, column 4 (need RS to resync)\njq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: error (at @DIR@/missing.json:0): break\n", 2),
        (&["-c", "-s", "--seq", "--stream", "[inputs]"][..], &["bad.seq", "missing.json"][..], "\u{1e}[]\n", "jq: ignoring parse error: Unmatched '}' in the middle of an array at line 1, column 4 (need RS to resync)\njq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-s", "--seq", "--stream", "first(inputs)"][..], &["bad.seq", "arr.seq", "missing.json"][..], "", "jq: ignoring parse error: Unmatched '}' in the middle of an array at line 1, column 4 (need RS to resync)\njq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-s", "--seq", "--stream", "[inputs]"][..], &["trunc.seq", "trunc.seq", "missing.json"][..], "", "jq: ignoring parse error: Truncated value at line 1, column 5\njq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: ignoring parse error: Unfinished JSON term at EOF at line 1, column 8\n", 2),
        (&["-c", "-s", "--seq", "--stream", "input"][..], &["nors.seq", "missing.json"][..], "", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: ignoring parse error: Unfinished abandoned text at EOF at line 2, column 0\n", 2),
        (&["-c", "-s", "--seq", "--stream", "input"][..], &["trunc.seq", "missing.json"][..], "", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: ignoring parse error: Unfinished JSON term at EOF at line 1, column 4\n", 2),
        (&["-c", "-s", "--seq", "--stream", "[inputs]"][..], &["empty.seq", "missing.json"][..], "", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 0\n", 2),
        (&["-c", "-s", "--seq", "--stream", "input"][..], &["arr.seq", "missing.json"][..], "", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: error (at @DIR@/missing.json:0): break\n", 2),
        (&["-c", "-s", "--seq", "--stream", "[inputs]"][..], &["arr.seq", "missing.json"][..], "\u{1e}[]\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-s", "--seq", "--stream", "[.,input]"][..], &["two.seq", "missing.json"][..], "", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: error (at @DIR@/missing.json:0): break\n", 2),
        (&["-c", "-s", "--seq", "--stream", "input"][..], &["bad.seq", "dir", "missing.json"][..], "", "jq: ignoring parse error: Unmatched '}' in the middle of an array at line 1, column 4 (need RS to resync)\njq: error: Is a directory\njq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: error (at @DIR@/missing.json:0): break\n", 2),
        (&["-c", "-s", "--seq", "--stream", "input"][..], &["arr.seq"][..], "\u{1e}[[0]]\n", "", 0),
        (&["-c", "-s", "--seq", "--stream", "[.,input]"][..], &["arr.seq"][..], "\u{1e}[[[[0],1]],[[0]]]\n", "", 0),
        (&["-c", "-s", "--seq", "--stream", "[inputs]"][..], &["trunc.seq"][..], "", "jq: error (at <unknown>): Unfinished JSON term at EOF at line 1, column 4\n", 5),
        (&["-c", "-s", "--seq", "--stream", "[.,input_filename]"][..], &["trunc.seq"][..], "\u{1e}[[[[0],1]],\"@DIR@/trunc.seq\"]\n", "jq: ignoring parse error: Unfinished JSON term at EOF at line 1, column 4\n", 0),
    ];
    check_rows(&dir.path().to_string_lossy(), rows)
}

/// `Is a directory` comes with the read after the array, unless a warning at the end of the text stopped
/// the main loop first. A program that makes no such read counts it with nothing said.
#[test]
fn a_last_directory_is_read_by_the_programs_extra_input_6676() -> Result<()> {
    let dir = fixtures()?;
    #[rustfmt::skip]
    let rows: &[Row<'_>] = &[
        (&["-c", "-s", "--seq", "--stream", "input"][..], &["two.seq", "dir"][..], "", "jq: error: Is a directory\njq: error (at <unknown>): break\n", 2),
        (&["-c", "-s", "--seq", "--stream", "[inputs]"][..], &["two.seq", "dir"][..], "\u{1e}[]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-s", "--seq", "--stream", "[.,input_filename]"][..], &["two.seq", "dir"][..], "\u{1e}[[[[],2]],\"@DIR@/dir\"]\n", "", 2),
        (&["-c", "-s", "--seq", "--stream", "input"][..], &["empty.seq", "dir"][..], "", "jq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 0\n", 2),
        (&["-c", "-s", "--seq", "--stream", "[inputs]"][..], &["trunc.seq", "dir"][..], "", "jq: ignoring parse error: Unfinished JSON term at EOF at line 1, column 4\n", 2),
        (&["-c", "-s", "--seq", "--stream", "first(inputs)"][..], &["nors.seq", "dir"][..], "", "jq: ignoring parse error: Unfinished abandoned text at EOF at line 2, column 0\n", 2),
        (&["-c", "-s", "--seq", "--stream", "input"][..], &["bad.seq", "missing.json", "dir"][..], "", "jq: ignoring parse error: Unmatched '}' in the middle of an array at line 1, column 4 (need RS to resync)\njq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: error: Is a directory\njq: error (at <unknown>): break\n", 2),
        (&["-c", "-s", "--seq", "--stream", "input"][..], &["bad.seq", "dir", "dir2"][..], "", "jq: ignoring parse error: Unmatched '}' in the middle of an array at line 1, column 4 (need RS to resync)\njq: error: Is a directory\njq: error: Is a directory\njq: error (at <unknown>): break\n", 2),
        (&["-c", "-s", "--seq", "--stream", "[inputs]"][..], &["mix.seq", "dir"][..], "\u{1e}[]\n", "jq: ignoring parse error: Unmatched ']' in the middle of an object at line 1, column 3 (need RS to resync)\njq: error: Is a directory\n", 2),
        (&["-c", "-s", "--seq", "--stream", "[inputs]"][..], &["rend.seq", "dir"][..], "", "jq: ignoring parse error: Unfinished JSON term at EOF at line 2, column 4\n", 2),
    ];
    check_rows(&dir.path().to_string_lossy(), rows)
}

/// #6700's stream, a failure before a last file ending in an unfinished term: the program's `input` raises
/// the fault, and with no such read, whatever the program could have done, nothing says it.
#[test]
fn the_fault_after_the_array_is_for_the_programs_input_alone_6676() -> Result<()> {
    let dir = fixtures()?;
    #[rustfmt::skip]
    let rows: &[Row<'_>] = &[
        (&["-c", "-s", "--seq", "--stream", "try input catch ."][..], &["two.seq", "missing.json", "trunc.seq"][..], "\u{1e}\"Unfinished JSON term at EOF at line 2, column 4\"\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-s", "--seq", "--stream", "[inputs]"][..], &["two.seq", "missing.json", "trunc.seq"][..], "", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: error (at <unknown>): Unfinished JSON term at EOF at line 2, column 4\n", 2),
        (&["-c", "-s", "--seq", "--stream", "input"][..], &["arr.seq", "missing.json", "trunc.seq"][..], "", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: error (at <unknown>): Unfinished JSON term at EOF at line 1, column 8\n", 2),
        (&["-c", "-s", "--seq", "--stream", "if false then input else . end"][..], &["two.seq", "missing.json", "trunc.seq"][..], "\u{1e}[[[],2],[[0],1]]\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["two.seq", "missing.json", "trunc.seq"][..], "\u{1e}[[[],2],[[0],1]]\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-s", "--seq", "--stream", "[., input_filename]"][..], &["two.seq", "missing.json", "trunc.seq"][..], "\u{1e}[[[[],2],[[0],1]],\"@DIR@/trunc.seq\"]\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
    ];
    check_rows(&dir.path().to_string_lossy(), rows)
}

/// A warning met after a failure stops the main loop with the array lost, so no program runs: what the
/// stream holds after that warning, a fault at the end of the text included, is never read or said.
#[test]
fn a_slurp_stopped_by_a_warning_says_nothing_after_it_6676() -> Result<()> {
    let dir = fixtures()?;
    #[rustfmt::skip]
    let rows: &[Row<'_>] = &[
        (&["-c", "-s", "--seq", "--stream", "[.,input]"][..], &["dir", "mix.seq", "rend.seq"][..], "", "jq: error: Is a directory\njq: ignoring parse error: Unmatched ']' in the middle of an object at line 1, column 3 (need RS to resync)\n", 2),
        (&["-c", "-s", "--seq", "--stream", "input"][..], &["dir", "mix.seq", "rend.seq"][..], "", "jq: error: Is a directory\njq: ignoring parse error: Unmatched ']' in the middle of an object at line 1, column 3 (need RS to resync)\n", 2),
        (&["-c", "-s", "--seq", "--stream", "[inputs]"][..], &["empty.seq", "missing.json", "mix.seq", "trunc.seq"][..], "", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: ignoring parse error: Unmatched ']' in the middle of an object at line 1, column 3 (need RS to resync)\n", 2),
        (&["-c", "-s", "--seq", "--stream", "try input catch ."][..], &["two.seq", "missing.json", "bad.seq", "trunc.seq"][..], "", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: ignoring parse error: Unmatched '}' in the middle of an array at line 2, column 4 (need RS to resync)\n", 2),
        (&["-c", "-s", "--seq", "--stream", "[inputs, input_filename]"][..], &["dir", "bad.seq", "nors.seq", "missing.json", "trunc.seq"][..], "", "jq: error: Is a directory\njq: ignoring parse error: Unmatched '}' in the middle of an array at line 1, column 4 (need RS to resync)\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["dir", "mix.seq", "rend.seq"][..], "", "jq: error: Is a directory\njq: ignoring parse error: Unmatched ']' in the middle of an object at line 1, column 3 (need RS to resync)\n", 2),
    ];
    check_rows(&dir.path().to_string_lossy(), rows)
}

/// `--stream-errors` reads the same way, under `-n` and without it. Under `-n` a failure is for the read
/// that reaches it, so a program that names `input` and never makes that read says nothing and exits 0.
/// The last two rows are `--stream` under `-n`, for comparison.
#[test]
fn stream_errors_reads_a_slurp_the_same_way_6676() -> Result<()> {
    let dir = fixtures()?;
    #[rustfmt::skip]
    let rows: &[Row<'_>] = &[
        (&["-c", "-n", "-s", "--seq", "--stream-errors", "input"][..], &["arr.seq", "missing.json"][..], "\u{1e}[[[0],1],[[0]]]\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream-errors", "[inputs]"][..], &["arr.seq", "missing.json"][..], "\u{1e}[[[[0],1],[[0]]]]\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream-errors", "if false then input else . end"][..], &["arr.seq", "missing.json"][..], "\u{1e}null\n", "", 0),
        (&["-c", "-n", "-s", "--seq", "--stream-errors", "[limit(0; inputs)]"][..], &["two.seq", "missing.json"][..], "\u{1e}[]\n", "", 0),
        (&["-c", "-n", "-s", "--seq", "--stream-errors", "def f: input; ."][..], &["two.seq", "missing.json"][..], "\u{1e}null\n", "", 0),
        (&["-c", "-n", "-s", "--seq", "--stream-errors", "if false then input else . end"][..], &["dir"][..], "\u{1e}null\n", "", 0),
        (&["-c", "-n", "-s", "--seq", "--stream-errors", "if false then input else . end"][..], &["missing.json", "dir"][..], "\u{1e}null\n", "", 0),
        (&["-c", "-n", "-s", "--seq", "--stream-errors", "input"][..], &["missing.json", "two.seq"][..], "\u{1e}[[[],2]]\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream-errors", "[inputs]"][..], &["two.seq", "dir"][..], "\u{1e}[[[[],2]]]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream-errors", "[inputs]"][..], &["bad.seq", "missing.json"][..], "\u{1e}[[[\"Unmatched '}' in the middle of an array at line 1, column 4 (need RS to resync)\",[0]]]]\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-s", "--seq", "--stream-errors", "input"][..], &["arr.seq", "missing.json"][..], "", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: error (at @DIR@/missing.json:0): break\n", 2),
        (&["-c", "-s", "--seq", "--stream-errors", "[inputs]"][..], &["arr.seq", "missing.json"][..], "\u{1e}[]\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-s", "--seq", "--stream-errors", "[.,input]"][..], &["two.seq", "missing.json"][..], "", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: error (at @DIR@/missing.json:0): break\n", 2),
        (&["-c", "-s", "--seq", "--stream-errors", "input"][..], &["two.seq", "dir"][..], "", "jq: error: Is a directory\njq: error (at <unknown>): break\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream", "def f: input; ."][..], &["two.seq", "missing.json"][..], "\u{1e}null\n", "", 0),
        (&["-c", "-n", "-s", "--seq", "--stream", "if false then input else . end"][..], &["two.seq", "missing.json"][..], "\u{1e}null\n", "", 0),
    ];
    check_rows(&dir.path().to_string_lossy(), rows)
}
