//! #6624: `--stream` and `--stream-errors` read their events a chunk at a time, and a directory
//! given last is placed by the program's reads there as it is on the JSON and `-R` routes (#6615):
//! the read that reaches it makes the run exit 2, and `jq: error: Is a directory` comes from the
//! program's next read after that. A last value that completes only at the end of its file (a bare
//! number) reaches it, and so does a read that finds the stream at its end; jq's main loop reads no
//! more once a read has failed, so a plain `--stream .` never says it. Every row was captured from
//! jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_directory_last_is_reported_by_the_read_after_the_one_that_reaches_it_in_a_stream_6624(
) -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::create_dir(dir.path().join("dir"))?;
    for (name, text) in [
        ("bare.json", "1"),
        ("one.json", "1\n"),
        ("two.json", "1\n2"),
        ("trunc.json", "[1,"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    #[rustfmt::skip]
    let rows = [
        (&["-c", "--stream", "input"][..], &["bare", "dir"][..], "", "jq: error: Is a directory\njq: error (at <unknown>): break\n", 2),
        (&["-c", "--stream", "."][..], &["bare", "dir"][..], "[[],1]\n", "", 2),
        (&["-c", "--stream", "input"][..], &["one", "dir"][..], "", "jq: error (at @DIR@/dir:0): break\n", 2),
        (&["-c", "--stream", "input"][..], &["two", "dir"][..], "[[],2]\n", "", 2),
        (&["-c", "--stream", "input"][..], &["trunc", "dir"][..], "", "jq: error (at @DIR@/dir:0): Unfinished JSON term at EOF at line 1, column 3\n", 2),
        (&["-c", "--stream", "[inputs]"][..], &["bare", "dir"][..], "[]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "--stream", "[inputs]"][..], &["one", "dir"][..], "[]\n", "", 2),
        (&["-c", "--stream", "[inputs],(try input catch .)"][..], &["one", "dir"][..], "[]\n\"break\"\n", "jq: error: Is a directory\n", 2),
        (&["-c", "--stream", "first(inputs)"][..], &["bare", "dir"][..], "", "jq: error: Is a directory\n", 2),
        (&["-c", "--stream", "(try input catch .),(try input catch .),(try input catch .)"][..], &["bare", "dir"][..], "\"break\"\n\"break\"\n\"break\"\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "--stream", "[inputs]"][..], &["bare", "dir"][..], "[[[],1]]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "--stream", "[inputs]"][..], &["one", "dir"][..], "[[[],1]]\n", "", 2),
        (&["-c", "-n", "--stream", "[inputs],(try input catch .)"][..], &["one", "dir"][..], "[[[],1]]\n\"break\"\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "--stream", "(try input catch .),(try input catch .),(try input catch .)"][..], &["dir"][..], "\"break\"\n\"break\"\n\"break\"\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "--stream", "input"][..], &["dir"][..], "", "jq: error (at @DIR@/dir:0): break\n", 2),
        (&["-c", "-n", "--stream", "(try input catch .),(try input catch .),(try input catch .)"][..], &["trunc", "dir"][..], "[[0],1]\n\"Unfinished JSON term at EOF at line 1, column 3\"\n\"break\"\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "--stream-errors", "[inputs]"][..], &["bare", "dir"][..], "[[[],1]]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "--stream-errors", "[inputs]"][..], &["one", "dir"][..], "[[[],1]]\n", "", 2),
        (&["-c", "-n", "--stream-errors", "[inputs]"][..], &["trunc", "dir"][..], "[[[0],1],[\"Unfinished JSON term at EOF at line 1, column 3\",[1]]]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "--stream-errors", "[inputs],(try input catch .)"][..], &["trunc", "dir"][..], "[[[0],1],[\"Unfinished JSON term at EOF at line 1, column 3\",[1]]]\n\"break\"\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "--stream", "[inputs]"][..], &["bare", "dir", "dir"][..], "[[[],1]]\n", "jq: error: Is a directory\njq: error: Is a directory\n", 2),
        (&["-c", "--stream", "input"][..], &["bare", "dir", "dir"][..], "", "jq: error: Is a directory\njq: error: Is a directory\njq: error (at <unknown>): break\n", 2),
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
