//! #6661: a directory given last is placed by the program's reads under `--seq` with `--stream` or
//! `--stream-errors` too, as it is on the other routes (#6615, #6624). The stream reader's `--seq`
//! text that faults at its end (`Unfinished abandoned text at EOF`) sends jq's reader into the
//! directory: the read that returns the fault counts the failure, which makes the run exit 2, and
//! `jq: error: Is a directory` comes from the program's next read. A program that stops at the
//! fault never makes that read. Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_directory_last_is_reported_by_the_next_read_under_seq_with_a_stream_flag_6661() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::create_dir(dir.path().join("dir"))?;
    for (name, text) in [
        ("bare.json", "1"),
        ("one.json", "1\n"),
        ("s.seq", "\u{1e}1\n\u{1e}2"),
        ("s2.seq", "\u{1e}1\n\u{1e}2\n"),
        ("ok.seq", "\u{1e}[1]\n"),
        // A fault only the plain `--seq` walk sees (`12` at a record's end is the stream reader's
        // `[[],12]`) ahead of one at the end: the per-fault flags the hook pops stay in step.
        ("mix.seq", "\u{1e}12\n\u{1e}3\u{1e}["),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    #[rustfmt::skip]
    let rows = [
        (&["-c", "-n", "--seq", "--stream-errors", "[range(3)|try input catch .]"][..], &["dir"][..], "\u{1e}[[\"Unfinished abandoned text at EOF at line 1, column 0\",[]],\"break\",\"break\"]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "--seq", "--stream-errors", "[range(1)|try input catch .]"][..], &["dir"][..], "\u{1e}[[\"Unfinished abandoned text at EOF at line 1, column 0\",[]]]\n", "", 2),
        (&["-c", "-n", "--seq", "--stream", "[range(3)|try input catch .]"][..], &["dir"][..], "\u{1e}[\"Unfinished abandoned text at EOF at line 1, column 0\",\"break\",\"break\"]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "--seq", "--stream", "[range(3)|try input catch .]"][..], &["one.json", "dir"][..], "\u{1e}[\"Unfinished abandoned text at EOF at line 2, column 0\",\"break\",\"break\"]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "--seq", "--stream-errors", "[inputs]"][..], &["bare.json", "dir"][..], "\u{1e}[[\"Unfinished abandoned text at EOF at line 1, column 1\",[]]]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "--seq", "--stream-errors", "input"][..], &["bare.json", "dir"][..], "", "jq: error: Is a directory\njq: error (at <unknown>): break\n", 2),
        (&["-c", "--seq", "--stream", "input"][..], &["one.json", "dir"][..], "", "jq: ignoring parse error: Unfinished abandoned text at EOF at line 2, column 0\n", 2),
        (&["-c", "--seq", "--stream", "[inputs]"][..], &["s.seq", "dir"][..], "\u{1e}[[[],2]]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "--seq", "--stream-errors", "[inputs]"][..], &["s.seq", "dir"][..], "\u{1e}[[[],2]]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "--seq", "--stream", "[inputs],(try input catch .)"][..], &["s2.seq", "dir"][..], "\u{1e}[[[],1],[[],2]]\n\u{1e}\"break\"\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "--seq", "--stream-errors", "[inputs]"][..], &["bare.json", "missing.json", "dir"][..], "\u{1e}[[\"Unfinished abandoned text at EOF at line 1, column 1\",[]]]\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: error: Is a directory\n", 2),
        (&["-c", "-n", "--seq", "--stream-errors", "[range(3)|try input catch .]"][..], &["ok.seq", "dir"][..], "\u{1e}[[[0],1],[[0]],\"break\"]\n", "", 2),
        (&["-c", "--seq", "--stream", "."][..], &["ok.seq", "dir"][..], "\u{1e}[[0],1]\n\u{1e}[[0]]\n", "", 2),
        (&["-c", "-n", "--seq", "--stream-errors", "[range(4)|try input catch .]"][..], &["mix.seq", "dir"][..], "\u{1e}[[[],12],[[],3],[\"Unfinished JSON term at EOF at line 2, column 4\",[0]],\"break\"]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "--seq", "--stream", "[range(4)|try input catch .]"][..], &["mix.seq", "dir"][..], "\u{1e}[[[],12],[[],3],\"Unfinished JSON term at EOF at line 2, column 4\",\"break\"]\n", "jq: error: Is a directory\n", 2),
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
