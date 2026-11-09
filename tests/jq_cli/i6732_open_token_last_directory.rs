//! #6732: under plain `--stream-errors`, a token left open across files (`[`, `[1,`) that a file
//! ending in a container with a scalar before its closer (`{"a":1}`) continues is read as one text,
//! and a directory given last then takes the read that delivers the closer the file held back. The
//! main loop reads no further after that read, so the fault the text ends in (`Unfinished JSON term
//! at EOF`) is for the program's own `input` reads, and a program that makes none never sees it.
//! (`input` over `open2.json obj.json dir` prints the fault and `input` over `open.json obj.json dir` does
//! not only because the extra event `[1,` makes puts the closer one read later: the program's read is the
//! one that reaches the directory in the first and the loop's in the second.)
//! The closer can also be in a file that only finishes a container an earlier file opened (`[[` then `1]`).
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
    std::fs::write(dir.path().join("ob.json"), "[[")?;
    std::fs::write(dir.path().join("two.json"), "1]")?;
    std::fs::write(dir.path().join("oa2.json"), r#"[{"a":"#)?;
    std::fs::write(dir.path().join("one.json"), "1}")?;
    std::fs::write(dir.path().join("obje.json"), "{\"a\":1}\n")?;
    std::fs::write(dir.path().join("ws.json"), "[1 ]")?;
    std::fs::write(dir.path().join("a.json"), r#"["a"#)?;
    std::fs::write(dir.path().join("b.json"), r#"b"]"#)?;
    // `1`, spaces, `]` at 4,095 and 4,094 bytes: a last read that fills jq's buffer is followed by an
    // empty read of the same file, which delivers the held-back closer in that file (#6760). These
    // rows agree with jq either way; the joined-text test's use of that rule is pinned by the `gone`
    // rows of `tests/jq_cli/i6760.rs`, which fail without it.
    std::fs::write(
        dir.path().join("full.json"),
        format!("1{}]", " ".repeat(4093)),
    )?;
    std::fs::write(
        dir.path().join("short.json"),
        format!("1{}]", " ".repeat(4092)),
    )?;
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
        // A newline after the closer reads with it (no held-back event), a space before it does not matter,
        // and a string that crosses the file boundary is one scalar.
        (&["-c", "--stream-errors", "."][..], &["open.json", "obje.json", "dir"][..], "[[0,\"a\"],1]\n[[0,\"a\"]]\n[\"Unfinished JSON term at EOF at line 2, column 0\",[0]]\n", "", 2),
        (&["-c", "--stream-errors", "."][..], &["a.json", "b.json", "dir"][..], "[[0],\"ab\"]\n[[0]]\n", "", 2),
        (&["-c", "--stream-errors", "[inputs]"][..], &["a.json", "b.json", "dir"][..], "[[[0]]]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "--stream-errors", "[inputs]"][..], &["open.json", "full.json", "dir"][..], "[[[0]]]\n", "", 2),
        (&["-c", "--stream-errors", "[inputs]"][..], &["open.json", "short.json", "dir"][..], "[[[0]]]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "--stream-errors", "."][..], &["open.json", "ws.json", "dir"][..], "[[0,0],1]\n[[0,0]]\n", "", 2),
        // A closer in a file that only finishes a container an earlier file opened, and under `-n`.
        (&["-c", "--stream-errors", "."][..], &["ob.json", "two.json", "dir"][..], "[[0,0],1]\n[[0,0]]\n", "", 2),
        (&["-c", "--stream-errors", "input"][..], &["ob.json", "two.json", "dir"][..], "[[0,0]]\n", "", 2),
        (&["-c", "--stream-errors", "."][..], &["oa2.json", "one.json", "dir"][..], "[[0,\"a\"],1]\n[[0,\"a\"]]\n", "", 2),
        (&["-c", "--stream-errors", "[inputs]"][..], &["oa2.json", "one.json", "dir"][..], "[[[0,\"a\"]],[\"Unfinished JSON term at EOF at line 1, column 8\",[0]]]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "--stream-errors", "-n", "[inputs]"][..], &["open.json", "obj.json", "dir"][..], "[[[0,\"a\"],1],[[0,\"a\"]],[\"Unfinished JSON term at EOF at line 1, column 8\",[0]]]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "--stream-errors", "-n", "input"][..], &["open.json", "obj.json", "dir"][..], "[[0,\"a\"],1]\n", "", 0),
        (&["-c", "--stream-errors", "-n", "[inputs]"][..], &["ob.json", "two.json", "dir"][..], "[[[0,0],1],[[0,0]],[\"Unfinished JSON term at EOF at line 1, column 4\",[0]]]\n", "jq: error: Is a directory\n", 2),
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
