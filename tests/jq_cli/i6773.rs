//! #6773: `--seq -s --stream-errors` reads a record left open (`\x1e[1,`) that runs on into a file, and the
//! fault it ends in is an event of the stream, so jq puts it inside the slurped array when two reads
//! follow the last file that has bytes (a failed file, an empty file or a directory behind another
//! failed file). succinctly kept a failed file in the middle out of the reader's sources under `--seq`,
//! so the fault was met one read late and dropped from the array: `trunc pa missing missing2` printed
//! the array without `["Unfinished JSON term at EOF ...",[1]]`. One failed file after the last file
//! with bytes (`trunc pa missing`, `trunc pa dir`) was already right and still is, as are
//! `--seq -s --stream` (whose fault is a warning), `-s --stream-errors`, and the `-n` and
//! non-slurp readers, which are rows here because they build the same tail source.
//! Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

/// One row: the flags and program, the files under the fixture directory (the ones not created do
/// not exist), and jq 1.7.1's stdout, stderr and exit status, with `@DIR@` for the directory.
type Row<'a> = (&'a [&'a str], &'a [&'a str], &'a str, &'a str, i32);

#[test]
fn a_fault_met_two_reads_behind_the_last_file_is_in_the_array_6773() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::create_dir(dir.path().join("dir"))?;
    for (name, bytes) in [
        ("trunc", &b"\x1e[1,"[..]),
        ("trunc2", b"\x1e{\"a\":"),
        ("pa", b"[1]"),
        ("empty", b""),
    ] {
        std::fs::write(dir.path().join(name), bytes)?;
    }
    #[rustfmt::skip]
    let rows: [Row<'_>; 21] = [
        (&["-c", "--seq", "-s", "--stream-errors", "."][..], &["trunc", "pa", "missing", "missing2"][..], "\u{1e}[[[0],1],[[1,0],1],[[1,0]],[\"Unfinished JSON term at EOF at line 1, column 7\",[1]]]\n", "jq: error: Could not open file @DIR@/missing: No such file or directory\njq: error: Could not open file @DIR@/missing2: No such file or directory\n", 2),
        (&["-c", "--seq", "-s", "--stream-errors", "."][..], &["trunc", "pa", "missing", "empty"][..], "\u{1e}[[[0],1],[[1,0],1],[[1,0]],[\"Unfinished JSON term at EOF at line 1, column 7\",[1]]]\n", "jq: error: Could not open file @DIR@/missing: No such file or directory\n", 2),
        (&["-c", "--seq", "-s", "--stream-errors", "."][..], &["trunc", "pa", "missing", "dir"][..], "\u{1e}[[[0],1],[[1,0],1],[[1,0]],[\"Unfinished JSON term at EOF at line 1, column 7\",[1]]]\n", "jq: error: Could not open file @DIR@/missing: No such file or directory\n", 2),
        (&["-c", "--seq", "-s", "--stream-errors", "."][..], &["trunc2", "pa", "missing", "missing2"][..], "\u{1e}[[[\"a\",0],1],[[\"a\",0]],[\"Unfinished JSON term at EOF at line 1, column 9\",[\"a\"]]]\n", "jq: error: Could not open file @DIR@/missing: No such file or directory\njq: error: Could not open file @DIR@/missing2: No such file or directory\n", 2),
        (&["-c", "--seq", "-s", "--stream-errors", "."][..], &["trunc", "pa", "missing2", "missing"][..], "\u{1e}[[[0],1],[[1,0],1],[[1,0]],[\"Unfinished JSON term at EOF at line 1, column 7\",[1]]]\n", "jq: error: Could not open file @DIR@/missing2: No such file or directory\njq: error: Could not open file @DIR@/missing: No such file or directory\n", 2),
        (&["-c", "--seq", "-s", "--stream-errors", "input_line_number"][..], &["trunc", "pa", "missing", "missing2"][..], "\u{1e}0\n", "jq: error: Could not open file @DIR@/missing: No such file or directory\njq: error: Could not open file @DIR@/missing2: No such file or directory\n", 2),
        (&["-c", "--seq", "-s", "--stream-errors", "."][..], &["trunc", "pa", "missing"][..], "\u{1e}[[[0],1],[[1,0],1],[[1,0]]]\n", "jq: error: Could not open file @DIR@/missing: No such file or directory\n", 2),
        (&["-c", "--seq", "-s", "--stream-errors", "."][..], &["trunc", "pa", "dir"][..], "\u{1e}[[[0],1],[[1,0],1],[[1,0]]]\n", "", 2),
        (&["-c", "--seq", "-s", "--stream-errors", "."][..], &["trunc", "pa"][..], "\u{1e}[[[0],1],[[1,0],1]]\n\u{1e}[[1,0]]\n\u{1e}[\"Unfinished JSON term at EOF at line 1, column 7\",[1]]\n", "", 0),
        (&["-c", "--seq", "-s", "--stream-errors", "."][..], &["trunc", "missing", "pa", "missing2"][..], "\u{1e}[[[0],1],[[1,0],1],[[1,0]]]\n", "jq: error: Could not open file @DIR@/missing: No such file or directory\njq: error: Could not open file @DIR@/missing2: No such file or directory\n", 2),
        (&["-c", "--seq", "-s", "--stream-errors", "."][..], &["pa", "missing", "missing2"][..], "\u{1e}[[\"Unfinished abandoned text at EOF at line 1, column 3\",[]]]\n", "jq: error: Could not open file @DIR@/missing: No such file or directory\njq: error: Could not open file @DIR@/missing2: No such file or directory\n", 2),
        (&["-c", "--seq", "-s", "--stream", "."][..], &["trunc", "pa", "missing", "missing2"][..], "", "jq: error: Could not open file @DIR@/missing: No such file or directory\njq: error: Could not open file @DIR@/missing2: No such file or directory\njq: ignoring parse error: Unfinished JSON term at EOF at line 1, column 7\n", 2),
        (&["-c", "--seq", "-s", "--stream", "."][..], &["trunc", "pa", "missing", "dir"][..], "", "jq: error: Could not open file @DIR@/missing: No such file or directory\njq: ignoring parse error: Unfinished JSON term at EOF at line 1, column 7\n", 2),
        (&["-c", "-s", "--stream-errors", "."][..], &["trunc", "pa", "missing", "missing2"][..], "[[\"Invalid numeric literal at line 1, column 2\",[]],[[0],1],[[0]]]\n", "jq: error: Could not open file @DIR@/missing: No such file or directory\njq: error: Could not open file @DIR@/missing2: No such file or directory\n", 2),
        (&["-c", "--seq", "-s", "--stream-errors", "."][..], &["trunc", "missing", "pa"][..], "\u{1e}[[[0],1],[[1,0],1]]\n", "jq: error: Could not open file @DIR@/missing: No such file or directory\n", 2),
        (&["-c", "--seq", "-s", "--stream-errors", "."][..], &["missing", "trunc", "pa", "missing2"][..], "\u{1e}[[[0],1],[[1,0],1],[[1,0]]]\n", "jq: error: Could not open file @DIR@/missing: No such file or directory\njq: error: Could not open file @DIR@/missing2: No such file or directory\n", 2),
        (&["-c", "--seq", "-s", "-n", "--stream-errors", "[., input]"][..], &["trunc", "pa", "missing", "missing2"][..], "\u{1e}[null,[[[0],1],[[1,0],1],[[1,0]],[\"Unfinished JSON term at EOF at line 1, column 7\",[1]]]]\n", "jq: error: Could not open file @DIR@/missing: No such file or directory\njq: error: Could not open file @DIR@/missing2: No such file or directory\n", 2),
        (&["-c", "--seq", "-s", "-n", "--stream-errors", "input"][..], &["trunc", "pa", "missing"][..], "\u{1e}[[[0],1],[[1,0],1],[[1,0]]]\n", "jq: error: Could not open file @DIR@/missing: No such file or directory\n", 2),
        (&["-c", "--seq", "-n", "--stream-errors", "[inputs]"][..], &["trunc", "pa", "missing", "missing2"][..], "\u{1e}[[[0],1],[[1,0],1],[[1,0]],[\"Unfinished JSON term at EOF at line 1, column 7\",[1]]]\n", "jq: error: Could not open file @DIR@/missing: No such file or directory\njq: error: Could not open file @DIR@/missing2: No such file or directory\n", 2),
        (&["-c", "--seq", "--stream", "[., input]"][..], &["trunc", "pa", "missing"][..], "\u{1e}[[[0],1],[[1,0],1]]\n", "jq: error: Could not open file @DIR@/missing: No such file or directory\njq: error (at @DIR@/missing:0): Unfinished JSON term at EOF at line 1, column 7\n", 2),
        (&["-c", "--seq", "--stream-errors", "[., input]"][..], &["trunc", "pa", "missing", "missing2"][..], "\u{1e}[[[0],1],[[1,0],1]]\n\u{1e}[[[1,0]],[\"Unfinished JSON term at EOF at line 1, column 7\",[1]]]\n", "jq: error: Could not open file @DIR@/missing: No such file or directory\njq: error: Could not open file @DIR@/missing2: No such file or directory\n", 2),
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
