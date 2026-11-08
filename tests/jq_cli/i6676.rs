//! #6676: a directory given last under `--seq -s` (and `--seq -s --stream` with `-n`): where the
//! slurp's array and the program's `input` reads stand when the stream has a warning, and whether
//! the directory fails the run. Under `-n` the failures are for the reads that reach them (a fault
//! the program meets stops it first, exit 5), and a warning at the end of a stream not read under
//! `-n` is raised by the read that has been through the directory, so jq's main loop stops there.
//! Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

/// One row: the flags and program, the files, then jq's stdout, stderr and exit status.
type Row<'a> = (&'a [&'a str], &'a [&'a str], &'a str, &'a str, i32);

#[test]
fn a_directory_last_under_seq_slurp_follows_jqs_reads_6676() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::create_dir(dir.path().join("dir"))?;
    std::fs::create_dir(dir.path().join("dir2"))?;
    for (name, text) in [
        ("mix.seq", &b"\x1e{]\n\x1e2"[..]),
        ("arr.seq", b"\x1e[1,2]\n"),
        ("trunc.seq", b"\x1e[1,"),
        ("two.seq", b"\x1e1\n\x1e2\n"),
        ("empty.json", b""),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    #[rustfmt::skip]
    let rows: &[Row<'_>] = &[
        (&["-c", "-n", "-s", "--seq", "input"][..], &["dir"][..], "", "jq: error (at @DIR@/dir:0): Unfinished abandoned text at EOF at line 1, column 0\n", 2),
        (&["-c", "-n", "-s", "--seq", "input"][..], &["dir", "dir2"][..], "", "jq: error: Is a directory\njq: error (at @DIR@/dir2:0): Unfinished abandoned text at EOF at line 1, column 0\n", 2),
        (&["-c", "-n", "-s", "--seq", "input"][..], &["empty", "dir"][..], "", "jq: error (at @DIR@/dir:0): Unfinished abandoned text at EOF at line 1, column 0\n", 2),
        (&["-c", "-n", "-s", "--seq", "input"][..], &["mix", "dir"][..], "", "jq: error (at @DIR@/mix.seq:1): Unmatched ']' at line 1, column 3 (need RS to resync)\n", 5),
        (&["-c", "-n", "-s", "--seq", "input"][..], &["mix", "dir", "dir2"][..], "", "jq: error (at @DIR@/mix.seq:1): Unmatched ']' at line 1, column 3 (need RS to resync)\n", 5),
        (&["-c", "-n", "-s", "--seq", "input"][..], &["mix", "miss", "dir"][..], "", "jq: error (at @DIR@/mix.seq:1): Unmatched ']' at line 1, column 3 (need RS to resync)\n", 5),
        (&["-c", "-n", "-s", "--seq", "input"][..], &["arr", "dir"][..], "\u{1e}[[1,2]]\n", "", 2),
        (&["-c", "-n", "-s", "--seq", "input"][..], &["arr", "miss", "dir"][..], "\u{1e}[[1,2]]\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "input"][..], &["two", "dir"][..], "\u{1e}[1,2]\n", "", 2),
        (&["-c", "-n", "-s", "--seq", "input"][..], &["trunc", "dir"][..], "", "jq: error (at @DIR@/dir:0): Unfinished JSON term at EOF at line 1, column 4\n", 2),
        (&["-c", "-n", "-s", "--seq", "input"][..], &["empty", "mix", "dir"][..], "", "jq: error (at @DIR@/mix.seq:1): Unmatched ']' at line 1, column 3 (need RS to resync)\n", 5),
        (&["-c", "-n", "-s", "--seq", "input"][..], &["mix", "arr", "dir"][..], "", "jq: error (at @DIR@/mix.seq:1): Unmatched ']' at line 1, column 3 (need RS to resync)\n", 5),
        (&["-c", "-n", "-s", "--seq", "input"][..], &["arr", "mix", "dir"][..], "", "jq: error (at @DIR@/mix.seq:1): Unmatched ']' at line 2, column 3 (need RS to resync)\n", 5),
        (&["-c", "-n", "-s", "--seq", "input"][..], &["two", "mix", "dir"][..], "", "jq: error (at @DIR@/mix.seq:1): Unmatched ']' at line 3, column 3 (need RS to resync)\n", 5),
        (&["-c", "-n", "-s", "--seq", "input"][..], &["mix", "two", "dir"][..], "", "jq: error (at @DIR@/mix.seq:1): Unmatched ']' at line 1, column 3 (need RS to resync)\n", 5),
        (&["-c", "-n", "-s", "--seq", "input"][..], &["trunc", "miss", "dir"][..], "", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: error (at @DIR@/dir:0): Unfinished JSON term at EOF at line 1, column 4\n", 2),
        (&["-c", "-n", "-s", "--seq", "input"][..], &["arr", "dir", "dir2"][..], "\u{1e}[[1,2]]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "input"][..], &["mix", "empty", "dir"][..], "", "jq: error (at @DIR@/mix.seq:1): Unmatched ']' at line 1, column 3 (need RS to resync)\n", 5),
        (&["-c", "-n", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["dir"][..], "\u{1e}\"Unfinished abandoned text at EOF at line 1, column 0\"\n\u{1e}[]\n\u{1e}\"break\"\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["dir", "dir2"][..], "\u{1e}\"Unfinished abandoned text at EOF at line 1, column 0\"\n\u{1e}[]\n\u{1e}\"break\"\n", "jq: error: Is a directory\njq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["empty", "dir"][..], "\u{1e}\"Unfinished abandoned text at EOF at line 1, column 0\"\n\u{1e}[]\n\u{1e}\"break\"\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["mix", "dir"][..], "\u{1e}\"Unmatched ']' at line 1, column 3 (need RS to resync)\"\n\u{1e}\"Potentially truncated top-level numeric value at EOF at line 2, column 2\"\n\u{1e}[]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["mix", "dir", "dir2"][..], "\u{1e}\"Unmatched ']' at line 1, column 3 (need RS to resync)\"\n\u{1e}\"Potentially truncated top-level numeric value at EOF at line 2, column 2\"\n\u{1e}[]\n", "jq: error: Is a directory\njq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["mix", "miss", "dir"][..], "\u{1e}\"Unmatched ']' at line 1, column 3 (need RS to resync)\"\n\u{1e}\"Potentially truncated top-level numeric value at EOF at line 2, column 2\"\n\u{1e}[]\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["arr", "dir"][..], "\u{1e}[[1,2]]\n\u{1e}\"break\"\n\u{1e}\"break\"\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["arr", "miss", "dir"][..], "\u{1e}[[1,2]]\n\u{1e}\"break\"\n\u{1e}\"break\"\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["two", "dir"][..], "\u{1e}[1,2]\n\u{1e}\"break\"\n\u{1e}\"break\"\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["trunc", "dir"][..], "\u{1e}\"Unfinished JSON term at EOF at line 1, column 4\"\n\u{1e}[]\n\u{1e}\"break\"\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["empty", "mix", "dir"][..], "\u{1e}\"Unmatched ']' at line 1, column 3 (need RS to resync)\"\n\u{1e}\"Potentially truncated top-level numeric value at EOF at line 2, column 2\"\n\u{1e}[]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["mix", "arr", "dir"][..], "\u{1e}\"Unmatched ']' at line 1, column 3 (need RS to resync)\"\n\u{1e}\"Potentially truncated top-level numeric value at line 2, column 3\"\n\u{1e}[[1,2]]\n", "", 2),
        (&["-c", "-n", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["arr", "mix", "dir"][..], "\u{1e}\"Unmatched ']' at line 2, column 3 (need RS to resync)\"\n\u{1e}\"Potentially truncated top-level numeric value at EOF at line 3, column 2\"\n\u{1e}[[1,2]]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["two", "mix", "dir"][..], "\u{1e}\"Unmatched ']' at line 3, column 3 (need RS to resync)\"\n\u{1e}\"Potentially truncated top-level numeric value at EOF at line 4, column 2\"\n\u{1e}[1,2]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["mix", "two", "dir"][..], "\u{1e}\"Unmatched ']' at line 1, column 3 (need RS to resync)\"\n\u{1e}\"Potentially truncated top-level numeric value at line 2, column 3\"\n\u{1e}[1,2]\n", "", 2),
        (&["-c", "-n", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["trunc", "miss", "dir"][..], "\u{1e}\"Unfinished JSON term at EOF at line 1, column 4\"\n\u{1e}[]\n\u{1e}\"break\"\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["arr", "dir", "dir2"][..], "\u{1e}[[1,2]]\n\u{1e}\"break\"\n\u{1e}\"break\"\n", "jq: error: Is a directory\njq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["mix", "empty", "dir"][..], "\u{1e}\"Unmatched ']' at line 1, column 3 (need RS to resync)\"\n\u{1e}\"Potentially truncated top-level numeric value at EOF at line 2, column 2\"\n\u{1e}[]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-s", "--seq", "input"][..], &["dir"][..], "", "jq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 0\n", 2),
        (&["-c", "-s", "--seq", "input"][..], &["dir", "dir2"][..], "", "jq: error: Is a directory\njq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 0\n", 2),
        (&["-c", "-s", "--seq", "input"][..], &["empty", "dir"][..], "", "jq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 0\n", 2),
        (&["-c", "-s", "--seq", "input"][..], &["mix", "dir"][..], "", "jq: ignoring parse error: Unmatched ']' at line 1, column 3 (need RS to resync)\njq: ignoring parse error: Potentially truncated top-level numeric value at EOF at line 2, column 2\n", 2),
        (&["-c", "-s", "--seq", "input"][..], &["mix", "dir", "dir2"][..], "", "jq: ignoring parse error: Unmatched ']' at line 1, column 3 (need RS to resync)\njq: error: Is a directory\njq: ignoring parse error: Potentially truncated top-level numeric value at EOF at line 2, column 2\n", 2),
        (&["-c", "-s", "--seq", "input"][..], &["mix", "miss", "dir"][..], "", "jq: ignoring parse error: Unmatched ']' at line 1, column 3 (need RS to resync)\njq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: ignoring parse error: Potentially truncated top-level numeric value at EOF at line 2, column 2\n", 2),
        (&["-c", "-s", "--seq", "input"][..], &["arr", "dir"][..], "", "jq: error: Is a directory\njq: error (at <unknown>): break\n", 2),
        (&["-c", "-s", "--seq", "input"][..], &["arr", "miss", "dir"][..], "", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: error: Is a directory\njq: error (at <unknown>): break\n", 2),
        (&["-c", "-s", "--seq", "input"][..], &["two", "dir"][..], "", "jq: error: Is a directory\njq: error (at <unknown>): break\n", 2),
        (&["-c", "-s", "--seq", "input"][..], &["trunc", "dir"][..], "", "jq: ignoring parse error: Unfinished JSON term at EOF at line 1, column 4\n", 2),
        (&["-c", "-s", "--seq", "input"][..], &["empty", "mix", "dir"][..], "", "jq: ignoring parse error: Unmatched ']' at line 1, column 3 (need RS to resync)\njq: ignoring parse error: Potentially truncated top-level numeric value at EOF at line 2, column 2\n", 2),
        (&["-c", "-s", "--seq", "input"][..], &["mix", "arr", "dir"][..], "", "jq: ignoring parse error: Unmatched ']' at line 1, column 3 (need RS to resync)\njq: ignoring parse error: Potentially truncated top-level numeric value at line 2, column 3\njq: error: Is a directory\njq: error (at <unknown>): break\n", 2),
        (&["-c", "-s", "--seq", "input"][..], &["arr", "mix", "dir"][..], "", "jq: ignoring parse error: Unmatched ']' at line 2, column 3 (need RS to resync)\njq: ignoring parse error: Potentially truncated top-level numeric value at EOF at line 3, column 2\n", 2),
        (&["-c", "-s", "--seq", "input"][..], &["two", "mix", "dir"][..], "", "jq: ignoring parse error: Unmatched ']' at line 3, column 3 (need RS to resync)\njq: ignoring parse error: Potentially truncated top-level numeric value at EOF at line 4, column 2\n", 2),
        (&["-c", "-s", "--seq", "input"][..], &["mix", "two", "dir"][..], "", "jq: ignoring parse error: Unmatched ']' at line 1, column 3 (need RS to resync)\njq: ignoring parse error: Potentially truncated top-level numeric value at line 2, column 3\njq: error: Is a directory\njq: error (at <unknown>): break\n", 2),
        (&["-c", "-s", "--seq", "input"][..], &["trunc", "miss", "dir"][..], "", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: ignoring parse error: Unfinished JSON term at EOF at line 1, column 4\n", 2),
        (&["-c", "-s", "--seq", "input"][..], &["arr", "dir", "dir2"][..], "", "jq: error: Is a directory\njq: error: Is a directory\njq: error (at <unknown>): break\n", 2),
        (&["-c", "-s", "--seq", "input"][..], &["mix", "empty", "dir"][..], "", "jq: ignoring parse error: Unmatched ']' at line 1, column 3 (need RS to resync)\njq: ignoring parse error: Potentially truncated top-level numeric value at EOF at line 2, column 2\n", 2),
        (&["-c", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["dir"][..], "", "jq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 0\n", 2),
        (&["-c", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["dir", "dir2"][..], "", "jq: error: Is a directory\njq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 0\n", 2),
        (&["-c", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["empty", "dir"][..], "", "jq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 0\n", 2),
        (&["-c", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["mix", "dir"][..], "", "jq: ignoring parse error: Unmatched ']' at line 1, column 3 (need RS to resync)\njq: ignoring parse error: Potentially truncated top-level numeric value at EOF at line 2, column 2\n", 2),
        (&["-c", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["mix", "dir", "dir2"][..], "", "jq: ignoring parse error: Unmatched ']' at line 1, column 3 (need RS to resync)\njq: error: Is a directory\njq: ignoring parse error: Potentially truncated top-level numeric value at EOF at line 2, column 2\n", 2),
        (&["-c", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["mix", "miss", "dir"][..], "", "jq: ignoring parse error: Unmatched ']' at line 1, column 3 (need RS to resync)\njq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: ignoring parse error: Potentially truncated top-level numeric value at EOF at line 2, column 2\n", 2),
        (&["-c", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["arr", "dir"][..], "\u{1e}\"break\"\n\u{1e}\"break\"\n\u{1e}\"break\"\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["arr", "miss", "dir"][..], "\u{1e}\"break\"\n\u{1e}\"break\"\n\u{1e}\"break\"\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: error: Is a directory\n", 2),
        (&["-c", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["two", "dir"][..], "\u{1e}\"break\"\n\u{1e}\"break\"\n\u{1e}\"break\"\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["trunc", "dir"][..], "", "jq: ignoring parse error: Unfinished JSON term at EOF at line 1, column 4\n", 2),
        (&["-c", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["empty", "mix", "dir"][..], "", "jq: ignoring parse error: Unmatched ']' at line 1, column 3 (need RS to resync)\njq: ignoring parse error: Potentially truncated top-level numeric value at EOF at line 2, column 2\n", 2),
        (&["-c", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["mix", "arr", "dir"][..], "\u{1e}\"break\"\n\u{1e}\"break\"\n\u{1e}\"break\"\n", "jq: ignoring parse error: Unmatched ']' at line 1, column 3 (need RS to resync)\njq: ignoring parse error: Potentially truncated top-level numeric value at line 2, column 3\njq: error: Is a directory\n", 2),
        (&["-c", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["arr", "mix", "dir"][..], "", "jq: ignoring parse error: Unmatched ']' at line 2, column 3 (need RS to resync)\njq: ignoring parse error: Potentially truncated top-level numeric value at EOF at line 3, column 2\n", 2),
        (&["-c", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["two", "mix", "dir"][..], "", "jq: ignoring parse error: Unmatched ']' at line 3, column 3 (need RS to resync)\njq: ignoring parse error: Potentially truncated top-level numeric value at EOF at line 4, column 2\n", 2),
        (&["-c", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["mix", "two", "dir"][..], "\u{1e}\"break\"\n\u{1e}\"break\"\n\u{1e}\"break\"\n", "jq: ignoring parse error: Unmatched ']' at line 1, column 3 (need RS to resync)\njq: ignoring parse error: Potentially truncated top-level numeric value at line 2, column 3\njq: error: Is a directory\n", 2),
        (&["-c", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["trunc", "miss", "dir"][..], "", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: ignoring parse error: Unfinished JSON term at EOF at line 1, column 4\n", 2),
        (&["-c", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["arr", "dir", "dir2"][..], "\u{1e}\"break\"\n\u{1e}\"break\"\n\u{1e}\"break\"\n", "jq: error: Is a directory\njq: error: Is a directory\n", 2),
        (&["-c", "-s", "--seq", "(try input catch .), (try input catch .), (try input catch .)"][..], &["mix", "empty", "dir"][..], "", "jq: ignoring parse error: Unmatched ']' at line 1, column 3 (need RS to resync)\njq: ignoring parse error: Potentially truncated top-level numeric value at EOF at line 2, column 2\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream", "input"][..], &["dir"][..], "", "jq: error (at @DIR@/dir:0): Unfinished abandoned text at EOF at line 1, column 0\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream", "input"][..], &["dir", "dir2"][..], "", "jq: error: Is a directory\njq: error (at @DIR@/dir2:0): Unfinished abandoned text at EOF at line 1, column 0\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream", "input"][..], &["empty", "dir"][..], "", "jq: error (at @DIR@/dir:0): Unfinished abandoned text at EOF at line 1, column 0\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream", "input"][..], &["mix", "dir"][..], "", "jq: error (at @DIR@/mix.seq:1): Unmatched ']' in the middle of an object at line 1, column 3 (need RS to resync)\n", 5),
        (&["-c", "-n", "-s", "--seq", "--stream", "input"][..], &["mix", "dir", "dir2"][..], "", "jq: error (at @DIR@/mix.seq:1): Unmatched ']' in the middle of an object at line 1, column 3 (need RS to resync)\n", 5),
        (&["-c", "-n", "-s", "--seq", "--stream", "input"][..], &["mix", "miss", "dir"][..], "", "jq: error (at @DIR@/mix.seq:1): Unmatched ']' in the middle of an object at line 1, column 3 (need RS to resync)\n", 5),
        (&["-c", "-n", "-s", "--seq", "--stream", "input"][..], &["arr", "dir"][..], "\u{1e}[[[0],1],[[1],2],[[1]]]\n", "", 2),
        (&["-c", "-n", "-s", "--seq", "--stream", "input"][..], &["arr", "miss", "dir"][..], "\u{1e}[[[0],1],[[1],2],[[1]]]\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream", "input"][..], &["two", "dir"][..], "\u{1e}[[[],1],[[],2]]\n", "", 2),
        (&["-c", "-n", "-s", "--seq", "--stream", "input"][..], &["trunc", "dir"][..], "", "jq: error (at @DIR@/dir:0): Unfinished JSON term at EOF at line 1, column 4\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream", "input"][..], &["empty", "mix", "dir"][..], "", "jq: error (at @DIR@/mix.seq:1): Unmatched ']' in the middle of an object at line 1, column 3 (need RS to resync)\n", 5),
        (&["-c", "-n", "-s", "--seq", "--stream", "input"][..], &["mix", "arr", "dir"][..], "", "jq: error (at @DIR@/mix.seq:1): Unmatched ']' in the middle of an object at line 1, column 3 (need RS to resync)\n", 5),
        (&["-c", "-n", "-s", "--seq", "--stream", "input"][..], &["arr", "mix", "dir"][..], "", "jq: error (at @DIR@/mix.seq:1): Unmatched ']' in the middle of an object at line 2, column 3 (need RS to resync)\n", 5),
        (&["-c", "-n", "-s", "--seq", "--stream", "input"][..], &["two", "mix", "dir"][..], "", "jq: error (at @DIR@/mix.seq:1): Unmatched ']' in the middle of an object at line 3, column 3 (need RS to resync)\n", 5),
        (&["-c", "-n", "-s", "--seq", "--stream", "input"][..], &["mix", "two", "dir"][..], "", "jq: error (at @DIR@/mix.seq:1): Unmatched ']' in the middle of an object at line 1, column 3 (need RS to resync)\n", 5),
        (&["-c", "-n", "-s", "--seq", "--stream", "input"][..], &["trunc", "miss", "dir"][..], "", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: error (at @DIR@/dir:0): Unfinished JSON term at EOF at line 1, column 4\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream", "input"][..], &["arr", "dir", "dir2"][..], "\u{1e}[[[0],1],[[1],2],[[1]]]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream", "input"][..], &["mix", "empty", "dir"][..], "", "jq: error (at @DIR@/mix.seq:1): Unmatched ']' in the middle of an object at line 1, column 3 (need RS to resync)\n", 5),
        (&["-c", "-n", "-s", "--seq", "--stream", "(try input catch .), (try input catch .), (try input catch .)"][..], &["dir"][..], "\u{1e}\"Unfinished abandoned text at EOF at line 1, column 0\"\n\u{1e}[]\n\u{1e}\"break\"\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream", "(try input catch .), (try input catch .), (try input catch .)"][..], &["dir", "dir2"][..], "\u{1e}\"Unfinished abandoned text at EOF at line 1, column 0\"\n\u{1e}[]\n\u{1e}\"break\"\n", "jq: error: Is a directory\njq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream", "(try input catch .), (try input catch .), (try input catch .)"][..], &["empty", "dir"][..], "\u{1e}\"Unfinished abandoned text at EOF at line 1, column 0\"\n\u{1e}[]\n\u{1e}\"break\"\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream", "(try input catch .), (try input catch .), (try input catch .)"][..], &["mix", "dir"][..], "\u{1e}\"Unmatched ']' in the middle of an object at line 1, column 3 (need RS to resync)\"\n\u{1e}[[[],2]]\n\u{1e}\"break\"\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream", "(try input catch .), (try input catch .), (try input catch .)"][..], &["mix", "dir", "dir2"][..], "\u{1e}\"Unmatched ']' in the middle of an object at line 1, column 3 (need RS to resync)\"\n\u{1e}[[[],2]]\n\u{1e}\"break\"\n", "jq: error: Is a directory\njq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream", "(try input catch .), (try input catch .), (try input catch .)"][..], &["mix", "miss", "dir"][..], "\u{1e}\"Unmatched ']' in the middle of an object at line 1, column 3 (need RS to resync)\"\n\u{1e}[[[],2]]\n\u{1e}\"break\"\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream", "(try input catch .), (try input catch .), (try input catch .)"][..], &["arr", "dir"][..], "\u{1e}[[[0],1],[[1],2],[[1]]]\n\u{1e}\"break\"\n\u{1e}\"break\"\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream", "(try input catch .), (try input catch .), (try input catch .)"][..], &["arr", "miss", "dir"][..], "\u{1e}[[[0],1],[[1],2],[[1]]]\n\u{1e}\"break\"\n\u{1e}\"break\"\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream", "(try input catch .), (try input catch .), (try input catch .)"][..], &["two", "dir"][..], "\u{1e}[[[],1],[[],2]]\n\u{1e}\"break\"\n\u{1e}\"break\"\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream", "(try input catch .), (try input catch .), (try input catch .)"][..], &["trunc", "dir"][..], "\u{1e}\"Unfinished JSON term at EOF at line 1, column 4\"\n\u{1e}[[[0],1]]\n\u{1e}\"break\"\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream", "(try input catch .), (try input catch .), (try input catch .)"][..], &["empty", "mix", "dir"][..], "\u{1e}\"Unmatched ']' in the middle of an object at line 1, column 3 (need RS to resync)\"\n\u{1e}[[[],2]]\n\u{1e}\"break\"\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream", "(try input catch .), (try input catch .), (try input catch .)"][..], &["mix", "arr", "dir"][..], "\u{1e}\"Unmatched ']' in the middle of an object at line 1, column 3 (need RS to resync)\"\n\u{1e}[[[],2],[[0],1],[[1],2],[[1]]]\n\u{1e}\"break\"\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream", "(try input catch .), (try input catch .), (try input catch .)"][..], &["arr", "mix", "dir"][..], "\u{1e}\"Unmatched ']' in the middle of an object at line 2, column 3 (need RS to resync)\"\n\u{1e}[[[0],1],[[1],2],[[1]],[[],2]]\n\u{1e}\"break\"\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream", "(try input catch .), (try input catch .), (try input catch .)"][..], &["two", "mix", "dir"][..], "\u{1e}\"Unmatched ']' in the middle of an object at line 3, column 3 (need RS to resync)\"\n\u{1e}[[[],1],[[],2],[[],2]]\n\u{1e}\"break\"\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream", "(try input catch .), (try input catch .), (try input catch .)"][..], &["mix", "two", "dir"][..], "\u{1e}\"Unmatched ']' in the middle of an object at line 1, column 3 (need RS to resync)\"\n\u{1e}[[[],2],[[],1],[[],2]]\n\u{1e}\"break\"\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream", "(try input catch .), (try input catch .), (try input catch .)"][..], &["trunc", "miss", "dir"][..], "\u{1e}\"Unfinished JSON term at EOF at line 1, column 4\"\n\u{1e}[[[0],1]]\n\u{1e}\"break\"\n", "jq: error: Could not open file @DIR@/missing.json: No such file or directory\njq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream", "(try input catch .), (try input catch .), (try input catch .)"][..], &["arr", "dir", "dir2"][..], "\u{1e}[[[0],1],[[1],2],[[1]]]\n\u{1e}\"break\"\n\u{1e}\"break\"\n", "jq: error: Is a directory\njq: error: Is a directory\n", 2),
        (&["-c", "-n", "-s", "--seq", "--stream", "(try input catch .), (try input catch .), (try input catch .)"][..], &["mix", "empty", "dir"][..], "\u{1e}\"Unmatched ']' in the middle of an object at line 1, column 3 (need RS to resync)\"\n\u{1e}[[[],2]]\n\u{1e}\"break\"\n", "jq: error: Is a directory\n", 2),
    ];
    for &(flags, names, stdout, stderr, status) in rows {
        let files: Vec<String> = names
            .iter()
            .map(|name| match *name {
                "dir" | "dir2" => format!("{d}/{name}"),
                "mix" | "arr" | "trunc" | "two" => format!("{d}/{name}.seq"),
                "empty" => format!("{d}/empty.json"),
                _ => format!("{d}/missing.json"),
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
