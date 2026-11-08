//! #6683: plain `--stream-errors` (a program that does not read `input`, so only jq's main loop
//! reads) reports an unopened file, and counts a directory given last, when a read gets there. A
//! program that halts on the first value never reads past a file whose first value is complete, so
//! `halt` and `halt_error` keep their own exit status and nothing is said of the files behind it;
//! a first value that needs the next file to end (a bare number with no newline) reaches the
//! failure on that read, which makes the run exit 2. Every row was captured from jq 1.7.1.

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
