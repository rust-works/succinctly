//! #6683: plain `--stream-errors` (a program that does not read `input`, so only jq's main loop
//! reads) reports an unopened file, and counts a directory given last, when a read gets there. A
//! program that halts on the first value never reads past a file whose first value is complete, so
//! `halt` and `halt_error` keep their own exit status and nothing is said of the files behind it;
//! a first value that needs the next file to end (a bare number with no newline) reaches the
//! failure on that read, which makes the run exit 2. Every row was captured from jq 1.7.1.
//!
//! The same holds for plain JSON, which reads its files whole: the bare number that ends the last
//! file before a directory given last is read on into the directory, so the failure is counted by
//! the read and a program that halts on that value exits 2, not with its own status.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_halting_program_does_not_reach_the_files_behind_its_first_value_under_stream_errors_6683(
) -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::create_dir(dir.path().join("dir"))?;
    std::fs::create_dir(dir.path().join("dir2"))?;
    std::fs::write(dir.path().join("pn.json"), "1\n")?;
    std::fs::write(dir.path().join("p.json"), "1")?;
    #[rustfmt::skip]
    let rows = [
        (&["-c", "--stream-errors", "halt_error"][..], &["pn.json", "dir"][..], "", "[[],1]\n", 5),
        (&["-c", "--stream-errors", "halt"][..], &["pn.json", "dir"][..], "", "", 0),
        (&["-c", "--stream-errors", "halt_error(3)"][..], &["pn.json", "dir"][..], "", "[[],1]\n", 3),
        (&["-c", "--stream-errors", "halt_error"][..], &["pn.json", "dir", "dir2"][..], "", "[[],1]\n", 5),
        (&["-c", "--stream-errors", "halt_error"][..], &["pn.json", "dir", "missing.json"][..], "", "[[],1]\n", 5),
        (&["-c", "--stream-errors", "halt_error"][..], &["pn.json", "missing.json"][..], "", "[[],1]\n", 5),
        (&["-c", "--stream-errors", "halt_error"][..], &["pn.json", "missing.json", "dir"][..], "", "[[],1]\n", 5),
        (&["-c", "--stream-errors", "."][..], &["pn.json", "dir"][..], "[[],1]\n", "", 2),
        (&["-c", "--stream-errors", "."][..], &["pn.json", "missing.json"][..], "[[],1]\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "--stream-errors", "halt_error"][..], &["p.json", "dir"][..], "", "[[],1]\n", 2),
        (&["-c", "--stream-errors", "halt_error"][..], &["p.json", "missing.json"][..], "", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\n[[],1]\n", 2),
        (&["-c", "--stream-errors", "-e", "halt"][..], &["pn.json", "dir"][..], "", "", 0),
        // A program that goes on reading reaches the good file behind the failure, whose bare number
        // reads on into the next failure: the loop's reads place both, as the up-front report did.
        (&["-c", "--stream-errors", "."][..], &["pn.json", "dir", "p.json", "missing.json"][..], "[[],1]\n[[],1]\n", "jq: error: Is a directory\njq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "--stream-errors", "limit(1;.)"][..], &["pn.json", "missing.json", "p.json", "dir"][..], "[[],1]\n[[],1]\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
    ];
    for (flags, names, stdout, stderr, status) in rows {
        let files: Vec<String> = names.iter().map(|name| format!("{d}/{name}")).collect();
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
fn a_halt_after_a_last_value_read_on_into_a_last_directory_exits_2_6683() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::create_dir(dir.path().join("dir"))?;
    std::fs::create_dir(dir.path().join("dir2"))?;
    std::fs::write(dir.path().join("pn.json"), "1\n")?;
    std::fs::write(dir.path().join("p.json"), "1")?;
    std::fs::write(dir.path().join("p2.json"), "1 2")?;
    std::fs::write(dir.path().join("a.json"), "[1]")?;
    std::fs::write(dir.path().join("e.json"), "")?;
    #[rustfmt::skip]
    let rows = [
        // The number completes only at the end of its file: the read reaches the directory.
        (&["-c", "halt_error"][..], &["p.json", "dir"][..], "", "1\n", 2),
        (&["-c", "halt"][..], &["p.json", "dir"][..], "", "", 2),
        (&["-c", "halt_error(3)"][..], &["p.json", "dir"][..], "", "1\n", 2),
        (&["-e", "-c", "halt_error"][..], &["p.json", "dir"][..], "", "1\n", 2),
        (&["-c", ".,halt"][..], &["p.json", "dir"][..], "1\n", "", 2),
        (&["-c", "halt_error"][..], &["p.json", "missing.json", "dir"][..], "", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\n1\n", 2),
        (&["-c", "halt_error"][..], &["p.json", "dir", "dir2"][..], "", "jq: error: Is a directory\n1\n", 2),
        (&["-c", "halt_error"][..], &["p.json", "e.json", "dir"][..], "", "1\n", 2),
        (&["-s", "-c", "halt_error"][..], &["p.json", "dir"][..], "", "[1]\n", 2),
        (&["-c", "halt_error"][..], &["p.json", "dir", "p2.json"][..], "", "jq: error: Is a directory\n11\n", 2),
        // The value completes on its own: the program stops before the reader moves on.
        (&["-n", "-c", "halt_error"][..], &["p.json", "dir"][..], "", "", 5),
        (&["-c", "halt_error"][..], &["pn.json", "dir"][..], "", "1\n", 5),
        (&["-c", "halt"][..], &["pn.json", "dir"][..], "", "", 0),
        (&["-c", "halt_error"][..], &["p2.json", "dir"][..], "", "1\n", 5),
        (&["-c", "halt_error"][..], &["a.json", "dir"][..], "", "[1]\n", 5),
    ];
    for (flags, names, stdout, stderr, status) in rows {
        let files: Vec<String> = names.iter().map(|name| format!("{d}/{name}")).collect();
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

// The rows below are a second pass at #6683: a halting program under `--stream-errors` never reaches
// the failures behind a first value that is whole in its own file, whatever files follow the first
// failure. Each was captured from jq 1.7.1; `gone.json` does not exist.

/// One row: the flags, the files, then jq's stdout, stderr and exit status.
type Row<'a> = (&'a [&'a str], &'a [&'a str], &'a str, &'a str, i32);

/// Creates the files the rows name (`gone` stays absent) and runs each row, for the rows of a
/// first value that completes in its own file with a file behind the first failure (#6683).
fn check_halt_rows(rows: &[Row<'_>]) -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::create_dir(dir.path().join("dir"))?;
    std::fs::create_dir(dir.path().join("dir2"))?;
    for (name, text) in [
        ("p.json", "1"),
        ("pn.json", "1\n"),
        ("arr.json", "[1,2]"),
        ("s.json", "\"a\"\n"),
        ("empty.json", ""),
        ("r2.json", "1 2"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    for &(flags, names, stdout, stderr, status) in rows {
        let files: Vec<String> = names
            .iter()
            .map(|name| match *name {
                "dir" | "dir2" => format!("{d}/{name}"),
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
fn a_halting_program_never_reaches_the_failures_behind_a_whole_first_value_6683() -> Result<()> {
    #[rustfmt::skip]
    let rows = [
        (&["-c", "--stream-errors", "halt_error"][..], &["pn", "dir", "p", "gone"][..], "", "[[],1]\n", 5),
        (&["-c", "--stream-errors", "halt_error"][..], &["arr", "dir", "p", "gone"][..], "", "[[0],1]\n", 5),
        (&["-c", "--stream-errors", "halt_error"][..], &["pn", "gone", "p", "dir"][..], "", "[[],1]\n", 5),
        (&["-c", "--stream-errors", "halt"][..], &["pn", "dir", "p", "gone"][..], "", "", 0),
        (&["-c", "--stream-errors", "halt"][..], &["arr", "dir", "p", "gone"][..], "", "", 0),
        (&["-c", "--stream-errors", "halt"][..], &["pn", "gone", "p", "dir"][..], "", "", 0),
        (&["-c", "--stream-errors", "halt_error(3)"][..], &["pn", "dir", "p", "gone"][..], "", "[[],1]\n", 3),
        (&["-c", "--stream-errors", "halt_error(3)"][..], &["arr", "dir", "p", "gone"][..], "", "[[0],1]\n", 3),
        (&["-c", "--stream-errors", "halt_error(3)"][..], &["pn", "gone", "p", "dir"][..], "", "[[],1]\n", 3),
        (&["-c", "--stream-errors", ".,halt_error"][..], &["pn", "dir", "p", "gone"][..], "[[],1]\n", "[[],1]\n", 5),
        (&["-c", "--stream-errors", ".,halt_error"][..], &["arr", "dir", "p", "gone"][..], "[[0],1]\n", "[[0],1]\n", 5),
        (&["-c", "--stream-errors", ".,halt_error"][..], &["pn", "gone", "p", "dir"][..], "[[],1]\n", "[[],1]\n", 5),
        (&["-c", "--stream-errors", "halt_error"][..], &["pn", "dir", "arr", "gone"][..], "", "[[],1]\n", 5),
        (&["-c", "--stream-errors", "halt_error"][..], &["pn", "dir", "pn", "gone"][..], "", "[[],1]\n", 5),
        (&["-c", "--stream-errors", "halt_error"][..], &["arr", "gone", "s", "dir"][..], "", "[[0],1]\n", 5),
        (&["-c", "--stream-errors", "-e", "halt_error"][..], &["pn", "dir", "p", "gone"][..], "", "[[],1]\n", 5),
        (&["-c", "--stream-errors", "halt_error"][..], &["arr", "dir", "r2", "gone"][..], "", "[[0],1]\n", 5),
    ];
    check_halt_rows(&rows)
}

/// Rows that matched jq before this change, and must go on matching.
#[test]
fn the_rows_that_reach_the_failures_keep_their_rows_6683() -> Result<()> {
    #[rustfmt::skip]
    let rows = [
        (&["-c", "--stream-errors", "halt_error"][..], &["pn", "dir", "gone"][..], "", "[[],1]\n", 5),
        (&["-c", "--stream-errors", "halt_error"][..], &["pn", "dir", "dir2"][..], "", "[[],1]\n", 5),
        (&["-c", "--stream-errors", "halt_error"][..], &["pn", "dir"][..], "", "[[],1]\n", 5),
        (&["-c", "--stream-errors", "."][..], &["pn", "dir", "p", "gone"][..], "[[],1]\n[[],1]\n", "jq: error: Is a directory\njq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "--stream-errors", "halt_error"][..], &["p", "dir", "p", "gone"][..], "", "jq: error: Is a directory\njq: error: Could not open file @DIR@/gone.json: No such file or directory\n[[],11]\n", 2),
        (&["-c", "--stream-errors", "halt_error"][..], &["p", "dir"][..], "", "[[],1]\n", 2),
        (&["-c", "--stream-errors", "halt_error"][..], &["pn", "dir", "empty", "gone"][..], "", "[[],1]\n", 5),
        (&["-c", "--stream-errors", "[inputs]"][..], &["pn", "dir", "p", "gone"][..], "[[[],1]]\n", "jq: error: Is a directory\njq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
    ];
    check_halt_rows(&rows)
}

/// Programs that stop after more than the first value, and an empty file between the first value
/// and the failure: the reads reach what they reach, and the rows match jq as they did before or
/// now do.
#[test]
fn programs_that_stop_after_a_few_values_reach_what_their_reads_reach_6683() -> Result<()> {
    #[rustfmt::skip]
    let rows = [
        (&["-c", "--stream-errors", ".,halt"][..], &["pn", "dir", "p", "gone"][..], "[[],1]\n", "", 0),
        (&["-c", "--stream-errors", ".,halt"][..], &["pn", "empty", "dir", "p"][..], "[[],1]\n", "", 0),
        (&["-c", "--stream-errors", ".,halt"][..], &["arr", "gone", "pn", "dir"][..], "[[0],1]\n", "", 0),
        (&["-c", "--stream-errors", "-e", ".,halt"][..], &["pn", "dir", "p", "gone"][..], "[[],1]\n", "", 0),
        (&["-c", "--stream-errors", "first(.)"][..], &["pn", "dir", "p", "gone"][..], "[[],1]\n[[],1]\n", "jq: error: Is a directory\njq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "--stream-errors", "first(.)"][..], &["pn", "empty", "dir", "p"][..], "[[],1]\n[[],1]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "--stream-errors", "first(.)"][..], &["arr", "gone", "pn", "dir"][..], "[[0],1]\n[[1],2]\n[[1]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "--stream-errors", "first(.)"][..], &["p", "empty", "dir", "pn"][..], "[[],11]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "--stream-errors", "limit(2;.)"][..], &["pn", "dir", "p", "gone"][..], "[[],1]\n[[],1]\n", "jq: error: Is a directory\njq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "--stream-errors", "limit(2;.)"][..], &["pn", "empty", "dir", "p"][..], "[[],1]\n[[],1]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "--stream-errors", "limit(2;.)"][..], &["arr", "gone", "pn", "dir"][..], "[[0],1]\n[[1],2]\n[[1]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "--stream-errors", "limit(2;.)"][..], &["p", "empty", "dir", "pn"][..], "[[],11]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "--stream-errors", ".,halt"][..], &["p", "empty", "dir", "pn"][..], "[[],11]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "--stream-errors", "first(.,.)"][..], &["pn", "dir", "p", "gone"][..], "[[],1]\n[[],1]\n", "jq: error: Is a directory\njq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "--stream-errors", "first(.,.)"][..], &["pn", "empty", "dir", "p"][..], "[[],1]\n[[],1]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "--stream-errors", "first(.,.)"][..], &["arr", "gone", "pn", "dir"][..], "[[0],1]\n[[1],2]\n[[1]]\n", "jq: error: Could not open file @DIR@/gone.json: No such file or directory\n", 2),
        (&["-c", "--stream-errors", "first(.,.)"][..], &["p", "empty", "dir", "pn"][..], "[[],11]\n", "jq: error: Is a directory\n", 2),
    ];
    check_halt_rows(&rows)
}
