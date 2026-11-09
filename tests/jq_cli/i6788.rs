//! #6788: under `--seq -s --stream` (and `-n -s`), a program that reads `input` raises the fault a
//! `--seq` record left open ends in. When the last file did not open, jq has the program standing at
//! that file, line 0, when it takes the fault: the error says `(at missing:0)` and `input_filename`
//! afterwards is `"missing"`. succinctly gave the fault no position (`(at <unknown>)`, `null`). A last
//! directory is the read jq itself leaves `<unknown>` at, and a plain `.` says the warning with no
//! position, so those rows were right and still are. Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

/// One row: the flags and program, the files under the fixture directory (the ones not created do
/// not exist), and jq 1.7.1's stdout, stderr and exit status, with `@DIR@` for the directory.
type Row<'a> = (&'a [&'a str], &'a [&'a str], &'a str, &'a str, i32);

#[test]
fn the_fault_a_program_takes_is_at_the_last_file_that_did_not_open_6788() -> Result<()> {
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
    let rows: [Row<'_>; 12] = [
        (&["-c", "--seq", "-s", "--stream", "input"][..], &["trunc", "pa", "missing"][..], "", "jq: error: Could not open file @DIR@/missing: No such file or directory\njq: error (at @DIR@/missing:0): Unfinished JSON term at EOF at line 1, column 7\n", 2),
        (&["-c", "--seq", "-s", "--stream", "[., input]"][..], &["trunc", "pa", "missing"][..], "", "jq: error: Could not open file @DIR@/missing: No such file or directory\njq: error (at @DIR@/missing:0): Unfinished JSON term at EOF at line 1, column 7\n", 2),
        (&["-c", "--seq", "-s", "--stream", "[inputs]"][..], &["trunc", "pa", "missing"][..], "", "jq: error: Could not open file @DIR@/missing: No such file or directory\njq: error (at @DIR@/missing:0): Unfinished JSON term at EOF at line 1, column 7\n", 2),
        (&["-c", "--seq", "-s", "--stream", "input"][..], &["trunc2", "pa", "missing2"][..], "", "jq: error: Could not open file @DIR@/missing2: No such file or directory\njq: error (at @DIR@/missing2:0): Unfinished JSON term at EOF at line 1, column 9\n", 2),
        (&["-c", "--seq", "-n", "-s", "--stream", "[inputs]"][..], &["trunc", "pa", "missing"][..], "", "jq: error: Could not open file @DIR@/missing: No such file or directory\njq: error (at @DIR@/missing:0): Unfinished JSON term at EOF at line 1, column 7\n", 2),
        (&["-c", "--seq", "-n", "--stream", "[inputs]"][..], &["trunc", "pa", "missing"][..], "", "jq: error: Could not open file @DIR@/missing: No such file or directory\njq: error (at @DIR@/missing:0): Unfinished JSON term at EOF at line 1, column 7\n", 2),
        (&["-c", "--seq", "-s", "--stream", "[input_filename, input_line_number, (try input catch .), input_filename, input_line_number]"][..], &["trunc", "pa", "missing"][..], "\u{1e}[\"@DIR@/missing\",0,\"Unfinished JSON term at EOF at line 1, column 7\",\"@DIR@/missing\",0]\n", "jq: error: Could not open file @DIR@/missing: No such file or directory\n", 2),
        (&["-c", "--seq", "-s", "--stream", "."][..], &["trunc", "pa", "missing"][..], "\u{1e}[[[0],1],[[1,0],1],[[1,0]]]\n", "jq: error: Could not open file @DIR@/missing: No such file or directory\n", 2),
        (&["-c", "--seq", "-s", "--stream", "input"][..], &["trunc", "pa", "dir"][..], "", "jq: error: Is a directory\njq: error (at <unknown>): Unfinished JSON term at EOF at line 1, column 7\n", 2),
        (&["-c", "--seq", "-s", "--stream", "input"][..], &["trunc", "pa"][..], "\u{1e}[[1,0]]\n", "jq: ignoring parse error: Unfinished JSON term at EOF at line 1, column 7\n", 0),
        (&["-c", "--seq", "-s", "--stream-errors", "input"][..], &["trunc", "pa", "missing"][..], "\u{1e}[\"Unfinished JSON term at EOF at line 1, column 7\",[1]]\n", "jq: error: Could not open file @DIR@/missing: No such file or directory\n", 2),
        (&["-c", "--seq", "-n", "-s", "--stream", "[inputs]"][..], &["trunc", "pa", "dir"][..], "", "jq: error: Is a directory\njq: error (at <unknown>): Unfinished JSON term at EOF at line 1, column 7\n", 2),
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
