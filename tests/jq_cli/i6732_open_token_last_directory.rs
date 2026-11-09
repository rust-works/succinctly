//! #6732: under plain `--stream-errors`, a token left open across files (`[`, `[1,`) that a file
//! ending in a container with a scalar before its closer (`{"a":1}`) continues is read as one text,
//! and a directory given last then takes the read that delivers the closer the file held back. The
//! main loop reads no further after that read, so the fault the text ends in (`Unfinished JSON term
//! at EOF`) is for the program's own `input` reads, and a program that makes none never sees it.
//! An empty file between, or a closer that is not held back, leaves the read to the fault as before.
//! Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_last_directory_that_takes_a_held_closers_read_stops_the_main_loop_before_the_eof_fault_6732(
) -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::create_dir(dir.path().join("dir"))?;
    std::fs::write(dir.path().join("open.json"), "[")?;
    std::fs::write(dir.path().join("open2.json"), "[1,")?;
    std::fs::write(dir.path().join("obj.json"), r#"{"a":1}"#)?;
    std::fs::write(dir.path().join("arr.json"), "[1,[2]]")?;
    std::fs::write(dir.path().join("empty.json"), "")?;
    #[rustfmt::skip]
    let rows = [
        (&["-c", "--stream-errors", "."][..], &["open.json", "obj.json", "dir"][..], "[[0,\"a\"],1]\n[[0,\"a\"]]\n", "", 2),
        (&["-c", "--stream-errors", "."][..], &["open2.json", "obj.json", "dir"][..], "[[0],1]\n[[1,\"a\"],1]\n[[1,\"a\"]]\n", "", 2),
        (&["-c", "--stream-errors", "first"][..], &["open.json", "obj.json", "dir"][..], "[0,\"a\"]\n[0,\"a\"]\n", "", 2),
        (&["-c", "--stream-errors", "first"][..], &["open2.json", "obj.json", "dir"][..], "[0]\n[1,\"a\"]\n[1,\"a\"]\n", "", 2),
        (&["-c", "--stream-errors", "input"][..], &["open.json", "obj.json", "dir"][..], "[[0,\"a\"]]\n", "", 2),
        (&["-c", "--stream-errors", "input"][..], &["open2.json", "obj.json", "dir"][..], "[[1,\"a\"],1]\n[\"Unfinished JSON term at EOF at line 1, column 10\",[1]]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "--stream-errors", "[inputs]"][..], &["open.json", "obj.json", "dir"][..], "[[[0,\"a\"]],[\"Unfinished JSON term at EOF at line 1, column 8\",[0]]]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "--stream-errors", ".,input"][..], &["open.json", "obj.json", "dir"][..], "[[0,\"a\"],1]\n[[0,\"a\"]]\n", "", 2),
        (&["-c", "--stream-errors", ".,input"][..], &["open2.json", "obj.json", "dir"][..], "[[0],1]\n[[1,\"a\"],1]\n[[1,\"a\"]]\n[\"Unfinished JSON term at EOF at line 1, column 10\",[1]]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "--stream-errors", "."][..], &["open.json", "obj.json", "empty.json", "dir"][..], "[[0,\"a\"],1]\n[[0,\"a\"]]\n[\"Unfinished JSON term at EOF at line 1, column 8\",[0]]\n", "", 2),
        (&["-c", "--stream-errors", "."][..], &["open.json", "arr.json", "dir"][..], "[[0,0],1]\n[[0,1,0],2]\n[[0,1,0]]\n[[0,1]]\n[\"Unfinished JSON term at EOF at line 1, column 8\",[0]]\n", "", 2),
        (&["-c", "--stream-errors", "."][..], &["open.json", "dir"][..], "[\"Unfinished JSON term at EOF at line 1, column 1\",[0]]\n", "", 2),
        (&["-c", "--stream-errors", "."][..], &["open.json", "obj.json", "dir", "obj.json"][..], "[[0,\"a\"],1]\n[[0,\"a\"]]\n", "", 2),
    ];
    for (flags, names, stdout, stderr, status) in rows {
        let files: Vec<String> = names.iter().map(|name| format!("{d}/{name}")).collect();
        let mut args: Vec<&str> = flags.to_vec();
        args.extend(files.iter().map(String::as_str));
        let (out, err, code) = run_jq_full(&args, None)?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (stdout, stderr, status),
            "{flags:?} {names:?}"
        );
    }
    Ok(())
}
