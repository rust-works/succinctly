//! #6676, #6681: under `--seq --stream -s` without `-n`, a program that reads no `input` gets the
//! whole slurp from jq's main loop in its first call, and the stream reader's warnings come out as
//! it meets each fault. A file that fails to open is said when the reader gets to it, so a warning
//! met before it leaves the array (`mix.seq dir` keeps `[[[],2]]`), a failure sits among the
//! warnings by source, and a directory given last is read after every warning, which counts it with
//! nothing said. A warning met after a failure was reported stops the loop with the array lost, and
//! the warning at the end of the text is met after every file. Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_slurp_places_its_open_failures_among_the_stream_readers_warnings_6676() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let d = dir.path().to_string_lossy().into_owned();
    std::fs::create_dir(dir.path().join("dir"))?;
    std::fs::create_dir(dir.path().join("dir2"))?;
    for (name, text) in [
        ("mix.seq", "\u{1e}{]\n\u{1e}2"),
        ("mid.seq", "\u{1e}1\n\u{1e}{]\n\u{1e}3\n"),
        ("trunc.seq", "\u{1e}[1,"),
        ("rend.seq", "\u{1e}12\n\u{1e}[1,"),
        ("ok.seq", "\u{1e}[1]\n"),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    #[rustfmt::skip]
    let rows = [
        (&["-c", "-s", "--seq", "--stream", "."][..], &["mix.seq", "dir"][..], "\u{1e}[[[],2]]\n", "jq: ignoring parse error: Unmatched ']' in the middle of an object at line 1, column 3 (need RS to resync)\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["mid.seq", "dir"][..], "\u{1e}[[[],1],[[],3]]\n", "jq: ignoring parse error: Unmatched ']' in the middle of an object at line 2, column 3 (need RS to resync)\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["mix.seq", "missing.json", "dir"][..], "\u{1e}[[[],2]]\n", "jq: ignoring parse error: Unmatched ']' in the middle of an object at line 1, column 3 (need RS to resync)\njq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-s", "--seq", "--stream", "length"][..], &["mid.seq", "dir", "dir2"][..], "\u{1e}2\n", "jq: ignoring parse error: Unmatched ']' in the middle of an object at line 2, column 3 (need RS to resync)\njq: error: Is a directory\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["mid.seq", "missing.json", "dir"][..], "\u{1e}[[[],1],[[],3]]\n", "jq: ignoring parse error: Unmatched ']' in the middle of an object at line 2, column 3 (need RS to resync)\njq: error: Could not open file @DIR@/missing.json: No such file or directory\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["trunc.seq", "dir"][..], "", "jq: ignoring parse error: Unfinished JSON term at EOF at line 1, column 4\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["rend.seq", "dir"][..], "", "jq: ignoring parse error: Unfinished JSON term at EOF at line 2, column 4\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["dir"][..], "", "jq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 0\n", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["ok.seq", "dir"][..], "\u{1e}[[[0],1],[[0]]]\n", "", 2),
        (&["-c", "-s", "--seq", "--stream", "."][..], &["mix.seq", "ok.seq", "dir"][..], "\u{1e}[[[],2],[[0],1],[[0]]]\n", "jq: ignoring parse error: Unmatched ']' in the middle of an object at line 1, column 3 (need RS to resync)\n", 2),
        (&["-c", "-s", "--seq", "--stream", "empty"][..], &["mix.seq", "dir"][..], "", "jq: ignoring parse error: Unmatched ']' in the middle of an object at line 1, column 3 (need RS to resync)\n", 2),
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
