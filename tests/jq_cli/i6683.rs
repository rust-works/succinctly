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
        // A good file behind a failure keeps the up-front report: its bare number reads on into the
        // next failure, which the loop that stops at that value would otherwise leave unsaid.
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
        // The value completes on its own: the program stops before the reader moves on.
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
