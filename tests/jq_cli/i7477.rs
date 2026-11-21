//! #7477: with one free file descriptor (`ulimit -n 4`: 0, 1 and 2 are open) jq reads its input files
//! one after another, and `succinctly jq` failed to open every one of them (`Could not open file ...: Too many
//! open files`, exit 2). #7214 had made its standard output a duplicate of descriptor 1, which took the one
//! free descriptor before the first input was opened. It now writes to descriptor 1 itself and takes none.
//!
//! Every row is (descriptor limit, arguments, stdout, stderr, exit status) captured from jq 1.7.1 under
//! `sh -c 'ulimit -n N; exec jq ...'` in a directory holding `a.json` (`{"a":1}` and `[2]` on two lines),
//! `t.txt` (`x` and `y`) and `p.jq` (`.a`). jq 1.8.2 gives the same answers. A limit of 3 is not tried: a
//! process cannot start on every platform with it.

#![cfg(unix)]

use std::process::Command;

use anyhow::Result;

/// (descriptor limit, arguments, stdout, stderr, exit status).
const ROWS: &[(u32, &[&str], &str, &str, i32)] = &[
    (
        4,
        &["-c", ".", "a.json", "a.json", "a.json"],
        "{\"a\":1}\n[2]\n{\"a\":1}\n[2]\n{\"a\":1}\n[2]\n",
        "",
        0,
    ),
    (4, &["-c", ".", "a.json"], "{\"a\":1}\n[2]\n", "", 0),
    (
        4,
        &["-c", "-s", ".", "a.json", "a.json"],
        "[{\"a\":1},[2],{\"a\":1},[2]]\n",
        "",
        0,
    ),
    (
        4,
        &["-R", ".", "t.txt", "t.txt"],
        "\"x\"\n\"y\"\n\"x\"\n\"y\"\n",
        "",
        0,
    ),
    (
        4,
        &["-c", "--stream", ".", "a.json", "a.json"],
        "[[\"a\"],1]\n[[\"a\"]]\n[[0],2]\n[[0]]\n[[\"a\"],1]\n[[\"a\"]]\n[[0],2]\n[[0]]\n",
        "",
        0,
    ),
    (
        4,
        &["-c", "--seq", ".", "a.json", "a.json"],
        "",
        "jq: ignoring parse error: Unfinished abandoned text at EOF at line 5, column 0\n",
        0,
    ),
    (4, &["-n", "1"], "1\n", "", 0),
    (
        4,
        &["-n", "-c", "[inputs]", "a.json", "a.json"],
        "[{\"a\":1},[2],{\"a\":1},[2]]\n",
        "",
        0,
    ),
    (
        4,
        &["-n", "-c", "[inputs]", "a.json", "gone.json", "a.json"],
        "[{\"a\":1},[2],{\"a\":1},[2]]\n",
        "jq: error: Could not open file gone.json: No such file or directory\n",
        2,
    ),
    (
        4,
        &["-c", "--arg", "x", "1", ".", "a.json"],
        "{\"a\":1}\n[2]\n",
        "",
        0,
    ),
    (
        4,
        &["-c", "--slurpfile", "s", "a.json", "$s", "a.json"],
        "[{\"a\":1},[2]]\n[{\"a\":1},[2]]\n",
        "",
        0,
    ),
    (
        4,
        &["-c", "--rawfile", "s", "t.txt", "$s", "a.json"],
        "\"x\\ny\\n\"\n\"x\\ny\\n\"\n",
        "",
        0,
    ),
    (
        4,
        &["-c", "-f", "p.jq", "a.json"],
        "1\n",
        "jq: error (at a.json:2): Cannot index array with string \"a\"\n",
        5,
    ),
    (
        4,
        &["-c", ".", "gone.json", "a.json"],
        "{\"a\":1}\n",
        "jq: error: Could not open file gone.json: No such file or directory\n",
        2,
    ),
    (
        5,
        &["-c", ".", "a.json", "a.json", "a.json"],
        "{\"a\":1}\n[2]\n{\"a\":1}\n[2]\n{\"a\":1}\n[2]\n",
        "",
        0,
    ),
    (5, &["-c", ".", "a.json"], "{\"a\":1}\n[2]\n", "", 0),
    (
        5,
        &["-c", "-s", ".", "a.json", "a.json"],
        "[{\"a\":1},[2],{\"a\":1},[2]]\n",
        "",
        0,
    ),
    (
        5,
        &["-R", ".", "t.txt", "t.txt"],
        "\"x\"\n\"y\"\n\"x\"\n\"y\"\n",
        "",
        0,
    ),
    (
        5,
        &["-c", "--stream", ".", "a.json", "a.json"],
        "[[\"a\"],1]\n[[\"a\"]]\n[[0],2]\n[[0]]\n[[\"a\"],1]\n[[\"a\"]]\n[[0],2]\n[[0]]\n",
        "",
        0,
    ),
    (
        5,
        &["-c", "--seq", ".", "a.json", "a.json"],
        "",
        "jq: ignoring parse error: Unfinished abandoned text at EOF at line 5, column 0\n",
        0,
    ),
    (5, &["-n", "1"], "1\n", "", 0),
    (
        5,
        &["-n", "-c", "[inputs]", "a.json", "a.json"],
        "[{\"a\":1},[2],{\"a\":1},[2]]\n",
        "",
        0,
    ),
    (
        5,
        &["-n", "-c", "[inputs]", "a.json", "gone.json", "a.json"],
        "[{\"a\":1},[2],{\"a\":1},[2]]\n",
        "jq: error: Could not open file gone.json: No such file or directory\n",
        2,
    ),
    (
        5,
        &["-c", "--arg", "x", "1", ".", "a.json"],
        "{\"a\":1}\n[2]\n",
        "",
        0,
    ),
    (
        5,
        &["-c", "--slurpfile", "s", "a.json", "$s", "a.json"],
        "[{\"a\":1},[2]]\n[{\"a\":1},[2]]\n",
        "",
        0,
    ),
    (
        5,
        &["-c", "--rawfile", "s", "t.txt", "$s", "a.json"],
        "\"x\\ny\\n\"\n\"x\\ny\\n\"\n",
        "",
        0,
    ),
    (
        5,
        &["-c", "-f", "p.jq", "a.json"],
        "1\n",
        "jq: error (at a.json:2): Cannot index array with string \"a\"\n",
        5,
    ),
    (
        5,
        &["-c", ".", "gone.json", "a.json"],
        "{\"a\":1}\n",
        "jq: error: Could not open file gone.json: No such file or directory\n",
        2,
    ),
];

#[test]
fn input_files_open_with_a_single_free_descriptor_7477() -> Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("a.json"), "{\"a\":1}\n[2]\n")?;
    std::fs::write(dir.path().join("t.txt"), "x\ny\n")?;
    std::fs::write(dir.path().join("p.jq"), ".a\n")?;
    for &(limit, args, stdout, stderr, status) in ROWS {
        let output = Command::new("sh")
            .arg("-c")
            .arg(format!("ulimit -n {limit}; exec \"$0\" \"$@\""))
            .arg(env!("CARGO_BIN_EXE_succinctly"))
            .arg("jq")
            .args(args)
            .current_dir(dir.path())
            .env("LC_ALL", "C")
            .output()?;
        assert_eq!(
            (
                String::from_utf8(output.stdout)?.as_str(),
                String::from_utf8(output.stderr)?.as_str(),
                output.status.code()
            ),
            (stdout, stderr, Some(status)),
            "ulimit -n {limit}: {args:?}"
        );
    }
    Ok(())
}
