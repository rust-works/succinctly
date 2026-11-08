//! #6615: without `-s`, a directory that is the last input file is read by the read that reaches
//! it, which makes the run exit 2, and `jq: error: Is a directory` comes from the program's next
//! read after that. A last value that completes only at the end of its file (a bare number, or a
//! `-R` line with no newline) reaches it, and so does a read that finds the stream at its end. A
//! `-n` program that stops before either never reaches it and exits 0. Every row was captured from
//! jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_directory_last_is_reported_by_the_read_after_the_one_that_reaches_it_6615() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::create_dir(dir.path().join("dir"))?;
    for (name, text) in [
        ("one.json", "1\n"),
        ("bare.json", "1"),
        ("str.json", "\"a\" 12"),
        ("empty.json", ""),
        ("lines.json", "a\nb"),
        ("linesn.json", "a\nb\n"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    #[rustfmt::skip]
    let rows = [
        (&["-c", "input"][..], &["bare", "dir"][..], "", "jq: error: Is a directory\njq: error (at <unknown>): break\n", 2),
        (&["-c", "[., input]"][..], &["bare", "dir"][..], "", "jq: error: Is a directory\njq: error (at <unknown>): break\n", 2),
        (&["-c", "first(inputs)"][..], &["bare", "dir"][..], "", "jq: error: Is a directory\n", 2),
        (&["-c", "[., (try input catch .), (try input catch .)]"][..], &["one", "dir"][..], "[1,\"break\",\"break\"]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "input"][..], &["bare", "dir"][..], "1\n", "", 2),
        (&["-c", "-n", "input"][..], &["one", "dir"][..], "1\n", "", 0),
        (&["-c", "-n", "input"][..], &["str", "dir"][..], "\"a\"\n", "", 0),
        (&["-c", "-n", "input"][..], &["one", "bare", "dir"][..], "1\n", "", 0),
        (&["-c", "-n", "input"][..], &["bare", "empty", "dir"][..], "1\n", "", 2),
        (&["-c", "-n", "input | input_filename"][..], &["bare", "dir"][..], "\"@DIR@/dir\"\n", "", 2),
        (&["-c", "-n", "[inputs]"][..], &["bare", "dir"][..], "[1]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "[inputs]"][..], &["one", "dir"][..], "[1]\n", "", 2),
        (&["-c", "-n", "[inputs], (try input catch .)"][..], &["one", "dir"][..], "[1]\n\"break\"\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "(try input catch .), (try input catch .)"][..], &["empty", "dir"][..], "\"break\"\n\"break\"\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-R", "input"][..], &["bare", "dir"][..], "\"1\"\n", "", 2),
        (&["-c", "-n", "-R", "input"][..], &["linesn", "dir"][..], "\"a\"\n", "", 0),
        (&["-c", "-n", "-R", "[inputs]"][..], &["linesn", "dir"][..], "[\"a\",\"b\"]\n", "", 2),
        (&["-c", "-R", "input"][..], &["lines", "dir"][..], "\"b\"\n", "", 2),
        (&["-c", "-R", "[., input]"][..], &["bare", "dir"][..], "", "jq: error: Is a directory\njq: error (at <unknown>): break\n", 2),
        (&["-c", "-R", "."][..], &["linesn", "dir"][..], "\"a\"\n\"b\"\n", "", 2),
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
