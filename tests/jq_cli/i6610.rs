//! #6610: a `--seq` stream that faults in the middle, with a directory given last. jq reads the
//! directory only with a read past the last file, and a fault in the middle stops a program that
//! reads the stream before it makes one: the run's status is that fault's (5), a missing file before
//! the directory is reported where the reads reach it, and a program that stops after its first
//! value never sees the directory (exit 0). A fault at the end of the text is raised by the read
//! that reaches the directory, so there the directory still fails the run (exit 2): those rows pin
//! what the deferral must not touch. Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_directory_last_is_left_to_the_reads_beside_a_seq_stream_that_faults_6610() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    for (name, text) in [
        ("f", "\u{1e}1\n\u{1e}{\"a\n\u{1e}5\n"),
        ("trunc", "\u{1e}[1,"),
        ("empty", ""),
        ("bad1", "\u{1e}{\"a\n"),
        ("c", "\u{1e}1\n\u{1e}2\n"),
    ] {
        std::fs::write(dir.path().join(format!("{name}.seq")), text)?;
    }
    std::fs::create_dir(dir.path().join("dd"))?;
    let path = |name: &str| match name {
        "dd" | "gone" => format!("{d}/{name}"),
        _ => format!("{d}/{name}.seq"),
    };
    for (flags, program, names, stdout, stderr, status) in [
        (&["-n"][..], "[inputs]", &["f", "dd"][..], "", "jq: error (at @DIR@/f.seq:3): Truncated value at line 3, column 1\n", 5),
        (&["-n"][..], "[inputs]", &["f", "gone", "dd"][..], "", "jq: error (at @DIR@/f.seq:3): Truncated value at line 3, column 1\n", 5),
        (&["-n"][..], "input", &["f", "dd"][..], "\u{1e}1\n", "", 0),
        (&["-n"][..], "first(inputs)", &["f", "dd"][..], "\u{1e}1\n", "", 0),
        (&[][..], ".", &["f", "gone", "dd"][..], "\u{1e}1\n\u{1e}5\n", "jq: ignoring parse error: Truncated value at line 3, column 1\njq: error: Could not open file @DIR@/gone: No such file or directory\n", 2),
        (&[][..], "input", &["f", "gone", "dd"][..], "", "jq: error (at @DIR@/f.seq:3): Truncated value at line 3, column 1\njq: error: Could not open file @DIR@/gone: No such file or directory\njq: error (at @DIR@/dd:0): break\n", 2),
        (&[][..], "[inputs]", &["f", "gone", "dd"][..], "\u{1e}[]\n", "jq: error (at @DIR@/f.seq:3): Truncated value at line 3, column 1\njq: error: Could not open file @DIR@/gone: No such file or directory\n", 2),
        // A fault at the end of the text is raised by the read that reaches the directory.
        (&["-n"][..], "[inputs]", &["trunc", "dd"][..], "", "jq: error (at @DIR@/dd:0): Unfinished JSON term at EOF at line 1, column 4\n", 2),
        (&["-n"][..], "[inputs]", &["empty", "dd"][..], "", "jq: error (at @DIR@/dd:0): Unfinished abandoned text at EOF at line 1, column 0\n", 2),
        (&["-n"][..], "[inputs]", &["bad1", "dd"][..], "", "jq: error (at @DIR@/dd:0): Unfinished string at EOF at line 2, column 0\n", 2),
        (&["-n"][..], "input", &["c", "dd"][..], "\u{1e}1\n", "", 0),
    ] {
        let mut args = vec!["--seq", "-c"];
        args.extend_from_slice(flags);
        args.push(program);
        let files: Vec<String> = names.iter().map(|name| path(name)).collect();
        args.extend(files.iter().map(String::as_str));
        let (out, err, code) = run_jq_full(&args, None)?;
        let expected = stderr.replace("@DIR@", &d);
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (stdout, expected.as_str(), status),
            "{flags:?} {program} {names:?}"
        );
    }
    Ok(())
}
