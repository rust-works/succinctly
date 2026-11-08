//! #6643: a `--seq` stream that faults in the middle and again at its end, with a directory given
//! last. jq reads the directory only with a read past the last file. A fault in the middle stops a
//! program that reads the stream before it makes one (exit 5), a program that stops after its first
//! value never makes it (exit 0), and the read of a fault at the end of the text reaches the directory
//! (exit 2). #6610 told the two apart for a stream with faults of one kind; the hook now places each
//! fault by its own: a stream holding both, a last value that ends with its file and no newline (the
//! reader's `Potentially truncated top-level numeric value at EOF`), and several faults in the
//! middle. Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn each_fault_of_a_seq_stream_is_placed_by_its_own_read_beside_a_last_directory_6643() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    for (name, text) in [
        ("f", "\u{1e}1\n\u{1e}{\"a\n\u{1e}5\n"),
        ("nonl", "\u{1e}1\n\u{1e}2"),
        ("trunc", "\u{1e}[1,"),
        ("c", "\u{1e}1\n\u{1e}2\n"),
        ("mideof", "\u{1e}1\n\u{1e}{\"a\n\u{1e}5\n\u{1e}[1,"),
        ("twomid", "\u{1e}1\n\u{1e}{\"a\n\u{1e}5\n\u{1e}}\n\u{1e}6\n"),
    ] {
        std::fs::write(dir.path().join(format!("{name}.seq")), text)?;
    }
    std::fs::create_dir(dir.path().join("dd"))?;
    let path = |name: &str| match name {
        "dd" => format!("{d}/dd"),
        _ => format!("{d}/{name}.seq"),
    };
    for (flags, program, names, stdout, stderr, status) in [
        (&["-n"][..], "[inputs]", &["f", "nonl", "dd"][..], "", "jq: error (at @DIR@/f.seq:3): Truncated value at line 3, column 1\n", 5),
        (&["-n"][..], "input", &["f", "nonl", "dd"][..], "\u{1e}1\n", "", 0),
        (&["-n"][..], "[inputs]", &["f", "trunc", "dd"][..], "", "jq: error (at @DIR@/f.seq:3): Truncated value at line 3, column 1\n", 5),
        (&["-n"][..], "input", &["f", "trunc", "dd"][..], "\u{1e}1\n", "", 0),
        (&["-n"][..], "[inputs]", &["mideof", "dd"][..], "", "jq: error (at @DIR@/mideof.seq:3): Truncated value at line 3, column 1\n", 5),
        (&["-n"][..], "input", &["mideof", "dd"][..], "\u{1e}1\n", "", 0),
        (&["-n"][..], "first(inputs)", &["twomid", "dd"][..], "\u{1e}1\n", "", 0),
        (&["-n"][..], "[inputs]", &["twomid", "dd"][..], "", "jq: error (at @DIR@/twomid.seq:3): Truncated value at line 3, column 1\n", 5),
        (&[][..], "input", &["f", "nonl", "dd"][..], "\u{1e}1\n", "jq: error (at @DIR@/f.seq:3): Truncated value at line 3, column 1\njq: ignoring parse error: Potentially truncated top-level numeric value at EOF at line 5, column 2\n", 2),
        (&[][..], "[inputs]", &["f", "trunc", "dd"][..], "", "jq: error (at @DIR@/f.seq:3): Truncated value at line 3, column 1\njq: error (at @DIR@/dd:0): Unfinished JSON term at EOF at line 4, column 4\n", 2),
        (&["-n"][..], "[inputs]", &["c", "nonl", "dd"][..], "", "jq: error (at @DIR@/dd:0): Potentially truncated top-level numeric value at EOF at line 4, column 2\n", 2),
        (&["-n"][..], "input", &["c", "nonl", "dd"][..], "\u{1e}1\n", "", 0),
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
