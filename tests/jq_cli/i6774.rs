//! #6774: under `--seq -s --stream`, a record left open (`\x1e[1,`) that runs on into a file and is
//! met by the last read, a directory, is a warning jq never says when the program cannot read it.
//! `input_line_number` reads nothing (it says where the read stands), so it is not handed the
//! unfinished term, and `--seq -s --stream input_line_number trunc pa dir` printed
//! `jq: ignoring parse error: Unfinished JSON term at EOF` where jq prints nothing. A program that
//! pulls `input` is handed it and keeps its behaviour, and `.` and `input_filename` already agreed.
//! Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

/// One row: the flags and program, the files under the fixture directory, and jq 1.7.1's stdout,
/// stderr and exit status, with `@DIR@` for the directory.
type Row<'a> = (&'a [&'a str], &'a [&'a str], &'a str, &'a str, i32);

#[test]
fn input_line_number_is_not_handed_an_unfinished_term_it_cannot_read_6774() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::create_dir(dir.path().join("dir"))?;
    for (name, bytes) in [
        ("trunc", &b"\x1e[1,"[..]),
        ("trunc2", b"\x1e{\"a\":"),
        ("pa", b"[1]"),
    ] {
        std::fs::write(dir.path().join(name), bytes)?;
    }
    #[rustfmt::skip]
    let rows: [Row<'_>; 10] = [
        (&["-c", "--seq", "-s", "--stream", "input_line_number"][..], &["trunc", "pa", "dir"][..], "\u{1e}0\n", "", 2),
        (&["-c", "--seq", "-s", "--stream", "[., input_line_number]"][..], &["trunc", "pa", "dir"][..], "\u{1e}[[[[0],1],[[1,0],1],[[1,0]]],0]\n", "", 2),
        (&["-c", "--seq", "-s", "--stream", "[., input_line_number, input_filename]"][..], &["trunc2", "pa", "dir"][..], "\u{1e}[[[[\"a\",0],1],[[\"a\",0]]],0,\"@DIR@/dir\"]\n", "", 2),
        (&["-c", "--seq", "-s", "--stream-errors", "input_line_number"][..], &["trunc", "pa", "dir"][..], "\u{1e}0\n", "", 2),
        (&["-c", "--seq", "-s", "--stream", "."][..], &["trunc", "pa", "dir"][..], "\u{1e}[[[0],1],[[1,0],1],[[1,0]]]\n", "", 2),
        (&["-c", "--seq", "-s", "--stream", "input_filename"][..], &["trunc", "pa", "dir"][..], "\u{1e}\"@DIR@/dir\"\n", "", 2),
        (&["-c", "--seq", "-s", "--stream", "[., input]"][..], &["trunc", "pa", "dir"][..], "", "jq: error: Is a directory\njq: error (at <unknown>): Unfinished JSON term at EOF at line 1, column 7\n", 2),
        (&["-c", "--seq", "-s", "--stream", "input_line_number"][..], &["trunc", "pa"][..], "\u{1e}0\n\u{1e}0\n", "jq: ignoring parse error: Unfinished JSON term at EOF at line 1, column 7\n", 0),
        (&["-c", "--seq", "-s", "--stream", "input_line_number"][..], &["trunc", "dir"][..], "", "jq: ignoring parse error: Unfinished JSON term at EOF at line 1, column 4\n", 2),
        (&["-c", "-s", "--stream", "input_line_number"][..], &["trunc", "pa", "dir"][..], "", "jq: parse error: Invalid numeric literal at line 1, column 2\n", 5),
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
