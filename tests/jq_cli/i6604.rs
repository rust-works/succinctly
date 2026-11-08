//! #6604: under `-s`, a program that reads `input` asks for one more input after the slurped array,
//! and jq's reader reads the directory that is the last input file then, so the read that finds the
//! stream at its end prints `jq: error: Is a directory`. A program that never makes that read
//! (`.`, `input_filename`, `input_line_number`, `-n input`, which gets the array itself) does not,
//! and a run without `-s` does not either (#6472). The exit status is 2 in every row. Every row was
//! captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_slurps_extra_read_reports_a_directory_last_input_6604() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::create_dir(dir.path().join("dir"))?;
    for (name, text) in [("one.json", "1\n"), ("seq.json", "\u{1e}1\n\u{1e}2\n")] {
        std::fs::write(dir.path().join(name), text)?;
    }
    #[rustfmt::skip]
    let rows = [
        (&["-c", "-s", "input"][..], &["one", "dir"][..], "", "jq: error: Is a directory\njq: error (at <unknown>): break\n", 2),
        (&["-c", "-s", "inputs"][..], &["one", "dir"][..], "", "jq: error: Is a directory\n", 2),
        (&["-c", "-s", "first(inputs)"][..], &["one", "dir"][..], "", "jq: error: Is a directory\n", 2),
        (&["-c", "-s", "[limit(1; inputs)]"][..], &["one", "dir"][..], "[]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-s", "input"][..], &["dir"][..], "", "jq: error: Is a directory\njq: error (at <unknown>): break\n", 2),
        (&["-c", "-s", "-n", "[inputs]"][..], &["one", "dir"][..], "[[1]]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-s", "-n", "input"][..], &["one", "dir"][..], "[1]\n", "", 2),
        (&["-c", "-s", "-e", "input"][..], &["one", "dir"][..], "", "jq: error: Is a directory\njq: error (at <unknown>): break\n", 2),
        (&["-c", "--seq", "-s", "input"][..], &["seq", "dir"][..], "", "jq: error: Is a directory\njq: error (at <unknown>): break\n", 2),
        (&["-c", "-s", "."][..], &["one", "dir"][..], "[1]\n", "", 2),
        (&["-c", "-s", "input_line_number"][..], &["one", "dir"][..], "0\n", "", 2),
        (&["-c", "-s", "input_filename"][..], &["one", "dir"][..], "\"@DIR@/dir\"\n", "", 2),
        (&["-c", "-s", "input"][..], &["one", "dir", "gone"][..], "", "jq: error: Is a directory\njq: error: Could not open file @DIR@/gone.json: No such file or directory\njq: error (at @DIR@/gone.json:0): break\n", 2),
        (&["-c", "-s", "input"][..], &["dir", "one"][..], "", "jq: error: Is a directory\njq: error (at <unknown>): break\n", 2),
        (&["-c", "input"][..], &["one", "dir"][..], "", "jq: error (at @DIR@/dir:0): break\n", 2),
    ];
    for (flags, names, stdout, stderr, status) in rows {
        let files: Vec<String> = names
            .iter()
            .map(|name| {
                if *name == "dir" {
                    format!("{d}/dir")
                } else {
                    format!("{d}/{name}.json")
                }
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
