//! #6755: `--stream-errors` over files that all open, with a program that reads no `input`, ran
//! `read_inputs`' placement of open failures with an empty list of them and indexed its first
//! entry, so the flag panicked (exit 101) on any file. Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn stream_errors_over_files_that_all_open_does_not_panic_6755() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::write(dir.path().join("small"), "[1]")?;
    std::fs::write(dir.path().join("smalln"), "[1]\n")?;
    std::fs::write(dir.path().join("empty"), "")?;
    std::fs::write(dir.path().join("obj"), r#"{"a":[1,2]}"#)?;
    std::fs::write(dir.path().join("p"), "1")?;
    #[rustfmt::skip]
    let rows = [
        (&["-c", "--stream-errors", "."][..], &["small"][..], "[[0],1]\n[[0]]\n", "", 0),
        (&["-c", "--stream-errors", "."][..], &["smalln"][..], "[[0],1]\n[[0]]\n", "", 0),
        (&["-c", "--stream-errors", "."][..], &["empty"][..], "", "", 0),
        (&["-c", "--stream-errors", "."][..], &["small", "small"][..], "[[0],1]\n[[0]]\n[[0],1]\n[[0]]\n", "", 0),
        (&["-c", "--stream-errors", "."][..], &["empty", "small"][..], "[[0],1]\n[[0]]\n", "", 0),
        (&["-c", "--stream-errors", "."][..], &["obj"][..], "[[\"a\",0],1]\n[[\"a\",1],2]\n[[\"a\",1]]\n[[\"a\"]]\n", "", 0),
        // A bare number ends with its file, and the next file's events follow it.
        (&["-c", "--stream-errors", "."][..], &["p"][..], "[[],1]\n", "", 0),
        (&["-c", "--stream-errors", "."][..], &["p", "smalln"][..], "[[],1]\n[[0],1]\n[[0]]\n", "", 0),
        (&["-c", "--stream-errors", "[., input_filename]"][..], &["small"][..], "[[[0],1],\"@DIR@/small\"]\n[[[0]],null]\n", "", 0),
        (&["-c", "--stream-errors", "halt_error"][..], &["small"][..], "", "[[0],1]\n", 5),
        (&["-c", "--stream-errors", "-e", "."][..], &["empty"][..], "", "", 4),
        // The runs that read `input` already took a different route.
        (&["-c", "--stream-errors", "[inputs]"][..], &["small"][..], "[[[0]]]\n", "", 0),
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
