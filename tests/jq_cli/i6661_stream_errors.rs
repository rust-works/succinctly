//! #6661: plain `--stream-errors` (without `-n`, or with a program that reads `input`) over two
//! failures at the end of the list says `jq: error: Is a directory` for each directory, and for the
//! one given last when a file that did not open comes before it, as the JSON and `-R` routes do
//! (#6625): the reads that follow place the directory, and the main loop's read never says it.
//! Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_last_directory_behind_another_failure_is_reported_under_stream_errors_6661() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::create_dir(dir.path().join("dir"))?;
    std::fs::create_dir(dir.path().join("dir2"))?;
    std::fs::write(dir.path().join("bare.json"), "1")?;
    #[rustfmt::skip]
    let rows = [
        (&["-c", "--stream-errors", "input"][..], &["bare.json", "dir", "dir2"][..], "", "jq: error: Is a directory\njq: error: Is a directory\njq: error (at <unknown>): break\n", 2),
        (&["-c", "--stream-errors", "input"][..], &["bare.json", "missing.json", "dir"][..], "", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: error: Is a directory\njq: error (at <unknown>): break\n", 2),
        (&["-c", "--stream-errors", "[inputs]"][..], &["bare.json", "missing.json", "dir"][..], "[]\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: error: Is a directory\n", 2),
        (&["-c", "--stream-errors", "[inputs]"][..], &["bare.json", "dir", "dir2"][..], "[]\n", "jq: error: Is a directory\njq: error: Is a directory\n", 2),
        (&["-c", "--stream-errors", "first(inputs)"][..], &["bare.json", "dir", "dir2"][..], "", "jq: error: Is a directory\njq: error: Is a directory\n", 2),
        (&["-c", "--stream-errors", "(try input catch .),(try input catch .),(try input catch .)"][..], &["bare.json", "dir", "dir2"][..], "\"break\"\n\"break\"\n\"break\"\n", "jq: error: Is a directory\njq: error: Is a directory\n", 2),
        (&["-c", "--stream-errors", "[inputs],(try input catch .)"][..], &["bare.json", "missing.json", "dir"][..], "[]\n\"break\"\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: error: Is a directory\n", 2),
        (&["-c", "--stream-errors", "."][..], &["bare.json", "dir", "dir2"][..], "[[],1]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "--stream-errors", "[inputs]"][..], &["bare.json", "missing.json", "dir"][..], "[[[],1]]\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: error: Is a directory\n", 2),
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
